#!/usr/bin/env python3
"""Generate the mechanical parts of crates/engine from the merged WEB.

Run with python3 -I.  Output files (overwritten):

    crates/engine/src/consts.rs    numeric and string @d constants
    crates/engine/src/access.rs    accessors from parameterised and alias @d macros
    crates/engine/src/state.rs     struct Tex, its capacities and section table
    crates/engine/src/<module>.rs  procedure stubs (only with --stubs; destroys ports!)

webgen is run once at M0.  After fan-out, consts.rs, state.rs and access.rs are frozen
and module files belong to their owners: do NOT re-run with --stubs.
"""
import os
import re
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import web as weblib  # noqa: E402
from modmap import MODULES, module_of, section_ranges  # noqa: E402

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
ENGINE = os.path.join(ROOT, "crates", "engine", "src")

# ---------------------------------------------------------------------------
# Names.
# ---------------------------------------------------------------------------
RUST_KEYWORDS = {
    "abstract", "as", "async", "await", "become", "box", "break", "const", "continue", "crate",
    "do", "dyn", "else", "enum", "extern", "false", "final", "fn", "for", "gen", "if", "impl",
    "in", "let", "loop", "macro", "match", "mod", "move", "mut", "override", "priv", "pub", "ref",
    "return", "self", "static", "struct", "super", "trait", "true", "try", "type", "typeof",
    "union", "unsafe", "unsized", "use", "virtual", "where", "while", "yield",
}
RENAME = {"type": "ty", "box": "box_reg", "final": "final_", "loop": "loop_", "mod": "mod_", "in": "in_",
          # §1858 hc[0]:=lc_code(c), which would collide with the setter of lc_code.
          "set_lc_code": "set_hc0_lc_code"}


def rust_name(n):
    if n in RENAME:
        return RENAME[n]
    if n in RUST_KEYWORDS:
        return n + "_"
    return n


def const_name(n):
    return n.upper()


# ---------------------------------------------------------------------------
# Overrides of numeric constants required by DESIGN.md.
# ---------------------------------------------------------------------------
CONST_OVERRIDE = {
    # DESIGN section 4.
    "null": 0,
    "mem_bot": 1,
    "min_halfword": 0,
    "max_quarterword": 0xFFFF,
    "max_halfword": 0xFFFFFFF,
    # Sequential allocation above eqtb_size replaces the classic hash region.
    "hash_size": 0,
    # hash_offset is web2c's pointer trick; unused.
    "hash_offset": 0,
    # Reserved fixed lists at low indices of toks, DESIGN section 5.  toks[0]=toks[1]=0 is null.
    # null_list must exceed 255: §14 says so, and §532 relies on it, since frozen_endv has cmd endv,
    # below active_char, so only cur_chr>255 tells it from a character.
    "null_list": 256,
    "omit_template": 258,
}

# What a format file holds, besides the record of dumped scalars in image.rs: exactly the arrays
# §1485-1507 dump, and the new structures that replace dumped ones.  Everything else, the `prefix`
# above all, is live state during §1517 and is not touched by a format load; a preamble checkpoint
# restores all sections.  docs/PORTING.md, notes on image.rs.
FORMAT_SECTIONS = set("""
eqtb toks hash hash_tab mem_lo mem_hi arena str_pool str_start font_info
trie_trl trie_tro trie_trc hyf_distance hyf_num hyf_next hyph_word hyph_list hyph_link pdf_mem obj_tab
font_check font_size font_dsize font_params hyphen_char skew_char font_name font_area font_bc font_ec
char_base width_base height_base depth_base italic_base lig_kern_base kern_base exten_base param_base
font_glue bchar_label font_bchar font_false_bchar
""".split())
# §1502 dumps only trie_op_ptr entries of these.
LIVE_OVERRIDE = {n: "(0, t.trie_op_ptr as usize + 1)" for n in ("hyf_distance", "hyf_num", "hyf_next")}

# Compile-time memory layout.  mem has MEM_CAP words; the first valid index is MEM_BOT=1.
# Lo memory is [MEM_BOT, MEM_SPLIT), hi memory is [MEM_SPLIT, MEM_TOP], so is_char_node(p)
# is the constant compare p >= MEM_SPLIT.
MEM_CAP = 1 << 25
MEM_TOP = MEM_CAP - 1
MEM_SPLIT = 1 << 24

PINNED_CONSTS = {
    "mem_bot": 1,
    "mem_top": MEM_TOP,
    "mem_max": MEM_TOP,
    "mem_min": 1,
}

# Macros that look numeric but must not be constants, or that are not useful as Rust.
NOT_CONST = {
    "term_in", "term_out", "debug", "gubed", "stat", "tats", "init", "tini", "Init", "Tini",
    "othercases", "endcases", "mtype", "text_char", "pointer", "sc", "do_nothing", "return",
    "loop", "abort", "TEX", "banner_k", "t_open_out", "clear_terminal", "wake_up_terminal",
    "file_name_size", "maxint",
}


class Expr(Exception):
    pass


class Evaluator:
    """Evaluate constant expressions over WEB macros."""

    def __init__(self, w):
        self.w = w
        self.memo = dict(CONST_OVERRIDE)
        self.memo.update(PINNED_CONSTS)
        self.busy = set()
        self.pascal_consts = {}
        self._read_pascal_consts()

    def _read_pascal_consts(self):
        mods = self.w.modules
        for sec in mods["Constants in the outer block"]["defs"]:
            t = weblib.strip_comments(self.w.sections[sec].pascal)
            for m in re.finditer(r"([A-Za-z_][A-Za-z_0-9]*)\s*=\s*([^;]*);", t):
                self.pascal_consts[m.group(1)] = m.group(2).strip()

    def value(self, name):
        if name in self.memo:
            return self.memo[name]
        if name in self.busy:
            raise Expr("cycle " + name)
        self.busy.add(name)
        try:
            ms = self.w.macros.get(name)
            if ms:
                m = ms[-1]
                if m.param:
                    raise Expr("param " + name)
                v = self.eval(m.body)
            elif name in self.pascal_consts:
                v = self.eval(self.pascal_consts[name])
            else:
                raise Expr("unknown " + name)
        finally:
            self.busy.discard(name)
        self.memo[name] = v
        return v

    TOK = re.compile(
        r"\s*(?:(@'[0-7]+)|(@\"[0-9A-Fa-f]+)|(\d+)|\"((?:[^\"]|\"\")*)\"|([A-Za-z_][A-Za-z_0-9]*)|(\S))"
    )

    def tokens(self, s):
        out = []
        pos = 0
        while pos < len(s):
            m = self.TOK.match(s, pos)
            if not m or m.end() == pos:
                break
            pos = m.end()
            if m.group(1):
                out.append(int(m.group(1)[2:], 8))
            elif m.group(2):
                out.append(int(m.group(2)[2:], 16))
            elif m.group(3):
                out.append(int(m.group(3)))
            elif m.group(4) is not None:
                st = m.group(4).replace('""', '"').replace("@@", "@")
                if len(st) != 1:
                    raise Expr("string")
                out.append(ord(st))
            elif m.group(5):
                out.append(("id", m.group(5)))
            else:
                out.append(m.group(6))
        return out

    def eval(self, s):
        toks = self.tokens(s)
        if not toks:
            raise Expr("empty")
        self.pos = 0
        saved = self.__dict__.get("_stack", [])
        v, p = self._sum(toks, 0)
        if p != len(toks):
            raise Expr("trailing")
        return v

    def _sum(self, t, p):
        sign = 1
        if p < len(t) and t[p] == "-":
            sign = -1
            p += 1
        elif p < len(t) and t[p] == "+":
            p += 1
        v, p = self._term(t, p)
        v *= sign
        while p < len(t) and t[p] in ("+", "-"):
            op = t[p]
            r, p = self._term(t, p + 1)
            v = v + r if op == "+" else v - r
        return v, p

    def _term(self, t, p):
        v, p = self._atom(t, p)
        while p < len(t):
            if t[p] == "*":
                r, p = self._atom(t, p + 1)
                v *= r
            elif t[p] == ("id", "div"):
                r, p = self._atom(t, p + 1)
                q = abs(v) // abs(r)
                v = q if (v < 0) == (r < 0) else -q
            elif t[p] == ("id", "mod"):
                r, p = self._atom(t, p + 1)
                q = abs(v) % abs(r)
                v = q if v >= 0 else -q
            else:
                break
        return v, p

    def _atom(self, t, p):
        if p >= len(t):
            raise Expr("eof")
        x = t[p]
        if isinstance(x, int):
            return x, p + 1
        if x == "(":
            v, p = self._sum(t, p + 1)
            if p >= len(t) or t[p] != ")":
                raise Expr("paren")
            return v, p + 1
        if x == "-":
            v, p = self._atom(t, p + 1)
            return -v, p
        if isinstance(x, tuple):
            n = x[1]
            if n in ("qi", "qo", "hi", "ho", "si", "so") and p + 1 < len(t) and t[p + 1] == "(":
                v, q = self._sum(t, p + 2)
                if t[q] != ")":
                    raise Expr("paren")
                return v, q + 1
            sub = Evaluator.value(self, n)
            return sub, p + 1
        raise Expr("bad atom %r" % (x,))


# ---------------------------------------------------------------------------
# consts.rs
# ---------------------------------------------------------------------------
def rust_bytes(s):
    out = []
    for ch in s:
        o = ord(ch)
        if ch == '"':
            out.append('\\"')
        elif ch == "\\":
            out.append("\\\\")
        elif 32 <= o < 127:
            out.append(ch)
        else:
            out.append("\\x%02x" % o)
    return 'b"' + "".join(out) + '"'


def eval_string_macro(ev, body):
    """Return bytes of a WEB string macro: "x", 'x' or concatenation of string macros."""
    parts = []
    for piece in re.findall(r"\"(?:[^\"]|\"\")*\"|'(?:[^']|'')*'|[A-Za-z_][A-Za-z_0-9]*|,", body):
        if piece == ",":
            continue
        if piece[0] == '"':
            parts.append(piece[1:-1].replace('""', '"').replace("@@", "@"))
        elif piece[0] == "'":
            parts.append(piece[1:-1].replace("''", "'").replace("@@", "@"))
        else:
            ms = ev.w.macros.get(piece)
            if not ms or ms[-1].param:
                return None
            sub = eval_string_macro(ev, ms[-1].body)
            if sub is None:
                return None
            parts.append(sub)
    return "".join(parts)


