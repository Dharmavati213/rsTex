#!/usr/bin/env python3
"""Expected numbers for the unit tests of pdfout.rs, taken from the oracle's PDF output.

Run with python3 -I.  It writes crates/engine/testdata/pdfout_oracle.tsv, one case per line, fields separated by a tab:

  bp     D  s                 what pdf_print_bp(s) writes with \\pdfdecimaldigits=D
  font   s                    what pdf_print_real(divide_scaled(s,one_hundred_bp,6),4) writes: a font's size in sp
  magbp  mag  D  s            what pdf_print_mag_bp(s) writes under \\mag=mag
  origin D  moves  out        pdf_set_origin(h,v) in turn from (0,page height), h and v given by moves, a list of h:v;
                              out is the two numbers of each cm that came out, ; between them
  rule   D  x  y  w  h  out   pdf_set_rule(x,y,w,h): what it writes between its q and Q, words joined by one space

Each comes from a document made here, run by `pdftex -ini` in a temporary directory with \\pdfcompresslevel=0.  Only numbers
and the rule's drawing operators are kept: the documents make no text.
"""
import os
import random
import re
import subprocess
import tempfile

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
OUT = os.path.join(ROOT, "crates", "engine", "testdata", "pdfout_oracle.tsv")

HEAD = r"""\catcode`\{=1 \catcode`\}=2 \catcode`\#=6
\pdfoutput=1 \pdfcompresslevel=0 \pdfinfoomitdate=1 \pdfsuppressptexinfo=-1 \pdftrailerid{}
\pdfpagewidth=400pt \pdfpageheight=400pt \pdfhorigin=0pt \pdfvorigin=0pt \batchmode
"""
PAGE_HEIGHT = 400 * 65536


def run(body):
    with tempfile.TemporaryDirectory() as d:
        with open(os.path.join(d, "t.tex"), "w") as f:
            f.write(HEAD + body + "\n\\end\n")
        env = dict(os.environ, HOME=d, MKTEXTFM="0", MKTEXPK="0", MKTEXFMT="0", SOURCE_DATE_EPOCH="1000000000")
        subprocess.run(["pdftex", "-ini", "-interaction=batchmode", "t.tex"], cwd=d, capture_output=True, env=env, timeout=300)
        with open(os.path.join(d, "t.pdf"), "rb") as f:
            return f.read().decode("latin-1")


def page_streams(pdf):
    """The page content streams, in order.  Pages are the only streams in these documents but font files."""
    return re.findall(r"/Length \d+\s*\n>>\nstream\n(.*?)\nendstream", pdf, re.S)


rng = random.Random(1933)

BP_VALUES = sorted(set(
    [0, 1, 2, 16, 32, 33, 64, 65, 66, 67, 328, 329, 657, 658, 659, 6577, 6578, 6579, 32890, 32891, 65535, 65536, 65781, 65782, 65783]
    + [657817, 657818, 6578175, 6578176, 6578177, 13156352, 98672640, 473628672, 1073741823]
    + [rng.randrange(1, 1 << 30) for _ in range(10)]
    + [rng.randrange(1, 1 << 12) for _ in range(10)]))
BP_VALUES += [-s for s in BP_VALUES if s]


def gen_bp():
    rows = []
    for digits in range(5):
        body = [r"\pdfdecimaldigits=%d \pdfvorigin=100pt" % digits]
        for s in BP_VALUES:
            body.append(r"\shipout\hbox to 0pt{\kern%dsp\vrule width 5pt height 5pt depth 0pt\hss}" % s)
        streams = page_streams(run("\n".join(body)))
        assert len(streams) == len(BP_VALUES), (len(streams), len(BP_VALUES))
        for s, stream in zip(BP_VALUES, streams):
            m = re.search(r"1 0 0 1 (\S+) (\S+) cm", stream)
            rows.append(("bp", digits, s, m.group(1)))
    return rows


FONT_SIZES = sorted(set([65536, 655360, 650000, 654321, 72270, 72000, 1, 2, 3, 5, 6, 7, 100, 655, 656, 6553, 6554, 65535, 65537, 123456789, 134217727]
                        + [rng.randrange(66, 1 << 27) for _ in range(30)]))


