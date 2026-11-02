#!/usr/bin/env python3
"""Expected results for the unit tests of hyph.rs and page.rs, taken from the oracle.

Run with python3 -I.  It writes, under crates/engine/testdata/:

  hyph_trie_oracle.tsv   a program of patterns and exceptions, then what the oracle's \\dump holds for it: hyph_count,
                         hyph_next, the slots of the exception table, trie_max, hyph_start, trie_op_ptr, trie_used,
                         and FNV-1a digests of trie_trl, trie_tro, trie_trc and of the hyf_distance, hyf_num, hyf_next
                         arrays.  The format is read directly, so this is the packed trie itself.
  hyph_words_oracle.tsv  a line with a program, then lines with \\language, the two hyphenmins and a word, and where the
                         oracle hyphenates it, found as plain's \\showhyphens finds it.
  page_oracle.tsv        a page: settings and the items of the contribution list, then the \\tracingpages lines the
                         oracle prints for it and the values of \\lastpenalty, \\lastkern, \\lastskip, \\lastnodetype.
                         No row reaches fire_up: the unit test runs build_page, whose callees up to there are real.

  hyph_recon_oracle.tsv  a font's ligature program and a word, hyphenated by an exception before its last letter, then the
                         nodes the oracle has before the discretionary, in its pre_break, and after it.  The font is made by
                         pltotf, in the temporary directory, since no TFM of this TeX Live has these operations: a binary is
                         not committed.  The test builds the same program by hand in font_info and calls reconstitute.

A program is a list of words separated by spaces:

  L=n    \\language=n     S=n  \\savinghyphcodes=n     C=c:v  \\lccode c=v     X=ta-ble  an exception     others  a pattern

Items of a page, separated by spaces, all dimensions in whole points:

  r10,2  \\hrule height 10pt depth 2pt
  g10,5,0,2  \\vskip 10pt plus 5pt minus 2pt; the third number is the order of the stretch
  k3  \\kern3pt       p50  \\penalty50       P  \\par, which only calls build_page
  i100,2,50:r10,2/g3,1,0,0/p7/r5,0  \\insert100 made with \\splitmaxdepth=2pt, \\floatingpenalty=50, its items after the colon

Settings, separated by spaces:  vsize=100 maxdepth=2 topskip=10  C100=500 (\\count)  D100=30 (\\dimen)
S100=w,st,order,sh (\\skip)  B100=h,d (\\box: a \\vbox of a rule).  A dimension of M is \\maxdimen.

Every case runs in `pdftex -ini -etex`, in a directory of its own.  One case per line, fields separated by a tab; a newline
inside a field is `|`.
"""
import gzip
import os
import random
import re
import struct
import subprocess
import sys
import tempfile

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
OUT = os.path.join(ROOT, "crates", "engine", "testdata")

PREAMBLE = r"""\catcode`\{=1 \catcode`\}=2 \catcode`\#=6 \catcode`\^=7 \catcode`\_=8
\catcode`\&=4 \catcode`\$=3 \catcode`\~=13
\scrollmode \tracingonline=0 \newlinechar=-1 \dimendef\maxdimen=255 \maxdimen=16383.99999pt
"""


def run_oracle(src, tmp, name="case"):
    path = os.path.join(tmp, name + ".tex")
    with open(path, "w", encoding="latin-1") as f:
        f.write(src)
    subprocess.run(
        ["pdftex", "-ini", "-etex", "-interaction=nonstopmode", "*" + name + ".tex"],
        cwd=tmp,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        check=False,
    )
    with open(os.path.join(tmp, name + ".log"), encoding="latin-1") as f:
        return f.read()


def program_tex(program):
    """TeX source for a program of patterns: each run of patterns becomes one \\patterns."""
    out, pats = [], []

    def flush():
        if pats:
            out.append("\\patterns{%s}" % " ".join(pats))
            pats.clear()

    for w in program.split():
        if w.startswith("L="):
            flush()
            out.append("\\language=%s " % w[2:])
        elif w.startswith("S="):
            flush()
            out.append("\\savinghyphcodes=%s " % w[2:])
        elif w.startswith("C="):
            flush()
            c, v = w[2:].split(":")
            out.append("\\lccode%s=%s " % (c, v))
        elif w.startswith("X="):
            flush()
            out.append("\\hyphenation{%s}" % w[2:])
        else:
            pats.append(w)
    flush()
    return "\n".join(out) + "\n"


