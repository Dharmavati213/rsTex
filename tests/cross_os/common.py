"""What the tools of tests/cross_os share: where things go, how an editor runs a build, how each system does what Unix has one way of doing.  Everything is written under target/cross-os, from
documents that are written here or are in this repository, and nothing else is ever compiled.
"""
import ctypes
import gzip
import hashlib
import importlib.util
import os
import platform
import re
import shutil
import subprocess
import sys
import time
import zlib

sys.dont_write_bytecode = True

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(os.path.dirname(HERE))
OUT = os.path.join(ROOT, "target", "cross-os")
LADDER = os.path.join(ROOT, "tests", "ladder")
WINDOWS = platform.system() == "Windows"
MACOS = platform.system() == "Darwin"
EXE = ".exe" if WINDOWS else ""
FIXED = {"SOURCE_DATE_EPOCH": "1700000000", "FORCE_SOURCE_DATE": "1", "TZ": "UTC"}
# What an editor gives a build, but for the output directory.
EDITOR = ["--until-stable", "-synctex=1", "-interaction=nonstopmode", "-file-line-error", "-recorder"]
STABLE = re.compile(rb"^rstex: until-stable: passes=(\d+) result=(\w+)", re.M)
TOOL = re.compile(rb"^rstex: until-stable: tool=(\S+) reason=(\S+) file=(.*?)\r?$", re.M)
CACHE_LINE = re.compile(rb"^rstex: outcache: (.*)$", re.M)
RESTORED = b"rstex: checkpoint: restored"
failed = []
passed = 0


def start():
    # What a run prints can hold anything, and a Windows runner's standard output is in a code page that has not all of it.
    sys.stdout.reconfigure(encoding="utf-8", errors="replace")
    if WINDOWS:
        # SEM_FAILCRITICALERRORS | SEM_NOGPFAULTERRORBOX | SEM_NOOPENFILEERRORBOX, which a child takes over: a program that dies waits for no one to close a box.
        ctypes.windll.kernel32.SetErrorMode(0x8003)


def say(name, ok, what=""):
    global passed
    print(f"{'ok  ' if ok else 'FAIL'} {name}{': ' + what if what else ''}", flush=True)
    if ok:
        passed += 1
    else:
        failed.append(name)
    return ok


def info(what):
    print(f"info {what}", flush=True)


def finish(title):
    print(f"{title}: {passed} of {passed + len(failed)} hold")
    for name in failed[:60]:
        print(f"  FAIL {name}")
    return 1 if failed else 0


def guarded(name, act, *args):
    """A section that breaks says so and the others go on: a run on these systems costs enough to want all it can say."""
    try:
        return act(*args)
    except Exception as e:  # a tool's own fault, or a system that answers otherwise than was thought
        say(f"{name} ran to its end", False, f"{type(e).__name__}: {e}")
        return None


def program(path):
    p = os.path.abspath(path)
    return p + EXE if WINDOWS and not p.lower().endswith(EXE) else p


def tool(name):
    """A TeX Live program, or another on the PATH, by its whole name, or None."""
    return shutil.which(name)


def cache_env(base):
    """What puts the cache directory under `base`: DESIGN.md section 12 has a rule for each system, and only Linux reads XDG_CACHE_HOME."""
    if WINDOWS:
        return {"LOCALAPPDATA": base}
    if MACOS:
        return {"HOME": base}
    return {"XDG_CACHE_HOME": base}


def cache_dir(base):
    """The directory that base gives, by DESIGN.md section 12's rule."""
    if WINDOWS:
        return os.path.join(base, "rstex", "cache")
    if MACOS:
        return os.path.join(base, "Library", "Caches", "rstex")
    return os.path.join(base, "rstex")


def forget_runs(base):
    """Everything in the cache but the format and the tree's tables, which cost seconds and say nothing here."""
    for name in ("checkpoints", "outcache"):
        rmtree(os.path.join(cache_dir(base), name))


def retrying(act, *args):
    """A file system call that, on Windows, can meet a handle that has not gone yet: a process that just ended, or a scanner looking at what it left."""
    for delay in (0.02, 0.05, 0.1, 0.2, 0.5, 1, 2) if WINDOWS else ():
        try:
            return act(*args)
        except PermissionError:
            time.sleep(delay)
    return act(*args)


def rmtree(path):
    if os.path.lexists(long_name(path)):
        retrying(shutil.rmtree, long_name(path))


def long_name(path):
    """A directory that may be longer than MAX_PATH, for Python's own calls: a test makes what the program under test is asked to find."""
    if WINDOWS and not path.startswith("\\\\?\\"):
        return "\\\\?\\" + os.path.abspath(path).replace("/", "\\")
    return path


def fresh(path):
    rmtree(path)
    os.makedirs(long_name(path))
    return path


def write(path, data):
    if isinstance(data, str):
        data = data.encode("utf-8")
    os.makedirs(os.path.dirname(long_name(path)), exist_ok=True)
    with open(long_name(path), "wb") as f:
        f.write(data)


def read(path):
    with open(long_name(path), "rb") as f:
        return f.read()


def state(d):
    """Relative name to bytes, of all that a directory holds.  The name with slashes, on every system."""
    res = {}
    top = long_name(d)
    for root, _, names in os.walk(top):
        for n in names:
            p = os.path.join(root, n)
            res[os.path.relpath(p, top).replace(os.sep, "/")] = read(p)
    return res


def put_back(d, s):
    """The directory holds what `s` says and nothing else."""
    fresh(d)
    for name, data in s.items():
        write(os.path.join(d, name), data)


