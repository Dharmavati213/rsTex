#!/usr/bin/env python3
"""Expected results for crates/pdfio's tests, taken from the oracle.  Run with python3 -I.

  python3 -I tests/oracle/gen_pdfio_tables.py          writes crates/pdfio/testdata/
  python3 -I tests/oracle/gen_pdfio_tables.py --check  compares crates/pdfio with system zlib and pdfTeX, both ways

It writes three small tables.  An input is described, not stored, wherever it is long:

  md5_oracle.tsv      `SPEC DIGEST`: \\pdfmdfivesum{...} by pdftex -ini, on text given as tokens, or
                      `file SEED LENGTH DIGEST`: \\pdfmdfivesum file{...}, on files made under a temporary directory
  inflate_oracle.tsv  `LEVEL SPEC ZLIB-HEX`: what system zlib makes, through Python's zlib module.  Only streams
                      that come out short: long ones are for --check, and are not committed.
  numfmt_g.tsv        `BITS TEXT`: an IEEE double in hex, and what C's %g prints for it.  The C is awk's printf
                      through libc, and it is checked against Python's %g on the same values.

A SPEC is two or three tab-separated fields, as in crates/pdfio/tests/pdfio.rs:

  hex HEX             these bytes
  repeat BYTE COUNT   a byte, decimal, COUNT times
  range FROM TO       the byte values FROM to TO, decimal, inclusive
  pattern SEED COUNT  COUNT bytes of a reproducible sequence holding every byte value

--check runs the example pdfio_tool.  System zlib inflates what crates/pdfio deflates, at every level and in pieces
of several sizes, and a second run gives the same bytes.  crates/pdfio inflates what system zlib makes at every level, strategy, window size and flush
mode, and the streams of a PDF that pdfTeX wrote.
"""
import json
import os
import random
import re
import struct
import subprocess
import sys
import tempfile
import zlib

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
OUT = os.path.join(ROOT, "crates", "pdfio", "testdata")


def pattern(seed, n):
    """pattern() in crates/pdfio/tests/pdfio.rs."""
    x = seed
    out = bytearray(n)
    for i in range(n):
        x = (x * 1664525 + 1013904223) & 0xFFFFFFFF
        out[i] = x >> 24
    return bytes(out)


def expand_spec(spec):
    kind = spec[0]
    if kind == "hex":
        return bytes.fromhex(spec[1])
    if kind == "repeat":
        return bytes([int(spec[1])]) * int(spec[2])
    if kind == "range":
        return bytes(range(int(spec[1]), int(spec[2]) + 1))
    if kind == "pattern":
        return pattern(int(spec[1]), int(spec[2]))
    raise ValueError(kind)


# One line of a page stream, the kind of data \pdfcompresslevel compresses most.
PDF_STREAM = b"BT /F1 9.9626 Tf 72 720 Td (Hello) Tj ET\n"


def h(b):
    return ("hex", b.hex())


# Text to hand to \pdfmdfivesum as tokens.  The catcodes below make all these plain characters.
MD5_STRINGS = [
    h(b""), h(b"a"), h(b"abc"), h(b"message digest"), h(b"abcdefghijklmnopqrstuvwxyz"),
    h(b"a b"), h(b"a  b"), h(b".,;:!?()[]<>=+-*/'`\"@&~_|$#"),
    h(b"\x80"), h(b"\xff"), h(b"\xc3\xa9"),
    ("range", "128", "255"), ("range", "33", "91"), ("range", "95", "122"), ("range", "48", "57"),
] + [("repeat", "120", str(n)) for n in (54, 55, 56, 57, 63, 64, 65, 119, 120, 121, 127, 128, 129, 200, 1000)]

MD5_FILES = [(1, n) for n in (0, 1, 2, 3, 54, 55, 56, 57, 63, 64, 65, 119, 120, 121, 127, 128, 129, 1023, 1024, 1025, 4096, 65536)]
MD5_FILES += [(7, n) for n in (10, 100, 1000, 10000, 100000)]


def tex_string(b):
    """A byte string in ^^ notation."""
    return "".join("^^%02x" % c for c in b)


