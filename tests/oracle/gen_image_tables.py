#!/usr/bin/env python3
"""Compares rstex-image with the oracle, object by object, and writes the expected results as tables.

Run with python3 -I from the repository root, with CARGO_TARGET_DIR set if you use one:

    python3 -I tests/oracle/gen_image_tables.py [--external] [--write] [--levels] [--fuzz N] [--engine pdflatex]
                                                [--jobs N] [--only SUBSTRING] [--show N]

Each case is `\\immediate\\pdfximage` of one or more files, in plain pdfTeX with \\pdfcompresslevel=0 and
\\pdfobjcompresslevel=0, so the objects sit uncompressed in the file behind a classic xref table.  The same case goes
through examples/imgsim.rs, which replays the crate's operations as the engine would.  The two must agree on every byte
of every object, on the order of the objects, on what TeX can see of the image (\\pdflastximagepages,
\\pdflastximagecolordepth, width, height), on warnings and on the failure message.

The inputs of the first set come from crates/image/tests/common/gen_inputs.rs, written under target/ at run time.  With
--external, a second set runs over real files: every PDF, PNG and JPEG in TeX Live's tree, and files made here by cjpeg,
ImageMagick, Ghostscript, pdflatex and qpdf, all under target/ too.

--write stores the results in crates/image/testdata/, which tests/image_oracle.rs reads.
"""
import argparse
import concurrent.futures
import hashlib
import os
import random
import re
import shutil
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
TARGET = os.environ.get("CARGO_TARGET_DIR") or os.path.join(ROOT, "target")
WORK = os.path.join(TARGET, "image-oracle")
INDIR = os.path.join(WORK, "in")
OUTDIR = os.path.join(WORK, "out")
EXTDIR = os.path.join(WORK, "ext")
TESTDATA = os.path.join(ROOT, "crates", "image", "testdata")
SIM = os.path.join(TARGET, "debug", "examples", "imgsim")

PARAM_DEFAULTS = {
    "major": 1, "minor": 5, "hicolor": 1, "applygamma": 0, "gamma": 1000, "imagegamma": 2200,
    "resolution": 0, "errorlevel": 0, "copyfonts": 0, "suppressinfo": 0, "underscore": 0, "suppressgroupwarning": 0,
}

BOXES = {1: "mediabox", 2: "cropbox", 3: "bleedbox", 4: "trimbox", 5: "artbox"}

# How many objects a run with no image writes.
BASELINE = [0]
# Which program, and how it ends a run: LaTeX's format has moved \\end.
ENGINE = ["pdftex"]


def tex_source(params, images):
    p = dict(PARAM_DEFAULTS)
    deferred = False
    objlevel = 0
    for w in params.split():
        k, _, v = w.partition("=")
        if k in ("next", "group", "start"):
            continue
        if k == "deferred":
            deferred = True
            continue
        if k == "objlevel":
            objlevel = int(v)
            continue
        p[k] = int(v or 1)
    lines = [
        r"\pdfoutput=1 \pdfcompresslevel=0 \pdfobjcompresslevel=%d" % objlevel,
        r"\pdfmajorversion=%d \pdfminorversion=%d" % (p["major"], p["minor"]),
        r"\pdfimagehicolor=%d \pdfimageapplygamma=%d \pdfgamma=%d \pdfimagegamma=%d \pdfimageresolution=%d"
        % (p["hicolor"], p["applygamma"], p["gamma"], p["imagegamma"], p["resolution"]),
        r"\pdfinclusionerrorlevel=%d \pdfinclusioncopyfonts=%d" % (p["errorlevel"], p["copyfonts"]),
        r"\pdfsuppressptexinfo=%d \pdfsuppresswarningpagegroup=%d" % (p["suppressinfo"], p["suppressgroupwarning"]),
        r"\pdfptexuseunderscore=%d" % p["underscore"],
    ]
    for spec in images:
        parts = spec.split(",")
        opts = ""
        box = ""
        for o in parts[1:]:
            k, v = o.split("=")
            if k == "page":
                opts += " page %s" % v
            elif k == "named":
                opts += " named {%s}" % v
            elif k == "colorspace":
                opts += " colorspace %s" % v
            elif k == "box":
                box = " " + BOXES[int(v)]
        # the keywords go in this order: page or named, colorspace, box
        pg = re.search(r" (page \d+|named \{[^}]*\})", opts)
        cs = re.search(r" colorspace \d+", opts)
        keys = (" " + pg.group(1).strip() if pg else "") + (cs.group(0) if cs else "") + box
        lines.append(r"%s\pdfximage%s {%s}" % ("" if deferred else r"\immediate", keys, parts[0]))
        lines.append(r"\setbox0\hbox{\pdfrefximage\pdflastximage}")
        lines.append(
            r"\immediate\write16{SEEN \the\pdflastximagepages/\the\pdflastximagecolordepth/\number\wd0/\number\ht0}"
        )
        if deferred:
            lines.append(r"\setbox1\hbox{\unhbox1 \pdfrefximage\pdflastximage}")
    lines.append(r"\shipout\box1" if deferred else r"\shipout\hbox{\kern1pt}")
    lines.append(r"\end" if ENGINE[0] == "pdftex" else r"\csname @@end\endcsname")
    return "\n".join(lines) + "\n"


