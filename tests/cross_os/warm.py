#!/usr/bin/env -S python3 -I
"""warm.py --engine PATH: a warm build is a cold one, as an editor runs them: --until-stable -synctex=1 -interaction=nonstopmode -file-line-error -recorder -output-directory=DIR.

Of public documents, from tests/cross_os and tests/ladder, in one directory that keeps its path, each time every file the output directory holds, byte for byte, against a cold run from the same state:

  cold      --no-snapshot, from nothing
  write     the same, from nothing and with no checkpoint: the run that writes one
  rebuild   with all that the last build left there, the PDF too: a LaTeX document has to say "restored", and every stream has to come from the old PDF
  sealed    the same once the checkpoint is old enough for a stat to show any change
  edited    with a word more near the end, from what the last build left, against --no-snapshot from that state
  undone    the word out again: the old state's answer, from a PDF that holds the other

and, without -synctex=1 and with a record at every page, page records, which a restored run keeps: a chain, a run from its last record, and an edit near the end, all of them against cold.  This is what `commit_range`, private mappings
of 16 KiB pages, a system with no mappings, and a file's identity without an inode come to, on real documents.
"""
import argparse
import re
import os
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import common as c  # noqa: E402

# A little of each kind, not the longest.
CHOSEN = ("article", "pictures", "plain", "00-plain-hello", "04-latex-hello", "05-latex-sample2e", "07-amsmath-suite", "12-graphics", "13-hyperref", "14-tikzcd-one", "17-class-amsart")
RECORDS = re.compile(rb"rstex: checkpoint: pages: (\d+) records, using (\d+)")


def edited(text):
    """The same document with a word more at its end, or None if there is no place to put one."""
    for end in (b"\\end{document}", b"\\bye", b"\\end"):
        at = text.rfind(end)
        if at >= 0:
            return text[:at] + b"\nOne more word.\n" + text[at:]
    return None


def stable_of(err):
    m = c.STABLE.findall(err)
    return m[-1] if m else None


def cache_says(err):
    """What the trace says a run took from the old PDF."""
    m = c.CACHE_LINE.findall(err)
    if not m:
        return {}
    w = m[-1].decode("latin-1").split()
    return {w[i]: int(w[i + 1]) for i in range(0, len(w) - 1, 2)}


class Doc:
    def __init__(self, engine, name, source, extras, root):
        self.engine, self.name = engine, name
        self.work = c.fresh(os.path.join(root, name))
        self.out = os.path.join(self.work, "o")
        self.main = os.path.basename(source)
        self.text = c.read(source)
        self.path = os.path.join(self.work, self.main)
        c.write(self.path, self.text)
        for x in extras:
            c.write(os.path.join(self.work, x), c.read(os.path.join(os.path.dirname(source), x)))
        self.format = c.format_of(source)
        self.cache = os.path.join(root, "cache")

    def run(self, flags, extra=(), env=None, text=None):
        if text is not None:
            c.write(self.path, text)
        os.makedirs(self.out, exist_ok=True)
        e = c.base_env(self.cache)
        e.update(RSTEX_CHECKPOINT_TRACE="1", RSTEX_OUTCACHE_TRACE="1", RSTEX_CHECKPOINT_POLICY="always")
        e.update(env or {})
        return c.run([self.engine, *extra, "-progname=" + self.format, *flags, "-output-directory=o", self.main], self.work, e)

    def judge(self, what, r, want, restored=None, finds_all=False):
        """`r` left what `want` holds."""
        name = f"{self.name}: {what}"
        problems = []
        status, _, err = r
        if status != 0:
            problems.append(f"status {status}: {err[-400:]!r}")
        now = c.state(self.out)
        d = c.first_difference(want, now)
        if d:
            problems.append(d)
        if restored is True and c.RESTORED not in err:
            problems.append(f"not restored: {err[-400:]!r}")
        said = stable_of(err)
        if said is None or said[1] != b"stable":
            problems.append(f"the stable line: {said}")
        cache = cache_says(err)
        if finds_all and cache.get("hits", 0) + cache.get("behind", 0) != cache.get("streams", 0):
            problems.append(f"found less than all in the old PDF: {cache}")
        return c.say(name, not problems, "; ".join(problems) if problems else f"passes {said[0].decode()}, {len(now)} files")


