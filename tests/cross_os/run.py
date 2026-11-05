#!/usr/bin/env -S python3 -I
"""run.py: the same PDF on every system.  DESIGN.md section 12: "every platform must produce the same PDF bytes for the same inputs".

    run.py --engine PATH --label NAME      compiles each document here, cold and then warm, and writes target/cross-os/NAME/sums.txt
    run.py --compare DIR DIR...            says if the sums.txt in those directories agree; exit status 1 if not

The documents are the .tex files beside this file, written for this, and those of tests/ladder, which are public too, and nothing else is ever compiled here.  Each says \\pdfinfoomitdate=1 and \\pdftrailerid{}, or for one of the
ladder its first line does, since the date and the /ID hold the clock and the working directory.  A document runs twice in one directory, so that the second run has the .aux.  Then once more, which restores the preamble checkpoint the
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
    """(name, its path, the line to give the program, other files it reads)."""
    out = [(f[:-4], os.path.join(HERE, f), f, ()) for f in sorted(os.listdir(HERE)) if f.endswith(".tex")]
    ladder = os.path.join(ROOT, "tests", "ladder")
    for f in sorted(os.listdir(ladder)):
        if f.endswith(".tex"):
            # An included PDF of the tree's has the place of the tree in /PTEX.FileName, and the date and the /ID go as above.
            out.append((f[:-4], os.path.join(ladder, f), "\\pdfinfoomitdate=1 \\pdftrailerid{} \\pdfsuppressptexinfo=-1 \\input " + f, ("ladder-logo.jpg", "ladder-page2.png") if f.startswith("12-") else ()))
    return out


def compile_all(engine, label):
    out = os.path.join(OUT, label)
    shutil.rmtree(out, ignore_errors=True)
    os.makedirs(out)
    # `second`: the first run only notes the preamble, whatever its size, the second is cold and writes, and the third restores.
    env = dict(os.environ, **ENV, **cache_under(os.path.join(out, "cache")), RSTEX_CHECKPOINT_TRACE="1", RSTEX_CHECKPOINT_POLICY="second")
    bad = 0
    lines = []
    for name, source, line, extras in documents():
        work = os.path.join(out, name)
        os.makedirs(work)
        shutil.copy(source, work)
        for x in extras:
            shutil.copy(os.path.join(os.path.dirname(source), x), work)
        pdfs = []
        traces = []
        fmt = format_of(source)
        for run in range(3):
            p = subprocess.run([engine, "-progname=" + fmt, "-interaction=nonstopmode", line], cwd=work, env=env, stdin=subprocess.DEVNULL, capture_output=True)
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
        if not restored and fmt != "pdftex":
            for i in (1, 2):
                print(f"{name}: what run {i + 1} said on stderr: {traces[i][-1500:]!r}")
        if pdfs[1] != pdfs[2]:
            print(f"{name}: the cold PDF and the run after it differ")
            bad += 1
        lines.append(f"{name} {hashlib.sha256(pdfs[2]).hexdigest()} {len(pdfs[2])}\n")
    with open(os.path.join(out, "sums.txt"), "w", newline="\n") as f:
        f.writelines(lines)
    return 1 if bad else 0


def format_of(path):
    with open(path, "rb") as f:
        return "pdflatex" if b"\\documentclass" in f.read() else "pdftex"


def compare(dirs):
    """Every sums*.txt that any system has, the documents' and those of the other tools, wherever in its directory: an artifact of several directories keeps their common part."""
    bad = 0
    found = {d: {n: os.path.join(top, n) for top, _, files in os.walk(d) for n in files if n.startswith("sums") and n.endswith(".txt")} for d in dirs}
    names = sorted({n for d in dirs for n in found[d]} | {"sums.txt"})
    for name in names:
        sums = {}
        # A system that stopped early says so, and the others are still compared: which of them agree is the point.
        for d in dirs:
            try:
                with open(found[d][name]) as f:
                    sums[d] = f.read()
            except (KeyError, OSError):
                print(f"{d} has no {name}: that system did not get as far as writing them")
                bad = 1
        have = [d for d in dirs if d in sums]
        if not have:
            bad = 1
            continue
        first = have[0]
        same = True
        for d in have[1:]:
            if sums[d] != sums[first]:
                a, b = sums[first].splitlines(), sums[d].splitlines()
                differ = [f"{x}\n--- and\n{y}" for x, y in zip(a, b) if x != y]
                print(f"{name}: {d} differs from {first}, on {len(differ)} lines of {len(a)} and {len(b)}:\n" + "\n".join(differ[:12]))
                # What the other tools write is reported.  The documents' PDFs are what must be the same bytes.
                if name == "sums.txt":
                    bad = 1
                same = False
        if same:
            print(f"{name}: {len(have)} systems agree on {sums[first].count(chr(10))} lines")
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