def gen_md5():
    with tempfile.TemporaryDirectory() as d:
        lines = [r"\catcode`\{=1 \catcode`\}=2 \catcode`\^=7 \catcode`\#=12 \catcode`\%=12 \catcode`\$=12 \catcode`\&=12 \catcode`\~=12 \catcode`\_=12 \scrollmode"]
        for c in range(128, 256):
            lines.append(r"\catcode%d=12" % c)
        for i, spec in enumerate(MD5_STRINGS):
            # Two spaces in a row would collapse in the tokenizer, so a space goes in as a catcode 12 character.
            lines.append(r"\catcode32=12 \immediate\write16{MD5S%d:\pdfmdfivesum{%s}}\catcode32=10" % (i, tex_string(expand_spec(spec))))
        for i, (seed, n) in enumerate(MD5_FILES):
            name = "f%d.bin" % i
            with open(os.path.join(d, name), "wb") as f:
                f.write(pattern(seed, n))
            lines.append(r"\immediate\write16{MD5F%d:\pdfmdfivesum file{%s}}" % (i, name))
        lines.append(r"\end")
        with open(os.path.join(d, "t.tex"), "w") as f:
            f.write("\n".join(lines) + "\n")
        env = dict(os.environ, HOME=d, TEXMFOUTPUT=d, MKTEXTFM="0", MKTEXPK="0", MKTEXFMT="0")
        subprocess.run(["pdftex", "-ini", "-etex", "-interaction=batchmode", "t.tex"], cwd=d, capture_output=True, env=env, timeout=120)
        log = open(os.path.join(d, "t.log"), "rb").read().decode("latin-1").replace("\n", "")
    found = dict(re.findall(r"(MD5[SF]\d+):([0-9A-F]{32})", log))
    out = []
    for i, spec in enumerate(MD5_STRINGS):
        dg = found.get("MD5S%d" % i)
        if not dg:
            raise SystemExit("no digest for string %d (%r); log: %s" % (i, spec, log[:2000]))
        out.append("\t".join(spec) + "\t" + dg + "\n")
    for i, (seed, n) in enumerate(MD5_FILES):
        dg = found.get("MD5F%d" % i)
        if not dg:
            raise SystemExit("no digest for file %d" % i)
        out.append("file\t%d\t%d\t%s\n" % (seed, n, dg))
    return out


def gen_inflate():
    specs = [h(b""), h(b"a"), h(b"abc"), h(PDF_STREAM), ("repeat", "97", "100"), ("repeat", "97", "40000"), ("repeat", "0", "5000"),
             ("range", "0", "20"), ("pattern", "3", "30")]
    out = []
    for level in (0, 1, 6, 9):
        for spec in specs:
            z = zlib.compress(expand_spec(spec), level)
            # A longer run of hex is what tools/privacy_guard.py rightly refuses.
            if len(z) <= 64:
                out.append("%d\t%s\t%s\n" % (level, "\t".join(spec), z.hex()))
    # Another window and strategy: valid streams that other writers make.
    for wbits in (9, 12):
        for strategy in (zlib.Z_DEFAULT_STRATEGY, zlib.Z_FILTERED, zlib.Z_HUFFMAN_ONLY, zlib.Z_RLE, zlib.Z_FIXED):
            c = zlib.compressobj(6, zlib.DEFLATED, wbits, 8, strategy)
            spec = ("repeat", "98", "50")
            z = c.compress(expand_spec(spec)) + c.flush()
            if len(z) <= 64:
                out.append("6\t%s\t%s\n" % ("\t".join(spec), z.hex()))
    return out


