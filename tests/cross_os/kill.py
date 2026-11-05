#!/usr/bin/env -S python3 -I
"""kill.py --engine PATH: what an editor does to a build that has gone stale, and what a viewer does to a PDF.

  A kill     a build is killed at some moment, as Child::kill does it (SIGKILL; TerminateProcess on Windows), and the next one starts at once.  What that build leaves, chain and cache and all, must give the bytes a cold
             run gives: with a checkpoint and with page records, a kill in each part of a run, in turn on two versions of the document.  Nothing may be left in the output directory that a build does not leave.
  A later pass  --until-stable starts the next pass as a new process where it has to, which is Windows: a kill of the process the editor started has to end it too, or it goes on writing while the next build runs.
  A viewer   on Windows a file open elsewhere may not be renamed over, or written to, by what the opener shared.  Each way a viewer can hold a PDF, against what pdfTeX does there.  What is not refused gives the bytes
             of a cold run; what is refused says so and has status 1; and with the file let go the next build gives the bytes of a cold run.
  The cache  on Windows too, a run that finds a file of the cache held: it goes without, and the bytes are the same.
"""
import argparse
import ctypes
import os
import re
import subprocess
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import common as c  # noqa: E402

FILE_SHARE_READ, FILE_SHARE_WRITE, FILE_SHARE_DELETE = 1, 2, 4


def hold(path, share):
    """As a viewer holds it: a handle to read with, and these sharing flags."""
    k = ctypes.windll.kernel32
    k.CreateFileW.restype = ctypes.c_void_p
    k.CreateFileW.argtypes = [ctypes.c_wchar_p, ctypes.c_uint32, ctypes.c_uint32, ctypes.c_void_p, ctypes.c_uint32, ctypes.c_uint32, ctypes.c_void_p]
    h = k.CreateFileW(path, 0x80000000, share, None, 3, 0, None)
    if h in (None, ctypes.c_void_p(-1).value):
        raise OSError(f"cannot hold {path}: {ctypes.GetLastError()}")
    return h


def let_go(h):
    k = ctypes.windll.kernel32
    k.CloseHandle.argtypes = [ctypes.c_void_p]
    k.CloseHandle(h)


class Build:
    def __init__(self, engine, root, name, text, flags, env=None):
        self.engine, self.flags = engine, flags
        self.work = c.fresh(os.path.join(root, name))
        self.out = os.path.join(self.work, "o")
        self.cache = os.path.join(root, "cache")
        self.env = c.base_env(self.cache)
        self.env.update(RSTEX_CHECKPOINT_TRACE="1", RSTEX_CHECKPOINT_POLICY="always")
        self.env.update(env or {})
        self.main = os.path.join(self.work, "doc.tex")
        self.text = text
        c.write(self.main, text)

    def argv(self, extra=()):
        return [self.engine, *extra, "-progname=pdflatex", *self.flags, "-output-directory=o", "doc.tex"]

    def run(self, extra=()):
        os.makedirs(self.out, exist_ok=True)
        return c.run(self.argv(extra), self.work, self.env)

    def start(self, stderr):
        os.makedirs(self.out, exist_ok=True)
        return subprocess.Popen(self.argv(), cwd=self.work, env=self.env, stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=stderr)

    def cold(self):
        """What a cold run makes of the document as it stands, from nothing."""
        c.rmtree(self.out)
        r = self.run(["--no-snapshot"])
        s = c.state(self.out)
        c.rmtree(self.out)
        if r[0] != 0:
            raise RuntimeError(f"a cold run has status {r[0]}: {r[2][-500:]!r}")
        return s


def word(text, which):
    return text.replace("\\end{document}", f"\nA last word {which}.\n\\end{{document}}")


