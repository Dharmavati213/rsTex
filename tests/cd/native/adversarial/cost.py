#!/usr/bin/env -S python3 -I
r"""Documents written to make the native path's check slow: the reading ahead, the library's passes and the walk over macros must never cost more than the real PGF they are there to avoid.

    python3 -I tests/cd/native/adversarial/cost.py --engine WRAPPER [--filter REGEX] [--work DIR] [--real-too]

Each document comes in two sizes, the second four times the first.  The engine is measured in user-mode instructions with the path on, and what it did with the diagrams is reported.  A line fails if the larger size costs more than 6 times the smaller, which a linear check does not, or if `--real-too` is given and the path costs more than real PGF.  Real PGF is only measured where it takes less than a few minutes.

A diagram of over 4000pt starts again with the path off, and many of these get that wide at the larger size.  Such a run costs what real PGF costs and what the path had spent before that, so it is measured against real PGF at that size, and fails if the path's part is more than real PGF's.
"""
import argparse
import os
import re
import shutil
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(os.path.dirname(os.path.dirname(os.path.dirname(HERE))))
sys.dont_write_bytecode = True
sys.path.insert(0, os.path.join(REPO, 'tools', 'prof'))
import proflib  # noqa: E402
ENV = {'SOURCE_DATE_EPOCH': '1700000000', 'FORCE_SOURCE_DATE': '1', 'TZ': 'UTC', 'LC_ALL': 'C', 'LANG': 'C', 'PATH': os.environ['PATH'], 'HOME': os.environ['HOME'], 'RSTEX_NO_SNAPSHOT': '1'}
HEAD = '\\documentclass{article}\n\\usepackage{tikz-cd}\n'


def doc(pre, body):
    return HEAD + pre + '\n\\begin{document}\n' + body + '\n\\end{document}\n'


def sparse(n):
    """Many rows and one long row: rows times columns is far more than there are cells."""
    return doc('', '\\begin{tikzcd} A ' + '\\\\ ' * n + 'B ' + '& ' * n + 'C \\end{tikzcd}')


def delimited(n):
    """A macro that looks for a delimiter far away, repeated."""
    return doc('\\def\\h#1\\stop{}', '\\begin{tikzcd} A \\arrow[r] & ' + '\\h ' * n + '\\stop' * n + ' \\end{tikzcd}')


def braces(n):
    """Arguments inside arguments: each looks through all that is inside it."""
    return doc('\\def\\h#1{#1}', '\\begin{tikzcd} A \\arrow[r] & ' + '\\h{' * min(n, 200) + 'x' + '}' * min(n, 200) + ' \\h{y}' * n + ' \\end{tikzcd}')


def names(n):
    """Long names, and many styles of the document, each of which takes the library another pass over all of it."""
    name = 'a' * 200
    styles = ''.join(f'\\tikzset{{st{chr(97 + k // 26)}{chr(97 + k % 26)}/.style={{red}}}}\n' for k in range(60))
    used = ','.join(f'st{chr(97 + k // 26)}{chr(97 + k % 26)}' for k in range(60))
    return doc(f'\\expandafter\\def\\csname {name}\\endcsname{{x}}\n' + styles, f'\\begin{{tikzcd}} A \\arrow[r,{used}] & ' + ('\\' + name + ' %\n') * n + ' \\end{tikzcd}')


def grid(n):
    """A big diagram, as plain as can be."""
    rows = []
    for r in range(n):
        rows.append(' & '.join('x \\arrow[r] \\arrow[d, "f"]' if r < n - 1 and c < n - 1 else 'x' for c in range(n)))
    return doc('', '\\begin{tikzcd}\n' + ' \\\\\n'.join(rows) + '\n\\end{tikzcd}')


def aliases(n):
    """Named nodes, and arrows to them by name."""
    cells = ' & '.join(f'|[alias=n{k}]| x' for k in range(n))
    return doc('', '\\begin{tikzcd} ' + ' \\arrow[to=n0] '.join(cells.split(' & ')[:1]) + ' & ' + ' & '.join(cells.split(' & ')[1:]) + ' \\\\ ' + ' '.join(f'\\arrow[to=n{k}]' for k in range(1, n)) + ' \\end{tikzcd}')


