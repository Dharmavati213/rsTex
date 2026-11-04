#!/usr/bin/env -S python3 -I
"""warm.py: what the preamble checkpoint of DESIGN.md section 7 is worth, and what it costs.

    warm.py --engine PATH [--base PATH] [--docs hello,ladder,main,LMB,summary,SGA1-VI,sga3] [-n 3] [--counters 3] [--no-commute] [--policy] [--json FILE]

Each document compiles the way bench.py does, in one directory that keeps its path, with the oracle's last-pass side files put back before every run:

    cold     RSTEX_NO_SNAPSHOT=1: no checkpoint looked for, none written
    write    an empty cache directory and RSTEX_CHECKPOINT_POLICY=always: a run that writes one
    warm     the cache that run left, once a run has left its seal: a restore, proved by RSTEX_CHECKPOINT_TRACE.  No PDF is there, so the output cache of DESIGN.md section 9 finds nothing
    hits     the same with the last PDF in its place, as between two runs of a document: all of its streams come from there
    last     a chain of page records written with that PDF in place, and a run from its last record that finds what comes after it there: what a save costs near the end of a document, the floor.  A document with no record has none

All but the last with RSTEX_NO_PAGES: this is the preamble's checkpoint alone.  tests/corpus/snapshot/pages.py session and tools/prof/resume.py are for what a run makes of page records at other places, and --policy, which is the program as it is, has them.

--policy adds what a user gets from an empty cache and the program's own policy: the first, second, third and fourth runs in a row, each timed from there.  --base adds the same cold run of
another binary with its own format.  Instructions first, as for everything on this machine: tools/prof/bench.py says why.  A document is only named by its manifest id, and
nothing of it is printed.  Reasons a checkpoint was refused are shown for documents that are not private.
"""
import argparse
import json
import os
import shutil
import statistics
import subprocess
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import proflib  # noqa: E402


def env_for(engine, doc, cache):
    env = proflib.corpus().entry_env(doc.entry)
    env['XDG_CACHE_HOME'] = cache
    if engine.no_commute:
        env['RSTEX_NO_COMMUTE'] = '1'
    env['RSTEX_CHECKPOINT_POLICY'] = 'always'
    env['RSTEX_NO_PAGES'] = '1'
    return env


def reset(work, doc, keep_pdf=False):
    pdf = os.path.join(work, doc.entry['jobname'] + '.pdf')
    kept = None
    if keep_pdf and os.path.exists(pdf):
        with open(pdf, 'rb') as f:
            kept = f.read()
    shutil.rmtree(work, ignore_errors=True)
    os.makedirs(work)
    proflib.corpus().restore_state(doc.prepare(), work)
    if kept is not None:
        with open(pdf, 'wb') as f:
            f.write(kept)


def measure(engine, doc, work, env, n, counters, before=None, keep_pdf=False):
    """Timed runs, then counted ones, each from the same starting point.  `before` runs ahead of each."""
    runs = []
    for _ in range(n):
        reset(work, doc, keep_pdf)
        if before:
            before()
        argv, cwd, _ = engine.command(doc, work)
        runs.append(proflib.run_timed(argv, cwd, env))
    cs = []
    for _ in range(counters):
        reset(work, doc, keep_pdf)
        if before:
            before()
        argv, cwd, _ = engine.command(doc, work)
        cs.append(proflib.run_counted(argv, cwd, env)['counters'])
    res = {
        'cpu_ms': statistics.median(r['cpu_ms'] for r in runs) if runs else None,
        'cpu_ms_min': min(r['cpu_ms'] for r in runs) if runs else None,
        'wall_ms': statistics.median(r['wall_ms'] for r in runs) if runs else None,
        'wall_ms_min': min(r['wall_ms'] for r in runs) if runs else None,
        'minflt': statistics.median(r['minflt'] for r in runs) if runs else None,
        'rc': sorted({r['rc'] for r in runs}),
    }
    for name in ('instructions', 'cycles', 'page-faults'):
        vals = [c[name]['value'] for c in cs if name in c]
        res[name] = statistics.median(vals) if vals else None
    return res


