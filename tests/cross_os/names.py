#!/usr/bin/env -S python3 -I
"""names.py --engine PATH: a name with something in it, wherever a name goes.  On Windows a user name with an accent or a space is common, on macOS a name comes back in NFD, and either can be long.

Each name is put in each place in turn, and the build made as an editor makes it, then compared with what pdfTeX does in the same place, where that means anything:

  the document      in a directory of that name and with that name, from there, by its relative name
  the whole path    the same, by an absolute name and an absolute output directory, from elsewhere
  the output        only the output directory has it
  the cache         the cache directory is under it, as under a user's home: a format is built there, a checkpoint written, and a run has to restore it and give the same bytes
  the home          ~/texmf, by the variable each system has for it, holds a package with that name in its path
  a TeX Live        a link of that name to the tree, on the PATH: a user's TeX Live under their home

The names, the long ones on Windows, and NFD against NFC on macOS, are in NAMES below.  What pdfTeX refuses is not asked of rstex, and is said; what pdfTeX does and rstex does not fails.
"""
import argparse
import os
import re
import shutil
import subprocess
import sys
import time
import unicodedata

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import common as c  # noqa: E402

DOC = "\\documentclass{article}\n\\pdfinfoomitdate=1 \\pdftrailerid{}\n\\begin{document}\n\\tableofcontents\n\\section{One}\\label{one}\nHello, see \\ref{one} and \\input{part}\\end{document}\n"
PART = "some words from a part.\n"
PLAIN_FLAGS = ["-synctex=1", "-interaction=nonstopmode", "-file-line-error", "-recorder"]
NAMES = [
    ("a space", "with space"),
    ("accents", "Zo\u00eb M\u00fcller"),
    ("another alphabet", "\u4e2d\u6587 \u0395\u03bb\u03bb"),
    ("outside the BMP", "clef \U0001d11e"),
    ("what a shell reads", "A & B (1)"),
    ("a tilde", "RUNNER~1"),
    ("a quote and a comma", "a'b,c"),
    ("a percent and a hash", "50% #1"),
]
if c.MACOS:
    # As a directory on HFS+, or a program that decomposes, gives it.
    NAMES.append(("NFD", unicodedata.normalize("NFD", "caf\u00e9 \u00fc")))
if c.WINDOWS:
    NAMES.append(("a short name's form", "PROGRA~1"))


def tree_of(pdftex):
    return os.path.dirname(os.path.dirname(os.path.dirname(os.path.realpath(pdftex))))


def compare(what, oracle, cand, where, cand_out=None):
    """Both ran in the same place, the oracle's output put aside.  What pdfTeX refuses is not asked of rstex."""
    (o_status, o_out, _), o_log, o_pdf = oracle
    (r_status, r_out, r_err), r_log, r_pdf = cand
    if o_status != 0 or not o_pdf:
        c.info(f"{what}: pdfTeX has status {o_status} here, and rstex {r_status}{', with a PDF' if r_pdf else ''}: pdfTeX says {[s for s in o_out.splitlines() if s.startswith(b'!')][:1]!r} {oracle[0][1][:300]!r} {oracle[0][2][-300:]!r}, rstex {[s for s in r_out.splitlines() if s.startswith(b'!')][:1]!r} {r_out[:300]!r} {r_err[-300:]!r}")
        return
    problems = []
    if r_status != 0 or not r_pdf:
        problems.append(f"status {r_status}, PDF {'there' if r_pdf else 'missing'}: {r_err[-300:]!r} {r_out[-300:]!r}")
    elif o_log is not None:
        d = c.against_the_oracle(o_log, r_log)
        if d:
            problems.append(f"the log, {d}")
    c.say(what, not problems, "; ".join(problems) or where)