STRING_MACRO = re.compile(
    r"(\"(?:[^\"]|\"\")*\"|'(?:[^']|'')*'|[A-Za-z_][A-Za-z_0-9]*)"
    r"(\s*,\s*(\"(?:[^\"]|\"\")*\"|'(?:[^']|'')*'|[A-Za-z_][A-Za-z_0-9]*))*"
)


def is_string_macro(ev, body):
    """True if the macro body is made only of string literals and other string macros."""
    if not STRING_MACRO.fullmatch(body):
        return False
    for piece in re.findall(r"[A-Za-z_][A-Za-z_0-9]*|\"(?:[^\"]|\"\")*\"|'(?:[^']|'')*'", body):
        if piece[0] in "\"'":
            continue
        ms = ev.w.macros.get(piece)
        if not ms or ms[-1].param or not is_string_macro(ev, ms[-1].body):
            return False
    return any(c in body for c in "\"'") or body in ev.w.macros


def gen_consts(w, ev):
    out = [
        "//! Numeric and string constants, generated by tools/webgen.py from the merged WEB.",
        "//! WEB names are upper-cased.  Overrides required by DESIGN.md are applied here:",
        "//! NULL=0, MEM_BOT=1, MIN_HALFWORD=0, MAX_QUARTERWORD=0xFFFF, HASH_SIZE=0, NULL_LIST=256 (see the notes).",
        "//! Frozen after M0.  Do not edit by hand and do not add to this file.",
        "#![forbid(unsafe_code)]",
        "#![allow(dead_code)]",
        "",
        "/// WEB's own value of max_quarterword.  The semantic limits at §296 (grouping levels),",
        "/// §973-974 (spans) and §1298 (replace count) are observable, so they use this value,",
        "/// not the storage limit MAX_QUARTERWORD.",
        "pub const WEB_MAX_QUARTERWORD: i32 = 255;",
        "/// What `pdf_dest_xyz_zoom` holds for \"no zoom\": WEB's `null`, the oracle's min_halfword, since 0",
        "/// is a legitimate zoom here (`zoom 0` prints `0.0`, no zoom prints `null`).  §784, §1565, §1603.",
        "pub const PDF_ZOOM_NULL: i32 = -0x0FFFFFFF;",
        "/// Boundary between lo and hi memory; is_char_node(p) is p >= MEM_SPLIT.",
        "pub const MEM_SPLIT: i32 = %d;" % MEM_SPLIT,
        "",
    ]
    seen = {}
    emitted = set(["web_max_quarterword", "mem_split", "pdf_zoom_null"])
    # first the pinned names that have no macro
    for name, v in sorted(PINNED_CONSTS.items()):
        out.append("pub const %s: i32 = %d;" % (const_name(name), v))
        emitted.add(const_name(name).lower())
    cur_part = None
    string_macros = 0
    for m in w.macro_list:
        if m.param or m.name in NOT_CONST:
            continue
        cname = const_name(m.name)
        if cname.lower() in emitted:
            continue
        v = None
        try:
            v = ev.value(m.name)
        except Expr:
            v = None
        if v is None:
            # string macro?
            b = m.body
            if is_string_macro(ev, b):
                s = eval_string_macro(ev, b)
                if s is not None:
                    out.append("/// §%d" % m.sec)
                    out.append("pub const %s: &[u8] = %s;" % (cname, rust_bytes(s)))
                    emitted.add(cname.lower())
                    string_macros += 1
            continue
        if not (-(2**63) <= v < 2**63):
            continue
        ty = "i32" if -(2**31) <= v < 2**31 else "i64"
        part = w.sections[m.sec].part
        if part != cur_part:
            cur_part = part
            title = next(t for (n, t, a, b) in w.parts if n == part)
            out.append("")
            out.append("// " + re.sub(r"[\\{}]", "", title))
        out.append("pub const %s: %s = %d; // §%d" % (cname, ty, v, m.sec))
        seen[m.name] = v
        emitted.add(cname.lower())
    # Pascal const section, for those not defined as macros
    out.append("")
    out.append("// @<Constants in the outer block@>")
    for name in ev.pascal_consts:
        cname = const_name(name)
        if cname.lower() in emitted:
            continue
        try:
            v = ev.value(name)
        except Expr:
            continue
        ty = "i32" if -(2**31) <= v < 2**31 else "i64"
        out.append("pub const %s: %s = %d;" % (cname, ty, v))
        seen[name] = v
        emitted.add(cname.lower())
    return "\n".join(out) + "\n", seen



# ---------------------------------------------------------------------------
# Global variables.
# ---------------------------------------------------------------------------
CAPS = [
    # name, value, comment.  Sizes are virtual; pages commit lazily.
    ("MEM_CAP", str(MEM_CAP), "mem words, 256 MiB; lo below MEM_SPLIT, hi above"),
    ("TOKS_CAP", "1 << 26", "toks heap cells, 256 MiB"),
    ("TB_CAP", "1 << 24", "builder stack cells, 64 MiB"),
    ("TX_CAP", "1 << 22", "transient token stack cells, 16 MiB"),
    ("EQTB_CAP", "1 << 21", "eqtb and text entries; run-time control sequences live above EQTB_SIZE"),
    ("HASH_TAB_CAP", "1 << 22", "open-addressing hash slots, a power of two"),
    ("POOL_CAP", "1 << 26", "str_pool bytes"),
    ("STR_CAP", "2097152 + 2", "str_start entries; ssup_max_strings+2"),
    ("BUF_CAP", "1 << 24", "buffer bytes"),
    ("NEST_CAP", "4001", "sup_nest_size+1"),
    ("SAVE_CAP", "1 << 21", "save_stack words"),
    ("STACK_CAP", "30001", "sup_stack_size+1"),
    ("MAX_IN_OPEN_CAP", "128", "sup_max_in_open+1"),
    ("PARAM_CAP", "32768", "sup_param_size+1"),
    ("FONT_CAP", "9001", "max_font_max+1"),
    ("FONT_MEM_CAP", "1 << 24", "font_info words"),
    ("TRIE_CAP", "0x400000", "ssup_trie_size+1"),
    ("HYPH_CAP", "65536", "ssup_hyph_size+1"),
    ("DVI_BUF_CAP", "65537", "sup_dvi_buf_size+1"),
    ("PDF_MEM_CAP", "10000001", "sup_pdf_mem_size+1"),
    ("PDF_OS_BUF_CAP", "5000001", "sup_pdf_os_buf_size+1"),
    ("OBJ_TAB_CAP", "8388608", "sup_obj_tab_size+1"),
    ("DEST_NAMES_CAP", "500001", "sup_dest_names_size+1"),
    ("FILE_NAME_CAP", "65536", "name_of_file bytes"),
    ("ARENA_CAP", "1 << 26", "general POD arena words (u64), 512 MiB; see arena_vec"),
]

# Web2c's dynamic ^T globals and everything else with an explicit representation.
SPECIAL_GLOBALS = {
    "name_of_file": "A<u8, FILE_NAME_CAP>",
    "buffer": "A<u8, BUF_CAP>",
    "str_pool": "A<u8, POOL_CAP>",
    "str_start": "A<i32, STR_CAP>",
    "nest": "A<ListStateRecord, NEST_CAP>",
    "save_stack": "A<Word, SAVE_CAP>",
    "input_stack": "A<InStateRecord, STACK_CAP>",
    "line_stack": "A<i32, MAX_IN_OPEN_CAP>",
    "source_filename_stack": "A<i32, MAX_IN_OPEN_CAP>",
    "full_source_filename_stack": "A<i32, MAX_IN_OPEN_CAP>",
    "eof_seen": "A<bool, MAX_IN_OPEN_CAP>",
    "grp_stack": "A<i32, MAX_IN_OPEN_CAP>",
    "if_stack": "A<i32, MAX_IN_OPEN_CAP>",
    "param_stack": "A<i32, PARAM_CAP>",
    "font_info": "A<Word, FONT_MEM_CAP>",
    "font_check": "A<FourQuarters, FONT_CAP>",
    "dvi_buf": "A<u8, DVI_BUF_CAP>",
    "pdf_mem": "A<i32, PDF_MEM_CAP>",
    "pdf_op_buf": "A<u8, {PDF_OP_BUF_SIZE as usize + 1}>",
    "pdf_os_buf": "A<u8, PDF_OS_BUF_CAP>",
    "pdf_os_objnum": "A<i32, {PDF_OS_MAX_OBJS as usize + 1}>",
    "pdf_os_objoff": "A<i32, {PDF_OS_MAX_OBJS as usize + 1}>",
    "obj_tab": "A<ObjEntry, OBJ_TAB_CAP>",
    "pdf_char_used": "A<A<u8, 32>, FONT_CAP>",
    "trie_trl": "A<i32, TRIE_CAP>",
    "trie_tro": "A<i32, TRIE_CAP>",
    "trie_trc": "A<u16, TRIE_CAP>",
    "hyph_word": "A<i32, HYPH_CAP>",
    "hyph_list": "A<i32, HYPH_CAP>",
    "hyph_link": "A<i32, HYPH_CAP>",
    "trie_c": "A<u8, TRIE_CAP>",
    "trie_o": "A<i32, TRIE_CAP>",
    "trie_l": "A<i32, TRIE_CAP>",
    "trie_r": "A<i32, TRIE_CAP>",
    "trie_hash": "A<i32, TRIE_CAP>",
    "trie_taken": "A<bool, TRIE_CAP>",
    "dest_names": "A<DestNameEntry, DEST_NAMES_CAP>",
    "xord": "A<u8, 256>",
    "xchr": "A<u8, 256>",
    "xprn": "A<u8, 256>",
    "xeq_level": "AO<i32, INT_BASE, {(EQTB_SIZE - INT_BASE) as usize + 1}>",
    "trie_op_hash": "AO<i32, NEG_TRIE_OP_SIZE, {(TRIE_OP_SIZE - NEG_TRIE_OP_SIZE) as usize + 1}>",
}
FONT_ARRAY_BOOL = {"font_used", "pdf_font_nobuiltin_tounicode", "pdf_font_has_space_char", "pdf_font_auto_expand"}

