#!/usr/bin/env python3
"""Expected results for linebreak.rs's search for breakpoints, taken from the oracle.

Run with python3 -I.  It writes crates/engine/testdata/linebreak_oracle.tsv.

Each case is a paragraph in a small language that this script turns into TeX and the test in linebreak.rs turns
into nodes by hand, so that no TFM is needed: boxes, rules, glue, kerns, penalties, math nodes, discretionaries and
characters of cmtt10, which has neither kerns nor ligatures among the letters.

Items, separated by a space:

  b5          \\hbox to 5pt{}                      v5     \\vbox{\\hrule width 5pt}
  r5          \\vrule width 5pt                    k5     \\kern 5pt
  g5,1,2      \\hskip 5pt plus 1pt minus 2pt       l5     \\leaders\\hrule\\hskip 5pt
  g0,1fil,0   the same with an infinite order      p50    \\penalty 50
  $           math shift, an even number of them around glue, kerns and penalties only
  L, E        \\beginL, \\endL
  cabc        the characters abc; \\hyphenchar is -1 unless the parameter hyphenchar says otherwise
  x, a, w     \\mark{}, \\vadjust{}, \\special{}
  d(A|B|C)    \\discretionary{A}{B}{C}, where A, B, C are items separated by commas

Parameters are NAME=VALUE: integers, dimensions in pt, glue in the notation of g, or for parshape i,w;i,w.

Format: fields separated by a tab.  A newline inside a field is byte 0x0C.

  #char_width    the width of each character of cmtt10, in sp
  name  parameters  list  \\prevgraf after the paragraph  what \\tracingparagraphs printed, up to its blank line
"""
import os
import re
import subprocess
import sys
import tempfile

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
OUT = os.path.join(ROOT, "crates", "engine", "testdata", "linebreak_oracle.tsv")

PREAMBLE = r"""\catcode`\{=1 \catcode`\}=2 \catcode`\#=6 \catcode`\^=7 \catcode`\_=8
\catcode`\&=4 \catcode`\$=3 \catcode`\~=13
\scrollmode \tracingonline=0 \newlinechar=-1 \errorcontextlines=0
\hbadness=10000 \vbadness=10000 \hfuzz=16383pt \vfuzz=16383pt \TeXXeTstate=1
\font\xx=cmtt10 \font\yy=cmmi10 \font\zz=cmsy10 \font\ww=cmex10
\textfont0=\xx \scriptfont0=\xx \scriptscriptfont0=\xx
\textfont1=\yy \scriptfont1=\yy \scriptscriptfont1=\yy
\textfont2=\zz \scriptfont2=\zz \scriptscriptfont2=\zz
\textfont3=\ww \scriptfont3=\ww \scriptscriptfont3=\ww
\hyphenchar\xx=-1 \xx \tracingparagraphs=1
\setbox0\hbox{a}\immediate\write-1{=char_width \number\wd0 }
"""


def words(n, w=20, g="5,2,1"):
    return " ".join(["b%d g%s" % (w, g)] * n) + " b%d" % w


def varied(n):
    """Widths that differ from word to word, as text does, without a random generator."""
    return " ".join("b%d g5,2,1" % (10 + (k * 7) % 23) for k in range(n)) + " b12"


LOOSE = "hsize=100 tolerance=10000 pretolerance=-1"
HYPH = "hsize=100 tolerance=10000 pretolerance=-1 hyphenpenalty=50 exhyphenpenalty=75"
NARROW = "parshape=200,0;90,0;200,0;90,0;200,0"

