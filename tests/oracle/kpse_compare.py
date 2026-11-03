#!/usr/bin/env python3
"""Compares crates/kpse with the real kpsewhich, name by name.  Run with python3 -I.

  python3 -I tests/oracle/kpse_compare.py [--quick] [--show N] [--jobs N]

It builds the example kpse_query, makes a synthetic fixture under $CARGO_TARGET_DIR/kpse-oracle (nothing
is written elsewhere; HOME and XDG_CACHE_HOME point there too, and mktex* is off through the environment),
and then asks both sides the same questions in the same order, in the same batches, since a
search leaves state behind: a directory where a file was found floats to the top of its list.

Sections:
  find        kpse_find_file for names that exist, near misses, names of another format, and odd names,
              over every format except the three glyph formats, under several configurations
  all         the same with -all
  var         -var-value for every variable of the real texmf.cnf, and of a fixture cnf with its quirks
  showpath    -show-path for every format
  expand      -expand-var, -expand-braces, -expand-path
  path        -path, which is kpathsea_path_search, not kpathsea_find_file
  safe        -safe-out-name and -safe-in-name under each openout_any
  tree        kpse_find_file when the tree is found in each other way: RSTEX_TEXMF, a well-known location
              with no kpsewhich on PATH, and with no cache to read or write.  The oracle is always plain kpsewhich.

gf, pk and bitmapfont go through kpathsea_find_glyph in kpsewhich, not kpathsea_find_file, so they have no oracle in `find`: only their paths are compared, in `showpath`,
and the search itself by the pk_* cases of tests/integration/pdffont.

The result is a count over a denominator per section.  Each difference is written to
kpse-oracle/diffs.tsv and the first few are printed.  The exit status is 1 if there is any.
"""
import argparse
import collections
import json
import os
import random
import re
import shutil
import stat
import subprocess
import sys
from concurrent.futures import ThreadPoolExecutor

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
TARGET = os.environ.get("CARGO_TARGET_DIR") or os.path.join(ROOT, "target")
FIX = os.path.join(TARGET, "kpse-oracle")

# Indexed by format code, as kpsewhich -format= knows them: tex-file.h and tex-file.c.
FORMATS = [
    "gf", "pk", "bitmap font", "tfm", "afm", "base", "bib", "bst", "cnf", "ls-R", "fmt", "map", "mem", "mf",
    "mfpool", "mft", "mp", "mppool", "MetaPost support", "ocp", "ofm", "opl", "otp", "ovf", "ovp",
    "graphic/figure", "tex", "TeX system documentation", "texpool", "TeX system sources", "PostScript header",
    "Troff fonts", "type1 fonts", "vf", "dvips config", "ist", "truetype fonts", "type42 fonts", "web2c files",
    "other text files", "other binary files", "misc fonts", "web", "cweb", "enc files", "cmap files",
    "subfont definition files", "opentype fonts", "pdftex config", "lig files", "texmfscripts", "lua",
    "font feature files", "cid maps", "mlbib", "mlbst", "clua", "ris", "bltxml",
]
GLYPH = {0, 1, 2}
MAIN_FORMATS = [3, 10, 11, 25, 26, 32, 33, 44, 36, 47, 48, 8, 9, 38, 39]

# What a sample of existing files for a format looks like: a suffix, or part of a directory name.
SAMPLE = {
    3: (".tfm",), 4: (".afm",), 5: (".base",), 6: (".bib",), 7: (".bst",), 8: (".cnf",), 10: (".fmt",), 11: (".map",),
    12: (".mem",), 13: (".mf",), 14: (".pool",), 15: (".mft",), 16: (".mp",), 17: (".pool",), 19: (".ocp",),
    20: (".ofm", ".tfm"), 21: (".opl", ".pl"), 22: (".otp",), 23: (".ovf", ".vf"), 24: (".ovp", ".vpl"),
    25: (".eps", ".epsi", ".pdf", ".png"), 26: (".tex", ".sty", ".cls", ".fd", ".def", ".clo", ".ldf"), 28: (".pool",),
    29: (".dtx", ".ins"), 30: (".pro", ".ps"), 32: (".pfb", ".pfa"), 33: (".vf",), 35: (".ist",),
    36: (".ttf", ".ttc", ".TTF", ".TTC", ".dfont"), 37: (".t42", ".T42"), 42: (".web", ".ch"), 43: (".w", ".web", ".ch"),
    44: (".enc",), 46: (".sfd",), 47: (".otf", ".OTF"), 49: (".lig",), 51: (".lua", ".luatex", ".tlu"), 52: (".fea",),
    53: (".cid", ".cidmap"), 54: (".mlbib", ".bib"), 55: (".mlbst", ".bst"), 56: (".dll", ".so"), 57: (".ris",),
    58: (".bltxml",),
}
SAMPLE_DIR = {
    9: "/", 18: "/metapost/support", 27: "/doc/", 31: "/groff", 34: "/dvips/", 38: "/web2c/", 39: "/tex/", 40: "/tex/",
    41: "/fonts/misc", 45: "/fonts/cmap", 48: "/pdftex/", 50: "/scripts/",
}


def base_env():
    env = {k: v for k, v in os.environ.items() if not re.match(r"(TEX|KPSE|MFINPUTS|MPINPUTS|BIB|BST|T1|TFM|VF|ENC|OPENOUT|RSTEX|SELFAUTO)", k)}
    env["HOME"] = os.path.join(FIX, "home")
    env["XDG_CACHE_HOME"] = os.path.join(FIX, "cache")
    env["KPATHSEA_WARNING"] = "0"
    for v in ("MKTEXPK", "MKTEXMF", "MKTEXTEX", "MKTEXTFM", "MKTEXFMT", "MKOCP", "MKOFM"):
        env[v] = "0"
    env.pop("TEXMFLOG", None)
    return env


def write(path, data=b""):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "wb") as f:
        f.write(data if isinstance(data, bytes) else data.encode())


