#!/usr/bin/env python3
"""What the oracle makes of deliberately damaged TFM files: crates/engine/testdata/tfm_oracle.tsv.

Run with python3 -I, from anywhere:

    python3 -I tests/oracle/gen_tfm_tables.py             regenerate the table
    python3 -I tests/oracle/gen_tfm_tables.py --rstex BIN also run BIN on each case and compare the logs

The files themselves are made at run time under target/tfm-damaged/ and are not committed.  Each comes from
one small synthetic font, BASE below, by the operations of a case; the unit test in crates/engine/src/tfm.rs
builds the same bytes from the same operations, and calls read_font_info on them.

Operations, separated by blanks:

    set:SEC:I=HEX   word I of section SEC (h c w t d i l k e p: header, char_info, width, height, depth, italic,
                    lig/kern, kern, exten, param)
    len:SEC=N       the section has N words: cut, or padded with zeros.  The size fields follow.
    bc=N ec=N       those size fields, moving nothing
    lf=N            add N to the lf field, which otherwise is right
    head:F=N        size field number F (0 is lf, 11 is np) is N, last of all
    cut=N           only the first N bytes of the file
    pad=N           N more zero bytes

Format of the table: name, font spec, s (the argument of read_font_info), operations, then what happened:

    BAD   the error messages, each line "! Font ...", separated by byte 0x0B

A run of one letter is written {a*255}, in any column.
    OK    font_params, dimens "d1,d2,...", chars "c:w,h,d,i,tagcode;...", \\pdfnoligatures, \\tagcode of 64, the messages
    LOST  rows named lost:LEVEL:CHAR: the message of char_warning at that \\tracinglostchars, on font "lost"

Each case is a separate run of `pdftex -ini -etex`, since 100 errors end a run.
"""
import os
import re
import shutil
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
OUT = os.path.join(ROOT, "crates", "engine", "testdata", "tfm_oracle.tsv")
WORK = os.path.join(ROOT, "target", "tfm-damaged")
MAX_LINE = 4000  # so that no message is broken across lines, whatever the names

# The font: bc=65 ('A'), ec=70 ('F').  B has a ligature program; C->D->A is a charlist; E is extensible.  The
# program starts and ends with a=255: right boundary char F, left boundary program at 1.
BASE = {
    "h": [0x01020304, 0x00A00000],
    "c": [0x01110400, 0x02200101, 0x03100244, 0x01000241, 0x01110300, 0x03210000],
    "w": [0x00000000, 0x00080000, 0x00099999, 0x000C0000],
    "t": [0x00000000, 0x000B3333, 0x000CCCCC],
    "d": [0x00000000, 0x00033333],
    "i": [0x00000000, 0x00019999],
    "l": [0xFF460002, 0x00418000, 0x80420043, 0xFF000001],
    "k": [0x00019999, 0xFFFE6667],
    "e": [0x41004243],
    "p": [0x00040000, 0x00055555, 0x0002AAAA, 0x0001C71C, 0x0006E147, 0x00100000, 0x00019999],
}
ORDER = "hcwtdilkep"


def build(ops):
    """The bytes of a file; crates/engine/src/tfm.rs has the same function."""
    sec = {k: list(v) for k, v in BASE.items()}
    bc, ec, dlf = 65, 70, 0
    head, cut, pad = {}, None, 0
    for op in ops.split():
        m = re.fullmatch(r"set:(\w):(\d+)=([0-9A-Fa-f]+)", op)
        if m:
            sec[m.group(1)][int(m.group(2))] = int(m.group(3), 16)
            continue
        m = re.fullmatch(r"len:(\w)=(\d+)", op)
        if m:
            s, n = sec[m.group(1)], int(m.group(2))
            sec[m.group(1)] = (s + [0] * n)[:n]
            continue
        m = re.fullmatch(r"head:(\d+)=(\d+)", op)
        if m:
            head[int(m.group(1))] = int(m.group(2))
            continue
        m = re.fullmatch(r"(bc|ec|lf|cut|pad)=(-?\d+)", op)
        if not m:
            raise ValueError(op)
        v = int(m.group(2))
        if m.group(1) == "bc":
            bc = v
        elif m.group(1) == "ec":
            ec = v
        elif m.group(1) == "lf":
            dlf = v
        elif m.group(1) == "cut":
            cut = v
        else:
            pad = v
    sizes = [len(sec[k]) for k in ORDER]
    lf = 6 + sum(sizes) + dlf
    fields = [lf, sizes[0], bc, ec] + sizes[2:]
    for f, v in head.items():
        fields[f] = v
    out = bytearray()
    for v in fields:
        out += (v & 0xFFFF).to_bytes(2, "big")
    for k in ORDER:
        for w in sec[k]:
            out += w.to_bytes(4, "big")
    out += bytes(pad)
    if cut is not None:
        out = out[:cut]
    return bytes(out)


