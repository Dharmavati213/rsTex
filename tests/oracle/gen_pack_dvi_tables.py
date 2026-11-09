#!/usr/bin/env python3
"""Expected results for pack.rs and dvi.rs, taken from the oracle.

Run with python3 -I.  It writes crates/engine/testdata/pack_dvi_oracle.tsv.

Each case is one box, written in a small notation that the Rust kit in pack.rs reads too, and made by the real
engine from the TeX source this script turns it into.  The Rust test builds the same list by hand through the same
calls TeX makes (`append_to_vlist`, `hpack`, `vpackage`), ships it with `dvi_ship_out`, and compares.

    h [to N | spread N] { items }     \\hbox
    v [to N | spread N] { items }     \\vbox
    s N                               the next box is lowered, or moved right in a vbox, by N sp
    r W H D                           \\vrule, \\hrule in a vbox; each field a number, or * if absent
    k N                               \\kern
    g W ST SH                         \\hskip, \\vskip in a vbox.  ST and SH are N, or Nfil, Nfill, Nfilll
    L a|c|x W ST SH item              \\leaders, \\cleaders, \\xleaders with that item and that glue
    c F N                             \\char N in font F, x or e
    l F N A B                         the ligature N of the characters A, B in font F
    M N                               a math node of subtype N: 6 \\beginL, 7 \\endL, 10 \\beginR, 11 \\endR

Format: fields separated by a tab, one case per line.  A newline inside a field is byte 0x0C.

    @font  id  check bytes  design size in sp
    @char  id  code  width height depth in sp
    name  parameters  notation  line  what packaging printed  wd ht dp badness  what ship_out printed
          length and FNV-1a of the DVI up to post  max_v max_h max_push

The DVI itself is not kept, since a dump of it is what privacy_guard.py stops: its length and a 32-bit FNV-1a, up to its `post`,
with the preamble's date fixed by the preamble of the case.  `gen_pack_dvi_tables.py NAME` prints the bytes.
`noship` in the parameters: the box is not shipped, and the last three fields are then empty.
`notrace`: no `\\tracingoutput`, so what ship_out printed is empty.

    gen_pack_dvi_tables.py NAME...    prints those cases, and writes nothing
"""
import os
import struct
import subprocess
import sys
import tempfile

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
OUT = os.path.join(ROOT, "crates", "engine", "testdata", "pack_dvi_oracle.tsv")

# The case's line numbers depend on this: its five lines, the parameters on one line, and a marker.
PREAMBLE = r"""\catcode`\{=1 \catcode`\}=2 \catcode`\#=6 \catcode`\^=7 \catcode`\_=8 \catcode`\&=4 \catcode`\$=3
\scrollmode \tracingonline=0 \newlinechar=-1 \errorcontextlines=0 \pdfoutput=0 \def\space{ }
\showboxbreadth=1000 \showboxdepth=1000 \boxmaxdepth=1073741823sp \baselineskip=12pt
\year=2026 \month=11 \day=2 \time=754 \tracingoutput=1 \count0=1
\font\xx=cmr10 \font\yy=cmmi10 \font\ee=ecrm1000 \xx
"""
LINE = 8  # the line of \setbox1

PARAM = {
    "hbadness": r"\hbadness=%s ",
    "vbadness": r"\vbadness=%s ",
    "hfuzz": r"\hfuzz=%ssp ",
    "vfuzz": r"\vfuzz=%ssp ",
    "overfullrule": r"\overfullrule=%ssp ",
    "boxmaxdepth": r"\boxmaxdepth=%ssp ",
    "baselineskip": r"\baselineskip=%ssp ",
    "lineskip": r"\lineskip=%ssp ",
    "lineskiplimit": r"\lineskiplimit=%ssp ",
    "hoffset": r"\hoffset=%ssp ",
    "voffset": r"\voffset=%ssp ",
    "texxet": r"\TeXXeTstate=%s ",
    "etex": "",
    "dvibuf": "",
    "noship": "",
    "notrace": r"\tracingoutput=0 ",
}

FONT = {"x": r"\xx", "e": r"\ee"}
MATH = {"6": r"\beginL ", "7": r"\endL ", "10": r"\beginR ", "11": r"\endR "}


def glue_part(word, key):
    return " %s %s" % (key, word if word[-1] in "fl" else word + "sp")


