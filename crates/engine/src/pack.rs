//! §816-855 Part 33: packaging.
//!
#![forbid(unsafe_code)]
#![deny(unreachable_patterns, unused_variables)]
#![allow(non_snake_case, unused_mut, unused_imports, dead_code)]

use crate::access::*;
use crate::consts::*;
use crate::host::*;
use crate::raw::*;
use crate::state::*;

/// Words in `Vars::subst_memo`, a power of 2.
const SUBST_MEMO_SIZE: usize = 1 << 10;
/// Entries in `Vars::char_ex_memo`, of two words each, a power of 2.
const CHAR_EX_MEMO_SIZE: usize = 1 << 11;

crate::pod_struct! {
    /// POD state private to this module.  Add fields here, not to state.rs.
    pub struct Vars {
        /// What §823's `do_subst_font` got from `expand_font` for a font and `ex_ratio*ef`: `x<<32 | f<<16 | k`, 0 for
        /// nothing.  A line's characters and try_break's margin characters ask the same few things millions of times,
        /// and the answer depends on nothing else until a font's expansion changes: `pack_forget_subst_memo`.
        pub subst_memo: A<u64, SUBST_MEMO_SIZE>,
        /// The last `vpackage`'s height and depth, whole: section 1265 subtracts from them, and the box's words clamp near 2^31.
        pub vpackage_height: i64,
        pub vpackage_depth: i64,
        /// What `char_stretch` and `char_shrink` give for a font and character, which line_break and hpack ask for on
        /// every character of every paragraph: `(char_ex_epoch + 1)<<32 | f<<8 | c`, then `stretch<<32 | shrink`.
        pub char_ex_memo: A<u64, { 2 * CHAR_EX_MEMO_SIZE }>,
        /// Counts what could change an answer in `char_ex_memo`: an ef code, or what `pack_forget_subst_memo` is told of.
        pub char_ex_epoch: u32,
    }
}

/// Non-POD state private to this module, in `self.host`.  Rarely needed.
#[derive(Default)]
pub struct HVars {}

/// §108 `badness` in the oracle's `integer`, for §836, §843, §850 and §853, whose excess and total can pass 2^31 and
/// must not be clamped: `\hbox to\maxdimen` with five `plus \maxdimen` gives 1 on the oracle, and 12 from the clamped
/// total.  tests/integration/pack/limits_badness.tex.  `linebreak.rs` and `page.rs` use it for their `i64` sums.
pub(crate) fn pack_badness(t: i64, s: i64) -> i32 {
    if t == 0 {
        0
    } else if s <= 0 {
        INF_BAD
    } else {
        let r = if t <= 7230584 {
            (t * 297) / s
        } else if s >= 1663497 {
            t / (s / 297)
        } else {
            t
        };
        if r > 1290 { INF_BAD } else { ((r * r * r + 0o400000) / 0o1000000) as i32 }
    }
}

