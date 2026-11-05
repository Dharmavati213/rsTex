#!/usr/bin/env -S python3 -I
r"""stable_tools.py --engine PATH: --until-stable with the real bibtex and makeindex of this system's TeX Live: the lines on standard error and the exit status, which an editor reads, as they are on every system, and the PDF.

Each step here is what stable.rs says it does, as text to compare on any system.  A pass that gets to the end says passes=N result=stable and has status 0; one that needs a program says tool=... and has status 10; one that stops at the limit
has status 11; an error has TeX's status.  The new passes are new processes on Windows, whose exit status has to come back up through each of them.  The PDFs' sums go in sums-stable.txt, for `run.py --compare`.
"""
import argparse
import os
import re
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import common as c  # noqa: E402

BIB = "@book{alpha, author={A. Author}, title={Alpha}, year={2001}, publisher={P}}\n@book{beta, author={B. Author}, title={Beta}, year={2002}, publisher={P}}\n"
BIBTEX = "\\documentclass{article}\n\\pdfinfoomitdate=1 \\pdftrailerid{}\n\\begin{document}\n\\section{One}\\label{one}\nWords, and a citation \\cite{alpha}.\n\nMARK\n\n\\bibliographystyle{plain}\n\\bibliography{refs}\n\\end{document}\n"
INDEX = "\\documentclass{article}\n\\pdfinfoomitdate=1 \\pdftrailerid{}\n\\usepackage{makeidx}\n\\makeindex\n\\begin{document}\nWords\\index{word}, and more\\index{more}.\n\nMARK\n\n\\printindex\n\\end{document}\n"
SETTLES = "\\documentclass{article}\n\\pdfinfoomitdate=1 \\pdftrailerid{}\n\\begin{document}\n\\tableofcontents\n\\section{One}\\label{one}\nSee \\ref{two}.\n\\newpage\n\\section{Two}\\label{two}\nSee \\ref{one}.\n\\end{document}\n"
NEVER = "\\documentclass{article}\n\\newwrite\\f\n\\newcount\\n\n\\IfFileExists{\\jobname.cnt}{\\input{\\jobname.cnt}}{\\gdef\\cntval{0}}\n\\n=\\cntval\\relax\\advance\\n by 1\n\\immediate\\openout\\f=\\jobname.cnt\n\\immediate\\write\\f{\\string\\gdef\\string\\cntval{\\the\\n}}\n\\immediate\\closeout\\f\n\\begin{document}\nRun \\the\\n.\n\\end{document}\n"
ERROR = "\\documentclass{article}\n\\begin{document}\n\\undefinedname\nWords.\n\\end{document}\n"


class Doc:
    def __init__(self, engine, root, name, text, files=None):
        self.engine = engine
        self.name = name
        self.work = c.fresh(os.path.join(root, name))
        self.out = os.path.join(self.work, "o")
        os.makedirs(self.out)
        self.env = c.base_env(os.path.join(root, "cache"))
        c.write(os.path.join(self.work, "doc.tex"), text)
        for k, v in (files or {}).items():
            c.write(os.path.join(self.work, k), v)

    def build(self, *extra):
        """A build: what it said, by lines of its own, and its status."""
        r = c.run([self.engine, "-progname=pdflatex", *c.EDITOR, *extra, "-output-directory=o", "./doc.tex"], self.work, self.env)
        lines = [s.rstrip("\r") for s in r[2].decode("utf-8", "replace").split("\n") if s.startswith("rstex: until-stable: ")]
        return lines, r[0]

    def tool(self, program, *args):
        env = dict(self.env)
        for var in ("BIBINPUTS", "BSTINPUTS"):
            env[var] = self.work + os.pathsep
        # bibtex in the output directory, as latexmk runs it: openout_any is paranoid, and the file is beside it.
        return c.run([c.tool(program), *args], self.out, env)


def step(d, what, want_status, want_lines):
    lines, status = d.build()
    # A pass's own line changes by what changed; each is compared as far as it is the same on every system, and a name is always with a slash.
    shown = [re.sub(r" changed=\d+", " changed=N", s[len("rstex: until-stable: "):]) for s in lines]
    return c.say(f"{d.name}: {what}", status == want_status and shown == want_lines, f"status {status}, said {shown}" + ("" if status == want_status and shown == want_lines else f"; expected {want_status}, {want_lines}"))


