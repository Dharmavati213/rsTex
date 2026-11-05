#!/usr/bin/env -S python3 -I
"""speed.py --engine PATH: what a build costs on this system, as info lines, and no gate: a speed bug that gives no wrong output shows nowhere else.  The macOS one where every run rebuilt its format because /dev/null's times changed is the kind.

LaTeX hello, a document of 50 pages and one of 40 tikz-cd diagrams, each cold and warm, by the wall clock from the caller, as an editor sees it, with that system's own pdfTeX beside them.  Under them what they are made of: how long the system takes
to start a process, to look at a file, to make one, to rename one, and to read back what a run reads, and each program's time in itself and in the system, its page faults and its peak.  Each is the minimum and the median of some runs.
On Windows, with real-time scanning, if it can be turned on, which a runner has off and a user's machine has on.
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


def usage(argv, cwd, env):
    """(user seconds, system seconds, page faults, peak bytes) of one run of a program."""
    p = subprocess.Popen(argv, cwd=cwd, env=env, stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    if not c.WINDOWS:
        _, _, u = os.wait4(p.pid, 0)
        p.returncode = 0
        return u.ru_utime, u.ru_stime, u.ru_minflt + u.ru_majflt, u.ru_maxrss * (1 if c.MACOS else 1024)
    import ctypes
    from ctypes import wintypes

    class Counters(ctypes.Structure):
        _fields_ = [("cb", wintypes.DWORD), ("PageFaultCount", wintypes.DWORD), ("PeakWorkingSetSize", ctypes.c_size_t), ("WorkingSetSize", ctypes.c_size_t), ("QuotaPeakPagedPoolUsage", ctypes.c_size_t), ("QuotaPagedPoolUsage", ctypes.c_size_t),
                    ("QuotaPeakNonPagedPoolUsage", ctypes.c_size_t), ("QuotaNonPagedPoolUsage", ctypes.c_size_t), ("PagefileUsage", ctypes.c_size_t), ("PeakPagefileUsage", ctypes.c_size_t)]

    p.wait()
    h = wintypes.HANDLE(int(p._handle))
    k = ctypes.windll.kernel32
    times = [wintypes.FILETIME() for _ in range(4)]
    k.GetProcessTimes(h, *[ctypes.byref(t) for t in times])
    in_s = lambda t: ((t.dwHighDateTime << 32) + t.dwLowDateTime) / 1e7
    m = Counters()
    m.cb = ctypes.sizeof(m)
    ctypes.windll.psapi.GetProcessMemoryInfo(h, ctypes.byref(m), m.cb)
    return in_s(times[3]), in_s(times[2]), m.PageFaultCount, m.PeakWorkingSetSize


def usage_line(name, argv, cwd, env):
    runs = [usage(argv, cwd, env) for _ in range(5)]
    best = min(runs, key=lambda r: r[0] + r[1])
    c.info(f"{name}: {best[0] * 1e3:.1f} ms in itself, {best[1] * 1e3:.1f} ms in the system, {best[2]} page faults, peak {best[3] >> 20} MiB")


def documents(root):
    d = {}
    for name, text in (("LaTeX hello", "\\documentclass{article}\\begin{document}Hello.\\end{document}\n"), ("50 pages", c.long_document(50)), ("40 tikz-cd diagrams", c.diagrams_document(40))):
        work = c.fresh(os.path.join(root, name.replace(" ", "-")))
        c.write(os.path.join(work, "doc.tex"), text)
        d[name] = work
    return d


def builds(engine, latex, root, prefix=""):
    root = c.fresh(os.path.join(root, "scanned")) if prefix else root
    env = c.base_env(os.path.join(root, "cache"))
    env["RSTEX_CHECKPOINT_POLICY"] = "always"
    for name, work in documents(root).items():
        name = prefix + name
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
        usage_line(f"{name}: rstex warm", [engine, "-progname=pdflatex", *flags], work, env)
        usage_line(f"{name}: rstex cold", [engine, "--no-snapshot", "-progname=pdflatex", *flags], work, env)
        if latex:
            clean()
            ref = []
            for _ in range(n):
                clean()
                ref += timed(one([latex, *flags]), 1)
            c.info(f"{name}: pdfTeX: {shown(ref)}")
            usage_line(f"{name}: pdfTeX", [latex, *flags], work, env)


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


def powershell(command):
    return subprocess.run(["powershell", "-NoProfile", "-Command", command], capture_output=True, text=True, check=False)


def defender(engine, latex, root):
    """Whether anything is looking at each file here, which a user's machine has and a runner may not, and if that can be turned on, what it costs."""
    if not c.WINDOWS:
        return
    status = "Get-MpComputerStatus | Select-Object -Property RealTimeProtectionEnabled,AntivirusEnabled,AMRunningMode | Format-List; Get-MpPreference | Select-Object -Property DisableRealtimeMonitoring,ExclusionPath | Format-List"
    r = powershell(status)
    c.info(f"Windows Defender here: {' '.join(r.stdout.split())} {r.stderr.strip()[:200]}")
    r = powershell("Set-MpPreference -DisableRealtimeMonitoring $false; Remove-MpPreference -ExclusionPath 'C:\\','D:\\','C:\\texlive','D:\\a' -ErrorAction SilentlyContinue")
    r2 = powershell(status)
    c.info(f"with real-time scanning asked for: {' '.join(r2.stdout.split())} {r.stderr.strip()[:200]}")
    if "RealTimeProtectionEnabled : True" not in r2.stdout:
        c.info("real-time scanning does not come on here, so its cost is not measured")
        return
    builds(engine, latex, root, "with real-time scanning: ")


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
    c.guarded("defender", defender, engine, c.tool("pdflatex"), root)
    c.rmtree(root)
    return c.finish("speed, no gate")


if __name__ == "__main__":
    sys.exit(main())