CASES = [
    # §1039: which passes run
    ("one_line", "hsize=100 tolerance=200 pretolerance=100", "b20 g5,2,1 b20"),
    ("first_pass_enough", "hsize=100 tolerance=200 pretolerance=10000", words(7)),
    ("first_pass_fails", "hsize=100 tolerance=10000 pretolerance=0", words(7)),
    ("no_first_pass", LOOSE, words(7)),
    ("emergency_pass", "hsize=100 tolerance=0 pretolerance=0 emergencystretch=50", words(7)),
    ("emergency_not_enough", "hsize=100 tolerance=0 pretolerance=-1 emergencystretch=1", words(7)),
    ("tolerance_above_inf_bad", "hsize=100 tolerance=100000 pretolerance=20000", words(7)),
    ("negative_tolerance", "hsize=100 tolerance=-5 pretolerance=-5", words(7)),
    # §1030: forced breaks, artificial demerits
    ("overfull_one_box", LOOSE, "b300"),
    ("overfull_then_more", LOOSE, "b300 g5,0,0 b20 g5,0,0 b300 g5,0,0 b20"),
    ("overfull_tolerance_0", "hsize=100 tolerance=0 pretolerance=-1", "b300 g5,0,0 b20 g5,0,0 b300"),
    ("nothing_fits_tight", "hsize=10 tolerance=10000 pretolerance=-1", words(3)),
    ("zero_hsize", "hsize=0 tolerance=10000 pretolerance=-1", words(3)),
    ("negative_hsize", "hsize=-50 tolerance=10000 pretolerance=-1", words(3)),
    # §1028, §1029: badness and fitness
    ("infinite_stretch", LOOSE, "b20 g0,1fil,0 b20 g5,1fill,0 b20 g5,1filll,0 b20 g90,0,0 b20"),
    ("stretch_exact", "hsize=100 tolerance=100 pretolerance=-1", "b45 g5,5,0 b40 g5,5,0 b45"),
    ("shrink_exact", "hsize=100 tolerance=100 pretolerance=-1", "b55 g5,0,5 b45 g5,0,5 b95 g5,0,5 b5"),
    ("loose_fit_bounds", LOOSE, "b10 g5,100,0 b10 g5,1,0 b50 g5,100,0 b10 g5,1,0 b10 g5,1000,0 b50"),
    ("big_shortfall_small_stretch", "hsize=200 tolerance=10000 pretolerance=-1", "b1 g1,0,0 b1 g1,1,0 b1 g1,20,0 b1"),
    ("negative_stretch", LOOSE, "b40 g5,-3,0 b40 g5,3,0 b40"),
    ("negative_shrink", LOOSE, "b100 g0,0,-3 b100 g0,0,3 b100"),
    # §1035: demerits
    ("line_penalty", "hsize=100 tolerance=10000 pretolerance=-1 linepenalty=500", words(8)),
    ("line_penalty_negative", "hsize=100 tolerance=10000 pretolerance=-1 linepenalty=-500", words(8)),
    ("line_penalty_huge", "hsize=100 tolerance=10000 pretolerance=-1 linepenalty=2147483647", words(8)),
    ("adj_demerits", "hsize=100 tolerance=10000 pretolerance=-1 adjdemerits=10000", "b95 g5,100,0 b20 g5,2,0 b95 g5,100,0 b95"),
    ("adj_demerits_negative", "hsize=100 tolerance=10000 pretolerance=-1 adjdemerits=-10000", "b95 g5,100,0 b20 g5,2,0 b95 g5,100,0 b95"),
    ("adj_demerits_huge", "hsize=100 tolerance=10000 pretolerance=-1 adjdemerits=2147483647", "b95 g5,100,0 b20 g5,2,0 b95 g5,100,0 b95"),
    ("demerits_above_2_31", "hsize=100 tolerance=10000 pretolerance=-1 adjdemerits=2147483647 linepenalty=9000 doublehyphendemerits=2147483647", "b95 d(b5||) b5 g5,100,0 b20 d(b5||) g5,2,0 b95 g5,100,0 b95"),
    ("demerits_far_below", "hsize=100 tolerance=10000 pretolerance=-1 adjdemerits=-1000000000", "b95 g5,100,0 b20 g5,2,0 b95 g5,100,0 b95"),
    # §1007 and penalties
    ("penalties", LOOSE, "b40 p50 g5,5,0 b40 p-50 g5,5,0 b40 p9999 g5,5,0 b40 p-9999 g5,5,0 b40 p10000 g5,5,0 b40 p-10000 g5,5,0 b40"),
    ("penalty_only_breaks", LOOSE, "b60 p0 b60 p100 b60 p-100 b60 p-20000 b60 p20000 b60"),
    ("forced_breaks", LOOSE, "b20 p-10000 b20 p-10000 p-10000 b20 g5,0,0 p-10000 b20"),
    # §1013, §1014: what is discarded after a break
    ("discard_after_break", LOOSE, "b60 g5,1,1 p10 g5,1,1 k3 g2,1,1 $ $ g5,1,1 b60 g5,1,1 k-3 p0 b60"),
    ("math_surround", "hsize=100 tolerance=10000 pretolerance=-1 mathsurround=3", "b60 $ $ g5,1,1 b60 $ g5,1,1 $ b60"),
    ("no_break_in_math", LOOSE, "b60 $ g5,1,1 k1 g5,1,1 p-10000 g5,1,1 $ g5,1,1 b60"),
    ("kern_before_glue", LOOSE, "b60 k2 g5,1,1 b60 k0 g5,1,1 b60 k2 b60"),
    ("leading_glue", LOOSE, "g5,1,1 p0 g5,1,1 b60 g5,1,1 b60"),
    ("LR_nodes", LOOSE, "b60 L g5,1,1 b60 E g5,1,1 b60 L E g5,1,1 b60"),
    ("other_nodes", LOOSE, "v30 x g5,1,1 r30 a g5,1,1 b30 w g5,1,1 w b60 x a w g5,1,1 b60"),
    ("rule_and_vbox_after_glue", LOOSE, "r60 g5,1,1 v60 g5,1,1 r60 p0 v60"),
    # §992: the end of the list
    ("ends_in_glue", LOOSE, "b60 g5,1,1 b60 g5,1,1"),
    ("ends_in_leaders", LOOSE, "b60 g5,1,1 b60 l5"),
    ("ends_in_kern", LOOSE, "b60 g5,1,1 b60 k5"),
    ("ends_in_penalty", LOOSE, "b60 g5,1,1 b60 p-10000"),
    ("par_fill_skip", "hsize=100 tolerance=10000 pretolerance=-1 parfillskip=0,1fil,0", words(6)),
    ("par_fill_skip_wide", "hsize=100 tolerance=10000 pretolerance=-1 parfillskip=50,0,0", words(6)),
    # §1003: background
    ("left_right_skip", "hsize=100 tolerance=10000 pretolerance=-1 leftskip=10,5,2 rightskip=10,5,2", words(7)),
    ("left_skip_fil", "hsize=100 tolerance=10000 pretolerance=-1 leftskip=0,1fil,0 rightskip=0,1fil,0", words(7)),
    ("left_skip_orders", "hsize=100 tolerance=10000 pretolerance=-1 leftskip=0,1fil,0 rightskip=0,1fill,0", words(7)),
    ("skip_negative", "hsize=100 tolerance=10000 pretolerance=-1 leftskip=-10,0,0 rightskip=-20,0,0", words(7)),
    # §1024-§1026: line lengths
    ("hang_positive_after", "hsize=100 tolerance=10000 pretolerance=-1 hangindent=30 hangafter=2", words(8)),
    ("hang_positive_after0", "hsize=100 tolerance=10000 pretolerance=-1 hangindent=30 hangafter=0", words(8)),
    ("hang_negative_after", "hsize=100 tolerance=10000 pretolerance=-1 hangindent=30 hangafter=-2", words(8)),
    ("hang_negative_indent", "hsize=100 tolerance=10000 pretolerance=-1 hangindent=-30 hangafter=2", words(8)),
    ("hang_negative_both", "hsize=100 tolerance=10000 pretolerance=-1 hangindent=-30 hangafter=-2", words(8)),
    ("parshape_two", "tolerance=10000 pretolerance=-1 parshape=10,50;0,150", words(8)),
    ("parshape_one", "tolerance=10000 pretolerance=-1 parshape=0,70", words(8)),
    ("parshape_short_long", "tolerance=10000 pretolerance=-1 " + NARROW, varied(12)),
    ("parshape_and_prevgraf", "tolerance=10000 pretolerance=-1 prevgraf=3 " + NARROW, varied(12)),
    ("hang_and_prevgraf", "hsize=100 tolerance=10000 pretolerance=-1 prevgraf=3 hangindent=30 hangafter=5", words(8)),
    ("prevgraf_past_parshape", "tolerance=10000 pretolerance=-1 prevgraf=9 " + NARROW, words(8)),
    # §1051: looseness
    ("looseness_plus", "hsize=100 tolerance=10000 pretolerance=-1 looseness=1", words(6, 20, "5,10,3")),
    ("looseness_plus_big", "hsize=100 tolerance=10000 pretolerance=-1 looseness=100", words(6, 20, "5,10,3")),
    ("looseness_minus", "hsize=100 tolerance=10000 pretolerance=-1 looseness=-1", words(6, 20, "5,10,3")),
    ("looseness_minus_big", "hsize=100 tolerance=10000 pretolerance=-1 looseness=-100", words(6, 20, "5,10,3")),
    ("looseness_first_pass", "hsize=100 tolerance=10000 pretolerance=10000 looseness=1", words(6, 20, "5,10,3")),
    ("looseness_retry", "hsize=100 tolerance=10000 pretolerance=100 looseness=-1", words(6, 20, "5,10,3")),
    ("looseness_emergency", "hsize=100 tolerance=200 pretolerance=100 emergencystretch=30 looseness=1", words(6, 20, "5,10,3")),
    ("looseness_long", "hsize=100 tolerance=1000 pretolerance=-1 looseness=2", varied(14)),
    # §1051's second branch: two ways to one more line, of which the later has fewer demerits
    ("looseness_same_lines", "hsize=100 tolerance=10000 pretolerance=-1 looseness=1 adjdemerits=100000000", "b30 g5,2,3 b45 g5,2,3 b30 g5,2,3 b20 g5,2,3 b5 g5,2,3 b25"),
    # §1012: many active nodes, deltas of all kinds
    ("many_active", "hsize=100 tolerance=10000 pretolerance=-1", varied(14)),
    ("many_active_adj_0", "hsize=100 tolerance=10000 pretolerance=-1 adjdemerits=0", varied(14)),
    ("many_active_adj_inf", "hsize=100 tolerance=10000 pretolerance=-1 adjdemerits=1073741823", varied(14)),
    ("deltas_and_fil", "hsize=100 tolerance=10000 pretolerance=-1", "b30 g5,1fil,1 b30 g5,1fill,1 b30 g5,1filll,1 b30 g5,1,1 " + varied(10)),
    ("negative_widths", "hsize=100 tolerance=10000 pretolerance=-1", "b80 g5,50,5 b80 g5,50,5 k-90 g5,50,5 b80 g5,50,5 k-90 b80 g5,50,5 b80"),
    ("negative_widths_2", LOOSE, "b50 g5,9,5 b40 g5,9,5 b20 g5,9,5 k-80 g5,9,5 b70 g5,9,5 b90 g5,9,5 b1 g5,9,5 k-120 p0 b90 g5,9,5 b90"),
    # §1045-§1047, §1016-§1018: discretionaries
    ("disc_empty", HYPH, "b60 d(||) b60 d(||) b60"),
    ("disc_pre", HYPH, "b60 d(b5||) b60 d(b5,k1||) b60"),
    ("disc_post", HYPH, "b60 d(|b5|) b60 d(|b5,r1|) b60"),
    ("disc_replace", HYPH, "b60 d(||b5) b60 d(||b5,k2,r3) b60"),
    ("disc_all", HYPH, "b60 d(b1|b2|b3) b60 d(b1,v2|r3,k4|b5,v6,r7,k8) b60"),
    ("disc_discards_after", HYPH, "b60 d(||b5) g5,1,1 k2 p0 g5,1,1 b60 d(b1||b5) g5,1,1 b60"),
    ("disc_post_keeps_after", HYPH, "b60 d(|b1|b5) g5,1,1 b60 d(b1|b2|b5) g5,1,1 k2 b60"),
    ("disc_chain", HYPH, "b45 d(b5||) b45 d(b5||) b45 d(b5||) b45 d(b5||) b45"),
    ("disc_double_hyphen", HYPH + " doublehyphendemerits=10000 finalhyphendemerits=5000", "b90 d(b5||) b90 d(b5||) b90 d(b5||) g5,50,0 b90 d(b5||)"),
    ("disc_final_hyphen", HYPH + " finalhyphendemerits=5000", "b90 d(b5||) b90 d(b5||) p-10000"),
    ("disc_replace_with_chars", HYPH, "b60 d(ca|cb|cc) b60 d(cab|cde|cfgh) b60"),
    ("disc_negative_penalty", "hsize=100 tolerance=10000 pretolerance=-1 hyphenpenalty=-50 exhyphenpenalty=-75", "b60 d(b5||) b60 d(||) b60"),
    ("disc_inf_penalty", "hsize=100 tolerance=10000 pretolerance=-1 hyphenpenalty=10000 exhyphenpenalty=-10000", "b60 d(b5||) b60 d(||) b60"),
    ("disc_two_adjacent", HYPH, "b60 d(b1|b2|) d(b3|b4|) b60"),
    ("disc_first", HYPH, "d(b1|b2|b3) b60 g5,1,1 b60"),
    ("disc_last", HYPH, "b60 g5,1,1 b60 d(b1|b2|b3)"),
    # §1043, §1071, §1073: characters, and hyphenation that stops at \hyphenchar<0
    ("chars", LOOSE, "cabc g5,1,1 cdefghijk g5,1,1 clmnopqrs g5,1,1 ctuvwxyz g5,1,1 cabcdefghijklmn"),
    ("chars_second_pass_uppercase", LOOSE, "cABC g5,1,1 cDEFGHIJK g5,1,1 cLMNOPQRS g5,1,1 cTUVWXYZ"),
    ("chars_after_nodes", LOOSE, "cabcdefgh k1 g5,1,1 k0 cabcdefgh w g5,1,1 w cabcdefgh g5,1,1 x cabcdefgh"),
    ("chars_with_digits", LOOSE, "c123 g5,1,1 c456abc g5,1,1 cabc g5,1,1 L E cabc g5,1,1 $ $ cabc"),
    ("hyph_word_then_box", LOOSE + " hyphenchar=45", "b60 g5,1,1 cabc b5 g5,1,1 cabc $ $ g5,1,1 cabcdefg r1 g5,1,1 ca1bc v1"),
    ("hyph_too_short", LOOSE + " hyphenchar=45 lefthyphenmin=31 righthyphenmin=32", "b60 g5,1,1 cabc g5,1,1 cdef g5,1,1 c1ab g5,1,1 k0 w L cabcd"),
    ("hyph_mins_too_big", LOOSE + " hyphenchar=45 lefthyphenmin=40 righthyphenmin=40", "b60 g5,1,1 cabc g5,1,1 cdef"),
    ("hyph_mins_clamped", LOOSE + " hyphenchar=45 lefthyphenmin=100 righthyphenmin=-3", "b60 g5,1,1 cabc g5,1,1 cdef"),
    ("hyph_char_too_big", LOOSE + " hyphenchar=256", "b60 g5,1,1 cabc g5,1,1 cdef"),
    ("hyph_uppercase_refused", LOOSE + " hyphenchar=45", "b60 g5,1,1 cAbc g5,1,1 c1Xabc g5,1,1 w cAbc"),
    ("hyph_uppercase_allowed", LOOSE + " hyphenchar=45 uchyph=1 lefthyphenmin=31 righthyphenmin=32", "b60 g5,1,1 cAbc g5,1,1 c1Xabc"),
    # Section 11: sums past 2^31sp, where the oracle's arithmetic is 64 bits wide.  The test can only ask of these what the
    # port's clamped sums still give, so nothing here discards more than 2^31sp after a break, or subtracts it again
    ("wide_unbreakable", LOOSE, "b16000 b16000 b16000"),
    ("wide_then_text", LOOSE, "b16000 b16000 b16000 g5,1,1 b60 g5,1,1 b60 g5,1,1 b60"),
    ("wide_rules_kerns", LOOSE, "r16000 k16000 k16000 r16000 g5,0,0 b60"),
    ("wide_negative", LOOSE, "k-16000 k-16000 k-16000 g5,0,0 b60 g5,0,0 b60"),
    ("wide_stretch", LOOSE, "b10 g0,16000,0 b10 p10000 g0,16000,0 b10 p10000 g0,16000,0 b10"),
    ("wide_shrink", LOOSE, "b400 g0,0,16000 b10 p10000 g0,0,16000 b10 p10000 g0,0,16000 b10"),
    ("wide_shortfall", "hsize=16000 tolerance=10000 pretolerance=-1", "k-16000 g5,1,0 k-16000 g5,1,0 b10"),
    ("wide_disc", HYPH, "b60 d(b16000,b16000,b16000||) b60 g5,1,1 b60"),
    ("big_discard", LOOSE, "b60 g8000,0,0 p0 g8000,0,0 p0 g8000,0,0 b60 g5,1,1 b60"),
]