def pdf_objects(pdf):
    """The file's objects in file order, by the classic xref table: [(number, bytes)]."""
    at = pdf.rindex(b"startxref")
    start = int(pdf[at + 9:].split()[0])
    p = pdf.index(b"\n", start) + 1
    offs = []
    while True:
        line_end = pdf.index(b"\n", p)
        head = pdf[p:line_end].split()
        if not head or head[0] == b"trailer":
            break
        first, n = int(head[0]), int(head[1])
        p = line_end + 1
        for i in range(n):
            e = pdf[p:p + 20]
            if e[17:18] == b"n":
                offs.append((int(e[:10]), first + i))
            p += 20
    offs.sort()
    out = []
    for i, (o, num) in enumerate(offs):
        end = offs[i + 1][0] if i + 1 < len(offs) else start
        out.append((num, pdf[o:end]))
    return out


def run_oracle(name, params, images, indir):
    tag = hashlib.sha1((name + params + "|".join(images)).encode()).hexdigest()[:12]
    out = os.path.join(OUTDIR, tag)
    os.makedirs(out, exist_ok=True)
    tex = os.path.join(out, "t.tex")
    with open(tex, "w") as f:
        f.write(tex_source(params, images))
    env = dict(os.environ, max_print_line="1000000", error_line="254", half_error_line="238", SOURCE_DATE_EPOCH="1700000000",
               FORCE_SOURCE_DATE="1", TEXMFOUTPUT=out)
    run = subprocess.run(
        [ENGINE[0], "-interaction=nonstopmode", "-output-directory=" + out, tex],
        cwd=indir, env=env, stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, check=False,
    )
    res = {"objects": [], "seen": [], "warnings": [], "printed": b"", "fail": None}
    if run.returncode < 0:
        res["fail"] = b"ORACLE CRASHED, signal %d" % -run.returncode
    logp = os.path.join(out, "t.log")
    log = open(logp, "rb").read() if os.path.exists(logp) else b""
    for m in re.finditer(rb"SEEN (-?\d+)/(-?\d+)/(-?\d+)/(-?\d+)", log):
        res["seen"].append(tuple(int(x) for x in m.groups()))
    m = re.search(rb"^!pdfTeX error: \S+ \(file [^\n]*?\): ([^\n]*)$", log, re.M)
    if not m:
        m = re.search(rb"^!pdfTeX error: \S+ \(file [^)]*\): ([^\n]*)$", log, re.M)
    if not m:
        m = re.search(rb"^!pdfTeX error: \S+: ([^\n]*)$", log, re.M)
    if res["fail"] is not None:
        pass
    elif m:
        res["fail"] = m.group(1)
    elif b"!pdfTeX error" in log or re.search(rb"^! ", log, re.M):
        e = re.search(rb"^!(?:pdfTeX error)?[^\n]*$", log, re.M)
        res["fail"] = b"OTHER: " + e.group(0)
    for m in re.finditer(rb"pdfTeX warning: \S+ \(file [^\n]*?\): ([^\n]*)", log):
        res["warnings"].append(m.group(1))
    for m in re.finditer(rb"^pdfTeX warning: \S+: ([^\n]*)", log, re.M):
        res["warnings"].append(m.group(1))
    # What tex_printf adds after " <name", up to the ">" or a warning's new line, less the font map's name.
    for base in dict.fromkeys(img.split(",")[0] for img in images):
        for m in re.finditer(rb"<(?:\./)?" + re.escape(base.encode()) + rb"([^>\n]*)", log):
            res["printed"] += re.sub(rb"\{[^}]*\.map\}", b"", m.group(1))
    pdfp = os.path.join(out, "t.pdf")
    if os.path.exists(pdfp) and res["fail"] is None:
        with open(pdfp, "rb") as f:
            pdf = f.read()
        if b"startxref" in pdf[-1100:]:
            res["objects"] = pdf_objects(pdf)
        else:
            res["fail"] = b"ORACLE WROTE NO XREF"
    shutil.rmtree(out, ignore_errors=True)
    return res


def run_sim(params, images, fontmap, indir, start):
    r = subprocess.run(
        [SIM, "run", indir, (params + " start=%d" % start).strip(), fontmap] + images,
        stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.PIPE, check=False,
    )
    out = {"objects": [], "levels": [], "seen": [], "warnings": [], "printed": b"", "fail": None, "crash": None}
    if r.returncode != 0:
        out["crash"] = r.stderr.decode("latin-1")[-400:]
        return out
    d = r.stdout
    p = 0
    while p < len(d):
        e = d.index(b"\n", p)
        line = d[p:e]
        if line.startswith(b"obj "):
            _, n, l, lev = line.split()
            l = int(l)
            out["objects"].append((int(n), d[e + 1:e + 1 + l]))
            out["levels"].append(int(lev))
            p = e + 1 + l + 1
            continue
        if line.startswith(b"seen "):
            out["seen"].append(tuple(int(x) for x in line.split()[1:]))
        elif line.startswith(b"warn "):
            out["warnings"].append(line[5:])
        elif line.startswith(b"printed "):
            out["printed"] = line[8:]
        elif line.startswith(b"fonts "):
            out["fonts"] = int(line[6:])
        elif line.startswith(b"fail "):
            out["fail"] = line[5:]
        p = e + 1
    return out