def gen_g():
    rng = random.Random(20261102)
    values = [0.0, -0.0, 1.0, -1.0, 0.001, 1000.0, 999999.0, 999999.5, 1000000.0, 9999995.0, 0.0001, 0.00001, 0.000099999949, 0.00009999995, 1e100, 1e-100, 1.5, 2.5, 0.5, 100000.0, 123456.0, 1234567.0, 12345.65, 12345.75, 5e-324, 1.7976931348623157e308, 0.3, 0.1, 16.5, -16.5, 833.0, 0.833, 3.14159265, -9.46, 1e15, 1e16]
    for _ in range(500):
        values.append(rng.uniform(-2000, 2000))
    for _ in range(200):
        values.append(rng.uniform(-1, 1) * 10.0 ** rng.randint(-12, 12))
    for _ in range(200):
        # What a font holds: floats, widened.
        values.append(struct.unpack("f", struct.pack("f", rng.uniform(-1, 1) * 10.0 ** rng.randint(-4, 4)))[0])
    for _ in range(200):
        # In the middle of two six-digit decimals, where the rounding shows.
        d = rng.randint(100000, 999999)
        values.append((d + 0.5) * 10.0 ** rng.randint(-10, 3))
    for _ in range(100):
        values.append(struct.unpack("d", struct.pack("Q", rng.getrandbits(64)))[0])
    values = [v for v in values if v == v and abs(v) != float("inf")]
    # glibc through awk, given all the digits of each value.
    text = "\n".join("%.17g" % v for v in values) + "\n"
    p = subprocess.run(["awk", '{printf "%g\\n", $1}'], input=text.encode(), capture_output=True, timeout=120)
    awk = p.stdout.decode().split("\n")
    out = []
    disagree = 0
    for v, a in zip(values, awk):
        py = "%g" % v
        if a != py:
            disagree += 1
            print("awk %s, Python %s, for %r" % (a, py, v), file=sys.stderr)
        bits = struct.unpack("Q", struct.pack("d", v))[0]
        out.append("%016x\t%s\n" % (bits, py))
    print("numfmt_g: %d values; awk and Python differ on %d" % (len(values), disagree))
    return out


def build_tool():
    p = subprocess.run(["cargo", "build", "-p", "rstex-pdfio", "--example", "pdfio_tool", "-j4", "--message-format=json"], cwd=ROOT, capture_output=True, env=dict(os.environ))
    if p.returncode != 0:
        sys.stderr.write(p.stderr.decode())
        raise SystemExit(2)
    for ln in p.stdout.decode().splitlines():
        m = json.loads(ln)
        if m.get("reason") == "compiler-artifact" and m.get("executable") and m["target"]["name"] == "pdfio_tool":
            return m["executable"]
    raise SystemExit("no pdfio_tool")


def check_deflate(exe):
    samples = [b"", b"a", b"abc", b"a" * 100000, bytes(range(256)) * 400, pattern(5, 200000),
               PDF_STREAM * 5000, pattern(6, 70000) + b"x" * 70000 + pattern(7, 70000)]
    reqs = [(level, piece, d) for d in samples for level in range(0, 10) for piece in (0, 1000, 16384, 65536)]
    text = "".join("D %d %d %s\n" % (l, pc, d.hex()) for l, pc, d in reqs)
    r = subprocess.run([exe], input=text.encode(), capture_output=True, timeout=600)
    if r.returncode != 0:
        raise SystemExit(r.stderr.decode())
    again = subprocess.run([exe], input=text.encode(), capture_output=True, timeout=600)
    reproducible = again.stdout == r.stdout
    lines = r.stdout.decode().split("\n")[:-1]
    ok = same = pieces = 0
    whole = {}
    by_level = {level: [0, 0] for level in range(0, 10)}
    for (level, piece, d), ln in zip(reqs, lines):
        z = bytes.fromhex(ln)
        try:
            good = zlib.decompress(z) == d
        except zlib.error as e:
            print("system zlib refuses level %d, pieces of %d: %s" % (level, piece, e))
            good = False
        ok += good
        if piece == 0:
            whole[(level, d)] = z
        else:
            pieces += 1
            same += whole[(level, d)] == z
            by_level[level][0] += whole[(level, d)] == z
            by_level[level][1] += 1
    print("deflate: system zlib inflated %d/%d streams from crates/pdfio correctly" % (ok, len(reqs)))
    # Not a requirement: see crates/pdfio/src/lib.rs.  System zlib gives the same bytes either way.
    print("deflate: written in pieces, same bytes as in one call: %d/%d" % (same, pieces))
    print("deflate:   by level: " + ", ".join("%d: %d/%d" % (k, v[0], v[1]) for k, v in by_level.items()))
    print("deflate: a second run gave the same bytes: %s" % reproducible)
    return ok == len(reqs) and reproducible