impl Tex {
    /// §817
    pub fn scan_spec(&mut self, c: i32, three_codes: bool) {
        let mut s = 0;
        let spec_code;
        if three_codes {
            s = self.saved(0);
        }
        'found: {
            if self.scan_keyword(b"to") {
                spec_code = EXACTLY;
            } else if self.scan_keyword(b"spread") {
                spec_code = ADDITIONAL;
            } else {
                spec_code = ADDITIONAL;
                self.cur_val = 0;
                break 'found;
            }
            self.scan_normal_dimen();
        }
        if three_codes {
            self.set_saved(0, s);
            self.save_ptr += 1;
        }
        self.set_saved(0, spec_code);
        self.set_saved(1, self.cur_val);
        self.save_ptr += 2;
        self.new_save_level(c);
        self.scan_left_brace();
    }

    /// §823
    pub fn check_expand_pars(&mut self, f: i32) -> bool {
        if self.pdf_font_step[f] == 0
            || (self.pdf_font_stretch[f] == NULL_FONT && self.pdf_font_shrink[f] == NULL_FONT)
        {
            return false;
        }
        if self.cur_font_step < 0 {
            self.cur_font_step = self.pdf_font_step[f];
        } else if self.cur_font_step != self.pdf_font_step[f] {
            self.pdf_error(
                b"font expansion",
                b"using fonts with different step of expansion in one paragraph is not allowed",
            );
        }
        let mut k = self.pdf_font_stretch[f];
        if k != NULL_FONT {
            if self.max_stretch_ratio < 0 {
                self.max_stretch_ratio = self.pdf_font_expand_ratio[k];
            } else if self.max_stretch_ratio != self.pdf_font_expand_ratio[k] {
                self.pdf_error(b"font expansion", b"using fonts with different limit of expansion in one paragraph is not allowed");
            }
        }
        k = self.pdf_font_shrink[f];
        if k != NULL_FONT {
            if self.max_shrink_ratio < 0 {
                self.max_shrink_ratio = -self.pdf_font_expand_ratio[k];
            } else if self.max_shrink_ratio != -self.pdf_font_expand_ratio[k] {
                self.pdf_error(b"font expansion", b"using fonts with different limit of expansion in one paragraph is not allowed");
            }
        }
        true
    }

    /// §823
    pub fn char_stretch(&mut self, f: i32, c: i32) -> i32 {
        let k = self.pdf_font_stretch[f];
        let ef = self.get_ef_code(f, c);
        if k != NULL_FONT && ef > 0 {
            let dw =
                self.char_width(k, self.char_info(k, c)) - self.char_width(f, self.char_info(f, c));
            if dw > 0 {
                return self.round_xn_over_d(dw, ef, 1000);
            }
        }
        0
    }

    /// §823
    pub fn char_shrink(&mut self, f: i32, c: i32) -> i32 {
        let k = self.pdf_font_shrink[f];
        let ef = self.get_ef_code(f, c);
        if k != NULL_FONT && ef > 0 {
            let dw =
                self.char_width(f, self.char_info(f, c)) - self.char_width(k, self.char_info(k, c));
            if dw > 0 {
                return self.round_xn_over_d(dw, ef, 1000);
            }
        }
        0
    }

    /// `(char_stretch(f, c), char_shrink(f, c))`.  They read a font's widths, which never change, and its ef code and
    /// its two expanded fonts, which say so to `char_ex_epoch`.  A `dw` so large that `round_xn_over_d` sets `arith_error`
    /// is not kept, so it is worked out each time, and sets it each time.
    #[inline]
    pub(crate) fn pack_char_expansion(&mut self, f: i32, c: i32) -> (i32, i32) {
        let key = ((self.v_pack.char_ex_epoch as u64 + 1) << 32) | ((f as u64) << 8) | c as u64;
        let slot = 2 * ((f.wrapping_mul(43) + c) as usize & (CHAR_EX_MEMO_SIZE - 1)) as i32;
        if self.v_pack.char_ex_memo[slot] == key {
            let w = self.v_pack.char_ex_memo[slot + 1];
            return ((w >> 32) as i32, w as i32);
        }
        let (st, sh) = (self.char_stretch(f, c), self.char_shrink(f, c));
        if !self.arith_error {
            self.v_pack.char_ex_memo[slot] = key;
            self.v_pack.char_ex_memo[slot + 1] = ((st as u32 as u64) << 32) | sh as u32 as u64;
        }
        (st, sh)
    }

    /// §823
    pub fn get_kern(&mut self, f: i32, lc: i32, rc: i32) -> i32 {
        let i = self.char_info(f, lc);
        if self.char_tag(i) != LIG_TAG {
            return 0;
        }
        let mut k = self.lig_kern_start(f, i);
        let mut j = self.font_info[k].qqqq();
        if self.skip_byte(j) > STOP_FLAG {
            k = self.lig_kern_restart(f, j);
            j = self.font_info[k].qqqq();
        }
        // WEB's `continue+1` is the head of this loop, and its `continue` reads the word at the end.
        loop {
            if self.next_char(j) == rc
                && self.skip_byte(j) <= STOP_FLAG
                && self.op_byte(j) >= KERN_FLAG
            {
                return self.char_kern(f, j);
            }
            if self.skip_byte(j) == 0 {
                k += 1;
            } else {
                if self.skip_byte(j) >= STOP_FLAG {
                    return 0;
                }
                k = k + self.skip_byte(j) + 1;
            }
            j = self.font_info[k].qqqq();
        }
    }

    /// §823
    pub fn kern_stretch(&mut self, p: i32) -> i32 {
        if self.prev_char_p == NULL || self.link(self.prev_char_p) != p || self.link(p) == NULL {
            return 0;
        }
        let mut l = self.prev_char_p;
        let mut r = self.link(p);
        if !self.is_char_node(l) {
            if self.ty(l) == LIGATURE_NODE {
                l = self.lig_char(l);
            } else {
                return 0;
            }
        }
        if !self.is_char_node(r) {
            if self.ty(r) == LIGATURE_NODE {
                r = self.lig_char(r);
            } else {
                return 0;
            }
        }
        if !(self.font(l) == self.font(r) && self.pdf_font_stretch[self.font(l)] != NULL_FONT) {
            return 0;
        }
        let d = self.get_kern(
            self.pdf_font_stretch[self.font(l)],
            self.character(l),
            self.character(r),
        );
        let ef = self.get_ef_code(self.font(l), self.character(l));
        self.round_xn_over_d(d - self.width(p), ef, 1000)
    }

    /// §823
    pub fn kern_shrink(&mut self, p: i32) -> i32 {
        if self.prev_char_p == NULL || self.link(self.prev_char_p) != p || self.link(p) == NULL {
            return 0;
        }
        let mut l = self.prev_char_p;
        let mut r = self.link(p);
        if !self.is_char_node(l) {
            if self.ty(l) == LIGATURE_NODE {
                l = self.lig_char(l);
            } else {
                return 0;
            }
        }
        if !self.is_char_node(r) {
            if self.ty(r) == LIGATURE_NODE {
                r = self.lig_char(r);
            } else {
                return 0;
            }
        }
        if !(self.font(l) == self.font(r) && self.pdf_font_shrink[self.font(l)] != NULL_FONT) {
            return 0;
        }
        let d = self.get_kern(
            self.pdf_font_shrink[self.font(l)],
            self.character(l),
            self.character(r),
        );
        let ef = self.get_ef_code(self.font(l), self.character(l));
        self.round_xn_over_d(self.width(p) - d, ef, 1000)
    }

    /// §823
    pub fn do_subst_font(&mut self, p: i32, ex_ratio: i32) {
        let mut r;
        if !self.is_char_node(p) && self.ty(p) == DISC_NODE {
            r = self.pre_break(p);
            while r != NULL {
                if self.is_char_node(r) || self.ty(r) == LIGATURE_NODE {
                    self.do_subst_font(r, ex_ratio);
                }
                r = self.link(r);
            }
            r = self.post_break(p);
            while r != NULL {
                if self.is_char_node(r) || self.ty(r) == LIGATURE_NODE {
                    self.do_subst_font(r, ex_ratio);
                }
                r = self.link(r);
            }
            return;
        }
        if self.is_char_node(p) {
            r = p;
        } else if self.ty(p) == LIGATURE_NODE {
            r = self.lig_char(p);
        } else {
            self.pdf_error(b"font expansion", b"invalid node type");
        }
        let f = self.font(r);
        let ef = self.get_ef_code(f, self.character(r));
        if ef == 0 {
            return;
        }
        let k;
        if (self.pdf_font_stretch[f] != NULL_FONT && ex_ratio > 0) || (self.pdf_font_shrink[f] != NULL_FONT && ex_ratio < 0) {
            k = self.pack_subst_expanded(f, ex_ratio * ef);
        } else {
            k = f;
        }
        if k != f {
            self.set_font(r, k);
            if !self.is_char_node(p) {
                r = self.lig_ptr(p);
                while r != NULL {
                    self.set_font(r, k);
                    r = self.link(r);
                }
            }
        }
    }

    /// The word of `Vars::subst_memo` for `f` and `x`, and what is in it if it is about them.
    #[inline(always)]
    fn pack_subst_slot(f: i32, x: i32) -> (i32, u64) {
        let key = ((x as u32 as u64) << 32) | ((f as u64) << 16);
        let slot = ((((x as u32) ^ ((f as u32) << 20)).wrapping_mul(0x9E3779B1)) >> (32 - SUBST_MEMO_SIZE.trailing_zeros())) as i32;
        (slot, key)
    }

    /// The font `do_subst_font` would put on character `c` of font `f`, if it can say so with nothing to do.
    pub(crate) fn pack_subst_known(&self, f: i32, c: i32, ex_ratio: i32) -> Option<i32> {
        let ef = self.pdf_font_ef_base[f];
        let ef = if ef == 0 { 1000 } else { self.pdf_mem[ef + c] };
        if ef == 0 || (if ex_ratio > 0 { self.pdf_font_stretch[f] } else { self.pdf_font_shrink[f] }) == NULL_FONT {
            return Some(f);
        }
        let (slot, key) = Tex::pack_subst_slot(f, ex_ratio * ef);
        let w = self.v_pack.subst_memo[slot];
        if w & !0xffff == key { Some((w & 0xffff) as i32) } else { None }
    }

    /// §823 `expand_font(f, ext_xn_over_d(x, limit, 1000000))` for `x = ex_ratio*ef`, whose sign says which limit.  `ef`
    /// is at most 1000 and more than 0 here, `ex_ratio` is between -1000 and 1000, and `limit` at most 1000, so the
    /// warning of `ext_xn_over_d` cannot come; the guard on the store keeps out what would break that.
    #[inline]
    fn pack_subst_expanded(&mut self, f: i32, x: i32) -> i32 {
        let (slot, key) = Tex::pack_subst_slot(f, x);
        let w = self.v_pack.subst_memo[slot];
        if w & !0xffff == key {
            return (w & 0xffff) as i32;
        }
        let limit = if x > 0 {
            self.pdf_font_expand_ratio[self.pdf_font_stretch[f]]
        } else {
            -self.pdf_font_expand_ratio[self.pdf_font_shrink[f]]
        };
        let e = self.ext_xn_over_d(x, limit, 1000000);
        let k = self.expand_font(f, e);
        if k != NULL_FONT && abs(x) <= 1000000 && abs(limit) <= 1000 {
            self.v_pack.subst_memo[slot] = key | k as u64;
        }
        k
    }

    /// A font's expansion changed, or the list of its expanded fonts.
    pub(crate) fn pack_forget_subst_memo(&mut self) {
        self.v_pack.subst_memo.0.fill(0);
        self.v_pack.char_ex_epoch += 1;
    }

    /// §823
    pub fn char_pw(&mut self, p: i32, side: i32) -> i32 {
        let mut p = p;
        if side == LEFT_SIDE {
            self.last_leftmost_char = NULL;
        } else {
            self.last_rightmost_char = NULL;
        }
        if p == NULL {
            return 0;
        }
        if !self.is_char_node(p) {
            if self.ty(p) == LIGATURE_NODE {
                p = self.lig_char(p);
            } else {
                return 0;
            }
        }
        let f = self.font(p);
        let c;
        if side == LEFT_SIDE {
            c = self.get_lp_code(f, self.character(p));
            self.last_leftmost_char = p;
        } else {
            c = self.get_rp_code(f, self.character(p));
            self.last_rightmost_char = p;
        }
        if c == 0 {
            return 0;
        }
        self.round_xn_over_d(self.quad(f), c, 1000)
    }

    /// §823
    pub fn new_margin_kern(&mut self, w: i32, p: i32, side: i32) -> i32 {
        let k = self.get_node(MARGIN_KERN_NODE_SIZE);
        self.set_ty(k, MARGIN_KERN_NODE);
        self.set_subtype(k, side);
        self.set_width(k, w);
        if p == NULL {
            self.pdf_error(b"margin kerning", b"invalid pointer to marginal char node");
        }
        let c = self.fast_get_avail();
        self.set_margin_char(k, c);
        self.set_character(c, self.character(p));
        self.set_font(c, self.font(p));
        k
    }

    /// §823
    ///
    /// `x`, `h`, `d`, `w`, `s` and the font totals are `i64`, for the oracle's 64-bit `scaled`: PORTING section 11.
    pub fn hpack(&mut self, p: i32, w: i32, m: i32) -> i32 {
        let mut p = p;
        let mut w = w as i64;
        let mut q: i32;
        let mut s: i64;
        let mut o: i32;
        let mut font_stretch = 0i64;
        let mut font_shrink = 0i64;
        self.last_badness = 0;
        let mut r = self.get_node(BOX_NODE_SIZE);
        self.set_ty(r, HLIST_NODE);
        self.set_subtype(r, MIN_QUARTERWORD);
        self.set_shift_amount(r, 0);
        q = r + LIST_OFFSET;
        self.set_link(q, p);
        if m == CAL_EXPAND_RATIO {
            self.prev_char_p = NULL;
            self.font_expand_ratio = 0;
        }
        let mut h = 0i64;
        // §824 Clear dimensions to zero
        let mut d = 0i64;
        let mut x = 0i64;
        self.total_stretch[NORMAL] = 0;
        self.total_shrink[NORMAL] = 0;
        self.total_stretch[FIL] = 0;
        self.total_shrink[FIL] = 0;
        self.total_stretch[FILL] = 0;
        self.total_shrink[FILL] = 0;
        self.total_stretch[FILLL] = 0;
        self.total_shrink[FILLL] = 0;
        if self.TeXXeT_en() {
            // §1710 Initialize the LR stack
            self.put_LR(BEFORE); // this will never match
        }
        // §825 Examine node |p| in the hlist
        while p != NULL {
            'reswitch: loop {
                while self.is_char_node(p) {
                    // §828 Incorporate character dimensions into the dimensions of the hbox
                    if m >= CAL_EXPAND_RATIO {
                        self.prev_char_p = p;
                        match m {
                            CAL_EXPAND_RATIO => {
                                let f = self.font(p);
                                let c = self.character(p);
                                let (st, sh) = self.pack_char_expansion(f, c);
                                font_stretch += st as i64;
                                font_shrink += sh as i64;
                            }
                            SUBST_EX_FONT => self.do_subst_font(p, self.font_expand_ratio),
                            _ => {}
                        }
                    }
                    let f = self.font(p);
                    let i = self.char_info(f, self.character(p));
                    let hd = self.height_depth(i);
                    x += self.char_width(f, i) as i64;
                    s = self.char_height(f, hd) as i64;
                    if s > h {
                        h = s;
                    }
                    s = self.char_depth(f, hd) as i64;
                    if s > d {
                        d = s;
                    }
                    p = self.link(p);
                }
                if p != NULL {
                    match self.ty(p) {
                        HLIST_NODE | VLIST_NODE | RULE_NODE | UNSET_NODE => {
                            // §827 Incorporate box dimensions into the dimensions of the hbox
                            x += self.width(p) as i64;
                            if self.ty(p) >= RULE_NODE {
                                s = 0;
                            } else {
                                s = self.shift_amount(p) as i64;
                            }
                            if self.height(p) as i64 - s > h {
                                h = self.height(p) as i64 - s;
                            }
                            if self.depth(p) as i64 + s > d {
                                d = self.depth(p) as i64 + s;
                            }
                        }
                        INS_NODE | MARK_NODE | ADJUST_NODE => {
                            if self.adjust_tail != NULL || self.pre_adjust_tail != NULL {
                                // §831 Transfer node |p| to the adjustment list
                                while self.link(q) != p {
                                    q = self.link(q);
                                }
                                if self.ty(p) == ADJUST_NODE {
                                    if self.adjust_pre(p) != 0 {
                                        // update_adjust_list(pre_adjust_tail)
                                        if self.pre_adjust_tail == NULL {
                                            self.confusion(b"pre vadjust");
                                        }
                                        self.set_link(self.pre_adjust_tail, self.adjust_ptr(p));
                                        while self.link(self.pre_adjust_tail) != NULL {
                                            self.pre_adjust_tail = self.link(self.pre_adjust_tail);
                                        }
                                    } else {
                                        // update_adjust_list(adjust_tail)
                                        if self.adjust_tail == NULL {
                                            self.confusion(b"pre vadjust");
                                        }
                                        self.set_link(self.adjust_tail, self.adjust_ptr(p));
                                        while self.link(self.adjust_tail) != NULL {
                                            self.adjust_tail = self.link(self.adjust_tail);
                                        }
                                    }
                                    p = self.link(p);
                                    self.free_node(self.link(q), SMALL_NODE_SIZE);
                                } else {
                                    self.set_link(self.adjust_tail, p);
                                    self.adjust_tail = p;
                                    p = self.link(p);
                                }
                                self.set_link(q, p);
                                p = q;
                            }
                        }
                        WHATSIT_NODE => {
                            // §1607 Incorporate a whatsit node into an hbox
                            if self.subtype(p) == PDF_REFXFORM_NODE
                                || self.subtype(p) == PDF_REFXIMAGE_NODE
                            {
                                x += self.pdf_width(p) as i64;
                                s = 0;
                                if self.pdf_height(p) as i64 - s > h {
                                    h = self.pdf_height(p) as i64 - s;
                                }
                                if self.pdf_depth(p) as i64 + s > d {
                                    d = self.pdf_depth(p) as i64 + s;
                                }
                            }
                        }
                        GLUE_NODE => {
                            // §832 Incorporate glue into the horizontal totals
                            let mut g = self.glue_ptr(p);
                            x += self.width(g) as i64;
                            o = self.stretch_order(g);
                            self.total_stretch[o] += self.stretch(g) as i64;
                            o = self.shrink_order(g);
                            self.total_shrink[o] += self.shrink(g) as i64;
                            if self.subtype(p) >= A_LEADERS {
                                g = self.leader_ptr(p);
                                if self.height(g) as i64 > h {
                                    h = self.height(g) as i64;
                                }
                                if self.depth(g) as i64 > d {
                                    d = self.depth(g) as i64;
                                }
                            }
                        }
                        MARGIN_KERN_NODE => {
                            if m == CAL_EXPAND_RATIO {
                                let c = self.margin_char(p);
                                let f = self.font(c);
                                self.do_subst_font(c, 1000);
                                if f != self.font(c) {
                                    font_stretch = font_stretch
                                        - self.width(p) as i64
                                        - self.char_pw(c, self.subtype(p)) as i64;
                                }
                                self.set_font(c, f);
                                self.do_subst_font(c, -1000);
                                if f != self.font(c) {
                                    font_shrink = font_shrink
                                        - self.width(p) as i64
                                        - self.char_pw(c, self.subtype(p)) as i64;
                                }
                                self.set_font(c, f);
                            } else if m == SUBST_EX_FONT {
                                let c = self.margin_char(p);
                                self.do_subst_font(c, self.font_expand_ratio);
                                let pw = self.char_pw(c, self.subtype(p));
                                self.set_width(p, -pw);
                            }
                            x += self.width(p) as i64;
                        }
                        KERN_NODE => {
                            if self.subtype(p) == NORMAL {
                                if m == CAL_EXPAND_RATIO {
                                    font_stretch += self.kern_stretch(p) as i64;
                                    font_shrink += self.kern_shrink(p) as i64;
                                } else if m == SUBST_EX_FONT {
                                    let mut k = 0;
                                    if self.font_expand_ratio > 0 {
                                        k = self.kern_stretch(p);
                                    } else if self.font_expand_ratio < 0 {
                                        k = self.kern_shrink(p);
                                    } else {
                                        self.pdfassert(false);
                                    }
                                    if k != 0 {
                                        let pc = self.prev_char_p;
                                        let nx = self.link(p);
                                        if self.is_char_node(nx) {
                                            let v = self.get_kern(
                                                self.font(pc),
                                                self.character(pc),
                                                self.character(nx),
                                            );
                                            self.set_width(p, v);
                                        } else if self.ty(nx) == LIGATURE_NODE {
                                            let lc = self.lig_char(nx);
                                            let v = self.get_kern(
                                                self.font(pc),
                                                self.character(pc),
                                                self.character(lc),
                                            );
                                            self.set_width(p, v);
                                        }
                                    }
                                }
                            }
                            x += self.width(p) as i64;
                        }
                        MATH_NODE => {
                            x += self.width(p) as i64;
                            if self.TeXXeT_en() {
                                // §1711 Adjust the LR stack for the |hpack| routine
                                if self.end_LR(p) {
                                    if self.info(self.LR_ptr) == self.end_LR_type(p) {
                                        self.pop_LR();
                                    } else {
                                        self.LR_problems += 1;
                                        self.set_ty(p, KERN_NODE);
                                        self.set_subtype(p, EXPLICIT);
                                    }
                                } else {
                                    self.push_LR(p);
                                }
                            }
                        }
                        LIGATURE_NODE => {
                            if m == SUBST_EX_FONT {
                                self.do_subst_font(p, self.font_expand_ratio);
                            }
                            // §826 Make node |p| look like a |char_node| and |goto reswitch|
                            self.mem[LIG_TRICK] = self.mem[self.lig_char(p)];
                            self.set_link(LIG_TRICK, self.link(p));
                            p = LIG_TRICK;
                            continue 'reswitch;
                        }
                        DISC_NODE => {
                            if m == SUBST_EX_FONT {
                                self.do_subst_font(p, self.font_expand_ratio);
                            }
                        }
                        _ => {}
                    }
                    p = self.link(p);
                }
                break;
            }
        }
        if self.adjust_tail != NULL {
            self.set_link(self.adjust_tail, NULL);
        }
        if self.pre_adjust_tail != NULL {
            self.set_link(self.pre_adjust_tail, NULL);
        }
        self.set_height(r, sat32(h));
        self.set_depth(r, sat32(d));
        // §833 Determine the value of |width(r)| and the appropriate glue setting
        let mut common_ending = 'determine: {
            if m == ADDITIONAL {
                w = x + w;
            }
            self.set_width(r, sat32(w));
            x = w - x; // now |x| is the excess to be made up
            if x == 0 {
                self.set_glue_sign(r, NORMAL);
                self.set_glue_order(r, NORMAL);
                self.set_glue_set(r, 0.0);
                break 'determine false;
            } else if x > 0 {
                // §834 Determine horizontal glue stretch setting
                // §835 Determine the stretch order
                if self.total_stretch[FILLL] != 0 {
                    o = FILLL;
                } else if self.total_stretch[FILL] != 0 {
                    o = FILL;
                } else if self.total_stretch[FIL] != 0 {
                    o = FIL;
                } else {
                    o = NORMAL;
                }
                if m == CAL_EXPAND_RATIO && o == NORMAL && font_stretch > 0 {
                    self.font_expand_ratio = self.divide_scaled(sat32(x), sat32(font_stretch), 3);
                    break 'determine false;
                }
                self.set_glue_order(r, o);
                self.set_glue_sign(r, STRETCHING);
                if self.total_stretch[o] != 0 {
                    self.set_glue_set(r, x as f64 / self.total_stretch[o] as f64);
                } else {
                    self.set_glue_sign(r, NORMAL);
                    self.set_glue_set(r, 0.0); // there's nothing to stretch
                }
                if o == NORMAL && self.list_ptr(r) != NULL {
                    // §836 Report an underfull hbox and |goto common_ending|, if this box is sufficiently bad
                    self.last_badness = pack_badness(x, self.total_stretch[NORMAL]);
                    if self.last_badness > self.hbadness() {
                        self.print_ln();
                        if self.last_badness > 100 {
                            self.print_nl_s(b"Underfull");
                        } else {
                            self.print_nl_s(b"Loose");
                        }
                        self.print_s(b" \\hbox (badness ");
                        self.print_int(self.last_badness);
                        break 'determine true;
                    }
                }
                break 'determine false;
            } else {
                // §840 Determine horizontal glue shrink setting
                // §841 Determine the shrink order
                if self.total_shrink[FILLL] != 0 {
                    o = FILLL;
                } else if self.total_shrink[FILL] != 0 {
                    o = FILL;
                } else if self.total_shrink[FIL] != 0 {
                    o = FIL;
                } else {
                    o = NORMAL;
                }
                if m == CAL_EXPAND_RATIO && o == NORMAL && font_shrink > 0 {
                    self.font_expand_ratio = self.divide_scaled(sat32(x), sat32(font_shrink), 3);
                    break 'determine false;
                }
                self.set_glue_order(r, o);
                self.set_glue_sign(r, SHRINKING);
                if self.total_shrink[o] != 0 {
                    self.set_glue_set(r, (-x) as f64 / self.total_shrink[o] as f64);
                } else {
                    self.set_glue_sign(r, NORMAL);
                    self.set_glue_set(r, 0.0); // there's nothing to shrink
                }
                if self.total_shrink[o] < -x && o == NORMAL && self.list_ptr(r) != NULL {
                    self.last_badness = 1000000;
                    self.set_glue_set(r, 1.0); // use the maximum shrinkage
                    // §842 Report an overfull hbox and |goto common_ending|, if this box is sufficiently bad
                    let over = -x - self.total_shrink[NORMAL];
                    if over > self.hfuzz() as i64 || self.hbadness() < 100 {
                        if self.overfull_rule() > 0 && over > self.hfuzz() as i64 {
                            while self.link(q) != NULL {
                                q = self.link(q);
                            }
                            let nr = self.new_rule();
                            self.set_link(q, nr);
                            self.set_width(nr, self.overfull_rule());
                        }
                        self.print_ln();
                        self.print_nl_s(b"Overfull \\hbox (");
                        self.print_scaled(over);
                        self.print_s(b"pt too wide");
                        break 'determine true;
                    }
                } else if o == NORMAL && self.list_ptr(r) != NULL {
                    // §843 Report a tight hbox and |goto common_ending|, if this box is sufficiently bad
                    self.last_badness = pack_badness(-x, self.total_shrink[NORMAL]);
                    if self.last_badness > self.hbadness() {
                        self.print_ln();
                        self.print_nl_s(b"Tight \\hbox (badness ");
                        self.print_int(self.last_badness);
                        break 'determine true;
                    }
                }
                break 'determine false;
            }
        };
        // §1713 goes back from `exit:` to `common_ending:`, so these two are a loop.
        loop {
            if common_ending {
                // §839 Finish issuing a diagnostic message for an overfull or underfull hbox
                if self.output_active {
                    self.print_s(b") has occurred while \\output is active");
                } else {
                    if self.pack_begin_line != 0 {
                        if self.pack_begin_line > 0 {
                            self.print_s(b") in paragraph at lines ");
                        } else {
                            self.print_s(b") in alignment at lines ");
                        }
                        self.print_int(abs(self.pack_begin_line));
                        self.print_s(b"--");
                    } else {
                        self.print_s(b") detected at line ");
                    }
                    self.print_int(self.line);
                }
                self.print_ln();
                self.font_in_short_display = NULL_FONT;
                self.short_display(self.list_ptr(r));
                self.print_ln();
                self.begin_diagnostic();
                self.show_box(r);
                self.end_diagnostic(true);
            }
            // exit:
            if self.TeXXeT_en() {
                // §1712 Check for LR anomalies at the end of |hpack|
                if self.info(self.LR_ptr) != BEFORE {
                    while self.link(q) != NULL {
                        q = self.link(q);
                    }
                    loop {
                        self.temp_ptr = q;
                        q = self.new_math(0, self.info(self.LR_ptr));
                        self.set_link(self.temp_ptr, q);
                        self.LR_problems += 10000;
                        self.pop_LR();
                        if self.info(self.LR_ptr) == BEFORE {
                            break;
                        }
                    }
                }
                if self.LR_problems > 0 {
                    // §1713 Report LR problems
                    self.print_ln();
                    self.print_nl_s(b"\\endL or \\endR problem (");
                    self.print_int(self.LR_problems / 10000);
                    self.print_s(b" missing, ");
                    self.print_int(self.LR_problems % 10000);
                    self.print_s(b" extra");
                    self.LR_problems = 0;
                    common_ending = true;
                    continue;
                }
                self.pop_LR();
                if self.LR_ptr != NULL {
                    self.confusion(b"LR1");
                }
            }
            break;
        }
        if m == CAL_EXPAND_RATIO && self.font_expand_ratio != 0 {
            self.font_expand_ratio = self.fix_int(self.font_expand_ratio, -1000, 1000);
            q = self.list_ptr(r);
            self.free_node(r, BOX_NODE_SIZE);
            r = self.hpack(q, sat32(w), SUBST_EX_FONT);
        }
        r
    }

    /// §844
    ///
    /// `x`, `w`, `d`, `h` and `s` are `i64`, for the oracle's 64-bit `scaled`: PORTING section 11.
    pub fn vpackage(&mut self, p: i32, h: i32, m: i32, l: i32) -> i32 {
        let mut p = p;
        let mut h = h as i64;
        let l = l as i64;
        let mut s: i64;
        let mut o: i32;
        self.last_badness = 0;
        let r = self.get_node(BOX_NODE_SIZE);
        self.set_ty(r, VLIST_NODE);
        self.set_subtype(r, MIN_QUARTERWORD);
        self.set_shift_amount(r, 0);
        self.set_list_ptr(r, p);
        let mut w = 0i64;
        // §824 Clear dimensions to zero
        let mut d = 0i64;
        let mut x = 0i64;
        self.total_stretch[NORMAL] = 0;
        self.total_shrink[NORMAL] = 0;
        self.total_stretch[FIL] = 0;
        self.total_shrink[FIL] = 0;
        self.total_stretch[FILL] = 0;
        self.total_shrink[FILL] = 0;
        self.total_stretch[FILLL] = 0;
        self.total_shrink[FILLL] = 0;
        // §845 Examine node |p| in the vlist
        while p != NULL {
            if self.is_char_node(p) {
                self.confusion(b"vpack");
            } else {
                match self.ty(p) {
                    HLIST_NODE | VLIST_NODE | RULE_NODE | UNSET_NODE => {
                        // §846 Incorporate box dimensions into the dimensions of the vbox
                        x = x + d + self.height(p) as i64;
                        d = self.depth(p) as i64;
                        if self.ty(p) >= RULE_NODE {
                            s = 0;
                        } else {
                            s = self.shift_amount(p) as i64;
                        }
                        if self.width(p) as i64 + s > w {
                            w = self.width(p) as i64 + s;
                        }
                    }
                    WHATSIT_NODE => {
                        // §1606 Incorporate a whatsit node into a vbox
                        if self.subtype(p) == PDF_REFXFORM_NODE
                            || self.subtype(p) == PDF_REFXIMAGE_NODE
                        {
                            x = x + d + self.pdf_height(p) as i64;
                            d = self.pdf_depth(p) as i64;
                            s = 0;
                            if self.pdf_width(p) as i64 + s > w {
                                w = self.pdf_width(p) as i64 + s;
                            }
                        }
                    }
                    GLUE_NODE => {
                        // §847 Incorporate glue into the vertical totals
                        x += d;
                        d = 0;
                        let mut g = self.glue_ptr(p);
                        x += self.width(g) as i64;
                        o = self.stretch_order(g);
                        self.total_stretch[o] += self.stretch(g) as i64;
                        o = self.shrink_order(g);
                        self.total_shrink[o] += self.shrink(g) as i64;
                        if self.subtype(p) >= A_LEADERS {
                            g = self.leader_ptr(p);
                            if self.width(g) as i64 > w {
                                w = self.width(g) as i64;
                            }
                        }
                    }
                    KERN_NODE => {
                        x = x + d + self.width(p) as i64;
                        d = 0;
                    }
                    _ => {}
                }
            }
            p = self.link(p);
        }
        self.set_width(r, sat32(w));
        if d > l {
            x = x + d - l;
            d = l;
        }
        self.set_depth(r, sat32(d));
        self.v_pack.vpackage_depth = d;
        // §848 Determine the value of |height(r)| and the appropriate glue setting
        let common_ending = 'determine: {
            if m == ADDITIONAL {
                h = x + h;
            }
            self.set_height(r, sat32(h));
            self.v_pack.vpackage_height = h;
            x = h - x; // now |x| is the excess to be made up
            if x == 0 {
                self.set_glue_sign(r, NORMAL);
                self.set_glue_order(r, NORMAL);
                self.set_glue_set(r, 0.0);
                break 'determine false;
            } else if x > 0 {
                // §849 Determine vertical glue stretch setting
                // §835 Determine the stretch order
                if self.total_stretch[FILLL] != 0 {
                    o = FILLL;
                } else if self.total_stretch[FILL] != 0 {
                    o = FILL;
                } else if self.total_stretch[FIL] != 0 {
                    o = FIL;
                } else {
                    o = NORMAL;
                }
                self.set_glue_order(r, o);
                self.set_glue_sign(r, STRETCHING);
                if self.total_stretch[o] != 0 {
                    self.set_glue_set(r, x as f64 / self.total_stretch[o] as f64);
                } else {
                    self.set_glue_sign(r, NORMAL);
                    self.set_glue_set(r, 0.0); // there's nothing to stretch
                }
                if o == NORMAL && self.list_ptr(r) != NULL {
                    // §850 Report an underfull vbox and |goto common_ending|, if this box is sufficiently bad
                    self.last_badness = pack_badness(x, self.total_stretch[NORMAL]);
                    if self.last_badness > self.vbadness() {
                        self.print_ln();
                        if self.last_badness > 100 {
                            self.print_nl_s(b"Underfull");
                        } else {
                            self.print_nl_s(b"Loose");
                        }
                        self.print_s(b" \\vbox (badness ");
                        self.print_int(self.last_badness);
                        break 'determine true;
                    }
                }
                break 'determine false;
            } else {
                // §852 Determine vertical glue shrink setting
                // §841 Determine the shrink order
                if self.total_shrink[FILLL] != 0 {
                    o = FILLL;
                } else if self.total_shrink[FILL] != 0 {
                    o = FILL;
                } else if self.total_shrink[FIL] != 0 {
                    o = FIL;
                } else {
                    o = NORMAL;
                }
                self.set_glue_order(r, o);
                self.set_glue_sign(r, SHRINKING);
                if self.total_shrink[o] != 0 {
                    self.set_glue_set(r, (-x) as f64 / self.total_shrink[o] as f64);
                } else {
                    self.set_glue_sign(r, NORMAL);
                    self.set_glue_set(r, 0.0); // there's nothing to shrink
                }
                if self.total_shrink[o] < -x && o == NORMAL && self.list_ptr(r) != NULL {
                    self.last_badness = 1000000;
                    self.set_glue_set(r, 1.0); // use the maximum shrinkage
                    // §853 Report an overfull vbox and |goto common_ending|, if this box is sufficiently bad
                    let over = -x - self.total_shrink[NORMAL];
                    if over > self.vfuzz() as i64 || self.vbadness() < 100 {
                        self.print_ln();
                        self.print_nl_s(b"Overfull \\vbox (");
                        self.print_scaled(over);
                        self.print_s(b"pt too high");
                        break 'determine true;
                    }
                } else if o == NORMAL && self.list_ptr(r) != NULL {
                    // §854 Report a tight vbox and |goto common_ending|, if this box is sufficiently bad
                    self.last_badness = pack_badness(-x, self.total_shrink[NORMAL]);
                    if self.last_badness > self.vbadness() {
                        self.print_ln();
                        self.print_nl_s(b"Tight \\vbox (badness ");
                        self.print_int(self.last_badness);
                        break 'determine true;
                    }
                }
                break 'determine false;
            }
        };
        if common_ending {
            // §851 Finish issuing a diagnostic message for an overfull or underfull vbox
            if self.output_active {
                self.print_s(b") has occurred while \\output is active");
            } else {
                if self.pack_begin_line != 0 {
                    // it's actually negative
                    self.print_s(b") in alignment at lines ");
                    self.print_int(abs(self.pack_begin_line));
                    self.print_s(b"--");
                } else {
                    self.print_s(b") detected at line ");
                }
                self.print_int(self.line);
                self.print_ln();
            }
            self.begin_diagnostic();
            self.show_box(r);
            self.end_diagnostic(true);
        }
        r
    }

    /// §855
    pub fn append_to_vlist(&mut self, b: i32) {
        if self.prev_depth() > self.pdf_ignored_dimen() {
            // `d` is `i64`: a box height clamped at 2^31-1, less a positive \prevdepth, must not wrap.
            let d = self.width(self.baseline_skip()) as i64
                - self.prev_depth() as i64
                - self.height(b) as i64;
            let p;
            if d < self.line_skip_limit() as i64 {
                p = self.new_param_glue(LINE_SKIP_CODE);
            } else {
                p = self.new_skip_param(BASELINE_SKIP_CODE);
                self.set_width(self.temp_ptr, sat32(d)); // |temp_ptr=glue_ptr(p)|
            }
            self.set_link(self.tail(), p);
            self.set_tail(p);
        }
        self.set_link(self.tail(), b);
        self.set_tail(b);
        self.set_prev_depth(self.depth(b));
    }

    // ---- External functions: the C side of web2c and pdfTeX, called by WEB. ----

    /// External: utils.c extxnoverd: computes in doubles, with its own rounding and a warning, not
    /// arith_error.  `r` at or below `DBL_EPSILON` subtracts one half, so 0 stays 0 and -0.4 gives 0.
    /// The oracle's result is 64 bits: past `i32` this clamps, after the same warning.
    pub fn ext_xn_over_d(&mut self, x: i32, n: i32, d: i32) -> i32 {
        let mut r = x as f64 * n as f64 / d as f64;
        if r > f64::EPSILON {
            r += 0.5;
        } else {
            r -= 0.5;
        }
        if r >= 2147483647.0 || r <= -2147483647.0 {
            // pdftex_warn, utils.c, without its optional " (file x)", which only a font reader sets.
            self.print_ln();
            self.print_ln();
            self.print_s(b"pdfTeX warning: ");
            let name = rstex_kpse::invocation_name();
            self.print_s(&name);
            self.print_s(b": arithmetic: number too big");
            self.print_ln();
        }
        r as i32
    }

    /// External: pdftex.h: font_info base + c
    pub fn get_lp_code(&mut self, f: i32, c: i32) -> i32 {
        if self.pdf_font_lp_base[f] == 0 {
            0
        } else {
            self.pdf_mem[self.pdf_font_lp_base[f] + c]
        }
    }

    /// External: pdftex.h
    pub fn get_rp_code(&mut self, f: i32, c: i32) -> i32 {
        if self.pdf_font_rp_base[f] == 0 {
            0
        } else {
            self.pdf_mem[self.pdf_font_rp_base[f] + c]
        }
    }

    /// External: pdftex.h
    pub fn get_ef_code(&mut self, f: i32, c: i32) -> i32 {
        if self.pdf_font_ef_base[f] == 0 {
            1000
        } else {
            self.pdf_mem[self.pdf_font_ef_base[f] + c]
        }
    }
}