def mask_padding(b):
    """Zeroes the bits past the last pixel of each row of a raw image with fewer than 8 bits to a pixel.

    png_combine_row keeps those bits of the buffer it copies a row into, and write_png's buffer comes from malloc, so in
    the oracle they are whatever the heap held.  The port writes zeros.  Nothing reads them.
    """
    if b"/Subtype /Image\n" not in b[:120] or b"\nstream\n" not in b:
        return b
    head_end = b.index(b"\nstream\n") + 8
    head = b[:head_end]
    if b"/Filter" in head:
        return b
    w = re.search(rb"/Width (\d+)", head)
    bits = re.search(rb"/BitsPerComponent (\d+)", head)
    if not w or not bits or int(bits.group(1)) >= 8:
        return b
    w, bits = int(w.group(1)), int(bits.group(1))
    used = (w * bits) % 8
    if used == 0:
        return b
    rb_ = (w * bits + 7) // 8
    end = b.rindex(b"\nendstream")
    data = bytearray(b[head_end:end])
    keep = (0xff << (8 - used)) & 0xff
    for i in range(rb_ - 1, len(data), rb_):
        data[i] &= keep
    return head + bytes(data) + b[end:]


def first_diff(a, b):
    n = min(len(a), len(b))
    for i in range(n):
        if a[i] != b[i]:
            return i
    return n


def known_difference(orc, sim):
    """The one difference the port keeps: it stops, where xpdf would run an image decoder on a page's content."""
    return orc["fail"] is None and sim["fail"] is not None and sim["fail"].startswith(
        b"PDF inclusion: content stream filter not supported")


def compare(orc, sim, images, params=""):
    """Differences as strings; none means equal."""
    deferred = "deferred" in params
    if known_difference(orc, sim):
        return []
    if sim.get("crash"):
        return ["sim crashed: " + sim["crash"]]
    probs = []
    if orc["fail"] is not None or sim["fail"] is not None:
        if orc["seen"] != sim["seen"] and not (orc["fail"] or b"").startswith(b"ORACLE"):
            probs.append("seen, before the failure: oracle %r, port %r" % (orc["seen"], sim["seen"]))
        # Where xpdf follows a null pointer or an assert fires, the port stops with a message.
        if orc["fail"] is not None and orc["fail"].startswith(b"ORACLE CRASHED") and sim["fail"] is not None:
            return probs
        if orc["fail"] != sim["fail"]:
            probs.append("failure: oracle %r, port %r" % (orc["fail"], sim["fail"]))
        return probs
    if orc["seen"] != sim["seen"]:
        probs.append("seen: oracle %r, port %r" % (orc["seen"], sim["seen"]))
    if sorted(orc["warnings"]) != sorted(sim["warnings"]):
        probs.append("warnings: oracle %r, port %r" % (orc["warnings"], sim["warnings"]))
    if orc["printed"] != sim["printed"]:
        probs.append("printed: oracle %r, port %r" % (orc["printed"], sim["printed"]))
    by_num = {n: mask_padding(b) for n, b in orc["objects"]}
    order = [n for n, _ in orc["objects"]]
    jb2 = any(i.split(",")[0].lower().endswith((".jb2", ".jbig2")) for i in images)
    i = 0
    if deferred and sim["objects"] and sim["objects"][0][0] in order:
        i = order.index(sim["objects"][0][0])
    for n, b in sim["objects"]:
        if n not in by_num:
            probs.append("object %d: not in the oracle's file" % n)
            continue
        if by_num[n] != b:
            k = first_diff(by_num[n], b)
            probs.append(
                "object %d differs at byte %d (lengths %d, %d): oracle %r, port %r"
                % (n, k, len(by_num[n]), len(b), by_num[n][max(k - 30, 0):k + 50], b[max(k - 30, 0):k + 50])
            )
        if i < len(order) and order[i] == n:
            i += 1
        elif not jb2:
            probs.append("object %d out of order: oracle has %r at that place" % (n, order[i] if i < len(order) else None))
    # What the port did not write: the oracle's file holds the page's own objects, the images' and the fonts'.
    if not jb2 and not deferred and sim.get("fonts", 0) == 0 and len(orc["objects"]) != BASELINE[0] + len(sim["objects"]):
        probs.append("the oracle wrote %d objects, %d of them the page's; the port %d"
                     % (len(orc["objects"]), BASELINE[0], len(sim["objects"])))
    return probs


def oracle_line(orc, sim):
    """A case's result, from the oracle's bytes, in the format of result_line in tests/common/mod.rs."""
    seen = "seen=" + ",".join("/".join(map(str, x)) for x in orc["seen"])
    if known_difference(orc, sim):
        return "seen= fail=" + sim["fail"].decode("latin-1")
    if orc["fail"] is not None:
        # Where the oracle crashes the table records the message the port stops with.
        crashed = orc["fail"].startswith(b"ORACLE CRASHED") and sim["fail"] is not None
        return seen + " fail=" + (sim["fail"] if crashed else orc["fail"]).decode("latin-1")
    by_num = {n: mask_padding(b) for n, b in orc["objects"]}
    blob = b""
    for n, _ in sim["objects"]:
        o = by_num.get(n, b"")
        blob += b"%d %d\n" % (n, len(o)) + o
    return "n=%d h=%s %s printed=%s warn=%s" % (
        len(sim["objects"]), hashlib.sha256(blob).hexdigest()[:16], seen, orc["printed"].decode("latin-1"),
        "|".join(sorted(w.decode("latin-1") for w in orc["warnings"])))


