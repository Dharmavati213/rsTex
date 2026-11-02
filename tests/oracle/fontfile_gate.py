#!/usr/bin/env python3
"""The gate for crates/fontfile: byte equality with the oracle.

Run with python3 -I, from anywhere:

    python3 -I tests/oracle/fontfile_gate.py [--limit N] [--workers N] [--keep] [--only SUBSTRING]

Each job is a small document typeset by `pdftex -ini` (no format, so a run costs milliseconds), with
\\pdfcompresslevel=0 and \\pdfobjcompresslevel=0 so that objects can be read through the xref table.  From its
PDF the script takes every /FontDescriptor, its font stream with /Length1-3, /FontName with the tag, /Flags,
/FontBBox, /ItalicAngle and the rest, /CharSet (\\pdfomitcharset=0), the /Differences of each encoding and each
/ToUnicode stream; from its log every warning and fatal error.  A job list goes in a manifest, and the example
crates/fontfile/examples/fontfile_gate.rs replays writefont.c with the crate and compares everything.

Everything is written under target/fontfile-gate.  No font is copied into the tree, and no input other than
TeX Live's files and the synthetic fonts made here is used.

Jobs:
  map    every entry of pdftex.map, with all characters of its TFM, with a dozen, and with one.  An entry whose
         TFM is not installed borrows one with 256 characters; one whose font program or encoding is not
         installed is left out, and the count says so.
  seac   a sample with a few high codes alone, so that seac has to pull in a base and an accent
  nosub  a sample with `<<`, so no subsetting
  slant  a sample with SlantFont and ExtendFont values
  reenc  a sample reencoded with encodings the font was not made for, so that glyphs go missing
  flags  a sample with /Flags in the map line
  pfa    a sample converted from PFB to PFA at run time
  multi  several fonts in a document: shared descriptors, the C's statics carried across fonts
  tou    a sample with \\pdfgentounicode=1 and glyphtounicode.tex
  ttf    the TrueType and OpenType fonts of TeX Live, under the map lines given here
  ttfsyn synthetic TrueType and OpenType fonts, for what those three do not reach
  syn    synthetic Type 1 fonts for the branches that real fonts do not reach, failures included
  enc    encoding files in odd shapes and in error
  sfd    subfont definition files in odd shapes and in error
  line   map lines good and bad
"""
import concurrent.futures
import os
import random
import re
import shutil
import struct
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
TARGET = os.environ.get("CARGO_TARGET_DIR") or os.path.join(ROOT, "target")
OUT = os.path.join(TARGET, "fontfile-gate")
WORK = os.path.join(OUT, "work")

PREAMBLE = r"""\catcode`\{=1 \catcode`\}=2
\pdfoutput=1 \pdfcompresslevel=0 \pdfobjcompresslevel=0 \pdfomitcharset=0
\pdfpagewidth=100pt \pdfpageheight=100pt \hsize=100pt \vsize=100pt
"""


def kpse(name, fmt=None):
    args = ["kpsewhich"]
    if fmt:
        args += ["-format", fmt]
    r = subprocess.run(args + [name], capture_output=True, text=True)
    p = r.stdout.strip()
    return p or None


def read_map():
    """The first line for each TFM, as FM_DUPIGNORE keeps."""
    path = kpse("pdftex.map")
    entries = {}
    with open(path, "rb") as f:
        for raw in f.read().split(b"\n"):
            line = raw.strip()
            if not line or line[:1] in b"%*#;":
                continue
            tfm = line.split()[0].decode("latin-1")
            if tfm not in entries:
                entries[tfm] = raw
    return entries


def tfm_codes(path):
    """Codes with char_exists: width index not zero."""
    with open(path, "rb") as f:
        d = f.read()
    lf, lh, bc, ec = struct.unpack(">4H", d[:8])
    codes = []
    if bc > ec:
        return codes
    for i in range(ec - bc + 1):
        if d[24 + 4 * lh + 4 * i] != 0:
            codes.append(bc + i)
    return [c for c in codes if c < 256]


class Font:
    """One font in a document: TFM, codes, and either a line of pdftex.map or a \\pdfmapline."""

    def __init__(self, tfm, codes, line, kind="file", files=()):
        self.tfm = tfm
        self.codes = codes
        self.line = line
        self.kind = kind
        self.files = files  # (name, bytes) to put in the job's directory


class Job:
    def __init__(self, jid, fonts, tounicode=False, extra=b""):
        self.id = jid
        self.fonts = fonts
        self.tounicode = tounicode
        self.extra = extra


def doc_text(job):
    t = PREAMBLE
    if job.tounicode:
        t += "\\input gtu.tex\n\\pdfgentounicode=1\n"
    seen = set()
    for f in job.fonts:
        if f.kind == "line" and f.line not in seen:
            seen.add(f.line)
            t += "\\pdfmapline{%s}\n" % f.line.decode("latin-1")
    n = 0
    body = ""
    for f in job.fonts:
        if not f.codes:
            continue
        n += 1
        name = "\\csname f%d\\endcsname" % n
        body += "\\expandafter\\font\\csname f%d\\endcsname=%s at 10pt \\pdfnoligatures%s\n" % (n, f.tfm, name)
        body += "\\shipout\\hbox{%s" % name
        for c in f.codes:
            body += "\\hbox{\\char%d}" % c
        body += "}\n"
    return t + body + "\\end\n"


def pdf_objects(data):
    i = data.rindex(b"startxref")
    off = int(data[i + 9 :].split()[0])
    head = data[off:off + 40].split(b"\n")
    assert head[0] == b"xref", head
    first, count = map(int, head[1].split())
    p = off + len(head[0]) + 1 + len(head[1]) + 1
    objs = {}
    for k in range(count):
        ent = data[p + 20 * k : p + 20 * k + 20]
        if ent[17:18] == b"n":
            objs[first + k] = int(ent[:10])
    return objs


def pdf_obj(data, offset):
    """The dictionary text and the stream bytes of the object at offset."""
    end = data.index(b"obj", offset) + 3
    s = data.find(b"stream\n", end)
    e = data.find(b"endobj", end)
    if s != -1 and s < e:
        d = data[end:s]
        m = re.search(rb"/Length (\d+)", re.sub(rb"/Length[123] \d+", b"", d))
        n = int(m.group(1))
        return d, data[s + 7 : s + 7 + n]
    return data[end:e], None


def num(d, key):
    m = re.search(b"/" + key + rb" (-?\d+)", d)
    return m.group(1).decode() if m else "-"


def hx(b):
    return b.hex() if b else "-"


LOG_WARN = re.compile(rb"^pdfTeX warning: \S+?(?: \(file ([^)]*)\))?: (.*)$")
LOG_ERR = re.compile(rb"^!pdfTeX error: \S+?(?: \(file ([^)]*)\))?: (.*)$")