def walk(n):
    """Macros that mention each other twice, deep: a walk without a memory never ends."""
    defs = ''.join(f'\\expandafter\\def\\csname m{chr(97 + k % 26)}{chr(97 + k // 26)}\\endcsname{{\\csname m{chr(97 + (k + 1) % 26)}{chr(97 + (k + 1) // 26)}\\endcsname\\csname m{chr(97 + (k + 1) % 26)}{chr(97 + (k + 1) // 26)}\\endcsname}}\n' for k in range(50))
    defs += f'\\expandafter\\def\\csname m{chr(97 + 50 % 26)}{chr(97 + 50 // 26)}\\endcsname{{x}}\n'
    # A body that TeX is not asked to run: the walk still looks at it.
    return doc(defs + '\\def\\go{\\iffalse\\maa\\fi}', ''.join('\\begin{tikzcd} \\go A \\arrow[r] & B \\end{tikzcd}\n\\par\n' for _ in range(n // 100 + 1)))


def many(n):
    """Many small diagrams: what each costs must not grow with those before it."""
    return doc('', ''.join('\\begin{tikzcd} A \\arrow[r] & B \\end{tikzcd}\n\\par\n' for _ in range(n // 10 + 1)))


def declines(n):
    """Many diagrams that cannot end, from a macro, on one paragraph."""
    return doc('\\def\\d{\\begin{tikzcd} A \\\\ B}', '\\d x ' * (n // 500 + 1) + '\n\\par')


def redefined(n):
    """A macro in a cell redefined before each diagram, with a long body."""
    body = 'x' * 2000
    return doc('', ''.join(f'\\def\\m{{{body}}}\\begin{{tikzcd}} \\m \\arrow[r] & B \\end{{tikzcd}}\n\\par\n' for _ in range(n // 100 + 1)))


def appended(n):
    """A style appended to before each diagram."""
    return doc('', ''.join('\\tikzcdset{every arrow/.append style={thick}}\\begin{tikzcd} A \\arrow[r] & B \\end{tikzcd}\n\\par\n' for _ in range(n // 100 + 1)))


def onecell(n):
    """All the arrows in one cell."""
    return doc('', '\\begin{tikzcd} A ' + '\\arrow[r] ' * n + '& B \\end{tikzcd}')


def runs(n):
    """Text between all those arrows, each run looked at by itself."""
    return doc('', '\\begin{tikzcd} ' + 'x \\arrow[r] ' * n + '& B \\end{tikzcd}')


def labels(n):
    """All the labels on one arrow."""
    return doc('', '\\begin{tikzcd} A \\arrow[r' + ',"f"' * n + '] & B \\end{tikzcd}')


def longlabel(n):
    """One long label."""
    return doc('', '\\begin{tikzcd} A \\arrow[r,"' + 'x' * n + '"] & B \\end{tikzcd}')


def keys(n):
    """A long list of keys on an arrow, on a node and on the diagram."""
    many = 'thick,' * n
    return doc('', f'\\begin{{tikzcd}}[{many}] |[{many}]| A \\arrow[r,{many}] & B \\end{{tikzcd}}')


def chain(n):
    """Each style of the document made of the one before."""
    defs = '\\tikzset{s0/.style={thick}}\n' + ''.join(f'\\tikzset{{s{k + 1}/.style={{s{k}}}}}\n' for k in range(n))
    return doc(defs, f'\\begin{{tikzcd}} A \\arrow[r,s{n}] & B \\end{{tikzcd}}')


def doubling(n):
    """Each style of the document made of the one before, twice."""
    depth = 2
    while (1 << depth) < n:
        depth += 1
    defs = '\\tikzset{s0/.style={thick}}\n' + ''.join(f'\\tikzset{{s{k + 1}/.style={{s{k},s{k}}}}}\n' for k in range(depth))
    return doc(defs, f'\\begin{{tikzcd}} A \\arrow[r,s{depth}] & B \\end{{tikzcd}}')


def nest(n):
    """Groups inside groups in a cell."""
    return doc('', '\\begin{tikzcd} ' + '{' * n + 'x' + '}' * n + ' \\arrow[r] & B \\end{tikzcd}')


def distinct(n):
    """Many macros in a cell, all different."""
    def name(k):
        return ''.join(chr(97 + (k >> (4 * i)) % 16) for i in range(4))
    defs = ''.join(f'\\expandafter\\def\\csname q{name(k)}\\endcsname{{x}}\n' for k in range(n))
    return doc(defs, '\\begin{tikzcd} ' + ' '.join(f'\\q{name(k)}' for k in range(n)) + ' \\arrow[r] & B \\end{tikzcd}')


DOCS = [('sparse', sparse, 1000), ('delimited', delimited, 4000), ('braces', braces, 4000), ('names', names, 4000), ('grid', grid, 10), ('aliases', aliases, 100), ('walk', walk, 400), ('many', many, 400), ('declines', declines, 2000), ('redefined', redefined, 1000), ('appended', appended, 400),
        ('onecell', onecell, 400), ('runs', runs, 400), ('labels', labels, 400), ('longlabel', longlabel, 4000), ('keys', keys, 400), ('chain', chain, 400), ('doubling', doubling, 400), ('nest', nest, 50), ('distinct', distinct, 1000)]


def measure(engine, text, work, extra, timeout=900):
    shutil.rmtree(work, ignore_errors=True)
    os.makedirs(work)
    with open(os.path.join(work, 'x.tex'), 'w') as f:
        f.write(text)
    env = dict(ENV, XDG_CACHE_HOME=os.path.join(work, 'cache'))
    argv = ['timeout', str(timeout), engine, '-fmt=pdflatex'] + extra + ['--commute-report', '-interaction=batchmode', '-no-shell-escape', 'x.tex']
    r = proflib.run_counted(argv, work, env, events=proflib.EVENTS[:1])
    if r['rc'] == 124:
        return None, 'timeout'
    with open(os.path.join(work, 'x.log'), 'rb') as f:
        log = f.read()
    return r['counters']['instructions']['value'] / 1e6, (log.count(b'[[commute native]]'), log.count(b'[[commute fallback]]'), log.count(b'[[commute restart]]'))


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument('--engine', required=True)
    ap.add_argument('--filter')
    ap.add_argument('--work', default=os.path.join(REPO, 'target', 'native', 'adversarial-cost'))
    ap.add_argument('--real-too', action='store_true')
    args = ap.parse_args()
    args.engine = os.path.abspath(args.engine)
    bad = 0
    print('document        size   Minstr  fates          size   Minstr  fates        ratio  real at the smaller size')
    for name, make, n in DOCS:
        if args.filter and not re.search(args.filter, name):
            continue
        a, fa = measure(args.engine, make(n), os.path.join(args.work, name), [])
        b, fb = measure(args.engine, make(4 * n), os.path.join(args.work, name), [])
        real = ''
        over = False
        if args.real_too and a is not None:
            r, _ = measure(args.engine, make(n), os.path.join(args.work, name), ['--no-commute'])
            real = f'{r:.1f}' if r is not None else 'timeout'
            over = r is not None and a > r
        ratio = b / a if a and b else None
        again = b is not None and fb[2] > 0
        if again:
            r, _ = measure(args.engine, make(4 * n), os.path.join(args.work, name), ['--no-commute'])
            real = f'{r:.1f} at the larger size' if r is not None else 'timeout'
            over = over or (r is not None and b > 2 * r)
        fail = (ratio is None or ratio > 6) and not again or over
        bad += fail
        print(f'{name:<14}{n:>6} {a or 0:>8.1f}  {str(fa):<12}{4 * n:>6} {b or 0:>8.1f}  {str(fb):<12} {ratio or 0:>5.2f}  {real}  {"FAIL" if fail else ""}')
    print('cost: ' + ('FAIL' if bad else 'PASS'))
    return 1 if bad else 0


if __name__ == '__main__':
    sys.exit(main())
