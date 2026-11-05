#!/usr/bin/env -S python3 -I
r"""Writes 18-key-pairs.tex beside it: the arrow keys two at a time.  A key alone is what 05-arrows.tex and the library's tests/cd have, and what they had of `loop` and of `xshift` was right apart, and wrong together.

    python3 -I tests/cd/native/adversarial/gen_key_pairs.py

Each document holds 50 diagrams, as the keys are many and a diagram of real PGF costs little beside loading it.  So a failure names a document, and its diagram is found from there.
"""
import itertools
import os

HERE = os.path.dirname(os.path.abspath(__file__))
PER_DOCUMENT = 50
CURVE = ['loop', 'loop right', 'loop above', 'loop left', 'loop below', 'bend left', 'bend right=20', 'in=30', 'out=120', 'looseness=2', 'relative', 'distance=20pt', 'min distance=9pt', 'max distance=5pt',
         'in distance=7pt', 'out distance=8pt', 'in looseness=2', 'out looseness=3']
SHIFT = ['xshift=1ex', 'yshift=3pt', 'shift={(1pt,2pt)}', 'shift left', 'shift left=2', 'shift right=3pt', 'transform nodes={xshift=2pt}']
ANCHOR = ['start anchor=north', 'end anchor=south', 'start anchor=20', 'end anchor=-20', 'start anchor={[xshift=1ex]north}', 'end anchor={[yshift=1ex]south}', 'start anchor={[xshift=2pt]}', 'end anchor={[xshift=2pt]}']
TIPS = ['hook', 'two heads', 'tail', 'maps to', 'dash', 'dashed', 'Rightarrow', 'equal', 'squiggly', 'no head', 'leftarrow']
LOOK = ['thick', 'red', 'draw=blue', 'line width=1pt', 'shorten >=2pt', 'shorten <=3pt', 'crossing over', 'phantom', 'overlay', 'transform canvas={xshift=1pt}', 'double', 'dotted']
LABEL = ['"f"', '"f"\'', '"f" description', '"f" near start', '"f" {pos=0.2}', '"f" {xshift=1pt}', '"f" sloped', '"f" {rotate=30}', '"f" marking', '"f" {inner sep=1pt}', 'near end', 'swap']
PAIRS = [(CURVE, SHIFT), (CURVE, ANCHOR), (CURVE, CURVE), (SHIFT, ANCHOR), (SHIFT, SHIFT), (ANCHOR, ANCHOR), (CURVE, TIPS), (CURVE, LOOK), (CURVE, LABEL), (SHIFT, TIPS), (SHIFT, LOOK), (SHIFT, LABEL),
         (ANCHOR, TIPS), (ANCHOR, LOOK), (ANCHOR, LABEL)]


def arrows():
    out = []
    for a, b in PAIRS:
        for x, y in itertools.product(a, b):
            for target in ('', 'r'):
                for first, second in ((x, y), (y, x)):
                    out.append(','.join(([target] if target else []) + [first, second]))
    return list(dict.fromkeys(out))


def main():
    keys = arrows()
    with open(os.path.join(HERE, '18-key-pairs.tex'), 'w', encoding='latin-1') as f:
        f.write('% The arrow keys two at a time, from gen_key_pairs.py.\n\\documentclass{article}\n\\usepackage{amsmath,amssymb}\n\\usepackage{tikz-cd}\n\\usetikzlibrary{decorations.pathmorphing}\n\\pdfpagewidth=5in \\pdfpageheight=5in\n')
        for n in range(0, len(keys), PER_DOCUMENT):
            f.write(f'%%%% case pairs-{n // PER_DOCUMENT}\n')
            for k in keys[n:n + PER_DOCUMENT]:
                f.write('\\begin{tikzcd} A \\arrow[%s] & Bx \\\\ C & D_1 \\end{tikzcd}\n\n' % k)
    print(f'{len(keys)} diagrams in {(len(keys) + PER_DOCUMENT - 1) // PER_DOCUMENT} documents')


if __name__ == '__main__':
    main()
