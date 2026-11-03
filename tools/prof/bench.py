#!/usr/bin/env -S python3 -I
"""bench.py: engines against each other and against pdflatex, on a named document set.

    bench.py --engine NAME=PATH [--engine NAME=PATH]... [--docs baseline] [-n 11] [--budget S] [--counters 3]
             [--cpu K] [--max-load L] [--load-patience S] [--no-oracle] [--format-build N] [--floors] [--json FILE]

--docs takes names, sets and manifest ids, separated by commas; tools/prof/proflib.py has them.

A document compiles once per run in a fresh directory under target/ with the oracle's last-pass side files in place, which is what tests/corpus/run.py bench calls
last pass.  Each run of each engine is timed by the plain spawn, in turn, so a warming cache favours neither.  --counters runs follow, under user-mode hardware
counters taken from outside the process, so a shipped build and pdflatex have instruction counts too.  Instructions do not depend on the load of the machine;
wall clock does, and user+sys less so, but not by nothing: a busy sibling hyperthread and a shared cache slow a program down whatever the clock says.

Formats: an engine gets its own, built once per binary by tests/integration/run.py's FORMAT_RECIPES; the oracle uses the installed ones.  The environment is
run.py's entry_env, so SOURCE_DATE_EPOCH and the rest are the same for all.  --format-build N also times N builds of pdflatex.fmt, the target of DESIGN.md section 2.
An engine's tree cache stays between its runs and a first run fills it: --floors shows that first-run cost on an empty one.

Why not tests/corpus/run.py bench: it is the right tool for a speed-up over the corpus, with its check that a timed engine's answers were verified.  It keeps the
minimum of N against the oracle only, has no per-binary formats, and no counters.  This one takes its documents, argv, environment and side files from there, by import.
"""
import argparse
import json
import os
import shutil
import statistics
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import proflib  # noqa: E402


def summary(runs):
    res = {'n': len(runs)}
    for key in ('wall_ms', 'cpu_ms', 'user_ms', 'sys_ms'):
        vals = [r[key] for r in runs]
        res[key] = {'median': statistics.median(vals), 'p90': proflib.percentile(vals, 90), 'min': min(vals)}
    res['minflt'] = statistics.median(r['minflt'] for r in runs)
    res['maxrss_kb'] = statistics.median(r['maxrss_kb'] for r in runs)
    return res


def once(engine, doc):
    work = proflib.Workdir(doc)
    argv, cwd, env = engine.command(doc, work.out)
    r = proflib.run_timed(argv, cwd, env)
    r['pdf'] = os.path.exists(os.path.join(work.out, doc.entry['jobname'] + '.pdf'))
    work.remove()
    return r


def counted(engine, doc):
    work = proflib.Workdir(doc)
    argv, cwd, env = engine.command(doc, work.out)
    r = proflib.run_counted(argv, cwd, env)
    work.remove()
    return r


def bench_doc(doc, engines, args):
    doc.prepare()
    load0, waited = proflib.wait_for_load(args.max_load, args.load_patience)
    for e in engines:
        if not e.is_oracle:
            e.format_path(doc.format)
            once(e, doc)                         # fills the tree cache
    runs = {e.label: [] for e in engines}
    t0 = time.monotonic()
    for i in range(args.n):
        order = engines if i % 2 == 0 else list(reversed(engines))
        for e in order:
            runs[e.label].append(once(e, doc))
        if i >= 2 and time.monotonic() - t0 > args.budget:      # a long document gets fewer runs
            break
    rec = {'id': doc.id, 'name': doc.name, 'load_before': load0, 'load_wait_s': waited, 'load_after': proflib.load_now(), 'engines': {}}
    for e in engines:
        rs = runs[e.label]
        s = summary(rs)
        s['exit'] = sorted({r['rc'] for r in rs})
        s['pdf'] = all(r['pdf'] for r in rs)
        if args.counters:
            cs = [counted(e, doc)['counters'] for _ in range(args.counters)]
            s['counters'] = {name: statistics.median(c[name]['value'] for c in cs if name in c) for name in {n for c in cs for n in c}}
            s['multiplexed'] = any(v['running_ns'] < v['enabled_ns'] for c in cs for v in c.values())
        rec['engines'][e.label] = s
    return rec


def print_doc(rec, base_label):
    print(f'{rec["name"]} ({rec["id"]}), load {rec["load_before"]:.1f} to {rec["load_after"]:.1f}')
    base = rec['engines'].get(base_label)
    print(f'  {"":<12}{"user+sys med":>13}{"p90":>9}{"min":>9}{"wall med":>10}{"p90":>9}{"min":>9}{"Minstr":>11}{"IPC":>6}{"faults":>8}{"RSS MiB":>9}{"cpu ratio":>11}{"instr ratio":>13}  exit')
    for label, s in rec['engines'].items():
        c = s.get('counters', {})
        instr = c.get('instructions')
        ipc = instr / c['cycles'] if instr and c.get('cycles') else None
        ratio = s['cpu_ms']['median'] / base['cpu_ms']['median'] if base else None
        iratio = instr / base['counters']['instructions'] if base and instr and base.get('counters', {}).get('instructions') else None
        print(f'  {label:<12}{s["cpu_ms"]["median"]:>13.1f}{s["cpu_ms"]["p90"]:>9.1f}{s["cpu_ms"]["min"]:>9.1f}{s["wall_ms"]["median"]:>10.1f}{s["wall_ms"]["p90"]:>9.1f}{s["wall_ms"]["min"]:>9.1f}'
              f'{(instr or 0) / 1e6:>11.1f}{(ipc or 0):>6.2f}{s["minflt"]:>8.0f}{s["maxrss_kb"] / 1024:>9.1f}{(ratio or 0):>11.3f}{(iratio or 0):>13.3f}  {s["exit"]}{"" if s["pdf"] else " NO PDF"}')
        if s.get('multiplexed'):
            print('    note: counters were multiplexed in this row')