# Globals that are not fields of Tex.
NOT_IN_IMAGE = {
    "yzmem", "zmem", "zeqtb", "yhash", "pool_file", "log_file", "dvi_file", "tfm_file", "pdf_file",
    "vf_file", "fmt_file", "input_file", "read_file", "write_file", "dump_name", "bound_default",
    "bound_name", "TEX_format_default", "output_comment", "help_line", "pdf_buf",
    "mem_bot", "mem_top", "mem_min", "mem_max",
}

# Added fields: not WEB globals.
EXTRA_SCALARS = [
    ("toks_top", "i32", "first never-used cell of toks"),
    ("tb_ptr", "i32", "top of the builder stack tb"),
    ("tx_ptr", "i32", "top of the transient token stack tx: \\csname, \\ifcsname and scan_keyword, whose tokens runaway never shows"),
    ("tb_def_mark", "i32", "builder mark of the list being defined: def_ref's analogue for runaway"),
    ("tb_match_mark", "i32", "builder mark of the macro argument being scanned: temp_head's analogue for runaway"),
    ("tb_align_mark", "i32", "builder mark of the template being scanned: hold_head's analogue for runaway"),
    ("hash_tab_size", "i32", "live slots in hash_tab, a power of two; 0 until hash.rs initialises it"),
    ("hash_tab_used", "i32", "occupied slots in hash_tab"),
    ("arena_top", "i32", "first free word of arena"),
    ("pdf_buf_is_os", "bool", "WEB's pdf_buf points at pdf_os_buf (true) or pdf_op_buf (false)"),
    ("tfm_temp", "i32", "tex.ch, §590: the byte just read by fget"),
    ("tex_input_type", "i32", "tex.ch, §563: 1 if \\input, 0 if \\openin; open_input's must_exist"),
]
EXTRA_ARRAYS = [
    ("tok_free", "A<i32, 256>", "free-list head per size class"),
]

# Hot scalars, in this order, form the first cache line(s).
HOT = [
    "cur_cmd", "cur_chr", "cur_cs", "cur_tok", "cur_input", "align_state", "param_ptr", "tb_ptr",
    "input_ptr", "scanner_status", "long_state", "warning_index", "def_ref", "cur_val",
    "cur_val_level", "selector", "cur_list", "toks_top", "avail", "dyn_used", "var_used",
    "cur_level", "save_ptr", "pool_ptr", "str_ptr", "hash_used", "cs_count", "cond_ptr", "if_limit",
    "cur_if", "in_open", "first", "last", "line", "max_buf_stack", "max_in_stack", "max_param_stack",
    "tally", "term_offset", "file_offset", "is_in_csname", "hi_mem_min", "lo_mem_max", "rover",
]

# Big arrays: placed after all scalars and small arrays, in this order, each one section of
# the section table with a live-range function (start, end) in elements.
BIG_ARRAYS = [
    ("eqtb", "A<Word, EQTB_CAP>", "(1, (EQTB_SIZE + 1 + t.hash_high) as usize)"),
    ("toks", "A<i32, TOKS_CAP>", "(0, t.toks_top as usize)"),
    ("hash", "A<i32, EQTB_CAP>", "(0, (EQTB_SIZE + 1 + t.hash_high) as usize)"),
    ("hash_tab", "A<u64, HASH_TAB_CAP>", "(0, t.hash_tab_size as usize)"),
    ("mem", "A<Word, MEM_CAP>", None),  # two sections, see below
    ("tb", "A<i32, TB_CAP>", "(0, t.tb_ptr as usize)"),
    ("tx", "A<i32, TX_CAP>", "(0, t.tx_ptr as usize)"),
    ("str_pool", SPECIAL_GLOBALS["str_pool"], "(0, t.pool_ptr as usize)"),
    ("str_start", SPECIAL_GLOBALS["str_start"], "(0, t.str_ptr as usize + 1)"),
    ("buffer", SPECIAL_GLOBALS["buffer"], "(0, t.first.max(t.last) as usize + 1)"),
    ("nest", SPECIAL_GLOBALS["nest"], "(0, t.nest_ptr as usize + 1)"),
    ("save_stack", SPECIAL_GLOBALS["save_stack"], "(0, t.save_ptr as usize + 8)"),
    ("input_stack", SPECIAL_GLOBALS["input_stack"], "(0, t.input_ptr as usize + 1)"),
    ("param_stack", SPECIAL_GLOBALS["param_stack"], "(0, t.param_ptr as usize + 1)"),
    ("font_info", SPECIAL_GLOBALS["font_info"], "(0, t.fmem_ptr as usize + 1)"),
    ("trie_trl", SPECIAL_GLOBALS["trie_trl"], "(0, t.trie_max as usize + 1)"),
    ("trie_tro", SPECIAL_GLOBALS["trie_tro"], "(0, t.trie_max as usize + 1)"),
    ("trie_trc", SPECIAL_GLOBALS["trie_trc"], "(0, t.trie_max as usize + 1)"),
    ("trie_c", SPECIAL_GLOBALS["trie_c"], "(0, 0)"),
    ("trie_o", SPECIAL_GLOBALS["trie_o"], "(0, 0)"),
    ("trie_l", SPECIAL_GLOBALS["trie_l"], "(0, 0)"),
    ("trie_r", SPECIAL_GLOBALS["trie_r"], "(0, 0)"),
    ("trie_hash", SPECIAL_GLOBALS["trie_hash"], "(0, 0)"),
    ("trie_taken", SPECIAL_GLOBALS["trie_taken"], "(0, 0)"),
    ("hyph_word", SPECIAL_GLOBALS["hyph_word"], "(0, t.hyph_size as usize + 1)"),
    ("hyph_list", SPECIAL_GLOBALS["hyph_list"], "(0, t.hyph_size as usize + 1)"),
    ("hyph_link", SPECIAL_GLOBALS["hyph_link"], "(0, t.hyph_size as usize + 1)"),
    ("pdf_mem", SPECIAL_GLOBALS["pdf_mem"], "(0, t.pdf_mem_ptr as usize + 1)"),
    ("pdf_os_buf", SPECIAL_GLOBALS["pdf_os_buf"], "(0, (if t.pdf_os_mode { t.pdf_ptr } else { t.pdf_os_ptr }) as usize + 1)"),
    ("obj_tab", SPECIAL_GLOBALS["obj_tab"], "(0, t.sys_obj_ptr as usize + 1)"),
    ("dest_names", SPECIAL_GLOBALS["dest_names"], "(0, t.pdf_dest_names_ptr as usize + 1)"),
    ("arena", "A<u64, ARENA_CAP>", "(0, t.arena_top as usize)"),
    ("dvi_buf", SPECIAL_GLOBALS["dvi_buf"], "(0, t.dvi_ptr as usize + 1)"),
    ("name_of_file", SPECIAL_GLOBALS["name_of_file"], "(0, t.name_length as usize + 2)"),
]

RECORD_TYPES = {
    "list_state_record": "ListStateRecord",
    "in_state_record": "InStateRecord",
    "obj_entry": "ObjEntry",
    "vf_stack_record": "VfStackRecord",
    "dest_name_entry": "DestNameEntry",
    "pdf_link_stack_record": "PdfLinkStackRecord",
    "four_quarters": "FourQuarters",
    "two_halves": "Word",
    "memory_word": "Word",
    "fmemory_word": "Word",
}
RECORD_FIELDS = {
    "ListStateRecord": dict(mode_field="i32", head_field="i32", tail_field="i32", eTeX_aux_field="i32",
                            pg_field="i32", ml_field="i32", aux_field="Word"),
    "InStateRecord": dict(state_field="i32", index_field="i32", start_field="i32", loc_field="i32",
                          limit_field="i32", name_field="i32", synctex_tag_field="i32"),
    "ObjEntry": dict(int0="i32", int1="i32", int2="i64", int3="i32", int4="i32"),
    "VfStackRecord": dict(stack_h="i32", stack_v="i32", stack_w="i32", stack_x="i32", stack_y="i32", stack_z="i32"),
    "DestNameEntry": dict(objname="i32", objnum="i32"),
    "PdfLinkStackRecord": dict(nesting_level="i32", link_node="i32", ref_link_node="i32"),
    "FourQuarters": dict(b0="i32", b1="i32", b2="i32", b3="i32"),
}
# Globals wider than the rule of section 4.  The oracle's `integer` is 64 bits and a sum of glue
# in a box does not wrap there; its ratio gives glue_set, so a clamped total would move the glue.
# Line breaking's running sums are measured from the first active node and come back through the
# deltas, so a clamped one changes every line after it.  docs/PORTING.md section 11.
WIDE_GLOBALS = {
    "total_stretch": "A<i64, 4>", "total_shrink": "A<i64, 4>",
    "active_width": "A<i64, 9>", "cur_active_width": "A<i64, 9>", "background": "A<i64, 9>",
    "break_width": "A<i64, 9>", "prev_active_width": "A<i64, 9>", "disc_width": "A<i64, 9>",
    "best_pl_short": "A<i64, 4>", "best_pl_glue": "A<i64, 4>",
}
BOOL_ALIAS = {"boolean"}
FLOAT_ALIAS = {"real", "glue_ratio"}
LONG_ALIAS = {"longinteger"}
FILE_TYPES = {"alpha_file", "byte_file", "word_file"}


class Global:
    __slots__ = ("name", "ty", "elem", "sec", "scalar", "lo", "hi", "rawtype")

    def __init__(self, name, ty, elem, sec, scalar, rawtype):
        self.name = name
        self.ty = ty
        self.elem = elem
        self.sec = sec
        self.scalar = scalar
        self.rawtype = rawtype


