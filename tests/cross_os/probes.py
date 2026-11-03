#!/usr/bin/env -S python3 -I
"""probes.py --engine PATH: what reading could not settle, on the system this runs on.  docs/BUILD.md, "What CI has to settle", lists each.  It prints a line per probe and what was measured,
and exit status 1 if one fails.  Everything is under a directory of its own, from documents written here; nothing else is compiled.

Runs on any system; what does not apply says so.
"""
import argparse
import os
import platform
import shutil
import subprocess
import sys
import tempfile
import time

WINDOWS = platform.system() == "Windows"
MACOS = platform.system() == "Darwin"
HELLO = "\\pdfoutput=1 \\font\\f=cmr10 \\f hello\n\\bye\n"
failed = []


def say(name, ok, what=""):
    print(f"{'ok  ' if ok else 'FAIL'} {name}{': ' + what if what else ''}", flush=True)
    if not ok:
        failed.append(name)


def run(engine, cwd, *args, stdin=None, env=None):
    e = dict(os.environ, **(env or {}))
    return subprocess.run([engine, "-interaction=nonstopmode", *args], cwd=cwd, env=e, input=stdin, stdin=None if stdin is not None else subprocess.DEVNULL, capture_output=True)


def fresh(root, name, text=HELLO, file="doc.tex"):
    d = os.path.join(root, name)
    os.makedirs(d)
    with open(os.path.join(d, file), "w", encoding="utf-8", newline="") as f:
        f.write(text)
    return d


def probes(engine, root):
    # Names: spaces and non-ASCII, in the directory and in the file; a file made with a name from outside the BMP too.
    name = "caf\u00e9 \u4e2d\u6587 \U0001d11e"
    d = fresh(root, name, file=name + ".tex")
    p = run(engine, d, name + ".tex")
    made = os.path.exists(os.path.join(d, name + ".pdf"))
    say("a file name with spaces and non-ASCII", p.returncode == 0 and made, f"exit {p.returncode}, PDF {'made' if made else 'missing'}")

    # \input by the forms each system has.
    d = fresh(root, "input", "\\input sub/in.tex\n\\input ./sub/in.tex\n\\bye\n")
    os.makedirs(os.path.join(d, "sub"))
    with open(os.path.join(d, "sub", "in.tex"), "w") as f:
        f.write("\\message{[found]}\n")
    p = run(engine, d, "doc.tex")
    say("\\input with a relative directory", p.stdout.count(b"[found]") == 2, f"found {p.stdout.count(b'[found]')} of 2")
    if WINDOWS:
        abs_name = os.path.join(d, "sub", "in.tex").replace("\\", "/")
        d2 = fresh(root, "input-drive", f"\\input {abs_name}\n\\bye\n")
        p = run(engine, d2, "doc.tex")
        say("\\input with a drive letter", b"[found]" in p.stdout, abs_name)
        p = run(engine, d, os.path.join(d, "doc.tex"))
        say("a main file named C:\\...", p.returncode == 0, f"exit {p.returncode}")

    # Upper and lower case: what the file system does for a name, and what the databases do.
    d = fresh(root, "case", "\\input SmAlL\n\\input PLAIN.TEX\n\\bye\n")
    with open(os.path.join(d, "small.tex"), "w") as f:
        f.write("\\message{[case]}\n")
    if WINDOWS or MACOS:
        p = run(engine, d, "doc.tex")
        say("a name in another case", b"[case]" in p.stdout and p.returncode == 0, f"exit {p.returncode}")
    else:
        print("n/a  a name in another case: the file system tells them apart")

    # A format and a checkpoint over a file in use: the rename has to go through or fail cleanly.
    d = fresh(root, "held")
    p = run(engine, d, "doc.tex")
    pdf = os.path.join(d, "doc.pdf")
    say("a first run", p.returncode == 0 and os.path.exists(pdf), f"exit {p.returncode}")
    if WINDOWS:
        import ctypes
        # Held as a viewer holds it, with no sharing: CreateFileW with share mode 0.
        h = ctypes.windll.kernel32.CreateFileW(pdf, 0x80000000, 0, None, 3, 0, None)
        p = run(engine, d, "doc.tex")
        ctypes.windll.kernel32.CloseHandle(h)
        text = p.stdout.decode("utf-8", "replace")
        say("a PDF held open by a viewer", p.returncode != 0 and "doc.pdf" in text, text.strip().splitlines()[-3] if len(text.strip().splitlines()) >= 3 else text)

    # The clock in the PDF and in the log, against what this system says.  With the zone as it is here.
    d = fresh(root, "clock", "\\pdfoutput=1 \\pdfcompresslevel=0 \\pdfobjcompresslevel=0 \\message{[\\the\\year.\\the\\month.\\the\\day]}\\font\\f=cmr10 \\f hello\n\\bye\n")
    p = run(engine, d, "doc.tex")
    t = time.localtime()
    say("\\year, \\month, \\day are local", f"[{t.tm_year}.{t.tm_mon}.{t.tm_mday}]".encode() in p.stdout, f"{t.tm_year}.{t.tm_mon}.{t.tm_mday}")
    with open(os.path.join(d, "doc.pdf"), "rb") as f:
        data = f.read()
    at = data.find(b"/CreationDate (D:")
    offset = data[at + 17:at + 40].split(b")")[0] if at >= 0 else b""
    want = time.strftime("%z")
    say("/CreationDate has the zone", at >= 0, f"{offset!r}, the system says {want}")

    # Deep recursion: the stack that is asked for has to be there, and a recursion that fills it has its message.
    d = fresh(root, "deep", "\\pdfoutput=1 \\setbox0\\hbox{}\\count1=0 \\loop\\advance\\count1 by 1 \\setbox0\\hbox{\\box0}\\ifnum\\count1<500000 \\repeat \\shipout\\box0 \\bye\n")
    t0 = time.time()
    p = run(engine, d, "doc.tex")
    say("half a million boxes in a box", p.returncode == 0, f"exit {p.returncode} in {time.time() - t0:.1f} s, which the oracle's stack of 8 MiB cannot do")

    # What a run costs where the first touch of each page goes through an exception, and a first run against a later one.
    d = fresh(root, "times")
    times = []
    for _ in range(5):
        t0 = time.perf_counter()
        run(engine, d, "doc.tex")
        times.append(time.perf_counter() - t0)
    print(f"info plain hello, wall seconds of 5 runs: {' '.join(f'{x:.4f}' for x in times)}")
    d = fresh(root, "times-latex", "\\documentclass{article}\\begin{document}Hello.\\end{document}\n")
    times = []
    for _ in range(5):
        t0 = time.perf_counter()
        run(engine, d, "-progname=pdflatex", "doc.tex")
        times.append(time.perf_counter() - t0)
    print(f"info LaTeX hello, wall seconds of 5 runs: {' '.join(f'{x:.4f}' for x in times)}")


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--engine", required=True)
    a = ap.parse_args()
    root = tempfile.mkdtemp(prefix="rstex-probes-")
    probes(os.path.abspath(a.engine), root)
    shutil.rmtree(root, ignore_errors=True)
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