def run_job(job):
    """Runs the oracle.  Returns the job's manifest text and where its streams are."""
    d = os.path.join(WORK, job.id)
    os.makedirs(d, exist_ok=True)
    for f in job.fonts:
        for name, data in f.files:
            with open(os.path.join(d, name), "wb") as o:
                o.write(data)
    if job.tounicode:
        shutil.copy(os.path.join(OUT, "gtu.tex"), os.path.join(d, "gtu.tex"))
    with open(os.path.join(d, "j.tex"), "w", encoding="latin-1") as o:
        o.write(doc_text(job))
    env = dict(os.environ, max_print_line="1000000", error_line="254", half_error_line="238")
    r = subprocess.run(
        ["pdftex", "-ini", "-interaction=batchmode", "-recorder", "j.tex"],
        cwd=d, env=env, capture_output=True, timeout=300,
    )
    out = ["job\t" + job.id]

    # Files as the oracle found them.
    found = {}
    try:
        with open(os.path.join(d, "j.fls"), "rb") as o:
            for ln in o.read().split(b"\n"):
                if ln.startswith(b"INPUT "):
                    p = ln[6:].decode("latin-1")
                    found[os.path.basename(p)] = p
    except OSError:
        pass

    def resolve(name):
        if name is None:
            return "-"
        p = found.get(name)
        if p is None:
            p = kpse(name)
        if p is None:
            local = os.path.join(d, name)
            return local if os.path.exists(local) else "-"
        return p if os.path.isabs(p) else os.path.join(d, p)

    for f in job.fonts:
        toks = re.findall(rb"[<\[]*([^\s<\[\"]+)", f.line)
        enc = next((t for t in toks if t.lower().endswith(b".enc")), None)
        ff = next((t for t in toks if t.lower().endswith((b".pfb", b".pfa", b".ttf", b".ttc", b".otf"))), None)
        sfdm = re.search(rb"@(\w+)@", f.line)
        sfd = os.path.join(d, sfdm.group(1).decode() + ".sfd") if sfdm else "-"
        out.append(
            "font\t%s\t%s\t%s\t%s\t%s\t%s\t%s"
            % (
                f.tfm,
                ",".join(map(str, f.codes)) or "-",
                resolve(enc.decode("latin-1")) if enc else "-",
                resolve(ff.decode("latin-1")) if ff else "-",
                sfd,
                f.kind,
                hx(f.line),
            )
        )
    out.append("tounicode\t%d" % (1 if job.tounicode else 0))

    log = b""
    try:
        with open(os.path.join(d, "j.log"), "rb") as o:
            log = o.read()
    except OSError:
        pass
    tfms = [f.tfm.encode("latin-1") for f in job.fonts]
    for ln in log.split(b"\n"):
        m = LOG_WARN.match(ln)
        if m:
            msg = m.group(2)
            fname = m.group(1) or b""
            if fname.endswith(b".map"):
                # A map file's warnings: only for the lines this job scans; duplicates are avl_do_entry's.
                if b"already exists" in msg or b"has been used" in msg:
                    continue
                if not any(b"`" + t + b"'" in msg for t in tfms):
                    continue
            out.append("warn\t" + hx(msg))
            continue
        m = LOG_ERR.match(ln)
        if m:
            out.append("error\t" + hx(m.group(2)))
    err = r.stderr
    am = re.search(rb"^\S+: (\S+:\d+: \w+: Assertion .* failed\.)$", err, re.M)
    if am:
        out.append("error\t" + hx(am.group(1)))
    elif r.returncode < 0:
        # glibc noticed the C's own heap overflow, or the like: nothing to compare with.
        out.append("crash\t" + hx(err.strip()[-200:] or b"signal %d" % -r.returncode))

    pdf = os.path.join(d, "j.pdf")
    data = b""
    if os.path.exists(pdf):
        with open(pdf, "rb") as o:
            data = o.read()
    if b"startxref" in data:  # not after a failed assert, which leaves a part of the file
        objs = pdf_objects(data)
        ordered = sorted(objs.items(), key=lambda x: x[1])
        cmap_n = 0
        desc_n = 0
        for n, offset in ordered:
            dct, stream = pdf_obj(data, offset)
            if b"/Type /FontDescriptor" in dct:
                name = re.search(rb"/FontName /(\S+)", dct).group(1)
                bbox = re.search(rb"/FontBBox \[([^\]]*)\]", dct)
                cs = re.search(rb"/CharSet \((.*?)\)\n", dct, re.S)
                ff = re.search(rb"/FontFile\d? (\d+) 0 R", dct)
                l1 = l2 = l3 = length = sp = "-"
                if ff:
                    fd, fs = pdf_obj(data, objs[int(ff.group(1))])
                    l1, l2, l3 = num(fd, b"Length1"), num(fd, b"Length2"), num(fd, b"Length3")
                    length = str(len(fs))
                    sp = os.path.join(d, "d%d.font" % desc_n)
                    with open(sp, "wb") as o:
                        o.write(fs)
                desc_n += 1
                out.append(
                    "desc\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s"
                    % (
                        hx(name),
                        num(dct, b"Flags"),
                        bbox.group(1).decode() if bbox else "-",
                        num(dct, b"ItalicAngle"),
                        num(dct, b"Ascent"),
                        num(dct, b"CapHeight"),
                        num(dct, b"Descent"),
                        num(dct, b"StemV"),
                        num(dct, b"XHeight"),
                        l1, l2, l3, length, sp,
                        hx(cs.group(1)) if cs else "-",
                    )
                )
            elif b"/Type /Encoding" in dct:
                m = re.search(rb"/Differences \[(.*?)\]\n", dct, re.S)
                out.append("enc\t" + hx(m.group(1)))
            elif stream is not None and stream.startswith(b"%!PS-Adobe-3.0 Resource-CMap"):
                title = re.search(rb"/Ordering \((.*?)\)", stream).group(1)
                tfm = next((f.tfm for f in job.fonts if title.startswith(f.tfm.encode("latin-1") + b"-")), None)
                sp = os.path.join(d, "u%d.cmap" % cmap_n)
                cmap_n += 1
                with open(sp, "wb") as o:
                    o.write(stream)
                out.append("unicode\t%s\t%s\t%s" % (tfm, hx(title), sp))
    out.append("end")
    return "\n".join(out) + "\n", d


# ---- PFB to PFA, and synthetic Type 1 fonts ----


def pfb_segments(d):
    i = 0
    segs = []
    while i + 1 < len(d) and d[i] == 128:
        t = d[i + 1]
        if t == 3:
            break
        n = struct.unpack("<I", d[i + 2 : i + 6])[0]
        segs.append((t, d[i + 6 : i + 6 + n]))
        i += 6 + n
    return segs


def pfb_to_pfa(d, width=64, nl=b"\n", upper=False):
    t = b""
    for kind, s in pfb_segments(d):
        if kind == 2:
            h = s.hex().encode()
            if upper:
                h = h.upper()
            t += nl.join(h[i : i + width] for i in range(0, len(h), width)) + nl
        else:
            t += s
    return t


def eexec(b, key=55665):
    out = bytearray()
    for p in b:
        c = p ^ (key >> 8)
        key = ((c + key) * 52845 + 22719) & 0xffff
        out.append(c)
    return bytes(out)


def cs_enc(plain, leniv=4):
    return eexec(b"\0" * leniv + plain, 4330)


def csnum(v):
    if -107 <= v <= 107:
        return bytes([v + 139])
    if 108 <= v <= 1131:
        v -= 108
        return bytes([247 + (v >> 8), v & 255])
    if -1131 <= v <= -108:
        v = -v - 108
        return bytes([251 + (v >> 8), v & 255])
    return b"\xff" + struct.pack(">i", v)


def cs(*items):
    """Numbers and operator names."""
    ops = dict(
        hstem=1, vstem=3, vmoveto=4, rlineto=5, hlineto=6, vlineto=7, rrcurveto=8, closepath=9, callsubr=10,
        ret=11, hsbw=13, endchar=14, rmoveto=21, hmoveto=22, vhcurveto=30, hvcurveto=31,
    )
    esc = dict(dotsection=0, vstem3=1, hstem3=2, seac=6, sbw=7, div=12, callothersubr=16, pop=17, setcurrentpoint=33)
    out = b""
    for x in items:
        if isinstance(x, int):
            out += csnum(x)
        elif isinstance(x, bytes):
            out += x
        elif x in ops:
            out += bytes([ops[x]])
        else:
            out += bytes([12, esc[x]])
    return out


SIMPLE = cs(0, 500, "hsbw", 100, 100, "rmoveto", 200, "hlineto", "closepath", "endchar")


def synth_t1(
    name=b"Syn",
    encoding=b"std",
    glyphs=None,
    subrs=None,
    leniv=None,
    rd=b"RD",
    np=b"NP",
    nd=b"ND",
    fonttype=1,
    header_extra=b"",
    private_extra=b"",
    charstrings_line=None,
    subrs_line=None,
    fontname=True,
    trailer=b"",
    declared_glyphs=None,
    fontmatrix=b"[0.001 0 0 0.001 0 0]",
    italic=b"/ItalicAngle -12.5 def",
    bbox=b"{-10 -20 1000 900}",
    tail=None,
    comment=600,
    subrs_end=b"ND\n",
    synthetic_prefix=False,
    pfb=True,
    cut=None,
    eexec_tail=b"",
):
    """A whole font program.  `glyphs` and `subrs` are lists of (name or index, plaintext charstring)."""
    if glyphs is None:
        glyphs = [(b".notdef", cs(0, 250, "hsbw", "endchar")), (b"A", SIMPLE), (b"B", SIMPLE)]
    iv = 4 if leniv is None else leniv
    enc = {
        b"std": b"/Encoding StandardEncoding def\n",
        b"dup": b"/Encoding 256 array\n0 1 255 {1 index exch /.notdef put} for\n"
        b"dup 65 /A put\ndup 66 /B put\ndup 67 /B put\nreadonly def\n",
    }.get(encoding, encoding)
    # The long comment makes t1_line_array big: the C rewrites lines without checking their room.
    h = b"%!PS-AdobeFont-1.0: " + name + b" 1.0\n%" + b"x" * comment + b"\n"
    h += b"12 dict begin\n/FontInfo 3 dict dup begin\n/FullName (" + name
    h += b") readonly def\n" + italic + b"\nend readonly def\n"
    if fontname:
        h += b"/FontName /" + name + b" def\n"
    h += b"/PaintType 0 def\n/FontType %d def\n" % fonttype
    h += b"/FontMatrix " + fontmatrix + b" readonly def\n"
    h += enc
    h += b"/FontBBox " + bbox + b" readonly def\n/UniqueID 5000000 def\n"
    h += header_extra
    h += b"currentdict end\ncurrentfile eexec\n"

    p = b"dup /Private 16 dict dup begin\n"
    p += b"/RD {string currentfile exch readstring pop} executeonly def\n"
    p += b"/ND {noaccess def} executeonly def\n/NP {noaccess put} executeonly def\n"
    p += b"/BlueValues [] def\n/password 5839 def\n/UniqueID 5000000 def\n"
    if leniv is not None:
        p += b"/lenIV %d def\n" % leniv
    p += private_extra

    def subr_block(items, count):
        b = subrs_line if subrs_line is not None else b"/Subrs %d array\n" % count
        for i, plain in items:
            e = cs_enc(plain, max(iv, 0))
            b += b"dup %d %d " % (i, len(e)) + rd + b" " + e + b" " + np + b"\n"
        b += subrs_end
        return b

    if synthetic_prefix:
        p += subr_block([(0, cs("ret"))], 1)
        p += b"hires 1 def\nhires 2 def\nhires 3 def\nhires 4 def\nhires 5 def\nhires 6 def\n"
    if subrs is not None:
        p += subr_block(subrs, len(subrs) if subrs else 0)
    p += charstrings_line if charstrings_line is not None else b"2 index /CharStrings %d dict dup begin\n" % (
        declared_glyphs if declared_glyphs is not None else len(glyphs)
    )
    for g, plain in glyphs:
        e = cs_enc(plain, max(iv, 0))
        p += b"/" + g + b" %d " % len(e) + rd + b" " + e + b" " + nd + b"\n"
    p += b"end\nend\nreadonly put\nnoaccess put\ndup /FontName get exch definefont pop\nmark currentfile closefile\n"
    p += eexec_tail
    enc_part = eexec(b"\0\0\0\0" + p)
    if tail is None:
        tail = (b"0" * 64 + b"\n") * 8 + b"cleartomark\n" + trailer
    if cut is not None:
        enc_part = enc_part[:cut]
    if pfb:
        return (
            b"\x80\x01" + struct.pack("<I", len(h)) + h
            + b"\x80\x02" + struct.pack("<I", len(enc_part)) + enc_part
            + b"\x80\x01" + struct.pack("<I", len(tail)) + tail + b"\x80\x03"
        )
    x = enc_part.hex().encode()
    return h + b"\n".join(x[i : i + 64] for i in range(0, len(x), 64)) + b"\n" + tail