def scalar_rust(t):
    t = t.strip()
    if t in BOOL_ALIAS:
        return "bool"
    if t in FLOAT_ALIAS:
        return "f64"
    if t in LONG_ALIAS:
        return "i64"
    if t in RECORD_TYPES:
        return RECORD_TYPES[t]
    return "i32"


def parse_globals(w, ev):
    out = []
    for n in w.modules["Global variables"]["defs"]:
        s = w.sections[n]
        t = weblib.strip_comments(s.pascal)
        t = re.sub(r"\b(init|tini|debug|gubed|stat|tats)\b", " ", t)
        t = re.sub(r"\s+", " ", t).strip()
        for decl in t.split(";"):
            decl = decl.strip()
            if not decl or ":" not in decl:
                continue
            names, ty = decl.split(":", 1)
            ty = ty.strip()
            for nm in names.split(","):
                nm = nm.strip()
                if not nm:
                    continue
                out.append((nm, ty, n))
    res = []
    for nm, ty, sec in out:
        if nm in NOT_IN_IMAGE:
            continue
        if ty in ("cstring", "const_cstring", "^char") or ty in FILE_TYPES:
            continue
        if nm in WIDE_GLOBALS:
            res.append(Global(nm, WIDE_GLOBALS[nm], "i64", sec, False, ty))
            continue
        if nm in SPECIAL_GLOBALS:
            r = SPECIAL_GLOBALS[nm]
            elem = re.match(r"AO?<(A<u8, 32>|[A-Za-z0-9_]+)", r).group(1)
            res.append(Global(nm, r, elem, sec, False, ty))
            continue
        if ty.startswith("^"):
            base = ty[1:].strip()
            elem = "bool" if base == "boolean" or nm in FONT_ARRAY_BOOL else scalar_rust(base)
            r = "A<%s, FONT_CAP>" % elem
            res.append(Global(nm, r, elem, sec, False, ty))
            continue
        m = re.match(r"(?:packed\s+)?array\s*\[(.*?)\]\s*of\s*(.*)", ty)
        if m:
            bound, base = m.group(1).strip(), m.group(2).strip()
            elem = scalar_rust(base) if ".." not in base else "i32"
            if ".." in bound:
                a, b = bound.split("..")
                lo, hi = ev.value_expr(a), ev.value_expr(b)
            else:
                lo, hi = RANGE_TYPES[bound]
            n_elem = hi + 1
            if lo < 0:
                raise Exception("negative bound needs SPECIAL_GLOBALS: " + nm)
            r = "A<%s, %d>" % (elem, n_elem)
            res.append(Global(nm, r, elem, sec, False, ty))
            continue
        res.append(Global(nm, scalar_rust(ty) if ".." not in ty else "i32", None, sec, True, ty))
    return res


RANGE_TYPES = {
    "text_char": (0, 255), "ASCII_code": (0, 255), "glue_ord": (0, 3), "vf_stack_index": (0, 100),
}


def value_expr(self, s):
    try:
        return self.eval(s)
    except Expr:
        # a bare identifier like very_loose_fit
        return self.value(s.strip())


Evaluator.value_expr = value_expr


# ---------------------------------------------------------------------------
# Macro translation and access.rs.
# ---------------------------------------------------------------------------
from webtrans import Translator, NoTrans, tokenize  # noqa: E402

# Control macros, macros with hand-written equivalents in tools/templates/access_hand.rs, and
# macros made obsolete by the token heap and the hash.  They are not translated.
SKIP_MACROS = set("""
debug gubed stat tats init tini Init Tini othercases endcases loop do_nothing return text_char
pointer sc mtype term_in term_out t_open_out update_terminal clear_terminal wake_up_terminal TEX
hi ho qi qo si so float unfloat float_constant set_glue_ratio_zero set_glue_ratio_one negate
double do_final_end succumb remainder free log_name
wterm wterm_ln wterm_cr wlog wlog_ln wlog_cr print_ASCII prompt_input print_err print_ignored_err
hlp1 hlp2 hlp3 hlp4 hlp5 hlp6 help0 help1 help2 help3 help4 help5 help6
token_ref_count add_token_ref store_new_token fast_store_new_token
next text hash_is_full prim_is_full is_char_node fast_get_avail
char_info char_info_end orig_char_info orig_char_info_end char_width char_width_end
char_italic char_italic_end char_height char_height_end char_depth char_depth_end
char_kern char_kern_end lig_kern_start lig_kern_restart lig_kern_restart_end
param param_end slant space space_stretch space_shrink x_height quad extra_space
mathsy mathsy_end math_x_height math_quad num1 num2 num3 denom1 denom2 sup1 sup2 sup3 sub1 sub2
sup_drop sub_drop delim1 delim2 axis_height
scanned_result scanned_result_end set_conversion set_conversion_end
define word_define global sa_define sa_word_define sa_def_box
nx_plus_y mult_integers scan_normal_dimen vpack natural left_pw right_pw fetch_box
append_list append_list_end pdf_append_list pdf_append_list_end
any_mode non_math any_state_plus add_delims_to four_cases
cur_file edit_file find_effective_tail find_effective_tail_eTeX fetch_effective_tail
fetch_effective_tail_eTeX check_effective_tail
abort fget fbyte call_func const_chk setup_bound_var setup_bound_var_end setup_bound_var_end_end
undump undump_end undump_end_end undump_size undump_size_end undump_size_end_end
format_debug format_debug_end too_small dump_four_ASCII undump_four_ASCII
if_cur_ptr_is_null_then_return_or_goto save_cur_string restore_cur_string
do_all_six do_seven_eight do_all_eight do_one_seven_eight return_sign bad_vf
begin_pseudoprint output_one_char add_char_stretch_end add_char_shrink_end sub_char_stretch_end
sub_char_shrink_end active_height trie_ref font_id_text chr_cmd ensure_dvi_open set_box_lr
set_box_lr_end billion vet_glue round_glue is_valid_char pdf_quick_out pdf_out pdf_room
assign_trace pdf_print_int_ln add_kern_stretch_end add_kern_shrink_end sub_kern_stretch_end
sub_kern_shrink_end
total_font_stretch total_font_shrink save_active_width restore_active_width copy_to_cur_active
set_break_width_to_background reset_disc_width add_disc_width_to_break_width
add_disc_width_to_active_width sub_disc_width_from_active_width store_break_width store_background
act_width kern_break cur_height set_height_zero active_short active_glue
""".split())

# Global names in macros that map to another field name.
GLOBAL_RENAME = {"tex_remainder": "remainder", "texmf_log_name": "log_name", "free_arr": "free",
                 "active_height": "active_width", "trie_ref": "trie_hash"}


class Gen:
    record_fields = RECORD_FIELDS

    def __init__(self, w, ev, consts, globs):
        self.w = w
        self.ev = ev
        self.consts = consts
        self.globals = {g.name: g for g in globs}
        for name, ty, _ in BIG_ARRAYS:
            if name not in self.globals:
                elem = re.match(r"AO?<(A<u8, 32>|[A-Za-z0-9_]+)", ty).group(1)
                self.globals[name] = Global(name, ty, elem, 0, False, ty)
        self.globals["hash"] = Global("hash", "A<i32, EQTB_CAP>", "i32", 274, False, "text")
        self.procs = w.proc_defs()
        self.tr = Translator(self)
        self.stub_sigs = {}

    def rust_name(self, n):
        return rust_name(n)

    def macro(self, n):
        ms = self.w.macros.get(n)
        if not ms or n in SKIP_MACROS:
            return None
        return ms[-1]

    def is_macro_skipped(self, n):
        return n in SKIP_MACROS and n in self.w.macros

    def const_ref(self, n):
        if n in self.consts:
            return const_name(n)
        if n in PINNED_CONSTS or n in CONST_OVERRIDE:
            return const_name(n)
        return None

    def is_global(self, n):
        return GLOBAL_RENAME.get(n, n) in self.globals

    def global_info(self, n):
        return self.globals[GLOBAL_RENAME.get(n, n)]

    def is_proc(self, n):
        return n in self.procs or n in HAND_PROCS

    def proc_info(self, n):
        return self.procs.get(n)

    def is_pure(self, n):
        return n in PURE_HAND

    def param_rust_type(self, n, i):
        p = self.procs.get(n)
        if p is None or i >= len(p.params):
            return "i32"
        return rust_type(p.params[i][2])

    def proc_ret(self, n):
        if n in HAND_PROCS:
            return HAND_PROCS[n]
        p = self.procs[n]
        return SIG_RET.get(n, rust_type(p.result) if p.result else None)


# Methods from access_hand.rs, usable in generated macro bodies: name -> return type.
HAND_PROCS = {
    "print_s": None, "print_nl_s": None, "print_esc_s": None, "print_err_s": None,
    "pdf_print_s": None, "pdf_print_ln_s": None, "update_terminal": None, "wake_up_terminal": None,
    "is_char_node": "bool", "pdf_out": None, "pdf_quick_out": None, "pdf_room": None,
    "text": "i32", "font_id_text": "i32",
}


PURE_HAND = {"is_char_node", "text", "font_id_text"}


def rust_type(t):
    if t is None:
        return None
    t = t.strip()
    if t in BOOL_ALIAS:
        return "bool"
    if t in FLOAT_ALIAS:
        return "f64"
    if t in LONG_ALIAS:
        return "i64"
    if t in RECORD_TYPES:
        return RECORD_TYPES[t]
    if t in FILE_TYPES:
        return "FileId"
    return "i32"