def glue(s):
    w, st, sh = s.split(",")

    def part(x):
        return x if x.endswith(("fil", "fill", "filll")) else x + "pt"

    return "%spt plus %s minus %s" % (w, part(st), part(sh))


def list_tex(s, sep=" "):
    out = []
    math = False
    for x in s.split(sep):
        if x == "$":
            # \relax keeps two math shifts in a row from being a display
            out.append("$" if math else r"$\relax ")
            math = not math
        elif x:
            out.append(item_tex(x))
    return "".join(out)


def item_tex(x):
    k, v = x[0], x[1:]
    if k == "b":
        return r"\hbox to %spt{}" % v
    if k == "v":
        return r"\vbox{\hrule width %spt}" % v
    if k == "r":
        return r"\vrule width %spt " % v
    if k == "k":
        return r"\kern %spt " % v
    if k == "g":
        return r"\hskip %s " % glue(v)
    if k == "l":
        return r"\leaders\hrule\hskip %spt " % v
    if k == "p":
        return r"\penalty %s " % v
    if k == "L":
        return r"\beginL "
    if k == "E":
        return r"\endL "
    if k == "c":
        return v
    if k == "x":
        return r"\mark{}"
    if k == "a":
        return r"\vadjust{}"
    if k == "w":
        return r"\special{}"
    if k == "d":
        a, b, c = v[1:-1].split("|")
        return r"\discretionary{%s}{%s}{%s}" % (list_tex(a, ","), list_tex(b, ","), list_tex(c, ","))
    raise ValueError(x)