/// The boxes of `tests/oracle/gen_pack_dvi_tables.py`, built by the same calls TeX makes.  Used here and in dvi.rs.
#[cfg(test)]
pub(crate) mod packkit {
    use super::*;
    use crate::show::showkit::*;

    pub struct Row {
        pub name: String,
        pub params: String,
        pub spec: String,
        pub line: i32,
        pub packed: String,
        pub dims: Vec<i32>,
        pub shipped: String,
        /// The length of the DVI up to `post`, and its FNV-1a.
        pub dvi_len: usize,
        pub dvi_fnv: u32,
        pub post: Vec<i32>,
    }

    /// As the generator computes it.
    pub fn fnv(data: &[u8]) -> u32 {
        data.iter().fold(2166136261, |h: u32, &b| (h ^ b as u32).wrapping_mul(16777619))
    }

    fn numbers(s: &str) -> Vec<i32> {
        s.split_whitespace().map(|x| x.parse().unwrap()).collect()
    }

    pub fn raw_rows() -> Vec<Vec<String>> {
        table(include_bytes!("../testdata/pack_dvi_oracle.tsv"))
    }

    pub fn rows() -> Vec<Row> {
        let mut v = Vec::new();
        for r in raw_rows() {
            if r[0].starts_with('@') {
                continue;
            }
            v.push(Row {
                name: r[0].clone(),
                params: r[1].clone(),
                spec: r[2].clone(),
                line: r[3].parse().unwrap(),
                packed: r[4].clone(),
                dims: numbers(&r[5]),
                shipped: r[6].clone(),
                dvi_len: r[7].split_whitespace().next().map_or(0, |x| x.parse().unwrap()),
                dvi_fnv: r[7].split_whitespace().nth(1).map_or(0, |x| x.parse().unwrap()),
                post: numbers(&r[8]),
            });
        }
        v
    }

