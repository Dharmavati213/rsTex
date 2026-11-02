#!/usr/bin/env python3
"""Expected results for the unit tests of pdfship.rs, taken from the oracle.

Run with python3 -I.  It writes crates/engine/testdata/pdfship_oracle.tsv, one row per line, fields separated by a tab,
the first being the kind of row:

  matrix   TEXT  0|1          does \\pdfsetmatrix{TEXT} on a page pass (sscanf with four numbers), or stop the run.
                              A tab in TEXT is written <TAB>.
  sorted   NAME               the names of \\pdfdest, in the order the PDF's name tree has them, which is §793's sort
  time     EPOCH  TEXT        /CreationDate under SOURCE_DATE_EPOCH=EPOCH
  id       TIME  FILE  HEX    /ID under that date and output file name: the digest of the two
  idalt    TEXT  HEX          /ID with \\pdftrailerid{TEXT}
  resname  Y M D T JOB TAG    the tag \\pdfuniqueresname makes under these \\year, \\month, \\day, \\time and job name

Each case is a one-page document run by `pdftex -ini -etex`, in a temporary directory.
"""
import os
import re
import subprocess
import tempfile

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
OUT = os.path.join(ROOT, "crates", "engine", "testdata", "pdfship_oracle.tsv")

PREAMBLE = r"""\catcode`\{=1 \catcode`\}=2 \catcode`\#=6
\pdfoutput=1 \pdfcompresslevel=0 \pdfobjcompresslevel=0 \scrollmode
"""

MATRIX = [
    "1 0 0 1", " 1 0 0 1", "1 0 0 1 ", "  1   0   0   1  ", "1.5 .5 -.5 +2.", "1 0 0", "1 0 0 1 x", "1 0 0 1 1", "", " ",
    "a 0 0 1", "1 a 0 1", "1 0 0 a", "1,0,0,1", "1e0 0 0 1", "1E0 0 0 1", "1e+0 0 0 1", "1e-0 0 0 1", "1e 0 0 1", "1E 0 0 1",
    "1e+ 0 0 1", "1e- 0 0 1", "1ex 0 0 1", "1e+x 0 0 1", "1e5e5 0 0 1", "1.5.5 0 0 1", ". 0 0 1", "- 0 0 1", "+ 0 0 1", "-. 0 0 1",
    "0x1 0 0 1", "0X1 0 0 1", "0x 0 0 1", "0x. 0 0 1", "0x.8 0 0 1", "0x1p 0 0 1", "0x1p+ 0 0 1", "0x1p1 0 0 1", "0x1P-1 0 0 1",
    "0xg 0 0 1", "0x1g 0 0 1", "00 00 00 00", "0 0 0 0", "-0 -0 -0 -0", "inf 0 0 1", "INF 0 0 1", "-inf 0 0 1", "+Inf 0 0 1",
    "infinity 0 0 1", "INFINITY 0 0 1", "infin 0 0 1", "infinit 0 0 1", "in 0 0 1", "i 0 0 1", "infx 0 0 1", "nan 0 0 1", "NaN 0 0 1",
    "-nan 0 0 1", "nan() 0 0 1", "nan(1) 0 0 1", "na 0 0 1", "n 0 0 1", "nanx 0 0 1", "1\t0\t0\t1", "1e400 0 0 1", "1e-400 0 0 1",
    "123456789012345678901234567890 0 0 1", ".5e1 0 0 1", "5.e-1 0 0 1", "1 0 0 1e", "1 0 0 1e+", "1 0 0 0x", "1 0 0 1.", "1 0 0 .",
    "0e 0 0 1", "0e0 0 0 1", "0.e 0 0 1", "1a 0 0 1", "1 0 0 1a", "1pt 0 0 1", "1 0 0 1%",
    "nan( 0 0 1", "nan(a 0 0 1", "nan(a_1) 0 0 1", "nan(-) 0 0 1", "nan(1)x 0 0 1", "-nan(1) 0 0 1", "1 0 0 nan(1)", "1 0 0 nan(",
    "-0x 0 0 1", "-0x. 0 0 1", "+0x1 0 0 1", "0x1e1 0 0 1", "0x1p1p1 0 0 1", "0x1.8 0 0 1", "0x1..8 0 0 1", "0xp1 0 0 1", "0x.p1 0 0 1",
    "1.e5 0 0 1", ".e5 0 0 1", "1e5. 0 0 1", "1e+-5 0 0 1", "--1 0 0 1", "+-1 0 0 1", "1-1 0 0 1", "1 0 0 1\t",
]

SORTED = [
    "a", "aa", "ab", "b", "A", "B", "Z", "0", "9", "*", ".", "z", "~",
    r"\(x", r"\)x", r"\\x", r"\nx", r"\tx", r"\rx", r"\bx", r"\fx", r"\zx", r"\101", r"\60", r"\7", r"\477", r"\1234", r"\0m",
    r"a\101b", "aAb", "aAc", "a ", "a b", r"q\8",
]