def family(name):
    """As family() in crates/image/tests/common/mod.rs."""
    f = name.split("_")
    if name[0] in "pav" and name[1].isdigit() and len(f) >= 3:
        return "_".join(f[:3])
    return f[0]


def run_case(name, params, images, fontmap, indir):
    orc = run_oracle(name, params, images, indir)
    # \\immediate writes the image first, so the file says what number it got.  Otherwise it is the first object made.
    start = orc["objects"][0][0] if orc["objects"] and "deferred" not in params else 1
    sim = run_sim(params, images, fontmap, indir, start)
    return name, params, images, orc, sim, compare(orc, sim, images, params)


def check_levels(name, params, images, fontmap, indir):
    """The second argument of pdf_begin_obj: with \\pdfobjcompresslevel above 0, which objects go in object streams.

    At \\pdfcompresslevel=0 an object in a stream is preceded by a comment, so both kinds can be found by searching.
    """
    probs = []
    for level in (1, 2, 3):
        par = (params + " objlevel=%d" % level).strip()
        tag = hashlib.sha1((name + par + "levels").encode()).hexdigest()[:12]
        out = os.path.join(OUTDIR, tag)
        os.makedirs(out, exist_ok=True)
        tex = os.path.join(out, "t.tex")
        with open(tex, "w") as f:
            f.write(tex_source(par, images))
        env = dict(os.environ, max_print_line="1000000", TEXMFOUTPUT=out)
        subprocess.run([ENGINE[0], "-interaction=nonstopmode", "-output-directory=" + out, tex], cwd=indir, env=env,
                       stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, check=False)
        pdfp = os.path.join(out, "t.pdf")
        if os.path.exists(pdfp):
            pdf = open(pdfp, "rb").read()
            sim = run_sim(par, images, fontmap, indir, 1)
            for (n, b), lev in zip(sim["objects"], sim["levels"]):
                head = b"%d 0 obj\n" % n
                body = b[len(head):]
                if lev > 0 and lev <= level:
                    want = b"% " + head + body[:-len(b"endobj\n")]
                else:
                    want = b
                if mask_padding(want) != want:
                    continue
                if want not in pdf:
                    probs.append("objlevel %d: object %d, level %d, not found as expected" % (level, n, lev))
        shutil.rmtree(out, ignore_errors=True)
    return probs


def build_sim():
    env = dict(os.environ, CARGO_TARGET_DIR=TARGET)
    subprocess.run(["cargo", "build", "-p", "rstex-image", "--example", "imgsim", "-j4"], cwd=ROOT, env=env, check=True)


def generated_cases():
    r = subprocess.run([SIM, "gen", INDIR], stdout=subprocess.PIPE, check=True)
    cases = []
    font_lines = []
    for line in r.stdout.decode("latin-1").splitlines():
        f = line.split("\t")
        if f[0] == "font":
            font_lines.append(f[1] + "\t" + f[2])
        else:
            cases.append((f[0], f[1], f[2:]))
    fm = os.path.join(WORK, "fontmap.tsv")
    with open(fm, "w") as o:
        o.write("\n".join(font_lines) + "\n")
    return cases, fm


#------------------------------------------------------------------------
# external files
#------------------------------------------------------------------------

def texlive_tree():
    r = subprocess.run(["kpsewhich", "-var-value", "TEXMFDIST"], stdout=subprocess.PIPE, check=True)
    return r.stdout.decode().strip()


def tree_files(top, exts, max_size):
    out = []
    for d, _, fs in os.walk(top):
        for f in fs:
            if os.path.splitext(f)[1].lower() in exts:
                p = os.path.join(d, f)
                try:
                    if os.path.getsize(p) <= max_size:
                        out.append(p)
                except OSError:
                    pass
    return sorted(out)


class FontMap:
    """The part of the real pdftex.map that lookup_fontmap uses: PostScript name to font file, for embedded Type 1."""

    def __init__(self):
        path = subprocess.run(["kpsewhich", "pdftex.map"], stdout=subprocess.PIPE, check=True).stdout.decode().strip()
        self.names = {}
        self.exists = {}
        with open(path, encoding="latin-1") as f:
            for line in f:
                line = line.strip()
                if not line or line[0] in "%#;*":
                    continue
                words = re.findall(r'"[^"]*"|\S+', line)
                if len(words) < 2:
                    continue
                ps = None
                fontfile = None
                quoted = ""
                for w in words[1:]:
                    if w.startswith('"'):
                        quoted = w
                    elif w.startswith("<"):
                        v = w.lstrip("<[")
                        if v.lower().endswith((".pfb", ".pfa")):
                            fontfile = v
                    elif ps is None and not w[0].isdigit():
                        ps = w
                if ps and fontfile and "SlantFont" not in quoted and "ExtendFont" not in quoted:
                    self.names.setdefault(ps, []).append(fontfile)

    def file_exists(self, name):
        if name not in self.exists:
            r = subprocess.run(["kpsewhich", name], stdout=subprocess.PIPE, check=False)
            self.exists[name] = bool(r.stdout.strip())
        return self.exists[name]

    def resolve(self, ps):
        for ff in self.names.get(ps, []):
            if self.file_exists(ff):
                return ff
        return None