def fnv(values):
    h = 0xcbf29ce484222325
    for v in values:
        for b in struct.pack("<q", v):
            h = ((h ^ b) * 0x100000001b3) & 0xffffffffffffffff
    return h


def read_ints(d, o, n, size):
    return [int.from_bytes(d[o + i * size : o + (i + 1) * size], "little", signed=True) for i in range(n)]


def dump_row(program, tmp):
    """Reads what the hyphenation part of the format holds.  Integers are 8 bytes in this oracle's format, trie_trc 1, the small_numbers of
    hyf_distance and hyf_num 2, hyf_next 2; the search below fails if that ever changes."""
    src = PREAMBLE + program_tex(program) + "\\year=2026 \\month=1 \\day=1 \\dump\n"
    log = run_oracle(src, tmp)
    m = re.search(r"Hyphenation trie of length (\d+) has (\d+) ops? out of", log)
    if not m:
        sys.exit("no trie line for %r: %s" % (program, log[-800:]))
    trie_max, ops = int(m.group(1)), int(m.group(2))
    m = re.search(r"^(\d+) hyphenation exceptions?$", log, re.M)
    count = int(m.group(1))
    used = [(int(a), int(b)) for a, b in re.findall(r"^  (\d+) for language (\d+)$", log, re.M)]
    d = gzip.open(os.path.join(tmp, "case.fmt")).read()
    found = []
    i = d.find(struct.pack("<q", trie_max))
    while i >= 0:
        o = i + 16 + (trie_max + 1) * 17
        questions = i + 16 + (trie_max + 1) * 16  # trie_char(0)="?"
        if read_ints(d, o, 1, 8) == [ops] and read_ints(d, i - 8 * (2 + 3 * count), 1, 8) == [count] and d[questions:questions + 1] == b"?":
            found.append(i)
        i = d.find(struct.pack("<q", trie_max), i + 1)
    if len(found) != 1:
        sys.exit("trie of %r found %d times" % (program, len(found)))
    i = found[0]
    next_ = read_ints(d, i - 8 * (1 + 3 * count), 1, 8)[0]
    words = read_ints(d, i - 8 * 3 * count, 3 * count, 8)
    hyph_slots = [(words[k] % 65536, words[k] // 65536) for k in range(0, 3 * count, 3)]
    hs = read_ints(d, i + 8, 1, 8)[0]
    o = i + 16
    trl = read_ints(d, o, trie_max + 1, 8)
    o += (trie_max + 1) * 8
    tro = read_ints(d, o, trie_max + 1, 8)
    o += (trie_max + 1) * 8
    trc = read_ints(d, o, trie_max + 1, 1)
    trc = [c & 255 for c in trc]
    o += trie_max + 1 + 8
    for size in (1, 2, 4, 8):
        for size3 in (1, 2, 4, 8):
            p = o + ops * (2 * size + size3)
            want = [x for u, k in used for x in (k, u)]
            if want and read_ints(d, p, len(want), 8) == want:
                break
        else:
            continue
        break
    else:
        if used:
            sys.exit("op tables of %r not found" % program)
        size, size3 = 1, 2
    dist = read_ints(d, o, ops, size)
    num = read_ints(d, o + ops * size, ops, size)
    nxt = read_ints(d, o + ops * 2 * size, ops, size3)
    return [
        program,
        count,
        next_,
        ",".join("%d:%d" % s for s in hyph_slots),
        trie_max,
        hs,
        ops,
        ",".join("%d:%d" % (k, u) for u, k in used),
        fnv(trl),
        fnv(tro),
        fnv(trc),
        fnv(dist + num + nxt),
    ]


def colliding_words(count):
    """Words of the same hash, so that the exception table has to link them."""
    slots = {}
    for a in "abcdeghijk":
        for b in "abcdeghijk":
            for c in "abcdeghijk":
                w = a + "-" + b + c
                h = ord(a)
                for x in (b, c, chr(0)):
                    h = (h + h + ord(x)) % 607
                slots.setdefault(h, []).append(w)
                if len(slots[h]) == count:
                    return slots[h]
    sys.exit("no collision")


COLLIDING = colliding_words(6)

PROGRAMS = [
    # No patterns at all: §1135's first branch.
    "",
    "X=ta-ble",
    # Knuth's example of Appendix H, and edge delimiters, levels at both ends and a digit after a digit.
    "a1b",
    "a1b 2cd e3f4g .x1 y2. hy3ph he2n hena4 hen5at 1na n2at 1tio 2io o2n",
    "1a2b3c4d5e6g7h8i9j",
    ".a1 1a. .1a a1. 1. .1 1.1",
    "a12b",
    # Families that do not fit, so first_fit walks its holes; shared subtries, so compress_trie merges.
    "ax bx cx dx ex gx hx ix jx kx lx mx nx ox px qx rx sx tx ux vx wx xx yx zx",
    "a1z b1z c1z d1z a2y b2y c2y d2y aa1 ab1 ac1 za1 zb1 zc1",
    # Several languages, in an order that makes §1122 move ops around.
    "L=5 a1b c2d L=2 a1b e3g L=255 a1b L=0 h4i a1b",
    "L=300 a1b L=-1 c1d L=256 e1g",
    # \savinghyphcodes: §1855 with and without patterns of the language, and lccode changes between.
    "S=1 L=1 a1b",
    "S=1 C=45:45 L=1 a1b L=3 C=97:0 c1d L=2 S=0 e1g",
    "S=1 L=255 a1b L=0 g1h L=7",
    "S=1 C=255:255 L=1 a1b",
    # Exceptions: repeats, a language, collisions in one hash slot and hyphens at the ends.
    "a1b X=ta-ble X=ta-ble X=tab-le L=1 X=ta-ble X=-ab X=ab- X=a-b-c",
    "X=aa-b X=ab-b X=ac-b X=ad-b X=ae-b X=ag-b X=ah-b X=ai-b",
    "X=" + " X=".join(COLLIDING),
    # One word of the maximal length, and past it: k stops at 63.
    "a" * 63 + "1 " + "b" * 70,
]


def letters(rng, n):
    return "".join(rng.choice("abcdeghijklmnopqrstuvwxyz") for _ in range(n))


def synthetic_patterns(seed, count):
    rng = random.Random(seed)
    seen, out = set(), []
    while len(out) < count:
        w = letters(rng, rng.randint(1, 5))
        if w in seen:
            continue
        seen.add(w)
        cut = rng.randint(0, len(w))
        p = w[:cut] + str(rng.randint(1, 9)) + w[cut:]
        if rng.random() < 0.3:
            cut = rng.randint(0, len(p))
            if cut == 0 or not p[cut - 1].isdigit():
                if cut == len(p) or not p[cut].isdigit():
                    p = p[:cut] + str(rng.randint(1, 9)) + p[cut:]
        if rng.random() < 0.05:
            p = "." + p
        elif rng.random() < 0.05:
            p = p + "."
        out.append(p)
    return " ".join(out)


PROGRAMS.append(synthetic_patterns(1, 400))
PROGRAMS.append("L=1 " + synthetic_patterns(2, 200) + " L=2 " + synthetic_patterns(3, 200))

WORD_PROGRAMS = [
    "hy3ph he2n hena4 hen5at 1na n2at 1tio 2io o2n",
    "hy3ph he2n hena4 hen5at 1na n2at 1tio 2io o2n X=ta-ble X=ta-ble X=hy-phe-na-tion X=pro-gram L=1 X=pro-g-ram",
    "L=1 a1b b1c c1d d1e L=2 a2b b1c X=a-e L=0 .a1 e1. a1b",
    synthetic_patterns(1, 400),
    "S=1 L=1 a1b b1c",
]

WORDS = [
    (0, 2, 3, "hyphenation"), (0, 1, 1, "hyphenation"), (0, 6, 2, "hyphenation"), (0, 2, 3, "nation"), (0, 2, 3, "abc"),
    (1, 2, 3, "table"), (1, 1, 1, "table"), (1, 2, 3, "hyphenation"), (1, 2, 3, "program"), (1, 2, 3, "programs"),
    (1, 2, 3, "tables"), (2, 1, 1, "abcde"), (2, 2, 2, "abcde"), (3, 1, 1, "unknown"),
]


def word_rows(tmp):
    rows = []
    rng = random.Random(4)
    words = [(0, i, l, r, w) for i, l, r, w in WORDS]
    words += [(1, 2, 1, 1, "abcde"), (2, 2, 1, 1, "abcde"), (2, 2, 1, 1, "ae"), (0, 2, 1, 1, "abab"), (0, 2, 1, 1, "aab"), (9, 2, 1, 1, "abcde")]
    words += [(1, 4, 1, 1, "abcabc"), (0, 4, 1, 1, "abcabc")]
    for _ in range(120):
        words.append((0, 3, rng.randint(1, 3), rng.randint(1, 3), letters(rng, rng.randint(4, 14))))
    for pi, program in enumerate(WORD_PROGRAMS):
        ws = [w for w in words if w[1] == pi]
        if not ws:
            continue
        rows.append(["P", program])
        src = PREAMBLE + program_tex(program)
        src += r"""\font\x=cmr10 \x \hyphenchar\x=`\- \linepenalty=10 \hyphenpenalty=50 \exhyphenpenalty=50 \hsize=\maxdimen \parfillskip=0pt \pretolerance=-1 \tolerance=-1 \hbadness=0
\showboxdepth=0 \showboxbreadth=1000
\def\w#1#2#3#4{\language=#1 \lefthyphenmin=#2 \righthyphenmin=#3 \setbox0\vbox{\ #4\par}\immediate\write-1{ENDWORD}}
"""
        for lang, _, l, r, w in ws:
            src += "\\w{%d}{%d}{%d}{%s}\n" % (lang, l, r, w)
        src += "\\end\n"
        log = run_oracle(src, tmp)
        parts = log.split("ENDWORD\n")
        if len(parts) != len(ws) + 1:
            sys.exit("words of program %d: %d parts for %d words: %s" % (pi, len(parts), len(ws), log[-800:]))
        for (lang, _, l, r, w), part in zip(ws, parts):
            m = re.findall(r"^\[\] \\x (.*)$", part, re.M)
            if len(m) != 1:
                sys.exit("word %r: %r" % (w, part))
            text = m[0]
            pos, n = [], 0
            for c in text:
                if c == "-":
                    pos.append(str(n))
                else:
                    n += 1
            if text.replace("-", "") != w:
                sys.exit("word %r came back as %r" % (w, text))
            rows.append(["W", lang, l, r, w, ",".join(pos)])
    return rows


PAGES = [
    ("simple", "vsize=100", "r10,0 g5,2,0,1 r10,0 p50 r10,0 p-50 r10,0 g0,0,0,0 p0"),
    ("topskip_bigger", "vsize=100 topskip=15", "r10,1 p7"),
    ("topskip_smaller", "vsize=100 topskip=5", "r10,1 p7"),
    ("topskip_negative_height", "vsize=100 topskip=5", "r-3,1 p7"),
    ("discards", "vsize=100", "g10,0,0,0 k5 p100 g3,1,0,1 r10,0 p0"),
    ("stretch_orders", "vsize=90", "r10,0 g1,5,0,0 r1,0 g1,2,1,0 r1,0 g1,3,2,0 r1,0 g1,4,3,0 r1,0 p3"),
    ("shrink_badness", "vsize=30", "r25,0 g5,0,0,3 r2,0 p1 g0,0,0,4 p2"),
    ("too_loose", "vsize=1000", "r1,0 p0 r1,0 p-1"),
    ("max_depth_small", "vsize=100 maxdepth=2", "r10,7 p1 r10,1 p2 r10,2 p3"),
    ("max_depth_large", "vsize=100 maxdepth=50", "r10,7 p1 r10,1 p2 g3,0,0,0 p3"),
    ("glue_after_box_after_glue", "vsize=100", "r10,0 g1,0,0,0 g1,0,0,0 p5 g1,0,0,0 r1,0 p6"),
    ("kerns", "vsize=100", "r10,0 k1 g1,0,0,0 k2 r3,0 k4 p5 k5 g1,0,0,0 p6"),
    ("penalties", "vsize=20", "r10,0 p10000 p9999 p-9999 p10000 g1,0,0,0 p0"),
    ("tie_takes_the_later", "vsize=20", "r20,0 p0 p0 p0 g0,0,0,0 p1 p-1"),
    # Insertions.
    ("ins_fits", "vsize=100 C100=1000 D100=M", "r10,0 i100,2,0:r5,1 p1 i100,2,0:r5,1 p2"),
    ("ins_scaled", "vsize=100 C100=500 D100=M", "r10,0 i100,2,0:r6,0 p1"),
    ("ins_scaled_up", "vsize=100 C100=2500 D100=M", "r10,0 i100,2,0:r6,0 p1"),
    ("ins_no_room_uses_the_count", "vsize=100 C100=0 D100=M", "r10,0 i100,2,0:r200,0 p1"),
    ("ins_only_page_then_box", "vsize=100 C100=1000 D100=M topskip=3", "i100,2,0:r5,1 p1 g1,0,0,0 r10,0 p2"),
    ("ins_skip", "vsize=100 C100=1000 D100=M S100=4,2,0,1", "r10,0 i100,2,0:r5,0 p1 i100,2,0:r5,0 p2"),
    ("ins_skip_fil", "vsize=100 C100=1000 D100=M S100=4,2,2,1", "r10,0 i100,2,0:r5,0 p1"),
    ("ins_box_there", "vsize=100 C100=1000 D100=M B100=7,2", "r10,0 i100,2,0:r5,0 p1"),
    ("ins_box_scaled", "vsize=100 C100=250 D100=M B100=8,2", "r10,0 i100,2,0:r4,0 p1"),
    ("ins_two_classes", "vsize=100 C100=1000 D100=M C50=1000 D50=M C200=1000 D200=M", "r10,0 i100,2,0:r5,0 i50,2,0:r6,0 i200,2,0:r7,0 i50,2,0:r1,0 p1"),
    # §1187: the split.  p= shows the penalty at the break, 0 for glue or a kern, -10000 for the end of the list.
    ("split_dimen_limit", "vsize=100 C100=1000 D100=20", "r10,0 i100,2,0:r15,1/p9/r15,1/p8/r15,1 p1"),
    ("split_page_limit", "vsize=30 C100=1000 D100=M", "r10,0 i100,2,0:r15,0/g1,0,0,0/r15,0 p1"),
    ("split_nothing_fits", "vsize=30 C100=1000 D100=M", "r10,0 i100,2,0:r45,0"),
    ("split_then_float_cost", "vsize=30 C100=1000 D100=M", "r10,0 i100,2,70:r15,0/g1,0,0,0/r15,0 i100,2,70:r1,0 i100,2,70:r1,0 r5,0 p1 p2"),
    ("split_then_negative_float_cost", "vsize=30 C100=1000 D100=M", "r10,0 i100,2,-70:r15,0/g1,0,0,0/r15,0 i100,2,-70:r1,0 r5,0 p1"),
    ("split_at_penalty_adds_it", "vsize=30 C100=1000 D100=M", "r10,0 i100,2,0:r15,0/p40/r15,0 r5,0 p1"),
    ("split_at_end_adds_eject", "vsize=30 C100=1000 D100=M", "r10,0 i100,2,0:r45,0 r-40,0 p1"),
    ("ends_in_glue", "vsize=100", "r10,0 p6 g3,1,0,2 P"),
    ("ends_in_rule", "vsize=100", "r10,0 g3,1,0,2 r1,0 P"),
    ("ends_in_insert", "vsize=100 C100=1000 D100=M", "r10,0 g3,1,0,2 i100,2,0:r1,0"),
    ("ends_in_discarded_glue", "vsize=100", "g3,1,0,2 P"),
    ("ends_in_discarded_penalty", "vsize=100", "p77 P"),
    ("split_count_zero", "vsize=100 C100=0 D100=20", "r10,0 i100,2,0:r15,1/p9/r15,1 p1"),
    ("split_count_negative", "vsize=100 C100=-5 D100=20", "r10,0 i100,2,0:r15,1/p9/r15,1 p1"),
    ("split_count_scaled", "vsize=30 C100=2000 D100=M", "r10,0 i100,2,0:r6,0/p4/r6,0/p3/r6,0 p1"),
    ("split_count_half", "vsize=30 C100=500 D100=M", "r10,0 i100,2,0:r40,0/g1,0,0,0/r30,0 p1"),
    ("split_depth_limit", "vsize=100 C100=1000 D100=20", "r10,0 i100,3,0:r15,8/p9/r15,1 p1"),
    ("split_glue_and_kern", "vsize=100 C100=1000 D100=30", "r10,0 i100,2,0:r15,0/k3/g1,0,0,0/r15,0/k3/r1,0/g1,5,0,1/r15,0 p1"),
    ("split_stretch_orders", "vsize=100 C100=1000 D100=30", "r10,0 i100,2,0:r10,0/g1,5,1,0/r10,0/g1,5,2,0/r10,0/g1,5,3,0/r10,0 p1"),
    ("split_ends_in_penalties", "vsize=100 C100=1000 D100=8", "r10,0 i100,2,0:r5,0/p10000/r5,0/p-10000/r1,0 p1"),
    ("split_deplorable", "vsize=100 C100=1000 D100=500", "r10,0 i100,2,0:r5,0/p3/r600,0 p1"),
    ("split_second_goes_on_the_same_node", "vsize=30 C100=1000 D100=M", "r5,0 i100,2,0:r10,0/p4/r30,0 i100,2,0:r4,0 p1"),
    # The arithmetic of section 11: with \vsize=\maxdimen the oracle goes past 2^31 and clamps to dimen(n)-height(r).
    ("sat_w_clamps", "vsize=M C100=1 D100=M", "r10,0 i100,2,0:r16000,0/p5/r16000,0/p4/r16000,0"),
    ("sat_w_clamps_dimen", "vsize=M C100=999 D100=40", "r10,0 i100,2,0:r30,0/p5/r30,0"),
]


def tex_dim(x):
    return "\\maxdimen " if x == "M" else "%spt " % x


def item_tex(item):
    k, f = item[0], item[1:].split(",")
    if k == "r":
        return "\\hrule height %spt depth %spt width 0pt " % (f[0], f[1])
    if k == "g":
        order = ["pt", "fil", "fill", "filll"][int(f[2])]
        return "\\vskip %spt plus %s%s minus %spt " % (f[0], f[1], order, f[3])
    if k == "k":
        return "\\kern%spt " % f[0]
    if k == "p":
        return "\\penalty%s " % f[0]
    if k == "P":
        return "\\par "
    if k == "i":
        head, body = item[1:].split(":", 1)
        n, smd, fp = head.split(",")
        return "\\splitmaxdepth=%spt \\floatingpenalty=%s \\insert%s{%s}" % (smd, fp, n, "".join(item_tex(i) for i in body.split("/")))
    sys.exit("item " + item)


def setting_tex(s):
    k, v = s.split("=")
    if k == "vsize":
        return "\\vsize=" + tex_dim(v)
    if k == "maxdepth":
        return "\\maxdepth=" + tex_dim(v)
    if k == "topskip":
        return "\\topskip=" + tex_dim(v)
    if k[0] == "C":
        return "\\count%s=%s " % (k[1:], v)
    if k[0] == "D":
        return "\\dimen%s=%s" % (k[1:], tex_dim(v))
    if k[0] == "S":
        w, st, order, sh = v.split(",")
        return "\\skip%s=%spt plus %s%s minus %spt " % (k[1:], w, st, ["pt", "fil", "fill", "filll"][int(order)], sh)
    if k[0] == "B":
        h, d = v.split(",")
        return "\\setbox%s=\\vbox{\\hrule height %spt depth %spt width 0pt}" % (k[1:], h, d)
    sys.exit("setting " + s)


def page_row(row, tmp):
    name, settings, items = row
    src = PREAMBLE + "\\maxdepth=\\maxdimen \\boxmaxdepth=\\maxdimen\n"
    src += "".join(setting_tex(s) for s in settings.split()) + "\n"
    src += "\\output={\\errmessage{OUTPUT FIRED}\\setbox0\\box255 \\deadcycles=0 }\n"
    src += "\\tracingpages=1\n"
    src += "".join(item_tex(i) for i in items.split()) + "\n"
    src += "\\tracingpages=0 \\edef\\y{\\number\\lastpenalty,\\number\\lastkern,\\number\\lastskip,\\number\\lastnodetype}\\immediate\\write-1{ENDCASE \\y}\n\\end\n"
    log = run_oracle(src, tmp)
    head, sep, tail = log.partition("ENDCASE ")
    if not sep or "OUTPUT FIRED" in head or "\n! " in head:
        sys.exit("page %s: %s" % (name, "|".join(l for l in head.split("\n") if l.startswith(("%", "!")))))
    lines = [l for l in head.split("\n") if l.startswith("%")]
    if not lines and name != "ends_in_discarded_glue" and name != "ends_in_discarded_penalty":
        sys.exit("page %s has no trace" % name)
    if any(len(l) >= 79 for l in lines):
        sys.exit("page %s: a line may have wrapped" % name)
    return [name, settings, items, "|".join(lines), tail.split("\n")[0]]


OPS = {"LIG": 0, "LIG/": 1, "/LIG": 2, "/LIG/": 3, "LIG/>": 5, "/LIG>": 6, "/LIG/>": 7, "/LIG/>>": 11}

# A row: name, program, word, the first j (0 needs a character of another font before the word), and whether the word's bchar is the
# font's (N: non_char).  In a program, | is a boundary character: as first, the left boundary's program, as next, the right boundary.
RECON = [
    ("no_program", "", "abcd", 1, "N"),
    ("lig", "a b LIG x", "abcd", 1, "N"),
    ("lig_mid", "b c LIG x", "abcd", 1, "N"),
    ("lig_then_lig", "a b LIG x;x c LIG y", "abcd", 1, "N"),
    ("lig_keep_right", "a b LIG/ x", "abcd", 1, "N"),
    ("lig_keep_left", "a b /LIG x", "abcd", 1, "N"),
    ("lig_keep_both", "a b /LIG/ x", "abcd", 1, "N"),
    ("lig_keep_right_move", "a b LIG/> x", "abcd", 1, "N"),
    ("lig_keep_left_move", "a b /LIG> x", "abcd", 1, "N"),
    ("lig_keep_both_move", "a b /LIG/> x", "abcd", 1, "N"),
    ("lig_keep_both_move2", "a b /LIG/>> x", "abcd", 1, "N"),
    ("keep_right_again", "a b LIG/ x;x b LIG y", "abcd", 1, "N"),
    ("keep_left_again", "a b /LIG x;a x LIG y", "abcd", 1, "N"),
    ("keep_left_kern", "a b /LIG x;a x KRN 1", "abcd", 1, "N"),
    ("keep_both_again", "a b /LIG/ x;a x LIG y;y b LIG q", "abcd", 1, "N"),
    ("keep_both_kerns", "a b /LIG/ x;a x KRN 1;x b KRN 2", "abcd", 1, "N"),
    ("keep_both_twice", "a b /LIG/ x;a x /LIG/ y;a y KRN 3", "abcd", 1, "N"),
    ("keep_both_move_chain", "a b /LIG/> x;x b LIG y", "abcd", 1, "N"),
    ("keep_both_move2_chain", "a b /LIG/>> x;x b LIG y;b c KRN 1", "abcd", 1, "N"),
    ("lig_then_keep_left", "a b LIG x;x c /LIG y", "abcd", 1, "N"),
    ("kern", "a b KRN 1", "abcd", 1, "N"),
    ("kerns", "a b KRN 1;b c KRN 2", "abcd", 1, "N"),
    ("second_step", "a q KRN 5;a r LIG y;a b KRN 1", "abcd", 1, "N"),
    ("second_step_lig", "a q KRN 5;a b LIG x", "abcd", 1, "N"),
    ("kern_after_lig", "a b LIG x;x c KRN 1", "abcd", 1, "N"),
    ("longer_word", "a b LIG x;c d KRN 1;e g LIG y;y h /LIG/ q", "abcdeghij", 1, "N"),
    ("left_boundary_kern", "| a KRN 1", "abcd", 0, "N"),
    ("left_boundary_lig", "| a LIG x", "abcd", 0, "N"),
    ("left_boundary_keep_right", "| a LIG/ x", "abcd", 0, "N"),
    ("left_boundary_keep_left", "| a /LIG x", "abcd", 0, "N"),
    ("left_boundary_keep_both", "| a /LIG/ x", "abcd", 0, "N"),
    ("right_boundary_kern", "d | KRN 1", "abcd", 1, "F"),
    ("right_boundary_lig", "d | LIG/ x", "abcd", 1, "F"),
    ("right_boundary_two", "b c KRN 2;d | LIG/ x", "abcd", 1, "F"),
]


def recon_pl(program):
    steps = [x.split() for x in program.split(";") if x]
    out = ["(FAMILY TST)", "(DESIGNSIZE R 10.0)"]
    if any(x[1] == "|" for x in steps):
        out.append("(BOUNDARYCHAR C z)")
    out.append("(FONTDIMEN (SLANT R 0.0) (SPACE R 1.0) (STRETCH R 0.5) (SHRINK R 0.3) (XHEIGHT R 1.0) (QUAD R 1.0))")
    if steps:
        out.append("(LIGTABLE")
        firsts = []
        for x in steps:
            if x[0] not in firsts:
                firsts.append(x[0])
        for f in firsts:
            out.append("   (LABEL BOUNDARYCHAR)" if f == "|" else "   (LABEL C %s)" % f)
            for x in steps:
                if x[0] != f:
                    continue
                nxt = "C z" if x[1] == "|" else "C " + x[1]
                if x[2] == "KRN":
                    out.append("   (KRN %s R %s.0)" % (nxt, x[3]))
                else:
                    out.append("   (%s %s C %s)" % (x[2], nxt, x[3]))
            out.append("   (STOP)")
        out.append("   )")
    for c in "abcdeghijklmnopqrstuvwxyz-":
        out.append("(CHARACTER C %s (CHARWD R 1.0) (CHARHT R 1.0))" % c)
    return "\n".join(out) + "\n"


def recon_token(line):
    m = re.fullmatch(r"\\x (\S) \(ligature ([^)]*)\)", line)
    if m:
        return "%s(%s)" % (m.group(1), m.group(2))
    m = re.fullmatch(r"\\x (\S)", line)
    if m:
        return m.group(1)
    m = re.fullmatch(r"\\kern ?(\d+)\.0", line)
    if m:
        return "k" + m.group(1)
    sys.exit("recon line " + line)


def recon_row(row, tmp):
    name, program, word, j0, bchar = row
    with open(os.path.join(tmp, "rc.pl"), "w") as f:
        f.write(recon_pl(program))
    r = subprocess.run(["pltotf", "rc.pl", "rc.tfm"], cwd=tmp, capture_output=True, text=True, check=False)
    if r.returncode != 0:
        sys.exit("pltotf on %s: %s%s" % (name, r.stdout, r.stderr))
    src = PREAMBLE + r"""\font\x=rc \font\y=cmr10 \hyphenchar\x=`\- \x
\lefthyphenmin=1 \righthyphenmin=1 \hsize=1000pt \parindent=0pt \parfillskip=0pt \pretolerance=-1 \tolerance=-1 \hbadness=10000 \linepenalty=10 \hyphenpenalty=50
\showboxdepth=100 \showboxbreadth=1000 \tracingonline=1
\hyphenation{%s-%s}
\setbox0=\vbox{\hskip0pt %s%s\par}\showbox0
\end
""" % (word[:-1], word[-1], "{\\y(}" if j0 == 0 else "", word)
    log = run_oracle(src, tmp)
    m = re.search(r"^> \\box0=\n\\vbox.*\n\.\\hbox.*\n((?:\.\..*\n)+)", log, re.M)
    if not m:
        sys.exit("recon %s: %s" % (name, log[-800:]))
    lines = m.group(1).rstrip("\n").split("\n")
    start = lines.index("..\\glue 0.0") + 1
    if j0 == 0:
        if lines[start] != "..\\y (":
            sys.exit("recon %s: %s" % (name, lines))
        start += 1
    prefix, pre, after, state = [], [], [], 0
    for l in lines[start:]:
        if l.startswith("..\\discretionary"):
            if "replacing" in l or state:
                sys.exit("recon %s: not a plain discretionary: %s" % (name, lines))
            state = 1
        elif l.startswith("..|"):
            sys.exit("recon %s has a post_break: %s" % (name, lines))
        elif l.startswith("..."):
            pre.append(recon_token(l[3:]))
        elif l == "..\\penalty 10000" or l.startswith("..\\glue"):
            break
        elif state:
            after.append(recon_token(l[2:]))
        else:
            prefix.append(recon_token(l[2:]))
    if not state:
        sys.exit("recon %s: no discretionary: %s" % (name, lines))
    return [name, program, word, j0, bchar, " ".join(prefix), " ".join(pre), " ".join(after)]


def write(name, rows):
    with open(os.path.join(OUT, name), "wb") as f:
        for r in rows:
            f.write(("\t".join(str(x) for x in r) + "\n").encode("latin-1"))
    print(name, len(rows))


def main():
    with tempfile.TemporaryDirectory() as tmp:
        write("hyph_trie_oracle.tsv", [dump_row(p, tmp) for p in PROGRAMS])
        write("hyph_words_oracle.tsv", word_rows(tmp))
        write("hyph_recon_oracle.tsv", [recon_row(r, tmp) for r in RECON])
        write("page_oracle.tsv", [page_row(r, tmp) for r in PAGES])


if __name__ == "__main__":
    main()