def gen_font():
    # A size that rounds to nothing stops the run at the first character, so those are left out.
    kept = [s for s in FONT_SIZES if s >= 66]
    pdf = run("\n".join(r"\font\x=cmr10 at %dsp \shipout\hbox{\x\char`A}" % s for s in kept))
    sizes = re.findall(r"/F\d+ (\S+) Tf", pdf)
    assert len(sizes) == len(kept), (len(sizes), len(kept))
    return [("font", s, v) for s, v in zip(kept, sizes)]


MAG_VALUES = [0, 1, 3, 65536, 655360, 1431655, 1000000, 2000000, 3333333, 7654321, 100000000] + [rng.randrange(1, 1 << 27) for _ in range(12)]


def gen_mag():
    rows = []
    for mag in (1000, 1500, 2000, 500, 333, 2718):
        for digits in (3, 4):
            body = [r"\mag=%d \pdfdecimaldigits=%d \pdfhorigin=0truept \pdfvorigin=0truept \pdflinkmargin=0pt" % (mag, digits)]
            for s in MAG_VALUES:
                body.append(r"\shipout\hbox to 0pt{\kern%dsp\pdfannot width 1pt height 1pt depth 0pt{/Subtype/Text}\hss}" % s)
            pdf = run("\n".join(body))
            rects = re.findall(r"/Rect \[(\S+) ", pdf)
            assert len(rects) == len(MAG_VALUES), (len(rects), len(MAG_VALUES))
            rows += [("magbp", mag, digits, s, v) for s, v in zip(MAG_VALUES, rects)]
    return rows


def gen_origin():
    rows = []
    for digits in range(5):
        cases = []
        for _ in range(12):
            size = rng.choice((30, 70, 700, 70000))
            cases.append([(rng.randrange(-size, size + 1), rng.randrange(-size, size + 1)) for _ in range(rng.randrange(2, 12))])
        body = [r"\pdfdecimaldigits=%d" % digits]
        for moves in cases:
            page = "".join(r"\kern%dsp\raise%dsp\hbox{\pdfliteral{}}" % m for m in moves)
            body.append(r"\shipout\hbox{%s}" % page)
        streams = page_streams(run("\n".join(body)))
        assert len(streams) == len(cases)
        for moves, stream in zip(cases, streams):
            base = max(0, max(r for _, r in moves))
            h = 0
            out = []
            for dh, r in moves:
                h += dh
                out.append("%d:%d" % (h, base - r))
            cms = re.findall(r"1 0 0 1 (\S+) (\S+) cm", stream)
            rows.append(("origin", digits, ",".join(out), ";".join("%s %s" % c for c in cms)))
    return rows


def gen_rule():
    rows = []
    for yo in (5000000, -5000000):
        for digits in (4, 0):
            cases = []
            for r in range(0, 8):
                for h in (1, 2, 3, 4, 65781, 65782):
                    cases.append((0, r, 655360, h))
                for w in (1, 2, 3, 4, 65781, 65782):
                    cases.append((r, 0, w, 655360))
            cases += [(0, 0, 65782, 65783), (0, 0, 65783, 65783), (0, 0, 65783, 65782), (3, 5, 65782, 65782), (0, 0, 7, 7)]
            body = [r"\pdfdecimaldigits=%d \pdfhorigin=%dsp \pdfvorigin=%dsp" % (digits, yo, yo)]
            # The strut of 20pt fixes the box's height, so the rule's y is the origin plus 20pt less the raise.
            for x, r, w, h in cases:
                body.append(r"\shipout\hbox{\vrule width 0pt height 20pt depth 0pt\kern%dsp\raise%dsp\hbox{\vrule width %dsp height %dsp depth 0pt}}"
                            % (x, r, w, h))
            streams = page_streams(run("\n".join(body)))
            assert len(streams) == len(cases)
            for (x, r, w, h), stream in zip(cases, streams):
                m = re.search(r"q\n(.*?)\nQ", stream, re.S)
                rows.append(("rule", digits, yo + x, yo + 20 * 65536 - r, w, h, " ".join(m.group(1).split())))
    return rows


def main():
    rows = gen_bp() + gen_font() + gen_mag() + gen_origin() + gen_rule()
    with open(OUT, "w") as f:
        for row in rows:
            f.write("\t".join(str(x) for x in row) + "\n")
    print("%d rows" % len(rows))


if __name__ == "__main__":
    main()