def strip_tag(n):
    return n[7:] if len(n) > 7 and n[6] == "+" and n[:6].isupper() and n[:6].isalpha() else n


def tools_available():
    return {t: shutil.which(t) is not None for t in ["cjpeg", "magick", "gs", "pdflatex", "qpdf"]}


def make_tool_files(d):
    """Files made by other programs.  Returns (name, params) pairs."""
    os.makedirs(d, exist_ok=True)
    have = tools_available()
    made = []

    def run(cmd, **kw):
        r = subprocess.run(cmd, cwd=d, stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, check=False, **kw)
        return r.returncode == 0

    if have["magick"]:
        # A picture to start from, with gradient and noise.  No personal image.
        base = "rgb.ppm"
        run(["magick", "-size", "64x48", "plasma:red-blue", "-seed", "1", "-attenuate", "0.2", "+noise", "Gaussian", base])
        run(["magick", base, "-colorspace", "Gray", "gray.pgm"])
        for name, args in [
            ("m_rgb8.png", ["-depth", "8"]),
            ("m_rgb16.png", ["-depth", "16"]),
            ("m_gray1.png", ["-colorspace", "Gray", "-monochrome"]),
            ("m_gray8.png", ["-colorspace", "Gray", "-depth", "8"]),
            ("m_gray16.png", ["-colorspace", "Gray", "-depth", "16"]),
            ("m_pal.png", ["-colors", "16", "-type", "Palette"]),
            ("PNG8:m_pal8.png", ["-colors", "200"]),
            ("m_rgba.png", ["-alpha", "set", "-channel", "A", "-evaluate", "set", "50%"]),
            ("PNG64:m_rgba16.png", ["-alpha", "set", "-channel", "A", "-evaluate", "set", "50%"]),
            ("m_ga.png", ["-colorspace", "Gray", "-alpha", "set", "-channel", "A", "-evaluate", "set", "50%", "-type", "GrayscaleAlpha"]),
            ("m_laced.png", ["-interlace", "PNG"]),
            ("m_laced16.png", ["-interlace", "PNG", "-depth", "16"]),
            ("m_gama.png", ["-set", "gamma", "0.45455"]),
            ("m_srgb.png", ["-define", "png:include-chunk=sRGB"]),
            ("m_nochunks.png", ["-strip", "-define", "png:exclude-chunks=all"]),
            ("m_trns.png", ["-transparent", "white", "-colors", "32", "-type", "PaletteAlpha"]),
            ("m_dens.png", ["-density", "300", "-units", "PixelsPerInch"]),
            ("m_denscm.png", ["-density", "118", "-units", "PixelsPerCentimeter"]),
            ("m_cmyk.jpg", ["-colorspace", "CMYK"]),
            ("m_cmyk_prog.jpg", ["-colorspace", "CMYK", "-interlace", "JPEG"]),
            ("m_rgb.jpg", []),
            ("m_gray.jpg", ["-colorspace", "Gray"]),
            ("m_prog.jpg", ["-interlace", "JPEG"]),
            ("m_dens300.jpg", ["-density", "300", "-units", "PixelsPerInch"]),
            ("m_denscm.jpg", ["-density", "118", "-units", "PixelsPerCentimeter"]),
            ("m_exif.jpg", ["-set", "EXIF:XResolution", "300/1"]),
        ]:
            if run(["magick", base] + args + [name]):
                made.append((name.split(":")[-1], ""))
        for name in ["m_rgb8.png", "m_gray8.png", "m_rgba.png", "m_laced.png", "m_pal.png", "m_gama.png"]:
            made.append((name, "applygamma=1 gamma=2200"))
            made.append((name, "minor=3"))
    if have["cjpeg"] and have["magick"]:
        for name, args, src in [
            ("c_rgb.jpg", [], "rgb.ppm"),
            ("c_gray.jpg", [], "gray.pgm"),
            ("c_grayscale.jpg", ["-grayscale"], "rgb.ppm"),
            ("c_prog.jpg", ["-progressive"], "rgb.ppm"),
            ("c_opt.jpg", ["-optimize"], "rgb.ppm"),
            ("c_q10.jpg", ["-quality", "10"], "rgb.ppm"),
            ("c_rst.jpg", ["-restart", "2"], "rgb.ppm"),
            ("c_arith.jpg", ["-arithmetic"], "rgb.ppm"),
            ("c_12.jpg", ["-precision", "12"], "rgb.ppm"),
        ]:
            with open(os.path.join(d, name), "wb") as o:
                r = subprocess.run(["cjpeg"] + args + [src], cwd=d, stdout=o, stderr=subprocess.DEVNULL, check=False)
            if r.returncode == 0:
                made.append((name, ""))
    tex_docs = []
    if have["pdflatex"]:
        body = {
            "l_text.tex": r"\documentclass{article}\begin{document}Hello $x^2$ \textbf{bold}\end{document}",
            "l_hyper.tex": r"\documentclass{article}\usepackage{hyperref}\hypersetup{pdftitle={T}}"
                           r"\begin{document}\section{A}\label{a}See \ref{a}.\newpage B\end{document}",
            "l_two.tex": r"\documentclass{article}\begin{document}One\newpage\mbox{}\pdfpageattr{/Rotate 90}Two\end{document}",
            "l_old.tex": r"\pdfminorversion=4\pdfobjcompresslevel=0\documentclass{article}\begin{document}Old\end{document}",
            "l_t1.tex": r"\documentclass{article}\usepackage[T1]{fontenc}\usepackage{lmodern}\begin{document}\'e\end{document}",
        }
        for name, src in body.items():
            with open(os.path.join(d, name), "w") as f:
                f.write(src + "\n")
            if run(["pdflatex", "-interaction=batchmode", name]):
                tex_docs.append(name[:-4] + ".pdf")
    pdfs = list(tex_docs)
    if have["gs"]:
        ps = os.path.join(d, "g.ps")
        with open(ps, "w") as f:
            f.write("%!PS\n/Times-Roman findfont 12 scalefont setfont 72 720 moveto (Hello) show 0 0 100 100 rectfill showpage\n"
                    "72 720 moveto (Second) show showpage\n")
        for name, lev in [("g_14.pdf", "1.4"), ("g_15.pdf", "1.5"), ("g_17.pdf", "1.7"), ("g_12.pdf", "1.2")]:
            if run(["gs", "-q", "-dNOPAUSE", "-dBATCH", "-sDEVICE=pdfwrite", "-dCompatibilityLevel=" + lev, "-sOutputFile=" + name, "g.ps"]):
                pdfs.append(name)
        if tex_docs and run(["gs", "-q", "-dNOPAUSE", "-dBATCH", "-sDEVICE=pdfwrite", "-sOutputFile=g_from_tex.pdf", tex_docs[0]]):
            pdfs.append("g_from_tex.pdf")
    if have["qpdf"]:
        for src in list(pdfs):
            stem = src[:-4]
            for tag, args in [
                ("q_objs", ["--object-streams=generate"]),
                ("q_noobjs", ["--object-streams=disable"]),
                ("q_linear", ["--linearize"]),
                ("q_uncomp", ["--stream-data=uncompress", "--object-streams=disable"]),
                ("q_qdf", ["--qdf", "--object-streams=disable"]),
                ("q_newline", ["--newline-before-endstream"]),
            ]:
                name = "%s_%s.pdf" % (stem, tag)
                if run(["qpdf"] + args + [src, name]):
                    pdfs.append(name)
        # Encryption: xpdf opens these with the empty user password, and what it decrypts is what gets copied.
        for src in [p for p in pdfs if "_q_" not in p][:3]:
            stem = src[:-4]
            for tag, args in [
                ("rc4_40", ["--allow-weak-crypto", "--encrypt", "", "o", "40", "--"]),
                ("rc4_128", ["--allow-weak-crypto", "--encrypt", "", "o", "128", "--use-aes=n", "--"]),
                ("aes_128", ["--allow-weak-crypto", "--encrypt", "", "o", "128", "--use-aes=y", "--"]),
                ("aes_128_clear", ["--allow-weak-crypto", "--encrypt", "", "o", "128", "--use-aes=y", "--cleartext-metadata", "--"]),
                ("aes_256", ["--encrypt", "", "o", "256", "--"]),
                ("aes_256_r5", ["--encrypt", "", "o", "256", "--force-R5", "--"]),
                ("noprint", ["--allow-weak-crypto", "--encrypt", "", "o", "128", "--print=none", "--use-aes=n", "--"]),
                ("userpw", ["--allow-weak-crypto", "--encrypt", "u", "o", "128", "--use-aes=n", "--"]),
            ]:
                name = "%s_e_%s.pdf" % (stem, tag)
                if run(["qpdf"] + args + [src, name]):
                    pdfs.append(name)
    for p in pdfs:
        made.append((p, ""))
    return made