def make_fixture():
    def rm_error(func, path, _exc):
        os.chmod(path, 0o700)
        func(path)
    if os.path.lexists(FIX):
        for dirpath, _, names in os.walk(FIX):
            for n in names:
                p = os.path.join(dirpath, n)
                if not os.path.islink(p):
                    try:
                        os.chmod(p, 0o600)
                    except OSError:
                        pass
        shutil.rmtree(FIX, onerror=rm_error)
    cwd = os.path.join(FIX, "cwd")
    # The current directory.
    for n in ["local.tex", "Mixed.TeX", "noext", "a.b.c", "two.tex.tex", "two.tex", "with space.tex", "$dollar.tex", "~tilde.tex",
              "comma,name.tex", "{brace}.tex", "semi;colon.tex", "colon:name.tex", "hash#name.tex", "pct%name.tex", ".hidden.tex",
              "x.tfm", "y.vf", "z.enc", "m.map", "f.pfb", "g.fmt", "pic.eps", "pic.png", "other.cls", "other.sty", "article.cls",
              "cmr10.tfm", "UPPER.TFM", "plain.fmt", "trailing.", "..dots", "...", "b.bib", "s.bst", "c.cnf", "w.web", "o.otf", "t.ttf",
              "sub/inner.tex", "sub/deep/er.tex", "sub/x.tfm", "sub/.hid/y.tex", "dir.tex/inside.tex", "a-b_c.tex", "-dash.tex", "--help"]:
        write(os.path.join(cwd, n), b"% fixture\n")
    write(os.path.join(cwd, "caf\u00e9.tex".encode().decode()), b"")
    with open(os.path.join(cwd.encode(), b"lat\xe9.tex"), "wb"):
        pass
    write(os.path.join(cwd, "L" + "o" * 250 + ".tex"), b"")  # 255 bytes, the maximum for a component
    write(os.path.join(cwd, "T" + "o" * 254), b"")  # what a longer name is truncated to
    os.symlink("local.tex", os.path.join(cwd, "link.tex"))
    os.symlink("nothing-there", os.path.join(cwd, "broken.tex"))
    os.symlink("sub", os.path.join(cwd, "dirlink"))
    os.symlink("sub", os.path.join(cwd, "linkdir.tex"))
    os.mkfifo(os.path.join(cwd, "fifo.tex"))
    write(os.path.join(cwd, "unreadable.tex"))
    os.chmod(os.path.join(cwd, "unreadable.tex"), 0)

    # A home directory: ~/texmf has no ls-R, so it is scanned, and two files share a name.
    home = os.path.join(FIX, "home")
    for n in ["texmf/tex/latex/pa/homepkg.sty", "texmf/tex/latex/pa/dup.sty", "texmf/tex/latex/pb/dup.sty", "texmf/tex/generic/gp/generic.tex",
              "texmf/fonts/tfm/foo/homefont.tfm", "texmf/fonts/type1/foo/homefont.pfb", "texmf/fonts/enc/foo/homefont.enc",
              "texmf/tex/latex/.hid/hiddenpkg.sty", "texmf/tex/latex/pa/sub/subpkg.sty", "texmf/web2c/pdftex/hometest.fmt",
              ".texlive/texmf-var/fonts/map/pdftex/updmap/pdftex.map", ".dotfile"]:
        write(os.path.join(home, n))
    os.symlink(os.path.join(home, "texmf/tex/latex/pa"), os.path.join(home, "texmf/tex/latex/plink"))

    # An extra tree, for TEXINPUTS.
    for n in ["extra/one.tex", "extra/d/two.tex", "extra/d/e/three.tex", "extra/d/dup.tex", "extra/f/dup.tex", "extra/f/SameName.tex", "extra/f/CASE.TEX"]:
        write(os.path.join(FIX, n))

    # Trees with databases.
    db = os.path.join(FIX, "dbtree")
    for n in ["top.tex", "a/one.tex", "a/dup.tex", "b/dup.tex", "a/disk_only.tex", ".hidden/secret.tex", "c/real.mf", "a/x.cnf", "fonts/tfm/dbfont.tfm", "web2c/pdftex/dbfmt.fmt"]:
        write(os.path.join(db, n))
    write(os.path.join(db, "ls-R"), "% ls-R -- filename database for kpathsea; do not change this line.\n"
          "stray-before-first-dir\n./:\nls-R\naliases\ntop.tex\nghost.tex\n.\n..\n\n./a:\none.tex\ndup.tex\nx.cnf\nnotondisk.tex\n\n./b:\ndup.tex\n\n"
          "./.hidden:\nsecret.tex\n\n./c:\nreal.mf\nalias.mf\n\n./fonts/tfm:\ndbfont.tfm\n\n./web2c/pdftex:\ndbfmt.fmt\n")
    write(os.path.join(db, "aliases"), "% aliases\n# more\nreal.mf other.mf\none.tex uno.tex\n")
    db2 = os.path.join(FIX, "dbtree2")
    for n in ["p/lower.tex", "q/crlf.tex", "q/trailing.tex"]:
        write(os.path.join(db2, n))
    write(os.path.join(db2, "ls-r"), b"./p:\r\nlower.tex\r\n\r\n./q:\ncrlf.tex\rtrailing.tex")
    db3 = os.path.join(FIX, "dbtree3")
    write(os.path.join(db3, "ls-R"), "% nothing usable here\n")
    write(os.path.join(db3, "t/unlisted.tex"))

    # A cnf file with its quirks.
    write(os.path.join(FIX, "cnf/texmf.cnf"), f"""% a comment
   # another
TEXMFHOME = {home}/texmf
FOO = a#b  %something
BAR=x
BAR=y
BAZ   value-without-equals
QUX.pdflatex = prog-specific
QUX = generic
QUX.other = other
CONT = one\\
two \\
three
EMPTYVAL =
=novar
.prog = x
NOPE .  = x
VAR.;bad = x
SEMI = a;b;c
USES = $FOO/$BAR/${{QUX}}
SELF = $SELF/x
LOOPA = $LOOPB
LOOPB = $LOOPA
TILDE = ~/x
BANG = !!~/x
MIXED = foo%bar # comment
MIXED2 = foo %bar#baz
MIXED3 = foo  %  # both
DOLLAR = a$
BRACE = {{a,b}}/{{c,d}}
try_me = yes
""")
    write(os.path.join(FIX, "cnf2/texmf.cnf"), "BAR = from-second-file\nONLY2 = second\nTEXMFVAR = $HOME/othervar\n")
    os.makedirs(os.path.join(FIX, "empty"), exist_ok=True)
    os.makedirs(os.path.join(FIX, "cache"), exist_ok=True)
    return cwd


