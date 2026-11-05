#!/usr/bin/env -S python3 -I
"""speed.py --engine PATH: what a build costs on this system, as info lines, and no gate: a speed bug that gives no wrong output shows nowhere else.  The macOS one where every run rebuilt its format because /dev/null's times changed is the kind.

LaTeX hello, a document of 50 pages and one of 40 tikz-cd diagrams, each cold and warm, by the wall clock from the caller, as an editor sees it, with that system's own pdfTeX beside them.  Under them what they are made of: how long the system takes
to start a process, to look at a file, to make one, to rename one, and to read back what a run reads.  Each is the minimum and the median of some runs.
"""
import argparse
import os
import statistics
import subprocess
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import common as c  # noqa: E402


def timed(act, n):
    out = []
    for _ in range(n):
        t0 = time.perf_counter()
        act()
        out.append(time.perf_counter() - t0)
    return out


def shown(xs, unit=1000.0, name="ms"):
    return f"min {min(xs) * unit:.1f} {name}, median {statistics.median(xs) * unit:.1f} {name}, of {len(xs)}"


def documents(root):
    d = {}
    for name, text in (("LaTeX hello", "\\documentclass{article}\\begin{document}Hello.\\end{document}\n"), ("50 pages", c.long_document(50)), ("40 tikz-cd diagrams", c.diagrams_document(40))):
        work = c.fresh(os.path.join(root, name.replace(" ", "-")))
        c.write(os.path.join(work, "doc.tex"), text)
        d[name] = work
    return d


def builds(engine, latex, root):
    env = c.base_env(os.path.join(root, "cache"))
    env["RSTEX_CHECKPOINT_POLICY"] = "always"
    for name, work in documents(root).items():
        out = os.path.join(work, "o")
        os.makedirs(out)
        flags = ["-interaction=nonstopmode", "-synctex=1", "-file-line-error", "-recorder", "-output-directory=o", "doc.tex"]
        n = 7 if name == "LaTeX hello" else 3

        def clean():
            c.rmtree(out)
            os.makedirs(out)

        def one(argv):
            def act():
                r = c.run(argv, work, env)
                if r[0] != 0:
                    raise RuntimeError(f"{name}: status {r[0]}: {r[1][-300:]!r}")
            return act

        clean()
        cold = []
        for _ in range(n):
            clean()
            cold += timed(one([engine, "--no-snapshot", "-progname=pdflatex", *flags]), 1)
        c.info(f"{name}: rstex cold: {shown(cold)}")
        # The format is there by now.  The first with a checkpoint, the rest from it, with the side files and the PDF of the run before them.
        c.forget_runs(os.path.join(root, "cache"))
        c.run([engine, "-progname=pdflatex", *flags], work, env)
        warm = timed(one([engine, "-progname=pdflatex", *flags]), n)
        c.info(f"{name}: rstex warm, everything in place: {shown(warm)}")
        if latex:
            clean()
            ref = []
            for _ in range(n):
                clean()
                ref += timed(one([latex, *flags]), 1)
            c.info(f"{name}: pdfTeX: {shown(ref)}")


def system_costs(engine, root):
    pdftex = c.tool("pdftex")
    for name, argv in (("rstex --version", [engine, "--version"]), ("pdfTeX --version", [pdftex, "--version"])):
        with open(os.devnull, "wb") as null:
            c.info(f"to start {name}: {shown(timed(lambda a=argv: subprocess.run(a, stdout=null, stderr=null, stdin=subprocess.DEVNULL, check=False), 30))}")
    d = c.fresh(os.path.join(root, "files"))
    for i in range(1000):
        with open(os.path.join(d, f"f{i}"), "wb") as f:
            f.write(b"x")
    names = [os.path.join(d, f"f{i}") for i in range(1000)]
    t0 = time.perf_counter()
    for n in names:
        os.stat(n)
    c.info(f"a stat: {(time.perf_counter() - t0) * 1e3:.1f} us")
    t0 = time.perf_counter()
    for n in names:
        try:
            os.stat(n + ".missing")
        except OSError:
            pass
    c.info(f"a stat of what is not there: {(time.perf_counter() - t0) * 1e3:.1f} us")
    t0 = time.perf_counter()
    for n in names:
        with open(n, "rb") as f:
            f.read()
    c.info(f"open, read and close: {(time.perf_counter() - t0) * 1e3:.1f} us")
    t0 = time.perf_counter()
    for i, n in enumerate(names):
        os.replace(n, n + ".r")
    c.info(f"a rename: {(time.perf_counter() - t0) * 1e3:.1f} us")
    t0 = time.perf_counter()
    for i in range(1000):
        with open(os.path.join(d, f"n{i}"), "wb") as f:
            f.write(b"x")
    c.info(f"make a file: {(time.perf_counter() - t0) * 1e3:.1f} us")
    # What a run reads: the format, whole.
    cache = c.cache_dir(os.path.join(root, "cache"))
    for top, _, files in os.walk(cache):
        for f in files:
            if f.endswith(".fmt") or f.endswith(".ck"):
                p = os.path.join(top, f)
                c.info(f"{f[-20:]}: {os.path.getsize(p) >> 10} KiB, to read it: {shown(timed(lambda q=p: c.read(q), 5))}")


def defender():
    """Whether anything is looking at each file here, which a user's machine has and a runner may not."""
    if not c.WINDOWS:
        return
    r = subprocess.run(["powershell", "-NoProfile", "-Command", "Get-MpComputerStatus | Select-Object -Property RealTimeProtectionEnabled,AntivirusEnabled,AMRunningMode | Format-List; Get-MpPreference | Select-Object -Property DisableRealtimeMonitoring | Format-List"],
                       capture_output=True, text=True, check=False)
    c.info(f"Windows Defender here: {' '.join(r.stdout.split())} {r.stderr.strip()[:200]}")


def main():
    c.start()
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--engine", required=True)
    ap.add_argument("--label", default="here")
    a = ap.parse_args()
    engine = c.program(a.engine)
    root = c.fresh(os.path.join(c.OUT, a.label, "speed"))
    c.guarded("builds", builds, engine, c.tool("pdflatex"), root)
    c.guarded("costs", system_costs, engine, root)
    c.guarded("defender", defender)
    c.rmtree(root)
    return c.finish("speed, no gate")


if __name__ == "__main__":
    sys.exit(main())