def external_cases(jobs):
    tree = texlive_tree()
    cases = []
    for p in tree_files(tree, {".pdf"}, 30 * 1024 * 1024):
        cases.append(("TL:" + os.path.relpath(p, tree), "", [p], tree))
    for p in tree_files(tree, {".png", ".jpg", ".jpeg"}, 30 * 1024 * 1024):
        cases.append(("TL:" + os.path.relpath(p, tree), "", [p], tree))
    made = make_tool_files(EXTDIR)
    for name, params in made:
        cases.append(("TOOL:" + name + ":" + params, params, [name], EXTDIR))
    return cases


def run_external(c, fm_obj, fm_dir):
    name, params, images, indir = c
    fontmap = "-"
    lines = []
    for img in images:
        if img.split(",")[0].lower().endswith(".pdf"):
            r = subprocess.run([SIM, "queries", indir, img], stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, check=False)
            for q in r.stdout.decode("latin-1").splitlines():
                ps = strip_tag(q)
                ff = fm_obj.resolve(ps)
                if ff:
                    lines.append(ps + "\t" + ff)
    if lines:
        fontmap = os.path.join(fm_dir, hashlib.sha1((name + "|".join(images)).encode()).hexdigest()[:16] + ".tsv")
        with open(fontmap, "w", encoding="latin-1") as f:
            f.write("\n".join(lines) + "\n")
    res = run_case(name, params, images, fontmap, indir)
    if fontmap != "-":
        os.unlink(fontmap)
    return res