def bibtex(engine, root, sums):
    d = Doc(engine, root, "bibtex", BIBTEX, {"refs.bib": BIB})
    # With no .bbl there is a program to run, after the pass that wrote the .aux.
    step(d, "the first build needs the program", 10, ["tool=bibtex reason=output-missing file=o/doc.aux", "passes=1 result=tool"])
    r = d.tool("bibtex", "doc")
    c.say("bibtex: the program ran", r[0] == 0, f"status {r[0]}, {r[1][-200:]!r}")
    step(d, "the next build settles", 0, ["pass=1 changed=N file=o/doc.aux", "passes=2 result=stable"])
    step(d, "and again, nothing to do", 0, ["passes=1 result=stable"])
    sums.append(("bibtex", c.read(os.path.join(d.out, "doc.pdf"))))
    # A new citation: the same program again.
    c.write(os.path.join(d.work, "doc.tex"), BIBTEX.replace("MARK", "Another \\cite{beta}."))
    step(d, "a new citation needs the program", 10, ["tool=bibtex reason=citations-changed file=o/doc.aux", "passes=1 result=tool"])
    d.tool("bibtex", "doc")
    lines, status = d.build()
    c.say("bibtex: and settles", status == 0 and lines and lines[-1].endswith("result=stable"), f"status {status}, {lines}")
    # A change of the .bib file, before any pass.
    c.write(os.path.join(d.work, "refs.bib"), BIB.replace("Alpha", "Alpha, again"))
    step(d, "a change of the .bib file comes first", 10, ["tool=bibtex reason=source-changed file=o/doc.aux", "passes=0 result=tool"])
    d.tool("bibtex", "doc")
    lines, status = d.build()
    c.say("bibtex: and settles after that", status == 0 and lines and lines[-1].endswith("result=stable"), f"status {status}, {lines}")
    sums.append(("bibtex-after-edits", c.read(os.path.join(d.out, "doc.pdf"))))
    # The record is the system's own file, and a name in it has to read back.
    c.say("bibtex: the record", os.path.exists(os.path.join(d.out, "doc.rstex-tools")), str(sorted(os.listdir(d.out))))


def makeindex(engine, root, sums):
    d = Doc(engine, root, "makeindex", INDEX)
    step(d, "the first build needs the program", 10, ["tool=makeindex reason=output-missing file=o/doc.idx", "passes=1 result=tool"])
    r = d.tool("makeindex", "-q", "doc.idx")
    c.say("makeindex: the program ran", r[0] == 0, f"status {r[0]}, {r[2][-200:]!r}")
    lines, status = d.build()
    c.say("makeindex: the next build settles", status == 0 and lines and lines[-1].endswith("result=stable"), f"status {status}, {lines}")
    step(d, "and again, nothing to do", 0, ["passes=1 result=stable"])
    sums.append(("makeindex", c.read(os.path.join(d.out, "doc.pdf"))))
    c.write(os.path.join(d.work, "doc.tex"), INDEX.replace("MARK", "Another\\index{another}."))
    step(d, "a new entry needs the program", 10, ["tool=makeindex reason=idx-changed file=o/doc.idx", "passes=1 result=tool"])


def plain_ones(engine, root, sums):
    d = Doc(engine, root, "settles", SETTLES)
    step(d, "two passes", 0, ["pass=1 changed=N file=o/doc.aux", "passes=2 result=stable"])
    step(d, "and then one", 0, ["passes=1 result=stable"])
    sums.append(("settles", c.read(os.path.join(d.out, "doc.pdf"))))
    # The limit, whose status comes up through every process that waited for the next.
    d = Doc(engine, root, "never", NEVER)
    d.build()
    c.write(os.path.join(d.work, "doc.tex"), NEVER + "% saved\n")
    lines, status = d.build("--max-passes=3")
    c.say("a document that never settles: the limit", status == 11 and lines[-1:] == ["rstex: until-stable: passes=3 result=limit file=o/doc.cnt"], f"status {status}, {lines}")
    # An error.
    d = Doc(engine, root, "error", ERROR)
    lines, status = d.build()
    c.say("a document with an error: TeX's status", status == 1 and lines[-1:] == ["rstex: until-stable: passes=1 result=error"], f"status {status}, {lines}")


def main():
    c.start()
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--engine", required=True)
    ap.add_argument("--label", default="here")
    a = ap.parse_args()
    engine = c.program(a.engine)
    root = c.fresh(os.path.join(c.OUT, a.label, "stable"))
    sums = []
    c.guarded("bibtex", bibtex, engine, root, sums)
    c.guarded("makeindex", makeindex, engine, root, sums)
    c.guarded("plain", plain_ones, engine, root, sums)
    with open(os.path.join(c.OUT, a.label, "sums-stable.txt"), "w", newline="\n") as f:
        for name, data in sums:
            f.write(f"{name} {c.sha(data)} {len(data)}\n")
    return c.finish("until stable, with the programs")


if __name__ == "__main__":
    sys.exit(main())