class Tex:
    """The notation into TeX source."""

    def __init__(self, spec):
        self.t = spec.split()
        self.i = 0
        self.chars = set()

    def take(self):
        self.i += 1
        return self.t[self.i - 1]

    def rule(self, vert):
        w, h, d = self.take(), self.take(), self.take()
        s = r"\hrule" if vert else r"\vrule"
        for key, val in (("width", w), ("height", h), ("depth", d)):
            if val != "*":
                s += " %s %ssp" % (key, val)
        return s + r"\relax "

    def glue(self, vert):
        w, st, sh = self.take(), self.take(), self.take()
        s = (r"\vskip " if vert else r"\hskip ") + w + "sp"
        if st != "0":
            s += glue_part(st, "plus")
        if sh != "0":
            s += glue_part(sh, "minus")
        return s + r"\relax "

    def box(self):
        kind = self.take()
        s = r"\vbox" if kind == "v" else r"\hbox"
        if self.t[self.i] in ("to", "spread"):
            key = self.take()
            s += " %s %ssp" % (key, self.take())
        assert self.take() == "{"
        s += "{"
        vert = kind == "v"
        shift = None
        while self.t[self.i] != "}":
            tok = self.take()
            if tok == "s":
                shift = self.take()
            elif tok in ("h", "v"):
                self.i -= 1
                if shift is not None:
                    s += (r"\moveright " if vert else r"\lower ") + shift + "sp"
                    shift = None
                s += self.box()
            elif tok == "r":
                s += self.rule(vert)
            elif tok == "k":
                s += r"\kern %ssp " % self.take()
            elif tok == "g":
                s += self.glue(vert)
            elif tok == "L":
                prim = {"a": r"\leaders", "c": r"\cleaders", "x": r"\xleaders"}[self.take()]
                gl = self.glue(vert)
                if self.t[self.i] == "r":
                    self.i += 1
                    item = self.rule(vert)
                else:
                    item = self.box()
                s += prim + item + gl
            elif tok == "c":
                f, n = self.take(), self.take()
                self.chars.add((f, int(n)))
                s += r"%s\char%s " % (FONT[f], n)
            elif tok == "l":
                f, n, a, b = self.take(), self.take(), self.take(), self.take()
                self.chars.add((f, int(n)))
                s += r"%s\char%s\char%s " % (FONT[f], a, b)
            elif tok == "M":
                s += MATH[self.take()]
            else:
                raise ValueError(tok)
        self.i += 1
        return s + "}"


def case(name, spec, params=""):
    return (name, params, spec)


def kerns(values):
    return " ".join("k %d r 1 1 0" % v for v in values)