def synth_cases():
    """Each case: (name, font bytes, codes, map line options)."""
    # cmr10 has every code here, so the characters always exist.
    A = [65, 66]
    hint = [
        (0, cs("ret")), (1, cs("ret")), (2, cs("ret")), (3, cs("ret")),
        (4, cs(10, 20, "hstem", "ret")), (5, cs(30, 40, "vstem", "ret")), (6, cs(50, 60, "hstem", "ret")),
    ]
    uses = [
        (b".notdef", cs(0, 250, "hsbw", "endchar")),
        (b"A", cs(0, 500, "hsbw", 4, 1, 3, "callothersubr", "pop", "callsubr", 10, 10, "rmoveto", "endchar")),
        (b"B", SIMPLE),
    ]
    flex = cs(
        0, 500, "hsbw", 1, "callsubr", 10, 10, "rmoveto", 2, "callsubr", 50, 5, 5, 3, 0, "callothersubr",
        "pop", "pop", "setcurrentpoint", "endchar",
    )
    array_enc = b"/Encoding [ /.notdef /A /B\n% comment\n/B /.notdef ] readonly def\n"
    dup_ops = (
        b"/Encoding 256 array\n0 1 255 {1 index exch /.notdef put} for\n"
        b"dup 65 /A put dup 66 /B put\ndup dup 67 exch 65 get put\ndup dup 65 1 getinterval 70 exch putinterval\n"
        b"dup 0x48 /B put\ndup 010 /B put\nsome junk here\nreadonly def\n"
    )
    seac_glyphs = [
        (b".notdef", cs(0, 250, "hsbw", "endchar")),
        (b"A", SIMPLE),
        (b"B", SIMPLE),
        (b"acute", SIMPLE),
        (b"Aacute", cs(0, 500, "hsbw", 0, 0, 0, 65, 194, "seac", "endchar")),
    ]
    aenc = b"/Encoding 256 array\n0 1 255 {1 index exch /.notdef put} for\ndup 65 /A put\ndup 66 /Aacute put\nreadonly def\n"
    cases = [
        ("std", synth_t1(), A, b""),
        ("std_pfa", synth_t1(pfb=False), A, b""),
        ("dupenc", synth_t1(encoding=b"dup"), [65, 67, 68], b""),
        ("arrayenc", synth_t1(encoding=array_enc), [1, 2, 3, 4], b""),
        ("dupops", synth_t1(encoding=dup_ops), [65, 66, 67, 70, 72, 8], b""),
        ("seac", synth_t1(encoding=aenc, glyphs=seac_glyphs), [66], b""),
        ("seac_std", synth_t1(glyphs=seac_glyphs), [65], b""),
        ("hints", synth_t1(subrs=hint, glyphs=uses), [65], b""),
        ("hints_b", synth_t1(subrs=hint, glyphs=uses), [66], b""),
        ("flex", synth_t1(subrs=hint, glyphs=[uses[0], (b"A", flex)]), [65], b""),
        ("noreturn", synth_t1(subrs=hint + [(7, cs(1, 2, "hstem"))],
                              glyphs=[uses[0], (b"A", cs(0, 500, "hsbw", 7, "callsubr", "endchar"))]), [65], b""),
        ("emptysubr", synth_t1(subrs=[(0, b"")],
                               glyphs=[uses[0], (b"A", cs(0, 500, "hsbw", 0, "callsubr", "endchar"))]), [65], b""),
        ("div", synth_t1(glyphs=[uses[0], (b"A", cs(0, 1000, 2, "div", "hsbw", 70000, 2, "div", 0, "rmoveto", "endchar"))]), [65], b""),
        ("leniv0", synth_t1(leniv=0, subrs=hint, glyphs=uses), [65], b""),
        ("leniv1", synth_t1(leniv=1, subrs=hint, glyphs=uses), [65], b""),
        ("leniv7", synth_t1(leniv=7, subrs=hint, glyphs=uses), [65], b""),
        ("tokens_dash", synth_t1(rd=b"-|", np=b"|", nd=b"|-", subrs=hint, glyphs=uses,
                                 private_extra=b"/-| {string currentfile exch readstring pop} executeonly def\n"), [65], b""),
        ("tokens_put", synth_t1(np=b"noaccess put", subrs=hint, glyphs=uses), [65], b""),
        ("tokens_dash_put", synth_t1(rd=b"-|", np=b"noaccess put", nd=b"|-", subrs=hint, glyphs=uses), [65], b""),
        ("tokens_bad", synth_t1(np=b"XX", subrs=hint, glyphs=uses), [65], b""),
        ("unusual_cs", synth_t1(charstrings_line=b"dup/CharStrings\n3 dict dup begin\n"), [65], b""),
        ("subrs0", synth_t1(subrs=[], glyphs=[uses[0], (b"A", SIMPLE)]), [65], b""),
        ("synthetic", synth_t1(subrs=hint, glyphs=uses, synthetic_prefix=True), [65], b""),
        ("nofontname", synth_t1(fontname=False), [65], b""),
        ("type3", synth_t1(fonttype=3), [65], b""),
        ("leniv_neg", synth_t1(leniv=-1), [65], b""),
        ("undefined_glyph", synth_t1(encoding=b"/Encoding 256 array\n0 1 255 {1 index exch /.notdef put} for\ndup 65 /A put\ndup 66 /Nope put\nreadonly def\n"), [65, 66], b""),
        ("notdef_code", synth_t1(encoding=b"dup"), [65, 1], b""),
        ("subr_range", synth_t1(subrs=hint, glyphs=[uses[0], (b"A", cs(0, 500, "hsbw", 99, "callsubr", "endchar"))]), [65], b""),
        ("subr_store_range", synth_t1(subrs=hint, subrs_line=b"/Subrs 2 array\n", glyphs=uses), [65], b""),
        ("bad_command", synth_t1(glyphs=[uses[0], (b"A", b"\x02" + SIMPLE)]), [65], b""),
        ("bad_escape", synth_t1(glyphs=[uses[0], (b"A", b"\x0c\x05" + SIMPLE)]), [65], b""),
        ("escape_range", synth_t1(glyphs=[uses[0], (b"A", b"\x0c\x7f" + SIMPLE)]), [65], b""),
        ("more_args", synth_t1(glyphs=[uses[0], (b"A", cs(1, 0, 500, "hsbw", "endchar"))]), [65], b""),
        ("less_args", synth_t1(glyphs=[uses[0], (b"A", cs(500, "hsbw", "endchar"))]), [65], b""),
        ("stack_pop", synth_t1(glyphs=[uses[0], (b"A", cs("div", "endchar"))]), [65], b""),
        ("too_many_glyphs", synth_t1(declared_glyphs=1), [65], b""),
        ("no_number", synth_t1(charstrings_line=b"2 index /CharStrings x dict dup begin\n"), [65], b""),
        ("bad_fontmatrix", synth_t1(fontmatrix=b"0.001 0 0 0.001 0 0"), [65], b"\"0.2 SlantFont\""),
        ("brace_fontmatrix", synth_t1(fontmatrix=b"{0.001 0 0 0.001 0 0}"), [65], b"\"0.2 SlantFont 1.1 ExtendFont\""),
        ("unclosed_fontmatrix", synth_t1(fontmatrix=b"[0.001 0 0 0.001 0 0"), [65], b"\"0.2 SlantFont\""),
        ("pred_enc_bad", synth_t1(encoding=b"/Encoding Foo def\n"), [65], b""),
        ("array_enc_bad", synth_t1(encoding=b"/Encoding [ /A /B foo ] readonly def\n"), [65], b""),
        ("noname_font", synth_t1(header_extra=b"/FontName 5 def\n"), [65], b""),
        ("uniqueid_forms", synth_t1(header_extra=b"/UniqueID 77 def\n/UniqueID 77 noaccess\n"), [65], b""),
        ("stemv", synth_t1(header_extra=b"/StdVW [85] def\n/XHeight 431 def\n/CapHeight 683 def\n/Ascender 694 def\n/Descender -194 def\n"), [65], b""),
        ("truncated", synth_t1(cut=60), [65], b""),
        ("long_line", synth_t1(comment=9000), [65], b""),
        ("subrs_no_end", synth_t1(subrs=hint, glyphs=uses, subrs_end=b""), [65], b""),
        ("subr_bad_command", synth_t1(subrs=hint + [(7, b"\x02")],
                                      glyphs=[uses[0], (b"A", cs(0, 500, "hsbw", 7, "callsubr", "endchar"))]), [65], b""),
        ("subr_missing", synth_t1(subrs=hint[:4] + [(6, cs("ret"))], subrs_line=b"/Subrs 8 array\n",
                                  glyphs=[uses[0], (b"A", cs(0, 500, "hsbw", 5, "callsubr", "endchar"))]), [65], b""),
        ("array_enc_too_many", synth_t1(encoding=b"/Encoding [\n" + b"/A " * 300 + b"\n] readonly def\n"), [65], b""),
        ("long_charstring", synth_t1(glyphs=[uses[0], (b"A", cs(0, 500, "hsbw") + cs(1, 1, "rlineto") * 12000 + cs("endchar"))]), [65], b""),
        ("bbox_huge", synth_t1(bbox=b"{1e20 -1e20 1000 900}"), [65], b""),
        ("bbox_nan", synth_t1(bbox=b"{nan -20 inf 900}"), [65], b""),
        ("italic_huge", synth_t1(italic=b"/ItalicAngle 1e20 def"), [65], b"\"0.2 SlantFont\""),
        ("italic_huge_neg", synth_t1(italic=b"/ItalicAngle -1e20 def"), [65], b"\"0.2 SlantFont\""),
        ("dup_on_one_line", synth_t1(encoding=(
            b"/Encoding 256 array\n0 1 255 {1 index exch /.notdef put} for\n"
            b"dup 65 /A put dup 66 /B put dup dup 67 exch 65 get put dup dup 65 1 getinterval 70 exch putinterval dup 71 /B put\n"
            b"dup 0b1001000 /A put\ndup -1 /A put\ndup +73 /A put\ndup 99999999999999999999 /A put\ndup 74 A put\n"
            b"readonly def\n")), [65, 66, 67, 70, 71, 72, 73, 74], b""),
        ("dup_same_cell", synth_t1(encoding=(
            b"/Encoding 256 array\n0 1 255 {1 index exch /.notdef put} for\ndup 65 /A put\ndup dup 65 exch 65 get put\n"
            b"dup 66 /B put\ndup 66 /.notdef put\nreadonly def\n")), [65, 66], b""),
        ("empty_file", b"", [65], b""),
        ("marker_at_once", b"\x80\x03", [65], b""),
        ("header_only", synth_t1(pfb=False)[:200], [65], b""),
        ("pfb_marker3_early", b"\x80\x01" + struct.pack("<I", 12) + b"%!PS-Adobe\n\n" + b"\x80\x03", [65], b""),
        ("pfb_extra", synth_t1(eexec_tail=b"XX\n"), [65], b""),
        ("pfa_bad_tail", synth_t1(pfb=False, tail=b"cleartomark\n"), [65], b""),
        ("pfa_ff_tail", synth_t1(pfb=False, tail=b"ffff\n"), [65], b""),
        ("pfa_no_tail", synth_t1(pfb=False, tail=b""), [65], b""),
    ]
    return cases