    /// Words of `font_info` a font's tables are given.
    const FONT_WORDS: i32 = 256 + 3 * 16;

    /// What `read_font_info` would have left, for the characters the cases use: `@font` and `@char`.
    fn install_fonts(t: &mut Tex) {
        let mut base = 1;
        for (f, id) in [(XX, "xx"), (EE, "ee")] {
            t.char_base[f] = base;
            t.width_base[f] = base + 256;
            t.height_base[f] = base + 256 + 16;
            t.depth_base[f] = base + 256 + 32;
            t.font_bc[f] = 0;
            t.font_ec[f] = 255;
            t.font_area[f] = S_EMPTY;
            let mut n = 1;
            for r in raw_rows() {
                if r[0] == "@font" && r[1] == id {
                    let c = numbers(&r[2]);
                    t.font_check[f] = FourQuarters {
                        b0: c[0],
                        b1: c[1],
                        b2: c[2],
                        b3: c[3],
                    };
                    t.font_size[f] = r[3].parse().unwrap();
                    t.font_dsize[f] = t.font_size[f];
                }
                if r[0] == "@char" && r[1] == id {
                    let code: i32 = r[2].parse().unwrap();
                    let m = [
                        r[3].parse::<i32>().unwrap(),
                        r[4].parse().unwrap(),
                        r[5].parse().unwrap(),
                    ];
                    t.font_info[t.char_base[f] + code].set_qqqq(FourQuarters {
                        b0: n,
                        b1: n * 16 + n,
                        b2: 0,
                        b3: 0,
                    });
                    t.font_info[t.width_base[f] + n].set_sc(m[0]);
                    t.font_info[t.height_base[f] + n].set_sc(m[1]);
                    t.font_info[t.depth_base[f] + n].set_sc(m[2]);
                    n += 1;
                }
            }
            assert!(n <= 15);
            base += FONT_WORDS;
        }
    }

