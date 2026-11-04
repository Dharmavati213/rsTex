#!/usr/bin/env -S python3 -I
"""The output cache against no cache, in rounds, with a control: for a machine whose clock cannot be trusted.

    paired.py BINARY DOCS [-n ROUNDS] [--base PATH]

In each round, in an order that turns: `none`, RSTEX_NO_OUTCACHE=1; `none2`, the same again in a directory of its own, whose ratio to `none` is the noise; `hits`, with the last PDF in its place.  All three
restore a sealed checkpoint.  --base adds `base`, another binary with no cache, as a fourth.  Printed: for each counter the median of the ratios of the rounds, each to `none` of its own round, and the
median values.  Instructions are exact; cycles are of this load.  A document is named by its id and nothing of it is printed.
"""
import argparse
import os
import shutil
import statistics
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
import proflib  # noqa: E402

KEYS = ('instr', 'cycles', 'cpu_ms', 'wall_ms', 'minflt', 'maxrss_kb')


class Side:
    def __init__(self, engine, doc, root, label, extra):
        self.engine, self.doc = engine, doc
        self.work = os.path.join(root, label, 'o')
        self.cache = os.path.join(root, label, 'cache')
        os.makedirs(self.work)
        os.makedirs(self.cache)
        self.env = proflib.corpus().entry_env(doc.entry)
        self.env.update(XDG_CACHE_HOME=self.cache, RSTEX_CHECKPOINT_POLICY='always')
        self.env.update(extra)
        self.rows = []

    def side_files(self):
        keep = self.doc.entry['jobname'] + '.pdf'
        for x in os.listdir(self.work):
            if x != keep:
                p = os.path.join(self.work, x)
                shutil.rmtree(p) if os.path.isdir(p) else os.remove(p)
        proflib.corpus().restore_state(self.doc.prepare(), self.work)

    def run(self, counted):
        self.side_files()
        argv, cwd, _ = self.engine.command(self.doc, self.work)
        return proflib.run_counted(argv, cwd, self.env) if counted else proflib.run_timed(argv, cwd, self.env)

    def round(self):
        r = self.run(True)
        r['instr'] = r['counters']['instructions']['value'] / 1e6
        r['cycles'] = r['counters']['cycles']['value'] / 1e6
        self.rows.append(r)


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument('binary')
    ap.add_argument('docs')
    ap.add_argument('-n', type=int, default=41)
    ap.add_argument('--base')
    args = ap.parse_args()
    engine = proflib.Engine('x', args.binary)
    for doc in proflib.resolve_docs(args.docs.split(',')):
        doc.prepare()
        root = os.path.join(proflib.out_root(), 'paired', doc.name.replace('/', '_'))
        shutil.rmtree(root, ignore_errors=True)
        sides = [Side(engine, doc, root, 'none', {'RSTEX_NO_OUTCACHE': '1'}), Side(engine, doc, root, 'none2', {'RSTEX_NO_OUTCACHE': '1'}), Side(engine, doc, root, 'hits', {})]
        names = ['none', 'none2', 'hits']
        if args.base:
            sides.append(Side(proflib.Engine('base', args.base), doc, root, 'base', {}))
            names.append('base')
        # The checkpoint, its seal, and the index with the PDF that goes with it.
        for s in sides:
            s.run(False)
        time.sleep(2.2)
        for s in sides:
            s.run(False)
            s.run(False)
        for i in range(args.n):
            for k in range(len(sides)):
                sides[(i + k) % len(sides)].round()
        print(f'{doc.name} ({doc.id}): {args.n} rounds, load {os.getloadavg()[0]:.0f}', flush=True)
        for key in KEYS:
            cells = []
            for name, s in zip(names, sides):
                ratio = statistics.median(a[key] / b[key] if b[key] else 1 for a, b in zip(s.rows, sides[0].rows))
                cells.append(f'{name} {statistics.median(r[key] for r in s.rows):>10.2f} ({ratio:.3f})')
            print(f'  {key:<10} ' + '  '.join(cells), flush=True)


if __name__ == '__main__':
    main()
