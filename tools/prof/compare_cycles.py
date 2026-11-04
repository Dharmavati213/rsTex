#!/usr/bin/env -S python3 -I
"""compare_cycles.py: two builds in turn, with a control, on a busy machine: what tells them apart when the clock cannot.

    compare_cycles.py --a PATH --b PATH --doc ID [--rounds 40] [--warm] [--keep-pdf] [--last] [--a-env NAME=VALUE] [--b-env NAME=VALUE] [--json FILE]

A round runs A, B and A again, in an order that turns round each round, each under user-mode counters taken from outside the process, as bench.py has them.  What it prints for each document:
the instructions of one run of each, which are exact; the paired ratio B/A of cycles, as the median over the rounds of B over the A beside it; and that ratio for A over the other A, the control, whose distance from 1 says how far the machine can be trusted today.
And user-mode faults.  --warm restores a checkpoint: the first run writes it and leaves a seal, and each run after that is proved by RSTEX_CHECKPOINT_TRACE to have restored.  --keep-pdf has each run of B find the PDF its last run left, for the output cache of
DESIGN.md section 9, and --last has it go on from the last of a chain of page records as well.  A document is only named by its manifest id, and nothing of it is printed.
"""
import argparse
import json
import os
import re
import shutil
import statistics
import subprocess
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import proflib  # noqa: E402


def parse_env(items):
    return dict(x.split('=', 1) for x in items)


def setup(engine, doc, tag, warm, extra, keep_pdf=False, last=False):
    root = os.path.join(proflib.out_root(), 'cmp-cycles', doc.name.replace('/', '_'))
    # Of one length, as the path can show in what a run writes.
    out = os.path.join(root, 'o-' + tag)
    cache = os.path.join(root, 'cache-' + tag)

    def reset():
        pdf = os.path.join(out, doc.entry['jobname'] + '.pdf') if keep_pdf else None
        kept = None
        if pdf and os.path.exists(pdf):
            with open(pdf, 'rb') as f:
                kept = f.read()
        shutil.rmtree(out, ignore_errors=True)
        os.makedirs(out)
        if not isinstance(doc, proflib.FormatDoc):
            proflib.corpus().restore_state(doc.prepare(), out)
        if kept is not None:
            with open(pdf, 'wb') as f:
                f.write(kept)

    reset()
    argv, cwd, env = engine.command(doc, out)
    if isinstance(doc, proflib.FormatDoc):
        cwd = out
    env = dict(env, **extra)
    if warm:
        shutil.rmtree(cache, ignore_errors=True)
        os.makedirs(cache)
        del env['RSTEX_NO_SNAPSHOT']
        env['XDG_CACHE_HOME'] = cache
        env['RSTEX_CHECKPOINT_POLICY'] = 'always'
        proflib.run_timed(argv, cwd, env)
        proflib.seal_checkpoint(reset, argv, cwd, env)
        reset()
        r = subprocess.run(argv, cwd=cwd, env=dict(env, RSTEX_CHECKPOINT_TRACE='1'), stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
        if b'restored' not in r.stderr:
            sys.exit(f'{doc.name}: not restored')
        if last:
            reset()
            r = subprocess.run(argv, cwd=cwd, env=dict(env, RSTEX_CHECKPOINT_TRACE='1', RSTEX_PAGE_READONLY='1'), stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
            if not re.search(rb'pages: \d+ records, using [1-9]', r.stderr):
                return None
            env['RSTEX_PAGE_READONLY'] = '1'
    return argv, cwd, env, reset


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--a', required=True)
    ap.add_argument('--b', required=True)
    ap.add_argument('--doc', required=True, help='names, as proflib.resolve_docs takes them, separated by commas')
    ap.add_argument('--rounds', type=int, default=40)
    ap.add_argument('--warm', action='store_true')
    ap.add_argument('--keep-pdf', action='store_true')
    ap.add_argument('--last', action='store_true')
    ap.add_argument('--a-env', action='append', default=[])
    ap.add_argument('--b-env', action='append', default=[])
    ap.add_argument('--json')
    args = ap.parse_args()
    ea, eb = proflib.Engine('a', args.a), proflib.Engine('b', args.b)
    results = []
    for doc in proflib.resolve_docs(args.doc.split(',')):
        doc.prepare()
        sa = setup(ea, doc, 'a', args.warm, parse_env(args.a_env))
        sb = setup(eb, doc, 'b', args.warm, parse_env(args.b_env), args.keep_pdf or args.last, args.last)
        if sb is None:
            print(f'{doc.name}: no record', flush=True)
            continue
        rounds = []
        for i in range(args.rounds):
            seen = {}
            for k, (argv, cwd, env, reset) in ((('a1', sa), ('b', sb), ('a2', sa)) if i % 2 == 0 else (('a2', sa), ('b', sb), ('a1', sa))):
                reset()
                seen[k] = proflib.run_counted(argv, cwd, env)
            rounds.append(seen)

        def of(k, what):
            return [r[k]['counters'][what]['value'] for r in rounds]

        instr_a, instr_b = statistics.median(of('a1', 'instructions')), statistics.median(of('b', 'instructions'))
        cb, c1, c2 = of('b', 'cycles'), of('a1', 'cycles'), of('a2', 'cycles')
        ratio = statistics.median(b / ((x + y) / 2) for b, x, y in zip(cb, c1, c2))
        control = statistics.median(x / y for x, y in zip(c1, c2))
        faults_a, faults_b = statistics.median(of('a1', 'page-faults')), statistics.median(of('b', 'page-faults'))
        row = {'doc': doc.name, 'warm': args.warm, 'rounds': args.rounds, 'instr_a': instr_a, 'instr_b': instr_b, 'instr_ratio': instr_b / instr_a, 'cycles_ratio': ratio, 'control': control, 'faults_a': faults_a, 'faults_b': faults_b}
        results.append(row)
        print(f'{doc.name + (" warm" if args.warm else ""):<34} Minstr {instr_a / 1e6:>10.2f} {instr_b / 1e6:>10.2f} ratio {instr_b / instr_a:.4f}; cycles, paired over {args.rounds}: {ratio:.4f}, control A/A {control:.4f}; faults {faults_a:.0f} {faults_b:.0f}', flush=True)
    if args.json:
        with open(args.json, 'w') as f:
            json.dump(results, f, indent=1)


if __name__ == '__main__':
    main()