    /// INITEX's state as far as these procedures read it, with the preamble of the generator's cases, and an in-memory
    /// DVI file: `Slot::Pdf` keeps what it is given, and `write_dvi` takes any slot, as `put_bytes` does.
    pub fn pack_machine(params: &str) -> &'static mut Tex {
        let t = case_machine("pack", true);
        t.nest_size = 100;
        t.set_mode(VMODE);
        t.set_tail(CONTRIB_HEAD);
        t.set_head(CONTRIB_HEAD);
        t.set_prev_depth(IGNORE_DEPTH);
        t.set_pdf_ignored_dimen(IGNORE_DEPTH);
        t.set_mag(1000);
        t.set_box_max_depth(MAX_DIMEN);
        t.set_count(0, 1);
        t.set_year(2026);
        t.set_month(11);
        t.set_day(2);
        t.set_time(754);
        t.set_tracing_output(1);
        t.dvi_buf_size = 16384;
        t.last_bop = -1;
        t.cur_s = -1;
        t.output_file_name = 1;
        t.host.files[DVI_FILE as usize] = Slot::Pdf(PdfOut::new(b"unused"));
        install_fonts(t);
        for kv in params.split(',') {
            let (k, v) = kv.split_once('=').unwrap_or((kv, ""));
            let n: i32 = v.parse().unwrap_or(0);
            match k {
                "hbadness" => t.set_hbadness(n),
                "vbadness" => t.set_vbadness(n),
                "hfuzz" => t.set_hfuzz(n),
                "vfuzz" => t.set_vfuzz(n),
                "overfullrule" => t.set_overfull_rule(n),
                "boxmaxdepth" => t.set_box_max_depth(n),
                "baselineskip" | "lineskip" => {
                    let s = t.new_spec(ZERO_GLUE);
                    t.set_width(s, n);
                    t.set_glue_par(
                        if k == "lineskip" {
                            LINE_SKIP_CODE
                        } else {
                            BASELINE_SKIP_CODE
                        },
                        s,
                    );
                }
                "lineskiplimit" => t.set_line_skip_limit(n),
                "hoffset" => t.set_h_offset(n),
                "voffset" => t.set_v_offset(n),
                "texxet" => t.set_TeXXeT_state(n),
                "etex" => t.eTeX_mode = n,
                "dvibuf" => t.dvi_buf_size = n,
                "notrace" => t.set_tracing_output(0),
                "noship" | "-" => {}
                _ => panic!("{}", k),
            }
        }
        t.half_buf = t.dvi_buf_size / 2;
        t.dvi_limit = t.dvi_buf_size;
        t
    }

    struct Scan<'a> {
        t: Vec<&'a str>,
        i: usize,
    }
    impl<'a> Scan<'a> {
        fn word(&mut self) -> &'a str {
            self.i += 1;
            self.t[self.i - 1]
        }
        fn num(&mut self) -> i32 {
            self.word().parse().unwrap()
        }
        fn peek(&self) -> &'a str {
            self.t[self.i]
        }
    }

    /// `N`, or `Nfil`, `Nfill`, `Nfilll`.
    fn order(w: &str) -> (i32, i32) {
        for (suffix, o) in [("filll", FILLL), ("fill", FILL), ("fil", FIL)] {
            if let Some(n) = w.strip_suffix(suffix) {
                return (n.parse::<i32>().unwrap() * UNITY, o);
            }
        }
        (w.parse().unwrap(), NORMAL)
    }

    fn append(t: &mut Tex, p: i32) {
        t.set_link(t.tail(), p);
        t.set_tail(p);
    }

    /// scan_rule_spec, with what it leaves in a field that is not given.
    fn rule(t: &mut Tex, s: &mut Scan, vert: bool) -> i32 {
        let q = t.new_rule();
        if vert {
            t.set_height(q, DEFAULT_RULE);
            t.set_depth(q, 0);
        } else {
            t.set_width(q, DEFAULT_RULE);
        }
        let (w, h, d) = (s.word(), s.word(), s.word());
        if w != "*" {
            t.set_width(q, w.parse().unwrap());
        }
        if h != "*" {
            t.set_height(q, h.parse().unwrap());
        }
        if d != "*" {
            t.set_depth(q, d.parse().unwrap());
        }
        q
    }

    /// append_glue.
    fn glue(t: &mut Tex, s: &mut Scan) -> i32 {
        let sp = t.new_spec(ZERO_GLUE);
        let w = s.num();
        let (st, so) = order(s.word());
        let (sh, sho) = order(s.word());
        t.set_width(sp, w);
        t.set_stretch(sp, st);
        t.set_stretch_order(sp, so);
        t.set_shrink(sp, sh);
        t.set_shrink_order(sp, sho);
        let p = t.new_glue(sp);
        t.set_glue_ref_count(sp, NULL);
        p
    }

    fn char_node(t: &mut Tex, f: i32, c: i32) -> i32 {
        let p = t.get_avail();
        t.set_font(p, f);
        t.set_character(p, c);
        p
    }

    fn font_of(w: &str) -> i32 {
        if w == "e" { EE } else { XX }
    }

    /// box_end for one box, and package at its end.
    fn scan_box(t: &mut Tex, s: &mut Scan) -> i32 {
        let vert = s.word() == "v";
        let (mut w, mut m) = (0, ADDITIONAL);
        if s.peek() == "to" || s.peek() == "spread" {
            if s.word() == "to" {
                m = EXACTLY;
            }
            w = s.num();
        }
        assert_eq!(s.word(), "{");
        t.push_nest();
        t.set_mode(if vert { -VMODE } else { -HMODE });
        if vert {
            t.set_prev_depth(t.pdf_ignored_dimen());
        }
        let mut shift = 0;
        while s.peek() != "}" {
            if s.peek() == "h" || s.peek() == "v" {
                let b = scan_box(t, s);
                t.set_shift_amount(b, shift);
                shift = 0;
                if vert {
                    t.append_to_vlist(b);
                } else {
                    append(t, b);
                }
                continue;
            }
            match s.word() {
                "s" => shift = s.num(),
                "r" => {
                    let q = rule(t, s, vert);
                    append(t, q);
                    if vert {
                        t.set_prev_depth(t.pdf_ignored_dimen());
                    }
                }
                "k" => {
                    let n = s.num();
                    let q = t.new_kern(n);
                    t.set_subtype(q, EXPLICIT);
                    append(t, q);
                }
                "g" => {
                    let q = glue(t, s);
                    append(t, q);
                }
                "L" => {
                    let sub = match s.word() {
                        "a" => A_LEADERS,
                        "c" => C_LEADERS,
                        _ => X_LEADERS,
                    };
                    let q = glue(t, s);
                    let item = if s.peek() == "r" {
                        s.word();
                        rule(t, s, vert)
                    } else {
                        scan_box(t, s)
                    };
                    t.set_subtype(q, sub);
                    t.set_leader_ptr(q, item);
                    append(t, q);
                }
                "c" => {
                    let f = font_of(s.word());
                    let c = s.num();
                    let q = char_node(t, f, c);
                    append(t, q);
                }
                "l" => {
                    let f = font_of(s.word());
                    let (n, a, b) = (s.num(), s.num(), s.num());
                    let (x, y) = (char_node(t, f, a), char_node(t, f, b));
                    t.set_link(x, y);
                    let q = t.new_ligature(f, n, x);
                    append(t, q);
                }
                "M" => {
                    let n = s.num();
                    let q = t.new_math(0, n);
                    append(t, q);
                }
                other => panic!("{}", other),
            }
        }
        s.word();
        let l = t.link(t.head());
        t.pop_nest();
        if vert {
            t.vpackage(l, w, m, t.box_max_depth())
        } else {
            t.hpack(l, w, m)
        }
    }

    pub fn build_box(t: &mut Tex, spec: &str) -> i32 {
        let mut s = Scan {
            t: spec.split_whitespace().collect(),
            i: 0,
        };
        let b = scan_box(t, &mut s);
        assert_eq!(s.i, s.t.len());
        b
    }
}