def in_place(engine, latex, what, work, argv_tail, out, env):
    """pdfTeX and then rstex, from the same state, in the same place."""
    log = os.path.join(out, "doc.log" if "doc" in argv_tail[-1] else "")
    runs = []
    for prog, extra in ((latex, []), (engine, ["-progname=pdflatex"])):
        c.rmtree(out)
        os.makedirs(c.long_name(out))
        # Two runs, for the .aux and the table of contents.
        r = None
        for _ in range(2):
            r = c.run([prog, *extra, *PLAIN_FLAGS, *argv_tail], work, env)
        logs = [n for n in os.listdir(c.long_name(out)) if n.endswith(".log")]
        pdfs = [n for n in os.listdir(c.long_name(out)) if n.endswith(".pdf")]
        runs.append((r, c.read(os.path.join(out, logs[0])) if logs else b"", bool(pdfs)))
    compare(what, runs[0], runs[1], log)
    # And as an editor has it: the passes go on over a new process on Windows, by this same command line.
    c.rmtree(out)
    os.makedirs(c.long_name(out))
    r = c.run([engine, "-progname=pdflatex", *c.EDITOR, *argv_tail], work, env)
    said = c.STABLE.findall(r[2])
    if runs[0][0][0] == 0:
        c.say(f"{what}: --until-stable", r[0] == 0 and bool(said) and said[-1][1] == b"stable", f"status {r[0]}, {said}, {r[2][-300:] if r[0] else ''}")


def document(engine, latex, kind, name, root, env):
    d = os.path.join(root, name)
    c.write(os.path.join(d, name + ".tex"), DOC)
    c.write(os.path.join(d, "part.tex"), PART)
    # Relative, from there.
    in_place(engine, latex, f"{kind}: the document, by its relative name", d, ["-output-directory=o", name + ".tex"], os.path.join(d, "o"), env)
    # Absolute, from elsewhere.  With what TeX calls its argument: a name with a space needs quotes of its own in TeX's line, which the program puts there.
    in_place(engine, latex, f"{kind}: the whole path", root, ["-output-directory=" + os.path.join(d, "abs"), os.path.join(d, name + ".tex")], os.path.join(d, "abs"), dict(env, TEXINPUTS=d + os.pathsep))
    # The output directory alone.
    plain = os.path.join(root, "plain-" + re.sub(r"\W", "", kind))
    c.write(os.path.join(plain, "doc.tex"), DOC)
    c.write(os.path.join(plain, "part.tex"), PART)
    in_place(engine, latex, f"{kind}: the output directory", plain, ["-output-directory=" + os.path.join(plain, name), "doc.tex"], os.path.join(plain, name), env)


def cache(engine, kind, name, root, want, doc_dir):
    """The cache under this name, with a format built there, and a checkpoint that restores."""
    base = os.path.join(root, "home " + name)
    env = c.base_env(os.path.join(base, "AppData", "Local") if c.WINDOWS else base)
    env.update(RSTEX_CHECKPOINT_TRACE="1", RSTEX_CHECKPOINT_POLICY="always")
    out = os.path.join(doc_dir, "o")
    runs = []
    for _ in range(3):
        c.rmtree(out)
        os.makedirs(out)
        runs.append(c.run([engine, "-progname=pdflatex", *c.EDITOR, "-output-directory=o", "doc.tex"], doc_dir, env))
    # The recorder names the format, which is where the cache is.
    now = {k: v for k, v in c.state(out).items() if not k.endswith(".fls")}
    d = c.first_difference({k: v for k, v in want.items() if not k.endswith(".fls")}, now)
    c.say(f"{kind}: the cache", runs[2][0] == 0 and c.RESTORED in runs[2][2] and not d, f"status {runs[2][0]}, {'restored' if c.RESTORED in runs[2][2] else 'not restored'}, {d}, {runs[2][2][-300:] if runs[2][0] else ''}")
    return base