def kills(engine, root, label, flags, env):
    """A kill at each of many moments, and a build at once after each."""
    text = c.long_document(30)
    b = Build(engine, root, "kill-" + label, text, flags, env)
    versions = [word(text, "one"), word(text, "two")]
    colds = []
    for v in versions:
        c.write(b.main, v)
        colds.append(b.cold())
    c.forget_runs(b.cache)
    # A cold run, with its checkpoint, and how long a run with a change takes: what a kill has to fall inside.
    c.write(b.main, versions[0])
    b.run()
    c.write(b.main, versions[1])
    t0 = time.perf_counter()
    b.run()
    took = time.perf_counter() - t0
    c.info(f"{label}: a rebuild with a change takes {took:.3f} s")
    killed = 0
    bad = []
    afters = []
    fractions = [0.01, 0.03, 0.06, 0.1, 0.15, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9, 0.95, 1.0, 1.05, 0.02, 0.05, 0.12, 0.25, 0.45, 0.65, 0.85, 0.99]
    for i, f in enumerate(fractions):
        v = i % 2
        c.write(b.main, versions[v])
        with open(os.path.join(root, "kill-stderr"), "wb") as err:
            p = b.start(err)
            time.sleep(f * took)
            ended = p.poll() is not None
            p.kill()
            p.wait()
        killed += not ended
        r = b.run()
        now = c.state(b.out)
        d = c.first_difference(colds[v], now)
        stable = c.STABLE.findall(r[2])
        if r[0] != 0 or d:
            # The one way TeX itself fails after a kill: an .aux cut short in the middle of a line, as pdfTeX leaves it too.  An error, and the next build finds the .aux that one wrote.
            cut = r[0] == 1 and stable and stable[-1][1] == b"error"
            r2 = b.run()
            now2 = c.state(b.out)
            d2 = c.first_difference(colds[v], now2)
            if cut and r2[0] == 0 and not d2:
                afters.append(f"{f}: an error, and then the bytes")
                continue
            bad.append(f"after a kill at {f} of its time: status {r[0]}, then {r2[0]}: {d2 or d}: {r[2][-300:]!r}")
    c.say(f"{label}: a build after a kill gives what a cold run gives, {len(fractions)} kills, {killed} of them before the end", not bad and killed >= len(fractions) // 2, "; ".join(bad[:3]))
    for a in afters:
        c.info(f"{label}: {a}")


def later_pass(engine, root):
    """A kill of the process an editor started, while a later pass is running."""
    b = Build(engine, root, "later-pass", c.long_document(30), c.EDITOR, None)
    c.rmtree(b.out)
    err_name = os.path.join(root, "later-stderr")
    with open(err_name, "wb") as err:
        p = b.start(err)
        t0 = time.perf_counter()
        while b"pass=1 changed=" not in c.read(err_name) and p.poll() is None and time.perf_counter() - t0 < 300:
            time.sleep(0.002)
        passed_one = time.perf_counter() - t0
        reached = p.poll() is None
        p.kill()
        p.wait()
    if not reached:
        c.say("a kill reaches a later pass", False, f"the first pass did not get as far: {c.read(err_name)[-400:]!r}")
        return
    # Whoever was running has had as long as a whole pass to show that it still is.
    seen = c.state(b.out)
    time.sleep(max(1.0, 2 * passed_one))
    now = c.state(b.out)
    c.say("a kill reaches a later pass", seen == now, f"first pass {passed_one:.2f} s; afterwards the output directory {'did not change' if seen == now else 'went on changing: ' + str(sorted(k for k in set(seen) | set(now) if seen.get(k) != now.get(k)))}")
    # And the status: a kill of the one that waits is the status of a kill.
    want = b.cold()
    r = b.run()
    c.say("a build after a kill of a later pass", r[0] == 0 and not c.first_difference(want, c.state(b.out)), f"status {r[0]}, {c.first_difference(want, c.state(b.out))}")


def viewer(engine, root):
    """Each way a viewer can hold the PDF while a build has a new one to write, and pdfTeX in the same place."""
    if not c.WINDOWS:
        print("n/a  a PDF held by a viewer: a rename goes over a file in use here")
        return
    latex = c.tool("pdflatex")
    text = "\\documentclass{article}\\begin{document}Hello.\\end{document}\n"
    b = Build(engine, root, "viewer", text, c.EDITOR, None)
    old = b.cold()
    pdf = os.path.join(b.out, "doc.pdf")
    c.write(b.main, text.replace("Hello.", "Hello, here."))
    new = b.cold()
    ways = [("nothing", 0, False), ("reading", FILE_SHARE_READ, False), ("reading and writing", FILE_SHARE_READ | FILE_SHARE_WRITE, True), ("reading and deleting", FILE_SHARE_READ | FILE_SHARE_DELETE, False),
            ("reading, writing and deleting", FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE, True)]
    for name, share, writes in ways:
        c.put_back(b.out, old)
        h = hold(pdf, share)
        try:
            r = b.run()
        finally:
            let_go(h)
        said = c.STABLE.findall(r[2])
        lines = [s for s in r[1].splitlines() if s.startswith(b"!") or b"doc.tex:" in s]
        now = c.state(b.out)
        c.info(f"a viewer that shares {name}: status {r[0]}, {said[-1] if said else None}, says {lines[:2]!r}, stderr {r[2][-200:]!r}, leaves {sorted(now)}")
        if writes:
            c.say(f"a viewer that shares {name}: the new bytes", r[0] == 0 and not c.first_difference(new, now), f"status {r[0]}, {c.first_difference(new, now)}")
        else:
            c.say(f"a viewer that shares {name}: refused, and it says so", r[0] == 1 and bool(lines) and now.get("doc.pdf") == old["doc.pdf"], f"status {r[0]}, {lines[:1]!r}")
        if latex:
            c.put_back(b.out, old)
            h = hold(pdf, share)
            try:
                o = c.run([latex, "-synctex=1", "-interaction=nonstopmode", "-file-line-error", "-recorder", "-output-directory=o", "doc.tex"], b.work, c.base_env(b.cache))
            finally:
                let_go(h)
            c.say(f"a viewer that shares {name}: the status pdfTeX has", o[0] == r[0], f"pdfTeX {o[0]}, rstex {r[0]}, pdfTeX says {[s for s in o[1].splitlines() if s.startswith(b'!') or b'doc.tex:' in s][:2]!r}")
        # Nothing of this stays, with the file let go.
        c.put_back(b.out, now if not writes else old)
        r = b.run()
        c.say(f"a viewer that shares {name}: let go, and the next build", r[0] == 0 and not c.first_difference(new, c.state(b.out)), f"status {r[0]}, {c.first_difference(new, c.state(b.out))}")


def viewer_during(engine, root):
    """A viewer that comes to the file while a build runs: past the first page, the file was open to write when it looked.  So it is the end of the build that finds the file held, and only there."""
    if not c.WINDOWS:
        return
    text = c.long_document(30)
    b = Build(engine, root, "viewer-during", text, c.EDITOR, None)
    c.write(b.main, word(text, "one"))
    new = b.cold()
    c.write(b.main, text)
    b.run()
    t0 = time.perf_counter()
    b.run()
    took = time.perf_counter() - t0
    pdf = os.path.join(b.out, "doc.pdf")
    c.write(b.main, word(text, "one"))
    with open(os.path.join(root, "viewer-stderr"), "wb") as err:
        p = b.start(err)
        time.sleep(took * 0.5)
        h = hold(pdf, FILE_SHARE_READ)
        out = p.wait()
        let_go(h)
    left = sorted(os.listdir(b.out))
    c.info(f"a viewer that comes during a build: status {out}, leaves {left}; its stderr {c.read(os.path.join(root, 'viewer-stderr'))[-300:]!r}")
    c.say("a viewer that comes during a build: refused with a status", out != 0)
    r = b.run()
    now = c.state(b.out)
    c.say("a viewer that comes during a build: the next build", r[0] == 0 and not c.first_difference(new, now), f"status {r[0]}, {c.first_difference(new, now)}")


def side_files_held(engine, root):
    """A side file that something has open, as a log tailed by another program or a SyncTeX file a viewer is reading.  What TeX does there is pdfTeX's business, and the same status and the same files are what is asked."""
    if not c.WINDOWS:
        return
    latex = c.tool("pdflatex")
    text = "\\documentclass{article}\\begin{document}Hello.\\end{document}\n"
    b = Build(engine, root, "side-files", text, c.EDITOR, None)
    old = b.cold()
    flags = ["-synctex=1", "-interaction=nonstopmode", "-file-line-error", "-recorder", "-output-directory=o", "doc.tex"]
    for name, share in (("doc.log", 0), ("doc.log", FILE_SHARE_READ), ("doc.synctex.gz", 0), ("doc.synctex.gz", FILE_SHARE_READ), ("doc.aux", 0), ("doc.fls", 0)):
        results = []
        for argv, env in (([latex, *flags], c.base_env(b.cache)), ([engine, "-progname=pdflatex", *flags], b.env)):
            c.put_back(b.out, old)
            h = hold(os.path.join(b.out, name), share)
            try:
                r = c.run(argv, b.work, env)
            finally:
                let_go(h)
            results.append((r[0], sorted(os.listdir(b.out)), [s for s in r[1].splitlines() if s.startswith(b"!") or b"SyncTeX" in s][:2], r[2][-200:]))
        same = results[0][0] == results[1][0] and results[0][1] == results[1][1]
        c.say(f"{name} held, sharing {share}: as pdfTeX", same, f"pdfTeX {results[0]}; rstex {results[1]}")
        # And nothing of it stays.
        c.put_back(b.out, old)
        r = b.run()
        c.say(f"{name} held, sharing {share}: let go, and the next build", r[0] == 0 and not c.first_difference(old, c.state(b.out)), f"status {r[0]}, {c.first_difference(old, c.state(b.out))}")


def cache_held(engine, root):
    """A file of the cache that another program, or another run, has open with no sharing."""
    if not c.WINDOWS:
        print("n/a  a file of the cache held: a rename goes over a file in use here")
        return
    b = Build(engine, root, "cache-held", "\\documentclass{article}\\begin{document}Hello.\\end{document}\n", c.EDITOR, None)
    want = b.cold()
    b.run()
    b.run()
    d = c.cache_dir(b.cache)
    kinds = {}
    for top, _, names in os.walk(d):
        for n in names:
            kinds.setdefault(re.sub(r"[0-9a-f]{16}", "H", n), os.path.join(top, n))
    for kind, path in sorted(kinds.items()):
        if not any(x in kind for x in (".ck", ".oc", ".fmt", "kpse-index", "kpse-tree", "map-", "seen", ".deps")):
            continue
        try:
            h = hold(path, 0)
        except OSError as e:
            c.info(f"cannot hold {kind}: {e}")
            continue
        try:
            r = b.run()
        finally:
            let_go(h)
        # With the format held the run builds its own, beside its outputs, and the recorder says which it read.
        now = {k: v for k, v in c.state(b.out).items() if not (k.endswith(".fls") and ".fmt" in kind)}
        d2 = c.first_difference({k: v for k, v in want.items() if k in now}, now)
        c.say(f"a file of the cache held, {kind}", r[0] == 0 and not d2, f"status {r[0]}, {d2}, {r[2][-200:] if r[0] else ''}")
    # And it goes on as before.  One run may take a checkpoint anew, after the run that had a format of its own.
    b.run()
    r = b.run()
    c.say("a file of the cache let go", r[0] == 0 and c.RESTORED in r[2], f"status {r[0]}, {r[2][-200:]!r}")


def two_at_once(engine, root):
    """A build that has not ended when the next one starts: an editor waits, but a second window may not.  Neither may take the other down, and a third gives the bytes."""
    b = Build(engine, root, "two", c.long_document(10), c.EDITOR, None)
    want = b.cold()
    b.run()
    with open(os.devnull, "wb") as null:
        ps = [b.start(null), b.start(null)]
        rcs = [p.wait() for p in ps]
    r = b.run()
    c.say("two builds at once, and a third", r[0] == 0 and not c.first_difference(want, c.state(b.out)), f"statuses {rcs}, then {r[0]}, {c.first_difference(want, c.state(b.out))}")


def main():
    c.start()
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--engine", required=True)
    ap.add_argument("--label", default="here")
    a = ap.parse_args()
    engine = c.program(a.engine)
    root = c.fresh(os.path.join(c.OUT, a.label, "kill"))
    no_sync = [f for f in c.EDITOR if not f.startswith("-synctex")]
    c.guarded("kills", kills, engine, root, "as an editor has it", c.EDITOR, None)
    c.guarded("kills", kills, engine, root, "with page records", no_sync, {"RSTEX_PAGE_CK": "1"})
    c.guarded("later pass", later_pass, engine, root)
    c.guarded("viewer", viewer, engine, root)
    c.guarded("viewer during", viewer_during, engine, root)
    c.guarded("side files held", side_files_held, engine, root)
    c.guarded("cache held", cache_held, engine, root)
    c.guarded("two at once", two_at_once, engine, root)
    return c.finish("kills and viewers")


if __name__ == "__main__":
    sys.exit(main())
