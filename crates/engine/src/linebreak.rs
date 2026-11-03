//! §989-1067 Parts 38-39: breaking paragraphs into lines.
//!
//! `line_break` keeps WEB's order but runs through two helpers, `linebreak_prepare` (§992, §1003, §1010, §1024) and
//! `linebreak_find_breaks` (§1039), so that the search for breakpoints can be tested without `hpack`.
//!
//! The oracle's scaled numbers have 64 bits, and nothing in WEB bounds the running widths, which add up everything
//! after the first active node.  A line's own width is such a sum less the deltas before its node, so a sum clamped at
//! 2^31sp would change lines whose own width is far below that: `etex/lastlinefit`, with `plus 16383pt` in each space,
//! gets 2pt of stretch for a line that has 16383pt.  So `active_width`, `cur_active_width`, `break_width`, `background`
//! and `disc_width` are `i64`, as are `shortfall`, `g`, `best_pl_short` and `best_pl_glue`, and this file has the oracle's
//! values.  A delta node's words are `sc`s until one of them needs more, when its `subtype` says all are whole words
//! (`linebreak_delta`); `active_short` and `active_glue` are always whole words.  Nothing outside this file reads either.
//!
//! DROPPED §1119: it only declares the procedures for hyphenation patterns, which are hyph.rs's.
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

/// §999 `do_all_eight(m)`: the body for `k` from 1 to 6, then 7 and 8 under `\pdfadjustspacing>1`.
macro_rules! do_all_eight {
    ($t:ident, |$k:ident| $body:expr) => {{
        for $k in 1..7 {
            $body;
        }
        if $t.pdf_adjust_spacing() > 1 {
            for $k in 7..9 {
                $body;
            }
        }
    }};
}

/// §999 `do_one_seven_eight(m)`.
macro_rules! do_one_seven_eight {
    ($t:ident, |$k:ident| $body:expr) => {{
        {
            let $k = 1;
            $body;
        }
        if $t.pdf_adjust_spacing() > 1 {
            for $k in 7..9 {
                $body;
            }
        }
    }};
}

impl Tex {
    /// §991
    pub fn line_break(&mut self, d: bool) {
        // §1038 and §1070: the locals are in `linebreak_find_breaks`.
        self.pack_begin_line = self.mode_line();
        self.linebreak_prepare();
        // §1039
        self.linebreak_find_breaks();
        // §1052
        self.post_line_break(d);
        // §1041
        self.linebreak_clean_up();
        self.pack_begin_line = 0;
    }

    /// §992, §1003, §1010 and §1024: the four pieces of `@<Get ready to start line breaking@>`, in order.
    fn linebreak_prepare(&mut self) {
        // §992
        self.set_link(TEMP_HEAD, self.link(self.head()));
        if self.is_char_node(self.tail()) {
            let p = self.new_penalty(INF_PENALTY);
            self.tail_append(p);
        } else if self.ty(self.tail()) != GLUE_NODE {
            let p = self.new_penalty(INF_PENALTY);
            self.tail_append(p);
        } else {
            let t = self.tail();
            self.set_ty(t, PENALTY_NODE);
            self.delete_glue_ref(self.glue_ptr(t));
            self.flush_node_list(self.leader_ptr(t));
            self.set_penalty(t, INF_PENALTY);
        }
        let p = self.new_param_glue(PAR_FILL_SKIP_CODE);
        self.set_link(self.tail(), p);
        self.last_line_fill = self.link(self.tail());
        self.init_cur_lang = self.prev_graf() % 0o200000;
        self.init_l_hyf = self.prev_graf() / 0o20000000;
        self.init_r_hyf = (self.prev_graf() / 0o200000) % 0o100;
        self.pop_nest();

        // §1003
        self.no_shrink_error_yet = true;
        // §1001 check_shrinkage
        if self.shrink_order(self.left_skip()) != NORMAL && self.shrink(self.left_skip()) != 0 {
            let v = self.finite_shrink(self.left_skip());
            self.set_left_skip(v);
        }
        if self.shrink_order(self.right_skip()) != NORMAL && self.shrink(self.right_skip()) != 0 {
            let v = self.finite_shrink(self.right_skip());
            self.set_right_skip(v);
        }
        let mut q = self.left_skip();
        let r = self.right_skip();
        self.background[1] = self.width(q) as i64 + self.width(r) as i64;
        self.background[2] = 0;
        self.background[3] = 0;
        self.background[4] = 0;
        self.background[5] = 0;
        let o = 2 + self.stretch_order(q);
        self.background[o] = self.stretch(q) as i64;
        let o = 2 + self.stretch_order(r);
        self.background[o] += self.stretch(r) as i64;
        self.background[6] = self.shrink(q) as i64 + self.shrink(r) as i64;
        if self.pdf_adjust_spacing() > 1 {
            self.background[7] = 0;
            self.background[8] = 0;
            self.max_stretch_ratio = -1;
            self.max_shrink_ratio = -1;
            self.cur_font_step = -1;
            self.prev_char_p = NULL;
        }

        // §1843
        self.do_last_line_fit = false;
        self.active_node_size = ACTIVE_NODE_SIZE_NORMAL;
        if self.last_line_fit() > 0 {
            q = self.glue_ptr(self.last_line_fill);
            if self.stretch(q) > 0 && self.stretch_order(q) > NORMAL {
                if self.background[3] == 0 && self.background[4] == 0 && self.background[5] == 0 {
                    self.do_last_line_fit = true;
                    self.active_node_size = ACTIVE_NODE_SIZE_EXTENDED;
                    self.fill_width[0] = 0;
                    self.fill_width[1] = 0;
                    self.fill_width[2] = 0;
                    let o = self.stretch_order(q) - 1;
                    self.fill_width[o] = self.stretch(q);
                }
            }
        }

        // §1010
        self.minimum_demerits = AWFUL_BAD as i64;
        self.minimal_demerits[TIGHT_FIT] = AWFUL_BAD as i64;
        self.minimal_demerits[DECENT_FIT] = AWFUL_BAD as i64;
        self.minimal_demerits[LOOSE_FIT] = AWFUL_BAD as i64;
        self.minimal_demerits[VERY_LOOSE_FIT] = AWFUL_BAD as i64;

        // §1024
        if self.par_shape_ptr() == NULL {
            if self.hang_indent() == 0 {
                self.last_special_line = 0;
                self.second_width = self.hsize();
                self.second_indent = 0;
            } else {
                // §1025
                self.last_special_line = abs(self.hang_after());
                if self.hang_after() < 0 {
                    self.first_width = self.hsize() - abs(self.hang_indent());
                    if self.hang_indent() >= 0 {
                        self.first_indent = self.hang_indent();
                    } else {
                        self.first_indent = 0;
                    }
                    self.second_width = self.hsize();
                    self.second_indent = 0;
                } else {
                    self.first_width = self.hsize();
                    self.first_indent = 0;
                    self.second_width = self.hsize() - abs(self.hang_indent());
                    if self.hang_indent() >= 0 {
                        self.second_indent = self.hang_indent();
                    } else {
                        self.second_indent = 0;
                    }
                }
            }
        } else {
            self.last_special_line = self.info(self.par_shape_ptr()) - 1;
            self.second_width = self.mem[self.par_shape_ptr() + 2 * (self.last_special_line + 1)].sc();
            self.second_indent = self.mem[self.par_shape_ptr() + 2 * self.last_special_line + 1].sc();
        }
        if self.looseness() == 0 {
            self.easy_line = self.last_special_line;
        } else {
            self.easy_line = MAX_HALFWORD;
        }
    }

    /// §1041
    fn linebreak_clean_up(&mut self) {
        let mut q = self.link(ACTIVE);
        while q != LAST_ACTIVE {
            self.cur_p = self.link(q);
            if self.ty(q) == DELTA_NODE {
                self.free_node(q, DELTA_NODE_SIZE);
            } else {
                self.free_node(q, self.active_node_size);
            }
            q = self.cur_p;
        }
        q = self.passive;
        while q != NULL {
            self.cur_p = self.link(q);
            self.free_node(q, PASSIVE_NODE_SIZE);
            q = self.cur_p;
        }
    }