# (name, operations, font spec, s, extra environment, how many fonts to load first)
CASES = [
    # Whole, with the sizes it can be asked for at.  z' and alpha change at 128pt, 256pt, 512pt and 1024pt.
    ("base", "", "", -1000),
    ("at5", "", " at 5pt", 327680),
    ("at127", "", " at 127.99998pt", 8388607),
    ("at128", "", " at 128pt", 8388608),
    ("at255", "", " at 255.99998pt", 16777215),
    ("at256", "", " at 256pt", 16777216),
    ("at1000", "", " at 1000pt", 65536000),
    ("atmax", "", " at 2047.99998pt", 134217727),
    ("at0sp", "", " at 1sp", 1),
    ("scaled500", "", " scaled 500", -500),
    ("scaled1", "", " scaled 1", -1),
    ("scaled2000", "", " scaled 2000", -2000),
    ("scaled32768", "", " scaled 32768", -32768),
    # The scale that gives 2048pt or more: the font loads at its design size.
    ("big_dsize", "set:h:1=7FFFFFFF", "", -1000),
    ("big_dsize_scaled1", "set:h:1=7FFFFFFF", " scaled 1000", -1000 - 0),
    ("big_dsize_scaled2", "set:h:1=7FFFFFFF", " scaled 1001", -1001),
    ("dsize10_scaled_over", "", " scaled 32767", -32767),
    ("dsize_2047", "set:h:1=7FFF0000", " scaled 1002", -1002),
    # Size fields: read_sixteen rejects a first byte above 127 in every field.
    *[("head%d_high" % f, "head:%d=32768" % f, "", -1000) for f in range(12)],
    ("lf_more", "lf=1", "", -1000),
    ("lf_less", "lf=-1", "", -1000),
    ("bc_past", "bc=72", "", -1000),
    ("bc_ec_plus1", "bc=71 len:c=0", "", -1000),
    ("no_chars", "bc=65 ec=64 len:c=0 len:l=0 len:e=0", "", -1000),
    ("bc256", "bc=256 ec=255 len:c=0 len:l=0 len:e=0", "", -1000),
    ("ec256", "ec=256 len:c=192", "", -1000),
    ("ec255", "ec=255 len:c=191", "", -1000),
    ("bc0", "bc=0 ec=70 len:c=71", "", -1000),
    ("nw0", "len:w=0", "", -1000),
    ("nh0", "len:t=0", "", -1000),
    ("nd0", "len:d=0", "", -1000),
    ("ni0", "len:i=0", "", -1000),
    # The header.
    ("lh0", "len:h=0", "", -1000),
    ("lh1", "len:h=1", "", -1000),
    ("lh3", "len:h=3", "", -1000),
    ("lh18", "len:h=18", "", -1000),
    ("dsize_negative", "set:h:1=80000000", "", -1000),
    ("dsize_negative2", "set:h:1=FFF00000", "", -1000),
    ("dsize_small", "set:h:1=000FFFFF", "", -1000),
    ("dsize_1pt", "set:h:1=00100000", "", -1000),
    ("checksum_any", "set:h:0=FFFFFFFF", "", -1000),
    # Character data.
    ("char_width", "set:c:0=04000000", "", -1000),
    ("char_height", "set:c:0=01300000", "", -1000),
    ("char_depth", "set:c:0=01020000", "", -1000),
    ("char_italic", "set:c:0=01000800", "", -1000),
    ("char_all_at_limit", "set:c:0=03210400", "", -1000),
    ("lig_range", "set:c:1=02200104", "", -1000),
    ("lig_last", "set:c:1=02200103", "", -1000),
    ("ext_range", "set:c:4=01110301", "", -1000),
    ("list_low", "set:c:3=01000240", "", -1000),
    ("list_high", "set:c:3=01000247", "", -1000),
    ("list_self", "set:c:5=03210246", "", -1000),
    ("list_two", "set:c:3=01000243", "", -1000),
    ("list_three", "set:c:0=01110242 set:c:1=02200243 set:c:2=03100241", "", -1000),
    ("list_forward", "set:c:0=01110242", "", -1000),
    ("list_to_missing", "set:c:0=01110245 set:c:4=00000000", "", -1000),
    ("tag_on_missing", "set:c:0=00000100", "", -1000),
    # Dimensions.
    ("width0", "set:w:0=00100000", "", -1000),
    ("height0", "set:t:0=00100000", "", -1000),
    ("depth0", "set:d:0=00100000", "", -1000),
    ("italic0", "set:i:0=00100000", "", -1000),
    ("width0_tiny", "set:w:0=00000001", "", -1000),
    ("width_a1", "set:w:1=01000000", "", -1000),
    ("width_a254", "set:w:1=FE000000", "", -1000),
    ("width_negative", "set:w:1=FFF00000", "", -1000),
    ("width_most_negative", "set:w:1=FF000000", "", -1000),
    ("width_most_positive", "set:w:1=00FFFFFF", "", -1000),
    ("width_extremes_at_max", "set:w:1=00FFFFFF set:w:2=FF000000 set:w:3=FFFFFFFF", " at 2047.99998pt", 134217727),
    ("height_a1", "set:t:1=01000000", "", -1000),
    ("depth_a1", "set:d:1=01000000", "", -1000),
    ("italic_a1", "set:i:1=01000000", "", -1000),
    # The ligature and kern program.
    ("lk_next_range", "set:l:0=FF460004", "", -1000),
    ("lk_next_high", "set:l:0=FF460100", "", -1000),
    ("lk_a129", "set:l:0=81460002", "", -1000),
    ("lk_not_first_255", "set:l:0=00418000 set:l:1=FF460002", "", -1000),
    ("lk_b_low", "set:l:1=00408000", "", -1000),
    ("lk_b_high", "set:l:1=00478000", "", -1000),
    ("lk_b_missing", "set:c:4=00000000 set:l:1=00458000", "", -1000),
    ("lk_b_bchar_missing", "set:c:5=00000000 set:l:1=00468000", "", -1000),
    ("lk_b_bchar_range", "set:l:0=FF500002 set:l:1=00508000", "", -1000),
    ("lk_lig_low", "set:l:2=80420040", "", -1000),
    ("lk_lig_high", "set:l:2=80420047", "", -1000),
    ("lk_lig_missing", "set:c:4=00000000 set:l:2=80420045", "", -1000),
    ("lk_kern_range", "set:l:1=00418002", "", -1000),
    ("lk_kern_high", "set:l:1=00418100", "", -1000),
    ("lk_kern_last", "set:l:1=00418001", "", -1000),
    ("lk_skip_past", "set:l:1=02418000", "", -1000),
    ("lk_skip_last", "set:l:1=01418000", "", -1000),
    ("lk_stop", "set:l:1=80418000", "", -1000),
    ("lk_a127", "set:l:1=7F418000", "", -1000),
    ("lk_last_not_255", "set:l:3=80410043", "", -1000),
    ("lk_last_range", "set:l:3=FF000004", "", -1000),
    ("lk_nl0", "len:l=0 set:c:1=02200000", "", -1000),
    ("kern_a1", "set:k:0=01000000", "", -1000),
    ("kern_nk0", "len:k=0 set:l:1=00410000", "", -1000),
    ("kern_nk0_used", "len:k=0", "", -1000),
    # Extensible recipes.
    ("ext_top_missing", "set:e:0=40004243", "", -1000),
    ("ext_mid_missing", "set:e:0=41404243", "", -1000),
    ("ext_bot_missing", "set:e:0=41004043", "", -1000),
    ("ext_rep_zero", "set:e:0=41004200", "", -1000),
    ("ext_rep_missing", "set:e:0=41004240", "", -1000),
    ("ext_zeros", "set:e:0=00000043", "", -1000),
    ("ne0", "len:e=0 set:c:4=01110000", "", -1000),
    # Parameters.
    ("np0", "len:p=0", "", -1000),
    ("np1", "len:p=1", "", -1000),
    ("np6", "len:p=6", "", -1000),
    ("np9", "len:p=9 set:p:7=00010000 set:p:8=FFFF0000", "", -1000),
    ("slant_huge", "set:p:0=7FFFFFFF", "", -1000),
    ("slant_negative", "set:p:0=80000000", "", -1000),
    ("slant_minus1", "set:p:0=FFFFFFFF", "", -1000),
    ("param_a1", "set:p:1=01000000", "", -1000),
    ("param_a255", "set:p:1=FFF00000", "", -1000),
    # Files of the wrong length.  The first three keep or add data; the rest run out at each part of the file.
    ("pad1", "pad=1", "", -1000),
    ("pad4", "pad=4", "", -1000),
    *[("cut%d" % n, "cut=%d" % n, "", -1000) for n in (0, 1, 2, 3, 23, 24, 25, 31, 32, 33, 40, 56, 72, 90, 110, 124, 128, 155)],
    ("cut156", "cut=156", "", -1000),
    # End of file with the sizes of a good font, but so large that 255s read past the end are all in range.
    ("eof_in_range", "head:1=2 len:w=300 len:t=20 len:d=20 len:i=70 len:e=300 cut=32", "", -1000),
    ("eof_all_size_fields", "head:0=0 head:1=2050 head:2=1 head:3=0 cut=8", "", -1000),
]