def tree_files():
    """Every file the databases of the real installation list."""
    env = base_env()
    out = []
    dbpath = subprocess.run(["kpsewhich", "--show-path=ls-R"], capture_output=True, env=env, check=True).stdout.decode().strip()
    for top in dbpath.split(":"):
        p = os.path.join(top, "ls-R")
        if not os.path.exists(p):
            continue
        cur = None
        for line in open(p, "rb").read().split(b"\n"):
            if line.endswith(b":") and line.startswith(b"./"):
                d = line[2:-1].decode("latin-1")
                cur = None if "/." in "/" + d else os.path.join(top, d)
            elif cur and line and line not in (b".", b"..") and not line.startswith(b"%"):
                try:
                    n = line.decode()
                except UnicodeDecodeError:
                    continue
                out.append((cur, n))
    # ls-R also lists directories under their parent; leave those in, as a name lookup must cope.
    return out


def odd_names(cwd):
    b = lambda s: s.encode() if isinstance(s, str) else s
    names = [
        "", " ", "  ", "\t", ".", "..", "...", "./", "../", ".//", "/", "//", "///", "x/", "/x/", "x//", "//x", "x//y",
        "local.tex", "local", "./local.tex", "./local", ".//local.tex", "././local.tex", "../cwd/local.tex", "../cwd/local", "LOCAL.TEX", "Local.Tex",
        "local.TEX", "LOCAL", "./LOCAL.TEX", "Mixed.TeX", "mixed.tex", "Mixed", "mixed", "MIXED.TEX", "noext", "NOEXT", "a.b.c", "a.b", "A.B.C",
        "two", "two.tex", "two.tex.tex", "with space", "with space.tex", "WITH SPACE.TEX", "$dollar.tex", "$dollar", "~tilde.tex", "~tilde",
        "comma,name.tex", "{brace}.tex", "semi;colon.tex", "colon:name.tex", "hash#name.tex", "pct%name.tex", ".hidden.tex", ".hidden", "..dots", "trailing.", "trailing",
        "x.tfm", "x", "X.TFM", "y.vf", "y", "z.enc", "z", "m.map", "m", "f.pfb", "f", "g.fmt", "g", "pic.eps", "pic", "pic.png", "other.cls", "other", "other.sty",
        "article.cls", "article", "ARTICLE.CLS", "cmr10.tfm", "cmr10", "CMR10", "UPPER.TFM", "UPPER", "upper.tfm", "upper", "plain.fmt", "plain", "b", "s", "c", "w", "o", "t",
        "sub/inner.tex", "sub/inner", "SUB/inner.tex", "sub/INNER.TEX", "sub/deep/er", "sub/deep/er.tex", "sub//inner.tex", "./sub/inner.tex", "sub/./inner.tex", "sub/../sub/inner.tex",
        "sub/.hid/y.tex", "sub/x.tfm", "sub/x", "dir.tex", "dir", "dir.tex/inside.tex", "dir.tex/inside", "dirlink/inner.tex", "linkdir.tex", "linkdir", "link.tex", "link", "broken.tex", "broken",
        "fifo.tex", "fifo", "unreadable.tex", "unreadable", "a-b_c.tex", "-dash.tex", "-dash", "--help", "-", "--", "-x", "=", "a=b", "*", "?", "[a]", "\\", "\\x", "article.cls\\", "%", "#", "!", "!!", "!!x", "!!local.tex", "!!./local.tex", "!!~/x",
        "$", "$$", "a$", "$ a", "$HOME", "$HOME/.dotfile", "${HOME}/.dotfile", "${HOME", "${}", "${NOPE}", "$NOPE", "$NOPE/x", "a${NOPE}b", "a$NOPE.b", "${NOPE}local.tex", "$NOPE.tex", "$TEXMF", "$TEXMFDIST", "$progname", "$engine", "$SELFAUTOLOC", "$SELFAUTODIR/share/texmf-dist/web2c/texmf.cnf",
        "~", "~/", "~/.dotfile", "~/nothing", "~//.dotfile", "~root", "~root/", "~root/x", "~nosuchuser", "~nosuchuser/x", "~nosuchuser/local.tex", "x~", "x~/y",
        "{a,b}", "{article,report}.cls", "article.cls,report.cls", "a:b", "a;b", "article.cls:report.cls", "article.cls;report.cls", ":", ";", "::", ":a", "a:",
        ".tex", ".cls", ".tfm", ".vf", ".pfb", ".enc", ".map", ".fmt", "tex", "cls", "tfm", "vf", "pfb", "enc", "map", "fmt", "ls-R", "ls-r", "aliases", "texmf.cnf", "texmf", "fmtutil.cnf", "updmap.cfg", "mktex.cnf",
        "article.", "article..cls", "article.CLS", "ARTICLE.cls", "article.cls.", "article.cls.tex", "article.cls ", " article.cls", "article .cls", "tools/.tex", "base/article.cls", "latex/base/article.cls",
        "tex/latex/base/article.cls", "texmf-dist/tex/latex/base/article.cls", "/usr/share/texmf-dist/tex/latex/base/article.cls", "/usr/share/texmf-dist/tex/latex/base/article", "/usr/share/texmf-dist/tex/latex/base/", "/usr/share/texmf-dist/tex/latex/base",
        "//usr/share/texmf-dist/tex/latex/base/article.cls", "/usr//share/texmf-dist/tex/latex/base/article.cls", "/USR/share/texmf-dist/tex/latex/base/article.cls", "/usr/share/texmf-dist/tex/latex/base/ARTICLE.CLS", "/usr/share/texmf-dist/tex/latex/base/Article.cls",
        "base//article.cls", "./base/article.cls", "../base/article.cls", "BASE/article.cls", "base/ARTICLE.CLS", "/etc/passwd", "/etc", "/etc/", "/etc//passwd", "//etc/passwd", "/ETC/PASSWD", "/etc/PASSWD", "/nonexistent/file.tex", "/nonexistent",
        cwd + "/local.tex", cwd + "/local", cwd + "/LOCAL.TEX", cwd + "/sub/inner.tex", cwd + "/SUB/inner.tex", cwd + "//local.tex", cwd + "/broken.tex", cwd + "/fifo.tex", cwd + "/unreadable.tex", cwd + "/dir.tex", cwd + "/", cwd,
        "homepkg.sty", "homepkg", "dup.sty", "generic.tex", "hiddenpkg.sty", "subpkg.sty", "homefont.tfm", "homefont", "homefont.pfb", "homefont.enc", "hometest.fmt", "hometest", "pa/homepkg.sty", "plink/homepkg.sty", "latex/pa/homepkg.sty", "tex/latex/pa/homepkg.sty", "HOMEPKG.STY", "Homepkg.sty",
        "one.tex", "two.tex", "three.tex", "d/two.tex", "e/three.tex", "d/e/three.tex", "dup.tex", "samename.tex", "SameName.tex", "SAMENAME.TEX", "case.tex", "CASE.TEX",
        "top.tex", "top", "ghost.tex", "ghost", "notondisk.tex", "disk_only.tex", "disk_only", "secret.tex", "real.mf", "real", "alias.mf", "alias", "other.mf", "uno.tex", "uno", "a/one.tex", "a/uno.tex", "b/dup.tex", "c/real.mf", "x.cnf", "dbfont.tfm", "dbfont", "dbfmt.fmt", "dbfmt", "lower.tex", "lower", "crlf.tex", "trailing.tex", "unlisted.tex", "stray-before-first-dir",
        "x" * 255, "x" * 256, "x" * 300, "x" * 5000, "L" + "o" * 250 + ".tex", "L" + "o" * 250, "L" + "o" * 251 + ".tex", "L" + "o" * 400 + ".tex", "T" + "o" * 254, "T" + "o" * 255, "T" + "o" * 300, "T" + "o" * 300 + ".tex", "sub/" + "y" * 300 + "/inner.tex", "a/" * 2100 + "x", ("y" * 300) + "/local.tex", "/" + "z" * 300 + "/local.tex",
        cwd + "/" + "z" * 300 + ".tex",
        "caf\u00e9.tex", "caf\u00e9", "CAF\u00c9.TEX", "\u00e9", "\u65e5\u672c\u8a9e.tex",
    ]
    out = [b(n) for n in names]
    out += [b"lat\xe9.tex", b"lat\xe9", b"\xff", b"\xff.tex", b"\x80\x81", b"\x01", b"\x7f", b"a\x7fb.tex", b"a\nb", b"local.tex\n", b"\nlocal.tex"]
    seen = set()
    res = []
    for n in out:
        if n not in seen:
            seen.add(n)
            res.append(n)
    return res