    /// §1039
    fn linebreak_find_breaks(&mut self) {
        // §1038
        let mut q: i32;
        let mut r: i32;
        let mut s: i32;
        let mut prev_s: i32;
        let mut f: i32;
        // §1070
        let mut j: i32;
        let mut c: i32;

        self.threshold = self.pretolerance();
        if self.threshold >= 0 {
            if self.tracing_paragraphs() > 0 {
                self.begin_diagnostic();
                self.print_nl_s(b"@firstpass");
            }
            self.second_pass = false;
            self.final_pass = false;
        } else {
            self.threshold = self.tolerance();
            self.second_pass = true;
            self.final_pass = self.emergency_stretch() <= 0;
            if self.tracing_paragraphs() > 0 {
                self.begin_diagnostic();
            }
        }
        'done: loop {
            if self.threshold > INF_BAD {
                self.threshold = INF_BAD;
            }
            if self.second_pass {
                // §1068
                if self.trie_not_ready {
                    self.init_trie();
                }
                self.cur_lang = self.init_cur_lang;
                self.l_hyf = self.init_l_hyf;
                self.r_hyf = self.init_r_hyf;
                self.set_hyph_index();
            }

            // §1040
            q = self.get_node(self.active_node_size);
            self.set_ty(q, UNHYPHENATED);
            self.set_fitness(q, DECENT_FIT);
            self.set_link(q, LAST_ACTIVE);
            self.set_break_node(q, NULL);
            self.set_line_number(q, self.prev_graf() + 1);
            self.linebreak_set_total_demerits(q, 0);
            self.set_link(ACTIVE, q);
            if self.do_last_line_fit {
                // §1845
                self.linebreak_set_active_short(q, 0);
                self.linebreak_set_active_glue(q, 0);
            }
            do_all_eight!(self, |k| self.active_width[k] = self.background[k]);
            self.passive = NULL;
            self.printed_node = TEMP_HEAD;
            self.pass_number = 0;
            self.font_in_short_display = NULL_FONT;

            self.cur_p = self.link(TEMP_HEAD);
            self.auto_breaking = true;
            self.prev_p = self.cur_p; // glue at beginning is not a legal breakpoint
            self.prev_char_p = NULL;
            self.prev_legal = NULL;
            self.rejected_cur_p = NULL;
            self.try_prev_break = false;
            self.before_rejected_cur_p = false;
            self.first_p = self.cur_p;
            while self.cur_p != NULL && self.link(ACTIVE) != LAST_ACTIVE {
                // §1042
                if self.is_char_node(self.cur_p) {
                    // §1043, with act_width in a local for the run of characters
                    self.prev_p = self.cur_p;
                    let mut act_width = self.active_width[1];
                    loop {
                        f = self.font(self.cur_p);
                        let ci = self.char_info(f, self.character(self.cur_p));
                        act_width += self.char_width(f, ci) as i64;
                        if self.pdf_adjust_spacing() > 1 && self.check_expand_pars(f) {
                            self.prev_char_p = self.cur_p;
                            let cc = self.character(self.cur_p);
                            let v = self.char_stretch(f, cc);
                            self.active_width[7] += v as i64;
                            let v = self.char_shrink(f, cc);
                            self.active_width[8] += v as i64;
                        }
                        self.cur_p = self.link(self.cur_p);
                        if !self.is_char_node(self.cur_p) {
                            break;
                        }
                    }
                    self.active_width[1] = act_width;
                }
                'done5: {
                    match self.ty(self.cur_p) {
                        HLIST_NODE | VLIST_NODE | RULE_NODE => {
                            self.active_width[1] += self.width(self.cur_p) as i64;
                        }
                        WHATSIT_NODE => {
                            // §1609
                            self.adv_past(self.cur_p);
                            if self.subtype(self.cur_p) == PDF_REFXFORM_NODE
                                || self.subtype(self.cur_p) == PDF_REFXIMAGE_NODE
                            {
                                self.active_width[1] += self.pdf_width(self.cur_p) as i64;
                            }
                        }
                        GLUE_NODE => {
                            // §1044
                            if self.auto_breaking {
                                if self.is_char_node(self.prev_p) {
                                    self.try_break(0, UNHYPHENATED);
                                } else if self.precedes_break(self.prev_p) {
                                    self.try_break(0, UNHYPHENATED);
                                } else if self.ty(self.prev_p) == KERN_NODE && self.subtype(self.prev_p) != EXPLICIT {
                                    self.try_break(0, UNHYPHENATED);
                                }
                            }
                            let g = self.glue_ptr(self.cur_p);
                            if self.shrink_order(g) != NORMAL && self.shrink(g) != 0 {
                                let v = self.finite_shrink(g);
                                self.set_glue_ptr(self.cur_p, v);
                            }
                            q = self.glue_ptr(self.cur_p);
                            self.active_width[1] += self.width(q) as i64;
                            let o = 2 + self.stretch_order(q);
                            self.active_width[o] += self.stretch(q) as i64;
                            self.active_width[6] += self.shrink(q) as i64;
                            if self.second_pass && self.auto_breaking {
                                // §1071
                                prev_s = self.cur_p;
                                s = self.link(prev_s);
                                if s != NULL {
                                    'done1: {
                                        // §1073
                                        'done2: loop {
                                            'cont: {
                                                if self.is_char_node(s) {
                                                    c = self.character(s);
                                                    self.hf = self.font(s);
                                                } else if self.ty(s) == LIGATURE_NODE {
                                                    if self.lig_ptr(s) == NULL {
                                                        break 'cont;
                                                    } else {
                                                        q = self.lig_ptr(s);
                                                        c = self.character(q);
                                                        self.hf = self.font(q);
                                                    }
                                                } else if self.ty(s) == KERN_NODE && self.subtype(s) == NORMAL {
                                                    break 'cont;
                                                } else if self.ty(s) == MATH_NODE && self.subtype(s) >= L_CODE {
                                                    break 'cont;
                                                } else if self.ty(s) == WHATSIT_NODE {
                                                    // §1610
                                                    if self.subtype(s) == LANGUAGE_NODE {
                                                        self.cur_lang = self.what_lang(s);
                                                        self.l_hyf = self.what_lhm(s);
                                                        self.r_hyf = self.what_rhm(s);
                                                        self.set_hyph_index();
                                                    }
                                                    break 'cont;
                                                } else {
                                                    break 'done1;
                                                }
                                                self.set_hc0_lc_code(c);
                                                if self.hc[0] != 0 {
                                                    if self.hc[0] == c || self.uc_hyph() > 0 {
                                                        break 'done2;
                                                    } else {
                                                        break 'done1;
                                                    }
                                                }
                                            }
                                            prev_s = s;
                                            s = self.link(prev_s);
                                        }
                                        // done2:
                                        self.hyf_char = self.hyphen_char[self.hf];
                                        if self.hyf_char < 0 {
                                            break 'done1;
                                        }
                                        if self.hyf_char > 255 {
                                            break 'done1;
                                        }
                                        self.ha = prev_s;
                                        if self.l_hyf + self.r_hyf > 63 {
                                            break 'done1;
                                        }
                                        // §1074
                                        self.hn = 0;
                                        'done3: loop {
                                            if self.is_char_node(s) {
                                                if self.font(s) != self.hf {
                                                    break 'done3;
                                                }
                                                self.hyf_bchar = self.character(s);
                                                c = self.hyf_bchar;
                                                self.set_hc0_lc_code(c);
                                                if self.hc[0] == 0 {
                                                    break 'done3;
                                                }
                                                if self.hn == 63 {
                                                    break 'done3;
                                                }
                                                self.hb = s;
                                                self.hn += 1;
                                                self.hu[self.hn] = c;
                                                self.hc[self.hn] = self.hc[0];
                                                self.hyf_bchar = NON_CHAR;
                                            } else if self.ty(s) == LIGATURE_NODE {
                                                // §1075
                                                if self.font(self.lig_char(s)) != self.hf {
                                                    break 'done3;
                                                }
                                                j = self.hn;
                                                q = self.lig_ptr(s);
                                                if q > NULL {
                                                    self.hyf_bchar = self.character(q);
                                                }
                                                while q > NULL {
                                                    c = self.character(q);
                                                    self.set_hc0_lc_code(c);
                                                    if self.hc[0] == 0 {
                                                        break 'done3;
                                                    }
                                                    if j == 63 {
                                                        break 'done3;
                                                    }
                                                    j += 1;
                                                    self.hu[j] = c;
                                                    self.hc[j] = self.hc[0];
                                                    q = self.link(q);
                                                }
                                                self.hb = s;
                                                self.hn = j;
                                                if odd(self.subtype(s)) {
                                                    self.hyf_bchar = self.font_bchar[self.hf];
                                                } else {
                                                    self.hyf_bchar = NON_CHAR;
                                                }
                                            } else if self.ty(s) == KERN_NODE && self.subtype(s) == NORMAL {
                                                self.hb = s;
                                                self.hyf_bchar = self.font_bchar[self.hf];
                                            } else {
                                                break 'done3;
                                            }
                                            s = self.link(s);
                                        }
                                        // §1076
                                        if self.hn < self.l_hyf + self.r_hyf {
                                            break 'done1; // l_hyf and r_hyf are >=1
                                        }
                                        'done4: loop {
                                            if !self.is_char_node(s) {
                                                match self.ty(s) {
                                                    LIGATURE_NODE => {}
                                                    KERN_NODE => {
                                                        if self.subtype(s) != NORMAL {
                                                            break 'done4;
                                                        }
                                                    }
                                                    WHATSIT_NODE | GLUE_NODE | PENALTY_NODE | INS_NODE | ADJUST_NODE
                                                    | MARK_NODE => break 'done4,
                                                    MATH_NODE => {
                                                        if self.subtype(s) >= L_CODE {
                                                            break 'done4;
                                                        } else {
                                                            break 'done1;
                                                        }
                                                    }
                                                    _ => break 'done1,
                                                }
                                            }
                                            s = self.link(s);
                                        }
                                        self.hyphenate();
                                    }
                                }
                            }
                        }
                        KERN_NODE => {
                            if self.subtype(self.cur_p) == EXPLICIT {
                                self.linebreak_kern_break();
                            } else {
                                self.active_width[1] += self.width(self.cur_p) as i64;
                                if self.pdf_adjust_spacing() > 1 && self.subtype(self.cur_p) == NORMAL {
                                    let v = self.kern_stretch(self.cur_p);
                                    self.active_width[7] += v as i64;
                                    let v = self.kern_shrink(self.cur_p);
                                    self.active_width[8] += v as i64;
                                }
                            }
                        }
                        LIGATURE_NODE => {
                            f = self.font(self.lig_char(self.cur_p));
                            let cc = self.character(self.lig_char(self.cur_p));
                            let ci = self.char_info(f, cc);
                            self.active_width[1] += self.char_width(f, ci) as i64;
                            if self.pdf_adjust_spacing() > 1 && self.check_expand_pars(f) {
                                self.prev_char_p = self.cur_p;
                                let v = self.char_stretch(f, cc);
                                self.active_width[7] += v as i64;
                                let v = self.char_shrink(f, cc);
                                self.active_width[8] += v as i64;
                            }
                        }
                        DISC_NODE => {
                            // §1045
                            s = self.pre_break(self.cur_p);
                            do_one_seven_eight!(self, |k| self.disc_width[k] = 0);
                            if s == NULL {
                                self.try_break(self.ex_hyphen_penalty(), HYPHENATED);
                            } else {
                                loop {
                                    // §1046
                                    let (w, st, sh) = self.linebreak_item_widths(s, b"disc3");
                                    self.disc_width[1] += w as i64;
                                    self.disc_width[7] += st as i64;
                                    self.disc_width[8] += sh as i64;
                                    s = self.link(s);
                                    if s == NULL {
                                        break;
                                    }
                                }
                                do_one_seven_eight!(self, |k| self.active_width[k] += self.disc_width[k]);
                                self.try_break(self.hyphen_penalty(), HYPHENATED);
                                do_one_seven_eight!(self, |k| self.active_width[k] -= self.disc_width[k]);
                            }
                            r = self.replace_count(self.cur_p);
                            s = self.link(self.cur_p);
                            while r > 0 {
                                // §1047
                                let (w, st, sh) = self.linebreak_item_widths(s, b"disc4");
                                self.active_width[1] += w as i64;
                                self.active_width[7] += st as i64;
                                self.active_width[8] += sh as i64;
                                r -= 1;
                                s = self.link(s);
                            }
                            self.prev_p = self.cur_p;
                            self.cur_p = s;
                            break 'done5;
                        }
                        MATH_NODE => {
                            if self.subtype(self.cur_p) < L_CODE {
                                self.auto_breaking = odd(self.subtype(self.cur_p));
                            }
                            self.linebreak_kern_break();
                        }
                        PENALTY_NODE => self.try_break(self.penalty(self.cur_p), UNHYPHENATED),
                        MARK_NODE | INS_NODE | ADJUST_NODE => {}
                        _ => self.confusion(b"paragraph"),
                    }
                    self.prev_p = self.cur_p;
                    self.cur_p = self.link(self.cur_p);
                }
            }
            if self.cur_p == NULL {
                // §1049
                self.try_break(EJECT_PENALTY, HYPHENATED);
                if self.link(ACTIVE) != LAST_ACTIVE {
                    // §1050
                    r = self.link(ACTIVE);
                    self.fewest_demerits = AWFUL_BAD as i64;
                    loop {
                        if self.ty(r) != DELTA_NODE && self.linebreak_total_demerits(r) < self.fewest_demerits {
                            self.fewest_demerits = self.linebreak_total_demerits(r);
                            self.best_bet = r;
                        }
                        r = self.link(r);
                        if r == LAST_ACTIVE {
                            break;
                        }
                    }
                    self.best_line = self.line_number(self.best_bet);
                    if self.looseness() == 0 {
                        break 'done;
                    }
                    // §1051
                    r = self.link(ACTIVE);
                    self.actual_looseness = 0;
                    loop {
                        if self.ty(r) != DELTA_NODE {
                            self.line_diff = self.line_number(r) - self.best_line;
                            if (self.line_diff < self.actual_looseness && self.looseness() <= self.line_diff)
                                || (self.line_diff > self.actual_looseness && self.looseness() >= self.line_diff)
                            {
                                self.best_bet = r;
                                self.actual_looseness = self.line_diff;
                                self.fewest_demerits = self.linebreak_total_demerits(r);
                            } else if self.line_diff == self.actual_looseness
                                && self.linebreak_total_demerits(r) < self.fewest_demerits
                            {
                                self.best_bet = r;
                                self.fewest_demerits = self.linebreak_total_demerits(r);
                            }
                        }
                        r = self.link(r);
                        if r == LAST_ACTIVE {
                            break;
                        }
                    }
                    self.best_line = self.line_number(self.best_bet);
                    if self.actual_looseness == self.looseness() || self.final_pass {
                        break 'done;
                    }
                }
            }
            // §1041
            self.linebreak_clean_up();
            if !self.second_pass {
                if self.tracing_paragraphs() > 0 {
                    self.print_nl_s(b"@secondpass");
                }
                self.threshold = self.tolerance();
                self.second_pass = true;
                self.final_pass = self.emergency_stretch() <= 0;
            } else {
                // if at first you don't succeed, ...
                if self.tracing_paragraphs() > 0 {
                    self.print_nl_s(b"@emergencypass");
                }
                self.background[2] += self.emergency_stretch() as i64;
                self.final_pass = true;
            }
        }
        // done:
        if self.tracing_paragraphs() > 0 {
            self.end_diagnostic(true);
            self.normalize_selector();
        }
        if self.do_last_line_fit {
            // §1853
            if self.linebreak_active_short(self.best_bet) == 0 {
                self.do_last_line_fit = false;
            } else {
                q = self.new_spec(self.glue_ptr(self.last_line_fill));
                self.delete_glue_ref(self.glue_ptr(self.last_line_fill));
                let w = self.width(q) as i64 + self.linebreak_active_short(self.best_bet) - self.linebreak_active_glue(self.best_bet);
                self.set_width(q, sat32(w));
                self.set_stretch(q, 0);
                self.set_glue_ptr(self.last_line_fill, q);
            }
        }
    }

    /// §997 `total_demerits(p)`, in a whole word: section 11.
    #[inline]
    fn linebreak_total_demerits(&self, p: i32) -> i64 {
        self.mem[p + 2].raw() as i64
    }
    /// §997 `total_demerits(p):=v`.
    #[inline]
    fn linebreak_set_total_demerits(&mut self, p: i32, v: i64) {
        self.mem[p + 2] = Word::from_raw(v as u64);
    }
    /// §1842 `active_short(p)`, in a whole word: section 11.
    #[inline]
    fn linebreak_active_short(&self, p: i32) -> i64 {
        self.mem[p + 3].raw() as i64
    }
    /// §1842 `active_short(p):=v`.
    #[inline]
    fn linebreak_set_active_short(&mut self, p: i32, v: i64) {
        self.mem[p + 3] = Word::from_raw(v as u64);
    }
    /// §1842 `active_glue(p)`, in a whole word: section 11.
    #[inline]
    fn linebreak_active_glue(&self, p: i32) -> i64 {
        self.mem[p + 4].raw() as i64
    }
    /// §1842 `active_glue(p):=v`.
    #[inline]
    fn linebreak_set_active_glue(&mut self, p: i32, v: i64) {
        self.mem[p + 4] = Word::from_raw(v as u64);
    }

    /// Word `k` of the delta node `r`.  Its `subtype`, which WEB does not use, says whether the words are `sc`s or, when one
    /// of them left the range of `i32`, whole words.  An `sc` leaves `lh` as it was, like the oracle's, whose `sc` is
    /// apart from its `hh`: raw.rs says what can read that.
    #[inline(always)]
    fn linebreak_delta(&self, r: i32, k: i32) -> i64 {
        if self.subtype(r) == 0 { self.mem[r + k].sc() as i64 } else { self.mem[r + k].raw() as i64 }
    }

    /// Words 1 to 6 of the delta node `q`, and 7 and 8 under `\pdfadjustspacing>1`, from `v`, whose slot 0 is unused.
    fn linebreak_set_delta(&mut self, q: i32, v: &[i64; 9]) {
        let last = if self.pdf_adjust_spacing() > 1 { 8 } else { 6 };
        let wide = v[1..=last].iter().any(|&x| x != x as i32 as i64);
        self.set_subtype(q, wide as i32);
        for k in 1..=last {
            if wide {
                self.mem[q + k as i32] = Word::from_raw(v[k] as u64);
            } else {
                self.mem[q + k as i32].set_sc(v[k] as i32);
            }
        }
    }

    /// §1042 `kern_break`.
    #[inline(always)]
    fn linebreak_kern_break(&mut self) {
        if !self.is_char_node(self.link(self.cur_p)) && self.auto_breaking {
            if self.ty(self.link(self.cur_p)) == GLUE_NODE {
                self.try_break(0, UNHYPHENATED);
            }
        }
        self.active_width[1] += self.width(self.cur_p) as i64;
    }

    /// §1017, §1018, §1046 and §1047, which differ only in where the three results go and in the message: the width of
    /// a node of a discretionary, and its stretch and shrink under font expansion (§1015), which are 0 otherwise.
    fn linebreak_item_widths(&mut self, s: i32, disc: &[u8]) -> (i32, i32, i32) {
        let w;
        let (mut st, mut sh) = (0, 0);
        if self.is_char_node(s) {
            let f = self.font(s);
            let c = self.character(s);
            w = self.char_width(f, self.char_info(f, c));
            if self.pdf_adjust_spacing() > 1 && self.check_expand_pars(f) {
                self.prev_char_p = s;
                st = self.char_stretch(f, c);
                sh = self.char_shrink(f, c);
            }
        } else {
            match self.ty(s) {
                LIGATURE_NODE => {
                    let f = self.font(self.lig_char(s));
                    let c = self.character(self.lig_char(s));
                    w = self.char_width(f, self.char_info(f, c));
                    if self.pdf_adjust_spacing() > 1 && self.check_expand_pars(f) {
                        self.prev_char_p = s;
                        st = self.char_stretch(f, c);
                        sh = self.char_shrink(f, c);
                    }
                }
                HLIST_NODE | VLIST_NODE | RULE_NODE | KERN_NODE => {
                    w = self.width(s);
                    if self.ty(s) == KERN_NODE && self.pdf_adjust_spacing() > 1 && self.subtype(s) == NORMAL {
                        st = self.kern_stretch(s);
                        sh = self.kern_shrink(s);
                    }
                }
                _ => self.confusion(disc),
            }
        }
        (w, st, sh)
    }

    /// §1002
    pub fn finite_shrink(&mut self, p: i32) -> i32 {
        if self.no_shrink_error_yet {
            self.no_shrink_error_yet = false;
            if self.tracing_paragraphs() > 0 {
                self.end_diagnostic(true);
            }
            self.print_err_s(b"Infinite glue shrinkage found in a paragraph");
            self.help(&[
                b"The paragraph just ended includes some glue that has",
                b"infinite shrinkability, e.g., `\\hskip 0pt minus 1fil'.",
                b"Such glue doesn't belong there---it allows a paragraph",
                b"of any length to fit on one line. But it's safe to proceed,",
                b"since the offensive shrinkability has been made finite.",
            ]);
            self.error();
            if self.tracing_paragraphs() > 0 {
                self.begin_diagnostic();
            }
        }
        let q = self.new_spec(p);
        self.set_shrink_order(q, NORMAL);
        self.delete_glue_ref(p);
        q
    }

    /// §1005
    pub fn push_node(&mut self, p: i32) {
        if self.hlist_stack_level > MAX_HLIST_STACK {
            self.pdf_error(b"push_node", b"stack overflow");
        }
        self.hlist_stack[self.hlist_stack_level] = p;
        self.hlist_stack_level += 1;
    }

    /// §1005
    pub fn pop_node(&mut self) -> i32 {
        self.hlist_stack_level -= 1;
        if self.hlist_stack_level < 0 {
            self.pdf_error(b"pop_node", b"stack underflow (internal error)");
        }
        self.hlist_stack[self.hlist_stack_level]
    }

    /// §1005
    pub fn find_protchar_left(&mut self, l: i32, d: bool) -> i32 {
        let mut l = l;
        if self.link(l) != NULL
            && self.ty(l) == HLIST_NODE
            && self.width(l) == 0
            && self.height(l) == 0
            && self.depth(l) == 0
            && self.list_ptr(l) == NULL
        {
            l = self.link(l); // for paragraph start with \parindent = 0pt
        } else if d {
            while self.link(l) != NULL && !(self.is_char_node(l) || self.non_discardable(l)) {
                l = self.link(l); // std. discardables at line break, TeXbook, p 95
            }
        }
        self.hlist_stack_level = 0;
        let mut run = true;
        loop {
            let t = l;
            while run && self.ty(l) == HLIST_NODE && self.list_ptr(l) != NULL {
                self.push_node(l);
                l = self.list_ptr(l);
            }
            while run && self.cp_skipable(l) {
                while self.link(l) == NULL && self.hlist_stack_level > 0 {
                    l = self.pop_node(); // don't visit this node again
                }
                if self.link(l) != NULL {
                    l = self.link(l);
                } else if self.hlist_stack_level == 0 {
                    run = false;
                }
            }
            if t == l {
                break;
            }
        }
        l
    }

    /// §1005
    pub fn find_protchar_right(&mut self, l: i32, r: i32) -> i32 {
        let (mut l, mut r) = (l, r);
        if r == NULL {
            return NULL;
        }
        self.hlist_stack_level = 0;
        let mut run = true;
        loop {
            let t = r;
            while run && self.ty(r) == HLIST_NODE && self.list_ptr(r) != NULL {
                self.push_node(l);
                self.push_node(r);
                l = self.list_ptr(r);
                r = l;
                while self.link(r) != NULL {
                    r = self.link(r);
                }
            }
            while run && self.cp_skipable(r) {
                while r == l && self.hlist_stack_level > 0 {
                    r = self.pop_node(); // don't visit this node again
                    l = self.pop_node();
                }
                if r != l && r != NULL {
                    r = self.prev_rightmost(l, r);
                } else if r == l && self.hlist_stack_level == 0 {
                    run = false;
                }
            }
            if t == r {
                break;
            }
        }
        r
    }

    /// §1005
    pub fn total_pw(&mut self, q: i32, p: i32) -> i32 {
        let mut l;
        let mut r;
        if self.break_node(q) == NULL {
            l = self.first_p;
        } else {
            l = self.cur_break(self.break_node(q));
        }
        r = self.prev_rightmost(self.prev_p, p); // get link(r)=p
        // let's look at the right margin first
        if p != NULL && self.ty(p) == DISC_NODE && self.pre_break(p) != NULL {
            // a disc_node with non-empty pre_break, protrude the last char of pre_break
            r = self.pre_break(p);
            while self.link(r) != NULL {
                r = self.link(r);
            }
        } else {
            r = self.find_protchar_right(l, r);
        }
        // now the left margin
        'done: {
            if l != NULL && self.ty(l) == DISC_NODE {
                if self.post_break(l) != NULL {
                    l = self.post_break(l); // protrude the first char
                    break 'done;
                } else {
                    // discard replace_count(l) nodes
                    let mut n = self.replace_count(l);
                    l = self.link(l);
                    while n > 0 {
                        if self.link(l) != NULL {
                            l = self.link(l);
                        }
                        n -= 1;
                    }
                }
            }
            l = self.find_protchar_left(l, true);
        }
        let a = self.left_pw(l);
        let b = self.right_pw(r);
        a + b
    }

    /// §822's `cal_margin_kern_var(p)`, with the avail node `cp`.  It says `left_pw` for the right margin's node too.
    fn linebreak_cal_margin_kern_var(&mut self, cp: i32, p: i32, stretch: &mut i32, shrink: &mut i32) {
        // With no protrusion for the character in its font or either expansion, all this adds 0, and if do_subst_font
        // already knows those two it makes no font either.  That is 6 of 7 calls on SGA3.  last_leftmost_char, which
        // left_pw would set, is set again before it is next read.
        let f = self.font(p);
        let c = self.character(p);
        if self.get_lp_code(f, c) == 0 {
            if let (Some(k1), Some(k2)) = (self.pack_subst_known(f, c, 1000), self.pack_subst_known(f, c, -1000)) {
                if self.get_lp_code(k1, c) == 0 && self.get_lp_code(k2, c) == 0 {
                    return;
                }
            }
        }
        self.set_character(cp, self.character(p));
        self.set_font(cp, self.font(p));
        self.do_subst_font(cp, 1000);
        if self.font(cp) != self.font(p) {
            let a = self.left_pw(p);
            let b = self.left_pw(cp);
            *stretch = *stretch + a - b;
        }
        self.set_font(cp, self.font(p));
        self.do_subst_font(cp, -1000);
        if self.font(cp) != self.font(p) {
            let a = self.left_pw(cp);
            let b = self.left_pw(p);
            *shrink = *shrink + a - b;
        }
    }

    /// §1005
    pub fn try_break(&mut self, pi: i32, break_type: i32) {
        let mut pi = pi;
        'exit: {
            // §1007
            if abs(pi) >= INF_PENALTY {
                if pi > 0 {
                    break 'exit; // this breakpoint is inhibited by infinite penalty
                } else {
                    pi = EJECT_PENALTY; // this breakpoint will be forced
                }
            }
            let mut no_break_yet = true;
            let mut prev_r = ACTIVE;
            let mut old_l = 0;
            // §1006: the other locals are declared where they are first set.  These two live across the iterations.
            let mut prev_prev_r = NULL;
            let mut line_width = 0;
            do_all_eight!(self, |k| self.cur_active_width[k] = self.active_width[k]);
            'cont: loop {
                let mut r = self.link(prev_r);
                // §1008
                if self.ty(r) == DELTA_NODE {
                    if self.subtype(r) == 0 {
                        do_all_eight!(self, |k| self.cur_active_width[k] += self.mem[r + k].sc() as i64);
                    } else {
                        do_all_eight!(self, |k| self.cur_active_width[k] += self.mem[r + k].raw() as i64);
                    }
                    prev_prev_r = prev_r;
                    prev_r = r;
                    continue 'cont;
                }
                // §1011
                let l = self.line_number(r);
                if l > old_l {
                    // now we are no longer in the inner loop
                    if self.minimum_demerits < AWFUL_BAD as i64 && (old_l != self.easy_line || r == LAST_ACTIVE) {
                        // §1012
                        if no_break_yet {
                            // §1013
                            no_break_yet = false;
                            do_all_eight!(self, |k| self.break_width[k] = self.background[k]);
                            let mut s = self.cur_p;
                            if break_type > UNHYPHENATED && self.cur_p != NULL {
                                // §1016
                                let mut t = self.replace_count(self.cur_p);
                                let mut v = self.cur_p;
                                s = self.post_break(self.cur_p);
                                while t > 0 {
                                    t -= 1;
                                    v = self.link(v);
                                    // §1017
                                    let (w, st, sh) = self.linebreak_item_widths(v, b"disc1");
                                    self.break_width[1] -= w as i64;
                                    self.break_width[7] -= st as i64;
                                    self.break_width[8] -= sh as i64;
                                }
                                while s != NULL {
                                    // §1018
                                    let (w, st, sh) = self.linebreak_item_widths(s, b"disc2");
                                    self.break_width[1] += w as i64;
                                    self.break_width[7] += st as i64;
                                    self.break_width[8] += sh as i64;
                                    s = self.link(s);
                                }
                                do_one_seven_eight!(self, |k| self.break_width[k] += self.disc_width[k]);
                                if self.post_break(self.cur_p) == NULL {
                                    s = self.link(v); // nodes may be discardable after the break
                                }
                            }
                            while s != NULL {
                                if self.is_char_node(s) {
                                    break;
                                }
                                match self.ty(s) {
                                    GLUE_NODE => {
                                        // §1014
                                        let v = self.glue_ptr(s);
                                        self.break_width[1] -= self.width(v) as i64;
                                        let o = 2 + self.stretch_order(v);
                                        self.break_width[o] -= self.stretch(v) as i64;
                                        self.break_width[6] -= self.shrink(v) as i64;
                                    }
                                    PENALTY_NODE => {}
                                    MATH_NODE => self.break_width[1] -= self.width(s) as i64,
                                    KERN_NODE => {
                                        if self.subtype(s) != EXPLICIT {
                                            break;
                                        } else {
                                            self.break_width[1] -= self.width(s) as i64;
                                        }
                                    }
                                    _ => break,
                                }
                                s = self.link(s);
                            }
                        }
                        // §1019
                        if self.ty(prev_r) == DELTA_NODE {
                            // modify an existing delta node
                            let mut v = [0i64; 9];
                            do_all_eight!(self, |k| v[k as usize] = self.linebreak_delta(prev_r, k) - self.cur_active_width[k] + self.break_width[k]);
                            self.linebreak_set_delta(prev_r, &v);
                        } else if prev_r == ACTIVE {
                            // no delta node needed at the beginning
                            do_all_eight!(self, |k| self.active_width[k] = self.break_width[k]);
                        } else {
                            let q = self.get_node(DELTA_NODE_SIZE);
                            self.set_link(q, r);
                            self.set_ty(q, DELTA_NODE);
                            let mut v = [0i64; 9];
                            do_all_eight!(self, |k| v[k as usize] = self.break_width[k] - self.cur_active_width[k]);
                            self.linebreak_set_delta(q, &v);
                            self.set_link(prev_r, q);
                            prev_prev_r = prev_r;
                            prev_r = q;
                        }
                        if abs(self.adj_demerits()) as i64 >= AWFUL_BAD as i64 - self.minimum_demerits {
                            self.minimum_demerits = AWFUL_BAD as i64 - 1;
                        } else {
                            self.minimum_demerits += abs(self.adj_demerits()) as i64;
                        }
                        for fit_class in VERY_LOOSE_FIT..=TIGHT_FIT {
                            if self.minimal_demerits[fit_class] <= self.minimum_demerits {
                                // §1021
                                let mut q = self.get_node(PASSIVE_NODE_SIZE);
                                self.set_link(q, self.passive);
                                self.passive = q;
                                self.set_cur_break(q, self.cur_p);
                                self.pass_number += 1;
                                self.set_serial(q, self.pass_number);
                                self.set_prev_break(q, self.best_place[fit_class]);
                                q = self.get_node(self.active_node_size);
                                self.set_break_node(q, self.passive);
                                self.set_line_number(q, self.best_pl_line[fit_class] + 1);
                                self.set_fitness(q, fit_class);
                                self.set_ty(q, break_type);
                                self.linebreak_set_total_demerits(q, self.minimal_demerits[fit_class]);
                                if self.do_last_line_fit {
                                    // §1851
                                    self.linebreak_set_active_short(q, self.best_pl_short[fit_class]);
                                    self.linebreak_set_active_glue(q, self.best_pl_glue[fit_class]);
                                }
                                self.set_link(q, r);
                                self.set_link(prev_r, q);
                                prev_r = q;
                                if self.tracing_paragraphs() > 0 {
                                    // §1022
                                    self.print_nl_s(b"@@");
                                    self.print_int(self.serial(self.passive));
                                    self.print_s(b": line ");
                                    self.print_int(self.line_number(q) - 1);
                                    self.print_char(b'.' as i32);
                                    self.print_int(fit_class);
                                    if break_type == HYPHENATED {
                                        self.print_char(b'-' as i32);
                                    }
                                    self.print_s(b" t=");
                                    self.print_int(self.linebreak_total_demerits(q));
                                    if self.do_last_line_fit {
                                        // §1852
                                        self.print_s(b" s=");
                                        self.print_scaled(self.linebreak_active_short(q));
                                        if self.cur_p == NULL {
                                            self.print_s(b" a=");
                                        } else {
                                            self.print_s(b" g=");
                                        }
                                        self.print_scaled(self.linebreak_active_glue(q));
                                    }
                                    self.print_s(b" -> @@");
                                    if self.prev_break(self.passive) == NULL {
                                        self.print_char(b'0' as i32);
                                    } else {
                                        self.print_int(self.serial(self.prev_break(self.passive)));
                                    }
                                }
                            }
                            self.minimal_demerits[fit_class] = AWFUL_BAD as i64;
                        }
                        self.minimum_demerits = AWFUL_BAD as i64;
                        // §1020
                        if r != LAST_ACTIVE {
                            let q = self.get_node(DELTA_NODE_SIZE);
                            self.set_link(q, r);
                            self.set_ty(q, DELTA_NODE);
                            let mut v = [0i64; 9];
                            do_all_eight!(self, |k| v[k as usize] = self.cur_active_width[k] - self.break_width[k]);
                            self.linebreak_set_delta(q, &v);
                            self.set_link(prev_r, q);
                            prev_prev_r = prev_r;
                            prev_r = q;
                        }
                    }
                    if r == LAST_ACTIVE {
                        break 'exit;
                    }
                    // §1026
                    if l > self.easy_line {
                        line_width = self.second_width;
                        old_l = MAX_HALFWORD - 1;
                    } else {
                        old_l = l;
                        if l > self.last_special_line {
                            line_width = self.second_width;
                        } else if self.par_shape_ptr() == NULL {
                            line_width = self.first_width;
                        } else {
                            line_width = self.mem[self.par_shape_ptr() + 2 * l].sc();
                        }
                    }
                }
                // §1027
                let mut artificial_demerits = false;
                // we're this much too short
                let mut shortfall = line_width as i64 - self.cur_active_width[1];
                if self.pdf_protrude_chars() > 1 {
                    shortfall += self.total_pw(r, self.cur_p) as i64;
                }
                if self.pdf_adjust_spacing() > 1 && shortfall != 0 {
                    let mut margin_kern_stretch = 0;
                    let mut margin_kern_shrink = 0;
                    if self.pdf_protrude_chars() > 1 {
                        // §822
                        let lp = self.last_leftmost_char;
                        let rp = self.last_rightmost_char;
                        let cp = self.fast_get_avail();
                        if lp != NULL {
                            self.linebreak_cal_margin_kern_var(cp, lp, &mut margin_kern_stretch, &mut margin_kern_shrink);
                        }
                        if rp != NULL {
                            self.linebreak_cal_margin_kern_var(cp, rp, &mut margin_kern_stretch, &mut margin_kern_shrink);
                        }
                        self.free_avail(cp);
                    }
                    let st = self.cur_active_width[7] + margin_kern_stretch as i64; // total_font_stretch
                    let sh = self.cur_active_width[8] + margin_kern_shrink as i64; // total_font_shrink
                    if shortfall > 0 && st > 0 {
                        if st > shortfall {
                            shortfall = (st / (self.max_stretch_ratio / self.cur_font_step) as i64) / 2;
                        } else {
                            shortfall -= st;
                        }
                    } else if shortfall < 0 && sh > 0 {
                        if sh > -shortfall {
                            shortfall = -(sh / (self.max_shrink_ratio / self.cur_font_step) as i64) / 2;
                        } else {
                            shortfall += sh;
                        }
                    }
                }
                let mut b: i32;
                let mut fit_class: i32;
                let mut g = 0i64; // §1844
                let mut node_r_stays_active;
                'deactivate: {
                    'found: {
                        if shortfall > 0 {
                            // §1028
                            if self.cur_active_width[3] != 0 || self.cur_active_width[4] != 0 || self.cur_active_width[5] != 0 {
                                if self.do_last_line_fit {
                                    if self.cur_p == NULL {
                                        // the last line of a paragraph
                                        // §1846
                                        'not_found: {
                                            let active_short = self.linebreak_active_short(r);
                                            let active_glue = self.linebreak_active_glue(r);
                                            if active_short == 0 || active_glue <= 0 {
                                                break 'not_found;
                                            }
                                            if self.cur_active_width[3] != self.fill_width[0] as i64
                                                || self.cur_active_width[4] != self.fill_width[1] as i64
                                                || self.cur_active_width[5] != self.fill_width[2] as i64
                                            {
                                                break 'not_found;
                                            }
                                            if active_short > 0 {
                                                g = self.cur_active_width[2];
                                            } else {
                                                g = self.cur_active_width[6];
                                            }
                                            if g <= 0 {
                                                break 'not_found;
                                            }
                                            self.arith_error = false;
                                            g = self.fract(g, active_short, active_glue, MAX_DIMEN) as i64;
                                            if self.last_line_fit() < 1000 {
                                                g = self.fract(g, self.last_line_fit(), 1000, MAX_DIMEN) as i64;
                                            }
                                            if self.arith_error {
                                                if active_short > 0 {
                                                    g = MAX_DIMEN as i64;
                                                } else {
                                                    g = -(MAX_DIMEN as i64);
                                                }
                                            }
                                            if g > 0 {
                                                // §1847
                                                if g > shortfall {
                                                    g = shortfall;
                                                }
                                                if g > 7230584 && self.cur_active_width[2] < 1663497 {
                                                    b = INF_BAD;
                                                    fit_class = VERY_LOOSE_FIT;
                                                    break 'found;
                                                }
                                                b = crate::pack::pack_badness(g, self.cur_active_width[2]);
                                                if b > 12 {
                                                    if b > 99 {
                                                        fit_class = VERY_LOOSE_FIT;
                                                    } else {
                                                        fit_class = LOOSE_FIT;
                                                    }
                                                } else {
                                                    fit_class = DECENT_FIT;
                                                }
                                                break 'found;
                                            } else if g < 0 {
                                                // §1848
                                                if -g > self.cur_active_width[6] {
                                                    g = -self.cur_active_width[6];
                                                }
                                                b = crate::pack::pack_badness(-g, self.cur_active_width[6]);
                                                if b > 12 {
                                                    fit_class = TIGHT_FIT;
                                                } else {
                                                    fit_class = DECENT_FIT;
                                                }
                                                break 'found;
                                            }
                                        }
                                    }
                                    shortfall = 0;
                                }
                                b = 0;
                                fit_class = DECENT_FIT; // infinite stretch
                            } else if shortfall > 7230584 && self.cur_active_width[2] < 1663497 {
                                b = INF_BAD;
                                fit_class = VERY_LOOSE_FIT;
                            } else {
                                b = crate::pack::pack_badness(shortfall, self.cur_active_width[2]);
                                if b > 12 {
                                    if b > 99 {
                                        fit_class = VERY_LOOSE_FIT;
                                    } else {
                                        fit_class = LOOSE_FIT;
                                    }
                                } else {
                                    fit_class = DECENT_FIT;
                                }
                            }
                        } else {
                            // §1029
                            if -shortfall > self.cur_active_width[6] {
                                b = INF_BAD + 1;
                            } else {
                                b = crate::pack::pack_badness(-shortfall, self.cur_active_width[6]);
                            }
                            if b > 12 {
                                fit_class = TIGHT_FIT;
                            } else {
                                fit_class = DECENT_FIT;
                            }
                        }
                        if self.do_last_line_fit {
                            // §1849
                            if self.cur_p == NULL {
                                shortfall = 0;
                            }
                            if shortfall > 0 {
                                g = self.cur_active_width[2];
                            } else if shortfall < 0 {
                                g = self.cur_active_width[6];
                            } else {
                                g = 0;
                            }
                        }
                    }
                    // found:
                    if b > INF_BAD || pi == EJECT_PENALTY {
                        // §1030
                        if self.final_pass
                            && self.minimum_demerits == AWFUL_BAD as i64
                            && self.link(r) == LAST_ACTIVE
                            && prev_r == ACTIVE
                        {
                            artificial_demerits = true; // set demerits zero, this break is forced
                        } else if b > self.threshold {
                            break 'deactivate;
                        }
                        node_r_stays_active = false;
                    } else {
                        prev_r = r;
                        if b > self.threshold {
                            continue 'cont;
                        }
                        node_r_stays_active = true;
                    }
                    // §1031
                    let mut d: i64;
                    if artificial_demerits {
                        d = 0;
                    } else {
                        // §1035
                        d = self.line_penalty() as i64 + b as i64;
                        if d.abs() >= 10000 {
                            d = 100000000;
                        } else {
                            d = d * d;
                        }
                        if pi != 0 {
                            if pi > 0 {
                                d += (pi * pi) as i64;
                            } else if pi > EJECT_PENALTY {
                                d -= (pi * pi) as i64;
                            }
                        }
                        if break_type == HYPHENATED && self.ty(r) == HYPHENATED {
                            if self.cur_p != NULL {
                                d += self.double_hyphen_demerits() as i64;
                            } else {
                                d += self.final_hyphen_demerits() as i64;
                            }
                        }
                        if abs(fit_class - self.fitness(r)) > 1 {
                            d += self.adj_demerits() as i64;
                        }
                    }
                    if self.tracing_paragraphs() > 0 {
                        // §1032
                        if self.printed_node != self.cur_p {
                            // §1033
                            self.print_nl_s(b"");
                            if self.cur_p == NULL {
                                self.short_display(self.link(self.printed_node));
                            } else {
                                let save_link = self.link(self.cur_p);
                                self.set_link(self.cur_p, NULL);
                                self.print_nl_s(b"");
                                self.short_display(self.link(self.printed_node));
                                self.set_link(self.cur_p, save_link);
                            }
                            self.printed_node = self.cur_p;
                        }
                        self.print_nl(b'@' as i32);
                        if self.cur_p == NULL {
                            self.print_esc_s(b"par");
                        } else if self.ty(self.cur_p) != GLUE_NODE {
                            if self.ty(self.cur_p) == PENALTY_NODE {
                                self.print_esc_s(b"penalty");
                            } else if self.ty(self.cur_p) == DISC_NODE {
                                self.print_esc_s(b"discretionary");
                            } else if self.ty(self.cur_p) == KERN_NODE {
                                self.print_esc_s(b"kern");
                            } else {
                                self.print_esc_s(b"math");
                            }
                        }
                        self.print_s(b" via @@");
                        if self.break_node(r) == NULL {
                            self.print_char(b'0' as i32);
                        } else {
                            self.print_int(self.serial(self.break_node(r)));
                        }
                        self.print_s(b" b=");
                        if b > INF_BAD {
                            self.print_char(b'*' as i32);
                        } else {
                            self.print_int(b);
                        }
                        self.print_s(b" p=");
                        self.print_int(pi);
                        self.print_s(b" d=");
                        if artificial_demerits {
                            self.print_char(b'*' as i32);
                        } else {
                            self.print_int(d);
                        }
                    }
                    d += self.linebreak_total_demerits(r); // the minimum total demerits from the beginning to cur_p via r
                    if d <= self.minimal_demerits[fit_class] {
                        self.minimal_demerits[fit_class] = d;
                        self.best_place[fit_class] = self.break_node(r);
                        self.best_pl_line[fit_class] = l;
                        if self.do_last_line_fit {
                            // §1850
                            self.best_pl_short[fit_class] = shortfall;
                            self.best_pl_glue[fit_class] = g;
                        }
                        if d < self.minimum_demerits {
                            self.minimum_demerits = d;
                        }
                    }
                    if node_r_stays_active {
                        continue 'cont; // prev_r has been set to r
                    }
                }
                // deactivate:
                // §1036
                self.set_link(prev_r, self.link(r));
                self.free_node(r, self.active_node_size);
                if prev_r == ACTIVE {
                    // §1037
                    r = self.link(ACTIVE);
                    if self.ty(r) == DELTA_NODE {
                        do_all_eight!(self, |k| self.active_width[k] += self.linebreak_delta(r, k));
                        do_all_eight!(self, |k| self.cur_active_width[k] = self.active_width[k]);
                        self.set_link(ACTIVE, self.link(r));
                        self.free_node(r, DELTA_NODE_SIZE);
                    }
                } else if self.ty(prev_r) == DELTA_NODE {
                    r = self.link(prev_r);
                    if r == LAST_ACTIVE {
                        do_all_eight!(self, |k| self.cur_active_width[k] -= self.linebreak_delta(prev_r, k));
                        self.set_link(prev_prev_r, LAST_ACTIVE);
                        self.free_node(prev_r, DELTA_NODE_SIZE);
                        prev_r = prev_prev_r;
                    } else if self.ty(r) == DELTA_NODE {
                        do_all_eight!(self, |k| self.cur_active_width[k] += self.linebreak_delta(r, k));
                        let mut v = [0i64; 9];
                        do_all_eight!(self, |k| v[k as usize] = self.linebreak_delta(prev_r, k) + self.linebreak_delta(r, k));
                        self.linebreak_set_delta(prev_r, &v);
                        self.set_link(prev_r, self.link(r));
                        self.free_node(r, DELTA_NODE_SIZE);
                    }
                }
            }
        }
        // exit:
        // §1034
        if self.cur_p == self.printed_node && self.cur_p != NULL && self.ty(self.cur_p) == DISC_NODE {
            let mut t = self.replace_count(self.cur_p);
            while t > 0 {
                t -= 1;
                self.printed_node = self.link(self.printed_node);
            }
        }
    }

    /// §1708.  `LR_ptr` is a local of `post_line_break`, not the global, so this cannot be `pop_LR` and `push_LR`.
    fn linebreak_adjust_LR(&mut self, q: i32, LR_ptr: i32) -> i32 {
        let mut LR_ptr = LR_ptr;
        if self.end_LR(q) {
            if LR_ptr != NULL && self.info(LR_ptr) == self.end_LR_type(q) {
                // pop_LR
                self.temp_ptr = LR_ptr;
                LR_ptr = self.link(self.temp_ptr);
                self.free_avail(self.temp_ptr);
            }
        } else {
            // push_LR(q)
            self.temp_ptr = self.get_avail();
            self.set_info(self.temp_ptr, self.end_LR_type(q));
            self.set_link(self.temp_ptr, LR_ptr);
            LR_ptr = self.temp_ptr;
        }
        LR_ptr
    }

    /// §1053
    pub fn post_line_break(&mut self, d: bool) {
        let mut q: i32;
        let mut r: i32;
        let mut s: i32;
        let mut LR_ptr = self.LR_save();
        // §1054
        q = self.break_node(self.best_bet);
        self.cur_p = NULL;
        loop {
            r = q;
            q = self.prev_break(q);
            self.set_next_break(r, self.cur_p);
            self.cur_p = r;
            if q == NULL {
                break;
            }
        }
        let mut cur_line = self.prev_graf() + 1;
        loop {
            // §1056
            if self.TeXXeT_en() {
                // §1707
                q = self.link(TEMP_HEAD);
                if LR_ptr != NULL {
                    self.temp_ptr = LR_ptr;
                    r = q;
                    loop {
                        s = self.new_math(0, self.begin_LR_type(self.info(self.temp_ptr)));
                        self.set_link(s, r);
                        r = s;
                        self.temp_ptr = self.link(self.temp_ptr);
                        if self.temp_ptr == NULL {
                            break;
                        }
                    }
                    self.set_link(TEMP_HEAD, r);
                }
                while q != self.cur_break(self.cur_p) {
                    if !self.is_char_node(q) && self.ty(q) == MATH_NODE {
                        // §1708
                        LR_ptr = self.linebreak_adjust_LR(q, LR_ptr);
                    }
                    q = self.link(q);
                }
            }

            // §1057
            q = self.cur_break(self.cur_p);
            let mut disc_break = false;
            let mut post_disc_break = false;
            let mut glue_break = false;
            if q != NULL {
                // q cannot be a char_node
                if self.ty(q) == GLUE_NODE {
                    self.delete_glue_ref(self.glue_ptr(q));
                    self.set_glue_ptr(q, self.right_skip());
                    self.set_subtype(q, RIGHT_SKIP_CODE + 1);
                    self.add_glue_ref(self.right_skip());
                    glue_break = true;
                    // goto done, which is the end of this `if`
                } else if self.ty(q) == DISC_NODE {
                    // §1058
                    let mut t = self.replace_count(q);
                    // §1059
                    if t == 0 {
                        r = self.link(q);
                    } else {
                        r = q;
                        while t > 1 {
                            r = self.link(r);
                            t -= 1;
                        }
                        s = self.link(r);
                        r = self.link(s);
                        self.set_link(s, NULL);
                        self.flush_node_list(self.link(q));
                        self.set_replace_count(q, 0);
                    }
                    if self.post_break(q) != NULL {
                        // §1060
                        s = self.post_break(q);
                        while self.link(s) != NULL {
                            s = self.link(s);
                        }
                        self.set_link(s, r);
                        r = self.post_break(q);
                        self.set_post_break(q, NULL);
                        post_disc_break = true;
                    }
                    if self.pre_break(q) != NULL {
                        // §1061
                        s = self.pre_break(q);
                        self.set_link(q, s);
                        while self.link(s) != NULL {
                            s = self.link(s);
                        }
                        self.set_pre_break(q, NULL);
                        q = s;
                    }
                    self.set_link(q, r);
                    disc_break = true;
                } else if self.ty(q) == KERN_NODE {
                    self.set_width(q, 0);
                } else if self.ty(q) == MATH_NODE {
                    self.set_width(q, 0);
                    if self.TeXXeT_en() {
                        // §1708
                        LR_ptr = self.linebreak_adjust_LR(q, LR_ptr);
                    }
                }
            } else {
                q = TEMP_HEAD;
                while self.link(q) != NULL {
                    q = self.link(q);
                }
            }
            // done:
            // at this point q is the rightmost breakpoint; the only exception is the case of a discretionary break with
            // non-empty pre_break, then q has been changed to the last node of the pre_break list
            if self.pdf_protrude_chars() > 0 {
                let mut p;
                let ptmp;
                if disc_break && (self.is_char_node(q) || self.ty(q) != DISC_NODE) {
                    // q has been reset to the last node of pre_break
                    p = q;
                    ptmp = p;
                } else {
                    p = self.prev_rightmost(self.link(TEMP_HEAD), q); // get link(p) = q
                    ptmp = p;
                    p = self.find_protchar_right(self.link(TEMP_HEAD), p);
                }
                let w = self.right_pw(p);
                if w != 0 {
                    // we have found a marginal kern, append it after ptmp
                    let k = self.new_margin_kern(-w, self.last_rightmost_char, RIGHT_SIDE);
                    self.set_link(k, self.link(ptmp));
                    self.set_link(ptmp, k);
                    if ptmp == q {
                        q = self.link(q);
                    }
                }
            }
            // if q was not a breakpoint at glue and has been reset to rightskip then we append rightskip after q now
            if !glue_break {
                // §1062
                r = self.new_param_glue(RIGHT_SKIP_CODE);
                self.set_link(r, self.link(q));
                self.set_link(q, r);
                q = r;
            }
            if self.TeXXeT_en() {
                // §1709
                if LR_ptr != NULL {
                    s = TEMP_HEAD;
                    r = self.link(s);
                    while r != q {
                        s = r;
                        r = self.link(s);
                    }
                    r = LR_ptr;
                    while r != NULL {
                        self.temp_ptr = self.new_math(0, self.info(r));
                        self.set_link(s, self.temp_ptr);
                        s = self.temp_ptr;
                        r = self.link(r);
                    }
                    self.set_link(s, q);
                }
            }

            // §1063
            r = self.link(q);
            self.set_link(q, NULL);
            q = self.link(TEMP_HEAD);
            self.set_link(TEMP_HEAD, r);
            // at this point q is the leftmost node; all discardable nodes have been discarded
            if self.pdf_protrude_chars() > 0 {
                let mut p = q;
                p = self.find_protchar_left(p, false); // no more discardables
                let w = self.left_pw(p);
                if w != 0 {
                    let k = self.new_margin_kern(-w, self.last_leftmost_char, LEFT_SIDE);
                    self.set_link(k, q);
                    q = k;
                }
            }
            if self.left_skip() != ZERO_GLUE {
                r = self.new_param_glue(LEFT_SKIP_CODE);
                self.set_link(r, q);
                q = r;
            }

            // §1066
            let cur_width;
            let cur_indent;
            if cur_line > self.last_special_line {
                cur_width = self.second_width;
                cur_indent = self.second_indent;
            } else if self.par_shape_ptr() == NULL {
                cur_width = self.first_width;
                cur_indent = self.first_indent;
            } else {
                cur_width = self.mem[self.par_shape_ptr() + 2 * cur_line].sc();
                cur_indent = self.mem[self.par_shape_ptr() + 2 * cur_line - 1].sc();
            }
            self.adjust_tail = ADJUST_HEAD;
            self.pre_adjust_tail = PRE_ADJUST_HEAD;
            if self.pdf_adjust_spacing() > 0 {
                self.just_box = self.hpack(q, cur_width, CAL_EXPAND_RATIO);
            } else {
                self.just_box = self.hpack(q, cur_width, EXACTLY);
            }
            self.set_shift_amount(self.just_box, cur_indent);

            // §1065
            if self.pdf_each_line_height() != self.pdf_ignored_dimen() {
                self.set_height(self.just_box, self.pdf_each_line_height());
            }
            if self.pdf_each_line_depth() != self.pdf_ignored_dimen() {
                self.set_depth(self.just_box, self.pdf_each_line_depth());
            }
            if self.pdf_first_line_height() != self.pdf_ignored_dimen() && cur_line == self.prev_graf() + 1 {
                self.set_height(self.just_box, self.pdf_first_line_height());
            }
            if self.pdf_last_line_depth() != self.pdf_ignored_dimen() && cur_line + 1 == self.best_line {
                self.set_depth(self.just_box, self.pdf_last_line_depth());
            }
            if PRE_ADJUST_HEAD != self.pre_adjust_tail {
                self.append_list(PRE_ADJUST_HEAD, self.pre_adjust_tail);
            }
            self.pre_adjust_tail = NULL;
            self.append_to_vlist(self.just_box);
            if ADJUST_HEAD != self.adjust_tail {
                self.append_list(ADJUST_HEAD, self.adjust_tail);
            }
            self.adjust_tail = NULL;

            // §1067
            if cur_line + 1 != self.best_line {
                // pen is an i64 local: four penalties of 2^31-1 add up on the oracle, section 11
                let mut pen: i64;
                q = self.inter_line_penalties_ptr();
                if q != NULL {
                    r = cur_line;
                    if r > self.penalty(q) {
                        r = self.penalty(q);
                    }
                    pen = self.penalty(q + r) as i64;
                } else {
                    pen = self.inter_line_penalty() as i64;
                }
                q = self.club_penalties_ptr();
                if q != NULL {
                    r = cur_line - self.prev_graf();
                    if r > self.penalty(q) {
                        r = self.penalty(q);
                    }
                    pen += self.penalty(q + r) as i64;
                } else if cur_line == self.prev_graf() + 1 {
                    pen += self.club_penalty() as i64;
                }
                if d {
                    q = self.display_widow_penalties_ptr();
                } else {
                    q = self.widow_penalties_ptr();
                }
                if q != NULL {
                    r = self.best_line - cur_line - 1;
                    if r > self.penalty(q) {
                        r = self.penalty(q);
                    }
                    pen += self.penalty(q + r) as i64;
                } else if cur_line + 2 == self.best_line {
                    if d {
                        pen += self.display_widow_penalty() as i64;
                    } else {
                        pen += self.widow_penalty() as i64;
                    }
                }
                if disc_break {
                    pen += self.broken_penalty() as i64;
                }
                if pen != 0 {
                    r = self.new_penalty(sat32(pen));
                    self.set_link(self.tail(), r);
                    self.set_tail(r);
                }
            }

            cur_line += 1;
            self.cur_p = self.next_break(self.cur_p);
            if self.cur_p != NULL && !post_disc_break {
                // §1055
                r = TEMP_HEAD;
                loop {
                    q = self.link(r);
                    if q == self.cur_break(self.cur_p) {
                        break; // cur_break(cur_p) is the next breakpoint
                    }
                    // now q cannot be null
                    if self.is_char_node(q) {
                        break;
                    }
                    if self.non_discardable(q) {
                        break;
                    }
                    if self.ty(q) == KERN_NODE && self.subtype(q) != EXPLICIT {
                        break;
                    }
                    r = q; // now type(q)=glue_node, kern_node, math_node, or penalty_node
                    if self.ty(q) == MATH_NODE && self.TeXXeT_en() {
                        // §1708
                        LR_ptr = self.linebreak_adjust_LR(q, LR_ptr);
                    }
                }
                if r != TEMP_HEAD {
                    self.set_link(r, NULL);
                    self.flush_node_list(self.link(TEMP_HEAD));
                    self.set_link(TEMP_HEAD, q);
                }
            }
            if self.cur_p == NULL {
                break;
            }
        }
        if cur_line != self.best_line || self.link(TEMP_HEAD) != NULL {
            self.confusion(b"line breaking");
        }
        self.set_prev_graf(self.best_line - 1);
        self.set_LR_save(LR_ptr);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::show::showkit::{XX, case_machine, pt};
    use crate::toks::testkit::printed;

    fn scaled(s: &str) -> i32 {
        pt(s.parse::<f64>().unwrap())
    }

    /// `5`, or `1fil` and its kin: an amount and its order.
    fn part(s: &str) -> (i32, i32) {
        for (end, o) in [("filll", FILLL), ("fill", FILL), ("fil", FIL)] {
            if let Some(x) = s.strip_suffix(end) {
                return (scaled(x), o);
            }
        }
        (scaled(s), NORMAL)
    }

    /// `w,st,sh`, as `\hskip` scans it.
    fn spec(t: &mut Tex, s: &str) -> i32 {
        let f: Vec<&str> = s.split(',').collect();
        let q = t.new_spec(ZERO_GLUE);
        t.set_width(q, scaled(f[0]));
        let (st, so) = part(f[1]);
        t.set_stretch(q, st);
        t.set_stretch_order(q, so);
        let (sh, sho) = part(f[2]);
        t.set_shrink(q, sh);
        t.set_shrink_order(q, sho);
        q
    }

    /// `append_glue`: the spec has only this one reference.
    fn glue(t: &mut Tex, s: &str) -> i32 {
        let q = spec(t, s);
        let g = t.new_glue(q);
        t.set_glue_ref_count(q, NULL);
        g
    }

    fn chain(t: &mut Tex, v: &[i32]) -> i32 {
        for w in v.windows(2) {
            t.set_link(w[0], w[1]);
        }
        v.first().copied().unwrap_or(NULL)
    }

    /// The nodes of one item of gen_linebreak_tables.py's language.  `math` is whether a formula is open.
    fn item(t: &mut Tex, x: &str, surround: i32, math: &mut bool, out: &mut Vec<i32>) {
        let (k, v) = x.split_at(1);
        match k {
            "b" => {
                let p = t.new_null_box();
                t.set_width(p, scaled(v));
                out.push(p);
            }
            "v" => {
                let p = t.new_null_box();
                t.set_ty(p, VLIST_NODE);
                t.set_width(p, scaled(v));
                out.push(p);
            }
            "r" => {
                let p = t.new_rule();
                t.set_width(p, scaled(v));
                out.push(p);
            }
            "k" => {
                let p = t.new_kern(scaled(v));
                t.set_subtype(p, EXPLICIT);
                out.push(p);
            }
            "g" => out.push(glue(t, v)),
            "l" => {
                let p = glue(t, &format!("{v},0,0"));
                t.set_subtype(p, A_LEADERS);
                let r = t.new_rule();
                t.set_leader_ptr(p, r);
                out.push(p);
            }
            "p" => {
                let p = t.new_penalty(v.parse().unwrap());
                out.push(p);
            }
            "$" => {
                let p = t.new_math(surround, if *math { AFTER } else { BEFORE });
                *math = !*math;
                out.push(p);
            }
            "L" => out.push(t.new_math(0, BEGIN_L_CODE)),
            "E" => out.push(t.new_math(0, END_L_CODE)),
            "c" => {
                for c in v.bytes() {
                    let p = t.get_avail();
                    t.set_font(p, XX);
                    t.set_character(p, c as i32);
                    out.push(p);
                }
            }
            "x" | "a" | "w" => {
                let p = t.get_node(SMALL_NODE_SIZE);
                t.set_ty(p, if k == "x" { MARK_NODE } else if k == "a" { ADJUST_NODE } else { WHATSIT_NODE });
                t.set_subtype(p, if k == "w" { SPECIAL_NODE } else { 0 });
                out.push(p);
            }
            "d" => {
                let parts: Vec<&str> = v[1..v.len() - 1].split('|').collect();
                let mut lists: Vec<Vec<i32>> = Vec::new();
                for s in &parts {
                    let mut l = Vec::new();
                    for y in s.split(',').filter(|y| !y.is_empty()) {
                        item(t, y, surround, math, &mut l);
                    }
                    lists.push(l);
                }
                let p = t.new_disc();
                let a = chain(t, &lists[0]);
                t.set_pre_break(p, a);
                let b = chain(t, &lists[1]);
                t.set_post_break(p, b);
                t.set_replace_count(p, lists[2].len() as i32);
                out.push(p);
                out.extend_from_slice(&lists[2]);
            }
            _ => panic!("no item {x}"),
        }
    }

    /// The machine for the search: a font without a TFM, whose characters all have one width.
    fn search_machine(char_width: i32) -> &'static mut Tex {
        let t = case_machine("", true);
        t.nest_size = 100;
        t.job_name = S_TEXPUT; // normalize_selector would open the log
        // What initialize does to last_active
        t.set_ty(LAST_ACTIVE, HYPHENATED);
        t.set_line_number(LAST_ACTIVE, MAX_HALFWORD);
        t.set_subtype(LAST_ACTIVE, 0);
        // What INITEX sets
        t.set_tolerance(10000);
        t.set_hang_after(1);
        for k in 0..26 {
            t.set_lc_code(b'a' as i32 + k, b'a' as i32 + k);
            t.set_lc_code(b'A' as i32 + k, b'a' as i32 + k);
        }
        t.set_tracing_paragraphs(1);
        t.char_base[XX] = 1000;
        t.width_base[XX] = 2000;
        for c in 0..256 {
            t.font_info[1000 + c].set_qqqq(FourQuarters { b0: 1, b1: 0, b2: 0, b3: 0 });
        }
        t.font_info[2001].set_sc(char_width);
        t.hyphen_char[XX] = -1;
        t
    }

    /// Everything of `line_break` before `post_line_break`, for a paragraph of the table.  Returns `var_used` from just
    /// before the search.
    fn search(t: &mut Tex, params: &str, para: &str) -> i32 {
        let mut surround = 0;
        for p in params.split_whitespace() {
            let (k, v) = p.split_once('=').unwrap();
            match k {
                "tolerance" => t.set_tolerance(v.parse().unwrap()),
                "pretolerance" => t.set_pretolerance(v.parse().unwrap()),
                "looseness" => t.set_looseness(v.parse().unwrap()),
                "hangafter" => t.set_hang_after(v.parse().unwrap()),
                "linepenalty" => t.set_line_penalty(v.parse().unwrap()),
                "adjdemerits" => t.set_adj_demerits(v.parse().unwrap()),
                "doublehyphendemerits" => t.set_double_hyphen_demerits(v.parse().unwrap()),
                "finalhyphendemerits" => t.set_final_hyphen_demerits(v.parse().unwrap()),
                "hyphenpenalty" => t.set_hyphen_penalty(v.parse().unwrap()),
                "exhyphenpenalty" => t.set_ex_hyphen_penalty(v.parse().unwrap()),
                "prevgraf" => t.set_prev_graf(v.parse().unwrap()),
                "hyphenchar" => t.hyphen_char[XX] = v.parse().unwrap(),
                "uchyph" => t.set_uc_hyph(v.parse().unwrap()),
                "lefthyphenmin" => t.set_left_hyphen_min(v.parse().unwrap()),
                "righthyphenmin" => t.set_right_hyphen_min(v.parse().unwrap()),
                "hsize" => t.set_hsize(scaled(v)),
                "hangindent" => t.set_hang_indent(scaled(v)),
                "emergencystretch" => t.set_emergency_stretch(scaled(v)),
                "mathsurround" => surround = scaled(v),
                "leftskip" => {
                    let s = spec(t, v);
                    t.set_left_skip(s);
                }
                "rightskip" => {
                    let s = spec(t, v);
                    t.set_right_skip(s);
                }
                "parfillskip" => {
                    let s = spec(t, v);
                    t.set_par_fill_skip(s);
                }
                "parshape" => {
                    let rows: Vec<&str> = v.split(';').collect();
                    let q = t.get_node(2 * rows.len() as i32 + 1);
                    t.set_info(q, rows.len() as i32);
                    for (j, r) in rows.iter().enumerate() {
                        let (i, w) = r.split_once(',').unwrap();
                        t.mem[q + 2 * j as i32 + 1].set_sc(scaled(i));
                        t.mem[q + 2 * j as i32 + 2].set_sc(scaled(w));
                    }
                    t.set_par_shape_ptr(q);
                }
                _ => panic!("no parameter {k}"),
            }
        }
        // new_graf
        let norm_min = |h: i32| h.clamp(1, 63);
        let (l, r) = (norm_min(t.left_hyphen_min()), norm_min(t.right_hyphen_min()));
        t.push_nest();
        t.set_mode(HMODE);
        t.set_prev_graf((l * 0o100 + r) * 0o200000);
        let mut math = false;
        for x in para.split(' ') {
            let mut v = Vec::new();
            item(t, x, surround, &mut math, &mut v);
            for p in v {
                t.tail_append(p);
            }
        }
        t.pack_begin_line = t.mode_line();
        t.linebreak_prepare();
        let used = t.var_used;
        t.linebreak_find_breaks();
        used
    }

    fn table() -> (i32, Vec<Vec<&'static str>>) {
        let text = core::str::from_utf8(include_bytes!("../testdata/linebreak_oracle.tsv")).unwrap();
        let mut rows: Vec<Vec<&str>> = text.lines().map(|l| l.split('\t').collect()).collect();
        let first = rows.remove(0);
        assert_eq!(first[0], "#char_width");
        (first[1].parse().unwrap(), rows)
    }

    #[test]
    fn the_search_prints_what_the_oracle_prints() {
        let (w, rows) = table();
        assert!(rows.len() >= 50);
        let mut bad = Vec::new();
        for row in &rows {
            let t = search_machine(w);
            let used = search(t, row[1], row[2]);
            let got = printed(t).trim_end_matches('\n').replace('\n', "\u{c}");
            let lines: i32 = row[3].parse().unwrap();
            if got != row[4] {
                let (a, b): (Vec<&str>, Vec<&str>) = (row[4].split('\u{c}').collect(), got.split('\u{c}').collect());
                let k = a.iter().zip(&b).position(|(x, y)| x != y).unwrap_or(a.len().min(b.len()));
                let from = k.saturating_sub(4);
                bad.push(format!(
                    "--- {}: line {k} of {}\nwant {:#?}\ngot  {:#?}",
                    row[0],
                    a.len(),
                    &a[from..(k + 3).min(a.len())],
                    &b[from..(k + 3).min(b.len())]
                ));
            } else if t.best_line - 1 != lines {
                bad.push(format!("--- {}: {} lines where the oracle made {}", row[0], t.best_line - 1, lines));
            }
            // The clean-up gives back all the search took
            t.linebreak_clean_up();
            assert_eq!(t.var_used, used, "{}", row[0]);
            Tex::dealloc(t);
        }
        assert!(bad.is_empty(), "{} of {} differ:\n{}", bad.len(), rows.len(), bad.join("\n"));
    }

    #[test]
    fn infinite_shrinkage_is_made_finite_once() {
        let t = search_machine(0);
        t.interaction = BATCH_MODE;
        t.no_shrink_error_yet = true;
        let p = spec(t, "1,2,3fil");
        let q = t.finite_shrink(p);
        assert!(!t.no_shrink_error_yet);
        assert_eq!(t.history, ERROR_MESSAGE_ISSUED);
        assert_eq!((t.width(q), t.stretch(q), t.shrink(q)), (pt(1.0), pt(2.0), pt(3.0)));
        assert_eq!(t.shrink_order(q), NORMAL);
        let errors = t.error_count;
        let g = t.new_spec(q);
        t.set_shrink_order(g, FILLL);
        let r = t.finite_shrink(g);
        assert_eq!(t.shrink_order(r), NORMAL);
        assert_eq!(t.error_count, errors, "only the first one complains");
        Tex::dealloc(t);
    }

    #[test]
    fn the_hlist_stack_gives_back_what_it_was_given() {
        let t = search_machine(0);
        t.hlist_stack_level = 0;
        t.push_node(5);
        t.push_node(7);
        assert_eq!(t.pop_node(), 7);
        assert_eq!(t.pop_node(), 5);
        assert_eq!(t.hlist_stack_level, 0);
        Tex::dealloc(t);
    }

    /// Only `find_protchar_right` calls `prev_rightmost`.
    #[test]
    fn find_protchar_left_skips_what_cannot_protrude() {
        let t = search_machine(0);
        let mut math = false;
        let mut nodes = Vec::new();
        for x in ["p0", "k0", "x", "ca", "b5"] {
            item(t, x, 0, &mut math, &mut nodes);
        }
        let l = chain(t, &nodes);
        assert_eq!(t.find_protchar_left(l, true), nodes[3], "the first character");
        assert_eq!(t.find_protchar_left(l, false), nodes[3], "with no discardables to skip first");

        // A character inside two boxes, each behind an empty box that may be skipped
        let mut inner = Vec::new();
        for x in ["x", "cb"] {
            item(t, x, 0, &mut math, &mut inner);
        }
        let i = chain(t, &inner);
        let b1 = t.new_null_box();
        t.set_list_ptr(b1, i);
        let b2 = t.new_null_box();
        t.set_list_ptr(b2, b1);
        assert_eq!(t.find_protchar_left(b2, false), inner[1]);
        assert_eq!(t.hlist_stack_level, 2);

        // A paragraph starting with \parindent=0pt: the empty box does not count
        let e = t.new_null_box();
        t.set_link(e, nodes[3]);
        assert_eq!(t.find_protchar_left(e, false), nodes[3]);

        // A list with nothing but skippable nodes gives up at the last one
        let mut rest = Vec::new();
        for x in ["x", "p0"] {
            item(t, x, 0, &mut math, &mut rest);
        }
        let r = chain(t, &rest);
        assert_eq!(t.find_protchar_left(r, false), rest[1]);

        // Out of a box with only skippable nodes, and on to what follows it
        let mut within = Vec::new();
        for x in ["x", "p0"] {
            item(t, x, 0, &mut math, &mut within);
        }
        let w = chain(t, &within);
        let b3 = t.new_null_box();
        t.set_list_ptr(b3, w);
        let mut after = Vec::new();
        item(t, "cc", 0, &mut math, &mut after);
        t.set_link(b3, after[0]);
        assert_eq!(t.find_protchar_left(b3, false), after[0]);
        assert_eq!(t.hlist_stack_level, 0);
        Tex::dealloc(t);
    }
}