# Not damaged, and not a file made here: (name, spec text, s).  nofile keeps the name as the unit test passes it.
NOFILE = [
    ("missing", "nosuchfontname", "", -1000),
    ("missing_at", "nosuchfontname", " at 5pt", 327680),
    ("missing_scaled", "nosuchfontname", " scaled 2000", -2000),
    ("missing_area", "nosuchdir/nosuchfontname", "", -1000),
    ("missing_quoted", '"no such font"', "", -1000),
    ("missing_quoted_area", '"no such dir/no such font"', "", -1000),
    ("name_255", "a" * 255, "", -1000),
    ("name_256", "a" * 256, "", -1000),
    ("area_256", "d" * 255 + "/x", "", -1000),
    ("area_255", "d" * 254 + "/x", "", -1000),
]

# Fonts with only some kinds of tag, for \\pdfnoligatures: list tags count as none, ext tags as ligatures.
NOLIG = [
    ("nolig_only_list", "set:c:1=02200000 set:c:4=01110000 len:l=0 len:k=0", "", -1000),
    ("nolig_ext", "set:c:1=02200000 len:l=0 len:k=0", "", -1000),
    ("nolig_lig", "set:c:4=01110000", "", -1000),
]
CASES += NOLIG

# Run with these sizes: (name, operations, environment, fonts loaded first).
ROOM = [
    ("room_memory", "len:p=30000", {"font_mem_size": "20000"}, 0),
    ("room_fonts", "", {"font_max": "50"}, 50),
]