# ---------------------------------------------------------------------------
# Procedure signatures.
# ---------------------------------------------------------------------------
# Parameters that are literal strings at every call site become byte slices.  (A 0 or "" passed
# there becomes b"".)
BYTES_PARAMS = {
    "check_pdfoutput": ["s"], "confusion": ["s"], "fatal_error": ["s"], "overflow": ["s"],
    "pdf_error": ["t", "p"], "pdf_warning": ["t", "p"], "pdf_indirect": ["s"],
    "pdf_indirect_ln": ["s"], "pdf_int_entry": ["s"], "pdf_int_entry_ln": ["s"], "pdf_str_entry": ["s"],
    "pdf_str_entry_ln": ["s"], "primitive": ["s"], "print_glue": ["s"], "print_spec": ["s"],
    "print_write_whatsit": ["s"], "prompt_file_name": ["s"], "restore_trace": ["s"],
    "scan_keyword": ["s"], "show_sa": ["s"], "str_in_str": ["r"], "substr_of_str": ["s"],
    "vf_error": ["msg"], "vf_local_font_warning": ["s"], "warn_dest_dup": ["s1", "s2"],
}
# Result type changes.
SIG_RET = {
    # These returned a tail pointer; they now append to the current token builder.
    "str_toks": None, "the_toks": None,
    # These do not return.
    "jump_out": "!", "overflow": "!", "confusion": "!", "fatal_error": "!", "pdf_error": "!",
    # scan_toks returns def_ref, not the tail: see PORTING.md.
    "scan_toks": "i32",
}
SIG_NOTE = {
    "str_toks": "Appends to the current token builder instead of returning a tail; see PORTING.md.",
    "the_toks": "Appends to the current token builder instead of returning a tail; see PORTING.md.",
    "scan_general_text": "Appends to the current token builder; cur_val is not set; see PORTING.md.",
    "scan_toks": "Returns def_ref (rc 0, never null), not the tail; see PORTING.md.",
    "show_token_list": "p and q are toks indices of tokens, not headers: pass h+1 for a list h; q=0 for none.",
    "token_show": "p is a list header in toks, or 0.",
    "begin_token_list": "p is a list header in toks, or 0.  Sets loc=p+1 unless t=MACRO.",
    "print_meaning": "cur_chr of a macro is a header in toks.",
    "read_toks": "Commits with tb_commit_ref; cur_val is the header.",
    "effective_char": "MLTeX is dropped: returns c.",
    "effective_char_info": "MLTeX is dropped: returns null_character unless the char exists.",
    "insert_src_special": "src-specials are dropped.",
    "append_src_special": "src-specials are dropped.",
    "get_strings_started": "Makes strings 0-255, then the well-known strings S_*; no pool file.",
    "id_lookup": "Open addressing in hash_tab; run-time control sequences are sequential above EQTB_SIZE.",
    "primitive": "s is the name.  A one-byte name is a single character: cur_val=s[0]+SINGLE_BASE.",
}
# Procedures implemented in toks.rs rather than in their part's module.
IMPLEMENTED_ELSEWHERE = {"delete_token_ref", "flush_list"}


def stub_signature(p):
    name = rust_name(p.name)
    if p.name == "flush_list":
        name = "mem_flush_list"
    parts = ["&mut self"]
    for var, a, ty in p.params:
        rn = rust_name(a)
        rty = rust_type(ty)
        if a in BYTES_PARAMS.get(p.name, []):
            rty = "&[u8]"
        if rty == "i64" and not var:
            # Most callers pass i32: accept both without a cast.  Start the body with
            # `let n: i64 = n.into();`.
            rty = "impl Into<i64>"
        if var:
            if rty == "FileId":
                parts.append("%s: FileId" % rn)
            else:
                parts.append("%s: &mut %s" % (rn, rty))
        else:
            parts.append("%s: %s" % (rn, rty))
    ret = SIG_RET.get(p.name, rust_type(p.result) if p.result else None)
    if p.name in ("scan_general_text",):
        ret = None
    s = "pub fn %s(%s)" % (name, ", ".join(parts))
    if ret:
        s += " -> " + ret
    return s


def gen_access(w, ev, gen):
    hand = open(os.path.join(ROOT, "tools", "templates", "access_hand.rs")).read()
    out = [
        "//! Node, eqtb and list accessors, from WEB's parameterised and alias @d macros.",
        "//! The part up to the marker is generated by tools/webgen.py; the rest is",
        "//! tools/templates/access_hand.rs.  Frozen after M0: see docs/PORTING.md.",
        "//!",
        "//! `link(p)` is `self.link(p)`, `link(p):=q` is `self.set_link(p,q)`.  WEB names are kept;",
        "//! `type` is `ty` and `box` is `box_reg`.",
        "#![forbid(unsafe_code)]",
        "#![allow(non_snake_case, dead_code, unused_parens, clippy::all)]",
        "",
        "use crate::consts::*;",
        "use crate::host::*;",
        "use crate::raw::*;",
        "use crate::state::*;",
        "",
        "impl Tex {",
    ]
    done = []
    failed = {}
    for m in w.macro_list:
        if m.name in SKIP_MACROS or m.name in NOT_CONST:
            continue
        if not m.param and const_or_str(m.name, gen, ev):
            continue
        try:
            t = gen.tr.method(m.name)
        except NoTrans as e:
            failed[m.name] = str(e)
            continue
        rn = rust_name(m.name)
        params = t["params"]
        plist = ", p: %s" % params[0] if params else ""
        ret = t.get("ret")
        self_ref = "&mut self" if t["mut"] else "&self"
        out.append("    /// §%d" % m.sec)
        out.append("    #[inline(always)]")
        if t.get("alias"):
            tgt = rust_name(t["alias"])
            arg = "p" if params else ""
            r = " -> %s" % ret if ret and ret != "void" else ""
            out.append("    pub fn %s(%s%s)%s { self.%s(%s) }" % (rn, self_ref, plist, r, tgt, arg))
            if t.get("set"):
                out.append("    #[inline(always)]")
                out.append("    pub fn set_%s(&mut self%s, v: %s) { self.set_%s(%sv) }" % (rn, plist, ret, tgt, "p, " if params else ""))
            done.append(m.name)
            continue
        if t.get("stmt") or ret == "void" or ret is None:
            body = t["get"]
            if not t.get("stmt"):
                body = body + ";"
            out.append("    pub fn %s(&mut self%s) {" % (rn, plist))
            for line in body.split("\n"):
                out.append("        " + line)
            out.append("    }")
            done.append(m.name)
            continue
        if ret == "bytes":
            del out[-2:]
            failed[m.name] = "byte string"
            continue
        out.append("    pub fn %s(%s%s) -> %s {" % (rn, self_ref, plist, ret))
        out.append("        " + t["get"])
        out.append("    }")
        if t.get("set"):
            out.append("    #[inline(always)]")
            out.append("    pub fn set_%s(&mut self%s, v: %s) {" % (rn, plist, ret))
            out.append("        " + t["set"]("v") + ";")
            out.append("    }")
        done.append(m.name)
    out.append("}")
    out.append("")
    out.append("// ---- end of generated part; the rest is tools/templates/access_hand.rs ----")
    out.append(hand)
    return "\n".join(out) + "\n", done, failed


def const_or_str(name, gen, ev):
    if name in gen.consts:
        return True
    if name in PINNED_CONSTS or name in CONST_OVERRIDE:
        return True
    ms = gen.w.macros.get(name)
    if ms:
        b = ms[-1].body
        if is_string_macro(ev, b):
            return True
    return False


# ---------------------------------------------------------------------------
# state.rs
# ---------------------------------------------------------------------------
RECORD_DEFS = """
crate::pod_struct! {
    /// list_state_record §230.
    pub struct ListStateRecord {
        pub mode_field: i32,
        pub head_field: i32,
        pub tail_field: i32,
        pub eTeX_aux_field: i32,
        pub pg_field: i32,
        pub ml_field: i32,
        pub aux_field: Word,
    }
}

crate::pod_struct! {
    /// in_state_record §322.
    pub struct InStateRecord {
        pub state_field: i32,
        pub index_field: i32,
        pub start_field: i32,
        pub loc_field: i32,
        pub limit_field: i32,
        pub name_field: i32,
        pub synctex_tag_field: i32,
    }
}

crate::pod_struct! {
    /// obj_entry §694.
    pub struct ObjEntry {
        pub int0: i32,
        pub int1: i32,
        pub int2: i64,
        pub int3: i32,
        pub int4: i32,
    }
}

crate::pod_struct! {
    /// vf_stack_record §722.
    pub struct VfStackRecord {
        pub stack_h: i32,
        pub stack_v: i32,
        pub stack_w: i32,
        pub stack_x: i32,
        pub stack_y: i32,
        pub stack_z: i32,
    }
}

crate::pod_struct! {
    /// dest_name_entry §1627.
    pub struct DestNameEntry {
        pub objname: i32,
        pub objnum: i32,
    }
}

crate::pod_struct! {
    /// pdf_link_stack_record §1632.
    pub struct PdfLinkStackRecord {
        pub nesting_level: i32,
        pub link_node: i32,
        pub ref_link_node: i32,
    }
}
"""


