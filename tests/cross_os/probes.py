#!/usr/bin/env -S python3 -I
"""probes.py --engine PATH: what reading could not settle, on the system this runs on.  docs/BUILD.md, "What has run on macOS and Windows", has what came of them.  It prints a line per probe and what was measured,
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
        # A share only an administrator has, which a runner is.  Reported and not judged: the $ goes through kpathsea's variable expansion, as it does in C.
        unc = "//localhost/" + abs_name[0] + "$" + abs_name[2:]
        d3 = fresh(root, "input-unc", f"\\input {unc}\n\\bye\n")
        p = run(engine, d3, "doc.tex")
        print(f"info \\input of a UNC name: {'found' if b'[found]' in p.stdout else 'not found'}, {unc}")
        # Longer than MAX_PATH: READABLE gives such a name the prefix \\\\?\\, std::fs does the same by itself.  Reported and not judged: a runner may not allow it.
        deep = abs_name.rsplit("/", 1)[0] + "/" + "/".join(["d" * 40] * 7)
        os.makedirs("\\\\?\\" + deep.replace("/", "\\"))
        with open("\\\\?\\" + deep.replace("/", "\\") + "\\in.tex", "w") as f:
            f.write("\\message{[found]}\n")
        d4 = fresh(root, "input-long", f"\\input {deep}/in.tex\n\\bye\n")
        p = run(engine, d4, "doc.tex")
        print(f"info \\input of a name of {len(deep) + 7} characters: {'found' if b'[found]' in p.stdout else 'not found'}")
        # The device readable.c sets apart.  LaTeX packages have asked for it to tell the system.
        d5 = fresh(root, "input-nul", "\\input nul\n\\message{[after nul]}\n\\bye\n")
        p = run(engine, d5, "doc.tex")
        say("\\input nul reads nothing", b"[after nul]" in p.stdout and p.returncode == 0, f"exit {p.returncode}")
        # What cmd makes of quotes: kpathsea_fsyscp_system.  A quoted program with a quoted argument is what it would take apart.
        d6 = fresh(root, "write18", "\\catcode`\\%=12\n\\immediate\\write18{\"%COMSPEC%\" /c \"echo seen>quoted.txt\"}\n\\immediate\\write18{'%COMSPEC%' /c 'echo seen>single.txt'}\n\\bye\n")
        p = run(engine, d6, "-shell-escape", "doc.tex")
        made = [n for n in ("quoted.txt", "single.txt") if os.path.exists(os.path.join(d6, n))]
        say("\\write18 with a quoted program and quoted arguments", len(made) == 2, f"made {made}, exit {p.returncode}")
        # input_line's ifdef WIN32.
        d7 = fresh(root, "mark", "\ufeff\\message{[\\ifhmode in a paragraph\\else no paragraph\\fi]}\n\\bye\n")
        p = run(engine, d7, "doc.tex")
        say("a byte order mark is passed over", b"[no paragraph]" in p.stdout, f"exit {p.returncode}")

    # A copy under pdfTeX's names, as a program that runs `pdflatex` finds it on the PATH: its name says the format.
    for name, text in (("pdflatex", "\\documentclass{article}\\begin{document}Hello.\\end{document}\n"), ("pdftex", HELLO)):
        d = fresh(root, "named-" + name, text)
        copy = os.path.join(d, name + (".exe" if WINDOWS else ""))
        shutil.copy2(engine, copy)
        p = subprocess.run([copy, "-interaction=nonstopmode", "doc.tex"], cwd=d, stdin=subprocess.DEVNULL, capture_output=True)
        say(f"a copy called {name}", p.returncode == 0 and os.path.exists(os.path.join(d, "doc.pdf")), f"exit {p.returncode}, {p.stderr[-200:]!r}")

    # Upper and lower case: what the file system does for a name, and what the databases do.  STORY.TEX is in ls-R as story.tex.
    d = fresh(root, "case", "\\input SmAlL\n\\input STORY.TEX\n\\bye\n")
    with open(os.path.join(d, "small.tex"), "w") as f:
        f.write("\\message{[case]}\n")
    if WINDOWS or MACOS:
        p = run(engine, d, "doc.tex")
        say("a name in another case", b"[case]" in p.stdout and p.returncode == 0, f"exit {p.returncode}")
    else:
        print("n/a  a name in another case: the file system tells them apart")

    if MACOS:
        # What APFS and HFS+ make of the two forms of an accented name.  Reported and not judged: kpathsea compares bytes too, so a miss in ls-R is what it would have.
        d = fresh(root, "nfd", "\\input caf\u00e9\n\\bye\n")
        with open(os.path.join(d, "cafe\u0301.tex"), "w", encoding="utf-8") as f:
            f.write("\\message{[nfd]}\n")
        p = run(engine, d, "doc.tex")
        names = sorted(n for n in os.listdir(d) if n.endswith(".tex") and n != "doc.tex")
        print(f"info a file made in NFD and asked for in NFC: {'found' if b'[nfd]' in p.stdout else 'not found'}, the directory lists {[ascii(n) for n in names]}")
        # The linker signed it for Apple's arm64, and only a signed program starts there.  Over a file that has run the kernel may remember the old signature, and kill the new one: that is cp, not this program.
        copy = os.path.join(root, "rstex-copy")
        shutil.copy2(engine, copy)
        p = subprocess.run([copy, "--version"], capture_output=True)
        say("a copy of the program starts", p.returncode == 0, f"exit {p.returncode}")
        shutil.copy2(engine, copy)
        p = subprocess.run([copy, "--version"], capture_output=True)
        print(f"info a copy over a copy that has run: exit {p.returncode}")
        print(f"info the page size is {os.sysconf('SC_PAGE_SIZE')}, on {platform.machine()}")

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
    if times[-1] > 1:
        # A later run should not build a format.  Say what it did, and what the cache directory looks like.
        p = run(engine, d, "-progname=pdflatex", "doc.tex", env={"RSTEX_FMT_VERBOSE": "1", "RSTEX_CHECKPOINT_TRACE": "1"})
        print(f"info a later LaTeX hello's stderr: {p.stderr[-2000:]!r}")
        cache = os.path.expanduser("~/Library/Caches" if MACOS else "~/.cache")
        for where in (cache, os.path.join(cache, "rstex")):
            if os.path.isdir(where):
                st = os.stat(where)
                print(f"info {where}: mode {st.st_mode:o} uid {st.st_uid} (mine {os.geteuid()}), holds {sorted(os.listdir(where))[:12]}")
            else:
                print(f"info {where}: not there")
        rstex = os.path.join(cache, "rstex")
        for fmt in sorted(os.listdir(rstex)) if os.path.isdir(rstex) else []:
            if fmt.startswith("fmt-"):
                for name in sorted(os.listdir(os.path.join(rstex, fmt))):
                    st = os.stat(os.path.join(rstex, fmt, name))
                    print(f"info {fmt}/{name}: {st.st_size} bytes, mode {st.st_mode:o}")


def main():
    # What a run prints can hold anything, and a Windows runner's standard output is in a code page that has not all of it.
    sys.stdout.reconfigure(encoding="utf-8", errors="replace")
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--engine", required=True)
    a = ap.parse_args()
    # A runner's temporary directory on Windows is under RUNNER~1, and in a document ~ is a macro, in pdfTeX as well.  The long name has none.
    root = os.path.realpath(tempfile.mkdtemp(prefix="rstex-probes-"))
    probes(os.path.abspath(a.engine), root)
    shutil.rmtree(root, ignore_errors=True)
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
