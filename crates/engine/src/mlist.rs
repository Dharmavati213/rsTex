//! §875-943 Parts 35-36: subroutines for math mode, typesetting math formulas.
//!
//! The oracle's `scaled` is 64 bits and ours is 32.  A box dimension can be near 2^31 here, and a font
//! parameter or `\delimiterfactor` can be anything, so what WEB adds or multiplies is computed in `i64`
//! and stored with `sat32`: see docs/PORTING.md, section 11.  `var_delimiter` takes a clamped `v`, so a delimiter asked to
//! be 90000pt stops at 2^31sp here: section 15, "Known divergences".
//!
//! WEB's `magic_offset` has no use here: the spacing table is `MATH_SPACING`, indexed directly, since there is no
//! string pool file.
#![forbid(unsafe_code)]
#![deny(unreachable_patterns, unused_variables)]
#![allow(non_snake_case, unused_mut, unused_imports, dead_code)]

use crate::access::*;
use crate::consts::*;
use crate::host::*;
use crate::raw::*;
use crate::state::*;

crate::pod_struct! {
    /// POD state private to this module.  Add fields here, not to state.rs.
    pub struct Vars {}
}

/// Non-POD state private to this module, in `self.host`.  Rarely needed.
#[derive(Default)]
pub struct HVars {}

/// `a+b` on two scaled values.
#[inline(always)]
fn add_sat(a: i32, b: i32) -> i32 {
    sat32(a as i64 + b as i64)
}

/// `a-b` on two scaled values.
#[inline(always)]
fn sub_sat(a: i32, b: i32) -> i32 {
    sat32(a as i64 - b as i64)
}

/// WEB's `half` on an intermediate, which the oracle's 64-bit `half` takes whole.
#[inline(always)]
fn half64(x: i64) -> i64 {
    if x & 1 != 0 { (x + 1) / 2 } else { x / 2 }
}

impl Tex {
    /// §875
    pub fn print_size(&mut self, s: i32) {
        if s == TEXT_SIZE {
            self.print_esc_s(b"textfont");
        } else if s == SCRIPT_SIZE {
            self.print_esc_s(b"scriptfont");
        } else {
            self.print_esc_s(b"scriptscriptfont");
        }
    }

    /// §879 Set up the values of cur_size and cur_mu, based on cur_style
    #[inline]
    fn mlist_set_cur_size_and_mu(&mut self) {
        if self.cur_style < SCRIPT_STYLE {
            self.cur_size = TEXT_SIZE;
        } else {
            self.cur_size = 16 * ((self.cur_style - TEXT_STYLE) / 2);
        }
        self.cur_mu = self.x_over_n(self.math_quad(self.cur_size), 18);
    }

    /// §880
    pub fn fraction_rule(&mut self, t: i32) -> i32 {
        let p = self.new_rule();
        self.set_height(p, t);
        self.set_depth(p, 0);
        p
    }

    /// §881
    pub fn overbar(&mut self, b: i32, k: i32, t: i32) -> i32 {
        let mut p = self.new_kern(k);
        self.set_link(p, b);
        let q = self.fraction_rule(t);
        self.set_link(q, p);
        p = self.new_kern(t);
        self.set_link(p, q);
        self.vpack_natural(p)
    }