def home(engine, latex, kind, name, root, doc_dir):
    """TEXMFHOME by a tilde, as a user's texmf.cnf has it: on Windows that is USERPROFILE, and the package is under it."""
    base = os.path.join(root, "user " + name)
    c.write(os.path.join(base, "texmf", "tex", "latex", "mine", "minepkg.sty"), "\\ProvidesPackage{minepkg}\\newcommand\\mine{from the home}\n")
    env = c.base_env(base if not c.WINDOWS else os.path.join(base, "AppData", "Local"))
    env["TEXMFHOME"] = "~/texmf"
    env["USERPROFILE" if c.WINDOWS else "HOME"] = base
    d = os.path.join(root, "home-doc")
    c.write(os.path.join(d, "doc.tex"), "\\documentclass{article}\\usepackage{minepkg}\\begin{document}\\mine\\end{document}\n")
    results = []
    for prog, extra in ((latex, []), (engine, ["-progname=pdflatex"])):
        for side in ("doc.aux", "doc.log", "doc.pdf"):
            if os.path.exists(os.path.join(d, side)):
                os.unlink(os.path.join(d, side))
        r = c.run([prog, *extra, "-interaction=nonstopmode", "doc.tex"], d, env)
        results.append((r, c.read(os.path.join(d, "doc.log")), r[0] == 0))
    compare(f"{kind}: ~/texmf under the home", results[0], results[1], base)


def tree_link(engine, kind, name, root, pdftex):
    """A TeX Live whose path has the name.  A link to it, first on the PATH, so that kpsewhich and pdfTeX say this name."""
    link = os.path.join(root, "TL " + name)
    tree = tree_of(pdftex)
    try:
        if c.WINDOWS:
            subprocess.run(["cmd", "/c", "mklink", "/J", link, tree], check=True, stdout=subprocess.DEVNULL)
        else:
            os.symlink(tree, link, target_is_directory=True)
    except (OSError, subprocess.CalledProcessError) as e:
        c.info(f"{kind}: no link to the tree: {e}")
        return
    bin_dir = os.path.relpath(os.path.dirname(os.path.realpath(pdftex)), tree)
    env = c.base_env(os.path.join(root, "tree-cache " + re.sub(r"\W", "", kind)))
    env["PATH"] = os.path.join(link, bin_dir) + os.pathsep + env["PATH"]
    d = os.path.join(root, "tree-doc")
    c.write(os.path.join(d, "doc.tex"), "\\documentclass{article}\\begin{document}Hello.\\end{document}\n")
    prog = os.path.join(link, bin_dir, "pdflatex" + c.EXE)
    results = []
    for argv in ([prog], [engine, "-progname=pdflatex"]):
        r = c.run([*argv, "-interaction=nonstopmode", "doc.tex"], d, env, timeout=300)
        results.append((r, None, r[0] == 0))
    compare(f"{kind}: a TeX Live under it", results[0], results[1], link)
    if c.WINDOWS:
        os.rmdir(link)
    else:
        os.unlink(link)


def long_paths(engine, latex, root, want_cache_doc):
    """Longer than MAX_PATH, which a user's name and a project's depth can come to.  What pdfTeX does is said; a document it can compile, rstex has to."""
    if not c.WINDOWS:
        print("n/a  names longer than MAX_PATH: there is no such limit here")
        return
    deep = os.path.join(root, *["d" * 40] * 7)
    c.write(os.path.join(deep, "doc.tex"), DOC)
    c.write(os.path.join(deep, "part.tex"), PART)
    in_place(engine, latex, f"a name of {len(deep) + 8} characters: the whole path", root, ["-output-directory=" + os.path.join(deep, "o"), os.path.join(deep, "doc.tex")], os.path.join(deep, "o"), dict(c.base_env(os.path.join(root, "cache")), TEXINPUTS=deep + os.pathsep))
    # What rstex says when pdfTeX has refused, which the comparison above only says: a user's document is theirs however pdfTeX fares.
    out = os.path.join(deep, "o2")
    os.makedirs(c.long_name(out))
    r = c.run([engine, "-progname=pdflatex", *c.EDITOR, "-output-directory=" + out, os.path.join(deep, "doc.tex")], root, dict(c.base_env(os.path.join(root, "cache")), TEXINPUTS=deep + os.pathsep))
    c.say("a long name: rstex compiles it", r[0] == 0, f"status {r[0]}, {r[2][-300:]!r}, {sorted(os.listdir(c.long_name(out)))}")
    # The cache under one, with a run that restores.
    base = os.path.join(root, *["c" * 40] * 7)
    os.makedirs(c.long_name(base))
    env = c.base_env(base)
    env.update(RSTEX_CHECKPOINT_TRACE="1", RSTEX_CHECKPOINT_POLICY="always")
    o = os.path.join(want_cache_doc, "o")
    runs = []
    for _ in range(3):
        c.rmtree(o)
        os.makedirs(o)
        runs.append(c.run([engine, "-progname=pdflatex", *c.EDITOR, "-output-directory=o", "doc.tex"], want_cache_doc, env))
    c.say(f"a cache under a name of {len(base)} characters", runs[2][0] == 0 and c.RESTORED in runs[2][2], f"status {runs[2][0]}, {runs[2][2][:160]!r} ... {runs[2][2][-200:]!r}")