def gen_state(w, ev, gen, globs):
    big_names = [b[0] for b in BIG_ARRAYS]
    scalars = [g for g in globs if g.scalar]
    arrays = [g for g in globs if not g.scalar and g.name not in big_names]
    by_name = {g.name: g for g in scalars}
    for n, ty, why in EXTRA_SCALARS:
        by_name[n] = Global(n, ty, None, 0, True, why)
        scalars.append(by_name[n])
    # Large arrays: everything with a FONT_CAP or an element count of 4096 or more.
    font_arrays = []
    other_big = []
    small_arrays = []
    for g in arrays:
        m = re.search(r",\s*(\d+)>$", g.ty)
        if "FONT_CAP" in g.ty:
            font_arrays.append(g)
        elif m and int(m.group(1)) >= 4096:
            other_big.append(g)
        elif g.name in ("xeq_level", "trie_op_hash"):
            other_big.append(g)
        else:
            small_arrays.append(g)
    out = [
        "//! The POD image: `struct Tex` holds every WEB global.  Generated by tools/webgen.py; frozen",
        "//! after M0.  Do not add fields here.  A module adds its own state to the `Vars` struct in",
        "//! its own file (docs/PORTING.md, \"Adding a global\").",
        "//!",
        "//! Types per the key type decision: every integer-like WEB type is `i32`, `boolean` is `bool`,",
        "//! `longinteger` is `i64`, `real` and `glue_ratio` are `f64`.  Arrays are inline and fixed.",
        "//! The exceptions are the sums of glue and of line breaking, in `WIDE_GLOBALS`: docs/PORTING.md section 11.",
        "//! Not here: file variables (Host), `help_line` (Host), `mem_bot`..`mem_max` (consts), and",
        "//! `pdf_buf` (a pointer in WEB; `pdf_buf_is_os` selects pdf_os_buf or pdf_op_buf).",
        "#![forbid(unsafe_code)]",
        "#![allow(non_snake_case, dead_code)]",
        "",
        "use crate::consts::*;",
        "use crate::host::Host;",
        "use crate::raw::*;",
        "",
        "// Capacities.  Virtual only: the mapping commits lazily.  The web2c size variables",
        "// (pool_size, stack_size, ...) remain fields, default to these, and may be set lower.",
    ]
    for n, v, why in CAPS:
        out.append("/// %s" % why)
        out.append("pub const %s: usize = %s;" % (n, v))
    out.append(RECORD_DEFS)
    out.append("/// The whole state of the engine; `#[repr(C)]`, placed in one reservation, never moved.")
    out.append("#[repr(C)]")
    out.append("pub struct Tex {")
    out.append("    // ---- hot scalars ----")
    emitted = set()

    def emit(g):
        comment = g.rawtype.replace("\n", " ")
        sec = " §%d" % g.sec if g.sec else ""
        out.append("    /// %s%s" % (re.sub(r"\s+", " ", comment), sec))
        out.append("    pub %s: %s," % (rust_name(g.name), g.ty))

    for n in HOT:
        emit(by_name[n])
        emitted.add(n)
    out.append("    // ---- other scalars, in WEB order ----")
    for g in scalars:
        if g.name in emitted:
            continue
        emit(g)
        emitted.add(g.name)
    out.append("    // ---- small arrays ----")
    for g in small_arrays:
        emit(g)
    for n, ty, why in EXTRA_ARRAYS:
        out.append("    /// %s" % why)
        out.append("    pub %s: %s," % (n, ty))
    out.append("    // ---- per-module state, each struct in the module's own file ----")
    for m in ALL_MODULES:
        out.append("    pub v_%s: crate::%s::Vars," % (m, modpath(m)))
    out.append("    // ---- big arrays; each is one section of the section table ----")
    sections = []
    for name, ty, rng in BIG_ARRAYS:
        out.append("    pub %s: %s," % (name, ty))
        if name == "mem":
            sections.append(("mem_lo", "mem", "Word", "(0, t.lo_mem_max as usize + 2)"))
            sections.append(("mem_hi", "mem", "Word", "(t.hi_mem_min as usize, MEM_CAP)"))
        else:
            sections.append((name, name, ty, rng))
    for g in font_arrays:
        emit(g)
        sections.append((g.name, g.name, g.ty, "(0, t.font_ptr as usize + 1)"))
    for g in other_big:
        emit(g)
        sections.append((g.name, g.name, g.ty, None))
    out.append("    // ---- not POD; never snapshotted; last ----")
    out.append("    pub host: Host,")
    out.append("}")
    out.append("")
    # Pod check
    out.append("const _: () = {")
    out.append("    fn _pod<T: Pod>() {}")
    out.append("    fn _check() {")
    seen_ty = set()
    for g in scalars + small_arrays + font_arrays + other_big:
        if g.ty not in seen_ty:
            seen_ty.add(g.ty)
            out.append("        _pod::<%s>();" % g.ty)
    for _, ty, _r in BIG_ARRAYS:
        if ty not in seen_ty:
            seen_ty.add(ty)
            out.append("        _pod::<%s>();" % ty)
    for m in ALL_MODULES:
        out.append("        _pod::<crate::%s::Vars>();" % modpath(m))
    out.append("    }")
    out.append("};")
    out.append("")
    # first big field
    first_big = BIG_ARRAYS[0][0]
    out.append('''/// One range of the POD image, for snapshots.
pub struct PodSection {
    pub name: &'static str,
    /// Byte offset of the array's element 0 in `Tex`.
    pub offset: usize,
    pub elem_size: usize,
    /// Number of elements in the array.
    pub cap: usize,
    /// The live range `(start, end)` in elements; the high-water mark for this array.
    pub live: fn(&Tex) -> (usize, usize),
    /// A format file holds this section.  A format load is not a checkpoint restore: it runs in
    /// the middle of §1517 and overwrites only what §1485-1507 dump.  docs/PORTING.md, image.rs.
    pub in_format: bool,
}

/// Every POD range of `Tex`.  The first entry is the prefix of scalars, small arrays and module
/// `Vars`; the others are arrays with live-range functions.  A test checks that they are disjoint
/// and cover the image up to `host`.
pub const POD_SECTIONS: &[PodSection] = &[''')
    out.append('    PodSection { name: "prefix", offset: 0, elem_size: 1, cap: core::mem::offset_of!(Tex, %s), live: |_| (0, core::mem::offset_of!(Tex, %s)), in_format: false },' % (first_big, first_big))
    names = [x[0] for x in sections]
    for n in FORMAT_SECTIONS:
        if n not in names:
            raise Exception("FORMAT_SECTIONS names no section: " + n)
    for name, field, ty, rng in sections:
        rng = LIVE_OVERRIDE.get(name, rng)
        if rng is None:
            elem_cap = "core::mem::size_of::<%s>() / core::mem::size_of::<%s>()" % (ty, elem_of(ty))
            live = "|_| (0, %s)" % elem_cap
        else:
            live = ("|t| %s" if "t." in rng else "|_| %s") % rng
        elem = elem_of(ty) if not name.startswith("mem") else "Word"
        cap = "core::mem::size_of::<%s>() / core::mem::size_of::<%s>()" % (ty if not name.startswith("mem") else "A<Word, MEM_CAP>", elem)
        out.append('    PodSection { name: "%s", offset: core::mem::offset_of!(Tex, %s), elem_size: core::mem::size_of::<%s>(), cap: %s, live: %s, in_format: %s },' % (name, field, elem, cap, live, "true" if name in FORMAT_SECTIONS else "false"))
    out.append("];")
    out.append('''
impl Tex {
    /// Allocates the image in a 2 MiB-aligned reservation and constructs `Host` in place.  Every
    /// other field is valid as zero bytes, which the `Pod` check above guarantees.
    pub fn alloc() -> &'static mut Tex {
        raw_alloc(Host::new())
    }

    /// Allocates with a given host, for tests and tools.
    pub fn alloc_with(host: Host) -> &'static mut Tex {
        raw_alloc(host)
    }

    /// Ends the life of an image from `alloc`, for tests.
    pub fn dealloc(t: &'static mut Tex) {
        drop_in_place_of(&mut t.host);
        free_image(t);
    }
}

fn raw_alloc(host: Host) -> &'static mut Tex {
    let t: &'static mut Tex = alloc_image::<Tex, Host>(core::mem::offset_of!(Tex, host), host);
    // `cs_token_flag+0`, backed up by §405 after `\\pdfprimitive\\fi`, reads eqtb[0].  Its cur_tok is
    // then cmd*256+chr, which must not be 0, the list terminator.  docs/PORTING.md section 15.
    t.eqtb[0].set_rh(1);
    t
}

/// Total POD size must stay under 2 GiB, so that `disp32(%rdi,%rcx,8)` addressing works.
const _: () = assert!(core::mem::offset_of!(Tex, host) < (1usize << 31));
''')
    return "\n".join(out) + "\n"


def elem_of(ty):
    m = re.match(r"AO?<(A<u8, 32>|[A-Za-z0-9_]+)", ty)
    return m.group(1)


def modpath(m):
    return m


# Module list includes files with no stub ranges.
ALL_MODULES = list(MODULES)