    /// §882
    pub fn var_delimiter(&mut self, d: i32, s: i32, v: i32) -> i32 {
        let mut f = NULL_FONT;
        let mut g: i32;
        let mut c = 0;
        let mut y: i32;
        let mut w: i64 = 0;
        let mut q = FourQuarters::default();
        let mut large_attempt = false;
        let mut z = self.small_fam(d);
        let mut x = self.small_char(d);
        'found: loop {
            // §883 Look at the variants of (z,x); set f and c whenever a better character is found
            if z != 0 || x != MIN_QUARTERWORD {
                z = z + s + 16;
                loop {
                    z -= 16;
                    g = self.fam_fnt(z);
                    if g != NULL_FONT {
                        // §884 Look at the list of characters starting with x in font g
                        y = x;
                        if y >= self.font_bc[g] && y <= self.font_ec[g] {
                            // continue:
                            loop {
                                q = self.orig_char_info(g, y);
                                if !self.char_exists(q) {
                                    break;
                                }
                                if self.char_tag(q) == EXT_TAG {
                                    f = g;
                                    c = y;
                                    break 'found;
                                }
                                let hd = self.height_depth(q);
                                let u = self.char_height(g, hd) as i64 + self.char_depth(g, hd) as i64;
                                if u > w {
                                    f = g;
                                    c = y;
                                    w = u;
                                    if u >= v as i64 {
                                        break 'found;
                                    }
                                }
                                if self.char_tag(q) != LIST_TAG {
                                    break;
                                }
                                y = self.rem_byte(q);
                            }
                        }
                    }
                    if z < 16 {
                        break;
                    }
                }
            }
            if large_attempt {
                break 'found; // there were none large enough
            }
            large_attempt = true;
            z = self.large_fam(d);
            x = self.large_char(d);
        }
        // found:
        let b;
        if f != NULL_FONT {
            // §886 Make variable b point to a box for (f,c)
            if self.char_tag(q) == EXT_TAG {
                // §889 Construct an extensible character in a new box b, using recipe rem_byte(q) and font f
                b = self.new_null_box();
                self.set_ty(b, VLIST_NODE);
                let r = self.font_info[self.exten_base[f] + self.rem_byte(q)].qqqq();
                // §890 Compute the minimum suitable height, w, and the corresponding number of extension steps, n
                c = self.ext_rep(r);
                let u = self.height_plus_depth(f, c) as i64;
                w = 0;
                q = self.char_info(f, c);
                self.set_width(b, add_sat(self.char_width(f, q), self.char_italic(f, q)));
                c = self.ext_bot(r);
                if c != MIN_QUARTERWORD {
                    w += self.height_plus_depth(f, c) as i64;
                }
                c = self.ext_mid(r);
                if c != MIN_QUARTERWORD {
                    w += self.height_plus_depth(f, c) as i64;
                }
                c = self.ext_top(r);
                if c != MIN_QUARTERWORD {
                    w += self.height_plus_depth(f, c) as i64;
                }
                let mut n = 0;
                if u > 0 {
                    while w < v as i64 {
                        w += u;
                        n += 1;
                        if self.ext_mid(r) != MIN_QUARTERWORD {
                            w += u;
                        }
                    }
                }
                c = self.ext_bot(r);
                if c != MIN_QUARTERWORD {
                    self.stack_into_box(b, f, c);
                }
                c = self.ext_rep(r);
                for _ in 1..=n {
                    self.stack_into_box(b, f, c);
                }
                c = self.ext_mid(r);
                if c != MIN_QUARTERWORD {
                    self.stack_into_box(b, f, c);
                    c = self.ext_rep(r);
                    for _ in 1..=n {
                        self.stack_into_box(b, f, c);
                    }
                }
                c = self.ext_top(r);
                if c != MIN_QUARTERWORD {
                    self.stack_into_box(b, f, c);
                }
                self.set_depth(b, sat32(w - self.height(b) as i64));
            } else {
                b = self.char_box(f, c);
            }
        } else {
            b = self.new_null_box();
            self.set_width(b, self.null_delimiter_space()); // use this width if no delimiter was found
        }
        let h = half64(self.height(b) as i64 - self.depth(b) as i64);
        self.set_shift_amount(b, sat32(h - self.axis_height(s) as i64));
        b
    }

    /// §885
    pub fn char_box(&mut self, f: i32, c: i32) -> i32 {
        let q = self.char_info(f, c);
        let hd = self.height_depth(q);
        let b = self.new_null_box();
        self.set_width(b, add_sat(self.char_width(f, q), self.char_italic(f, q)));
        self.set_height(b, self.char_height(f, hd));
        self.set_depth(b, self.char_depth(f, hd));
        let p = self.get_avail();
        self.set_character(p, c);
        self.set_font(p, f);
        self.set_list_ptr(b, p);
        b
    }

    /// §887
    pub fn stack_into_box(&mut self, b: i32, f: i32, c: i32) {
        let p = self.char_box(f, c);
        self.set_link(p, self.list_ptr(b));
        self.set_list_ptr(b, p);
        self.set_height(b, self.height(p));
    }

    /// §888
    pub fn height_plus_depth(&mut self, f: i32, c: i32) -> i32 {
        let q = self.char_info(f, c);
        let hd = self.height_depth(q);
        add_sat(self.char_height(f, hd), self.char_depth(f, hd))
    }

    /// §891
    pub fn rebox(&mut self, b: i32, w: i32) -> i32 {
        let mut b = b;
        if self.width(b) != w && self.list_ptr(b) != NULL {
            if self.ty(b) == VLIST_NODE {
                b = self.hpack_natural(b);
            }
            let mut p = self.list_ptr(b);
            if self.is_char_node(p) && self.link(p) == NULL {
                let f = self.font(p);
                let v = self.char_width(f, self.char_info(f, self.character(p)));
                if v != self.width(b) {
                    let k = self.new_kern(sub_sat(self.width(b), v));
                    self.set_link(p, k);
                }
            }
            self.free_node(b, BOX_NODE_SIZE);
            b = self.new_glue(SS_GLUE);
            self.set_link(b, p);
            while self.link(p) != NULL {
                p = self.link(p);
            }
            let g = self.new_glue(SS_GLUE);
            self.set_link(p, g);
            self.hpack(b, w, EXACTLY)
        } else {
            self.set_width(b, w);
            b
        }
    }

    /// §892 `mu_mult(x)`: `n` and `f` are the integer and fraction parts of the caller's `m`.
    #[inline]
    fn mu_mult(&mut self, n: i32, f: i32, x: i32) -> i32 {
        let y = self.xn_over_d(x, f, 65536);
        self.nx_plus_y(n, x, y)
    }

    /// §892
    pub fn math_glue(&mut self, g: i32, m: i32) -> i32 {
        let mut n = self.x_over_n(m, 65536);
        let mut f = self.remainder;
        if f < 0 {
            n -= 1;
            f += 65536;
        }
        let p = self.get_node(GLUE_SPEC_SIZE);
        let v = self.mu_mult(n, f, self.width(g));
        self.set_width(p, v); // convert mu to pt
        self.set_stretch_order(p, self.stretch_order(g));
        if self.stretch_order(p) == NORMAL {
            let v = self.mu_mult(n, f, self.stretch(g));
            self.set_stretch(p, v);
        } else {
            self.set_stretch(p, self.stretch(g));
        }
        self.set_shrink_order(p, self.shrink_order(g));
        if self.shrink_order(p) == NORMAL {
            let v = self.mu_mult(n, f, self.shrink(g));
            self.set_shrink(p, v);
        } else {
            self.set_shrink(p, self.shrink(g));
        }
        p
    }

    /// §893
    pub fn math_kern(&mut self, p: i32, m: i32) {
        if self.subtype(p) == MU_GLUE {
            let mut n = self.x_over_n(m, 65536);
            let mut f = self.remainder;
            if f < 0 {
                n -= 1;
                f += 65536;
            }
            let v = self.mu_mult(n, f, self.width(p));
            self.set_width(p, v);
            self.set_subtype(p, EXPLICIT);
        }
    }

    /// §894
    pub fn flush_math(&mut self) {
        self.flush_node_list(self.link(self.head()));
        self.flush_node_list(self.incompleat_noad());
        self.set_link(self.head(), NULL);
        self.set_tail(self.head());
        self.set_incompleat_noad(NULL);
    }

    /// §896
    pub fn clean_box(&mut self, p: i32, s: i32) -> i32 {
        let mut q: i32;
        'found: {
            match self.math_type(p) {
                MATH_CHAR => {
                    self.cur_mlist = self.new_noad();
                    let n = self.nucleus(self.cur_mlist);
                    self.mem[n] = self.mem[p];
                }
                SUB_BOX => {
                    q = self.info(p);
                    break 'found;
                }
                SUB_MLIST => self.cur_mlist = self.info(p),
                _ => {
                    q = self.new_null_box();
                    break 'found;
                }
            }
            let save_style = self.cur_style;
            self.cur_style = s;
            self.mlist_penalties = false;
            self.mlist_to_hlist(); // recursive call
            q = self.link(TEMP_HEAD);
            self.cur_style = save_style; // restore the style
            self.mlist_set_cur_size_and_mu(); // §879
        }
        // found:
        let x;
        if self.is_char_node(q) || q == NULL {
            x = self.hpack_natural(q);
        } else if self.link(q) == NULL && self.ty(q) <= VLIST_NODE && self.shift_amount(q) == 0 {
            x = q; // it's already clean
        } else {
            x = self.hpack_natural(q);
        }
        // §897 Simplify a trivial box
        q = self.list_ptr(x);
        if self.is_char_node(q) {
            let r = self.link(q);
            if r != NULL && self.link(r) == NULL && !self.is_char_node(r) && self.ty(r) == KERN_NODE {
                // unneeded italic correction
                self.free_node(r, MEDIUM_NODE_SIZE);
                self.set_link(q, NULL);
            }
        }
        x
    }

    /// §898
    pub fn fetch(&mut self, a: i32) {
        self.cur_c = self.character(a);
        self.cur_f = self.fam_fnt(self.fam(a) + self.cur_size);
        if self.cur_f == NULL_FONT {
            // §899 Complain about an undefined family and set cur_i null
            self.print_err_s(b"");
            self.print_size(self.cur_size);
            self.print_char(b' ' as i32);
            self.print_int(self.fam(a));
            self.print_s(b" is undefined (character ");
            self.print(self.cur_c);
            self.print_char(b')' as i32);
            self.help(&[
                b"Somewhere in the math formula just ended, you used the",
                b"stated character from an undefined font family. For example,",
                b"plain TeX doesn't allow \\it or \\sl in subscripts. Proceed,",
                b"and I'll try to forget that I needed that character.",
            ]);
            self.error();
            self.cur_i = self.null_character;
            self.set_math_type(a, EMPTY);
        } else {
            if self.cur_c >= self.font_bc[self.cur_f] && self.cur_c <= self.font_ec[self.cur_f] {
                self.cur_i = self.orig_char_info(self.cur_f, self.cur_c);
            } else {
                self.cur_i = self.null_character;
            }
            if !self.char_exists(self.cur_i) {
                self.char_warning(self.cur_f, self.cur_c);
                self.set_math_type(a, EMPTY);
                self.cur_i = self.null_character;
            }
        }
    }

    /// §902
    pub fn mlist_to_hlist(&mut self) {
        // Tuck the global parameters away as local variables.
        let mlist = self.cur_mlist;
        let penalties = self.mlist_penalties;
        let style = self.cur_style;
        let mut q = mlist;
        let mut r = NULL;
        let mut r_type = OP_NOAD;
        let mut t: i32;
        let mut p = NULL;
        let mut x: i32;
        let mut y: i32;
        let mut z: i32;
        let mut pen: i32;
        let mut s: i32;
        let mut max_h = 0;
        let mut max_d = 0;
        self.mlist_set_cur_size_and_mu(); // §879
        while q != NULL {
            // §903 Process node-or-noad q as much as possible in preparation for the second pass
            'done_with_node: {
                'done_with_noad: {
                    'check_dimensions: {
                        // §904 Do first-pass processing based on type(q)
                        let mut delta;
                        // reswitch:
                        loop {
                            delta = 0;
                            match self.ty(q) {
                                BIN_NOAD => match r_type {
                                    BIN_NOAD | OP_NOAD | REL_NOAD | OPEN_NOAD | PUNCT_NOAD | LEFT_NOAD => {
                                        self.set_ty(q, ORD_NOAD);
                                        continue;
                                    }
                                    _ => {}
                                },
                                REL_NOAD | CLOSE_NOAD | PUNCT_NOAD | RIGHT_NOAD => {
                                    // §905 Convert a final bin_noad to an ord_noad
                                    if r_type == BIN_NOAD {
                                        self.set_ty(r, ORD_NOAD);
                                    }
                                    if self.ty(q) == RIGHT_NOAD {
                                        break 'done_with_noad;
                                    }
                                }
                                // §909 Cases for noads that can follow a bin_noad
                                LEFT_NOAD => break 'done_with_noad,
                                FRACTION_NOAD => {
                                    self.make_fraction(q);
                                    break 'check_dimensions;
                                }
                                OP_NOAD => {
                                    delta = self.make_op(q);
                                    if self.subtype(q) == LIMITS {
                                        break 'check_dimensions;
                                    }
                                }
                                ORD_NOAD => self.make_ord(q),
                                OPEN_NOAD | INNER_NOAD => {}
                                RADICAL_NOAD => self.make_radical(q),
                                OVER_NOAD => self.make_over(q),
                                UNDER_NOAD => self.make_under(q),
                                ACCENT_NOAD => self.make_math_accent(q),
                                VCENTER_NOAD => self.make_vcenter(q),
                                // §906 Cases for nodes that can appear in an mlist, after which we goto done_with_node
                                STYLE_NODE => {
                                    self.cur_style = self.subtype(q);
                                    self.mlist_set_cur_size_and_mu(); // §879
                                    break 'done_with_node;
                                }
                                CHOICE_NODE => {
                                    // §907 Change this node to a style node followed by the correct choice
                                    match self.cur_style / 2 {
                                        0 => {
                                            p = self.display_mlist(q);
                                            self.set_display_mlist(q, NULL);
                                        }
                                        1 => {
                                            p = self.text_mlist(q);
                                            self.set_text_mlist(q, NULL);
                                        }
                                        2 => {
                                            p = self.script_mlist(q);
                                            self.set_script_mlist(q, NULL);
                                        }
                                        3 => {
                                            p = self.script_script_mlist(q);
                                            self.set_script_script_mlist(q, NULL);
                                        }
                                        _ => {} // there are no other cases
                                    }
                                    self.flush_node_list(self.display_mlist(q));
                                    self.flush_node_list(self.text_mlist(q));
                                    self.flush_node_list(self.script_mlist(q));
                                    self.flush_node_list(self.script_script_mlist(q));
                                    self.set_ty(q, STYLE_NODE);
                                    self.set_subtype(q, self.cur_style);
                                    self.set_width(q, 0);
                                    self.set_depth(q, 0);
                                    if p != NULL {
                                        z = self.link(q);
                                        self.set_link(q, p);
                                        while self.link(p) != NULL {
                                            p = self.link(p);
                                        }
                                        self.set_link(p, z);
                                    }
                                    break 'done_with_node;
                                }
                                INS_NODE | MARK_NODE | ADJUST_NODE | WHATSIT_NODE | PENALTY_NODE | DISC_NODE => {
                                    break 'done_with_node;
                                }
                                RULE_NODE => {
                                    if self.height(q) > max_h {
                                        max_h = self.height(q);
                                    }
                                    if self.depth(q) > max_d {
                                        max_d = self.depth(q);
                                    }
                                    break 'done_with_node;
                                }
                                GLUE_NODE => {
                                    // §908 Convert math glue to ordinary glue
                                    if self.subtype(q) == MU_GLUE {
                                        x = self.glue_ptr(q);
                                        y = self.math_glue(x, self.cur_mu);
                                        self.delete_glue_ref(x);
                                        self.set_glue_ptr(q, y);
                                        self.set_subtype(q, NORMAL);
                                    } else if self.cur_size != TEXT_SIZE && self.subtype(q) == COND_MATH_GLUE {
                                        p = self.link(q);
                                        if p != NULL && (self.ty(p) == GLUE_NODE || self.ty(p) == KERN_NODE) {
                                            self.set_link(q, self.link(p));
                                            self.set_link(p, NULL);
                                            self.flush_node_list(p);
                                        }
                                    }
                                    break 'done_with_node;
                                }
                                KERN_NODE => {
                                    self.math_kern(q, self.cur_mu);
                                    break 'done_with_node;
                                }
                                _ => self.confusion(b"mlist1"),
                            }
                            break;
                        }
                        // §930 Convert nucleus(q) to an hlist and attach the sub/superscripts
                        match self.math_type(self.nucleus(q)) {
                            MATH_CHAR | MATH_TEXT_CHAR => {
                                // §931 Create a character node p for nucleus(q), possibly followed by a kern for the
                                // italic correction, and set delta to the italic correction if a subscript is present
                                self.fetch(self.nucleus(q));
                                if self.char_exists(self.cur_i) {
                                    delta = self.char_italic(self.cur_f, self.cur_i);
                                    p = self.new_character(self.cur_f, self.cur_c);
                                    if self.math_type(self.nucleus(q)) == MATH_TEXT_CHAR && self.space(self.cur_f) != 0 {
                                        delta = 0; // no italic correction in mid-word of text font
                                    }
                                    if self.math_type(self.subscr(q)) == EMPTY && delta != 0 {
                                        let k = self.new_kern(delta);
                                        self.set_link(p, k);
                                        delta = 0;
                                    }
                                } else {
                                    p = NULL;
                                }
                            }
                            EMPTY => p = NULL,
                            SUB_BOX => p = self.info(self.nucleus(q)),
                            SUB_MLIST => {
                                self.cur_mlist = self.info(self.nucleus(q));
                                let save_style = self.cur_style;
                                self.mlist_penalties = false;
                                self.mlist_to_hlist(); // recursive call
                                self.cur_style = save_style;
                                self.mlist_set_cur_size_and_mu(); // §879
                                p = self.hpack_natural(self.link(TEMP_HEAD));
                            }
                            _ => self.confusion(b"mlist2"),
                        }
                        self.set_new_hlist(q, p);
                        if self.math_type(self.subscr(q)) == EMPTY && self.math_type(self.supscr(q)) == EMPTY {
                            break 'check_dimensions;
                        }
                        self.make_scripts(q, delta);
                    }
                    // check_dimensions:
                    z = self.hpack_natural(self.new_hlist(q));
                    if self.height(z) > max_h {
                        max_h = self.height(z);
                    }
                    if self.depth(z) > max_d {
                        max_d = self.depth(z);
                    }
                    self.free_node(z, BOX_NODE_SIZE);
                }
                // done_with_noad:
                r = q;
                r_type = self.ty(r);
                if r_type == RIGHT_NOAD {
                    r_type = LEFT_NOAD;
                    self.cur_style = style;
                    self.mlist_set_cur_size_and_mu(); // §879
                }
            }
            // done_with_node:
            q = self.link(q);
        }
        // §905 Convert a final bin_noad to an ord_noad
        if r_type == BIN_NOAD {
            self.set_ty(r, ORD_NOAD);
        }
        // §936 Make a second pass over the mlist, removing all noads and inserting the proper spacing and penalties
        p = TEMP_HEAD;
        self.set_link(p, NULL);
        q = mlist;
        r_type = 0;
        self.cur_style = style;
        self.mlist_set_cur_size_and_mu(); // §879
        while q != NULL {
            'done: {
                'delete_q: {
                    // §937 If node q is a style node, change the style and goto delete_q; otherwise if it is not a
                    // noad, put it into the hlist, advance q, and goto done; otherwise set s to the size of noad q,
                    // set t to the associated type, and set pen to the associated penalty
                    t = ORD_NOAD;
                    s = NOAD_SIZE;
                    pen = INF_PENALTY;
                    match self.ty(q) {
                        OP_NOAD | OPEN_NOAD | CLOSE_NOAD | PUNCT_NOAD | INNER_NOAD => t = self.ty(q),
                        BIN_NOAD => {
                            t = BIN_NOAD;
                            pen = self.bin_op_penalty();
                        }
                        REL_NOAD => {
                            t = REL_NOAD;
                            pen = self.rel_penalty();
                        }
                        ORD_NOAD | VCENTER_NOAD | OVER_NOAD | UNDER_NOAD => {}
                        RADICAL_NOAD => s = RADICAL_NOAD_SIZE,
                        ACCENT_NOAD => s = ACCENT_NOAD_SIZE,
                        FRACTION_NOAD => s = FRACTION_NOAD_SIZE,
                        LEFT_NOAD | RIGHT_NOAD => t = self.make_left_right(q, style, max_d, max_h),
                        STYLE_NODE => {
                            // §939 Change the current style and goto delete_q
                            self.cur_style = self.subtype(q);
                            s = STYLE_NODE_SIZE;
                            self.mlist_set_cur_size_and_mu(); // §879
                            break 'delete_q;
                        }
                        WHATSIT_NODE | PENALTY_NODE | RULE_NODE | DISC_NODE | ADJUST_NODE | INS_NODE | MARK_NODE
                        | GLUE_NODE | KERN_NODE => {
                            self.set_link(p, q);
                            p = q;
                            q = self.link(q);
                            self.set_link(p, NULL);
                            break 'done;
                        }
                        _ => self.confusion(b"mlist3"),
                    }
                    // §942 Append inter-element spacing based on r_type and t
                    if r_type > 0 {
                        // not the first noad
                        match MATH_SPACING[(r_type * 8 + t - 9 * ORD_NOAD) as usize] {
                            b'0' => x = 0,
                            b'1' => x = if self.cur_style < SCRIPT_STYLE { THIN_MU_SKIP_CODE } else { 0 },
                            b'2' => x = THIN_MU_SKIP_CODE,
                            b'3' => x = if self.cur_style < SCRIPT_STYLE { MED_MU_SKIP_CODE } else { 0 },
                            b'4' => x = if self.cur_style < SCRIPT_STYLE { THICK_MU_SKIP_CODE } else { 0 },
                            _ => self.confusion(b"mlist4"),
                        }
                        if x != 0 {
                            y = self.math_glue(self.glue_par(x), self.cur_mu);
                            z = self.new_glue(y);
                            self.set_glue_ref_count(y, NULL);
                            self.set_link(p, z);
                            p = z;
                            self.set_subtype(z, x + 1); // store a symbolic subtype
                        }
                    }
                    // §943 Append any new_hlist entries for q, and any appropriate penalties
                    if self.new_hlist(q) != NULL {
                        self.set_link(p, self.new_hlist(q));
                        loop {
                            p = self.link(p);
                            if self.link(p) == NULL {
                                break;
                            }
                        }
                    }
                    if penalties && self.link(q) != NULL && pen < INF_PENALTY {
                        r_type = self.ty(self.link(q));
                        if r_type != PENALTY_NODE && r_type != REL_NOAD {
                            z = self.new_penalty(pen);
                            self.set_link(p, z);
                            p = z;
                        }
                    }
                    if self.ty(q) == RIGHT_NOAD {
                        t = OPEN_NOAD;
                    }
                    r_type = t;
                }
                // delete_q:
                r = q;
                q = self.link(q);
                self.free_node(r, s);
            }
            // done:
        }
    }

    /// §910
    pub fn make_over(&mut self, q: i32) {
        let b = self.clean_box(self.nucleus(q), self.cramped_style(self.cur_style));
        let t = self.default_rule_thickness();
        let v = self.overbar(b, sat32(3 * t as i64), t);
        self.set_info(self.nucleus(q), v);
        self.set_math_type(self.nucleus(q), SUB_BOX);
    }

    /// §911
    pub fn make_under(&mut self, q: i32) {
        let x = self.clean_box(self.nucleus(q), self.cur_style);
        let t = self.default_rule_thickness();
        let p = self.new_kern(sat32(3 * t as i64));
        self.set_link(x, p);
        let r = self.fraction_rule(t);
        self.set_link(p, r);
        let y = self.vpack_natural(x);
        let delta = self.height(y) as i64 + self.depth(y) as i64 + t as i64;
        self.set_height(y, self.height(x));
        self.set_depth(y, sat32(delta - self.height(y) as i64));
        self.set_info(self.nucleus(q), y);
        self.set_math_type(self.nucleus(q), SUB_BOX);
    }

    /// §912
    pub fn make_vcenter(&mut self, q: i32) {
        let v = self.info(self.nucleus(q));
        if self.ty(v) != VLIST_NODE {
            self.confusion(b"vcenter");
        }
        let delta = self.height(v) as i64 + self.depth(v) as i64;
        self.set_height(v, sat32(self.axis_height(self.cur_size) as i64 + half64(delta)));
        self.set_depth(v, sat32(delta - self.height(v) as i64));
    }

    /// §913
    pub fn make_radical(&mut self, q: i32) {
        let x = self.clean_box(self.nucleus(q), self.cramped_style(self.cur_style));
        let t = self.default_rule_thickness() as i64;
        let mut clr: i64;
        if self.cur_style < TEXT_STYLE {
            // display style
            clr = t + abs(self.math_x_height(self.cur_size)) as i64 / 4;
        } else {
            clr = t;
            clr = clr + clr.abs() / 4;
        }
        let hx = self.height(x) as i64;
        let dx = self.depth(x) as i64;
        let y = self.var_delimiter(self.left_delimiter(q), self.cur_size, sat32(hx + dx + clr + t));
        let delta = self.depth(y) as i64 - (hx + dx + clr);
        if delta > 0 {
            clr += half64(delta); // increase the actual clearance
        }
        self.set_shift_amount(y, sat32(-(hx + clr)));
        let o = self.overbar(x, sat32(clr), self.height(y));
        self.set_link(y, o);
        let h = self.hpack_natural(y);
        self.set_info(self.nucleus(q), h);
        self.set_math_type(self.nucleus(q), SUB_BOX);
    }

    /// §914
    pub fn make_math_accent(&mut self, q: i32) {
        self.fetch(self.accent_chr(q));
        if self.char_exists(self.cur_i) {
            let mut i = self.cur_i;
            let mut c = self.cur_c;
            let f = self.cur_f;
            // §917 Compute the amount of skew
            let mut s = 0;
            if self.math_type(self.nucleus(q)) == MATH_CHAR {
                self.fetch(self.nucleus(q));
                if self.char_tag(self.cur_i) == LIG_TAG {
                    let mut a = self.lig_kern_start(self.cur_f, self.cur_i);
                    self.cur_i = self.font_info[a].qqqq();
                    if self.skip_byte(self.cur_i) > STOP_FLAG {
                        a = self.lig_kern_restart(self.cur_f, self.cur_i);
                        self.cur_i = self.font_info[a].qqqq();
                    }
                    loop {
                        if self.next_char(self.cur_i) == self.skew_char[self.cur_f] {
                            if self.op_byte(self.cur_i) >= KERN_FLAG && self.skip_byte(self.cur_i) <= STOP_FLAG {
                                s = self.char_kern(self.cur_f, self.cur_i);
                            }
                            break;
                        }
                        if self.skip_byte(self.cur_i) >= STOP_FLAG {
                            break;
                        }
                        a = a + self.skip_byte(self.cur_i) + 1;
                        self.cur_i = self.font_info[a].qqqq();
                    }
                }
            }
            // done1:
            let mut x = self.clean_box(self.nucleus(q), self.cramped_style(self.cur_style));
            let w = self.width(x);
            let mut h = self.height(x);
            // §916 Switch to a larger accent if available and appropriate
            loop {
                if self.char_tag(i) != LIST_TAG {
                    break;
                }
                let y = self.rem_byte(i);
                i = self.orig_char_info(f, y);
                if !self.char_exists(i) {
                    break;
                }
                if self.char_width(f, i) > w {
                    break;
                }
                c = y;
            }
            // done:
            let mut delta = if h < self.x_height(f) { h } else { self.x_height(f) };
            if (self.math_type(self.supscr(q)) != EMPTY || self.math_type(self.subscr(q)) != EMPTY)
                && self.math_type(self.nucleus(q)) == MATH_CHAR
            {
                // §918 Swap the subscript and superscript into box x
                self.flush_node_list(x);
                x = self.new_noad();
                let (nx, nq) = (self.nucleus(x), self.nucleus(q));
                self.mem[nx] = self.mem[nq];
                let (px, sq) = (self.supscr(x), self.supscr(q));
                self.mem[px] = self.mem[sq];
                let (bx, bq) = (self.subscr(x), self.subscr(q));
                self.mem[bx] = self.mem[bq];
                self.mem[sq] = self.empty_field;
                self.mem[bq] = self.empty_field;
                self.set_math_type(nq, SUB_MLIST);
                self.set_info(nq, x);
                x = self.clean_box(nq, self.cur_style);
                delta = sat32(delta as i64 + self.height(x) as i64 - h as i64);
                h = self.height(x);
            }
            let mut y = self.char_box(f, c);
            self.set_shift_amount(y, sat32(s as i64 + half64(w as i64 - self.width(y) as i64)));
            self.set_width(y, 0);
            let mut p = self.new_kern(-delta);
            self.set_link(p, x);
            self.set_link(y, p);
            y = self.vpack_natural(y);
            self.set_width(y, self.width(x));
            if self.height(y) < h {
                // §915 Make the height of box y equal to h
                p = self.new_kern(sub_sat(h, self.height(y)));
                self.set_link(p, self.list_ptr(y));
                self.set_list_ptr(y, p);
                self.set_height(y, h);
            }
            self.set_info(self.nucleus(q), y);
            self.set_math_type(self.nucleus(q), SUB_BOX);
        }
    }

    /// §919
    pub fn make_fraction(&mut self, q: i32) {
        if self.thickness(q) == DEFAULT_CODE {
            self.set_thickness(q, self.default_rule_thickness());
        }
        // §920 Create equal-width boxes x and z for the numerator and denominator, and compute the default
        // amounts shift_up and shift_down by which they are displaced from the baseline
        let mut x = self.clean_box(self.numerator(q), self.num_style(self.cur_style));
        let mut z = self.clean_box(self.denominator(q), self.denom_style(self.cur_style));
        if self.width(x) < self.width(z) {
            x = self.rebox(x, self.width(z));
        } else {
            z = self.rebox(z, self.width(x));
        }
        let mut shift_up: i64;
        let mut shift_down: i64;
        if self.cur_style < TEXT_STYLE {
            // display style
            shift_up = self.num1(self.cur_size) as i64;
            shift_down = self.denom1(self.cur_size) as i64;
        } else {
            shift_down = self.denom2(self.cur_size) as i64;
            if self.thickness(q) != 0 {
                shift_up = self.num2(self.cur_size) as i64;
            } else {
                shift_up = self.num3(self.cur_size) as i64;
            }
        }
        let t = self.thickness(q) as i64;
        let axis = self.axis_height(self.cur_size) as i64;
        let dx = self.depth(x) as i64;
        let hz = self.height(z) as i64;
        let mut delta: i64;
        if t == 0 {
            // §921 Adjust shift_up and shift_down for the case of no fraction line
            let drt = self.default_rule_thickness() as i64;
            let clr = if self.cur_style < TEXT_STYLE { 7 * drt } else { 3 * drt };
            delta = half64(clr - ((shift_up - dx) - (hz - shift_down)));
            if delta > 0 {
                shift_up += delta;
                shift_down += delta;
            }
        } else {
            // §922 Adjust shift_up and shift_down for the case of a fraction line
            let clr = if self.cur_style < TEXT_STYLE { 3 * t } else { t };
            delta = half64(t);
            let delta1 = clr - ((shift_up - dx) - (axis + delta));
            let delta2 = clr - ((axis - delta) - (hz - shift_down));
            if delta1 > 0 {
                shift_up += delta1;
            }
            if delta2 > 0 {
                shift_down += delta2;
            }
        }
        // §923 Construct a vlist box for the fraction, according to shift_up and shift_down
        let v = self.new_null_box();
        self.set_ty(v, VLIST_NODE);
        self.set_height(v, sat32(shift_up + self.height(x) as i64));
        self.set_depth(v, sat32(self.depth(z) as i64 + shift_down));
        self.set_width(v, self.width(x)); // this also equals width(z)
        let mut p;
        if t == 0 {
            p = self.new_kern(sat32((shift_up - dx) - (hz - shift_down)));
            self.set_link(p, z);
        } else {
            let y = self.fraction_rule(self.thickness(q));
            p = self.new_kern(sat32((axis - delta) - (hz - shift_down)));
            self.set_link(y, p);
            self.set_link(p, z);
            p = self.new_kern(sat32((shift_up - dx) - (axis + delta)));
            self.set_link(p, y);
        }
        self.set_link(x, p);
        self.set_list_ptr(v, x);
        // §924 Put the fraction into a box with its delimiters, and make new_hlist(q) point to it
        let d = if self.cur_style < TEXT_STYLE { self.delim1(self.cur_size) } else { self.delim2(self.cur_size) };
        x = self.var_delimiter(self.left_delimiter(q), self.cur_size, d);
        self.set_link(x, v);
        z = self.var_delimiter(self.right_delimiter(q), self.cur_size, d);
        self.set_link(v, z);
        let h = self.hpack_natural(x);
        self.set_new_hlist(q, h);
    }

    /// §925
    pub fn make_op(&mut self, q: i32) -> i32 {
        let delta;
        if self.subtype(q) == NORMAL && self.cur_style < TEXT_STYLE {
            self.set_subtype(q, LIMITS);
        }
        if self.math_type(self.nucleus(q)) == MATH_CHAR {
            self.fetch(self.nucleus(q));
            if self.cur_style < TEXT_STYLE && self.char_tag(self.cur_i) == LIST_TAG {
                // make it larger
                let c = self.rem_byte(self.cur_i);
                let i = self.orig_char_info(self.cur_f, c);
                if self.char_exists(i) {
                    self.cur_c = c;
                    self.cur_i = i;
                    self.set_character(self.nucleus(q), c);
                }
            }
            delta = self.char_italic(self.cur_f, self.cur_i);
            let x = self.clean_box(self.nucleus(q), self.cur_style);
            if self.math_type(self.subscr(q)) != EMPTY && self.subtype(q) != LIMITS {
                self.set_width(x, sub_sat(self.width(x), delta)); // remove italic correction
            }
            // center vertically
            let h = half64(self.height(x) as i64 - self.depth(x) as i64);
            self.set_shift_amount(x, sat32(h - self.axis_height(self.cur_size) as i64));
            self.set_math_type(self.nucleus(q), SUB_BOX);
            self.set_info(self.nucleus(q), x);
        } else {
            delta = 0;
        }
        if self.subtype(q) == LIMITS {
            // §926 Construct a box with limits above and below it, skewed by delta
            let mut x = self.clean_box(self.supscr(q), self.sup_style(self.cur_style));
            let mut y = self.clean_box(self.nucleus(q), self.cur_style);
            let mut z = self.clean_box(self.subscr(q), self.sub_style(self.cur_style));
            let v = self.new_null_box();
            self.set_ty(v, VLIST_NODE);
            self.set_width(v, self.width(y));
            if self.width(x) > self.width(v) {
                self.set_width(v, self.width(x));
            }
            if self.width(z) > self.width(v) {
                self.set_width(v, self.width(z));
            }
            x = self.rebox(x, self.width(v));
            y = self.rebox(y, self.width(v));
            z = self.rebox(z, self.width(v));
            let h = self.half(delta);
            self.set_shift_amount(x, h);
            self.set_shift_amount(z, -self.shift_amount(x));
            self.set_height(v, self.height(y));
            self.set_depth(v, self.depth(y));
            // §927 Attach the limits to y and adjust height(v), depth(v) to account for their presence
            let spacing5 = self.big_op_spacing5();
            if self.math_type(self.supscr(q)) == EMPTY {
                self.free_node(x, BOX_NODE_SIZE);
                self.set_list_ptr(v, y);
            } else {
                let mut shift_up = self.big_op_spacing3() as i64 - self.depth(x) as i64;
                if shift_up < self.big_op_spacing1() as i64 {
                    shift_up = self.big_op_spacing1() as i64;
                }
                let mut p = self.new_kern(sat32(shift_up));
                self.set_link(p, y);
                self.set_link(x, p);
                p = self.new_kern(spacing5);
                self.set_link(p, x);
                self.set_list_ptr(v, p);
                let h = self.height(v) as i64 + spacing5 as i64 + self.height(x) as i64 + self.depth(x) as i64 + shift_up;
                self.set_height(v, sat32(h));
            }
            if self.math_type(self.subscr(q)) == EMPTY {
                self.free_node(z, BOX_NODE_SIZE);
            } else {
                let mut shift_down = self.big_op_spacing4() as i64 - self.height(z) as i64;
                if shift_down < self.big_op_spacing2() as i64 {
                    shift_down = self.big_op_spacing2() as i64;
                }
                let mut p = self.new_kern(sat32(shift_down));
                self.set_link(y, p);
                self.set_link(p, z);
                p = self.new_kern(spacing5);
                self.set_link(z, p);
                let d = self.depth(v) as i64 + spacing5 as i64 + self.height(z) as i64 + self.depth(z) as i64 + shift_down;
                self.set_depth(v, sat32(d));
            }
            self.set_new_hlist(q, v);
        }
        delta
    }

    /// §928
    pub fn make_ord(&mut self, q: i32) {
        // restart:
        loop {
            if self.math_type(self.subscr(q)) != EMPTY
                || self.math_type(self.supscr(q)) != EMPTY
                || self.math_type(self.nucleus(q)) != MATH_CHAR
            {
                return;
            }
            let mut p = self.link(q);
            if p == NULL
                || self.ty(p) < ORD_NOAD
                || self.ty(p) > PUNCT_NOAD
                || self.math_type(self.nucleus(p)) != MATH_CHAR
                || self.fam(self.nucleus(p)) != self.fam(self.nucleus(q))
            {
                return;
            }
            self.set_math_type(self.nucleus(q), MATH_TEXT_CHAR);
            self.fetch(self.nucleus(q));
            if self.char_tag(self.cur_i) != LIG_TAG {
                return;
            }
            let mut a = self.lig_kern_start(self.cur_f, self.cur_i);
            self.cur_c = self.character(self.nucleus(p));
            self.cur_i = self.font_info[a].qqqq();
            if self.skip_byte(self.cur_i) > STOP_FLAG {
                a = self.lig_kern_restart(self.cur_f, self.cur_i);
                self.cur_i = self.font_info[a].qqqq();
            }
            loop {
                // §929 If instruction cur_i is a kern with cur_c, attach the kern after q; or if it is a
                // ligature with cur_c, combine noads q and p appropriately; then return if the cursor has
                // moved past a noad, or goto restart
                if self.next_char(self.cur_i) == self.cur_c && self.skip_byte(self.cur_i) <= STOP_FLAG {
                    if self.op_byte(self.cur_i) >= KERN_FLAG {
                        p = self.new_kern(self.char_kern(self.cur_f, self.cur_i));
                        self.set_link(p, self.link(q));
                        self.set_link(q, p);
                        return;
                    } else {
                        self.check_interrupt(); // allow a way out of infinite ligature loop
                        match self.op_byte(self.cur_i) {
                            1 | 5 => self.set_character(self.nucleus(q), self.rem_byte(self.cur_i)), // =:|, =:|>
                            2 | 6 => self.set_character(self.nucleus(p), self.rem_byte(self.cur_i)), // |=:, |=:>
                            3 | 7 | 11 => {
                                // |=:|, |=:|>, |=:|>>
                                let r = self.new_noad();
                                self.set_character(self.nucleus(r), self.rem_byte(self.cur_i));
                                self.set_fam(self.nucleus(r), self.fam(self.nucleus(q)));
                                self.set_link(q, r);
                                self.set_link(r, p);
                                if self.op_byte(self.cur_i) < 11 {
                                    self.set_math_type(self.nucleus(r), MATH_CHAR);
                                } else {
                                    self.set_math_type(self.nucleus(r), MATH_TEXT_CHAR); // prevent combination
                                }
                            }
                            _ => {
                                self.set_link(q, self.link(p));
                                self.set_character(self.nucleus(q), self.rem_byte(self.cur_i)); // =:
                                let (bq, bp) = (self.subscr(q), self.subscr(p));
                                self.mem[bq] = self.mem[bp];
                                let (sq, sp) = (self.supscr(q), self.supscr(p));
                                self.mem[sq] = self.mem[sp];
                                self.free_node(p, NOAD_SIZE);
                            }
                        }
                        if self.op_byte(self.cur_i) > 3 {
                            return;
                        }
                        self.set_math_type(self.nucleus(q), MATH_CHAR);
                        break; // goto restart
                    }
                }
                if self.skip_byte(self.cur_i) >= STOP_FLAG {
                    return;
                }
                a = a + self.skip_byte(self.cur_i) + 1;
                self.cur_i = self.font_info[a].qqqq();
            }
        }
    }

    /// §932
    pub fn make_scripts(&mut self, q: i32, delta: i32) {
        let mut p = self.new_hlist(q);
        let mut shift_up: i64;
        let mut shift_down: i64;
        if self.is_char_node(p) {
            shift_up = 0;
            shift_down = 0;
        } else {
            let z = self.hpack_natural(p);
            let t = if self.cur_style < SCRIPT_STYLE { SCRIPT_SIZE } else { SCRIPT_SCRIPT_SIZE };
            shift_up = self.height(z) as i64 - self.sup_drop(t) as i64;
            shift_down = self.depth(z) as i64 + self.sub_drop(t) as i64;
            self.free_node(z, BOX_NODE_SIZE);
        }
        let mut x;
        if self.math_type(self.supscr(q)) == EMPTY {
            // §933 Construct a subscript box x when there is no superscript
            x = self.clean_box(self.subscr(q), self.sub_style(self.cur_style));
            self.set_width(x, add_sat(self.width(x), self.script_space()));
            if shift_down < self.sub1(self.cur_size) as i64 {
                shift_down = self.sub1(self.cur_size) as i64;
            }
            let clr = self.height(x) as i64 - (self.math_x_height(self.cur_size) as i64 * 4).abs() / 5;
            if shift_down < clr {
                shift_down = clr;
            }
            self.set_shift_amount(x, sat32(shift_down));
        } else {
            // §934 Construct a superscript box x
            x = self.clean_box(self.supscr(q), self.sup_style(self.cur_style));
            self.set_width(x, add_sat(self.width(x), self.script_space()));
            let mut clr = if odd(self.cur_style) {
                self.sup3(self.cur_size)
            } else if self.cur_style < TEXT_STYLE {
                self.sup1(self.cur_size)
            } else {
                self.sup2(self.cur_size)
            } as i64;
            if shift_up < clr {
                shift_up = clr;
            }
            clr = self.depth(x) as i64 + abs(self.math_x_height(self.cur_size)) as i64 / 4;
            if shift_up < clr {
                shift_up = clr;
            }
            if self.math_type(self.subscr(q)) == EMPTY {
                self.set_shift_amount(x, sat32(-shift_up));
            } else {
                // §935 Construct a sub/superscript combination box x, with the superscript offset by delta
                let y = self.clean_box(self.subscr(q), self.sub_style(self.cur_style));
                self.set_width(y, add_sat(self.width(y), self.script_space()));
                if shift_down < self.sub2(self.cur_size) as i64 {
                    shift_down = self.sub2(self.cur_size) as i64;
                }
                let dx = self.depth(x) as i64;
                let hy = self.height(y) as i64;
                clr = 4 * self.default_rule_thickness() as i64 - ((shift_up - dx) - (hy - shift_down));
                if clr > 0 {
                    shift_down += clr;
                    clr = (self.math_x_height(self.cur_size) as i64 * 4).abs() / 5 - (shift_up - dx);
                    if clr > 0 {
                        shift_up += clr;
                        shift_down -= clr;
                    }
                }
                self.set_shift_amount(x, delta); // superscript is delta to the right of the subscript
                p = self.new_kern(sat32((shift_up - dx) - (hy - shift_down)));
                self.set_link(x, p);
                self.set_link(p, y);
                x = self.vpack_natural(x);
                self.set_shift_amount(x, sat32(shift_down));
            }
        }
        if self.new_hlist(q) == NULL {
            self.set_new_hlist(q, x);
        } else {
            p = self.new_hlist(q);
            while self.link(p) != NULL {
                p = self.link(p);
            }
            self.set_link(p, x);
        }
    }

    /// §938
    pub fn make_left_right(&mut self, q: i32, style: i32, max_d: i32, max_h: i32) -> i32 {
        self.cur_style = style;
        self.mlist_set_cur_size_and_mu(); // §879
        let mut delta2 = max_d as i64 + self.axis_height(self.cur_size) as i64;
        let mut delta1 = max_h as i64 + max_d as i64 - delta2;
        if delta2 > delta1 {
            delta1 = delta2; // delta1 is max distance from axis
        }
        let mut delta = (delta1 / 500) * self.delimiter_factor() as i64;
        delta2 = delta1 + delta1 - self.delimiter_shortfall() as i64;
        if delta < delta2 {
            delta = delta2;
        }
        let v = self.var_delimiter(self.delimiter(q), self.cur_size, sat32(delta));
        self.set_new_hlist(q, v);
        self.ty(q) - (LEFT_NOAD - OPEN_NOAD) // open_noad or close_noad
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mem::memkit::*;
    use crate::show::showkit::table;
    use crate::toks::testkit::*;

    const PT: i32 = 65536;

    /// A font by hand, as read_font_info leaves one, since tfm.rs is another module's.  `Chr` gives an index into each
    /// of the first four tables, as a TFM does; index 0 of `widths` means the character does not exist.
    #[derive(Clone, Copy, Default)]
    struct Chr {
        c: i32,
        w: i32,
        h: i32,
        d: i32,
        i: i32,
        tag: i32,
        rem: i32,
    }
    #[derive(Default)]
    struct Font<'a> {
        widths: &'a [i32],
        heights: &'a [i32],
        depths: &'a [i32],
        italics: &'a [i32],
        chars: &'a [Chr],
        lig: &'a [(i32, i32, i32, i32)],
        kerns: &'a [i32],
        exten: &'a [(i32, i32, i32, i32)],
        params: &'a [i32],
    }

    fn scaled(t: &mut Tex, v: &[i32]) -> i32 {
        let base = t.fmem_ptr;
        for &x in v {
            let k = t.fmem_ptr;
            t.font_info[k].set_sc(x);
            t.fmem_ptr += 1;
        }
        base
    }

    fn quarters(t: &mut Tex, v: &[(i32, i32, i32, i32)]) -> i32 {
        let base = t.fmem_ptr;
        for &(b0, b1, b2, b3) in v {
            let k = t.fmem_ptr;
            t.font_info[k].set_qqqq(FourQuarters { b0, b1, b2, b3 });
            t.fmem_ptr += 1;
        }
        base
    }

    fn add_font(t: &mut Tex, f: i32, s: &Font) {
        let bc = s.chars.iter().map(|x| x.c).min().unwrap_or(1);
        let ec = s.chars.iter().map(|x| x.c).max().unwrap_or(0);
        t.font_bc[f] = bc;
        t.font_ec[f] = ec;
        t.char_base[f] = t.fmem_ptr - bc;
        for c in bc..=ec {
            let mut q = (0, 0, 0, 0);
            if let Some(x) = s.chars.iter().find(|x| x.c == c) {
                q = (x.w, x.h * 16 + x.d, x.i * 4 + x.tag, x.rem);
            }
            quarters(t, &[q]);
        }
        t.width_base[f] = scaled(t, s.widths);
        t.height_base[f] = scaled(t, s.heights);
        t.depth_base[f] = scaled(t, s.depths);
        t.italic_base[f] = scaled(t, s.italics);
        t.lig_kern_base[f] = quarters(t, s.lig);
        t.kern_base[f] = scaled(t, s.kerns) - KERN_BASE_OFFSET;
        t.exten_base[f] = quarters(t, s.exten);
        t.param_base[f] = scaled(t, s.params) - 1;
        t.skew_char[f] = -1;
    }

    /// `mem` set up, printing, an empty font 0, a symbol font in family 2 and an extension font in family 3, with the
    /// parameters math reads.  Fonts from 3 on are left to the test.  Axis 3pt, quad 18pt, x-height 4pt; rule thickness 0.5pt.
    fn math_machine() -> &'static mut Tex {
        let t = mem_machine();
        print_setup(t);
        t.fmem_ptr = 100; // what the null font's parameters read stays zero
        t.font_max = 10;
        let mut sy = [0; 22];
        sy[X_HEIGHT_CODE as usize - 1] = 4 * PT;
        sy[QUAD_CODE as usize - 1] = 18 * PT;
        sy[21] = 3 * PT;
        add_font(t, 1, &Font { params: &sy, ..Default::default() });
        let mut ex = [0; 13];
        ex[7] = PT / 2;
        add_font(t, 2, &Font { params: &ex, ..Default::default() });
        for s in [TEXT_SIZE, SCRIPT_SIZE, SCRIPT_SCRIPT_SIZE] {
            t.set_fam_fnt(2 + s, 1);
            t.set_fam_fnt(3 + s, 2);
        }
        t.cur_style = TEXT_STYLE;
        t.cur_size = TEXT_SIZE;
        t.set_null_delimiter_space(PT + PT / 5);
        t
    }

    /// Characters along a list: 10 lists 11, which lists 12, of total height 10pt, 20pt, 30pt, width 5pt, and 10 has an
    /// italic correction of 1pt.  16 is 10pt too and has no list.  13 is extensible by recipe 0, with top 20, mid 21, bot
    /// 22, rep 23, 14 by recipe 1, with only a rep, 30 by recipe 2, whose rep has no height.  15 lists 13.
    fn delimiter_font(t: &mut Tex, f: i32) {
        let chr = |c, w, h, d, tag, rem| Chr { c, w, h, d, i: 0, tag, rem };
        add_font(
            t,
            f,
            &Font {
                widths: &[0, 5 * PT, 7 * PT],
                heights: &[0, 8 * PT, 15 * PT, 25 * PT, 4 * PT, 5 * PT],
                depths: &[0, 2 * PT, 5 * PT],
                italics: &[0, PT],
                chars: &[
                    Chr { c: 10, w: 1, h: 1, d: 1, i: 1, tag: LIST_TAG, rem: 11 },
                    chr(11, 1, 2, 2, LIST_TAG, 12),
                    chr(12, 1, 3, 2, 0, 0),
                    chr(13, 2, 1, 1, EXT_TAG, 0),
                    chr(14, 2, 1, 1, EXT_TAG, 1),
                    chr(15, 1, 1, 1, LIST_TAG, 13),
                    chr(16, 1, 1, 1, 0, 0),
                    chr(20, 2, 4, 1, 0, 0),
                    chr(21, 2, 5, 1, 0, 0),
                    chr(22, 2, 4, 2, 0, 0),
                    Chr { c: 23, w: 2, h: 1, d: 1, i: 1, tag: 0, rem: 0 },
                    chr(30, 1, 0, 0, EXT_TAG, 2),
                    chr(31, 1, 0, 0, 0, 0),
                ],
                exten: &[(20, 21, 22, 23), (0, 0, 0, 23), (0, 0, 0, 31)],
                ..Default::default()
            },
        );
    }

    fn delim(t: &mut Tex, a: i32, b: i32, c: i32, e: i32) -> i32 {
        let d = t.get_node(2);
        t.set_small_fam(d, a);
        t.set_small_char(d, b);
        t.set_large_fam(d, c);
        t.set_large_char(d, e);
        d
    }

    /// The characters in a vbox from the top, or the one in an hbox.
    fn chars_of(t: &Tex, b: i32) -> Vec<i32> {
        let mut v = Vec::new();
        let mut p = t.list_ptr(b);
        while p != NULL {
            if t.is_char_node(p) {
                v.push(t.character(p));
            } else {
                assert_eq!(t.ty(p), HLIST_NODE);
                let q = t.list_ptr(p);
                assert!(t.is_char_node(q) && t.link(q) == NULL);
                v.push(t.character(q));
            }
            p = t.link(p);
        }
        v
    }

    #[test]
    fn no_delimiter_gives_an_empty_box_on_the_axis() {
        let t = math_machine();
        let d = delim(t, 0, 0, 0, 0);
        let b = t.var_delimiter(d, TEXT_SIZE, 100 * PT);
        assert_eq!((t.ty(b), t.list_ptr(b)), (HLIST_NODE, NULL));
        assert_eq!((t.width(b), t.height(b), t.depth(b)), (PT + PT / 5, 0, 0));
        assert_eq!(t.shift_amount(b), -3 * PT);
        // A family with no font, and a character beyond the font, are the same.
        delimiter_font(t, 3);
        t.set_fam_fnt(4, 3);
        for (fam, c) in [(5, 10), (4, 9), (4, 200)] {
            let d = delim(t, fam, c, 0, 0);
            let b = t.var_delimiter(d, TEXT_SIZE, 0);
            assert_eq!(t.list_ptr(b), NULL, "fam {} char {}", fam, c);
        }
        Tex::dealloc(t);
    }

    #[test]
    fn the_list_is_walked_until_a_character_is_tall_enough() {
        let t = math_machine();
        delimiter_font(t, 3);
        t.set_fam_fnt(4, 3);
        let d = delim(t, 4, 10, 0, 0);
        for (v, want) in [
            (0, 10),
            (10 * PT, 10),
            (10 * PT + 1, 11),
            (20 * PT, 11),
            (20 * PT + 1, 12),
            (30 * PT, 12),
            (30 * PT + 1, 12), // nothing is enough, so the largest
            (i32::MAX, 12),
        ] {
            let b = t.var_delimiter(d, TEXT_SIZE, v);
            assert_eq!(chars_of(t, b), [want], "v {}", v);
        }
        let b = t.var_delimiter(d, TEXT_SIZE, 0);
        assert_eq!((t.width(b), t.height(b), t.depth(b)), (6 * PT, 8 * PT, 2 * PT), "the width has the italic correction");
        assert_eq!(t.shift_amount(b), 0, "half(height-depth) is the axis");
        let b = t.var_delimiter(d, SCRIPT_SIZE, 0);
        assert_eq!(chars_of(t, b), [10], "the sizes go on down to the text font");
        Tex::dealloc(t);
    }

    #[test]
    fn sizes_are_tried_from_the_given_one_down() {
        let t = math_machine();
        delimiter_font(t, 3);
        delimiter_font(t, 4);
        t.set_fam_fnt(4 + SCRIPT_SCRIPT_SIZE, NULL_FONT);
        t.set_fam_fnt(4 + SCRIPT_SIZE, 3);
        t.set_fam_fnt(4, 4);
        let d = delim(t, 4, 10, 0, 0);
        let b = t.var_delimiter(d, SCRIPT_SCRIPT_SIZE, 0);
        assert_eq!(t.font(t.list_ptr(b)), 3);
        let b = t.var_delimiter(d, TEXT_SIZE, 0);
        assert_eq!(t.font(t.list_ptr(b)), 4);
        // Nothing is enough in either: only u>w replaces, so the first largest stays.
        let b = t.var_delimiter(d, SCRIPT_SIZE, 30 * PT + 1);
        assert_eq!((t.font(t.list_ptr(b)), chars_of(t, b)), (3, vec![12]));
        Tex::dealloc(t);
    }

    #[test]
    fn the_large_variant_comes_after_the_small() {
        let t = math_machine();
        delimiter_font(t, 3);
        t.set_fam_fnt(4, 3);
        let d = delim(t, 4, 16, 4, 11);
        let b = t.var_delimiter(d, TEXT_SIZE, 5 * PT);
        assert_eq!(chars_of(t, b), [16]);
        let b = t.var_delimiter(d, TEXT_SIZE, 15 * PT);
        assert_eq!(chars_of(t, b), [11]);
        let b = t.var_delimiter(d, TEXT_SIZE, 25 * PT);
        assert_eq!(chars_of(t, b), [12]);
        // Family 0 with character 0 is no variant; another family with character 0 is one, here out of range.
        let d = delim(t, 0, 0, 4, 10);
        let b = t.var_delimiter(d, TEXT_SIZE, 15 * PT);
        assert_eq!(chars_of(t, b), [11]);
        let d = delim(t, 4, 0, 4, 12);
        let b = t.var_delimiter(d, TEXT_SIZE, 0);
        assert_eq!(chars_of(t, b), [12]);
        Tex::dealloc(t);
    }

    #[test]
    fn an_extensible_is_stacked_from_the_recipe() {
        let t = math_machine();
        delimiter_font(t, 3);
        t.set_fam_fnt(4, 3);
        // Top 20 is 4+2=6pt, mid 21 is 5+2=7pt, bot 22 is 4+5=9pt, rep 23 is 8+2=10pt.  The pieces alone give 22pt.
        for (v, reps) in [(0, 0), (22 * PT, 0), (22 * PT + 1, 1), (42 * PT, 1), (42 * PT + 1, 2), (62 * PT, 2)] {
            let d = delim(t, 4, 13, 0, 0);
            let b = t.var_delimiter(d, TEXT_SIZE, v);
            assert_eq!(t.ty(b), VLIST_NODE);
            let mut want = vec![20];
            want.extend(std::iter::repeat(23).take(reps));
            want.push(21);
            want.extend(std::iter::repeat(23).take(reps));
            want.push(22);
            assert_eq!(chars_of(t, b), want, "v {}", v);
            let w = 22 * PT + 2 * reps as i32 * 10 * PT;
            assert_eq!(t.height(b), 4 * PT, "the height of the top piece");
            assert_eq!(t.depth(b), w - 4 * PT);
            assert_eq!(t.width(b), 8 * PT, "the width and italic correction of the rep, 7pt and 1pt");
            assert_eq!(t.shift_amount(b), (4 * PT - (w - 4 * PT)) / 2 - 3 * PT);
        }
        // Only a rep, which is all the height.
        for (v, reps) in [(0, 0), (1, 1), (10 * PT, 1), (10 * PT + 1, 2)] {
            let d = delim(t, 4, 14, 0, 0);
            let b = t.var_delimiter(d, TEXT_SIZE, v);
            assert_eq!(chars_of(t, b), vec![23; reps], "v {}", v);
        }
        // A list that ends in an extensible.
        let d = delim(t, 4, 15, 0, 0);
        let b = t.var_delimiter(d, TEXT_SIZE, 11 * PT);
        assert_eq!(t.ty(b), VLIST_NODE);
        // A rep with no height cannot reach v: no steps, and no endless loop.
        let d = delim(t, 4, 30, 0, 0);
        let b = t.var_delimiter(d, TEXT_SIZE, 100 * PT);
        assert_eq!(chars_of(t, b), Vec::<i32>::new());
        assert_eq!((t.height(b), t.depth(b)), (0, 0));
        Tex::dealloc(t);
    }

    #[test]
    fn make_left_right_takes_the_size_of_the_tallest_side() {
        let t = math_machine();
        delimiter_font(t, 3);
        t.set_fam_fnt(4, 3);
        let q = t.get_node(NOAD_SIZE);
        t.set_ty(q, LEFT_NOAD);
        let set = |t: &mut Tex, c: i32| {
            let d = t.delimiter(q);
            t.set_small_fam(d, 4);
            t.set_small_char(d, c);
            t.set_large_fam(d, 0);
            t.set_large_char(d, 0);
        };
        t.set_delimiter_factor(1000);
        t.set_delimiter_shortfall(0);
        // delta1 is 8pt-3pt above the axis: v is exactly 10pt, and a sp more goes on to the next character.
        set(t, 10);
        assert_eq!(t.make_left_right(q, TEXT_STYLE, 0, 8 * PT), OPEN_NOAD);
        assert_eq!(chars_of(t, t.new_hlist(q)), [10]);
        set(t, 10);
        t.make_left_right(q, TEXT_STYLE, 0, 8 * PT + 1);
        assert_eq!(chars_of(t, t.new_hlist(q)), [11]);
        // Below the axis, delta1 is depth+axis.
        set(t, 10);
        t.make_left_right(q, TEXT_STYLE, 2 * PT, 0);
        assert_eq!(chars_of(t, t.new_hlist(q)), [10]);
        set(t, 10);
        t.make_left_right(q, TEXT_STYLE, 2 * PT + 1, 0);
        assert_eq!(chars_of(t, t.new_hlist(q)), [11]);
        t.set_ty(q, RIGHT_NOAD);
        set(t, 10);
        assert_eq!(t.make_left_right(q, TEXT_STYLE, 0, 0), CLOSE_NOAD);
        Tex::dealloc(t);
    }

    #[test]
    fn make_left_right_does_not_wrap() {
        let t = math_machine();
        delimiter_font(t, 3);
        t.set_fam_fnt(4, 3);
        let q = t.get_node(NOAD_SIZE);
        t.set_ty(q, LEFT_NOAD);
        let set = |t: &mut Tex, c: i32| {
            let d = t.delimiter(q);
            t.set_small_fam(d, 4);
            t.set_small_char(d, c);
            t.set_large_fam(d, 0);
            t.set_large_char(d, 0);
        };
        // delta1 is 103pt, and (delta1 div 500)*factor is about -2^35, whose low 32 bits read as about 9300pt.  It must
        // lose to delta2, which is 206pt: 10 steps of 20pt and the 22pt of the pieces.
        t.set_delimiter_factor(-2500000);
        t.set_delimiter_shortfall(0);
        set(t, 13);
        t.make_left_right(q, TEXT_STYLE, 100 * PT, 100 * PT);
        assert_eq!(chars_of(t, t.new_hlist(q)).len(), 3 + 2 * 10);
        // delta1+delta1 past 2^31, and \delimitershortfall bringing it back.
        t.set_delimiter_factor(0);
        t.set_delimiter_shortfall(MAX_DIMEN);
        set(t, 10);
        t.make_left_right(q, TEXT_STYLE, MAX_DIMEN, MAX_DIMEN);
        assert_eq!(chars_of(t, t.new_hlist(q)), [12]);
        // A positive product past 2^31 clamps, and stays what is largest: the largest character.
        t.set_delimiter_factor(i32::MAX);
        t.set_delimiter_shortfall(0);
        set(t, 10);
        t.make_left_right(q, TEXT_STYLE, 100 * PT, 100 * PT);
        assert_eq!(chars_of(t, t.new_hlist(q)), [12]);
        Tex::dealloc(t);
    }

    #[test]
    fn math_glue_and_math_kern_match_the_oracle() {
        let rows = table(include_bytes!("../testdata/mlist_mu_oracle.tsv"));
        assert!(rows.len() >= 200);
        let t = math_machine();
        let mut bad = Vec::new();
        for row in &rows {
            let n: Vec<i32> = row[..7].iter().map(|x| x.parse().unwrap()).collect();
            t.set_param(QUAD_CODE, 1, n[0]);
            t.cur_style = TEXT_STYLE;
            t.mlist_set_cur_size_and_mu();
            let g = t.get_node(GLUE_SPEC_SIZE);
            t.set_width(g, n[1]);
            t.set_stretch(g, n[2]);
            t.set_stretch_order(g, n[3]);
            t.set_shrink(g, n[4]);
            t.set_shrink_order(g, n[5]);
            let p = t.math_glue(g, t.cur_mu);
            t.print_spec(p, b"");
            let glue = printed(t);
            let k = t.new_kern(n[6]);
            t.set_subtype(k, MU_GLUE);
            t.math_kern(k, t.cur_mu);
            assert_eq!(t.subtype(k), EXPLICIT);
            t.print_scaled(t.width(k));
            let kern = printed(t);
            if glue != row[7] || kern != row[8] {
                bad.push(format!("{:?}: got {:?} {:?}", row, glue, kern));
            }
            t.free_node(g, GLUE_SPEC_SIZE);
            t.free_node(p, GLUE_SPEC_SIZE);
            t.free_node(k, MEDIUM_NODE_SIZE);
        }
        assert!(bad.is_empty(), "{} of {} differ:\n{}", bad.len(), rows.len(), bad.join("\n"));
        Tex::dealloc(t);
    }

    #[test]
    fn math_kern_leaves_other_kerns_alone() {
        let t = math_machine();
        for s in [NORMAL, EXPLICIT, ACC_KERN] {
            let k = t.new_kern(3 * PT);
            t.set_subtype(k, s);
            t.math_kern(k, PT);
            assert_eq!((t.width(k), t.subtype(k)), (3 * PT, s));
        }
        Tex::dealloc(t);
    }

    #[test]
    fn cur_size_and_cur_mu_follow_the_style() {
        let t = math_machine();
        for (style, size) in [
            (DISPLAY_STYLE, TEXT_SIZE),
            (DISPLAY_STYLE + CRAMPED, TEXT_SIZE),
            (TEXT_STYLE, TEXT_SIZE),
            (TEXT_STYLE + CRAMPED, TEXT_SIZE),
            (SCRIPT_STYLE, SCRIPT_SIZE),
            (SCRIPT_STYLE + CRAMPED, SCRIPT_SIZE),
            (SCRIPT_SCRIPT_STYLE, SCRIPT_SCRIPT_SIZE),
            (SCRIPT_SCRIPT_STYLE + CRAMPED, SCRIPT_SCRIPT_SIZE),
        ] {
            t.cur_style = style;
            t.mlist_set_cur_size_and_mu();
            assert_eq!((t.cur_size, t.cur_mu), (size, PT), "style {}", style);
        }
        Tex::dealloc(t);
    }

    /// Characters for make_ord: A and R have a program, the others none.
    fn ligature_font(t: &mut Tex, f: i32) {
        let p = |c: u8, tag: i32, rem: i32| Chr { c: c as i32, w: 1, h: 1, d: 1, i: 0, tag, rem };
        add_font(
            t,
            f,
            &Font {
                widths: &[0, 5 * PT],
                heights: &[0, 7 * PT],
                depths: &[0, PT],
                italics: &[0],
                chars: &[
                    p(b'A', LIG_TAG, 0),
                    p(b'B', 0, 0),
                    p(b'C', 0, 0),
                    p(b'D', 0, 0),
                    p(b'E', 0, 0),
                    p(b'F', 0, 0),
                    p(b'G', 0, 0),
                    p(b'H', 0, 0),
                    p(b'I', 0, 0),
                    p(b'J', 0, 0),
                    p(b'K', 0, 0),
                    p(b'L', 0, 0),
                    p(b'R', LIG_TAG, 12),
                    p(b'X', 0, 0),
                    p(b'Y', 0, 0),
                ],
                lig: &[
                    (0, b'B' as i32, KERN_FLAG, 0),
                    (0, b'C' as i32, 0, b'X' as i32),  // =:
                    (0, b'D' as i32, 1, b'X' as i32),  // =:|
                    (0, b'E' as i32, 2, b'X' as i32),  // |=:
                    (0, b'F' as i32, 3, b'X' as i32),  // |=:|
                    (0, b'G' as i32, 5, b'X' as i32),  // =:|>
                    (0, b'H' as i32, 6, b'X' as i32),  // |=:>
                    (0, b'I' as i32, 7, b'X' as i32),  // |=:|>
                    (0, b'J' as i32, 11, b'X' as i32), // |=:|>>
                    (1, b'K' as i32, KERN_FLAG, 1),    // skips the next
                    (0, b'L' as i32, KERN_FLAG, 0),
                    (STOP_FLAG, b'Y' as i32, KERN_FLAG, 0),
                    (STOP_FLAG + 1, 0, 0, 13), // restart at 13
                    (STOP_FLAG, b'B' as i32, KERN_FLAG, 1),
                ],
                kerns: &[PT, 2 * PT],
                ..Default::default()
            },
        );
    }

    fn char_noad(t: &mut Tex, ty: i32, fam: i32, c: u8) -> i32 {
        let p = t.new_noad();
        t.set_ty(p, ty);
        let n = t.nucleus(p);
        t.set_math_type(n, MATH_CHAR);
        t.set_fam(n, fam);
        t.set_character(n, c as i32);
        p
    }

    /// `A` and then `c`, as noads, in family 4.  Returns q, which is A.
    fn pair(t: &mut Tex, c: u8) -> i32 {
        let q = char_noad(t, ORD_NOAD, 4, b'A');
        let p = char_noad(t, ORD_NOAD, 4, c);
        t.set_link(q, p);
        q
    }

    fn lig_machine() -> &'static mut Tex {
        let t = math_machine();
        ligature_font(t, 3);
        t.set_fam_fnt(4, 3);
        t
    }

    fn noads(t: &Tex, q: i32) -> Vec<(i32, i32, i32)> {
        let mut v = Vec::new();
        let mut q = q;
        while q != NULL {
            if t.ty(q) == KERN_NODE {
                v.push((KERN_NODE, t.width(q), 0));
            } else {
                v.push((t.ty(q), t.character(t.nucleus(q)), t.math_type(t.nucleus(q))));
            }
            q = t.link(q);
        }
        v
    }

    const X: i32 = b'X' as i32;
    const O: i32 = ORD_NOAD;

    #[test]
    fn make_ord_kerns_and_ligatures() {
        let t = lig_machine();
        let a = b'A' as i32;
        for (c, want) in [
            (b'B', vec![(O, a, MATH_TEXT_CHAR), (KERN_NODE, PT, 0), (O, b'B' as i32, MATH_CHAR)]),
            // =: leaves one noad and restarts, and with no next noad stays a math_char
            (b'C', vec![(O, X, MATH_CHAR)]),
            // =:| restarts on the new left character, which has no program
            (b'D', vec![(O, X, MATH_TEXT_CHAR), (O, b'D' as i32, MATH_CHAR)]),
            // |=: restarts, and the new right character has no entry
            (b'E', vec![(O, a, MATH_TEXT_CHAR), (O, X, MATH_CHAR)]),
            // |=:| restarts with the new noad next, which has no entry
            (b'F', vec![(O, a, MATH_TEXT_CHAR), (O, X, MATH_CHAR), (O, b'F' as i32, MATH_CHAR)]),
            // the forms with > do not restart
            (b'G', vec![(O, X, MATH_TEXT_CHAR), (O, b'G' as i32, MATH_CHAR)]),
            (b'H', vec![(O, a, MATH_TEXT_CHAR), (O, X, MATH_CHAR)]),
            (b'I', vec![(O, a, MATH_TEXT_CHAR), (O, X, MATH_CHAR), (O, b'I' as i32, MATH_CHAR)]),
            // >> makes the middle one a math_text_char
            (b'J', vec![(O, a, MATH_TEXT_CHAR), (O, X, MATH_TEXT_CHAR), (O, b'J' as i32, MATH_CHAR)]),
            // the entry after a skip of 1 is passed over, and a later one is seen
            (b'K', vec![(O, a, MATH_TEXT_CHAR), (KERN_NODE, 2 * PT, 0), (O, b'K' as i32, MATH_CHAR)]),
            (b'L', vec![(O, a, MATH_TEXT_CHAR), (O, b'L' as i32, MATH_CHAR)]),
            // the last entry of the program, and past it
            (b'Y', vec![(O, a, MATH_TEXT_CHAR), (KERN_NODE, PT, 0), (O, b'Y' as i32, MATH_CHAR)]),
            (b'X', vec![(O, a, MATH_TEXT_CHAR), (O, X, MATH_CHAR)]),
        ] {
            let q = pair(t, c);
            t.make_ord(q);
            assert_eq!(noads(t, q), want, "next character {}", c as char);
        }
        Tex::dealloc(t);
    }

    #[test]
    fn make_ord_restart_program() {
        let t = lig_machine();
        let q = char_noad(t, ORD_NOAD, 4, b'R');
        let p = char_noad(t, ORD_NOAD, 4, b'B');
        t.set_link(q, p);
        t.make_ord(q);
        assert_eq!(
            noads(t, q),
            [(O, b'R' as i32, MATH_TEXT_CHAR), (KERN_NODE, 2 * PT, 0), (O, b'B' as i32, MATH_CHAR)]
        );
        Tex::dealloc(t);
    }

    #[test]
    fn make_ord_when_the_tests_fail() {
        let t = lig_machine();
        let a = b'A' as i32;
        let b = b'B' as i32;
        // q has a script, or a nucleus other than a char
        for x in [0, 1] {
            let q = pair(t, b'B');
            let f = if x == 0 { t.subscr(q) } else { t.supscr(q) };
            t.set_math_type(f, MATH_CHAR);
            t.make_ord(q);
            assert_eq!(noads(t, q), [(O, a, MATH_CHAR), (O, b, MATH_CHAR)]);
        }
        let q = pair(t, b'B');
        t.set_math_type(t.nucleus(q), SUB_BOX);
        t.make_ord(q);
        assert_eq!(t.link(t.link(q)), NULL);
        // no next, no noad next, a type out of range, no char, another family
        let q = char_noad(t, ORD_NOAD, 4, b'A');
        t.make_ord(q);
        assert_eq!(noads(t, q), [(O, a, MATH_CHAR)]);
        let k = t.new_kern(PT);
        t.set_link(q, k);
        t.make_ord(q);
        assert_eq!(t.math_type(t.nucleus(q)), MATH_CHAR);
        for ty in [INNER_NOAD, RADICAL_NOAD] {
            let q = pair(t, b'B');
            t.set_ty(t.link(q), ty);
            t.make_ord(q);
            assert_eq!(t.math_type(t.nucleus(q)), MATH_CHAR, "type {}", ty);
        }
        for ty in [OP_NOAD, BIN_NOAD, REL_NOAD, OPEN_NOAD, CLOSE_NOAD, PUNCT_NOAD] {
            let q = pair(t, b'B');
            t.set_ty(t.link(q), ty);
            t.make_ord(q);
            assert_eq!(noads(t, q)[1], (KERN_NODE, PT, 0), "type {}", ty);
        }
        let q = pair(t, b'B');
        let n = t.nucleus(t.link(q));
        t.set_math_type(n, SUB_MLIST);
        t.make_ord(q);
        assert_eq!(t.math_type(t.nucleus(q)), MATH_CHAR);
        let q = pair(t, b'B');
        let n = t.nucleus(t.link(q));
        t.set_fam(n, 5);
        t.make_ord(q);
        assert_eq!(t.math_type(t.nucleus(q)), MATH_CHAR);
        // q's character has no program: it stays a math_text_char, with no kern
        let q = char_noad(t, ORD_NOAD, 4, b'B');
        let p = char_noad(t, ORD_NOAD, 4, b'B');
        t.set_link(q, p);
        t.make_ord(q);
        assert_eq!(noads(t, q), [(O, b, MATH_TEXT_CHAR), (O, b, MATH_CHAR)]);
        Tex::dealloc(t);
    }

    #[test]
    fn a_ligature_takes_the_scripts_of_the_second_noad() {
        let t = lig_machine();
        let q = pair(t, b'C');
        let p = t.link(q);
        let x = t.supscr(p);
        t.set_math_type(x, MATH_CHAR);
        t.set_character(x, b'S' as i32);
        let x = t.subscr(p);
        t.set_math_type(x, MATH_CHAR);
        t.set_character(x, b'T' as i32);
        let r = char_noad(t, ORD_NOAD, 4, b'B');
        t.set_link(p, r);
        t.make_ord(q);
        assert_eq!(t.link(q), r);
        assert_eq!(t.character(t.nucleus(q)), X);
        assert_eq!((t.math_type(t.supscr(q)), t.character(t.supscr(q))), (MATH_CHAR, b'S' as i32));
        assert_eq!((t.math_type(t.subscr(q)), t.character(t.subscr(q))), (MATH_CHAR, b'T' as i32));
        Tex::dealloc(t);
    }

    #[test]
    fn fetch_complains_about_an_undefined_family_at_each_size() {
        let t = lig_machine();
        let x = t.get_node(1);
        for (size, want) in [
            (TEXT_SIZE, "! \\textfont 5 is undefined (character a)."),
            (SCRIPT_SIZE, "! \\scriptfont 5 is undefined (character a)."),
            (SCRIPT_SCRIPT_SIZE, "! \\scriptscriptfont 5 is undefined (character a)."),
        ] {
            t.cur_size = size;
            t.set_math_type(x, MATH_CHAR);
            t.set_fam(x, 5);
            t.set_character(x, b'a' as i32);
            t.fetch(x);
            let got = printed(t);
            assert!(got.contains(want), "got {:?}", got);
            assert_eq!(t.math_type(x), EMPTY);
            assert!(!t.char_exists(t.cur_i));
            assert_eq!(t.cur_f, NULL_FONT);
        }
        Tex::dealloc(t);
    }

    #[test]
    fn fetch_finds_a_character() {
        let t = lig_machine();
        let x = t.get_node(1);
        t.cur_size = TEXT_SIZE;
        t.set_math_type(x, MATH_CHAR);
        t.set_fam(x, 4);
        t.set_character(x, b'A' as i32);
        t.fetch(x);
        assert_eq!((t.cur_c, t.cur_f, t.math_type(x)), (b'A' as i32, 3, MATH_CHAR));
        assert!(t.char_exists(t.cur_i));
        assert_eq!(t.char_tag(t.cur_i), LIG_TAG);
        Tex::dealloc(t);
    }

    #[test]
    fn char_box_and_stack_into_box() {
        let t = math_machine();
        delimiter_font(t, 3);
        let b = t.char_box(3, 10);
        assert_eq!((t.width(b), t.height(b), t.depth(b)), (6 * PT, 8 * PT, 2 * PT));
        assert_eq!((t.character(t.list_ptr(b)), t.font(t.list_ptr(b))), (10, 3));
        assert_eq!(t.height_plus_depth(3, 11), 20 * PT);
        let v = t.new_null_box();
        t.stack_into_box(v, 3, 10);
        t.stack_into_box(v, 3, 11);
        assert_eq!(t.height(v), 15 * PT);
        assert_eq!(chars_of(t, v), [11, 10]);
        Tex::dealloc(t);
    }

    #[test]
    fn fraction_rule_and_print_size() {
        let t = math_machine();
        let r = t.fraction_rule(3 * PT);
        assert_eq!((t.ty(r), t.height(r), t.depth(r), t.width(r)), (RULE_NODE, 3 * PT, 0, NULL_FLAG));
        t.print_size(TEXT_SIZE);
        assert_eq!(printed(t), "\\textfont");
        t.print_size(SCRIPT_SIZE);
        assert_eq!(printed(t), "\\scriptfont");
        t.print_size(SCRIPT_SCRIPT_SIZE);
        assert_eq!(printed(t), "\\scriptscriptfont");
        Tex::dealloc(t);
    }

    #[test]
    fn make_vcenter_centres_on_the_axis_of_the_size() {
        let t = math_machine();
        let q = t.new_noad();
        let v = t.new_null_box();
        t.set_ty(v, VLIST_NODE);
        t.set_height(v, 6 * PT);
        t.set_depth(v, 3 * PT + 1);
        t.set_info(t.nucleus(q), v);
        t.make_vcenter(q);
        // half(9pt+1) rounds up, 3pt is the axis
        assert_eq!(t.height(v), 3 * PT + (9 * PT + 2) / 2);
        assert_eq!(t.height(v) + t.depth(v), 9 * PT + 1);
        Tex::dealloc(t);
    }

    #[test]
    fn clean_box_takes_a_clean_box_as_it_is() {
        let t = math_machine();
        delimiter_font(t, 3);
        let x = t.get_node(1);
        t.set_math_type(x, SUB_BOX);
        let b = t.new_null_box();
        t.set_info(x, b);
        assert_eq!(t.clean_box(x, TEXT_STYLE), b);
        // One character and a kern, which goes: the box keeps its width
        let k = t.new_kern(PT);
        let c = t.get_avail();
        t.set_character(c, 10);
        t.set_font(c, 3);
        t.set_link(c, k);
        t.set_list_ptr(b, c);
        t.set_width(b, 5 * PT);
        let used = t.var_used;
        assert_eq!(t.clean_box(x, TEXT_STYLE), b);
        assert_eq!(t.link(c), NULL);
        assert_eq!(t.width(b), 5 * PT);
        assert_eq!(t.var_used, used - MEDIUM_NODE_SIZE);
        // Two characters, or something else after the character, stay
        let d = t.get_avail();
        t.set_character(d, 10);
        t.set_font(d, 3);
        t.set_link(c, d);
        assert_eq!(t.clean_box(x, TEXT_STYLE), b);
        assert_eq!(t.link(c), d);
        let g = t.new_glue(ZERO_GLUE);
        t.set_link(c, g);
        assert_eq!(t.clean_box(x, TEXT_STYLE), b);
        assert_eq!(t.link(c), g);
        Tex::dealloc(t);
    }

    #[test]
    fn half64_rounds_as_half_does() {
        let t = machine();
        for x in [-5, -4, -3, -2, -1, 0, 1, 2, 3, 4, 5, 655361, -655361, i32::MAX, -i32::MAX] {
            assert_eq!(half64(x as i64), t.half(x) as i64, "{}", x);
        }
        assert_eq!(half64(1 << 40), 1 << 39);
        assert_eq!(half64((1 << 40) + 1), (1 << 39) + 1);
        assert_eq!(half64(-(1 << 40) - 1), -(1 << 39));
        Tex::dealloc(t);
    }
}
