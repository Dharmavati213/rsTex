#!/usr/bin/env -S python3 -I
r"""native_cost.py: what a diagram costs on the native path of DESIGN.md section 10, beyond the typesetting of its cells and labels.

    native_cost.py --engine PATH [--count 100] [--commute PATH-TO-THE-EXAMPLE]

Three documents with the same preamble: COUNT copies of a typical diagram, the same with the path off, and COUNT copies of its cells and labels typeset in boxes that are thrown away.  In user-mode
instructions, per diagram: the path against real PGF, and the path less the bare boxes.  What it prints as the library's part is the example crates/commute/examples/cost.rs, COUNT times over, against once.
Also the many declines of a body with no \end within reach, at COUNT and four times that: a document that does it must stay linear.
"""
import argparse
import os
import shutil
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import proflib  # noqa: E402

PRE = r'''\documentclass{article}
\usepackage{amsmath}
\usepackage{tikz-cd}
\begin{document}
'''
DIAGRAM = r'''\setbox0=\hbox{\begin{tikzcd}
  A \arrow[r, "f"] \arrow[d, "g"'] & B \arrow[d, "h"] \arrow[r, hook] & C \arrow[d, "k", two heads] \\
  D \arrow[r, "i"'] & E \arrow[r, Rightarrow] & F
\end{tikzcd}}
'''
BOXES = r'''\setbox0=\hbox{%s}
''' % ''.join(r'\hbox{$A$}\hbox{$B$}\hbox{$C$}\hbox{$D$}\hbox{$E$}\hbox{$F$}' + r'\hbox{$\scriptstyle %s$}' % c for c in 'fghki')
DECLINE = r'''\begin{mycd} A \arrow[r] & B \end{mycd}
'''


def count(engine_path, work, tex, env, n=3):
    os.makedirs(work, exist_ok=True)
    path = os.path.join(work, 'x.tex')
    with open(path, 'w') as f:
        f.write(tex)
    best = None
    for _ in range(n):
        argv = [engine_path, '-progname=pdflatex', '-interaction=batchmode', '-output-directory=' + work, path]
        r = proflib.run_counted(argv, work, env)
        v = r['counters']['instructions']['value']
        best = v if best is None else min(best, v)
    return best


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument('--engine', required=True)
    ap.add_argument('--count', type=int, default=100)
    ap.add_argument('--commute')
    args = ap.parse_args()
    root = os.path.join(proflib.out_root(), 'native-cost')
    shutil.rmtree(root, ignore_errors=True)
    os.makedirs(root)
    env = {'PATH': os.environ['PATH'], 'HOME': os.environ['HOME'], 'RSTEX_NO_SNAPSHOT': '1', 'XDG_CACHE_HOME': os.path.join(proflib.out_root(), 'native-cost-cache'), 'SOURCE_DATE_EPOCH': '1700000000', 'FORCE_SOURCE_DATE': '1'}
    off = dict(env, RSTEX_NO_COMMUTE='1')
    n = args.count
    end = '\\end{document}\n'
    # The first use of each font and of the tables is part of no diagram: the same diagram once, ahead of all three.
    first = PRE + DIAGRAM
    base = count(args.engine, os.path.join(root, 'base'), first + end, env)
    native = count(args.engine, os.path.join(root, 'native'), first + DIAGRAM * n + end, env)
    boxes = count(args.engine, os.path.join(root, 'boxes'), first + BOXES * n + end, env)
    base_off = count(args.engine, os.path.join(root, 'base-off'), first + end, off)
    real = count(args.engine, os.path.join(root, 'real'), first + DIAGRAM * 5 + end, off, 1)
    print(f'{n} diagrams of 6 cells, 6 arrows and 5 labels, per diagram:')
    print(f'  real PGF                    {(real - base_off) / 5 / 1e6:8.3f} M instructions')
    print(f'  native path                 {(native - base) / n / 1e6:8.3f} M')
    print(f'  its cells and labels alone  {(boxes - base) / n / 1e6:8.3f} M')
    print(f'  the path beyond those       {(native - boxes) / n / 1e6:8.3f} M')
    if args.commute:
        one = proflib.run_counted([args.commute, '1'], root, env)['counters']['instructions']['value']
        many = proflib.run_counted([args.commute, str(n + 1)], root, env)['counters']['instructions']['value']
        print(f'  the library\'s part          {(many - one) / n / 1e6:8.3f} M')
    wrapper = PRE.replace(r'\begin{document}', r'\newenvironment{mycd}{\begin{tikzcd}}{\end{tikzcd}}' + '\n\\begin{document}')
    lo = count(args.engine, os.path.join(root, 'decl1'), wrapper + DECLINE * n + end, env, 1)
    hi = count(args.engine, os.path.join(root, 'decl4'), wrapper + DECLINE * (4 * n) + end, env, 1)
    print(f'declines through a wrapper, per diagram at {n} and at {4 * n}: {lo / n / 1e6:.3f} and {hi / (4 * n) / 1e6:.3f} M (real PGF runs in all of them)')
    return 0


if __name__ == '__main__':
    sys.exit(main())