def rows():
    r = []
    # §833-§843: every way out of hpack, and its message.
    r += [
        case("h_empty", "h { }"),
        case("h_rule_dims", "h { r 10 20 30 r 5 40 1 }"),
        case("h_exact", "h to 10 { r 10 1 1 }"),
        case("h_stretch_loose", "h to 110 { r 100 1 1 g 0 100 0 }"),
        case("h_stretch_loose_quiet", "h to 110 { r 100 1 1 g 0 100 0 }", "hbadness=1"),
        case("h_stretch_loose_exactly_quiet", "h to 164 { r 100 1 1 g 0 100 0 }", "hbadness=26"),
        case("h_stretch_underfull", "h to 200 { r 100 1 1 g 0 100 0 }"),
        case("h_stretch_underfull_quiet", "h to 200 { r 100 1 1 g 0 100 0 }", "hbadness=100"),
        case("h_stretch_101", "h to 201 { r 100 1 1 g 0 100 0 }", "hbadness=100"),
        case("h_stretch_nothing", "h to 200 { r 100 1 1 }"),
        case("h_stretch_empty_list", "h to 200 { }"),
        case("h_stretch_fil", "h to 200 { g 0 1fil 0 g 0 100 0 }"),
        case("h_stretch_fill", "h to 200 { g 0 1fil 0 g 0 2fill 0 g 0 100 0 }"),
        case("h_stretch_filll", "h to 200 { g 0 1fil 0 g 0 2fill 0 g 0 3filll 0 }"),
        case("h_stretch_cancel", "h to 200 { g 0 3fil 0 g 0 -3fil 0 g 0 100 0 }"),
        case("h_stretch_cancel_all", "h to 200 { g 0 3fil 0 g 0 -3fil 0 }"),
        case("h_stretch_negative", "h to 200 { g 0 -100 0 r 1 1 1 }"),
        case("h_spread_stretch", "h spread 50 { r 10 1 1 g 0 100 0 }"),
        case("h_spread_shrink", "h spread -5 { r 10 1 1 g 0 0 100 }"),
        case("h_tight", "h to 95 { r 100 1 1 g 0 0 100 }"),
        case("h_tight_quiet", "h to 95 { r 100 1 1 g 0 0 100 }", "hbadness=1"),
        case("h_overfull", "h to 50 { r 100 1 1 g 0 0 10 }"),
        case("h_overfull_rule", "h to 50 { r 100 1 1 g 0 0 10 }", "overfullrule=65536"),
        case("h_overfull_fuzz", "h to 50 { r 100 1 1 g 0 0 10 }", "hfuzz=40"),
        case("h_overfull_fuzz_quiet", "h to 50 { r 100 1 1 g 0 0 10 }", "hfuzz=40,hbadness=100"),
        case("h_overfull_fuzz_rule_quiet", "h to 50 { r 100 1 1 g 0 0 10 }", "hfuzz=40,hbadness=100,overfullrule=65536"),
        case("h_overfull_exact_fuzz", "h to 50 { r 100 1 1 g 0 0 10 }", "hfuzz=39,overfullrule=65536"),
        case("h_overfull_no_glue", "h to 50 { r 100 1 1 }", "overfullrule=65536"),
        case("h_shrink_fil", "h to 50 { r 100 1 1 g 0 0 1fil g 0 0 200 }"),
        case("h_shrink_fill", "h to 50 { r 100 1 1 g 0 0 1fil g 0 0 2fill }"),
        case("h_shrink_filll", "h to 50 { r 100 1 1 g 0 0 1fil g 0 0 2fill g 0 0 3filll }"),
        case("h_shrink_cancel", "h to 50 { r 100 1 1 g 0 0 3fil g 0 0 -3fil g 0 0 200 }"),
        case("h_shrink_nothing_empty", "h to -5 { }"),
        case("h_shrink_negative_total", "h to 50 { r 100 1 1 g 0 0 -10 }"),
        case("h_shift", "h { s 5 h { r 1 10 20 } s -7 h { r 1 30 4 } }"),
        case("h_shift_negative_depth", "h { s 50 h { r 1 10 20 } s -70 h { r 1 30 40 } }"),
        case("h_rule_shift_ignored", "h { r 1 30 40 }"),
        case("h_leader_rule_dims", "h { L a 10 0 0 r 5 30 40 }"),
        case("h_leader_box_dims", "h { L a 10 0 0 h { r 5 30 40 } }"),
        case("h_chars", "h { c x 97 c x 113 c x 120 }"),
        case("h_chars_set1", "h { c e 128 c e 200 c e 97 }"),
        case("h_two_fonts", "h { c x 97 c e 97 c x 97 c e 200 }"),
        case("h_ligature", "h { l x 12 102 105 c x 97 }"),
        case("h_ligature_first", "h { l x 12 102 105 k 5 l x 12 102 105 }"),
        case("h_chars_underfull_short_display", "h to 2000000 { c x 97 k 5 c e 97 g 0 100 0 l x 12 102 105 r 1 1 1 h { r 1 1 1 } }"),
    ]
    # §848-§854: and of vpackage.
    r += [
        case("v_empty", "v { }"),
        case("v_rule_dims", "v { r 10 20 30 r 5 40 1 }"),
        case("v_exact", "v to 51 { r 10 20 30 r 5 1 0 }"),
        case("v_stretch_loose", "v to 110 { r 1 100 0 g 0 100 0 }"),
        case("v_stretch_quiet", "v to 110 { r 1 100 0 g 0 100 0 }", "vbadness=1"),
        case("v_stretch_underfull", "v to 200 { r 1 100 0 g 0 100 0 }"),
        case("v_stretch_nothing", "v to 200 { r 1 100 0 }"),
        case("v_stretch_empty", "v to 200 { }"),
        case("v_stretch_fil", "v to 200 { g 0 1fil 0 g 0 100 0 }"),
        case("v_stretch_fill", "v to 200 { g 0 1fil 0 g 0 2fill 0 }"),
        case("v_stretch_filll", "v to 200 { g 0 1fil 0 g 0 2fill 0 g 0 3filll 0 }"),
        case("v_stretch_cancel", "v to 200 { g 0 3fil 0 g 0 -3fil 0 g 0 100 0 }"),
        case("v_spread", "v spread 50 { r 1 100 0 g 0 100 0 }"),
        case("v_spread_shrink", "v spread -5 { r 1 100 0 g 0 0 100 }"),
        case("v_tight", "v to 95 { r 1 100 0 g 0 0 100 }"),
        case("v_tight_quiet", "v to 95 { r 1 100 0 g 0 0 100 }", "vbadness=1"),
        case("v_overfull", "v to 50 { r 1 100 0 g 0 0 10 }"),
        case("v_overfull_fuzz", "v to 50 { r 1 100 0 g 0 0 10 }", "vfuzz=40"),
        case("v_overfull_fuzz_quiet", "v to 50 { r 1 100 0 g 0 0 10 }", "vfuzz=40,vbadness=100"),
        case("v_overfull_no_glue", "v to 50 { r 1 100 0 }"),
        case("v_shrink_fil", "v to 50 { r 1 100 0 g 0 0 1fil g 0 0 200 }"),
        case("v_shrink_fill", "v to 50 { r 1 100 0 g 0 0 1fil g 0 0 2fill }"),
        case("v_shrink_filll", "v to 50 { r 1 100 0 g 0 0 3filll }"),
        case("v_shrink_empty", "v to -5 { }"),
        case("v_max_depth", "v { r 10 20 30 }", "boxmaxdepth=7"),
        case("v_max_depth_to", "v to 60 { r 10 20 30 }", "boxmaxdepth=7"),
        case("v_max_depth_negative", "v { r 10 20 30 }", "boxmaxdepth=-7"),
        case("v_max_depth_equal", "v { r 10 20 30 }", "boxmaxdepth=30"),
        case("v_glue_resets_depth", "v { r 10 20 30 g 5 0 0 r 10 20 30 }"),
        case("v_kern_resets_depth", "v { r 10 20 30 k 5 r 10 20 30 }"),
        case("v_box_keeps_depth", "v { h { r 10 20 30 } h { r 10 20 30 } }", "baselineskip=0,lineskiplimit=-1073741823"),
        case("v_shift_width", "v { s 5 h { r 10 1 0 } s -7 h { r 30 1 0 } }"),
        case("v_shift_width_negative", "v { s -50 h { r 10 1 0 } }"),
        case("v_rule_width", "v { r 10 1 0 r 30 1 0 }"),
        case("v_leader_width", "v { L a 10 0 0 r 50 1 0 }"),
        case("v_leader_box_width", "v { L a 10 0 0 h { r 50 1 0 } }"),
    ]
    # §855: both ways, and \prevdepth's boundary at the ignored value.
    r += [
        case("baseline", "v { h { r 1 10 2 } h { r 1 10 2 } }", "baselineskip=20"),
        case("baseline_exact_limit", "v { h { r 1 10 2 } h { r 1 10 2 } }", "baselineskip=20,lineskiplimit=8"),
        case("baseline_over_limit", "v { h { r 1 10 2 } h { r 1 10 2 } }", "baselineskip=20,lineskiplimit=9,lineskip=3"),
        case("baseline_negative", "v { h { r 1 10 2 } h { r 1 10 2 } }", "baselineskip=5,lineskip=3,lineskiplimit=-100"),
        case("baseline_after_rule", "v { h { r 1 10 2 } r 1 1 0 h { r 1 10 2 } }", "baselineskip=20"),
        case("baseline_after_kern", "v { h { r 1 10 2 } k 1 h { r 1 10 2 } }", "baselineskip=20"),
    ]
    # §652, §661: running dimensions; rules that do not show.
    r += [
        case("rule_running_h", "h { h { r 1 10 20 } r 5 * * r 5 7 * r 5 * 3 }"),
        case("rule_running_v", "v { h { r 10 1 0 } r * 5 5 r 3 5 5 }", "baselineskip=0,lineskiplimit=-1073741823"),
        case("rule_zero", "h { r 0 5 5 r 5 0 0 r 5 -5 5 r 5 5 -5 r -5 5 5 k 3 r 2 1 1 }"),
        case("rule_zero_v", "v { r 0 5 5 r 5 0 0 r 5 -5 5 r 5 5 -5 r -5 5 5 k 3 r 2 1 1 }"),
        case("rule_default", "h { r * * * }"),
        case("rule_default_v", "v { r * * * }"),
        case("rule_in_nested", "v { s 3 h { k 2 r 5 1 1 } k 9 s 3 h { k 2 r 5 1 1 } s 4 v { k 2 r 5 1 1 } }"),
    ]
    # §651, §660: boxes in boxes; void boxes; a push that is taken back.
    r += [
        case("void_in_h", "h to 200 { h to 5 { } v to 9 { } g 0 1fil 0 r 3 3 3 }"),
        case("void_in_v", "v { h to 7 { } v spread 7 { } k 3 r 2 2 2 }"),
        case("box_nothing_inside", "h { h { k 5 } r 3 1 1 h { g 5 0 0 } r 3 1 1 }"),
        case("box_only_kern_v", "v { v { k 5 } r 3 1 1 v { g 5 0 0 } r 3 1 1 }"),
        case("nest_push_kept", "h { h { r 3 1 1 } h { h { r 3 1 1 } } v { r 3 1 1 } }"),
        case("nest_deep", "h { h { h { h { h { h { r 3 1 1 } } } } } r 3 1 1 }"),
        case("nest_shift", "h { s 5 h { r 3 1 1 } s -5 h { r 3 1 1 } s 5 v { r 3 1 1 } s 9 v { h { r 3 1 1 } } }"),
    ]
    # §634-§641: movement's sizes, and the y, z, w, x it finds, as in the example of section 631 of the WEB.
    r += [
        case("move_sizes_right", "h { %s }" % kerns([1, 127, 128, 129, 32767, 32768, 8388607, 8388608, 16777216])),
        case("move_sizes_left", "h { r 1 1 0 %s }" % kerns([-2, -128, -129, -130, -32768, -32769, -32770, -8388608, -8388609, -16777217])),
        case("move_sizes_down", "v { %s }" % kerns([1, 127, 128, 129, 32767, 32768, 8388607, 8388608, 16777216])),
        case("move_sizes_up", "v { %s }" % kerns([-2, -128, -129, -130, -32768, -32769, -32770, -8388608, -8388609, -16777217])),
        case("move_digits_v", "v { %s }" % kerns([3, 1, 4, 1, 5, 9, 2, 6, 5, 3, 5, 8, 9])),
        case("move_digits_h", "h { %s }" % kerns([3, 1, 4, 1, 5, 9, 2, 6, 5, 3, 5, 8, 9])),
        case("move_repeat_v", "v { %s }" % kerns([7] * 6)),
        case("move_repeat_h", "h { %s }" % kerns([7] * 6)),
        case("move_two_v", "v { %s }" % kerns([7, 8] * 6)),
        case("move_three_h", "h { %s }" % kerns([7, 8, 9] * 5)),
        case("move_mixed_sizes", "v { %s }" % kerns([300, 40000, 300, 9000000, 40000, 300, 9000000])),
        case("move_negative_match", "h { %s }" % kerns([500, -700, 500, -700, -700, 500])),
        case("move_across_push", "v { %s h { %s } %s h { %s } %s }" % (kerns([7, 8]), kerns([7, 8, 7]), kerns([7, 8]), kerns([8, 7, 8]), kerns([7, 8]))),
        case("move_pruned", "h { h { %s } %s h { %s } }" % (kerns([7, 8, 9]), kerns([7, 8, 9]), kerns([7, 8, 9]))),
        case("move_y_then_z_v", "v { %s }" % kerns([1, 2, 3, 2, 1, 3, 2, 1, 3, 4, 3, 4, 1])),
        case("move_y_then_z_h", "h { %s }" % kerns([1, 2, 3, 2, 1, 3, 2, 1, 3, 4, 3, 4, 1])),
    ]
    # §653, §662: glue set, rounded on its running total, in each order; what doesn't match its order isn't stretched.
    r += [
        case("glue_h_stretch", "h to 1000 { g 10 100 0 r 1 1 1 g 10 200 0 r 1 1 1 g 10 300 0 r 1 1 1 }", "hbadness=10000"),
        case("glue_h_shrink", "h to 10 { g 10 0 100 r 1 1 1 g 10 0 200 r 1 1 1 g 10 0 300 r 1 1 1 }", "hbadness=10000"),
        case("glue_h_order_mismatch", "h to 1000 { g 10 100 0 r 1 1 1 g 10 1fil 0 r 1 1 1 g 10 2fil 0 r 1 1 1 g 10 0 5 r 1 1 1 }"),
        case("glue_h_shrink_order_mismatch", "h to 10 { g 100 100 3 r 1 1 1 g 10 0 1fil r 1 1 1 g 10 0 2fil r 1 1 1 }"),
        case("glue_v_stretch", "v to 1000 { g 10 100 0 r 1 1 0 g 10 200 0 r 1 1 0 g 10 300 0 r 1 1 0 }", "vbadness=10000"),
        case("glue_v_shrink", "v to 10 { g 10 0 100 r 1 1 0 g 10 0 200 r 1 1 0 g 10 0 300 r 1 1 0 }", "vbadness=10000"),
        case("glue_v_order_mismatch", "v to 1000 { g 10 100 0 r 1 1 0 g 10 1fil 0 r 1 1 0 g 10 2fil 0 r 1 1 0 }"),
        case("glue_h_compat", "h to 1000 { g 10 100 0 r 1 1 1 g 10 200 0 r 1 1 1 g 10 300 0 r 1 1 1 }", "hbadness=10000,etex=0"),
        case("glue_v_compat", "v to 1000 { g 10 100 0 r 1 1 0 g 10 200 0 r 1 1 0 g 10 300 0 r 1 1 0 }", "vbadness=10000,etex=0"),
        case("glue_huge_ratio", "h to 16000000 { g 0 1 0 r 1 1 1 g 0 1 0 r 1 1 1 g 0 1 0 }", "hbadness=10000"),
        case("glue_billion", "h to 1073741823 { g 0 1 0 r 1 1 1 g 0 1 0 r 1 1 1 }", "hbadness=10000"),
        case("glue_billion_v", "v to 1073741823 { g 0 1 0 r 1 1 0 g 0 1 0 r 1 1 0 }", "vbadness=10000"),
    ]
    # §654-§656, §663-§665: leaders.
    box_h = "h { r 7 2 1 }"
    box_v = "v { r 7 2 1 }"
    r += [
        case("leaders_rule_h", "h { r 3 1 1 L a 20 0 0 r * 5 5 r 3 1 1 }"),
        case("leaders_rule_h_running", "h { r 3 9 9 L a 20 0 0 r * * * r 3 1 1 }"),
        case("leaders_rule_h_glue", "h to 100 { r 3 1 1 L a 20 1fil 0 r * 5 5 r 3 1 1 }"),
        case("leaders_rule_v", "v { r 3 1 0 L a 20 0 0 r 5 * * r 3 1 0 }"),
        case("leaders_rule_v_running", "v { r 30 1 0 L a 20 0 0 r * * * r 3 1 0 }"),
        case("leaders_rule_v_glue", "v to 100 { r 3 1 0 L a 20 1fil 0 r 5 * * r 3 1 0 }"),
        case("leaders_a_h", "h { r 3 1 1 L a 100 0 0 %s r 3 1 1 }" % box_h),
        case("leaders_c_h", "h { r 3 1 1 L c 100 0 0 %s r 3 1 1 }" % box_h),
        case("leaders_x_h", "h { r 3 1 1 L x 100 0 0 %s r 3 1 1 }" % box_h),
        case("leaders_a_h_exact", "h { L a 70 0 0 %s r 3 1 1 }" % box_h),
        case("leaders_c_h_exact", "h { L c 70 0 0 %s r 3 1 1 }" % box_h),
        case("leaders_x_h_exact", "h { L x 70 0 0 %s r 3 1 1 }" % box_h),
        case("leaders_a_h_too_small", "h { r 1 1 1 L a 5 0 0 %s r 3 1 1 }" % box_h),
        case("leaders_x_h_too_small", "h { r 1 1 1 L x 5 0 0 %s r 3 1 1 }" % box_h),
        case("leaders_zero_width_box", "h { r 3 1 1 L a 50 0 0 h { k 0 } r 3 1 1 }"),
        case("leaders_zero_glue", "h { r 3 1 1 L a 0 0 0 %s r 3 1 1 }" % box_h),
        case("leaders_negative_glue", "h { r 3 1 1 L a -5 0 0 %s r 3 1 1 }" % box_h),
        case("leaders_void_box", "h { r 3 1 1 L a 50 0 0 h to 7 { } r 3 1 1 }"),
        case("leaders_vbox_in_h", "h { r 3 1 1 L a 50 0 0 %s r 3 1 1 }" % box_v),
        case("leaders_stretched_h", "h to 100 { L a 0 1fil 0 %s r 3 1 1 L x 0 1fil 0 %s L c 0 1fil 0 %s }" % (box_h, box_h, box_h)),
        case("leaders_a_v", "v { r 3 1 0 L a 100 0 0 %s r 3 1 0 }" % box_v),
        case("leaders_c_v", "v { r 3 1 0 L c 100 0 0 %s r 3 1 0 }" % box_v),
        case("leaders_x_v", "v { r 3 1 0 L x 100 0 0 %s r 3 1 0 }" % box_v),
        case("leaders_a_v_exact", "v { L a 30 0 0 %s r 3 1 0 }" % box_v),
        case("leaders_x_v_too_small", "v { r 1 1 0 L x 2 0 0 %s r 3 1 0 }" % box_v),
        case("leaders_zero_height_box", "v { r 3 1 0 L a 50 0 0 v { k 0 } r 3 1 0 }"),
        case("leaders_hbox_in_v", "v { r 3 1 0 L a 50 0 0 %s r 3 1 0 }" % box_h),
        case("leaders_stretched_v", "v to 100 { L a 0 1fil 0 %s r 3 1 0 L x 0 1fil 0 %s L c 0 1fil 0 %s }" % (box_v, box_v, box_v)),
        case("leaders_nested", "h { L a 50 0 0 h { L a 20 0 0 %s } r 3 1 1 }" % box_h),
        case("leaders_compat", "h to 100 { L a 0 1fil 0 %s r 3 1 1 }" % box_h, "etex=0"),
    ]
    # §644, §645, §669: offsets.
    r += [
        case("offset_h", "h { r 3 1 1 }", "hoffset=100"),
        case("offset_v", "v { r 3 1 0 }", "voffset=100"),
        case("offset_negative", "v { h { r 3 1 1 } }", "hoffset=-100,voffset=-1000"),
        case("offset_both_big", "h { r 3 1 1 }", "hoffset=1000000,voffset=20000000"),
    ]
    # §1711-§1713: LR in hpack, with the display of both kinds of problem; §1714-§1718 where no reversing is wanted.
    r += [
        case("lr_balanced", "h { M 6 r 3 1 1 M 7 }", "texxet=1"),
        case("lr_balanced_nested", "h { M 6 M 6 r 3 1 1 M 7 r 3 1 1 M 7 }", "texxet=1"),
        case("lr_off", "h { r 3 1 1 }", "texxet=0"),
        case("lr_missing", "h { M 6 r 3 1 1 }", "texxet=1"),
        case("lr_missing_two", "h { M 6 M 6 r 3 1 1 }", "texxet=1"),
        case("lr_extra", "h { r 3 1 1 M 7 r 3 1 1 }", "texxet=1"),
        case("lr_extra_two", "h { M 7 r 3 1 1 M 11 }", "texxet=1"),
        case("lr_extra_and_missing", "h { M 6 M 11 r 3 1 1 }", "texxet=1"),
        case("lr_missing_then_underfull", "h to 100 { M 6 r 3 1 1 }", "texxet=1"),
        case("lr_extra_then_overfull", "h to 1 { r 3 1 1 M 7 }", "texxet=1,overfullrule=9"),
        case("lr_missing_r", "h { M 10 r 3 1 1 }", "texxet=1,noship"),
    ]
    # §1699: glue in eTeX mode that becomes a kern, with leaders too, while nothing is reversed.
    r += [
        case("etex_glue_kern_h", "h to 100 { g 5 1fil 0 r 3 1 1 L a 5 1fil 0 r * 1 1 r 3 1 1 L c 5 1fil 0 %s }" % box_h),
    ]
    # §625, §640, §641: half-buffer swaps, with a location that has already gone.  `dvi_buf_size` from the environment.
    many = [7, 8, 9, 300, 40000, 9000000] * 24
    # Old widths again after their commands have gone, first with nothing seen, then with a y seen on the way.
    gone = list(range(1000, 1150)) + [1000, 1001, 5000, 5000, 1002, 5000, 1003]
    r += [
        case("swap_h", "h { %s }" % kerns(many), "dvibuf=800,notrace"),
        case("swap_v", "v { %s }" % kerns(many), "dvibuf=800,notrace"),
        case("swap_gone_h", "h { %s }" % kerns(gone), "dvibuf=800,notrace"),
        case("swap_gone_v", "v { %s }" % kerns(gone), "dvibuf=800,notrace"),
        case("swap_nested", "v { %s h { %s } %s }" % (kerns(many[:60]), kerns(many), kerns(many[:60])), "dvibuf=800,notrace"),
    ]
    return r


