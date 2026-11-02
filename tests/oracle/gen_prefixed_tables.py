#!/usr/bin/env python3
"""Expected results for prefixed.rs's unit tests, taken from the oracle.

Run with python3 -I.  It writes crates/engine/testdata/prefixed_oracle.tsv: a line of TeX, then the lines of the
oracle's log that begin with "> ", "! " or "### " for it, in `pdftex -ini -etex`, all lines in one document, in order.

Only what runs before the rest of the engine exists is here: \\let, \\futurelet, the prefix loop and its errors, the
errors of \\advance before a number is read, get_r_token, \\show and \\showifs.  What needs a scanner is in
tests/integration/prefixed/.

Format: one case per line, fields separated by a tab.
"""
import os
import re
import subprocess
import sys
import tempfile

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
OUT = os.path.join(ROOT, "crates", "engine", "testdata", "prefixed_oracle.tsv")

PREAMBLE = r"""\catcode`\{=1 \catcode`\}=2 \catcode`\#=6 \catcode`\^=7 \catcode`\_=8
\catcode`\&=4 \catcode`\$=3 \catcode`\~=13
\scrollmode \endlinechar=-1 \errorcontextlines=0
"""

CASES = [
    # \let, its spaces and its equals sign
    r"\let\a=b\show\a",
    r"\let\a b\show\a",
    r"\let\a= b\show\a",
    r"\let\a  =  b\show\a",
    r"\let\a==\show\a",
    r"\let\a= =\show\a",
    r"\let\a=  =\show\a",
    r"\let\a\relax\show\a",
    r"\let\a=\relax\show\a",
    r"\let\newone=\let\show\newone",
    r"\newone\a=c\show\a",
    r"\let\a=\undefinedone\show\a",
    r"\let\a=\a\show\a",
    r"\let\other=\other\show\other",
    # categories
    r"\let\a={\show\a",
    r"\let\a=}\show\a",
    r"\let\a=$\show\a",
    r"\let\a=&\show\a",
    r"\let\a=#\show\a",
    r"\let\a=^\show\a",
    r"\let\a=_\show\a",
    r"\let\a=~\show\a",
    r"\let\a=1\show\a",
    # primitives
    r"\let\a=\count\show\a",
    r"\let\a=\toks\show\a",
    r"\let\a=\long\show\a",
    r"\let\a=\end\show\a",
    # \futurelet: the two tokens go back in order
    r"\futurelet\a\relax b\show\a",
    r"\futurelet\a\relax=\show\a",
    r"\futurelet\a\relax\relax\show\a",
    r"\futurelet\a\show\relax",
    r"\futurelet\a\show\undefinedtwo\show\a",
    # the prefix loop and WEB 1391
    r"\long\let\a=d\show\a",
    r"\outer\let\a=e\show\a",
    r"\global\let\a=f\show\a",
    r"\protected\let\a=g\show\a",
    r"\long\long\outer\outer\global\global\protected\protected\let\a=h\show\a",
    r"\long \relax  \relax\let\a=i\show\a",
    r"\global\relax\futurelet\a\relax j\show\a",
    # WEB 1390: the command goes back and is read again
    r"\long\hrule",
    r"\outer\par",
    r"\global k",
    r"\protected{",
    r"\long\outer\global}",
    r"\global\show\a",
    r"\long\showifs",
    # \advance before a number is read, WEB 1415
    r"\advance x",
    r"\multiply\relax",
    r"\divide\hbox",
    r"\advance\par",
    r"\advance 1",
    r"\multiply{",
    r"\divide\end",
    r"\advance\toks",
    # get_r_token: the token goes back if it is not a control sequence
    r"\let 1=m\show\newinaccessible",
    r"\let=n",
    r"\let{=o",
    r"\futurelet 2\relax p",
    r"\let   \a=q\show\a",
    # \show of each kind
    r"\show a",
    r"\show 1",
    r"\show{",
    r"\show}",
    r"\show$",
    r"\show&",
    r"\show#",
    r"\show^",
    r"\show_",
    r"\show~",
    r"\show\ ",
    r"\show\.",
    r"\show\relax",
    r"\show\show",
    r"\show\undefinedthree",
    # \showifs with nothing open
    r"\showifs",
    # the interaction commands, in the middle of what is shown
    r"\nonstopmode\show c\scrollmode",
    r"\long\scrollmode",
    r"\global\nonstopmode\scrollmode",
]


def main():
    with tempfile.TemporaryDirectory() as tmp:
        body = [PREAMBLE]
        for k, text in enumerate(CASES):
            body.append("\\immediate\\write16{CASE:%d:}" % k)
            body.append(text)
        body.append("\\immediate\\write16{CASE:%d:}" % len(CASES))
        body.append("\\end")
        path = os.path.join(tmp, "t.tex")
        with open(path, "w", encoding="latin-1") as f:
            f.write("\n".join(body) + "\n")
        subprocess.run(["pdftex", "-ini", "-etex", "-interaction=nonstopmode", "t.tex"], cwd=tmp,
                       stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, check=False)
        with open(os.path.join(tmp, "t.log"), encoding="latin-1") as f:
            log = f.read()
    parts = re.split(r"^CASE:(\d+):$", log, flags=re.M)
    if [int(x) for x in parts[1::2]] != list(range(len(CASES) + 1)):
        sys.exit("markers out of order: %s\n%s" % (parts[1::2], log[-2000:]))
    with open(OUT, "w", encoding="latin-1") as f:
        for k, text in enumerate(CASES):
            lines = [l for l in parts[2 * k + 2].split("\n") if l.startswith(("> ", "! ", "### "))]
            f.write("\t".join([text] + lines) + "\n")
    print("prefixed:", len(CASES), "cases")


if __name__ == "__main__":
    main()
