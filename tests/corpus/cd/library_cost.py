#!/usr/bin/env -S python3 -I
r"""The cost of one parse + layout + PDF cycle of the commute library, in user-mode instructions, from outside the process.

    cargo bench -p commute --bench cycle --no-run
    python3 -I tests/corpus/cd/library_cost.py target/release/deps/cycle-HASH [--limit-square 200000]

Runs crates/commute/benches/cycle.rs for N and 2N cycles, so that what the difference divided by N shows has no start-up or first diagram in it.
"""
import argparse
import os
import sys

REPO = os.path.dirname(os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__)))))
sys.path.insert(0, os.path.join(REPO, 'tools', 'prof'))
import proflib  # noqa: E402


def instructions(tool, which, n):
    r = proflib.run_counted([tool, which, str(n)], REPO, dict(os.environ))
    if r.get('rc', 0):
        raise SystemExit(f'{tool} {which} {n}: {r}')
    return r['counters']['instructions']['value']


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument('tool')
    ap.add_argument('--limit-square', type=float, default=200000)
    args = ap.parse_args()
    bad = 0
    for which, n in (('square', 20000), ('10', 2000), ('40', 100)):
        a = instructions(args.tool, which, n)
        b = instructions(args.tool, which, 2 * n)
        per = (b - a) / n
        name = 'square' if which == 'square' else f'{which}x{which} grid'
        print(f'{name}: {per:,.0f} instructions per cycle')
        if which == 'square' and per > args.limit_square:
            print(f'  above the limit of {args.limit_square:,.0f}')
            bad = 1
    return bad


if __name__ == '__main__':
    sys.exit(main())
