#!/usr/bin/env -S python3 -I
"""run.py: the same PDF on every system.  DESIGN.md section 12: "every platform must produce the same PDF bytes for the same inputs".

    run.py --engine PATH --label NAME      compiles each document here, cold and then warm, and writes target/cross-os/NAME/sums.txt
    run.py --compare DIR DIR...            says if the sums.txt in those directories agree; exit status 1 if not

The documents are the .tex files beside this file: written for this, public, and nothing else is ever compiled here.  Each says \\pdfinfoomitdate=1 and \\pdftrailerid{}, since the date and
the /ID hold the clock and the working directory.  A document runs twice in one directory, so that the second run has the .aux.  Then once more, which restores the preamble checkpoint the
second run made, and that run's PDF has to be the same: it is the path through `commit_range` on Windows and through private mappings of 16 KiB pages on Apple's arm64, which no other test on
those systems goes through.  Exit status 1 if a run fails or a warm PDF differs.

What it needs: a TeX Live with the same packages on each system.  Everything it writes is under target/cross-os/, and CI uploads that, which is the PDFs of these documents and their logs.
"""
import argparse
import hashlib
import os
import platform
import shutil
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(os.path.dirname(HERE))
OUT = os.path.join(ROOT, "target", "cross-os")
ENV = {"SOURCE_DATE_EPOCH": "1700000000", "FORCE_SOURCE_DATE": "1", "TZ": "UTC"}


def cache_under(path):
    """What puts the cache directory under `path`: DESIGN.md section 12 has a rule for each system, and only Linux reads XDG_CACHE_HOME."""
    system = platform.system()
    if system == "Windows":
        return {"LOCALAPPDATA": path}
    if system == "Darwin":
        return {"HOME": path}
    return {"XDG_CACHE_HOME": path}


def documents():
    return sorted(f for f in os.listdir(HERE) if f.endswith(".tex"))


def compile_all(engine, label):
    out = os.path.join(OUT, label)
    shutil.rmtree(out, ignore_errors=True)
    os.makedirs(out)
    # `second`: the first run only notes the preamble, whatever its size, the second is cold and writes, and the third restores.
    env = dict(os.environ, **ENV, **cache_under(os.path.join(out, "cache")), RSTEX_CHECKPOINT_TRACE="1", RSTEX_CHECKPOINT_POLICY="second")
    bad = 0
    lines = []
    for doc in documents():
        name = doc[:-4]
        work = os.path.join(out, name)
        os.makedirs(work)
        shutil.copy(os.path.join(HERE, doc), work)
        pdfs = []
        traces = []
        for run in range(3):
            p = subprocess.run([engine, "-progname=" + first_line_format(doc), "-interaction=nonstopmode", doc], cwd=work, env=env, stdin=subprocess.DEVNULL, capture_output=True)
            traces.append(p.stderr.decode("utf-8", "replace"))
            if p.returncode != 0:
                print(f"{name}: run {run + 1} exited with {p.returncode}\n{p.stdout.decode('utf-8', 'replace')[-2000:]}\n{traces[-1][-2000:]}")
                bad += 1
                break
            with open(os.path.join(work, name + ".pdf"), "rb") as f:
                pdfs.append(f.read())
        if len(pdfs) != 3:
            continue
        restored = "restored" in traces[2]
        print(f"{name}: {len(pdfs[2])} bytes, sha256 {hashlib.sha256(pdfs[2]).hexdigest()}, third run {'restored a checkpoint' if restored else 'ran cold'}")
        if pdfs[1] != pdfs[2]:
            print(f"{name}: the cold PDF and the run after it differ")
            bad += 1
        lines.append(f"{name} {hashlib.sha256(pdfs[2]).hexdigest()} {len(pdfs[2])}\n")
    with open(os.path.join(out, "sums.txt"), "w", newline="\n") as f:
        f.writelines(lines)
    return 1 if bad else 0


def first_line_format(doc):
    with open(os.path.join(HERE, doc), encoding="latin-1") as f:
        return f.readline().split("format:")[1].strip()


def compare(dirs):
    sums = {}
    bad = 0
    # A system that stopped early says so, and the others are still compared: which of them agree is the point.
    for d in dirs:
        try:
            with open(os.path.join(d, "sums.txt")) as f:
                sums[d] = f.read()
        except OSError:
            print(f"{d} has no sums.txt: that system did not get as far as writing them")
            bad = 1
    dirs = [d for d in dirs if d in sums]
    if not dirs:
        return 1
    first = dirs[0]
    for d in dirs[1:]:
        if sums[d] != sums[first]:
            print(f"{d} differs from {first}:\n{sums[d]}--- against\n{sums[first]}")
            bad = 1
    if not bad:
        print(f"{len(dirs)} systems agree on {sums[first].count(chr(10))} documents")
        print(sums[first], end="")
    return bad


def main():
    # What a run prints can hold anything, and a Windows runner's standard output is in a code page that has not all of it.
    sys.stdout.reconfigure(encoding="utf-8", errors="replace")
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--engine")
    ap.add_argument("--label")
    ap.add_argument("--compare", nargs="+", metavar="DIR")
    a = ap.parse_args()
    if a.compare:
        return compare(a.compare)
    if not a.engine or not a.label:
        ap.error("--engine and --label, or --compare")
    return compile_all(os.path.abspath(a.engine), a.label)


if __name__ == "__main__":
    sys.exit(main())