def synth_job(name, font, codes, specials):
    fn = "syn-%s.%s" % (name.replace("_", "-"), "pfa" if font[:1] != b"\x80" and font else "pfb")
    line = b"=cmr10 Syn " + specials + b" <" + fn.encode()
    return Job("syn_" + name, [Font("cmr10", codes, line, "line", [(fn, font)])])


def synth_jobs():
    hint = [
        (0, cs("ret")), (1, cs("ret")), (2, cs("ret")), (3, cs("ret")),
        (4, cs(10, 20, "hstem", "ret")), (5, cs(30, 40, "vstem", "ret")), (6, cs(50, 60, "hstem", "ret")),
    ]
    uses = [
        (b".notdef", cs(0, 250, "hsbw", "endchar")),
        (b"A", cs(0, 500, "hsbw", 4, 1, 3, "callothersubr", "pop", "callsubr", 10, 10, "rmoveto", "endchar")),
        (b"B", SIMPLE),
    ]
    jobs = []
    for name, font, codes, specials in synth_cases():
        jobs.append(synth_job(name, font, codes, specials))
    # Encoding files in odd shapes, on a font with those glyphs.
    encs = [
        ("crlf", b"/E [ /A /B\r\n/.notdef\r\n/B ] def\r\n"),
        ("tabs", b"% header\n\n/E\t[\t/A\t/B\n\t/B\t] def\n"),
        ("comment_end", b"/E [ /A % one\n/B % two\n] def % done\n"),
        ("glued", b"/E[/A/B/A/B] def\n"),
        ("no_name", b"E [ /A ] def\n"),
        ("no_bracket", b"/E /A /B ] def\n"),
        ("no_def", b"/E [ /A /B ]\n"),
        ("junk", b"/E [ /A foo /B ] def\n"),
        ("too_many", b"/E [ " + b"/A " * 300 + b"] def\n"),
        ("too_long", b"/E [ /" + b"A" * 5000 + b" ] def\n"),
        ("eol_at_limit", b"/E [ /" + b"A" * 4089 + b"\n] def\n"),
        ("no_end", b"/E [ /A /B\n"),
        ("empty", b""),
        ("only_comment", b"% nothing\n"),
    ]
    for name, data in encs:
        font = synth_t1()
        jobs.append(Job("enc_" + name, [Font(
            "cmr10", [0, 1, 2, 3], b"=cmr10 Syn <e-%s.enc <syn.pfb" % name.encode(), "line",
            [("syn.pfb", font), ("e-%s.enc" % name, data)],
        )]))
    # A file not starting with a PFB marker in the middle: a bad segment marker.
    good = synth_t1()
    k = good.index(b"\x80\x02")
    bad = good[:k] + b"\x81" + good[k + 1 :]
    jobs.append(Job("syn_bad_marker", [Font("cmr10", [65], b"=cmr10 Syn <syn-bad.pfb", "line", [("syn-bad.pfb", bad)])]))
    # The statics that outlive one font: `pop` with no callothersubr, in the second font by name order.
    a = synth_t1(name=b"CarryA", subrs=hint, glyphs=uses)
    b = synth_t1(name=b"CarryB", subrs=hint, glyphs=[uses[0], (b"A", cs(0, 500, "hsbw", "pop", "callsubr", "endchar"))])
    jobs.append(Job("syn_carry", [
        Font("cmr10", [65], b"=cmr10 CarryA <syn-carry-a.pfb", "line", [("syn-carry-a.pfb", a)]),
        Font("cmr12", [65], b"=cmr12 CarryB <syn-carry-b.pfb", "line", [("syn-carry-b.pfb", b)]),
    ]))
    # A font without /FontName after one with: t1_fontname_offset goes on from the first.
    c = synth_t1(name=b"StaleA")
    d = synth_t1(name=b"StaleB", fontname=False)
    jobs.append(Job("syn_stale_offset", [
        Font("cmr10", [65], b"=cmr10 StaleA <syn-stale-a.pfb", "line", [("syn-stale-a.pfb", c)]),
        Font("cmr12", [65], b"=cmr12 StaleB <syn-stale-b.pfb", "line", [("syn-stale-b.pfb", d)]),
    ]))
    # A charstring that leaves a number on the stack, for the next one.
    e = synth_t1(name=b"LeftA", glyphs=[(b".notdef", cs(0, 250, "hsbw", "endchar")),
                                        (b"A", cs(0, 500, "hsbw", 7))])
    jobs.append(Job("syn_left_on_stack", [Font("cmr10", [65], b"=cmr10 LeftA <syn-left.pfb", "line", [("syn-left.pfb", e)])]))
    return jobs


# ---- the rest of the jobs ----


# ---- a synthetic TrueType font, for what the three in TeX Live do not reach ----


def tt_simple(odd=False):
    """A glyph with one point; `odd` leaves it at an odd length."""
    g = struct.pack(">hhhhh", 1, 0, 0, 100, 100) + struct.pack(">HH", 0, 0) + bytes([1, 0, 0])
    return g if odd or len(g) % 2 == 0 else g + b"\0"


def tt_composite(parts, instructions=b""):
    """`parts`: (glyph, flags, argument bytes).  MORE_COMPONENTS is set here."""
    g = struct.pack(">hhhhh", -1, 0, 0, 100, 100)
    for i, (gid, flags, args) in enumerate(parts):
        if i + 1 < len(parts):
            flags |= 1 << 5
        if instructions:
            flags |= 1 << 8
        g += struct.pack(">HH", flags, gid) + args
    if instructions:
        g += struct.pack(">H", len(instructions)) + instructions
    return g + (b"\0" if len(g) % 2 else b"")


def tt_cmap4(mapping, range_offset=False):
    """A format 4 subtable.  With `range_offset` the first segment goes through glyphIdArray."""
    codes = sorted(mapping)
    segs = []
    for c in codes:
        if segs and c == segs[-1][1] + 1 and mapping[c] == mapping[segs[-1][0]] + c - segs[-1][0]:
            segs[-1][1] = c
        else:
            segs.append([c, c])
    segs.append([0xFFFF, 0xFFFF])
    n = len(segs)
    ends = b"".join(struct.pack(">H", e) for s, e in segs)
    starts = b"".join(struct.pack(">H", s) for s, e in segs)
    deltas = b""
    offs = b""
    ids = b""
    for k, (s, e) in enumerate(segs):
        if s == 0xFFFF:
            deltas += struct.pack(">H", 1)
            offs += struct.pack(">H", 0)
        elif range_offset and k == 0:
            deltas += struct.pack(">H", 0)
            offs += struct.pack(">H", 2 * (n - k))
            for c in range(s, e + 1):
                ids += struct.pack(">H", mapping[c])
        else:
            deltas += struct.pack(">H", (mapping[s] - s) & 0xFFFF)
            offs += struct.pack(">H", 0)
    body = struct.pack(">HHHH", 2 * n, 0, 0, 0) + ends + b"\0\0" + starts + deltas + offs + ids
    return struct.pack(">HHH", 4, 6 + len(body), 0) + body


def tt_cmap_table(subs):
    """`subs`: (pid, eid, subtable)."""
    head = struct.pack(">HH", 0, len(subs))
    off = 4 + 8 * len(subs)
    body = b""
    for pid, eid, t in subs:
        head += struct.pack(">HHI", pid, eid, off + len(body))
        body += t
    return head + body


