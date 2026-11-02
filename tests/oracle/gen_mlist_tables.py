#!/usr/bin/env python3
"""Expected results for mlist.rs's math_glue and math_kern, taken from the oracle.

Run with python3 -I.  It writes crates/engine/testdata/mlist_mu_oracle.tsv, with one case per line:

    quad  width  stretch  stretch_order  shrink  shrink_order  kern  glue_text  kern_text

The first seven fields are numbers: \\fontdimen6 of the font in family 2, and the three components of a \\mskip and the
width of a \\mkern, all in sp.  The last two are what the oracle's \\showbox prints after \\glue and \\kern, for
that formula at text size.  print_scaled's five decimals determine a value in sp, so these are exact.

Everything goes through the same scanning as a document does: the mu values are written with all their decimals,
which round_decimals reads back to the same sp.
"""
import os
import re
import subprocess
import tempfile

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
OUT = os.path.join(ROOT, "crates", "engine", "testdata", "mlist_mu_oracle.tsv")

PREAMBLE = r"""\catcode`\{=1 \catcode`\}=2 \catcode`\$=3
\scrollmode \tracingonline=0 \showboxbreadth=100 \showboxdepth=100
\font\sy=cmsy10 \font\ex=cmex10 \font\rm=cmr10
\textfont0=\rm \scriptfont0=\rm \scriptscriptfont0=\rm
\textfont1=\rm \scriptfont1=\rm \scriptscriptfont1=\rm
\textfont2=\sy \scriptfont2=\sy \scriptscriptfont2=\sy
\textfont3=\ex \scriptfont3=\ex \scriptscriptfont3=\ex
"""

PT = 65536

QUADS = [
    10 * PT, 18 * PT, 18 * PT - 1, 18 * PT + 1, 1, 17, 18, 19, 1000, 100 * PT, 16383 * PT,
    -10 * PT, -18 * PT, -18 * PT + 1, -18 * PT - 1, -1, -17, -18, -19, -1000, -100 * PT, -16383 * PT, 0,
]

# (width, stretch, stretch_order, shrink, shrink_order, kern)
GLUES = [
    (PT, 0, 0, 0, 0, PT),
    (18 * PT, 2 * PT, 0, 3 * PT, 0, 18 * PT),
    (-PT, -PT, 0, -PT, 0, -PT),
    (PT // 2, PT // 3, 0, PT // 7, 0, PT // 2),
    (1, 1, 0, 1, 0, 1),
    (-1, -1, 0, -1, 0, -1),
    (16383 * PT, 16383 * PT, 0, 16383 * PT, 0, 16383 * PT),
    (-16383 * PT, 1, 0, -1, 0, -16383 * PT),
    (3 * PT, 2 * PT, 1, 3 * PT, 2, 3 * PT),
    (3 * PT, 2 * PT, 3, 3 * PT, 1, 0),
    (3 * PT, -PT // 2, 2, -7, 3, 1),
    (0, 0, 0, 0, 0, 0),
]


def number(sp):
    """A decimal that TeX scans back to exactly sp/65536: every 16-bit fraction ends within 16 digits."""
    s = "-" if sp < 0 else ""
    sp = abs(sp)
    return "%s%d.%016d" % (s, sp // PT, (sp % PT) * 10**16 // PT)


def unit(order):
    return ["mu", "fil", "fill", "filll"][order]


def rows():
    return [(q,) + g for q in QUADS for g in GLUES]


def source(row):
    q, w, st, sto, sh, sho, k = row
    return (
        r"\fontdimen6\sy=%dsp\setbox1=\hbox{$\mskip %smu plus %s%s minus %s%s\mkern %smu$}\showbox1 "
        % (q, number(w), number(st), unit(sto), number(sh), unit(sho), number(k))
        + "\n"
    )


def run():
    all_rows = rows()
    with tempfile.TemporaryDirectory() as d:
        with open(os.path.join(d, "t.tex"), "w") as f:
            f.write(PREAMBLE)
            for r in all_rows:
                f.write(source(r))
            f.write("\\end\n")
        subprocess.run(
            ["pdftex", "-ini", "-interaction=nonstopmode", "t.tex"],
            cwd=d,
            stdout=subprocess.DEVNULL,
            check=False,
        )
        with open(os.path.join(d, "t.log"), encoding="latin1") as f:
            log = f.read()
    glues = re.findall(r"^\.\\glue ?(.*)$", log, re.M)
    kerns = re.findall(r"^\.\\kern ?(.*)$", log, re.M)
    assert len(glues) == len(all_rows) and len(kerns) == len(all_rows), (len(glues), len(kerns), len(all_rows))
    out = []
    for r, g, k in zip(all_rows, glues, kerns):
        out.append("\t".join(str(x) for x in r) + "\t" + g + "\t" + k)
    return out


def main():
    lines = run()
    with open(OUT, "w") as f:
        f.write("\n".join(lines) + "\n")
    print("%s: %d rows" % (OUT, len(lines)))


if __name__ == "__main__":
    main()