def nfc_nfd(engine, latex, root):
    """A file that was made in one form and is asked for in the other: APFS finds either, and HFS+ only hands out the one."""
    if not c.MACOS:
        return
    d = os.path.join(root, "forms")
    c.write(os.path.join(d, unicodedata.normalize("NFD", "caf\u00e9") + ".tex"), DOC)
    c.write(os.path.join(d, "part.tex"), PART)
    results = []
    for prog, extra in ((latex, []), (engine, ["-progname=pdflatex"])):
        c.fresh(os.path.join(d, "o"))
        r = c.run([prog, *extra, *PLAIN_FLAGS, "-output-directory=o", unicodedata.normalize("NFC", "caf\u00e9") + ".tex"], d, c.base_env(os.path.join(root, "cache")))
        logs = [n for n in os.listdir(os.path.join(d, "o")) if n.endswith(".log")]
        results.append((r, c.read(os.path.join(d, "o", logs[0])) if logs else b"", any(n.endswith(".pdf") for n in os.listdir(os.path.join(d, "o")))))
    compare("a file made in NFD and asked for in NFC", results[0], results[1], str([ascii(n) for n in os.listdir(os.path.join(d, "o"))]))


def main():
    c.start()
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--engine", required=True)
    ap.add_argument("--label", default="here")
    ap.add_argument("--only")
    a = ap.parse_args()
    engine = c.program(a.engine)
    latex, pdftex = c.tool("pdflatex"), c.tool("pdftex")
    # A runner's temporary directory on Windows is under RUNNER~1, and in a document ~ is a macro: the target directory has a plain name.
    root = c.fresh(os.path.join(c.OUT, a.label, "names"))
    shared = c.base_env(os.path.join(root, "cache"))
    # What a build gives with a cache of a plain name.
    plain_doc = os.path.join(root, "plain-doc")
    c.write(os.path.join(plain_doc, "doc.tex"), DOC)
    c.write(os.path.join(plain_doc, "part.tex"), PART)
    os.makedirs(os.path.join(plain_doc, "o"))
    r = c.run([engine, "-progname=pdflatex", "--no-snapshot", *c.EDITOR, "-output-directory=o", "doc.tex"], plain_doc, shared)
    if r[0] != 0:
        c.say("a plain name", False, f"status {r[0]}, {r[2][-400:]!r}")
        return c.finish("names")
    want = c.state(os.path.join(plain_doc, "o"))
    for kind, name in NAMES:
        if a.only and not re.search(a.only, kind):
            continue
        c.guarded(f"{kind}, the document", document, engine, latex, kind, name, root, shared)
        c.guarded(f"{kind}, the cache", cache, engine, kind, name, root, want, plain_doc)
        c.guarded(f"{kind}, the home", home, engine, latex, kind, name, root, plain_doc)
    for kind, name in NAMES[:2]:
        if not a.only:
            c.guarded(f"{kind}, a TeX Live", tree_link, engine, kind, name, root, pdftex)
    if not a.only:
        c.guarded("long", long_paths, engine, latex, root, plain_doc)
        c.guarded("forms", nfc_nfd, engine, latex, root)
    return c.finish("names")


if __name__ == "__main__":
    sys.exit(main())