# ---------------------------------------------------------------------------
# External functions.  The C side of web2c and pdfTeX, which WEB calls but does not define.
# (module, signature, note)
# ---------------------------------------------------------------------------
EXTERNS = [
    # lib/, kpathsea: the file interface lives in host.rs with real bodies; see host.rs.
    # tex.ch / texmfmp.c
    ("web2c", "pub fn call_edit(&mut self, filename_start: i32, fn_length: i32, linenumber: i32) -> !", "texmfmp.c calledit; \\end with E"),
    ("web2c", "pub fn print_c_string(&mut self, s: &[u8])", "print each byte with print_char"),
    ("web2c", "pub fn recorder_change_filename(&mut self, new_name: &[u8])", "the .fls recorder: journal"),
    ("web2c", "pub fn make_src_special(&mut self, srcfilename: i32, lineno: i32) -> i32", "src-specials are dropped"),
    ("web2c", "pub fn remember_source_info(&mut self, srcfilename: i32, lineno: i32)", "src-specials are dropped"),
    ("web2c", "pub fn is_new_source(&mut self, srcfilename: i32, lineno: i32) -> bool", "src-specials are dropped"),
    ("web2c", "pub fn get_job_name(&mut self, name: i32) -> i32", "-jobname"),
    ("web2c", "pub fn ipc_page(&mut self, is_eof: i32)", "IPC is dropped: no-op"),
    # synctex.c
    ("web2c", "pub fn synctex_abort(&mut self, log_opened: bool)", "synctex.c, M7"),
    ("web2c", "pub fn synctex_sheet(&mut self, mag: i32)", "synctex.c, M7"),
    ("web2c", "pub fn synctex_teehs(&mut self)", "synctex.c, M7"),
    ("web2c", "pub fn synctex_vlist(&mut self, this_box: i32)", "synctex.c, M7"),
    ("web2c", "pub fn synctex_tsilv(&mut self, this_box: i32)", "synctex.c, M7"),
    ("web2c", "pub fn synctex_void_vlist(&mut self, p: i32, this_box: i32)", "synctex.c, M7"),
    ("web2c", "pub fn synctex_hlist(&mut self, this_box: i32)", "synctex.c, M7"),
    ("web2c", "pub fn synctex_tsilh(&mut self, this_box: i32)", "synctex.c, M7"),
    ("web2c", "pub fn synctex_void_hlist(&mut self, p: i32, this_box: i32)", "synctex.c, M7"),
    ("web2c", "pub fn synctex_math(&mut self, p: i32, this_box: i32)", "synctex.c, M7"),
    ("web2c", "pub fn synctex_horizontal_rule_or_glue(&mut self, p: i32, this_box: i32)", "synctex.c, M7"),
    ("web2c", "pub fn synctex_kern(&mut self, p: i32, this_box: i32)", "synctex.c, M7"),
    ("web2c", "pub fn synctex_char(&mut self, p: i32, this_box: i32)", "synctex.c, M7"),
    ("web2c", "pub fn synctex_node(&mut self, p: i32, this_box: i32)", "synctex.c, M7"),
    ("web2c", "pub fn synctex_current(&mut self)", "synctex.c, M7"),
    ("web2c", "pub fn synctex_pdfxform(&mut self, p: i32)", "synctex.c, M7"),
    ("web2c", "pub fn synctex_mrofxfdp(&mut self)", "synctex.c, M7"),
    ("web2c", "pub fn synctex_pdfrefxform(&mut self, objnum: i32)", "synctex.c, M7"),
    ("web2c", "pub fn synctex_terminate(&mut self, log_opened: bool)", "synctex.c, M7"),
    ("web2c", "pub fn synctex_init_command(&mut self)", "synctex.c, M7"),
    ("web2c", "pub fn synctex_start_input(&mut self)", "synctex.c, M7"),
    # utils.c
    ("buildtoks", "pub fn escapehex(&mut self, in_: i32)", "utils.c, \\pdfescapehex"),
    ("buildtoks", "pub fn unescapehex(&mut self, in_: i32)", "utils.c, \\pdfunescapehex"),
    ("buildtoks", "pub fn escapename(&mut self, in_: i32)", "utils.c, \\pdfescapename"),
    ("buildtoks", "pub fn escapestring(&mut self, in_: i32)", "utils.c, \\pdfescapestring"),
    ("buildtoks", "pub fn getfilesize(&mut self, s: i32)", "texmfmp.c, \\pdffilesize"),
    ("buildtoks", "pub fn getfilemoddate(&mut self, s: i32)", "texmfmp.c, \\pdffilemoddate"),
    ("buildtoks", "pub fn getfiledump(&mut self, s: i32, offset: i32, length: i32)", "texmfmp.c, \\pdffiledump"),
    ("buildtoks", "pub fn getmd5sum(&mut self, s: i32, file: bool)", "texmfmp.c, \\pdfmdfivesum"),
    ("buildtoks", "pub fn matchstrings(&mut self, s: i32, t: i32, subcount: i32, icase: bool) -> i32", "utils.c, \\pdfmatch: POSIX ERE, leftmost-longest"),
    ("buildtoks", "pub fn getmatch(&mut self, i: i32)", "utils.c, \\pdflastmatch"),
    ("buildtoks", "pub fn newcolorstack(&mut self, s: i32, literal_mode: i32, pagestart: bool) -> i32", "utils.c: arena in the image"),
    ("buildtoks", "pub fn getcreationdate(&mut self)", "texmfmp.c, \\pdfcreationdate"),
    ("pack", "pub fn ext_xn_over_d(&mut self, x: i32, n: i32, d: i32) -> i32", "utils.c extxnoverd: computes in doubles, with its own rounding and a warning, not arith_error"),
    ("pdfship", "pub fn colorstackset(&mut self, colstack_no: i32, s: i32) -> i32", "utils.c"),
    ("pdfship", "pub fn colorstackpush(&mut self, colstack_no: i32, s: i32) -> i32", "utils.c"),
    ("pdfship", "pub fn colorstackpop(&mut self, colstack_no: i32) -> i32", "utils.c"),
    ("pdfship", "pub fn colorstackcurrent(&mut self, colstack_no: i32) -> i32", "utils.c"),
    ("pdfship", "pub fn colorstackskippagestart(&mut self, colstack_no: i32) -> i32", "utils.c"),
    ("pdfship", "pub fn checkpdfsave(&mut self, cur_h: i32, cur_v: i32)", "utils.c"),
    ("pdfship", "pub fn checkpdfrestore(&mut self, cur_h: i32, cur_v: i32)", "utils.c"),
    ("pdfship", "pub fn pdfshipoutbegin(&mut self, shipping_page: bool)", "utils.c"),
    ("pdfship", "pub fn pdfshipoutend(&mut self, shipping_page: bool)", "utils.c"),
    ("pdfship", "pub fn pdfsetmatrix(&mut self, in_: i32, cur_h: i32, cur_v: i32) -> i32", "utils.c"),
    ("ext", "pub fn matrixtransformrect(&mut self, llx: i32, lly: i32, urx: i32, ury: i32)", "utils.c"),
    ("ext", "pub fn matrixrecalculate(&mut self, urx: i32)", "utils.c"),
    ("ext", "pub fn matrixused(&mut self) -> bool", "utils.c"),
    ("ext", "pub fn getllx(&mut self) -> i32", "utils.c"),
    ("ext", "pub fn getlly(&mut self) -> i32", "utils.c"),
    ("ext", "pub fn geturx(&mut self) -> i32", "utils.c"),
    ("ext", "pub fn getury(&mut self) -> i32", "utils.c"),
    ("pdfship", "pub fn print_ID(&mut self, filename: i32)", "utils.c printID, /ID"),
    ("pdfship", "pub fn print_ID_alt(&mut self, toks: i32)", "utils.c printIDalt"),
    ("pdfship", "pub fn set_job_id(&mut self, year: i32, month: i32, day: i32, time: i32)", "utils.c setjobid"),
    ("pdfship", "pub fn make_cstring(&mut self, s: i32) -> Vec<u8>", "utils.c makecstring"),
    ("pdfship", "pub fn kpse_init_prog(&mut self, prefix: &[u8], dpi: i32, mode: &[u8], fallback: &[u8])", "PK fonts are dropped: no-op"),
    ("pdfship", "pub fn kpse_set_program_enabled(&mut self, fmt: i32, value: bool, level: i32)", "mktex is dropped: no-op"),
    ("pdfout", "pub fn write_stream_length(&mut self, length: i64, offset: i64)", "utils.c writestreamlength: a deferred field of the PDF chunk list"),
    ("pdfout", "pub fn write_zip(&mut self, finish: bool)", "writezip.c: zlib-rs via rstex-pdfio, into the PDF chunk list"),
    ("pdfout", "pub fn write_pdf(&mut self, a: i32, b: i32)", "pdf_buf[a..=b] into the PDF chunk list"),
    ("pdfout", "pub fn get_pk_char_width(&mut self, f: i32, w: i32) -> i32", "PK fonts are dropped: reports an error"),
    ("pdfout", "pub fn pdf_mark_char(&mut self, f: i32, c: i32)", "pdftex.h: pdf_char_used bit"),
    ("pdfout", "pub fn pdf_char_marked(&mut self, f: i32, c: i32) -> bool", "pdftex.h: pdf_char_used bit"),
    ("pdfout", "pub fn pdfassert(&mut self, cond: bool)", "pdftex.h: confusion if false"),
    # mapfile.c
    ("pdffont", "pub fn hasfmentry(&mut self, f: i32) -> bool", "mapfile.c"),
    ("pdffont", "pub fn isscalable(&mut self, f: i32) -> bool", "mapfile.c"),
    ("pdffont", "pub fn hasspacechar(&mut self, f: i32) -> bool", "mapfile.c"),
    ("pdffont", "pub fn pdfmapfile(&mut self, t: i32)", "mapfile.c: t is a token list header"),
    ("pdffont", "pub fn pdfmapline(&mut self, t: i32)", "mapfile.c: t is a token list header"),
    ("pdffont", "pub fn pdf_init_map_file(&mut self, map_name: &[u8])", "mapfile.c"),
    ("pdffont", "pub fn def_tounicode(&mut self, glyph: i32, unistr: i32)", "tounicode.c"),
    ("pdffont", "pub fn do_pdf_font(&mut self, font_objnum: i32, f: i32)", "writefont.c via rstex-fontfile"),
    ("pdffont", "pub fn new_vf_packet(&mut self, f: i32) -> i32", "vfpacket.c: VF is deferred"),
    ("pdffont", "pub fn storepacket(&mut self, f: i32, c: i32, s: i32)", "vfpacket.c: VF is deferred"),
    ("pdffont", "pub fn start_packet(&mut self, f: i32, c: i32)", "vfpacket.c: VF is deferred"),
    # avlstuff.c
    ("pdfobj", "pub fn avl_put_obj(&mut self, objptr: i32, t: i32)", "avlstuff.c: PodMap in the arena"),
    ("pdfobj", "pub fn avl_find_obj(&mut self, t: i32, i: i32, byname: i32) -> i32", "avlstuff.c: PodMap in the arena"),
    ("pdffont", "pub fn packet_byte(&mut self) -> i32", "vfpacket.c: VF is deferred"),
    ("pdffont", "pub fn push_packet_state(&mut self)", "vfpacket.c: VF is deferred"),
    ("pdffont", "pub fn pop_packet_state(&mut self)", "vfpacket.c: VF is deferred"),
    ("pdffont", "pub fn allocvffnts(&mut self)", "utils.c: the arrays are fixed here, so a capacity check"),
    ("pdffont", "pub fn pdfmaplinesp(&mut self)", "mapfile.c: map line of the \\pdfspacefont"),
    ("pdffont", "pub fn write_fontstuff(&mut self)", "writefont.c via rstex-fontfile"),
    # the rest of utils.c, called without parentheses in WEB
    ("pdfship", "pub fn libpdffinish(&mut self)", "utils.c: frees C-side tables; a no-op here"),
    ("pdfship", "pub fn remove_pdffile(&mut self)", "utils.c: on fatal error"),
    ("pdfship", "pub fn garbage_warning(&mut self)", "utils.c"),
    ("pdfship", "pub fn get_resname_prefix(&mut self) -> i32", "utils.c"),
    ("pdfship", "pub fn print_creation_date(&mut self)", "texmfmp.c"),
    ("pdfship", "pub fn print_mod_date(&mut self)", "texmfmp.c"),
    ("pdfship", "pub fn colorstackused(&mut self) -> i32", "utils.c"),
    ("pdfship", "pub fn flush_jbig2_page0_objects(&mut self)", "JBIG2 is dropped: no-op"),
    ("ext", "pub fn init_start_time(&mut self)", "texmfmp.c: World holds the start time"),
    ("ext", "pub fn synctexpdfrefxform(&mut self, objnum: i32)", "synctex.c, M7"),
    ("files", "pub fn make_full_name_string(&mut self) -> i32", "texmfmp.c: the string of host.full_name_of_file"),
    ("dvi", "pub fn write_dvi(&mut self, a: i32, b: i32)", "dvi_buf[a..=b] to DVI_FILE, through host.put_bytes"),
    ("image", "pub fn dumpimagemeta(&mut self)", "writeimg.c: the arena is in the snapshot, so normally no-op"),
    ("image", "pub fn dumptounicode(&mut self)", "tounicode.c: the arena is in the snapshot, so normally no-op"),
    ("image", "pub fn undumptounicode(&mut self)", "tounicode.c: the arena is in the snapshot, so normally no-op"),
    ("main", "pub fn make_pdftex_banner(&mut self)", "utils.c: sets pdftex_banner"),
    ("main", "pub fn read_tcx_file(&mut self)", "TCX reader dropped: installs the hardcoded cp227 printable table"),
    ("main", "pub fn dump_core(&mut self)", "debug only"),
    # pdftex.h font codes
    ("pack", "pub fn get_lp_code(&mut self, f: i32, c: i32) -> i32", "pdftex.h: font_info base + c"),
    ("pack", "pub fn get_rp_code(&mut self, f: i32, c: i32) -> i32", "pdftex.h"),
    ("pack", "pub fn get_ef_code(&mut self, f: i32, c: i32) -> i32", "pdftex.h"),
    ("pdffont", "pub fn get_kn_bs_code(&mut self, f: i32, c: i32) -> i32", "pdftex.h"),
    ("pdffont", "pub fn get_st_bs_code(&mut self, f: i32, c: i32) -> i32", "pdftex.h"),
    ("pdffont", "pub fn get_sh_bs_code(&mut self, f: i32, c: i32) -> i32", "pdftex.h"),
    ("pdffont", "pub fn get_kn_bc_code(&mut self, f: i32, c: i32) -> i32", "pdftex.h"),
    ("pdffont", "pub fn get_kn_ac_code(&mut self, f: i32, c: i32) -> i32", "pdftex.h"),
    # writeimg.c
    ("ext", "pub fn check_image_b(&mut self, procset: i32) -> bool", "writeimg.c"),
    ("ext", "pub fn check_image_c(&mut self, procset: i32) -> bool", "writeimg.c"),
    ("ext", "pub fn check_image_i(&mut self, procset: i32) -> bool", "writeimg.c"),
    ("ext", "pub fn is_pdf_image(&mut self, img: i32) -> bool", "writeimg.c"),
    ("ext", "pub fn is_png_image(&mut self, img: i32) -> bool", "writeimg.c"),
    ("ext", "pub fn epdf_orig_x(&mut self, img: i32) -> i32", "writeimg.c"),
    ("ext", "pub fn epdf_orig_y(&mut self, img: i32) -> i32", "writeimg.c"),
    ("ext", "pub fn image_width(&mut self, img: i32) -> i32", "writeimg.c"),
    ("ext", "pub fn image_height(&mut self, img: i32) -> i32", "writeimg.c"),
    ("ext", "pub fn image_rotate(&mut self, img: i32) -> i32", "writeimg.c"),
    ("ext", "pub fn image_pages(&mut self, img: i32) -> i32", "writeimg.c"),
    ("ext", "pub fn image_x_res(&mut self, img: i32) -> i32", "writeimg.c"),
    ("ext", "pub fn image_y_res(&mut self, img: i32) -> i32", "writeimg.c"),
    ("ext", "pub fn image_colordepth(&mut self, img: i32) -> i32", "writeimg.c"),
    ("ext", "pub fn get_image_group_ref(&mut self, img: i32) -> i32", "writeimg.c"),
    ("ext", "pub fn set_image_group_ref(&mut self, img: i32, r: i32)", "writeimg.c"),
    ("ext", "pub fn read_image(&mut self, s: i32, page_num: i32, page_name: i32, colorspace: i32, page_box: i32, pdfmajorversion: i32, pdfminorversion: i32, inclusion_errorlevel: i32) -> i32", "writeimg.c; JPEG now, PNG and PDF deferred"),
    ("ext", "pub fn delete_image(&mut self, img: i32)", "writeimg.c"),
    ("ext", "pub fn update_image_procset(&mut self, img: i32)", "writeimg.c"),
    ("ext", "pub fn write_image(&mut self, img: i32)", "writeimg.c"),
    ("image", "pub fn undump_image_meta(&mut self, pdfmajorversion: i32, pdfminorversion: i32, pdfinclusionerrorlevel: i32)", "writeimg.c: the arena is in the snapshot, so normally no-op"),
]
EXTERN_NAMES = {re.search(r"pub fn ([A-Za-z_0-9]+)", e[1]).group(1) for e in EXTERNS}