#[cfg(test)]
mod tests {
    use super::packkit::*;
    use super::*;
    use crate::show::showkit::*;
    use crate::toks::testkit::*;

    /// utils.c extxnoverd, and the font expansion call in section 823: a truncating or integer version
    /// picks another expanded font here, which changes `Tm` in the PDF.
    #[test]
    fn ext_xn_over_d_rounds_as_utils_c() {
        let t = Tex::alloc();
        // do_subst_font: ex_ratio*ef * limit / 1000000.  r=0.64 gives 1, not 0.
        assert_eq!(t.ext_xn_over_d(640 * 1000, 1, 1000000), 1);
        assert_eq!(t.ext_xn_over_d(400 * 1000, 1, 1000000), 0);
        assert_eq!(
            t.ext_xn_over_d(500 * 1000, 1, 1000000),
            1,
            "exactly one half rounds away from zero"
        );
        assert_eq!(t.ext_xn_over_d(0, 20, 1000000), 0);
        assert_eq!(t.ext_xn_over_d(1, 1, 1000000000), 0, "r>DBL_EPSILON");
        assert_eq!(t.ext_xn_over_d(-400 * 1000, 1, 1000000), 0);
        assert_eq!(t.ext_xn_over_d(-600 * 1000, 1, 1000000), -1);
        assert_eq!(t.ext_xn_over_d(-1000 * 1000, 20, 1000000), -20);
        assert_eq!(t.ext_xn_over_d(1000 * 1000, 20, 1000000), 20);
        // A double, not a 32-bit product: 2^30 * 300 / 7200, from \pdfximage sizes in section 1552.
        assert_eq!(t.ext_xn_over_d(1 << 30, 300, 7200), 44739243);
        Tex::dealloc(t);
    }