def parse_params(params):
    d = {}
    for kv in filter(None, params.split(",")):
        k, _, v = kv.partition("=")
        assert k in PARAM, k
        d[k] = v
    return d


def run(name, params, spec, tmp):
    p = parse_params(params)
    tex = Tex(spec)
    src = tex.box()
    assert tex.i == len(tex.t), spec
    setup = "".join(PARAM[k] % v if "%" in PARAM[k] else PARAM[k] for k, v in p.items() if PARAM[k])
    body = (
        PREAMBLE
        + setup
        + "\n"
        + r"\immediate\write-1{@@start}" + "\n"
        + r"\setbox1=" + src + "\n"
        + r"\immediate\write-1{@@dims \number\wd1 \space\number\ht1 \space\number\dp1 \space\number\badness}" + "\n"
        + (r"\shipout\box1" if "noship" not in p else r"\setbox1=\box2") + "\n"
        + r"\immediate\write-1{@@end}" + "\n"
        + r"\end" + "\n"
    )
    path = os.path.join(tmp, "case.tex")
    with open(path, "w", encoding="latin-1") as f:
        f.write(body)
    for ext in (".log", ".dvi"):
        try:
            os.remove(os.path.join(tmp, "case" + ext))
        except FileNotFoundError:
            pass
    env = dict(os.environ)
    if "dvibuf" in p:
        env["dvi_buf_size"] = p["dvibuf"]
    args = ["pdftex", "-ini", "-interaction=scrollmode"]
    if p.get("etex", "1") == "1":
        args += ["-etex", "*case.tex"]
    else:
        args += ["case.tex"]
    res = subprocess.run(args, cwd=tmp, env=env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
    with open(os.path.join(tmp, "case.log"), "rb") as f:
        log = f.read().decode("latin-1")
    if res.returncode != 0 or "\n!" in log:
        raise SystemExit("case %s: the oracle reports an error:\n%s" % (name, log))
    start = log.index("\n", log.index("@@start")) + 1
    dims_at = log.index("@@dims")
    prefix = log[start:dims_at]
    dims_end = log.index("\n", dims_at)
    dims = log[dims_at + len("@@dims "):dims_end]
    shipped = log[dims_end + 1:log.index("@@end")] if "notrace" not in p else ""
    if "noship" in p:
        return prefix, dims, "", [], "", tex.chars
    with open(os.path.join(tmp, "case.dvi"), "rb") as f:
        dvi = f.read()
    # The postamble, from its end: post_post q i 223 223 223 223 ...
    k = len(dvi) - 1
    while dvi[k] == 223:
        k -= 1
    q = struct.unpack(">I", dvi[k - 4:k])[0]
    assert dvi[q] == 248
    max_v, max_h, max_s = struct.unpack(">IIH", dvi[q + 17:q + 27])
    return prefix, dims, shipped, list(dvi[:q]), "%d %d %d" % (max_v, max_h, max_s), tex.chars


def fnv(data):
    h = 2166136261
    for b in data:
        h = ((h ^ b) * 16777619) & 0xFFFFFFFF
    return h


def field(s):
    assert "\t" not in s and "\x0c" not in s
    return s.replace("\n", "\x0c")


def fonts(all_chars, tmp):
    """The metrics of what the cases use, from the oracle; the check bytes and design size from the TFM."""
    out = []
    names = {"x": ("xx", "cmr10"), "e": ("ee", "ecrm1000")}
    for key in sorted(names):
        fid, file = names[key]
        tfm = subprocess.run(["kpsewhich", file + ".tfm"], stdout=subprocess.PIPE, check=True).stdout.decode().strip()
        with open(tfm, "rb") as f:
            d = f.read()
        lh = struct.unpack(">H", d[2:4])[0]
        assert lh >= 2
        check = " ".join(str(b) for b in d[24:28])
        out.append("\t".join(["@font", fid, check, str(struct.unpack(">i", d[28:32])[0] >> 4)]))
    for key in sorted(names):
        fid, file = names[key]
        codes = sorted(c for f, c in all_chars if f == key)
        if not codes:
            continue
        body = PREAMBLE + "".join(
            "\\immediate\\write-1{@@c %d \\number\\fontcharwd\\%s %d \\space\\number\\fontcharht\\%s %d \\space\\number\\fontchardp\\%s %d }\n"
            % (c, fid, c, fid, c, fid, c)
            for c in codes
        )
        body += "\\end\n"
        with open(os.path.join(tmp, "m.tex"), "w", encoding="latin-1") as f:
            f.write(body)
        subprocess.run(["pdftex", "-ini", "-etex", "-interaction=batchmode", "*m.tex"], cwd=tmp, stdout=subprocess.DEVNULL, check=True)
        with open(os.path.join(tmp, "m.log"), "rb") as f:
            log = f.read().decode("latin-1")
        for line in log.split("\n"):
            if line.startswith("@@c "):
                _, c, w, h, dp = line.split()
                out.append("\t".join(["@char", fid, c, w, h, dp]))
    return out


def main():
    wanted = set(sys.argv[1:])
    with tempfile.TemporaryDirectory() as tmp:
        lines = []
        chars = set()
        names = set()
        for name, params, spec in rows():
            assert name not in names, name
            names.add(name)
            prefix, dims, shipped, dvi, post, cs = run(name, params, spec, tmp)
            chars |= cs
            if wanted:
                if name in wanted:
                    print("%s\n%s\n--- packaging\n%s\n--- dims %s\n--- ship\n%s\n--- dvi\n%s\n--- post %s" % (name, spec, prefix, dims, shipped, dvi, post))
                continue
            lines.append(
                "\t".join(
                    [name, params or "-", spec, str(LINE), field(prefix), dims, field(shipped), "%d %d" % (len(dvi), fnv(dvi)) if dvi else "", post]
                )
            )
        if wanted:
            return
        lines = fonts(chars, tmp) + lines
    with open(OUT, "w", encoding="latin-1", newline="\n") as f:
        f.write("\n".join(lines) + "\n")
    print("%d cases" % len(names))


if __name__ == "__main__":
    main()
