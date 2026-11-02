#!/usr/bin/env python3
"""Expected results for the unit tests of mathbuild.rs, taken from the oracle.

Run with python3 -I.  It writes, under crates/engine/testdata/:

  mathbuild_shape_oracle.tsv   \\hsize, \\hangindent, \\hangafter, \\prevgraf and \\parshape in scaled points,
                               then the \\displaywidth, \\displayindent and \\predisplaysize of a display that
                               starts on an empty paragraph, in a \\vbox: section 1327 of init_math

Format: one case per line, fields separated by a tab.  A \\parshape is `indent length` pairs joined by `;`,
or `-` for none.  The test sets up the same state and calls init_math on the Rust side.
"""
import os
import re
import subprocess
import sys
import tempfile

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
OUT = os.path.join(ROOT, "crates", "engine", "testdata")

PREAMBLE = r"""\catcode`\{=1 \catcode`\}=2 \catcode`\#=6 \catcode`\^=7 \catcode`\_=8
\catcode`\&=4 \catcode`\$=3 \catcode`\~=13
\scrollmode \tracingonline=0 \newlinechar=-1
\font\x=cmsy10 \textfont2=\x \scriptfont2=\x \scriptscriptfont2=\x
\font\y=cmex10 \textfont3=\y \scriptfont3=\y \scriptscriptfont3=\y
"""

PT = 65536
MAX = 2147483647
MAXD = 1073741823
H = 100 * PT
I = 10 * PT

# (hsize, hangindent, hangafter, prevgraf, parshape)
CASES = [(H, 0, 1, 0, None)]
for ha in (0, 1, 2, 3, 4, -1, -2, -3):
    for pg in (0, 1, 2):
        CASES.append((H, I, ha, pg, None))
        CASES.append((H, -I, ha, pg, None))
CASES += [
    (H, 0, 5, 0, None), (H, 2 * H, 0, 0, None), (H, -2 * H, 0, 0, None),
    (MAXD, MAXD, 0, 0, None), (MAXD, -MAXD, 0, 0, None), (-MAXD, MAXD, 0, 0, None), (-MAXD, -MAXD, 0, 0, None),
    # None of these sums wraps on the oracle.
    (H, 0, 1, MAX, None), (H, I, 0, MAX, None), (H, I, MAX, MAX, None), (H, I, -MAX, MAX, None),
    (H, I, MAX, MAX - 1, None), (H, I, MAX, MAX - 2, None), (H, I, MAX, MAX - 3, None), (H, I, -MAX, MAX - 1, None),
    (H, I, MAX, 0, None), (H, I, -MAX, 0, None), (H, I, 1 - MAX, MAX - 1, None),
]
SHAPE3 = [(1 * PT, 11 * PT), (2 * PT, 12 * PT), (3 * PT, 13 * PT)]
for pg in (0, 1, 2, 3, 9, MAX - 1, MAX):
    CASES.append((H, 0, 1, pg, SHAPE3))
CASES += [(H, 0, 1, 0, SHAPE3[:1]), (H, 0, 1, 5, SHAPE3[:1]), (H, I, 0, 0, SHAPE3[:2]), (H, I, 0, 1, SHAPE3[:2])]


def shape_field(s):
    return "-" if s is None else ";".join("%d %d" % p for p in s)


def main():
    body = []
    for hs, hi, ha, pg, shape in CASES:
        ps = "" if shape is None else "\\parshape=%d %s " % (len(shape), " ".join("%dsp %dsp" % p for p in shape))
        # Inside the box: its start clears the shape, by normal_paragraph.  After \noindent: new_graf clears \prevgraf.
        body.append("\\hsize=%dsp \\setbox1\\vbox{\\hangindent=%dsp \\hangafter=%d %s\\noindent\\prevgraf=%d $$"
                    "\\immediate\\write16{ROW:\\number\\displaywidth:\\number\\displayindent:\\number\\predisplaysize}$$}"
                    % (hs, hi, ha, ps, pg))
    with tempfile.TemporaryDirectory() as tmp:
        with open(os.path.join(tmp, "t.tex"), "w", encoding="latin-1") as f:
            f.write(PREAMBLE + "\n".join(body) + "\n\\end\n")
        subprocess.run(["pdftex", "-ini", "-interaction=nonstopmode", "t.tex"], cwd=tmp, stdin=subprocess.DEVNULL,
                       stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, check=False)
        with open(os.path.join(tmp, "t.log"), encoding="latin-1") as f:
            log = f.read()
    rows = re.findall(r"^ROW:(-?\d+):(-?\d+):(-?\d+)$", log, re.M)
    if len(rows) != len(CASES):
        sys.exit("%d results for %d cases" % (len(rows), len(CASES)))
    if "\n! " in log:
        sys.exit("an error in the oracle's log")
    with open(os.path.join(OUT, "mathbuild_shape_oracle.tsv"), "w") as f:
        for (hs, hi, ha, pg, shape), r in zip(CASES, rows):
            f.write("\t".join([str(hs), str(hi), str(ha), str(pg), shape_field(shape)] + list(r)) + "\n")
    print("shape:", len(CASES), "cases")


if __name__ == "__main__":
    main()