def param_tex(p):
    k, v = p.split("=")
    if k == "parshape":
        rows = v.split(";")
        return r"\parshape=%d %s " % (len(rows), " ".join("%spt %spt" % tuple(r.split(",")) for r in rows))
    if k in ("leftskip", "rightskip", "parfillskip"):
        return "\\%s=%s " % (k, glue(v))
    if k in ("hsize", "hangindent", "emergencystretch", "mathsurround"):
        return "\\%s=%spt " % (k, v)
    return "\\%s=%s " % (k, v)


def main():
    with tempfile.TemporaryDirectory() as d:
        src = [PREAMBLE]
        for name, params, para in CASES:
            src.append(r"\immediate\write-1{=begin %s}" % name)
            # \hyphenchar is not undone at the end of a group
            hc = [p[11:] for p in params.split() if p.startswith("hyphenchar=")]
            src.append(r"\hyphenchar\xx=%s" % (hc[0] if hc else "-1"))
            # \prevgraf reads as 0 inside \write
            # new_graf clears \prevgraf, so that one has to come in horizontal mode
            ps = params.split()
            src.append(r"\setbox1\vbox{%s\noindent %s%s\par\count255=\prevgraf \immediate\write-1{=end \the\count255}}" % (
                "".join(param_tex(p) for p in ps if not p.startswith(("prevgraf=", "hyphenchar="))),
                "".join(param_tex(p) for p in ps if p.startswith("prevgraf=")),
                list_tex(para)))
        src.append(r"\end")
        with open(os.path.join(d, "t.tex"), "w") as f:
            f.write("\n".join(src) + "\n")
        r = subprocess.run(["pdftex", "-ini", "-etex", "-interaction=nonstopmode", "*t.tex"], cwd=d,
                           stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, env=dict(os.environ, SOURCE_DATE_EPOCH="0"))
        with open(os.path.join(d, "t.log"), encoding="latin-1") as f:
            log = f.read()
        if r.returncode != 0 and "!" in log:
            sys.stderr.write(log[log.index("!") - 200:log.index("!") + 800])
            sys.exit("the oracle stopped on an error")
    width = re.search(r"=char_width (\d+)", log).group(1)
    rows = ["#char_width\t" + width]
    too_long = []
    for name, params, para in CASES:
        m = re.search(r"^=begin %s\n(.*?)^=end (\d+)$" % re.escape(name), log, re.S | re.M)
        if not m:
            sys.exit("no result for " + name)
        trace = m.group(1).split("\n\n")[0].rstrip("\n")
        if not trace or trace.startswith("!"):
            sys.exit("no trace for %s: %r" % (name, m.group(1)[:300]))
        if len(trace) >= 4000:
            too_long.append("%s has %d" % (name, len(trace)))
        rows.append("\t".join([name, params, para, m.group(2), trace.replace("\n", "\f")]))
    if too_long:
        sys.exit("the terminal buffer of the test keeps 4096 bytes: " + ", ".join(too_long))
    with open(OUT, "w", encoding="latin-1", newline="\n") as f:
        f.write("\n".join(rows) + "\n")
    print("%d cases in %s" % (len(CASES), OUT))


if __name__ == "__main__":
    main()