PREAMBLE = r"""\catcode`\{=1 \catcode`\}=2 \catcode`\#=6 \catcode`\^=7
\newlinechar=-1 \errorcontextlines=0 \scrollmode
\font\y=cmr10 at 1pt
"""

# The chars, the dimens and the number of params.  \y is loaded after \x, so that \fontdimen tells how many it has.
REPORT = r"""\immediate\write16{NAME:\fontname\x}%
\def\dims#1{\ifnum#1<8 \immediate\write16{DIM:#1:\number\fontdimen#1\x}%
  \expandafter\dims\expandafter{\number\numexpr#1+1\expandafter}\fi}%
\count1=0
\def\chars{\iffontchar\x\count1
    \immediate\write16{CHAR:\number\count1:\number\fontcharwd\x\count1,\number\fontcharht\x\count1,%
\number\fontchardp\x\count1,\number\fontcharic\x\count1,\number\tagcode\x\count1}\fi
  \advance\count1 1 \ifnum\count1<256 \expandafter\chars\fi}%
\ifx\x\nullfont\else
  \font\z=cmr10 at 2pt
  \dims{1}\chars
  \immediate\write16{NOLIG:\number\pdfnoligatures\x:\number\tagcode\x 64}%
  \fontdimen50000\x=1pt
\fi
\end
"""


def run(tmp, name, spec, env, fonts_first, binary=("pdftex", "-ini", "-etex")):
    body = [PREAMBLE]
    for k in range(fonts_first - 1):
        body.append(r"\font\q=cmr10 at %dpt" % (k + 2))
    body.append(r"\font\x=" + spec + "\\relax\n")
    body.append(REPORT)
    path = os.path.join(tmp, name + ".drive.tex")
    with open(path, "w") as f:
        f.write("".join(l if l.endswith("\n") else l + "\n" for l in body))
    e = dict(os.environ, max_print_line=str(MAX_LINE), error_line="79", half_error_line="50", MKTEXTFM="0", TFMFONTS=".:" + "{$TEXMF/fonts/tfm}//")
    e.update(env)
    subprocess.run(list(binary) + ["-interaction=nonstopmode", name + ".drive.tex"], cwd=tmp, env=e,
                   stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, check=False)
    with open(os.path.join(tmp, name + ".drive.log"), "rb") as f:
        return f.read().decode("latin-1")