def flow(engine, name, source, extras, root):
    d = Doc(engine, name, source, extras, root)
    latex = d.format == "pdflatex"
    # Cold, from nothing.
    r = d.run(c.EDITOR, ["--no-snapshot"])
    cold = c.state(d.out)
    if r[0] != 0 or not c.say(f"{name}: cold", stable_of(r[2]) is not None and stable_of(r[2])[1] == b"stable", f"status {r[0]}, {stable_of(r[2])}"):
        c.info(f"{name}: what a cold run said: {r[2][-600:]!r}")
        return
    if not any(n.endswith(".synctex.gz") for n in cold):
        c.say(f"{name}: a SyncTeX file", False, str(sorted(cold)))
    # The run that writes.
    c.forget_runs(d.cache)
    c.rmtree(d.out)
    d.judge("the run that writes a checkpoint", d.run(c.EDITOR), cold)
    d.judge("a rebuild with everything in place", d.run(c.EDITOR), cold, restored=latex or None, finds_all=True)
    # RACY_NS of checkpoint.rs, and a little: a file's stamp from the same moment as the checkpoint says nothing.
    time.sleep(2.2)
    d.run(c.EDITOR)
    d.judge("a sealed rebuild", d.run(c.EDITOR), cold, restored=latex or None, finds_all=True)
    # An edit.
    new = edited(d.text)
    if new is None:
        return
    c.write(d.path, new)
    c.put_back(d.out, cold)
    d.run(c.EDITOR, ["--no-snapshot"])
    cold_new = c.state(d.out)
    c.put_back(d.out, cold)
    # Whether it restores is the document's: a class that has read the main file past the place cannot.
    d.judge("an edit near the end", d.run(c.EDITOR), cold_new)
    c.put_back(d.out, cold_new)
    d.judge("the edit undone", d.run(c.EDITOR, text=d.text), cold)


def records(engine, root):
    """Page records, on a document of some length."""
    source = os.path.join(root, "chain-source", "chain.tex")
    c.write(source, c.long_document(12))
    d = Doc(engine, "chain", source, (), root)
    flags = [f for f in c.EDITOR if not f.startswith("-synctex")]
    every = {"RSTEX_PAGE_CK": "1"}
    d.run(flags, ["--no-snapshot"])
    cold = c.state(d.out)
    c.forget_runs(d.cache)
    d.judge("page records: the run that writes", d.run(flags, env=every), cold)
    r = d.run(flags, env=every)
    d.judge("page records: the chain", r, cold, restored=True)
    r = d.run(flags, env=every)
    m = RECORDS.findall(r[2])
    d.judge("page records: a run from the last", r, cold, restored=True)
    c.say("chain: page records were made and used", bool(m) and int(m[-1][0]) > 0 and int(m[-1][1]) > 0, str(m))
    c.info(f"chain: a trace says {[s for s in r[2].splitlines() if b'pages' in s][:3]!r}")
    # An edit near the end: all the records before it hold.
    new = edited(d.text)
    c.write(d.path, new)
    c.put_back(d.out, cold)
    d.run(flags, ["--no-snapshot"])
    cold_new = c.state(d.out)
    c.put_back(d.out, cold)
    r = d.run(flags, env=every)
    m = RECORDS.findall(r[2])
    d.judge("page records: an edit near the end", r, cold_new, restored=True)
    c.say("chain: the edit went on from a record", any(int(x[1]) > 0 for x in m), str(m))
    # Under a lock that is already held: a run goes without, and the bytes are the same.
    c.put_back(d.out, cold_new)
    d.judge("page records: the run after the edit", d.run(flags, env=every), cold_new, restored=True)


def main():
    c.start()
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--engine", required=True)
    ap.add_argument("--label", default="here")
    ap.add_argument("--only")
    a = ap.parse_args()
    engine = c.program(a.engine)
    root = c.fresh(os.path.join(c.OUT, a.label, "warm"))
    for name, source, extras in c.documents():
        if name in CHOSEN and (not a.only or re.search(a.only, name)):
            c.guarded(name, flow, engine, name, source, extras, root)
    if not a.only or a.only == "chain":
        c.guarded("chain", records, engine, root)
    return c.finish("warm equals cold")


if __name__ == "__main__":
    sys.exit(main())