def sample_for_format(code, files, rng, by_suffix, n_exist, n_near, n_cross, all_names):
    """(class, name) pairs for one format."""
    pool = []
    if code in SAMPLE:
        for s in SAMPLE[code]:
            pool.extend(by_suffix.get(s, []))
    elif code in SAMPLE_DIR:
        d = SAMPLE_DIR[code]
        pool = [f for f in files if d in f[0]]
    out = []
    pool = sorted(set(pool))
    chosen = rng.sample(pool, min(n_exist, len(pool))) if pool else []
    for d, n in chosen:
        stem = n.rsplit(".", 1)[0] if "." in n else n
        parts = d.strip("/").split("/")
        k = rng.randrange(9)
        if k <= 3:
            out.append(("exist", n))
        elif k <= 5:
            out.append(("exist", stem))
        elif k == 6:
            out.append(("exist", parts[-1] + "/" + n))
        elif k == 7:
            out.append(("exist", "/".join(parts[-2:]) + "/" + n))
        else:
            out.append(("exist", os.path.join(d, n)))
    near = rng.sample(pool, min(n_near, len(pool))) if pool else []
    for d, n in near:
        k = rng.randrange(7)
        out.append(("near", {0: n + "x", 1: "x" + n, 2: n.upper(), 3: n[:-1], 4: n + ".tex", 5: n.replace("a", "e", 1), 6: "nosuchdir/" + n}[k]))
    for d, n in rng.sample(all_names, min(n_cross, len(all_names))):
        out.append(("cross", n))
    return out


def run_kpsewhich(env, cwd, progname, engine, fmt, must, names):
    """One process, many names.  Returns a list of results, None for not found."""
    cmd = ["kpsewhich", f"-progname={progname}"]
    if engine:
        cmd.append(f"-engine={engine}")
    cmd.append(f"-format={FORMATS[fmt]}")
    if must:
        cmd.append("-must-exist")
    cmd.append("--")
    cmd += names
    p = subprocess.run(cmd, capture_output=True, env=env, cwd=cwd, timeout=120)
    lines = p.stdout.split(b"\n")
    if lines and lines[-1] == b"":
        lines.pop()
    # A miss is a blank line, except that nothing follows the last name.
    if len(names) == 1:
        joined = b"\n".join(lines)
        return [joined if joined else None]
    if len(lines) not in (len(names), len(names) - 1):
        raise SystemExit(f"kpsewhich gave {len(lines)} lines for {len(names)} names: {cmd[:6]}")
    res = [ln if ln else None for ln in lines]
    while len(res) < len(names):
        res.append(None)
    return res


def run_rust(exe, env, cwd, progname, engine, requests):
    """One process, one line per request.  Returns the result lists."""
    cmd = [exe, f"--progname={progname}"]
    cmd.append(f"--engine={engine}" if engine else "--no-engine")
    text = "\n".join("\t".join(r) for r in requests) + "\n"
    p = subprocess.run(cmd, input=text.encode(), capture_output=True, env=env, cwd=cwd, timeout=120)
    if p.returncode != 0:
        raise SystemExit(f"kpse_query failed: {p.stderr.decode(errors='replace')[:2000]}")
    out = []
    for ln in p.stdout.decode().split("\n")[:-1]:
        out.append([] if ln == "-" else [bytes.fromhex(x) for x in ln.split(",")])
    if len(out) != len(requests):
        raise SystemExit(f"kpse_query gave {len(out)} lines for {len(requests)} requests")
    return out


def hexs(b):
    return b.hex()


class Tally:
    def __init__(self):
        self.ok = collections.Counter()
        self.total = collections.Counter()
        self.found = collections.Counter()
        self.explained = collections.Counter()
        self.diffs = []

    def add(self, section, key, oracle, mine, detail):
        self.total[section] += 1
        self.found[(section, key)] += 1 if oracle else 0
        self.total[(section, key)] += 1
        if oracle == mine:
            self.ok[section] += 1
            self.ok[(section, key)] += 1
        else:
            self.diffs.append((section, key, detail, oracle, mine))