def first_difference(a, b):
    """Which files of two states differ, and how, shortly."""
    names = sorted(k for k in set(a) | set(b) if a.get(k) != b.get(k))
    if not names:
        return None
    out = []
    for k in names[:4]:
        if k not in a or k not in b:
            out.append(f"{k} only in the {'second' if k not in a else 'first'}")
            continue
        x, y = text_of(k, a[k]), text_of(k, b[k])
        where = next((i for i, (p, q) in enumerate(zip(x.split(b"\n"), y.split(b"\n"))) if p != q), None)
        if where is None:
            out.append(f"{k}: {len(x)} and {len(y)} bytes")
        else:
            out.append(f"{k}: line {where + 1}: {x.split(bytes([10]))[where][:150]!r} and {y.split(bytes([10]))[where][:150]!r}")
    return "; ".join(out) + (f"; and {len(names) - 4} more" if len(names) > 4 else "")


def text_of(name, data):
    """What is compared in a report of what differs: the text inside a gzip file."""
    if name.endswith(".gz"):
        try:
            return gzip.decompress(data)
        except (OSError, EOFError, zlib.error):
            pass
    return data


def members(data):
    """How many gzip members a file has, by walking them."""
    n, at = 0, 0
    while at < len(data):
        d = zlib.decompressobj(31)
        d.decompress(data[at:])
        n += 1
        at = len(data) - len(d.unused_data)
    return n


def sha(data):
    return hashlib.sha256(data).hexdigest()


def run(argv, cwd, env, stdin=None, timeout=600):
    """A program with its output kept.  (status, stdout, stderr), and a status of None if it was killed at the limit, which goes with what it said by then."""
    e = dict(env)
    p = subprocess.Popen(argv, cwd=cwd, env=e, stdin=subprocess.PIPE if stdin is not None else subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    try:
        out, err = p.communicate(stdin, timeout=timeout)
        return p.returncode, out, err
    except subprocess.TimeoutExpired:
        kill_tree(p)
        out, err = p.communicate()
        return None, out, err


def kill_tree(p):
    """All that a process started as well: taskkill, on Windows, and a process group that has been asked for, on Unix."""
    if WINDOWS:
        subprocess.run(["taskkill", "/T", "/F", "/PID", str(p.pid)], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, check=False)
    p.kill()


def base_env(cache=None):
    env = dict(os.environ, **FIXED)
    env.pop("RSTEX_NO_SNAPSHOT", None)
    if cache is not None:
        env.update(cache_env(cache))
    return env


def format_of(path):
    """Which format a document in this repository is for."""
    with open(path, "rb") as f:
        return "pdflatex" if b"\\documentclass" in f.read() else "pdftex"


def integration():
    """tests/integration/run.py: its masks are what a comparison with the oracle goes by."""
    if "rstex_integration_run" not in sys.modules:
        spec = importlib.util.spec_from_file_location("rstex_integration_run", os.path.join(ROOT, "tests", "integration", "run.py"))
        mod = importlib.util.module_from_spec(spec)
        sys.modules["rstex_integration_run"] = mod
        spec.loader.exec_module(mod)
    return sys.modules["rstex_integration_run"]


def masked(data, name=""):
    """A log or a terminal's text, as tests/integration/run.py compares it: with the banner, a program's name and a PDF's length taken out, and a line with a place in it left to the caller."""
    m = integration()
    drop = re.compile(r"\(\d+ pages?, [^)]*\)")
    return [drop.sub("", s) for s in m.mask_lines(data, {}, True)]


def against_the_oracle(a, b):
    """Why a log of the oracle's and one of the candidate's are not the same, or None."""
    x, y = masked(a), masked(b)
    for i, (p, q) in enumerate(zip(x, y)):
        if p != q:
            return f"line {i + 1}: {p[:150]!r} and {q[:150]!r}"
    return None if len(x) == len(y) else f"{len(x)} lines and {len(y)}"


def documents():
    """Public documents, with a name that says where each is from: (name, path, files that go with it)."""
    out = [(f[:-4], os.path.join(HERE, f), ()) for f in sorted(os.listdir(HERE)) if f.endswith(".tex")]
    for f in sorted(os.listdir(LADDER)):
        if f.endswith(".tex"):
            out.append((f[:-4], os.path.join(LADDER, f), ("ladder-logo.jpg", "ladder-page2.png") if f.startswith("12-") else ()))
    return out


def long_document(pages=50):
    """About `pages` pages, with a table of contents and references, in plain words: what it costs is the engine's and not a package's."""
    text = ["\\documentclass{article}\n\\pdfinfoomitdate=1 \\pdftrailerid{}\n\\begin{document}\n\\tableofcontents\n\\newpage\n"]
    para = "Words of an ordinary paragraph, with a formula $a^2+b^2=c^2$ now and then, and nothing else that a package would have to do; " * 6
    for k in range(pages):
        text.append(f"\\section{{Part {k}}}\\label{{s{k}}}\nSee section \\ref{{s{(k + 1) % pages}}} on page \\pageref{{s{(k + 1) % pages}}}.\n{para}\n\n{para}\n\n{para}\n\n{para}\n\\newpage\n")
    return "".join(text) + "\\end{document}\n"


TIKZCD = "A_{%d} \\arrow[r, \"f\"] \\arrow[d, \"g\"'] & B \\arrow[d, \"h\"] \\\\\nC \\arrow[r, hook, \"k\"'] & D \\arrow[ul, dashed]\n"


def diagrams_document(n=40):
    """`n` tikz-cd diagrams."""
    body = "".join("\\begin{tikzcd}\n" + TIKZCD % k + "\\end{tikzcd}\n\n" for k in range(n))
    return "\\documentclass{article}\n\\pdfinfoomitdate=1 \\pdftrailerid{}\n\\usepackage{tikz-cd}\n\\begin{document}\n" + body + "\\end{document}\n"