def result(log):
    msgs = re.findall(r"^! Font \\x=.*$", log, re.M)
    if not re.search(r"^NAME:", log, re.M):
        sys.exit("no NAME line in\n" + log)
    if not re.search(r"^CHAR:|^DIM:", log, re.M):
        return "BAD\t" + "\x0b".join(msgs)
    # The error message for \fontdimen50000 gives the number of parameters.
    only = re.search(r"^! Font \\x has only (\d+) fontdimen parameters", log, re.M)
    if not only:
        sys.exit("no parameter count in\n" + log)
    dims = [m[1] for m in re.findall(r"^DIM:(\d):(-?\d+)$", log, re.M)]
    chars = ";".join("%s:%s" % m for m in re.findall(r"^CHAR:(\d+):(-?\d+,-?\d+,-?\d+,-?\d+,-?\d+)$", log, re.M))
    nolig = re.search(r"^NOLIG:(-?\d+):(-?\d+)$", log, re.M)
    return "OK\t%s\t%s\t%s\t%s\t%s\t%s" % (only.group(1), ",".join(dims), chars, nolig.group(1), nolig.group(2), "\x0b".join(msgs))


def lost(level, c):
    """The message for character c, missing or not, from char_warning."""
    name = "lostdrive"
    body = PREAMBLE + r"""\font\x=lost \x \tracinglostchars=%d \setbox0\hbox{\char%d\relax}\end
""" % (level, c)
    with open(os.path.join(WORK, name + ".tex"), "w") as f:
        f.write(body)
    subprocess.run(["pdftex", "-ini", "-etex", "-interaction=nonstopmode", name + ".tex"], cwd=WORK,
                   env=dict(os.environ, max_print_line=str(MAX_LINE)),
                   stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, check=False)
    with open(os.path.join(WORK, name + ".log"), "rb") as f:
        log = f.read().decode("latin-1")
    return "\x0b".join(re.findall(r"^(?:! )?Missing character.*$", log, re.M))


def spec_of(name, text):
    return name + text


def main():
    rstex = None
    if "--rstex" in sys.argv:
        rstex = (sys.argv[sys.argv.index("--rstex") + 1], "-ini", "-etex")
    shutil.rmtree(WORK, ignore_errors=True)
    os.makedirs(WORK)
    rows = []
    bad = 0
    for name, ops, text, s in CASES:
        with open(os.path.join(WORK, name + ".tfm"), "wb") as f:
            f.write(build(ops))
        log = run(WORK, name, spec_of(name, text), {}, 0)
        rows.append("\t".join([name, text, str(s), ops, result(log)]))
        if rstex:
            bad += compare(log, run(WORK, name, spec_of(name, text), {}, 0, rstex), name)
    for name, spec, text, s in NOFILE:
        log = run(WORK, name, spec + text, {}, 0)
        rows.append("\t".join([name, text, str(s), "@" + spec, result(log)]))
    for name, ops, env, first in ROOM:
        with open(os.path.join(WORK, name + ".tfm"), "wb") as f:
            f.write(build(ops))
        log = run(WORK, name, name, env, first)
        rows.append("\t".join([name, "", "-1000", ops, result(log)]))
    with open(os.path.join(WORK, "lost.tfm"), "wb") as f:
        f.write(build(""))
    for level in (0, 1, 2, 3):
        for c in (64, 65, 71, 0, 200):
            rows.append("\t".join(["lost:%d:%d" % (level, c), "", "", "", "LOST", lost(level, c)]))
    with open(OUT, "w", encoding="latin-1", newline="\n") as f:
        f.write(re.sub(r"([ad])\1{39,}", lambda m: "{%s*%d}" % (m.group(1), len(m.group(0))), "\n".join(rows)) + "\n")
    print(len(rows), "cases in", OUT)
    if rstex and bad:
        sys.exit("%d cases differ from the oracle" % bad)


def masked(log):
    return re.sub(r"^This is .*\n", "", log, flags=re.M)


def compare(a, b, name):
    if masked(a) == masked(b):
        return 0
    print("differs:", name)
    return 1


if __name__ == "__main__":
    main()