def mutate(rng, data):
    """Damage: a few flipped bytes, a truncation, a zeroed run, or for text a changed digit."""
    d = bytearray(data)
    k = rng.randrange(6)
    if k == 0 and len(d) > 40:
        del d[rng.randrange(len(d) // 2, len(d)):]
    elif k == 1 and len(d) > 20:
        a = rng.randrange(len(d) - 8)
        d[a:a + rng.randrange(1, 8)] = bytes(8)[:rng.randrange(1, 8)]
    elif k == 2:
        # a digit in the text part: lengths, offsets, numbers, references
        digits = [i for i, c in enumerate(d) if 0x30 <= c <= 0x39]
        for _ in range(rng.randrange(1, 4)):
            if digits:
                d[rng.choice(digits)] = 0x30 + rng.randrange(10)
    elif k == 3:
        # a keyword broken
        for kw in (b"obj", b"stream", b"xref", b"trailer", b"Length", b"Kids", b"Pages", b"IDAT", b"PLTE"):
            i = bytes(d).find(kw)
            if i >= 0 and rng.random() < 0.3:
                d[i + rng.randrange(len(kw))] = rng.randrange(256)
    else:
        for _ in range(rng.randrange(1, 5)):
            if d:
                d[rng.randrange(len(d))] = rng.randrange(256)
    return bytes(d)


def fuzz(args, cases):
    """Damaged files: the port must not panic, and must fail or copy as the oracle does."""
    rng = random.Random(args.seed)
    d = os.path.join(WORK, "fuzz")
    shutil.rmtree(d, ignore_errors=True)
    os.makedirs(d)
    pool = [c for c in cases if len(c[2]) == 1 and "," not in c[2][0] and "deferred" not in c[1]]
    extra = [c for c in os.listdir(EXTDIR)] if os.path.isdir(EXTDIR) else []
    extra = [e for e in extra if e.endswith((".pdf", ".png", ".jpg")) and os.path.getsize(os.path.join(EXTDIR, e)) < 200000]
    jobs = []
    for i in range(args.fuzz):
        if extra and rng.random() < 0.3:
            name = rng.choice(extra)
            src = os.path.join(EXTDIR, name)
            params = ""
        else:
            c = rng.choice(pool)
            name = c[2][0]
            src = os.path.join(INDIR, name)
            params = c[1]
        with open(src, "rb") as f:
            data = f.read()
        out = "f%05d%s" % (i, os.path.splitext(name)[1])
        with open(os.path.join(d, out), "wb") as f:
            f.write(mutate(rng, data))
        jobs.append((out, params, [out]))
    same = 0
    crash = 0
    bad = []
    panics = 0
    failed = 0
    with concurrent.futures.ThreadPoolExecutor(args.jobs) as ex:
        fmo = FontMap()
        fm_dir = os.path.join(WORK, "fm")
        os.makedirs(fm_dir, exist_ok=True)
        futs = [ex.submit(run_external, (n, p, i, d), fmo, fm_dir) for n, p, i in jobs]
        for fu in futs:
            n, p, im, orc, sim, probs = fu.result()
            if sim.get("crash"):
                panics += 1
            if orc["fail"] is not None and orc["fail"].startswith(b"ORACLE"):
                crash += 1
            elif orc["fail"] is not None:
                failed += 1
            if probs:
                bad.append((n, p, im, probs))
            else:
                same += 1
    print("damaged files: %d/%d agree; the oracle failed cleanly on %d and crashed on %d; the port panicked on %d"
          % (same, len(jobs), failed, crash, panics))
    for n, p, im, probs in bad[: args.show]:
        print("--- %s [%s]" % (n, p))
        for q in probs[:2]:
            print("    " + q[:600])
    return bad


def kind_of(images):
    e = os.path.splitext(images[0].split(",")[0])[1].lower()
    return {".png": "PNG", ".jpg": "JPEG", ".jpeg": "JPEG", ".jb2": "JBIG2", ".jbig2": "JBIG2", ".pdf": "PDF"}.get(e, "?")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--external", action="store_true")
    ap.add_argument("--write", action="store_true")
    ap.add_argument("--levels", action="store_true", help="also check the object stream levels")
    ap.add_argument("--jobs", type=int, default=4)
    ap.add_argument("--only", default="")
    ap.add_argument("--show", type=int, default=40)
    ap.add_argument("--no-build", action="store_true")
    ap.add_argument("--fuzz", type=int, default=0, help="also compare this many damaged files")
    ap.add_argument("--seed", type=int, default=20261102)
    ap.add_argument("--engine", default="pdftex", help="pdftex, or pdflatex for the same binary with LaTeX's format")
    args = ap.parse_args()
    ENGINE[0] = args.engine
    if not args.no_build:
        build_sim()
    os.makedirs(OUTDIR, exist_ok=True)
    cases, fm = generated_cases()
    BASELINE[0] = len(run_oracle("baseline", "", [], INDIR)["objects"])
    if args.fuzz:
        fuzz(args, cases)
    if args.only:
        cases = [c for c in cases if args.only in c[0]]
    tally = {}
    bad = []
    fams = {}
    stats = {"objects": 0, "bytes": 0, "failures": {}, "crashes": 0, "fonts": 0}

    def account(k, n, p, im, orc, sim, probs):
        t = tally.setdefault(k, [0, 0])
        t[1] += 1
        if probs:
            bad.append((n, p, im, probs))
        else:
            t[0] += 1
            if known_difference(orc, sim):
                stats["known"] = stats.get("known", 0) + 1
            stats["objects"] += len(sim["objects"])
            stats["bytes"] += sum(len(b) for _, b in sim["objects"])
            # Objects equal only once the padding bits of a row are zeroed: see mask_padding.
            by_num = dict(orc["objects"])
            stats["padding"] = stats.get("padding", 0) + sum(
                1 for n, b in sim["objects"] if n in by_num and by_num[n] != b)
            stats["fonts"] += sim.get("fonts", 0)
            if orc["fail"] is not None:
                if orc["fail"].startswith(b"ORACLE"):
                    stats["crashes"] += 1
                m = re.sub(rb"[<(].*", b"", orc["fail"])[:60].decode("latin-1")
                stats["failures"][m] = stats["failures"].get(m, 0) + 1

    with concurrent.futures.ThreadPoolExecutor(args.jobs) as ex:
        futs = [ex.submit(run_case, n, p, i, fm, INDIR) for n, p, i in cases]
        for fu in futs:
            n, p, im, orc, sim, probs = fu.result()
            account(kind_of(im), n, p, im, orc, sim, probs)
            fams.setdefault(family(n), []).append(oracle_line(orc, sim))
        if args.levels:
            lv = [ex.submit(check_levels, n, p, i, fm, INDIR) for n, p, i in cases
                  if kind_of(i) == "PDF" and "deferred" not in p or n.startswith("two_alpha")]
            count = 0
            for fu in lv:
                count += 1
                for q in fu.result():
                    print("LEVELS: " + q)
            print("object stream levels checked on %d cases, at \\pdfobjcompresslevel 1, 2, 3" % count)

    def report(title):
        print(title)
        for k, (good, total) in sorted(tally.items()):
            print("  %-10s %d/%d" % (k, good, total))
        for n, p, im, probs in bad[: args.show]:
            print("--- %s [%s] %s" % (n, p, " ".join(im)))
            for q in probs[:3]:
                print("    " + q[:900])
        if len(bad) > args.show:
            print("... and %d more" % (len(bad) - args.show))
        print("  %d objects, %d bytes compared; %d font descriptors; %d oracle crashes; counted as agreeing: %d where the "
              "port stops on an image filter in a content array, and %d objects equal except for padding bits" % (
                  stats["objects"], stats["bytes"], stats["fonts"], stats["crashes"], stats.get("known", 0),
                  stats.get("padding", 0)))
        for m, c in sorted(stats["failures"].items()):
            print("    %4d fail with: %s" % (c, m))

    report("generated inputs:")
    if args.write and not args.only:
        os.makedirs(TESTDATA, exist_ok=True)
        with open(os.path.join(TESTDATA, "image_oracle.tsv"), "w", encoding="latin-1", newline="\n") as f:
            f.write("# family, number of cases, then what the oracle does with each when it is hashed together, or the\n")
            f.write("# text if there is one case.  From tests/oracle/gen_image_tables.py on pdfTeX 1.40.29.  Do not edit.\n")
            for fam, ls in fams.items():
                body = "\n".join(ls) + "\n"
                if len(ls) == 1:
                    f.write("%s\t1\t%s\n" % (fam, ls[0]))
                else:
                    f.write("%s\t%d\t%s\n" % (fam, len(ls), hashlib.sha256(body.encode("latin-1")).hexdigest()[:16]))
    if args.external:
        fmo = FontMap()
        ext = external_cases(args.jobs)
        fm_dir = os.path.join(WORK, "fm")
        os.makedirs(fm_dir, exist_ok=True)
        tally.clear()
        bad.clear()
        stats.update({"objects": 0, "bytes": 0, "failures": {}, "crashes": 0, "fonts": 0, "known": 0, "padding": 0})
        with concurrent.futures.ThreadPoolExecutor(args.jobs) as ex:
            futs = [ex.submit(run_external, c, fmo, fm_dir) for c in ext]
            more = []
            for fu in futs:
                n, p, im, orc, sim, probs = fu.result()
                source = "TL " if n.startswith("TL:") else "tool "
                account(source + kind_of(im), n, p, im, orc, sim, probs)
                # More pages of a PDF: the last alone, and the first two in one run, sharing objects.
                if kind_of(im) == "PDF" and orc["seen"] and orc["seen"][0][0] > 1:
                    last = orc["seen"][0][0]
                    root = EXTDIR if n.startswith("TOOL:") else texlive_tree()
                    more.append(ex.submit(run_external, (n + ":last", p, [im[0] + ",page=%d" % last], root), fmo, fm_dir))
                    more.append(ex.submit(run_external, (n + ":two", p, [im[0], im[0] + ",page=2"], root), fmo, fm_dir))
            for fu in more:
                n, p, im, orc, sim, probs = fu.result()
                source = "TL " if n.startswith("TL:") else "tool "
                account(source + "PDF more", n, p, im, orc, sim, probs)
        report("external inputs:")
    return 1 if bad else 0


sys.exit(main())