def zlib_streams():
    """(what it is, plain, zlib stream), from system zlib, in every way it can be told to work."""
    samples = [b"", b"a", b"abcabcabd" * 50, b"a" * 100000, bytes(range(256)) * 100, pattern(8, 100000),
               PDF_STREAM * 2000]
    for i, d in enumerate(samples):
        for level in range(0, 10):
            yield ("level %d, sample %d" % (level, i), d, zlib.compress(d, level))
        for wbits in range(9, 16):
            for strategy in (zlib.Z_DEFAULT_STRATEGY, zlib.Z_FILTERED, zlib.Z_HUFFMAN_ONLY, zlib.Z_RLE, zlib.Z_FIXED):
                for mem in (1, 8, 9):
                    c = zlib.compressobj(6, zlib.DEFLATED, wbits, mem, strategy)
                    yield ("window %d, strategy %d, memlevel %d, sample %d" % (wbits, strategy, mem, i), d, c.compress(d) + c.flush())
        for flush in (zlib.Z_SYNC_FLUSH, zlib.Z_FULL_FLUSH, zlib.Z_BLOCK, zlib.Z_PARTIAL_FLUSH):
            c = zlib.compressobj(6)
            z = b""
            for k in range(0, len(d), 5000):
                z += c.compress(d[k : k + 5000]) + c.flush(flush)
            yield ("flush %d, sample %d" % (flush, i), d, z + c.flush())


def pdftex_streams():
    """(what it is, plain, zlib stream), from a PDF pdfTeX writes with a font embedded, at its usual level."""
    tex = r"""\pdfoutput=1 \pdfcompresslevel=9 \pdfobjcompresslevel=2 \pdfminorversion=5
\font\f=cmr10 \f Some text in a font, for a content stream and a font file. \vfill\eject
\pdfinfo{/Title (rstex test)} \font\g=cmbx12 \g And a second page with another font. \bye
"""
    out = []
    with tempfile.TemporaryDirectory() as d:
        with open(os.path.join(d, "t.tex"), "w") as f:
            f.write(tex)
        env = dict(os.environ, MKTEXTFM="0", MKTEXPK="0", MKTEXFMT="0")
        subprocess.run(["pdftex", "-interaction=batchmode", "t.tex"], cwd=d, capture_output=True, env=env, timeout=120)
        data = open(os.path.join(d, "t.pdf"), "rb").read()
    for i, m in enumerate(re.finditer(rb"stream\r?\n", data)):
        if data[m.start() - 3 : m.start()] == b"end":
            continue
        c = zlib.decompressobj()
        try:
            plain = c.decompress(data[m.end() :])
        except zlib.error:
            continue
        if not c.eof:
            continue
        z = data[m.end() : len(data) - len(c.unused_data)]
        out.append(("pdfTeX stream %d" % i, plain, z))
    return out


def check_inflate(exe):
    streams = list(zlib_streams())
    from_pdftex = pdftex_streams()
    if len(from_pdftex) < 4:
        raise SystemExit("expected streams in pdfTeX's PDF, got %d" % len(from_pdftex))
    streams += from_pdftex
    # Each is given with something after it, as a stream in a PDF is: all of it must be told apart.
    text = "".join("I %s\n" % (z + b"\nendstream").hex() for _, _, z in streams)
    r = subprocess.run([exe], input=text.encode(), capture_output=True, timeout=600)
    if r.returncode != 0:
        raise SystemExit(r.stderr.decode())
    lines = r.stdout.decode().split("\n")[:-1]
    ok = 0
    for (what, plain, z), ln in zip(streams, lines):
        took, _, hexdata = ln.partition(" ")
        if took == str(len(z)) and bytes.fromhex(hexdata) == plain:
            ok += 1
        else:
            print("inflate differs: %s: %s" % (what, ln[:80]))
    print("inflate: crates/pdfio inflated %d/%d streams from system zlib correctly, %d of them from pdfTeX's PDF" % (ok, len(streams), len(from_pdftex)))
    return ok == len(streams)


def main():
    if "--check" in sys.argv:
        exe = build_tool()
        a = check_deflate(exe)
        b = check_inflate(exe)
        return 0 if a and b else 1
    os.makedirs(OUT, exist_ok=True)
    for name, lines in (("md5_oracle.tsv", gen_md5()), ("inflate_oracle.tsv", gen_inflate()), ("numfmt_g.tsv", gen_g())):
        with open(os.path.join(OUT, name), "w") as f:
            f.writelines(lines)
        print("%s: %d lines" % (name, len(lines)))
    return 0


if __name__ == "__main__":
    sys.exit(main())
