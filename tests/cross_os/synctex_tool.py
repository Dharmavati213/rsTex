#!/usr/bin/env -S python3 -I
"""synctex_tool.py --engine PATH: TeX Live's own synctex program on this system, forward and back, on the file rstex writes, against its answers on the file that system's pdfTeX writes.

rstex's file is made of more than one gzip member once there is more than a page, and the program reads that through zlib, of which this system's build may have another version.  In one directory that keeps its path, with the
oracle's side files put back first, the text of the file must be the same, byte for byte, and each answer of `synctex view` and `synctex edit` must be the same, and most of them real answers.  A document in a directory with an
accent as well, which on Windows has the names of the file's Input: lines in the code page of the file calls.
"""
import argparse
import gzip
import os
import re
import shutil
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import common as c  # noqa: E402

PART = "Words that come from a second file, so that a tag other than the first one appears, with a formula $x^2+y^2=z^2$ and more words after it,\nand a second line of them that goes on for a while, to have something to find on it.\n\n"
MAIN = "\\documentclass{article}\n\\pdfinfoomitdate=1 \\pdftrailerid{}\n\\begin{document}\nA first line of plain words, and then $a+b=c$ in mathematics.\n\n\\input{part}\n\n" + "A paragraph of its own, on a line of its own, with enough words in it to fill the line and more besides.\n\n\\newpage\n\\input{part}\n" * 4 + "\\end{document}\n"
RECORD = re.compile(rb"^[\[(hvkg$xr](\d+),(\d+):(-?\d+),(-?\d+)")
SP_PER_BP = 65781.76


def run_pair(engine, latex, synctex, name, work):
    """The oracle, then rstex, each from the oracle's side files, and the same questions of both."""
    env = c.base_env(os.path.join(os.path.dirname(work), "cache"))
    out = os.path.join(work, "o")
    flags = ["-synctex=1", "-interaction=nonstopmode", "-output-directory=o", "doc.tex"]
    os.makedirs(out, exist_ok=True)
    for _ in range(2):
        r = c.run([latex, *flags], work, env)
    if r[0] != 0:
        c.say(f"{name}: pdfTeX compiles it", False, f"status {r[0]}: {r[1][-300:]!r}")
        return
    oracle = c.state(out)
    side = {k: v for k, v in oracle.items() if k.endswith((".aux", ".toc"))}
    c.put_back(out, side)
    r = c.run([engine, "-progname=pdflatex", *flags], work, env)
    ours = c.state(out)
    if r[0] != 0 or "doc.synctex.gz" not in ours:
        c.say(f"{name}: rstex compiles it", False, f"status {r[0]}: {r[2][-300:]!r}")
        return
    n = c.members(ours["doc.synctex.gz"])
    pages = gzip.decompress(ours["doc.synctex.gz"]).count(b"{")
    c.say(f"{name}: rstex's file has a member for more than a page", n > 1 and pages > 1, f"{n} members, {pages} pages, against pdfTeX's {c.members(oracle['doc.synctex.gz'])}")
    a, b = gzip.decompress(oracle["doc.synctex.gz"]), gzip.decompress(ours["doc.synctex.gz"])
    d = next((i for i, (p, q) in enumerate(zip(a.split(b"\n"), b.split(b"\n"))) if p != q), None)
    c.say(f"{name}: the text of the file", a == b, f"{len(a)} bytes" if a == b else f"line {d}: {a.split(bytes([10]))[d or 0][:120]!r} and {b.split(bytes([10]))[d or 0][:120]!r}")
    # Questions from the oracle's records: where each kept a place.
    places, page = [], 0
    inputs = {}
    for line in a.split(b"\n"):
        if line.startswith(b"Input:"):
            tag, _, path = line[6:].partition(b":")
            inputs[tag] = path
        elif line.startswith(b"{"):
            page = int(line[1:])
        else:
            m = RECORD.match(line)
            if m:
                places.append((page, int(m.group(3)) / SP_PER_BP, int(m.group(4)) / SP_PER_BP))
    places = places[:: max(1, len(places) // 120)]
    questions = [["edit", "-o", f"{p}:{x + 72.27:.3f}:{y + 72.27:.3f}:o/doc.pdf"] for p, x, y in places]
    questions += [["view", "-i", f"{line}:0:{file}", "-o", "o/doc.pdf"] for file in ("doc.tex", "part.tex") for line in range(1, 16)]
    answers = {}
    for side_name, s in (("oracle", oracle), ("rstex", ours)):
        c.put_back(out, s)
        answers[side_name] = [c.run([synctex, *q], work, env) for q in questions]
    bad = [q for q, x, y in zip(questions, answers["oracle"], answers["rstex"]) if x[:2] != y[:2]]
    real = sum(1 for x in answers["rstex"] if b"Page:" in x[1] or b"Input:" in x[1])
    first = bad[0] if bad else None
    c.say(f"{name}: synctex's answers", not bad and real >= len(questions) // 2, f"{len(questions) - len(bad)} of {len(questions)} the same, {real} real answers{'; first differs: ' + repr(first) if first else ''}")
    if bad:
        i = questions.index(first)
        c.info(f"{name}: on pdfTeX's file: {answers['oracle'][i][1][:300]!r}; on rstex's: {answers['rstex'][i][1][:300]!r}")
    c.info(f"{name}: an answer back: {[x[1] for x in answers['rstex'] if b'Input:' in x[1]][:1]!r}")


def main():
    c.start()
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--engine", required=True)
    ap.add_argument("--label", default="here")
    a = ap.parse_args()
    engine = c.program(a.engine)
    latex, synctex = c.tool("pdflatex"), c.tool("synctex")
    if not synctex:
        c.say("there is a synctex program", False, "tools/ci/install_texlive.py installs one")
        return c.finish("synctex")
    v = c.run([synctex, "version"], c.OUT, os.environ)
    c.info(f"synctex says {v[1][:200]!r}")
    root = c.fresh(os.path.join(c.OUT, a.label, "synctex"))
    for name, d in (("a plain name", "doc"), ("a name with an accent and a space", "caf\u00e9 doc")):
        work = c.fresh(os.path.join(root, d))
        c.write(os.path.join(work, "doc.tex"), MAIN)
        c.write(os.path.join(work, "part.tex"), PART)
        c.guarded(name, run_pair, engine, latex, synctex, name, work)
    shutil.rmtree(root, ignore_errors=True)
    return c.finish("synctex")


if __name__ == "__main__":
    sys.exit(main())