    /// What `hpack` and `vpackage` print, and the dimensions and `\badness` they leave, against the oracle, for the
    /// nodes that need neither font expansion nor a whatsit.  dvi.rs compares the DVI of the same boxes.
    #[test]
    fn packaging_matches_the_oracle() {
        let mut bad = Vec::new();
        let rows = rows();
        assert!(rows.len() >= 170);
        for row in &rows {
            let t = pack_machine(&row.params);
            t.line = row.line;
            let b = build_box(t, &row.spec);
            let got = printed(t);
            if got != row.packed {
                bad.push(format!(
                    "--- {} ({})\nwant {:?}\ngot  {:?}",
                    row.name, row.spec, row.packed, got
                ));
            }
            let dims = vec![t.width(b), t.height(b), t.depth(b), t.last_badness];
            if dims != row.dims {
                bad.push(format!(
                    "--- {} ({})\nwant {:?}\ngot  {:?}",
                    row.name, row.spec, row.dims, dims
                ));
            }
            Tex::dealloc(t);
        }
        assert!(bad.is_empty(), "{} differ:\n{}", bad.len(), bad.join("\n"));
    }

    /// PORTING section 11: past 2^31 the totals stay exact, the stored dimension clamps, and no sign flips.
    #[test]
    fn past_two_to_the_31_nothing_wraps() {
        let big = MAX_DIMEN;
        let t = pack_machine("-");
        // Four \hskip16383pt: the oracle's width is 65532pt.
        let spec = format!("h {{ k {b} k {b} k {b} k {b} }}", b = big);
        let b = build_box(t, &spec);
        assert_eq!(t.width(b), i32::MAX);
        assert_eq!(printed(t), "");

        // Exact totals: four times 2^30 of stretch, of which a quarter is wanted.  A clamped total would give a half.
        let spec = format!(
            "h to {} {{ g 0 {b} 0 g 0 {b} 0 g 0 {b} 0 g 0 {b} 0 }}",
            big,
            b = big
        );
        let b = build_box(t, &spec);
        assert_eq!(t.glue_set(b), 0.25);
        assert_eq!(t.glue_sign(b), STRETCHING);
        Tex::dealloc(t);

        // The natural height passes 2^31 and `to 0` shrinks: the excess is negative as the oracle's is, and the message
        // has the same number.
        let t = pack_machine("-");
        let spec = format!("v to 0 {{ k {b} k {b} k {b} g 0 0 5 }}", b = big);
        let b = build_box(t, &spec);
        assert_eq!(t.glue_sign(b), SHRINKING);
        assert!(printed(t).contains("Overfull \\vbox (49151.99988pt too high)"));
        Tex::dealloc(t);

        // section 855: the height of a clamped box less a positive \prevdepth.
        let t = pack_machine("baselineskip=0,lineskip=5,lineskiplimit=-100");
        t.set_prev_depth(big);
        let tall = t.new_null_box();
        t.set_height(tall, i32::MAX);
        let (tail, skip) = (t.tail(), t.line_skip());
        t.append_to_vlist(tall);
        let g = t.link(tail);
        assert_eq!(t.ty(g), GLUE_NODE);
        assert_eq!(
            t.glue_ptr(g),
            skip,
            "the deficiency is very negative, so \\lineskip"
        );
        assert_eq!(t.link(g), tall);
        Tex::dealloc(t);
    }

