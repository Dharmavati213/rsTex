#!/usr/bin/env -S python3 -I
"""stable_cost.py: what a clean save costs under `--until-stable`, in two builds, as tools/prof/compare_cycles.py has it for a plain run.

    stable_cost.py --a PATH --b PATH [--docs hello,main] [--rounds 40] [--counters 3] [--lemma] [--no-flag] [--env NAME=VALUE]... [--json FILE]

Each build has a directory and a cache of its own under target/, of one length, and builds the document with its flag until nothing changes, and then a checkpoint with its seal is there, and the last PDF.  A clean save is that same run again, which reads the files and writes them: it is held back from answering without a pass by RSTEX_NO_SHORTCUT.
What it prints: the instructions of a run and its system calls by name, by tools/prof/ktrace.py, which are exact, and the median over the rounds of the paired ratio b/a of user-mode cycles and of CPU, with in brackets a/a over the same rounds, which says how far the machine can be trusted.  A round runs a, b, a, turned round each time.
What the kernel does for a call is in the CPU and not in the counts: this machine counts user mode only.  --lemma adds Lemma's flags, and --no-flag leaves out `--until-stable`, for a plain run.  A document is named by its manifest id, and nothing of it is printed.
"""
import argparse
import collections
import json
import os
import shutil
import statistics
import subprocess
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import ktrace  # noqa: E402
import proflib  # noqa: E402

LEMMA = ['-synctex=1', '-interaction=nonstopmode', '-file-line-error', '-recorder']


def prepare(engine, doc, tag, args, extra):
    root = os.path.join(proflib.out_root(), 'stable-cost', doc.name.replace('/', '_'))
    out = os.path.join(root, 'o-' + tag)
    cache = os.path.join(root, 'cache-' + tag)
    shutil.rmtree(out, ignore_errors=True)
    shutil.rmtree(cache, ignore_errors=True)
    os.makedirs(out)
    os.makedirs(cache)
    proflib.corpus().restore_state(doc.prepare(), out)
    argv, cwd, env = engine.command(doc, out)
    flags = ([] if args.no_flag else ['--until-stable']) + (LEMMA if args.lemma else [])
    argv = argv[:1] + flags + argv[1:]
    env = dict(env, XDG_CACHE_HOME=cache, RSTEX_NO_SHORTCUT='1')
    env.pop('RSTEX_NO_SNAPSHOT', None)
    env.update(extra)
    # To where it settles, a checkpoint written and sealed, and an index of the last PDF.
    for _ in range(6):
        subprocess.run(argv, cwd=cwd, env=env, stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, check=False)
    proflib.seal_checkpoint(lambda: None, argv, cwd, env)
    return argv, cwd, env


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument('--a', required=True)
    ap.add_argument('--b', required=True)
    ap.add_argument('--docs', default='hello,main')
    ap.add_argument('--rounds', type=int, default=40)
    ap.add_argument('--counters', type=int, default=3)
    ap.add_argument('--lemma', action='store_true')
    ap.add_argument('--no-flag', action='store_true')
    ap.add_argument('--env', action='append', default=[])
    ap.add_argument('--json')
    args = ap.parse_args()
    extra = dict(x.split('=', 1) for x in args.env)
    a, b = proflib.Engine('a', args.a), proflib.Engine('b', args.b)
    results = []
    for doc in proflib.resolve_docs(args.docs.split(',')):
        runs = {'a': prepare(a, doc, 'a', args, extra), 'b': prepare(b, doc, 'b', args, extra)}
        counted = {k: [proflib.run_counted(*v) for _ in range(args.counters)] for k, v in runs.items()}
        calls = {}
        for k, v in runs.items():
            t = ktrace.trace(*v)
            calls[k] = collections.Counter(c['name'] for c in t['calls'])
        ratios = {'cycles': [], 'cycles_aa': [], 'cpu': [], 'cpu_aa': [], 'wall': [], 'wall_aa': []}
        for r in range(args.rounds):
            order = ('a', 'b', 'a2') if r % 2 else ('a2', 'b', 'a')
            got = {}
            for k in order:
                got[k] = proflib.run_counted(*runs[k[0]])
            ca, cb, ca2 = (got[x]['counters']['cycles']['value'] for x in ('a', 'b', 'a2'))
            ratios['cycles'].append(cb / ((ca + ca2) / 2))
            ratios['cycles_aa'].append(ca / ca2)
            ratios['cpu'].append(got['b']['cpu_ms'] / ((got['a']['cpu_ms'] + got['a2']['cpu_ms']) / 2))
            ratios['cpu_aa'].append(got['a']['cpu_ms'] / got['a2']['cpu_ms'])
            ratios['wall'].append(got['b']['wall_ms'] / ((got['a']['wall_ms'] + got['a2']['wall_ms']) / 2))
            ratios['wall_aa'].append(got['a']['wall_ms'] / got['a2']['wall_ms'])
        ins = {k: statistics.median(c['counters']['instructions']['value'] for c in v) for k, v in counted.items()}
        rc = {k: sorted({c['rc'] for c in v}) for k, v in counted.items()}
        med = {k: statistics.median(v) for k, v in ratios.items()} if args.rounds else {}
        total = {k: sum(v.values()) for k, v in calls.items()}
        diff = {n: calls['b'][n] - calls['a'][n] for n in sorted(set(calls['a']) | set(calls['b'])) if calls['b'][n] != calls['a'][n]}
        rec = {'doc': doc.name, 'instructions': ins, 'ratio': ins['b'] / ins['a'], 'status': rc, 'calls': total, 'calls_differ': diff, 'medians': med}
        results.append(rec)
        print(f'{doc.name}: {ins["a"] / 1e6:.3f} M to {ins["b"] / 1e6:.3f} M instructions, {rec["ratio"]:.5f}; status {rc}; {total["a"]} to {total["b"]} system calls, {diff}', flush=True)
        if med:
            print(f'  rounds {args.rounds}: cycles {med["cycles"]:.4f} ({med["cycles_aa"]:.4f}), CPU {med["cpu"]:.4f} ({med["cpu_aa"]:.4f}), wall {med["wall"]:.4f} ({med["wall_aa"]:.4f})', flush=True)
    if args.json:
        with open(args.json, 'w') as f:
            json.dump(results, f, indent=1)


if __name__ == '__main__':
    main()