def format_builds(engines, n):
    print(f'\nformat build, pdflatex.fmt, {n} runs (DESIGN.md section 2: pdflatex 5.0 s, target under 2.5 s):')
    out = {}
    for e in engines:
        runs = []
        for i in range(n):
            d = os.path.join(proflib.out_root(), 'w', f'fmt-{os.getpid()}-{e.label}-{i}')
            os.makedirs(d)
            runs.append(proflib.run_format_build(e, 'pdflatex', d))
            shutil.rmtree(d, ignore_errors=True)
        s = summary(runs)
        d = os.path.join(proflib.out_root(), 'w', f'fmt-{os.getpid()}-{e.label}-c')
        os.makedirs(d)
        s['counters'] = {k: v['value'] for k, v in proflib.run_counted(proflib.format_command(e, 'pdflatex'), d, proflib.format_env(e))['counters'].items()}
        shutil.rmtree(d, ignore_errors=True)
        out[e.label] = s
        print(f'  {e.label:<12} user+sys median {s["cpu_ms"]["median"] / 1e3:.2f} s, p90 {s["cpu_ms"]["p90"] / 1e3:.2f}, min {s["cpu_ms"]["min"] / 1e3:.2f}; wall median {s["wall_ms"]["median"] / 1e3:.2f} s; {s["counters"].get("instructions", 0) / 1e9:.2f} G instructions')
    return out


def floors(engines, n):
    """What starting and ending costs on its own: the shortest thing each binary can do, and a static-free /bin/true for the runner itself.  And a first run's tree search."""
    print(f'\nfloors, wall clock, min and median of {n}:')
    out = {}
    argvs = [('/bin/true', ['/bin/true'])]
    for e in engines:
        path = shutil.which('pdftex') if e.is_oracle else e.path
        argvs.append((f'{e.label} --version', [path, '--version']))
    for label, argv in argvs:
        walls = [proflib.run_timed(argv, '/', {k: os.environ[k] for k in ('PATH', 'HOME') if k in os.environ})['wall_ms'] for _ in range(n)]
        out[label] = {'min': min(walls), 'median': statistics.median(walls)}
        print(f'  {label:<24} {min(walls):>8.2f} {statistics.median(walls):>8.2f} ms')
    return out


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument('--engine', action='append', default=[], metavar='NAME=PATH')
    ap.add_argument('--docs', default='baseline')
    ap.add_argument('-n', type=int, default=11)
    ap.add_argument('--budget', type=float, default=120, help='seconds of timed runs per document: at least 3 rounds, and fewer than -n past this')
    ap.add_argument('--counters', type=int, default=3, help='runs under hardware counters, per document and engine; 0 for none')
    ap.add_argument('--cpu', type=int, default=None, help='pin everything to this CPU')
    ap.add_argument('--max-load', type=float, default=None, help='wait before each document for the one-minute load average to fall under this')
    ap.add_argument('--load-patience', type=float, default=300, help='seconds to wait for it')
    ap.add_argument('--no-oracle', action='store_true')
    ap.add_argument('--format-build', type=int, default=0, metavar='N')
    ap.add_argument('--floors', action='store_true')
    ap.add_argument('--json')
    args = ap.parse_args()
    if args.cpu is not None:
        os.sched_setaffinity(0, {args.cpu})
    engines = []
    if not args.no_oracle:
        engines.append(proflib.Engine('pdflatex'))
    for spec in args.engine:
        label, _, path = spec.partition('=')
        if not path:
            ap.error('--engine takes NAME=PATH')
        engines.append(proflib.Engine(label, path))
    if not engines:
        ap.error('no engine')
    base = 'pdflatex' if not args.no_oracle else engines[0].label
    print(f'{os.uname().nodename}: {os.cpu_count()} CPUs, pin {args.cpu if args.cpu is not None else "none"}, load now {proflib.load_now():.1f}; {args.n} timed runs and {args.counters} counted runs each')
    for e in engines:
        if not e.is_oracle:
            print(f'{e.label}: {e.path} sha {e.sha[:12]}')
    results = {'cpu': args.cpu, 'n': args.n, 'counters': args.counters, 'engines': {e.label: e.path or shutil.which('pdflatex') for e in engines}, 'docs': []}
    if args.floors:
        results['floors'] = floors(engines, max(args.n, 21))
    for doc in proflib.resolve_docs(args.docs.split(',')):
        rec = bench_doc(doc, engines, args)
        results['docs'].append(rec)
        print_doc(rec, base)
        sys.stdout.flush()
    if args.format_build:
        results['format_build'] = format_builds(engines, args.format_build)
    if args.json:
        with open(args.json, 'w') as f:
            json.dump(results, f, indent=1)


if __name__ == '__main__':
    main()