def traced(engine, doc, work, env):
    """One run with the trace on: what it said about the checkpoint."""
    reset(work, doc)
    argv, cwd, _ = engine.command(doc, work)
    e = dict(env)
    e['RSTEX_CHECKPOINT_TRACE'] = '1'
    p = subprocess.run(argv, cwd=cwd, env=e, stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
    return [line[len('rstex: checkpoint: '):] for line in p.stderr.decode('latin-1').splitlines() if line.startswith('rstex: checkpoint: ')]


def traced_keeping(engine, doc, work, env):
    """One run with the trace on from where the last left the PDF, writing nothing to a chain: what it said."""
    argv, cwd, _ = engine.command(doc, work)
    p = subprocess.run(argv, cwd=cwd, env=dict(env, RSTEX_CHECKPOINT_TRACE='1', RSTEX_PAGE_READONLY='1'), stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
    return [line[len('rstex: checkpoint: '):] for line in p.stderr.decode('latin-1').splitlines() if line.startswith('rstex: checkpoint: ')]


def cache_size(cache):
    total = 0
    for root, _, names in os.walk(os.path.join(cache, 'rstex', 'checkpoints')):
        for n in names:
            total += os.path.getsize(os.path.join(root, n))
    return total


def one_doc(engine, base, doc, args):
    doc.prepare()
    root = os.path.join(proflib.out_root(), 'warm', doc.name.replace('/', '_'))
    shutil.rmtree(root, ignore_errors=True)
    work = os.path.join(root, 'o')
    cache = os.path.join(root, 'cache')
    os.makedirs(cache)
    # kpathsea's answer about where the tree is lives in the same directory: bench.py's first run has filled the same for its own.
    rec = {'id': doc.id, 'name': doc.name}
    env = env_for(engine, doc, cache)
    cold_env = dict(env, RSTEX_NO_SNAPSHOT='1')
    traced(engine, doc, work, cold_env)
    rec['cold'] = measure(engine, doc, work, cold_env, args.n, args.counters)
    if base:
        base_env = dict(env_for(base, doc, cache), RSTEX_NO_SNAPSHOT='1')
        rec['base'] = measure(base, doc, work, base_env, args.n, args.counters)

    def empty_cache():
        shutil.rmtree(os.path.join(cache, 'rstex', 'checkpoints'), ignore_errors=True)
    rec['write'] = measure(engine, doc, work, env, args.n, args.counters, before=empty_cache)
    empty_cache()
    said = traced(engine, doc, work, env)
    rec['first_run_said'] = said if not doc.id.startswith(('user/', 'local/')) else [s.split(':')[0] for s in said]
    rec['checkpoint_bytes'] = cache_size(cache)
    said = traced(engine, doc, work, env)
    rec['restored'] = 'restored' in said
    rec['second_run_said'] = said if not doc.id.startswith(('user/', 'local/')) else [s.split(':')[0] for s in said]
    if rec['restored']:
        argv, cwd, _ = engine.command(doc, work)
        proflib.seal_checkpoint(lambda: reset(work, doc), argv, cwd, env)
        rec['warm'] = measure(engine, doc, work, env, args.n, args.counters)
        rec['hits'] = measure(engine, doc, work, env, args.n, args.counters, keep_pdf=True)
        chained = {k: v for k, v in env.items() if k != 'RSTEX_NO_PAGES'}
        reset(work, doc, True)
        proflib.run_timed(argv, cwd, chained)
        reset(work, doc, True)
        if any(' records, using ' in s and not s.endswith('using 0') for s in traced_keeping(engine, doc, work, chained)):
            rec['last'] = measure(engine, doc, work, dict(chained, RSTEX_PAGE_READONLY='1'), args.n, args.counters, keep_pdf=True)
    if args.policy:
        auto = {k: v for k, v in env.items() if k not in ('RSTEX_CHECKPOINT_POLICY', 'RSTEX_NO_PAGES')}

        def runs_before(k):
            def before():
                empty_cache()
                for _ in range(k):
                    reset(work, doc)
                    argv, cwd, _ = engine.command(doc, work)
                    proflib.run_timed(argv, cwd, auto)
                reset(work, doc)
            return before
        for k, label in enumerate(('first', 'second', 'third', 'fourth')):
            rec[label] = measure(engine, doc, work, auto, args.n, args.counters, before=runs_before(k))
    return rec


def row(label, r, cold):
    instr = r.get('instructions') or 0
    ratio = instr / cold['instructions'] if cold and cold.get('instructions') and instr else 0
    print(f'  {label:<7}{instr / 1e6:>11.2f}{ratio:>8.3f}{(r.get("cycles") or 0) / 1e6:>11.1f}{(r.get("page-faults") or 0):>8.0f}{r["cpu_ms"] or 0:>9.1f}{r["cpu_ms_min"] or 0:>9.1f}{r["wall_ms"] or 0:>9.1f}{r["wall_ms_min"] or 0:>9.1f}  {r["rc"]}')


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument('--engine', required=True)
    ap.add_argument('--base')
    ap.add_argument('--docs', default='hello,ladder,main,LMB,summary,SGA1-VI,sga3')
    ap.add_argument('-n', type=int, default=3)
    ap.add_argument('--counters', type=int, default=3)
    ap.add_argument('--no-commute', action='store_true', help='real PGF for every diagram, for --engine')
    ap.add_argument('--policy', action='store_true')
    ap.add_argument('--json')
    args = ap.parse_args()
    engine = proflib.Engine('new', args.engine, no_commute=args.no_commute)
    base = proflib.Engine('base', args.base) if args.base else None
    recs = []
    print(f'{"":<9}{"Minstr":>11}{"/cold":>8}{"Mcycles":>11}{"faults":>8}{"cpu med":>9}{"cpu min":>9}{"wall med":>9}{"wall min":>9}')
    for doc in proflib.resolve_docs(args.docs.split(',')):
        rec = one_doc(engine, base, doc, args)
        recs.append(rec)
        print(f'{doc.name}, checkpoint {rec["checkpoint_bytes"] / 1e6:.2f} MB, first run: {rec["first_run_said"]}, second run: {rec["second_run_said"]}')
        for label in ('base', 'cold', 'write', 'warm', 'hits', 'last', 'first', 'second', 'third', 'fourth'):
            if label in rec:
                row(label, rec[label], rec['cold'])
        sys.stdout.flush()
    if args.json:
        with open(args.json, 'w') as f:
            json.dump(recs, f, indent=1)


if __name__ == '__main__':
    main()