    /// section 823's get_lp_code and the like read `pdf_mem`, and have these defaults without a table: pdftex.h.
    #[test]
    fn codes_read_the_font_tables() {
        let t = Tex::alloc();
        assert_eq!(
            (
                t.get_lp_code(3, 65),
                t.get_rp_code(3, 65),
                t.get_ef_code(3, 65)
            ),
            (0, 0, 1000)
        );
        t.pdf_font_lp_base[3] = 100;
        t.pdf_font_rp_base[3] = 400;
        t.pdf_font_ef_base[3] = 700;
        t.pdf_mem[165] = -5;
        t.pdf_mem[465] = 6;
        t.pdf_mem[765] = 0;
        assert_eq!(
            (
                t.get_lp_code(3, 65),
                t.get_rp_code(3, 65),
                t.get_ef_code(3, 65)
            ),
            (-5, 6, 0)
        );
        Tex::dealloc(t);
    }

    /// section 823 get_kern on a hand-made lig/kern program: a restart, a skip, a stop, a ligature on the way, and a kern hit.
    #[test]
    fn get_kern_walks_the_program() {
        let t = Tex::alloc();
        let f = 1;
        t.char_base[f] = 100;
        t.kern_base[f] = 600 - KERN_BASE_OFFSET;
        t.lig_kern_base[f] = 300;
        let word = |t: &mut Tex, k: i32, b0: i32, b1: i32, b2: i32, b3: i32| {
            t.font_info[k].set_qqqq(FourQuarters { b0, b1, b2, b3 });
        };
        // Character 65 has its program at 0; character 66 has no tag; character 67 starts with a restart.
        word(t, 100 + 65, 1, 0, LIG_TAG, 0);
        word(t, 100 + 66, 1, 0, 0, 0);
        word(t, 100 + 67, 1, 0, LIG_TAG, 4);
        t.font_info[600].set_sc(777);
        t.font_info[601].set_sc(-888);
        word(t, 300, 0, 70, 0, 9); // a ligature with 70: op_byte below kern_flag
        word(t, 301, 1, 71, KERN_FLAG, 0); // kern 0 with 71, then skip one word
        word(t, 302, 0, 72, KERN_FLAG, 1); // skipped
        word(t, 303, STOP_FLAG, 73, KERN_FLAG, 1); // the last word: kern 1 with 73
        word(t, 304, STOP_FLAG + 1, 0, 1, 49); // a restart: 256*1+49 from lig_kern_base
        word(t, 300 + 256 + 49, STOP_FLAG, 74, KERN_FLAG, 1); // the target
        assert_eq!(t.get_kern(f, 66, 71), 0, "no lig/kern program");
        assert_eq!(t.get_kern(f, 65, 71), 777);
        assert_eq!(t.get_kern(f, 65, 73), -888);
        assert_eq!(t.get_kern(f, 65, 70), 0, "a ligature is passed over");
        assert_eq!(t.get_kern(f, 65, 72), 0, "a skipped word is not read");
        assert_eq!(t.get_kern(f, 65, 99), 0, "the stop");
        assert_eq!(t.get_kern(f, 67, 74), -888, "through the restart");
        Tex::dealloc(t);
    }

    /// section 823 new_margin_kern, the one-word cell of section 16, and char_pw's two sides.
    #[test]
    fn margin_kern_and_char_pw() {
        let t = pack_machine("-");
        let c = t.get_avail();
        t.set_font(c, XX);
        t.set_character(c, 97);
        let k = t.new_margin_kern(123, c, RIGHT_SIDE);
        assert_eq!(
            (t.ty(k), t.subtype(k), t.width(k)),
            (MARGIN_KERN_NODE, RIGHT_SIDE, 123)
        );
        let m = t.margin_char(k);
        assert!(t.is_char_node(m));
        assert_ne!(m, c);
        assert_eq!((t.font(m), t.character(m), t.link(m)), (XX, 97, NULL));

        // Nothing but the bookkeeping for a null pointer, or for a node that isn't a character or a ligature.
        t.last_leftmost_char = 1;
        t.last_rightmost_char = 1;
        assert_eq!(t.char_pw(NULL, LEFT_SIDE), 0);
        assert_eq!((t.last_leftmost_char, t.last_rightmost_char), (NULL, 1));
        assert_eq!(t.char_pw(k, RIGHT_SIDE), 0);
        assert_eq!(t.last_rightmost_char, NULL);
        // A character with no code: the side's last char is still recorded, and 0 is returned before round_xn_over_d.
        assert_eq!(t.char_pw(c, LEFT_SIDE), 0);
        assert_eq!(t.last_leftmost_char, c);
        let l = t.new_ligature(XX, 97, NULL);
        assert_eq!(t.char_pw(l, RIGHT_SIDE), 0);
        assert_eq!(t.last_rightmost_char, t.lig_char(l));
        Tex::dealloc(t);
    }

    /// section 823 check_expand_pars: its three answers and what it records.  The two pdf_errors are integration cases.
    #[test]
    fn check_expand_pars_records_the_limits() {
        let t = Tex::alloc();
        t.cur_font_step = -1;
        t.max_stretch_ratio = -1;
        t.max_shrink_ratio = -1;
        assert!(!t.check_expand_pars(1), "no step");
        t.pdf_font_step[1] = 5;
        assert!(!t.check_expand_pars(1), "no expanded fonts");
        assert_eq!(t.cur_font_step, -1);
        t.pdf_font_stretch[1] = 2;
        t.pdf_font_shrink[1] = 3;
        t.pdf_font_expand_ratio[2] = 20;
        t.pdf_font_expand_ratio[3] = -15;
        assert!(t.check_expand_pars(1));
        assert_eq!(
            (t.cur_font_step, t.max_stretch_ratio, t.max_shrink_ratio),
            (5, 20, 15)
        );
        assert!(t.check_expand_pars(1), "the same limits again");
        Tex::dealloc(t);
    }
}
