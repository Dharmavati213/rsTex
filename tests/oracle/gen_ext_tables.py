#!/usr/bin/env python3
"""Expected results for ext.rs's unit test of scale_image, taken from the oracle.

Run with python3 -I.  It writes crates/engine/testdata/ext_scale_oracle.tsv.

Each row is one \\pdfximage of a synthetic JBIG2 file, which is the only image format that can state any size and
resolution in a few bytes.  The file exists only in a temporary directory.  The columns are

    x y xr yr default_res wd ht dp  wd' ht' dp'

x and y are the image's size, xr and yr its resolution as scale_image's second half gets it (0 if the first half said
"too small" or "too large"), default_res is \\pdfimageresolution, wd ht dp the rule spec with -1073741824 for "running",
and wd' ht' dp' what \\number\\wd0 and so on print afterwards, in sp, for an hbox that holds the image: so a negative
height or depth reads as 0.  These can pass 2^31, since the oracle's scaled has 64 bits; the test passes over those, as the port
follows the official pdfTeX there: tests/integration/image/wrap_scale.
"""
import os
import re
import struct
import subprocess
import tempfile

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
OUT = os.path.join(ROOT, "crates", "engine", "testdata", "ext_scale_oracle.tsv")

RUNNING = -1073741824
PT = 65536
ONE_INCH = 226.0


def jbig2(w, h, xres=0, yres=0):
    """A file with one page: the page information segment, the end of the page and the end of the file."""
    s = b"\x97JB2\r\n\x1a\n" + b"\x01" + struct.pack(">I", 1)
    d = struct.pack(">IIIIBH", w, h, xres, yres, 0, 0)
    s += struct.pack(">IBBBI", 0, 48, 0, 1, len(d)) + d
    s += struct.pack(">IBBBI", 1, 49, 0, 1, 0)
    s += struct.pack(">IBBBI", 2, 51, 0, 0, 0)
    return s


def ppm(res):
    """Pixels per metre that read_jbig2_info turns back into `res`."""
    return int(res / 0.0254 + 0.5)


def rows():
    out = []
    sizes = [(1, 1), (4, 4), (100, 200), (640, 480), (1000, 1000), (5000, 3), (3, 5000), (65535, 65535), (100000, 1)]
    for x, y in sizes:
        for res in (0, 72, 96, 300, 600, 1200, 65535):
            for dflt in (0, 72, 150, 65535):
                out.append((x, y, res, res, dflt, RUNNING, RUNNING, RUNNING))
    # Different resolutions, and one of them zero: default_res takes over, or 72 does.
    out += [
        (1000, 1000, 300, 600, 0, RUNNING, RUNNING, RUNNING),
        (1000, 1000, 600, 300, 150, RUNNING, RUNNING, RUNNING),
        (1000, 1000, 300, 0, 0, RUNNING, RUNNING, RUNNING),
        (1000, 1000, 0, 300, 0, RUNNING, RUNNING, RUNNING),
        (1000, 1000, 300, 0, 150, RUNNING, RUNNING, RUNNING),
    ]
    # Every combination of a given rule spec, with a size that has an inexact ratio.
    for x, y in [(640, 480), (3, 7), (7, 3), (1000000, 1)]:
        for wd in (RUNNING, 100 * PT):
            for ht in (RUNNING, 50 * PT, 0):
                for dp in (RUNNING, 10 * PT, 0):
                    out.append((x, y, 96, 96, 0, wd, ht, dp))
                    out.append((x, y, 0, 0, 0, wd, ht, dp))
    # Past 2^31, in the image and in the result.
    big = [(0x7FFFFFFF, 10), (0x80000001, 10), (0xFFFFFFFF, 10), (10, 0xFFFFFFFF), (0xFFFFFFFF, 0xFFFFFFFF)]
    for x, y in big:
        out.append((x, y, 0, 0, 0, RUNNING, RUNNING, RUNNING))
        out.append((x, y, 0, 0, 0, 16000 * PT, RUNNING, RUNNING))
        out.append((x, y, 0, 0, 0, RUNNING, 16000 * PT, RUNNING))
        out.append((x, y, 0, 0, 0, RUNNING, RUNNING, 16000 * PT))
        out.append((x, y, 0, 0, 0, RUNNING, 1, RUNNING))
        out.append((x, y, 0, 0, 0, 1, RUNNING, 1))
    return out


def rule_spec(v):
    wd, ht, dp = v
    s = ""
    if wd != RUNNING:
        s += " width %dsp" % wd
    if ht != RUNNING:
        s += " height %dsp" % ht
    if dp != RUNNING:
        s += " depth %dsp" % dp
    return s


def main():
    table = rows()
    with tempfile.TemporaryDirectory() as d:
        doc = [
            r"\catcode`\{=1 \catcode`\}=2 \catcode`\#=6",
            r"\pdfoutput=1 \batchmode",
        ]
        for k, (x, y, xr, yr, dflt, wd, ht, dp) in enumerate(table):
            with open(os.path.join(d, "i%d.jb2" % k), "wb") as f:
                f.write(jbig2(x, y, ppm(xr), ppm(yr)))
            doc.append(r"\pdfimageresolution=%d" % dflt)
            doc.append(r"\immediate\write16{CASE %d}" % k)
            doc.append(r"\pdfximage%s {i%d.jb2}" % (rule_spec((wd, ht, dp)), k))
            doc.append(r"\setbox0\hbox{\pdfrefximage\pdflastximage}")
            doc.append(r"\immediate\write16{RESULT %d \number\wd0,\number\ht0,\number\dp0}" % k)
        doc.append(r"\end")
        with open(os.path.join(d, "t.tex"), "w") as f:
            f.write("\n".join(doc) + "\n")
        subprocess.run(["pdftex", "-ini", "-interaction=batchmode", "t.tex"], cwd=d, stdout=subprocess.DEVNULL)
        log = open(os.path.join(d, "t.log"), "rb").read().decode("latin-1")

    # The first half of scale_image decides on the resolutions; the log says what it decided.
    lines = []
    parts = re.split(r"CASE (\d+)\n", log)
    for i in range(1, len(parts), 2):
        k = int(parts[i])
        body = parts[i + 1]
        m = re.search(r"RESULT %d (-?\d+),(-?\d+),(-?\d+)" % k, body)
        assert m, "case %d did not finish: %s" % (k, body[:200])
        x, y, xr, yr, dflt, wd, ht, dp = table[k]
        said = "image resolution ignored" in body
        computes_small = (xr != 0 or yr != 0) and (x / ONE_INCH >= xr or y / ONE_INCH >= yr)
        assert said == computes_small, "case %d: %s" % (k, body[:200])
        if said:
            xr = yr = 0
        lines.append("\t".join(str(v) for v in (x, y, xr, yr, dflt, wd, ht, dp, m.group(1), m.group(2), m.group(3))))
    assert len(lines) == len(table)
    with open(OUT, "w") as f:
        f.write("# gen_ext_tables.py: x y xr yr default_res wd ht dp, then wd ht dp in sp\n")
        f.write("\n".join(lines) + "\n")
    print("%d rows" % len(lines))


if __name__ == "__main__":
    main()