def configs(cwd):
    env0 = base_env()
    home = os.path.join(FIX, "home")
    extra = os.path.join(FIX, "extra")
    db = os.path.join(FIX, "dbtree")
    db2 = os.path.join(FIX, "dbtree2")
    db3 = os.path.join(FIX, "dbtree3")
    real = "/usr/share/texmf-dist"
    c = [
        ("default-pdflatex", "pdflatex", "pdftex", {}),
        ("default-pdftex", "pdftex", "pdftex", {}),
        ("no-engine", "kpsewhich", None, {}),
        ("texinputs", "pdflatex", "pdftex", {"TEXINPUTS": f"./sub//:{extra}//::", "TEXINPUTS_pdflatex": "", "TFMFONTS": f":{extra}//", "T1INPUTS": "./sub"}),
        ("texinputs-prog", "pdflatex", "pdftex", {"TEXINPUTS.pdflatex": f"{extra}/d//;.", "TEXINPUTS": f"{extra}/f"}),
        ("cnf-chain", "pdflatex", "pdftex", {"TEXMFCNF": f"{FIX}/cnf:{FIX}/cnf2:{real}/web2c"}),
        ("db-tree", "pdflatex", "pdftex", {"TEXMFDBS": f"{db}:{db2}:{db3}", "TEXINPUTS": f"{db}//:!!{db2}//:{db3}//:.", "MFINPUTS": f"!!{db}//", "TEXFORMATS": f"{db}//", "TFMFONTS": f"{db}/fonts//", "TEXMFCNF": f"{real}/web2c"}),
        ("db-tree-prefix", "pdflatex", "pdftex", {"TEXMFDBS": f"{db}/:{db2}/", "TEXINPUTS": f"{db}/a:{db}/b//:{db}//b:{db}/a/:!!{db}"}),
        ("no-casefold", "pdflatex", "pdftex", {"texmf_casefold_search": "0"}),
        ("no-std-ext-first", "pdflatex", "pdftex", {"try_std_extension_first": "f"}),
        ("nlink", "pdflatex", "pdftex", {"texmf_nlink_for_leaf": "1", "TEXMFHOME": f"{home}/texmf"}),
        ("compile-time", "pdflatex", "pdftex", {"TEXMFCNF": f"{FIX}/empty"}),
        ("kpse-dot", "pdflatex", "pdftex", {"KPSE_DOT": f"{cwd}/sub"}),
        ("other-progname", "my-prog", "pdftex", {"MY-PROGINPUTS": f"{extra}", "PDFTEXFONTS": f"{extra}", "MY-PROGFONTS": f"{extra}//"}),
    ]
    out = []
    for name, prog, eng, over in c:
        env = dict(env0)
        env.update(over)
        out.append((name, prog, eng, env))
    return out


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--quick", action="store_true", help="a small run")
    ap.add_argument("--show", type=int, default=60, help="differences to print")
    ap.add_argument("--width", type=int, default=240, help="characters to print of a value")
    ap.add_argument("--raw-path", action="store_true", help="kpsewhich -path without reading the cnf files first")
    ap.add_argument("--jobs", type=int, default=4)
    ap.add_argument("--seed", type=int, default=20261102)
    args = ap.parse_args()

    env0 = dict(os.environ)
    p = subprocess.run(["cargo", "build", "-p", "rstex-kpse", "--example", "kpse_query", "-j4", "--message-format=json"], cwd=ROOT, capture_output=True, env=env0)
    if p.returncode != 0:
        sys.stderr.write(p.stderr.decode())
        return 2
    exe = None
    for ln in p.stdout.decode().splitlines():
        m = json.loads(ln)
        if m.get("reason") == "compiler-artifact" and m.get("executable") and m["target"]["name"] == "kpse_query":
            exe = m["executable"]
    if not exe:
        raise SystemExit("no kpse_query")

    cwd = make_fixture()
    rng = random.Random(args.seed)
    files = tree_files()
    by_suffix = collections.defaultdict(list)
    for d, n in files:
        i = n.rfind(".")
        if i > 0:
            by_suffix[n[i:]].append((d, n))
    all_names = sorted(set((d, n) for d, n in files))
    odd = odd_names(cwd)

    scale = 0.1 if args.quick else 1.0
    n_exist, n_near, n_cross = int(260 * scale), int(100 * scale), int(60 * scale)
    tally = Tally()
    jobs = []  # (config index, fmt, must, [(class, name bytes)])
    cfgs = configs(cwd)

    for ci, (cname, prog, eng, env) in enumerate(cfgs):
        full = ci == 0
        for fmt in range(len(FORMATS)):
            if fmt in GLYPH:
                continue
            if full:
                pairs = sample_for_format(fmt, files, rng, by_suffix, n_exist, n_near, n_cross, all_names)
            else:
                pairs = sample_for_format(fmt, files, rng, by_suffix, n_exist // 12, n_near // 12, n_cross // 12, all_names)
            names = [(k, v.encode()) for k, v in pairs] + [("odd", n) for n in odd]
            # Same names both ways in a format's main searches.
            jobs.append((ci, fmt, False, names))
            if fmt in MAIN_FORMATS or full:
                if fmt in MAIN_FORMATS or fmt % 5 == 0:
                    jobs.append((ci, fmt, True, names))

    def split(names):
        batch, size = [], 0
        for k, n in names:
            # A name with a newline makes the reply ambiguous: it gets a process of its own.
            if b"\n" in n or b"\0" in n:
                yield [(k, n)]
                continue
            if size + len(n) > 60000 or len(batch) >= 300:
                yield batch
                batch, size = [], 0
            batch.append((k, n))
            size += len(n) + 1
        if batch:
            yield batch

    def do_find(job):
        ci, fmt, must, names = job
        cname, prog, eng, env = cfgs[ci]
        results = []
        for batch in split(names):
            ns = [n for _, n in batch]
            oracle = run_kpsewhich(env, cwd, prog, eng, fmt, must, ns)
            mine = run_rust(exe, env, cwd, prog, eng, [("find", str(fmt), "1" if must else "0", hexs(n)) for n in ns])
            for (k, n), o, m in zip(batch, oracle, mine):
                m1 = m[0] if m else None
                results.append((cname, fmt, must, k, n, o, m1))
        return results

    with ThreadPoolExecutor(args.jobs) as ex:
        for res in ex.map(do_find, jobs):
            for cname, fmt, must, k, n, o, m in res:
                tally.add("find", (cname, k), o, m, f"{FORMATS[fmt]}\tmust={int(must)}\t{n!r}")

    # find -all, one process per name, since the reply cannot be aligned otherwise.
    all_jobs = []
    for ci in range(len(cfgs)):
        some = rng.sample(odd, min(int(120 * scale) if ci else int(400 * scale), len(odd)))
        for n in some:
            all_jobs.append((ci, rng.choice(MAIN_FORMATS), rng.random() < 0.3, n))
        pool = rng.sample(all_names, int(30 * scale) if ci else int(400 * scale))
        for d, nm in pool:
            fmt = rng.choice([f for f in MAIN_FORMATS if f != 9])
            all_jobs.append((ci, fmt, False, nm.encode()))

    def do_all(job):
        ci, fmt, must, n = job
        cname, prog, eng, env = cfgs[ci]
        cmd = ["kpsewhich", f"-progname={prog}"] + ([f"-engine={eng}"] if eng else []) + [f"-format={FORMATS[fmt]}", "-all"] + (["-must-exist"] if must else []) + ["--", n]
        o = subprocess.run(cmd, capture_output=True, env=env, cwd=cwd, timeout=120).stdout
        oracle = o.split(b"\n")[:-1] if o else []
        mine = run_rust(exe, env, cwd, prog, eng, [("all", str(fmt), "1" if must else "0", hexs(n))])[0]
        return cname, fmt, must, n, oracle, mine

    with ThreadPoolExecutor(args.jobs) as ex:
        for cname, fmt, must, n, o, m in ex.map(do_all, all_jobs):
            tally.add("all", cname, o, m, f"{FORMATS[fmt]}\tmust={int(must)}\t{n!r}")

    # Variables.
    cnf_text = open("/usr/share/texmf-dist/web2c/texmf.cnf", "rb").read().decode("latin-1")
    variables = sorted(set(re.findall(r"^\s*([A-Za-z0-9_]+)(?:\.[^\s=]+)?\s*=?", cnf_text, re.M)))
    fix_vars = ["FOO", "BAR", "BAZ", "QUX", "CONT", "EMPTYVAL", "NOPE", "VAR", "SEMI", "USES", "SELF", "LOOPA", "LOOPB", "TILDE", "BANG", "MIXED", "MIXED2", "MIXED3", "DOLLAR", "BRACE", "try_me", "ONLY2", "UNSET_VAR", "progname", "engine", "SELFAUTOLOC", "SELFAUTODIR", "SELFAUTOPARENT", "SELFAUTOGRANDPARENT", "HOME", "", "a b", "a=b", "TEXMFCNF"]
    var_jobs = []
    for ci, (cname, prog, eng, env) in enumerate(cfgs):
        vs = variables if ci in (0, 5) else rng.sample(variables, min(40, len(variables)))
        for v in list(vs) + fix_vars:
            var_jobs.append((ci, v))

    def do_var(job):
        ci, v = job
        cname, prog, eng, env = cfgs[ci]
        cmd = ["kpsewhich", f"-progname={prog}"] + ([f"-engine={eng}"] if eng else []) + [f"-var-value={v}"]
        r = subprocess.run(cmd, capture_output=True, env=env, cwd=cwd, timeout=120)
        o = r.stdout[:-1] if r.stdout.endswith(b"\n") else r.stdout
        oracle = o if r.returncode == 0 else None
        m = run_rust(exe, env, cwd, prog, eng, [("var", hexs(v.encode()))])[0]
        return cname, v, oracle, (m[0] if m else None)

    with ThreadPoolExecutor(args.jobs) as ex:
        for cname, v, o, m in ex.map(do_var, var_jobs):
            tally.add("var", cname, o, m, repr(v))

    # -show-path for every format.
    def do_show(job):
        ci, fmt = job
        cname, prog, eng, env = cfgs[ci]
        cmd = ["kpsewhich", f"-progname={prog}"] + ([f"-engine={eng}"] if eng else []) + [f"-show-path={FORMATS[fmt]}"]
        r = subprocess.run(cmd, capture_output=True, env=env, cwd=cwd, timeout=120)
        m = run_rust(exe, env, cwd, prog, eng, [("showpath", str(fmt))])[0]
        return cname, fmt, r.stdout.rstrip(b"\n"), (m[0] if m else None)

    show_jobs = [(ci, f) for ci in range(len(cfgs)) for f in range(len(FORMATS))]
    with ThreadPoolExecutor(args.jobs) as ex:
        for cname, fmt, o, m in ex.map(do_show, show_jobs):
            tally.add("showpath", cname, o, m, FORMATS[fmt])

    # Expansion.
    strings = ["", "a", "$", "a$", "$$", "$ a", "$HOME", "${HOME}", "$NOPE", "${NOPE}", "a${NOPE}b", "${oops", "$TEXMF", "${TEXMF}", "$TEXMFDIST/x", "$progname", "$engine", "$SELFAUTOLOC",
               "~", "~/x", "!!~/x", "~root", "x~", "{a,b}", "{a,b}{c,d}", "{a,{b,c}}d", "x{a,b}y", "{a", "a}", "{}", "{,}", "a,b", "a:b", "a;b", "a::b", ":a", "a:", "{a:b}", "{a;b}", "{$TEXMF,x}", "$TEXMF/tex//", "!!a:b", "!!{a,b}", "{!!a,b}", "{~/a,~/b}", "${HOME}/{a,b}", ".", "./", "./x", "x/.", "{.,..}", "$NOPE:{a,$NOPE}", "$FOO", "$BAR", "$USES", "$LOOPA", "$BRACE", "$MIXED", "{$BRACE,z}", "$TILDE", "$BANG", "$TEXINPUTS", "$TEXFORMATS", "$WEB2C", "$TEXMFDBS", "$TEXMFCNF", "a{b", "a{b,c", "${A}{", "$A{b,c}", "{$A", "{${A}", "{${A},b}"]
    expand_jobs = []
    for ci in range(len(cfgs)):
        for flag, req in (("-expand-var", "expandvar"), ("-expand-braces", "expandbraces"), ("-expand-path", "expandpath")):
            for s in strings:
                expand_jobs.append((ci, flag, req, s))
        # SELF = $SELF/x: kpsewhich does not return from this in -expand-braces or -expand-path, since
        # brace_expand_element recurses on an ever longer string.  crates/kpse stops, and is tested for it.
        expand_jobs.append((ci, "-expand-var", "expandvar", "$SELF"))

    def do_expand(job):
        ci, flag, req, s = job
        cname, prog, eng, env = cfgs[ci]
        cmd = ["kpsewhich", f"-progname={prog}"] + ([f"-engine={eng}"] if eng else []) + [f"{flag}={s}"]
        r = subprocess.run(cmd, capture_output=True, env=env, cwd=cwd, timeout=120)
        m = run_rust(exe, env, cwd, prog, eng, [(req, hexs(s.encode()))])[0]
        return cname, flag, s, r.stdout[:-1] if r.stdout.endswith(b"\n") else r.stdout, (m[0] if m else None)

    with ThreadPoolExecutor(args.jobs) as ex:
        for cname, flag, s, o, m in ex.map(do_expand, expand_jobs):
            tally.add("expand", (cname, flag), o, m, f"{flag}\t{s!r}")

    # -path, which goes through kpathsea_path_search.
    paths = [".", "./sub", "./sub//", ".//", f"{FIX}/extra//", f"{FIX}/extra/d//e", f"{FIX}//dup.tex", f"{FIX}/extra//:.", "$TEXMF/fonts/tfm/public/cm", "!!/usr/share/texmf-dist/fonts/tfm//", "/usr/share/texmf-dist/fonts/tfm/public/cm", "/usr/share/texmf-dist/tex/latex/base", "$HOME/texmf/tex/latex//", "$HOME/texmf/tex//pa", "$HOME/texmf//latex//", "{.,$HOME/texmf/tex//}"]
    pnames = ["local.tex", "LOCAL.TEX", "inner.tex", "sub/inner.tex", "one.tex", "two.tex", "three.tex", "dup.tex", "cmr10.tfm", "CMR10.TFM", "article.cls", "homepkg.sty", "dup.sty", "generic.tex", "subpkg.sty", "er.tex", "deep/er.tex", "e/three.tex", "/etc/passwd", "~/.dotfile", "$HOME/.dotfile", "nothing", "", ".", "..", "sub", "x/"]
    path_jobs = []
    for ci in (0, 3, 6, 8):
        for pth in paths:
            for nm in pnames:
                for must in (False, True):
                    path_jobs.append((ci, pth, nm, must))
    if args.quick:
        path_jobs = rng.sample(path_jobs, len(path_jobs) // 10)

    # kpathsea_path_search with the cnf files not read yet: the first miss looks up texmf_casefold_search
    # inside path_search's loop, which runs read_all_cnf, whose own searches leave the non-reentrant
    # kpse->path at NULL.  The loop ends there, so kpsewhich -path=a:b never looks in b.  crates/kpse reads
    # the cnf files first, as kpathsea_find_file always does.  So -expand-var makes kpsewhich read them
    # first too, and its line of output is skipped.  The variable must be one that only a cnf file sets, or
    # nothing is read: TEXMFDBS is in the environment under db-tree.  --raw-path leaves this out, to see
    # the difference.
    early = [] if args.raw_path else ["-expand-var=$TEXMFMAIN"]

    def do_path(job):
        ci, pth, nm, must = job
        cname, prog, eng, env = cfgs[ci]
        cmd = ["kpsewhich", f"-progname={prog}"] + ([f"-engine={eng}"] if eng else []) + early + [f"-path={pth}"] + (["-must-exist"] if must else []) + ["--", nm]
        r = subprocess.run(cmd, capture_output=True, env=env, cwd=cwd, timeout=120)
        lines = r.stdout.split(b"\n")
        o = b"\n".join(lines[1 if early else 0 :]).rstrip(b"\n") or None
        m = run_rust(exe, env, cwd, prog, eng, [("pathsearch", "1" if must else "0", hexs(pth.encode()), hexs(nm.encode()))])[0]
        return cname, pth, nm, must, o, (m[0] if m else None)

    with ThreadPoolExecutor(args.jobs) as ex:
        for cname, pth, nm, must, o, m in ex.map(do_path, path_jobs):
            tally.add("path", cname, o, m, f"{pth!r}\tmust={int(must)}\t{nm!r}")

    # Name checks.  No empty name: kpsewhich's own option parsing refuses an empty argument before
    # the library is asked ("empty argument for safe-out-name").
    safe = ["a.tex", "sub/a.tex", ".rc", "./a", "./.a", "../a", "a/../b", "a/..", "..", ".", "...", "..a", "a..", "a../b", "../", "x/.y", "x/..y", "x/./y", ".x/y", "/abs", "/abs/.x", "/abs/../x", "$HOME/x", "$HOME/.x", "~/x", "~/.x", "$NOPE/x", "$TEXMFVAR/x", "$TEXMFSYSVAR/x", "$TEXMFVAR/.x", "/var/lib/texmf/x", "/var/lib/texmf", "/var/lib/texmfx", f"{FIX}/out/a", f"{FIX}/out", f"{FIX}/outx", f"{FIX}/out/../a", f"{FIX}/out/.a", f"{FIX}/home/.texlive/texmf-var/a", "a b", "\\x", "a\\..\\b", "x/y/", "/", "//x"]
    safe_envs = [{}, {"openout_any": "a"}, {"openout_any": "r"}, {"openout_any": "p"}, {"openout_any": "n"}, {"openout_any": "y"}, {"openout_any": "0"}, {"openout_any": "1"}, {"openout_any": "zzz"},
                 {"TEXMF_OUTPUT_DIRECTORY": f"{FIX}/out"}, {"TEXMFOUTPUT": f"{FIX}/out"}, {"TEXMFVAR": f"{FIX}/out"}, {"TEXMFSYSVAR": f"{FIX}/out"}]
    safe_jobs = []
    for over in safe_envs:
        for s in safe:
            for flag, req in (("-safe-out-name", "safeout"), ("-safe-extended-out-name", "safeoutext"), ("-safe-in-name", "safein"), ("-safe-extended-in-name", "safeinext")):
                safe_jobs.append((over, s, flag, req))
    if args.quick:
        safe_jobs = rng.sample(safe_jobs, len(safe_jobs) // 10)

    def do_safe(job):
        over, s, flag, req = job
        env = base_env()
        env.update(over)
        r = subprocess.run(["kpsewhich", "-progname=pdflatex", f"{flag}={s}"], capture_output=True, env=env, cwd=cwd, timeout=120)
        m = run_rust(exe, env, cwd, "pdflatex", "pdftex", [(req, hexs(s.encode()))])[0]
        return over, s, flag, r.returncode == 0, m == [b"1"]

    with ThreadPoolExecutor(args.jobs) as ex:
        for over, s, flag, o, m in ex.map(do_safe, safe_jobs):
            tally.add("safe", flag, o, m, f"{flag}\t{s!r}\t{over}")

    # The other ways of finding the tree.  The real tree is /usr/share/texmf-dist on this system.
    empty_path = os.path.join(FIX, "empty")
    no_cache = os.path.join(FIX, "cwd", "local.tex")  # a file, so that nothing can be made under it
    tree_cfgs = [
        ("RSTEX_TEXMF", {"RSTEX_TEXMF": "/usr/share/texmf-dist", "XDG_CACHE_HOME": no_cache}),
        ("well-known", {"PATH": empty_path, "XDG_CACHE_HOME": no_cache}),
        ("no-cache", {"XDG_CACHE_HOME": no_cache}),
        ("stale-cache", {"XDG_CACHE_HOME": os.path.join(FIX, "stale")}),
    ]
    write(os.path.join(FIX, "stale", "rstex", "kpse-tree"), "rstex-kpse-tree 1\npath=somewhere else\nprog=/nonexistent\nstamp=1:1\nloc=/wrong\n")
    tree_jobs = []
    for cname, over in tree_cfgs:
        for fmt in MAIN_FORMATS:
            pairs = sample_for_format(fmt, files, rng, by_suffix, n_exist // 10, n_near // 10, n_cross // 10, all_names)
            names = [(k, v.encode()) for k, v in pairs] + [("odd", n) for n in odd if b"\n" not in n]
            tree_jobs.append((cname, over, fmt, names))

    def do_tree(job):
        cname, over, fmt, names = job
        env_oracle = base_env()
        env_rust = base_env()
        env_rust.update(over)
        out = []
        for batch in split(names):
            ns = [n for _, n in batch]
            oracle = run_kpsewhich(env_oracle, cwd, "pdflatex", "pdftex", fmt, False, ns)
            mine = run_rust(exe, env_rust, cwd, "pdflatex", "pdftex", [("find", str(fmt), "0", hexs(n)) for n in ns])
            for n, o, m in zip(ns, oracle, mine):
                out.append((cname, fmt, n, o, m[0] if m else None))
        return out

    # With no binary to locate, the SELFAUTO* values under RSTEX_TEXMF and a well-known location are derived
    # from the tree, not the real ones.  So a name using them differs, and so does the path of the two formats
    # whose default is built from them, cnf and clua: kpsewhich looks for a "cnf" file in /usr/bin here.  No
    # other format's path differs, and the engine never looks up a file by these.  All else must agree.
    def tree_explained(cname, fmt, n):
        return cname in ("RSTEX_TEXMF", "well-known") and (b"SELFAUTO" in n or FORMATS[fmt] in ("cnf", "clua"))

    with ThreadPoolExecutor(args.jobs) as ex:
        for res in ex.map(do_tree, tree_jobs):
            for cname, fmt, n, o, m in res:
                if o != m and tree_explained(cname, fmt, n):
                    tally.explained[cname] += 1
                    tally.total["tree"] += 1
                    tally.total[("tree", cname)] += 1
                else:
                    tally.add("tree", cname, o, m, f"{FORMATS[fmt]}\t{n!r}")

    # A control: the comparison must be able to fail.  Each of these misconfigures the Rust side only.
    # The result must differ on some names, or a clean result above would mean nothing.
    control_names = [b"local.tex", b"LOCAL.TEX", b"Mixed", b"mixed.tex", b"sub/INNER.TEX", b"article.cls", b"cmr10.tfm", b"homepkg.sty"]
    controls = [
        ("casefold off", {"texmf_casefold_search": "0"}, 26),
        ("TEXINPUTS elsewhere", {"TEXINPUTS": "/nonexistent"}, 26),
        ("no TeX tree", {"TEXMFCNF": os.path.join(FIX, "empty")}, 26),
    ]
    control_ok = True
    for cname, over, fmt in controls:
        env_oracle = base_env()
        env_rust = base_env()
        env_rust.update(over)
        oracle = run_kpsewhich(env_oracle, cwd, "pdflatex", "pdftex", fmt, False, control_names)
        mine = run_rust(exe, env_rust, cwd, "pdflatex", "pdftex", [("find", str(fmt), "0", hexs(n)) for n in control_names])
        differ = sum(1 for o, m in zip(oracle, mine) if o != (m[0] if m else None))
        print(f"control, {cname}: {differ}/{len(control_names)} names differ, as they must")
        control_ok = control_ok and differ > 0
    if not control_ok:
        print("A CONTROL FOUND NO DIFFERENCE: this comparison cannot be trusted")
        return 2

    # Report.
    print(f"names from the databases: {len(files)}; odd names: {len(odd)}")
    print()
    for section in ("find", "all", "var", "showpath", "expand", "path", "safe", "tree"):
        print(f"{section}: {tally.ok[section]}/{tally.total[section]}")
        keys = sorted(k for k in tally.total if isinstance(k, tuple) and k[0] == section)
        for k in keys:
            print(f"    {str(k[1]):42} {tally.ok[k]}/{tally.total[k]}   oracle found {tally.found[k]}")
    print()
    n_diff = len(tally.diffs)
    print(f"differences: {n_diff}, and explained in the tree section, where SELFAUTO* cannot be real: {sum(tally.explained.values())} {dict(tally.explained)}")
    with open(os.path.join(FIX, "diffs.tsv"), "w", errors="replace") as f:
        for section, key, detail, o, m in tally.diffs:
            f.write(f"{section}\t{key}\t{detail}\t{o!r}\t{m!r}\n")
    def brief(v):
        t = repr(v).replace(FIX, "<FIX>")
        return t if len(t) <= args.width else t[: args.width] + "..."
    for section, key, detail, o, m in tally.diffs[: args.show]:
        print(f"  {section} {key} {brief(detail)}\n      kpsewhich: {brief(o)}\n      rstex:     {brief(m)}")
    if n_diff > args.show:
        print(f"  ... and {n_diff - args.show} more, in {os.path.join(FIX, 'diffs.tsv')}")
    return 1 if n_diff else 0


if __name__ == "__main__":
    sys.exit(main())