def synth_ttf(
    n=40,
    post=0x20000,
    glyphs=None,
    names=None,
    loca_long=False,
    cmap=None,
    pclt=False,
    extras=True,
    os2_version=1,
    name_records=None,
    nhmtx=None,
    ttc=False,
    italic=-0xC8000,  # -12.5
    upem=2048,
    omit=(),
    otf=False,
):
    """`glyphs`: glyph number to bytes, the rest simple.  `names`: glyph number to a /post name."""
    glyphs = glyphs or {}
    names = names or {}
    data = [glyphs.get(i, tt_simple()) for i in range(n)]
    loca = [0]
    for g in data:
        loca.append(loca[-1] + len(g))
    if not loca_long:
        assert all(x % 2 == 0 and x < 0x20000 for x in loca)
    t = {}
    t[b"head"] = struct.pack(">IIIIHH", 0x00010000, 0x00010000, 0, 0x5F0F3CF5, 0, upem) + b"\0" * 16 + struct.pack(
        ">hhhhHHhhh", -100, -200, 1100, 900, 0, 8, 2, 1 if loca_long else 0, 0
    )
    hm = n if nhmtx is None else nhmtx
    t[b"hhea"] = struct.pack(">IhhhHhhhhhh", 0x00010000, 800, -200, 0, 1000, 0, 0, 900, 1, 0, 0) + b"\0" * 8 + struct.pack(">hH", 0, hm)
    t[b"maxp"] = struct.pack(">IH", 0x00010000, n) + b"\0" * 26
    t[b"hmtx"] = b"".join(struct.pack(">Hh", 500 + i, 10) for i in range(hm)) + b"".join(struct.pack(">h", 10) for i in range(hm, n))
    t[b"glyf"] = b"".join(data)
    t[b"loca"] = b"".join(struct.pack(">I", x) for x in loca) if loca_long else b"".join(struct.pack(">H", x // 2) for x in loca)
    p = struct.pack(">IIhhIIIII", post, italic & 0xFFFFFFFF, -100, 50, 0, 0, 0, 0, 0)
    if post == 0x20000:
        idx = b""
        extra = b""
        k = 0
        for i in range(n):
            nm = names.get(i, b"g%03d" % i)
            if isinstance(nm, int):
                idx += struct.pack(">H", nm)  # a Macintosh name by number
            else:
                idx += struct.pack(">H", 258 + k)
                extra += bytes([len(nm)]) + nm
                k += 1
        p += struct.pack(">H", n) + idx + extra
    t[b"post"] = p
    recs = name_records
    if recs is None:
        recs = [(1, 0, 0, 6, b"SynTT"), (1, 0, 0, 1, b"Syn TT"), (3, 1, 0x409, 6, "SynTT".encode("utf-16-be")), (3, 1, 0x409, 2, "x".encode("utf-16-be"))]
    strs = b""
    nm = struct.pack(">HHH", 0, len(recs), 6 + 12 * len(recs))
    for pid, eid, lang, nid, text in recs:
        nm += struct.pack(">HHHHHH", pid, eid, lang, nid, len(text), len(strs))
        strs += text
    t[b"name"] = nm + strs
    t[b"OS/2"] = struct.pack(">H", os2_version) + b"\x01" * 76 + struct.pack(">II", 0, 0)
    if cmap is None:
        cmap = [(3, 1, tt_cmap4({0x41 + i: 3 + i for i in range(20)}))]
    t[b"cmap"] = tt_cmap_table(cmap)
    if pclt:
        t[b"PCLT"] = struct.pack(">IIHHHH", 0x00010000, 0, 1000, 450, 0, 650) + b"\0" * 36
    if extras:
        t[b"fpgm"] = b"\xb0\x01\xb0"
        t[b"cvt "] = b"\0\x10\0\x20"
        t[b"prep"] = b"\xb1\x01"
    if otf:
        t[b"CFF "] = bytes(range(256)) * 3 + b"\xff\xfe\xfd"
        for k in (b"glyf", b"loca", b"maxp", b"hmtx", b"cmap", b"name", b"OS/2"):
            t.pop(k, None)
    for k in omit:
        t.pop(k, None)
    tags = sorted(t)
    base = 16 if ttc else 0
    off = base + 12 + 16 * len(tags)
    d = struct.pack(">IHHHH", 0x00010000, len(tags), 0, 0, 0)
    body = b""
    for tag in tags:
        d += tag + struct.pack(">III", 0, off + len(body), len(t[tag]))
        body += t[tag] + b"\0" * (-len(t[tag]) % 4)
    font = d + body
    if ttc:
        return b"ttcf" + struct.pack(">III", 0x00010000, 1, 16) + font
    return font


def synth_ttf_jobs():
    jobs = []
    with open(kpse("cmr10.tfm"), "rb") as f:
        cmr = f.read()
    with open(kpse("ec-lmr10.tfm"), "rb") as f:
        t1tfm = f.read()

    def enc_of(items):
        return b"/E [ " + b" ".join(b"/" + x for x in items) + b" ] def\n"

    mixed = enc_of([
        b"g005", b"g006", b"uni0041", b"uni0042", b"uni0043", b"index7", b"index0x8", b"nosuch", b"uniFFFD", b"index999",
        b".notdef", b"g007", b"a", b"g009",
    ])

    def case(name, font, enc=mixed, codes=None, fn="syn.ttf", line=None, tfm="cmr10", files=(), tfmfile=None):
        codes = list(range(14)) if codes is None else codes
        fl = [(fn, font)] + list(files)
        l = line
        if l is None:
            l = b"=%s " % tfm.encode()
            if enc is not None:
                fl.append(("syn.enc", enc))
                l += b"<syn.enc "
            l += b"<" + fn.encode()
        if tfmfile is not None:
            fl.append((tfm + ".tfm", tfmfile))
        jobs.append(Job("ttfsyn_" + name, [Font(tfm, codes, l, "line", fl)]))

    # Composites with every kind of argument and transform, one nested, some with instructions.
    comp = {
        10: tt_composite([(11, 1, struct.pack(">hh", 1, 2)), (12, 0, b"\x01\x02")]),
        11: tt_composite([(12, 1 << 3, struct.pack(">BBh", 1, 2, 0x4000))]),
        12: tt_composite([(13, 1 << 6, struct.pack(">BBhh", 1, 2, 0x4000, 0x4000))], b"\xb0\x01"),
        13: tt_composite([(14, 1 << 7, struct.pack(">BBhhhh", 1, 2, 0x4000, 0, 0, 0x4000)), (15, 0, b"\x03\x04")]),
    }
    comp_enc = enc_of([b"g010", b"g013", b"g005"])
    case("composite", synth_ttf(glyphs=comp), comp_enc, [0, 1, 2])
    case("composite_long", synth_ttf(glyphs=comp, loca_long=True), comp_enc, [0, 1, 2])
    # /loca: an odd offset, a table too long for the short form.
    case("loca_odd", synth_ttf(glyphs={5: tt_simple(True), 6: tt_simple(True)}, loca_long=True), enc_of([b"g005", b"g006", b"g007"]), [0, 1, 2])
    case("loca_big", synth_ttf(n=300, glyphs={i: tt_simple() + b"\0" * 1200 for i in range(2, 200)}, loca_long=True), enc_of([b"g%03d" % (i + 2) for i in range(128)]), list(range(128)))
    # The /post formats.  Format 1 needs the 258 Macintosh glyphs: the C writes them regardless.
    many = 300
    case("post1", synth_ttf(n=many, post=0x10000), enc_of([b"space", b"exclam", b"Delta", b"macron", b"periodcentered", b"a", b"dmacron", b"g005"]), list(range(8)))
    case("post3", synth_ttf(n=many, post=0x30000), mixed)
    case("post25", synth_ttf(n=many, post=0x28000), mixed)
    named = {5: 3, 6: 4, 7: 171, 8: 226, 9: b"Omega", 12: 0}
    case("post2_mac", synth_ttf(post=0x20000, names=named), enc_of([b"space", b"exclam", b"Delta", b"periodcentered", b"Omega", b"g010"]), list(range(6)))
    # cmap: none that is Unicode, (0,3), an unsupported format, a glyph array, codes mapped twice.
    case("cmap_none", synth_ttf(cmap=[(1, 0, tt_cmap4({0x41: 3}))]), mixed)
    case("cmap_03", synth_ttf(cmap=[(0, 3, tt_cmap4({0x41: 3, 0x42: 9}))]), mixed)
    case("cmap_f6", synth_ttf(cmap=[(3, 1, struct.pack(">HHH", 6, 10, 0) + b"\0" * 4)]), mixed)
    case("cmap_array", synth_ttf(cmap=[(3, 1, tt_cmap4({0x41: 3, 0x42: 7, 0x43: 2, 0x60: 9}, True))]), mixed)
    dbl = tt_cmap4({0x41: 3, 0x42: 4})
    # Two segments over one code, made by hand: 0x41-0x42 and 0x42-0x43.
    twice = struct.pack(">HHH", 4, 8 * 2 + 6 + 2 + 3 * 8, 0) + struct.pack(">HHHH", 6, 0, 0, 0)
    twice += struct.pack(">HHH", 0x42, 0x43, 0xFFFF) + b"\0\0" + struct.pack(">HHH", 0x41, 0x42, 0xFFFF)
    twice += struct.pack(">HHH", (3 - 0x41) & 0xFFFF, (9 - 0x42) & 0xFFFF, 1) + b"\0" * 6
    case("cmap_twice", synth_ttf(cmap=[(3, 1, twice)]), mixed)
    case("cmap_range", synth_ttf(n=10, cmap=[(3, 1, tt_cmap4({0x41: 50}))]), mixed)
    case("cmap_glyph_zero", synth_ttf(cmap=[(3, 1, tt_cmap4({0x41: 0}))]), mixed)
    bad_offset = struct.pack(">HHH", 4, 8 * 2 + 6 + 2 + 2 * 8, 0) + struct.pack(">HHHH", 4, 0, 0, 0)
    bad_offset += struct.pack(">HH", 0x41, 0xFFFF) + b"\0\0" + struct.pack(">HH", 0x41, 0xFFFF)
    bad_offset += struct.pack(">HH", 0, 1) + struct.pack(">HH", 100, 0)
    case("cmap_bad_offset", synth_ttf(cmap=[(3, 1, bad_offset)]), mixed)
    case("index_zero", synth_ttf(), enc_of([b"index0"]), [0])
    # Enough glyphs for the trimmed table: 256 names in a font of more.
    all_names = enc_of([b"g%03d" % (i + 2) for i in range(256)])
    case("trimmed", synth_ttf(n=many), all_names, list(range(256)), tfm="zzt1", tfmfile=t1tfm)
    # Tables that may be there or not, and odd contents.
    case("pclt", synth_ttf(pclt=True), mixed)
    case("no_extras", synth_ttf(extras=False), mixed)
    case("os2_v6", synth_ttf(os2_version=6), mixed)
    case("hmtx_short", synth_ttf(nhmtx=5), mixed)
    case("upem1000", synth_ttf(upem=1000), mixed)
    case("italic_pos", synth_ttf(italic=0x18000), mixed)
    case("ttc", synth_ttf(ttc=True), mixed, fn="syn.ttc")
    case("ttc_plain", synth_ttf(), mixed, fn="plain.ttc")
    case("whole", synth_ttf(), None, [65], line=b"=cmr10 <<syn.ttf")
    case("whole_pclt", synth_ttf(pclt=True, extras=False), None, [65], line=b"=cmr10 ZZ <<syn.ttf")
    case("no_post", synth_ttf(omit=[b"post"]), mixed)
    case("no_os2", synth_ttf(omit=[b"OS/2"]), mixed)
    case("no_cmap", synth_ttf(omit=[b"cmap"]), mixed)
    case("truncated", synth_ttf()[:60], mixed)
    case("truncated_late", synth_ttf()[:700], mixed)
    # The name table: only Unicode, with delimiters, none.
    uni = [(3, 1, 0x409, 6, "Uni Name(x)".encode("utf-16-be"))]
    case("name_uni", synth_ttf(name_records=uni), mixed)
    case("name_mac_delims", synth_ttf(name_records=[(1, 0, 0, 6, b"Mac <Name> [x] {y} /z %w")]), mixed)
    case("name_none", synth_ttf(name_records=[(1, 0, 0, 2, b"Regular")]), mixed)
    case("name_long_mac", synth_ttf(name_records=[(1, 0, 0, 6, b"M" * 300)]), mixed)
    case("name_long_uni", synth_ttf(name_records=[(3, 1, 0, 6, b"\0U" * 150)]), mixed)
    case("name_ids", synth_ttf(name_records=[(1, 0, 0, 1, b"A"), (1, 0, 0, 3, b"B"), (1, 0, 0, 4, b"C"), (1, 0, 0, 6, b"D"), (3, 0, 0, 1, b"\0E"), (3, 1, 0, 4, b"\0F"), (2, 0, 0, 1, b"G")]), mixed)
    # Subfonts.
    sfd = b"01 0x41_0x44 0x7000 \\\n 0x45\n02 0x10000\n"
    files = [("sft.sfd", sfd), ("sub01.tfm", cmr), ("sub02.tfm", cmr)]
    jobs.append(Job("ttfsyn_subfont", [Font("sub01", [0, 1, 2, 3, 4, 5, 6], b"+sub@sft@ <syn.ttf", "line", [("syn.ttf", synth_ttf())] + files)]))
    jobs.append(Job("ttfsyn_subfont_big_code", [Font("sub02", [0], b"+sub@sft@ <syn.ttf", "line", [("syn.ttf", synth_ttf())] + files)]))
    jobs.append(Job("ttfsyn_subfont_pideid", [Font("sub01", [0, 1], b"+sub@sft@ Y PidEid=0,3 <syn.ttf", "line", [("syn.ttf", synth_ttf(cmap=[(0, 3, tt_cmap4({0x41: 3, 0x42: 4}))]))] + files)]))
    jobs.append(Job("ttfsyn_subfont_nocmap", [Font("sub01", [0, 1], b"+sub@sft@ Y PidEid=1,0 <syn.ttf", "line", [("syn.ttf", synth_ttf())] + files)]))
    jobs.append(Job("ttfsyn_subfont_f6", [Font("sub01", [0, 1], b"+sub@sft@ <syn.ttf", "line", [("syn.ttf", synth_ttf(cmap=[(3, 1, struct.pack(">HHH", 6, 10, 0) + b"\0" * 4)]))] + files)]))
    # Two subfonts of one file: the cmap is read and warned about once.
    jobs.append(Job("ttfsyn_subfont_two", [
        Font("sub01", [0, 1], b"+sub@sft@ <syn.ttf", "line", [("syn.ttf", synth_ttf(cmap=[(3, 1, twice)]))] + files),
        Font("sub02", [1], b"+sub@sft@ <syn.ttf", "line"),
    ]))
    # OpenType.
    case("otf", synth_ttf(otf=True, pclt=True), None, [65], fn="syn.otf", line=b"=cmr10 ZZ <<syn.otf")
    case("otf_no_cff", synth_ttf(otf=True, omit=[b"CFF "]), None, [65], fn="syn.otf", line=b"=cmr10 ZZ <<syn.otf")
    case("otf_no_post", synth_ttf(otf=True, omit=[b"post"]), None, [65], fn="syn.otf", line=b"=cmr10 ZZ <<syn.otf")
    return jobs


def tou_jobs():
    """The glyph name forms of set_glyph_unicode, and enough ranges and strings for several blocks."""
    font = synth_t1()
    forms = [
        b"A", b"a", b"B", b"C", b"Gamma", b"A.sc", b".foo", b"ffi", b"f_f_i", b"A_B", b"a_B", b"uni0041_uni0042",
        b"nosuch_A", b"uni0041", b"uni00410042", b"uniD800", b"uni004", b"uni00410", b"unizzzz", b"u1F600", b"u0041",
        b"u41", b"u110000", b"uD800", b"u1234567", b"_", b"A_", b"_A", b"uni00FF", b"uni0100", b"uni01FF", b"uni0200",
        b"uni", b"u", b"unix", b"ux",
    ]
    jobs = [Job("tou_forms", [Font(
        "cmr10", [65], b"=cmr10 Syn <forms.enc <syn.pfb", "line",
        [("syn.pfb", font), ("forms.enc", b"/E [ " + b" ".join(b"/" + x for x in forms) + b" ] def\n")],
    )], tounicode=True)]
    across = [b"uni%04X" % (0xE0 + i) for i in range(256)]
    jobs.append(Job("tou_across_bytes", [Font(
        "cmr10", [65], b"=cmr10 Syn <across.enc <syn.pfb", "line",
        [("syn.pfb", font), ("across.enc", b"/E [ " + b" ".join(b"/" + x for x in across) + b" ] def\n")],
    )], tounicode=True))
    ranges = []
    for i in range(128):
        ranges += [b"uni%04X" % (0x1000 + 16 * i), b"uni%04X" % (0x1000 + 16 * i + 1)]
    jobs.append(Job("tou_many_ranges", [Font(
        "cmr10", [65], b"=cmr10 Syn <ranges.enc <syn.pfb", "line",
        [("syn.pfb", font), ("ranges.enc", b"/E [ " + b" ".join(b"/" + x for x in ranges) + b" ] def\n")],
    )], tounicode=True))
    singles = [b"uni%04X" % (0x2000 + 16 * i) for i in range(256)]
    jobs.append(Job("tou_many_singles", [Font(
        "cmr10", [65], b"=cmr10 Syn <singles.enc <syn.pfb", "line",
        [("syn.pfb", font), ("singles.enc", b"/E [ " + b" ".join(b"/" + x for x in singles) + b" ] def\n")],
    )], tounicode=True))
    # The assertions on lengths: a long name with a component, and strings that add up.
    long_name = b"A_" + b"x" * 300
    jobs.append(Job("tou_long_name", [Font(
        "cmr10", [65], b"=cmr10 Syn <long.enc <syn.pfb", "line",
        [("syn.pfb", font), ("long.enc", b"/E [ /" + long_name + b" ] def\n")],
    )], tounicode=True))
    long_seq = b"ffi_" * 40 + b"ffi"
    jobs.append(Job("tou_long_seq", [Font(
        "cmr10", [65], b"=cmr10 Syn <longseq.enc <syn.pfb", "line",
        [("syn.pfb", font), ("longseq.enc", b"/E [ /" + long_seq + b" ] def\n")],
    )], tounicode=True))
    # A name with no component that is too long is cut by snprintf: it finds nothing.
    jobs.append(Job("tou_long_plain", [Font(
        "cmr10", [65], b"=cmr10 Syn <longp.enc <syn.pfb", "line",
        [("syn.pfb", font), ("longp.enc", b"/E [ /" + b"x" * 300 + b" ] def\n")],
    )], tounicode=True))
    return jobs


def line_jobs():
    bad = [
        b'=zz1 Y "1.5 SlantFont" <cmr10.pfb',
        b'=zz2 Y "3 ExtendFont" <cmr10.pfb',
        b'=zz3 Y "1 SlantFont <cmr10.pfb',
        b'=zz4 Y cmr10.pfb',
        b'=zz5 Y "0.2 SlantFont"',
        b'=zz6 Y "3 FooBar" <cmr10.pfb',
        b'=zz7 <foo.ttf <8r.enc <<a.ttf',
        b'=zz8 Y <<8r.enc <<a.ttf',
        b'=zz9 PidEid=3,1 <a.pfb',
        b'=zz10 Y PidEid=3,1 <a.pfb',
        b'=<cmr10.pfb',
        b'=zz12 <<cmr10.pfb "1.0005 ExtendFont"',
        b'=zz13 Y 34 <cmr10.pfb',
        b'=zz14 8r.enc <cmr10.pfb',
        b'=zz15 Y "nan SlantFont" <cmr10.pfb',
        b'=zz16 Y "0x1p-3 SlantFont" <cmr10.pfb',
        b'=zz17 Y "1e-1 SlantFont" <cmr10.pfb',
        b'=zz18 Y "0.5ExtendFont" <cmr10.pfb',
        b'=zz19 Y "-1.0005 ExtendFont" <cmr10.pfb',
        b'=zz20 Y "TeXBase1Encoding ReEncodeFont" <8r.enc <cmr10.pfb',
        b'=zz21 Y "inf SlantFont" <cmr10.pfb',
        b"=* a comment",
        b"=# a comment",
        b"=; a comment",
        b"=% a comment".replace(b"%", b"\\csname @percentchar\\endcsname") if False else b"=; another",
        b"=zz26",
        b"=zz27 Y < a.pfb",
        b"=zz28 Y << a.pfb",
        b"=zz29 Y <[ a.pfb",
        b"=zz30 <ab",
        b"=zz" + b"x" * 1100 + b" Y <cmr10.pfb",
        b"=zz32 Y " + b"y" * 1100 + b" <cmr10.pfb",
        b"=zz33 Y <" + b"z" * 1100 + b".pfb",
        b"-zz34 Y <cmr10.pfb",
        b"=zz35 Y \"nano SlantFont\" <cmr10.pfb",
        b"=zz36 Y \"info SlantFont\" <cmr10.pfb",
        b"=zz37 Y \"infinit SlantFont\" <cmr10.pfb",
        b"=zz38 Y \"infinity SlantFont\" <cmr10.pfb",
        b"=zz39 Y \"- SlantFont\" <cmr10.pfb",
        b"=zz40 Y \"+ SlantFont\" <cmr10.pfb",
        b"=zz41 Y \"0x SlantFont\" <cmr10.pfb",
        b"=zz42 Y \". SlantFont\" <cmr10.pfb",
        b"=zz43 Y \"0xE SlantFont\" <cmr10.pfb",
        b"=zz44 Y \"1e400 SlantFont\" <cmr10.pfb",
        b"=zz45 Y \"1e-400 SlantFont\" <cmr10.pfb",
        b"=zz46 Y \"0xESlantFont\" <cmr10.pfb",
        b"=zz47 Y \"nax SlantFont\" <cmr10.pfb",
        b"=zz48 Y \"inx SlantFont\" <cmr10.pfb",
        b"=zz49 Y \"5",
        b"=zz50 Y \"-",
        b"=zz51 Y \"",
        b"=zz52 Y \"0x1p-3",
    ]
    jobs = []
    good = {11, 12, 13, 14, 16, 17, 18, 19, 20}
    with open(kpse("cmr10.tfm"), "rb") as f:
        cmr = f.read()
    for i, line in enumerate(bad):
        tfm = line.split()[0][1:].decode() if line[1:2] != b"<" else "zz"
        if i in good:
            jobs.append(Job("line_%02d" % i, [Font(tfm, [65, 66], line, "line", [(tfm + ".tfm", cmr)])]))
        else:
            jobs.append(Job("line_%02d" % i, [Font(tfm, [], line, "line")]))
    # Values through the float arithmetic, visible as -Slant_N and -Extend_N in the font name.
    values = [".167", ".2", "-.25", ".3333", "-1", "1", ".0005", "1.0005", ".1234567", ".9999", "1e-1", "0x1p-3",
              ".5", ".6", "-.6", ".75", "+.5", "0x.8p0", "0x1.8p-2", "0x10p-8", "0X1P-3", "1E-1", ".5e0", "5e-1"]
    for i, v in enumerate(values):
        line = b'=cmr10 CMR10 "%s SlantFont" <cmr10.pfb' % v.encode()
        jobs.append(Job("slantval_%02d" % i, [Font("cmr10", [65], line, "line")]))
    values = [".8", "1.1", ".85", "1.0005", "2", ".5", "1.001", "0.999", "1.2345678"]
    for i, v in enumerate(values):
        line = b'=cmr10 CMR10 "%sExtendFont" <cmr10.pfb' % v.encode()
        jobs.append(Job("extendval_%02d" % i, [Font("cmr10", [65], line, "line")]))
    # No font file.
    jobs.append(Job("builtin_std", [Font("cmr10", [65], b"=cmr10 Times-Roman", "line")]))
    jobs.append(Job("builtin_other", [Font("cmr10", [65], b"=cmr10 Foo", "line")]))
    jobs.append(Job("builtin_flags", [Font("cmr10", [65], b"=cmr10 Helvetica-BoldOblique 96", "line")]))
    jobs.append(Job("builtin_courier", [Font("cmr10", [65, 66], b'=cmr10 Courier "TeXBase1Encoding ReEncodeFont" <8r.enc', "line")]))
    return jobs


def ttf_jobs():
    jobs = []
    ttfs = [p for p in (kpse("marvosym.ttf"), kpse("emo-lingchi.ttf"), kpse("Redacted-Regular.ttf")) if p]
    enc_uni = b"/UniEnc [ " + b" ".join(b"/uni%04X" % (0x20 + i) for i in range(128)) + b" ] def\n"
    enc_idx = b"/IdxEnc [ " + b" ".join(b"/index%d" % (2 + i) for i in range(128)) + b" ] def\n"
    enc_mix = b"/MixEnc [ /A /B /uni0041 /index5 /nosuchglyph /a /.notdef /index99999 /space ] def\n"
    sfd = b"# a subfont definition\n01 0x20_0x7e\n02 0x41 0x42 \\\n 0xA0_0xFF\n"
    letters = list(range(33, 127))
    for path in ttfs:
        base = os.path.basename(path)
        stem = base.split(".")[0].replace("-", "")
        jobs.append(Job("ttf_whole_" + stem, [Font("cmr10", [65, 66], b"=cmr10 <<" + base.encode(), "line")]))
        jobs.append(Job("ttf_8r_" + stem, [Font("cmr10", letters, b"=cmr10 <8r.enc <" + base.encode(), "line")]))
        jobs.append(Job("ttf_8r_few_" + stem, [Font("cmr10", [65], b"=cmr10 <8r.enc <" + base.encode(), "line")]))
        jobs.append(Job("ttf_uni_" + stem, [Font("cmr10", letters, b"=cmr10 <uni.enc <" + base.encode(), "line", [("uni.enc", enc_uni)])]))
        jobs.append(Job("ttf_idx_" + stem, [Font("cmr10", list(range(0, 100)), b"=cmr10 <idx.enc <" + base.encode(), "line", [("idx.enc", enc_idx)])]))
        jobs.append(Job("ttf_mix_" + stem, [Font("cmr10", list(range(0, 9)), b"=cmr10 <mix.enc <" + base.encode(), "line", [("mix.enc", enc_mix)])]))
        jobs.append(Job("ttf_nosubset_enc_" + stem, [Font("zzbad", [], b"=zzbad <8r.enc <<" + base.encode(), "line")]))
        jobs.append(Job("ttf_subset_noenc_" + stem, [Font("cmr10", [65], b"=cmr10 <" + base.encode(), "line")]))
        with open(kpse("cmr10.tfm"), "rb") as f:
            cmr = f.read()
        files = [("sft.sfd", sfd), ("sub01.tfm", cmr), ("sub02.tfm", cmr)]
        jobs.append(Job("ttf_subfont_" + stem, [
            Font("sub01", [65, 66, 70], b"+sub@sft@ <" + base.encode(), "line", files),
            Font("sub02", [0, 1, 2], b"+sub@sft@ <" + base.encode(), "line"),
        ]))
    # .sfd files in odd shapes, and in error.
    sfds = [
        ("offsets", b"01 5: 0x41 0x42 100: 0x43\n"),
        ("octal", b"01 0101 0102 0x43\n"),
        ("comment_last", b"01 0x41_0x43\n# the end"),
        ("no_final_nl", b"01 0x41_0x43"),
        ("cont", b"01 0x41 \\\n0x42 \\\n0x43\n"),
        ("bad_token", b"01 foo\n"),
        ("bad_range", b"01 0x43_0x41\n"),
        ("big_range", b"01 0_0x200\n"),
        ("bad_range_token", b"01 1_x\n"),
        ("long_line", b"01 " + b"0x41 " * 80 + b"\n"),
        ("empty", b""),
    ]
    if ttfs:
        with open(kpse("cmr10.tfm"), "rb") as f:
            cmr = f.read()
        base = os.path.basename(ttfs[0]).encode()
        for name, data in sfds:
            # An empty file makes no entry for sub01, which would then be a PK font: not used here.
            codes = [] if name == "empty" else [0, 1, 2]
            jobs.append(Job("sfd_" + name, [Font(
                "sub01", codes, b"+sub@sft@ <" + base, "line", [("sft.sfd", data), ("sub01.tfm", cmr)],
            )]))
    for name in ("lmroman10-regular.otf", "lmmono10-regular.otf"):
        if kpse(name):
            stem = name.split(".")[0].replace("-", "")
            jobs.append(Job("otf_" + stem, [Font("cmr10", [65], b"=cmr10 <<" + name.encode(), "line")]))
            jobs.append(Job("otf_enc_" + stem, [Font("cmr10", [65, 97], b"=cmr10 <8r.enc <<" + name.encode(), "line")]))
            jobs.append(Job("otf_subset_" + stem, [Font("cmr10", [65], b"=cmr10 <" + name.encode(), "line")]))
    return jobs


def build_jobs(rng, limit=None):
    entries = read_map()
    jobs = []
    fonts = []
    names = sorted(entries)

    # A TFM that is not installed is replaced, in the job's directory, by a copy of one with 256 characters:
    # the map line and the font program are what is under test, and they do not look at the metrics.
    surrogate_path = kpse("ec-lmr10.tfm")
    with open(surrogate_path, "rb") as f:
        surrogate = f.read()
    surrogate_codes = tfm_codes(surrogate_path)

    def tfm_info(tfm):
        ff = re.search(rb"([^\s<\[\"]+\.(?:pfb|pfa|ttf|ttc|otf))", entries[tfm], re.I)
        if ff and not kpse(ff.group(1).decode("latin-1")):
            return tfm, None, None  # the map names a font program that is not installed
        en = re.search(rb"([^\s<\[\"]+\.enc)", entries[tfm], re.I)
        if en and not kpse(en.group(1).decode("latin-1")):
            return tfm, None, None  # or an encoding
        p = kpse(tfm + ".tfm")
        if p:
            return tfm, tfm_codes(p), None
        return tfm, surrogate_codes, surrogate

    with concurrent.futures.ThreadPoolExecutor(8) as ex:
        info = list(ex.map(tfm_info, names))
    no_program = 0
    no_chars = 0
    borrowed = 0
    for tfm, codes, tfm_file in info:
        if codes is None:
            no_program += 1
            continue
        if not codes:
            no_chars += 1
            continue
        if tfm_file is not None:
            borrowed += 1
        fonts.append((tfm, codes, entries[tfm], tfm_file))
    print(
        "map entries %d: usable %d (of which %d with a borrowed TFM), font program or encoding not installed %d, TFM without chars %d"
        % (len(names), len(fonts), borrowed, no_program, no_chars)
    )
    rng.shuffle(fonts)
    if limit:
        fonts = fonts[:limit]

    def tfm_files(tfm, tfm_file):
        return [(tfm + ".tfm", tfm_file)] if tfm_file else ()

    for tfm, codes, raw, tf in fonts:
        safe = re.sub(r"\W", "_", tfm)
        files = tfm_files(tfm, tf)
        jobs.append(Job("map_all_" + safe, [Font(tfm, codes, raw, "file", files)]))
        jobs.append(Job("map_few_" + safe, [Font(tfm, sorted(rng.sample(codes, min(12, len(codes)))), raw, "file", files)]))
        jobs.append(Job("map_one_" + safe, [Font(tfm, [rng.choice(codes)], raw, "file", files)]))

    t1 = [f for f in fonts if b".pfb" in f[2]]
    sample = t1[:: max(1, len(t1) // 250)]

    def strip_line(raw):
        return raw.strip()

    for tfm, codes, raw, tf in sample:
        safe = re.sub(r"\W", "_", tfm)
        few = sorted(rng.sample(codes, min(40, len(codes))))
        files = tfm_files(tfm, tf)
        line = strip_line(raw)
        nosub = re.sub(rb"<(\S+\.pfb)", rb"<<\1", line)
        jobs.append(Job("nosub_" + safe, [Font(tfm, few, b"=" + nosub, "line", files)]))
        specials = rng.choice([b".167 SlantFont", b"-.1 SlantFont", b".85 ExtendFont", b"1.2 ExtendFont",
                               b".21 SlantFont .9 ExtendFont", b"1.0005 ExtendFont", b".3333 SlantFont"])
        if re.search(rb'"[^"]*"', line):
            sl = re.sub(rb'"[^"]*"', b'"' + specials + b' X ReEncodeFont"', line, count=1)
        else:
            sl = re.sub(rb"(^\S+ \S+) ", rb'\1 "' + specials + rb'" ', line, count=1)
        jobs.append(Job("slant_" + safe, [Font(tfm, few, b"=" + sl, "line", files)]))
        if b".enc" not in line:
            enc = rng.choice([b"8r.enc", b"texnansi.enc", b"lm-ec.enc", b"cm-super-t1.enc", b"ec.enc"])
            enc = enc if kpse(enc.decode()) else b"8r.enc"
            re_line = re.sub(rb"<(\S+\.pfb)", b"<" + enc + rb" <\1", line)
            jobs.append(Job("reenc_" + safe, [Font(tfm, few, b"=" + re_line, "line", files)]))
        fl = re.sub(rb"^(\S+ \S+) ", rb"\1 34 ", line, count=1)
        jobs.append(Job("flags_" + safe, [Font(tfm, few, b"=" + fl, "line", files)]))
        jobs.append(Job("tou_" + safe, [Font(tfm, few, raw, "file", files)], tounicode=True))

    # Accented characters alone, so that seac has to pull in a base and an accent nothing else asked for.
    for tfm, codes, raw, tf in t1[:: max(1, len(t1) // 400)]:
        if b".enc" not in raw:
            continue
        high = [c for c in codes if c >= 0xC0][:6]
        if high:
            safe = re.sub(r"\W", "_", tfm)
            jobs.append(Job("seac_" + safe, [Font(tfm, high, raw, "file", tfm_files(tfm, tf))]))

    # PFB to PFA: a sample, with two line widths.
    for k, (tfm, codes, raw, tf) in enumerate(sample[:60]):
        m = re.search(rb"<(\S+\.pfb)", raw)
        path = kpse(m.group(1).decode())
        if not path:
            continue
        with open(path, "rb") as f:
            pfa = pfb_to_pfa(f.read(), 64 if k % 2 else 32, b"\r\n" if k % 3 == 0 else b"\n", k % 5 == 0)
        fn = m.group(1).decode()[:-4] + ".pfa"
        line = re.sub(rb"<\S+\.pfb", b"<" + fn.encode(), raw.strip())
        safe = re.sub(r"\W", "_", tfm)
        few = sorted(rng.sample(codes, min(30, len(codes))))
        jobs.append(Job("pfa_" + safe, [Font(tfm, few, b"=" + line, "line", [(fn, pfa)] + list(tfm_files(tfm, tf)))]))

    # Several fonts in a run.
    by_pfb = {}
    for f in t1:
        m = re.search(rb"<(\S+\.pfb)", f[2])
        by_pfb.setdefault(m.group(1), []).append(f)
    shared = [v for v in by_pfb.values() if len(v) >= 2]
    rng.shuffle(shared)
    for k, group in enumerate(shared[:80]):
        chosen = group[:4]
        jobs.append(Job("multi_shared_%03d" % k, [
            Font(t, sorted(rng.sample(c, min(25, len(c)))), raw, "file", tfm_files(t, tf)) for t, c, raw, tf in chosen
        ]))
    for k in range(80):
        chosen = rng.sample(t1, min(6, len(t1)))
        jobs.append(Job("multi_mixed_%03d" % k, [
            Font(t, sorted(rng.sample(c, min(25, len(c)))), raw, "file", tfm_files(t, tf)) for t, c, raw, tf in chosen
        ]))

    jobs += line_jobs()
    jobs += ttf_jobs()
    jobs += synth_ttf_jobs()
    jobs += synth_jobs()
    jobs += tou_jobs()
    return jobs, fonts


def write_gtu():
    src = kpse("glyphtounicode.tex")
    with open(src, "rb") as f:
        text = f.read()
    extra = (
        b"\\pdfglyphtounicode{tfm:cmr10/Gamma}{0393 0393}\n"
        b"\\pdfglyphtounicode{a}{0061 0301}\n"
        b"\\pdfglyphtounicode{B}{110000}\n"
        b"\\pdfglyphtounicode{C}{00C7}\n"
        b"\\pdfglyphtounicode{D}{ 0044 }\n"
        b"\\pdfglyphtounicode{E}{zz}\n"
        b"\\pdfglyphtounicode{}{0041}\n"
        b"\\pdfglyphtounicode{.notdef}{0041}\n"
        b"\\pdfglyphtounicode{F}{ }\n"
        b"\\pdfglyphtounicode{G}{0047 0048  0049}\n"
    )
    text += extra
    with open(os.path.join(OUT, "gtu.tex"), "wb") as f:
        f.write(text)
    pairs = re.findall(rb"^\\pdfglyphtounicode\{([^}]*)\}\{([^}]*)\}", text, re.M)
    with open(os.path.join(OUT, "gtu.tsv"), "wb") as f:
        for a, b in pairs:
            f.write(a + b"\t" + b + b"\n")


def main():
    args = sys.argv[1:]
    limit = int(args[args.index("--limit") + 1]) if "--limit" in args else None
    workers = int(args[args.index("--workers") + 1]) if "--workers" in args else 6
    only = args[args.index("--only") + 1] if "--only" in args else None
    keep = "--keep" in args
    if os.path.isdir(WORK):
        shutil.rmtree(WORK)
    os.makedirs(WORK)
    rng = random.Random(20261102)
    write_gtu()
    jobs, fonts = build_jobs(rng, limit)
    if only:
        jobs = [j for j in jobs if only in j.id]
    print("jobs:", len(jobs))
    manifest = os.path.join(OUT, "manifest.tsv")
    dirs = {}
    with open(manifest, "w", encoding="latin-1") as m, concurrent.futures.ThreadPoolExecutor(workers) as ex:
        for job, (text, d) in zip(jobs, ex.map(run_job, jobs)):
            m.write(text)
            dirs[job.id] = d
    print("oracle done, comparing")
    subprocess.run(
        ["cargo", "build", "--release", "-j4", "-p", "rstex-fontfile", "--example", "fontfile_gate"],
        cwd=ROOT, check=True,
    )
    exe = os.path.join(TARGET, "release", "examples", "fontfile_gate")
    r = subprocess.run([exe, manifest, os.path.join(OUT, "gtu.tsv")], capture_output=True, text=True, errors="replace")
    with open(os.path.join(OUT, "report.txt"), "w") as f:
        f.write(r.stdout)
    fails = [ln for ln in r.stdout.split("\n") if ln.startswith("FAIL")]
    passes = [ln for ln in r.stdout.split("\n") if ln.startswith("PASS")]
    for ln in passes:
        if " SKIP " in ln:
            print(ln[:300])
    print("fatal errors reproduced:", sum(1 for ln in passes if "fatal error" in ln))
    summary = {}
    for ln in passes + fails:
        family = ln.split()[1].split("_")[0]
        s = summary.setdefault(family, [0, 0])
        s[0 if ln.startswith("PASS") else 1] += 1
    for family in sorted(summary):
        print("%-10s pass %5d  fail %5d" % (family, summary[family][0], summary[family][1]))
    print(r.stdout.strip().split("\n")[-1])
    if r.returncode != 0:
        print("gate tool exit", r.returncode, r.stderr[-2000:])
    for ln in fails[:40]:
        print(ln[:600])
    if fails:
        print("... %d failures, all in %s" % (len(fails), os.path.join(OUT, "report.txt")))
    if not keep:
        failed = {ln.split()[1] for ln in fails}
        for jid, d in dirs.items():
            if jid not in failed:
                shutil.rmtree(d, ignore_errors=True)
    sys.exit(1 if fails or r.returncode != 0 else 0)


if __name__ == "__main__":
    main()