# ---------------------------------------------------------------------------
# Stubs.
# ---------------------------------------------------------------------------
HAND_MODULES = {"toks"}  # written by hand, with their stubs included
MODULE_NOTES = {
    "copy": ["DROPPED §218: delete_token_ref is in toks.rs; token_ref_count is in the toks header."],
    "web2c": [
        "MLTeX is dropped: effective_char returns c; §1881-1894 need no marker anywhere.",
        "DROPPED §1930: src-specials are dropped; get_nullstr stays, at §1931.",
    ],
}


def gen_module(w, gen, module):
    ranges = section_ranges(module)
    what = "; ".join("§%d-%d %s" % (a, b, wh) for a, b, wh in ranges)
    out = [
        "//! " + what + ".",
        "//!",
        "//! Stubs generated by tools/webgen.py; replace them in place.  See docs/PORTING.md.  A section",
        "//! not ported must be listed here, as `DROPPED §N: reason` or `DROPPED §N-M: reason`.",
        "//! A procedure includes every module reachable from it, wherever defined: `tools/web.py tangle NAME`.",
    ]
    for line in MODULE_NOTES.get(module, []):
        out.append("//! " + line)
    out += [
        "#![forbid(unsafe_code)]",
        "#![deny(unreachable_patterns)]",
        "#![allow(non_snake_case, unused_variables, unused_mut, unused_imports, dead_code)]",
        "",
        "use crate::access::*;",
        "use crate::consts::*;",
        "use crate::host::*;",
        "use crate::raw::*;",
        "use crate::state::*;",
        "",
        "crate::pod_struct! {",
        "    /// POD state private to this module.  Add fields here, not to state.rs.",
        "    pub struct Vars {}",
        "}",
        "",
        "/// Non-POD state private to this module, in `self.host`.  Rarely needed.",
        "#[derive(Default)]",
        "pub struct HVars {}",
        "",
        "impl Tex {",
    ]
    n = 0
    for p in w.procs:
        if p.forward or p.name in IMPLEMENTED_ELSEWHERE and p.name != "flush_list":
            continue
        if module_of(p.sec) != module:
            continue
        if p.name not in gen.procs or gen.procs[p.name] is not p:
            continue
        note = SIG_NOTE.get(p.name)
        out.append("    /// §%d" % p.sec)
        if note:
            out.append("    /// " + note)
        if p.name == "flush_list":
            out.append("    /// One-word mem lists only (§764, §765, §1274); token lists use toks.rs's flush_list.")
        out.append("    %s {" % stub_signature(p))
        out.append('        todo!("§%d %s")' % (p.sec, p.name))
        out.append("    }")
        out.append("")
        n += 1
    ex = [e for e in EXTERNS if e[0] == module]
    if ex:
        out.append("    // ---- External functions: the C side of web2c and pdfTeX, called by WEB. ----")
        out.append("")
        for _m, sig, note in ex:
            fn = re.search(r"pub fn ([A-Za-z_0-9]+)", sig).group(1)
            out.append("    /// External: %s" % note)
            out.append("    %s {" % sig)
            out.append('        todo!("external %s")' % fn)
            out.append("    }")
            out.append("")
    if out[-1] == "":
        out.pop()
    out.append("}")
    return "\n".join(out) + "\n", n


def gen_lib(w):
    out = [
        "//! rstex-engine: the port of pdfTeX.  One crate, since WEB's call graph is one strongly",
        "//! connected component.  See docs/DESIGN.md and docs/PORTING.md.",
        "#![allow(non_snake_case, special_module_name, clippy::all)]",
        "",
        "#[macro_use]",
        "pub mod raw;",
        "pub mod access;",
        "pub mod consts;",
        "pub mod host;",
        "pub mod platform;",
        "pub mod state;",
        "",
    ]
    for m in ALL_MODULES:
        out.append("pub mod %s;" % m)
    out.append("")
    return "\n".join(out)


def write(path, text):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w", encoding="utf-8") as f:
        f.write(text)


def main():
    w = weblib.Web()
    ev = Evaluator(w)
    consts_text, consts = gen_consts(w, ev)
    globs = parse_globals(w, ev)
    gen = Gen(w, ev, consts, globs)
    do_stubs = "--stubs" in sys.argv
    access_text, done, failed = gen_access(w, ev, gen)
    write(os.path.join(ENGINE, "consts.rs"), consts_text)
    write(os.path.join(ENGINE, "access.rs"), access_text)
    write(os.path.join(ENGINE, "state.rs"), gen_state(w, ev, gen, globs))
    write(os.path.join(ENGINE, "lib.rs"), gen_lib(w))
    total = 0
    if do_stubs:
        for m in ALL_MODULES:
            if m in HAND_MODULES:
                continue
            text, n = gen_module(w, gen, m)
            total += n
            write(os.path.join(ENGINE, m + ".rs"), text)
    import json
    write(os.path.join(ROOT, "tools", "generated.json"), json.dumps(
        dict(consts=sorted(consts), accessors=sorted(done), left_to_owner=failed,
             skipped_macros=sorted(SKIP_MACROS & set(w.macros))), indent=1))
    print("consts:", len(consts), "accessors:", len(done), "left to owners:", len(failed), "stubs:", total)


if __name__ == "__main__":
    main()