TIMES = [0, 1, 1000000000, 1234567890, 1700000000, 2147483647, 4102444800]

IDALT = ["x", "some text", "(paren)", "1234567890"]

RESNAME = [
    (2026, 11, 2, 720, "jobone"),
    (2000, 1, 1, 0, "jobtwo"),
    (1999, 12, 31, 1439, "a"),
    (2026, 11, 2, 721, "jobone"),
]


def run(doc, env=None, jobname="t"):
    """The document's log and PDF, as bytes; the PDF is empty if the run stopped."""
    with tempfile.TemporaryDirectory() as d:
        with open(os.path.join(d, jobname + ".tex"), "wb") as f:
            f.write(doc.encode("latin-1"))
        e = dict(os.environ)
        e.pop("SOURCE_DATE_EPOCH", None)
        e.pop("FORCE_SOURCE_DATE", None)
        e.update(env or {})
        subprocess.run(
            ["pdftex", "-ini", "-etex", "-interaction=nonstopmode", "*" + jobname + ".tex"],
            cwd=d, env=e, stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=30,
        )
        with open(os.path.join(d, jobname + ".log"), "rb") as f:
            log = f.read()
        pdf = b""
        p = os.path.join(d, jobname + ".pdf")
        if os.path.exists(p):
            with open(p, "rb") as f:
                pdf = f.read()
        return log, pdf


def matrix_rows():
    rows = []
    for text in MATRIX:
        assert "\n" not in text
        doc = PREAMBLE + "\\catcode`\\%=12 \\catcode`\\^^I=12 \\shipout\\hbox{\\pdfsetmatrix{" + text + "}}\n\\end\n"
        log, pdf = run(doc)
        ok = b"Unrecognized format" not in log
        assert ok == (pdf != b""), text
        rows.append(("matrix", text, "1" if ok else "0"))
    return rows


def sorted_rows():
    # The names are read with | as the escape character and \ as an ordinary one.
    doc = PREAMBLE + "\\catcode`\\|=0 |catcode`|\\=12\n|shipout|hbox{%\n".replace("%", "|relax") + "".join(
        "|pdfdest name{" + n + "} fit\n" for n in SORTED
    ) + "}\n|end\n"
    log, pdf = run(doc)
    assert pdf, log
    names = []
    for tree in re.findall(rb"/Names \[(.*?)\]", pdf, re.S):
        names += re.findall(rb"\(((?:\\.|[^\\()]|\([^()]*\))*)\) *\d+ 0 R", tree, re.S)
    got = [n.decode("latin-1") for n in names]
    assert sorted(got) == sorted(SORTED), (got, SORTED)
    return [("sorted", n) for n in got]


def time_rows():
    rows = []
    for t in TIMES:
        log, pdf = run(PREAMBLE + "\\shipout\\hbox{}\n\\end\n", {"SOURCE_DATE_EPOCH": str(t)})
        d = re.search(rb"/CreationDate \((.*?)\)", pdf).group(1).decode()
        i = re.search(rb"/ID \[<([0-9A-Fa-f]+)> <([0-9A-Fa-f]+)>\]", pdf)
        assert i.group(1) == i.group(2)
        rows.append(("time", str(t), d))
        rows.append(("id", d, "t.pdf", i.group(1).decode()))
    return rows


def idalt_rows():
    rows = []
    for text in IDALT:
        log, pdf = run(PREAMBLE + "\\pdftrailerid{" + text + "}\\shipout\\hbox{}\n\\end\n")
        i = re.search(rb"/ID \[<([0-9A-Fa-f]+)> <([0-9A-Fa-f]+)>\]", pdf)
        rows.append(("idalt", text, i.group(1).decode()))
    return rows


def resname_rows():
    rows = []
    for y, m, d, t, job in RESNAME:
        doc = PREAMBLE + "\\font\\x=cmr10 \\pdfuniqueresname=1 \\year=%d \\month=%d \\day=%d \\time=%d\n" % (y, m, d, t)
        doc += "\\shipout\\hbox{\\x a}\n\\end\n"
        log, pdf = run(doc, jobname=job)
        tag = re.search(rb"/Font << /F\d+(\w{6}) ", pdf).group(1).decode()
        rows.append(("resname", str(y), str(m), str(d), str(t), job, tag))
    return rows


def main():
    rows = matrix_rows() + sorted_rows() + time_rows() + idalt_rows() + resname_rows()
    with open(OUT, "w", newline="\n", encoding="latin-1") as f:
        for r in rows:
            f.write("\t".join(x.replace("\t", "<TAB>") for x in r) + "\n")
    print("%d rows in %s" % (len(rows), OUT))


if __name__ == "__main__":
    main()
