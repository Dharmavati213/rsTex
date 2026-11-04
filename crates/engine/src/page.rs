//! §1144-1205 Parts 44-45: breaking vertical lists into pages, the page builder.
//!
//! The oracle's `integer` is 64 bits, so its page totals never wrap.  Three `\maxdimen` rules in a row
//! with no breakpoint between them pass 2^31 in `page_total` and in `cur_height`, and a wrapped sum
//! would turn negative and move the break.  `active_height` is `active_width`, which is `i64`, so
//! `vert_break` keeps the oracle's values, with `cur_height` written out as `active_width[1]`.  So are `page_so_far`, `insert_penalties`
//! and `least_page_cost`, which can come back from past 2^31 and choose the break, with `\pagetotal` and the like clamped where they are read.
//! Every other running sum here is in a node's word, so it is computed in `i64` and stored with `sat32`, per the
//! rule of docs/PORTING.md section 11, not only the products its table names.
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

impl Tex {
    /// §1145
    pub fn prune_page_top(&mut self, p: i32, s: bool) -> i32 {
        let mut p = p;
        let mut prev_p = TEMP_HEAD; // lags one step behind p
        let mut r = 0; // set before use: split_disc is null when s is true, §1154
        self.set_link(TEMP_HEAD, p);
        while p != NULL {
            let discard;
            match self.ty(p) {
                HLIST_NODE | VLIST_NODE | RULE_NODE => {
                    // §1146 Insert glue for split_top_skip and set p:=null
                    let q = self.new_skip_param(SPLIT_TOP_SKIP_CODE);
                    self.set_link(prev_p, q);
                    self.set_link(q, p);
                    // now temp_ptr=glue_ptr(q)
                    if self.width(self.temp_ptr) > self.height(p) {
                        self.set_width(
                            self.temp_ptr,
                            sat32(self.width(self.temp_ptr) as i64 - self.height(p) as i64),
                        );
                    } else {
                        self.set_width(self.temp_ptr, 0);
                    }
                    p = NULL;
                    discard = false;
                }
                WHATSIT_NODE | MARK_NODE | INS_NODE => {
                    if self.ty(p) == WHATSIT_NODE
                        && (self.subtype(p) == PDF_SNAPY_NODE
                            || self.subtype(p) == PDF_SNAPY_COMP_NODE)
                    {
                        self.print_s(b"snap node being discarded");
                        discard = true; // goto discard_or_move
                    } else {
                        prev_p = p;
                        p = self.link(prev_p);
                        discard = false;
                    }
                }
                GLUE_NODE | KERN_NODE | PENALTY_NODE => discard = true,
                _ => self.confusion(b"pruning"),
            }
            if discard {
                // discard_or_move:
                let q = p;
                p = self.link(q);
                self.set_link(q, NULL);
                self.set_link(prev_p, p);
                if s {
                    if self.split_disc() == NULL {
                        self.set_split_disc(q);
                    } else {
                        self.set_link(r, q);
                    }
                    r = q;
                } else {
                    self.flush_node_list(q);
                }
            }
        }
        self.link(TEMP_HEAD)
    }

    /// §1147
    pub fn vert_break(&mut self, p: i32, h: i32, d: i32) -> i32 {
        let mut p = p;
        let mut prev_p = p; // an initial glue node is not a legal breakpoint
        let mut q: i32;
        let mut least_cost = AWFUL_BAD;
        let mut best_place = NULL;
        let mut prev_dp = 0;
        self.active_width[1] = 0;
        self.active_width[2] = 0;
        self.active_width[3] = 0;
        self.active_width[4] = 0;
        self.active_width[5] = 0;
        self.active_width[6] = 0;
        loop {
            // §1149 If node p is a legal breakpoint, check if this break is the best known,
            // and goto done if p is null or if the page-so-far is already too full to accept more stuff
            'not_found: {
                'update_heights: {
                    let pi;
                    if p == NULL {
                        pi = EJECT_PENALTY;
                    } else {
                        // §1150 Use node p to update the current height and depth measurements;
                        // if this node is not a legal breakpoint, goto not_found or update_heights,
                        // otherwise set pi to the associated penalty at the break
                        match self.ty(p) {
                            HLIST_NODE | VLIST_NODE | RULE_NODE => {
                                self.active_width[1] += prev_dp as i64 + self.height(p) as i64;
                                prev_dp = self.depth(p);
                                break 'not_found;
                            }
                            WHATSIT_NODE => {
                                // §1612 Process whatsit p in vert_break loop, goto not_found
                                if self.subtype(p) == PDF_REFXFORM_NODE
                                    || self.subtype(p) == PDF_REFXIMAGE_NODE
                                {
                                    self.active_width[1] += prev_dp as i64 + self.pdf_height(p) as i64;
                                    prev_dp = self.pdf_depth(p);
                                }
                                break 'not_found;
                            }
                            GLUE_NODE => {
                                if self.precedes_break(prev_p) {
                                    pi = 0;
                                } else {
                                    break 'update_heights;
                                }
                            }
                            KERN_NODE => {
                                let t = if self.link(p) == NULL {
                                    PENALTY_NODE
                                } else {
                                    self.ty(self.link(p))
                                };
                                if t == GLUE_NODE {
                                    pi = 0;
                                } else {
                                    break 'update_heights;
                                }
                            }
                            PENALTY_NODE => pi = self.penalty(p),
                            MARK_NODE | INS_NODE => break 'not_found,
                            _ => self.confusion(b"vertbreak"),
                        }
                    }
                    // §1151 Check if node p is a new champion breakpoint; then goto done
                    // if p is a forced break or if the page-so-far is already too full
                    if pi < INF_PENALTY {
                        // §1152 Compute the badness, b, using awful_bad if the box is too full
                        let mut b;
                        if self.active_width[1] < h as i64 {
                            if self.active_width[3] != 0
                                || self.active_width[4] != 0
                                || self.active_width[5] != 0
                            {
                                b = 0;
                            } else {
                                b = crate::pack::pack_badness(h as i64 - self.active_width[1], self.active_width[2]);
                            }
                        } else if self.active_width[1] - h as i64 > self.active_width[6] {
                            b = AWFUL_BAD;
                        } else {
                            b = crate::pack::pack_badness(self.active_width[1] - h as i64, self.active_width[6]);
                        }
                        if b < AWFUL_BAD {
                            if pi <= EJECT_PENALTY {
                                b = pi;
                            } else if b < INF_BAD {
                                b += pi;
                            } else {
                                b = DEPLORABLE;
                            }
                        }
                        if b <= least_cost {
                            best_place = p;
                            least_cost = b;
                            self.best_height_plus_depth = sat32(self.active_width[1] + prev_dp as i64);
                        }
                        if b == AWFUL_BAD || pi <= EJECT_PENALTY {
                            return best_place; // done:
                        }
                    }
                    if self.ty(p) < GLUE_NODE || self.ty(p) > KERN_NODE {
                        break 'not_found;
                    }
                }
                // update_heights:
                // §1153 Update the current height and depth measurements with respect to a glue or kern node p
                if self.ty(p) == KERN_NODE {
                    q = p;
                } else {
                    q = self.glue_ptr(p);
                    let o = 2 + self.stretch_order(q);
                    self.active_width[o] += self.stretch(q) as i64;
                    self.active_width[6] += self.shrink(q) as i64;
                    if self.shrink_order(q) != NORMAL && self.shrink(q) != 0 {
                        let e = self.ignore_primitive_error();
                        if self.is_bit_set(e, 1) {
                            self.print_ignored_err_s(
                                b"Infinite glue shrinkage found in box being split",
                            );
                        } else {
                            self.print_err_s(b"Infinite glue shrinkage found in box being split");
                            self.help(&[
                                b"The box you are \\vsplitting contains some infinitely",
                                b"shrinkable glue, e.g., `\\vss' or `\\vskip 0pt minus 1fil'.",
                                b"Such glue doesn't belong there; but you can safely proceed,",
                                b"since the offensive shrinkability has been made finite.",
                            ]);
                            self.error();
                        }
                        let r = self.new_spec(q);
                        self.set_shrink_order(r, NORMAL);
                        self.delete_glue_ref(q);
                        self.set_glue_ptr(p, r);
                        q = r;
                    }
                }
                self.active_width[1] += prev_dp as i64 + self.width(q) as i64;
                prev_dp = 0;
            }
            // not_found:
            if prev_dp > d {
                self.active_width[1] += prev_dp as i64 - d as i64;
                prev_dp = d;
            }
            prev_p = p;
            p = self.link(prev_p);
        }
    }

    /// §1154
    pub fn vsplit(&mut self, n: i32, h: i32) -> i32 {
        self.cur_val = n;
        let v = self.fetch_box(); // the box to be split
        self.flush_node_list(self.split_disc());
        self.set_split_disc(NULL);
        if self.sa_mark() != NULL {
            if self.do_marks(VSPLIT_INIT, 0, self.sa_mark()) {
                self.set_sa_mark(NULL);
            }
        }
        if self.split_first_mark() != NULL {
            self.delete_token_ref(self.split_first_mark());
            self.set_split_first_mark(NULL);
            self.delete_token_ref(self.split_bot_mark());
            self.set_split_bot_mark(NULL);
        }
        // §1155 Dispense with trivial cases of void or bad boxes
        if v == NULL {
            return NULL;
        }
        if self.ty(v) != VLIST_NODE {
            self.print_err_s(b"");
            self.print_esc_s(b"vsplit");
            self.print_s(b" needs a ");
            self.print_esc_s(b"vbox");
            self.help(&[
                b"The box you are trying to split is an \\hbox.",
                b"I can't split such a box, so I'll leave it alone.",
            ]);
            self.error();
            return NULL;
        }
        let mut q = self.vert_break(self.list_ptr(v), h, self.split_max_depth());
        // §1156 Look at all the marks in nodes before the break, and set the final link to null at the break
        let mut p = self.list_ptr(v);
        if p == q {
            self.set_list_ptr(v, NULL);
        } else {
            loop {
                if self.ty(p) == MARK_NODE {
                    if self.mark_class(p) != 0 {
                        // §1827 Update the current marks for vsplit
                        self.find_sa_element(MARK_VAL, self.mark_class(p), true);
                        if self.sa_split_first_mark(self.cur_ptr) == NULL {
                            self.set_sa_split_first_mark(self.cur_ptr, self.mark_ptr(p));
                            self.add_token_ref(self.mark_ptr(p));
                        } else {
                            self.delete_token_ref(self.sa_split_bot_mark(self.cur_ptr));
                        }
                        self.set_sa_split_bot_mark(self.cur_ptr, self.mark_ptr(p));
                        self.add_token_ref(self.mark_ptr(p));
                    } else if self.split_first_mark() == NULL {
                        self.set_split_first_mark(self.mark_ptr(p));
                        self.set_split_bot_mark(self.split_first_mark());
                        let m = self.split_first_mark();
                        self.set_token_ref_count(m, self.token_ref_count(m) + 2);
                    } else {
                        self.delete_token_ref(self.split_bot_mark());
                        self.set_split_bot_mark(self.mark_ptr(p));
                        self.add_token_ref(self.split_bot_mark());
                    }
                }
                if self.link(p) == q {
                    self.set_link(p, NULL);
                    break;
                }
                p = self.link(p);
            }
        }
        q = self.prune_page_top(q, self.saving_vdiscards() > 0);
        p = self.list_ptr(v);
        self.free_node(v, BOX_NODE_SIZE);
        if q != NULL {
            q = self.vpack_natural(q);
        }
        self.change_box(q); // the eq_level of the box stays the same
        self.vpackage(p, h, EXACTLY, self.split_max_depth())
    }

    /// §1162 `print_plus(i)(s)`.
    fn print_plus(&mut self, i: i32, s: &[u8]) {
        if self.page_so_far[i] != 0 {
            self.print_s(b" plus ");
            self.print_scaled(self.page_so_far[i]);
            self.print_s(s);
        }
    }

    /// §1162
    pub fn print_totals(&mut self) {
        self.print_scaled(self.page_total());
        self.print_plus(2, b"");
        self.print_plus(3, b"fil");
        self.print_plus(4, b"fill");
        self.print_plus(5, b"filll");
        if self.page_shrink() != 0 {
            self.print_s(b" minus ");
            self.print_scaled(self.page_shrink());
        }
    }

    /// §1164
    pub fn freeze_page_specs(&mut self, s: i32) {
        self.page_contents = s;
        self.set_page_goal(self.vsize() as i64);
        self.page_max_depth = self.max_depth();
        self.set_page_depth(0);
        self.page_so_far[1] = 0;
        self.page_so_far[2] = 0;
        self.page_so_far[3] = 0;
        self.page_so_far[4] = 0;
        self.page_so_far[5] = 0;
        self.page_so_far[6] = 0;
        self.least_page_cost = AWFUL_BAD as i64;
        if self.tracing_pages() > 0 {
            self.begin_diagnostic();
            self.print_nl_s(b"%% goal height=");
            self.print_scaled(self.page_goal());
            self.print_s(b", max depth=");
            self.print_scaled(self.page_max_depth);
            self.end_diagnostic(false);
        }
    }

    /// §1169
    #[cold]
    pub fn box_error(&mut self, n: i32) {
        self.error();
        self.begin_diagnostic();
        self.print_nl_s(b"The following box has been deleted:");
        self.show_box(self.box_reg(n));
        self.end_diagnostic(true);
        self.flush_node_list(self.box_reg(n));
        self.set_box_reg(n, NULL);
    }

    /// §1170
    pub fn ensure_vbox(&mut self, n: i32) {
        let p = self.box_reg(n); // the box register contents
        if p != NULL {
            if self.ty(p) == HLIST_NODE {
                self.print_err_s(b"Insertions can only be added to a vbox");
                self.help(&[
                    b"Tut tut: You're trying to \\insert into a",
                    b"\\box register that now contains an \\hbox.",
                    b"Proceed, and I'll discard its present contents.",
                ]);
                self.box_error(n);
            }
        }
    }

    /// §1171
    pub fn build_page(&mut self) {
        if self.link(CONTRIB_HEAD) == NULL || self.output_active {
            return;
        }
        'repeat: loop {
            // continue:
            let p = self.link(CONTRIB_HEAD);
            // §1173 Update the values of last_glue, last_penalty, and last_kern
            if self.last_glue != MAX_HALFWORD {
                self.delete_glue_ref(self.last_glue);
            }
            self.last_penalty = 0;
            self.last_kern = 0;
            self.last_node_type = self.ty(p) + 1;
            if self.ty(p) == GLUE_NODE {
                self.last_glue = self.glue_ptr(p);
                self.add_glue_ref(self.last_glue);
            } else {
                self.last_glue = MAX_HALFWORD;
                if self.ty(p) == PENALTY_NODE {
                    self.last_penalty = self.penalty(p);
                } else if self.ty(p) == KERN_NODE {
                    self.last_kern = self.width(p);
                }
            }
            // §1174 Move node p to the current page; if it is time for a page break, put the nodes
            // following the break back onto the contribution list, and return to the user's output
            // routine if there is one
            'done: {
                'done1: {
                    'contribute: {
                        'update_heights: {
                            // §1177 If the current page is empty and node p is to be deleted, goto done1;
                            // otherwise use node p to update the state of the current page; if this node
                            // is an insertion, goto contribute; otherwise if this node is not a legal
                            // breakpoint, goto contribute or update_heights; otherwise set pi to the
                            // penalty associated with this breakpoint
                            let pi;
                            match self.ty(p) {
                                HLIST_NODE | VLIST_NODE | RULE_NODE => {
                                    if self.page_contents < BOX_THERE {
                                        // §1178 Initialize the current page, insert the \topskip glue ahead
                                        // of p, and goto continue
                                        if self.page_contents == EMPTY {
                                            self.freeze_page_specs(BOX_THERE);
                                        } else {
                                            self.page_contents = BOX_THERE;
                                        }
                                        let q = self.new_skip_param(TOP_SKIP_CODE); // now temp_ptr=glue_ptr(q)
                                        if self.width(self.temp_ptr) > self.height(p) {
                                            self.set_width(
                                                self.temp_ptr,
                                                sat32(
                                                    self.width(self.temp_ptr) as i64
                                                        - self.height(p) as i64,
                                                ),
                                            );
                                        } else {
                                            self.set_width(self.temp_ptr, 0);
                                        }
                                        self.set_link(q, p);
                                        self.set_link(CONTRIB_HEAD, q);
                                        continue 'repeat;
                                    } else {
                                        // §1179 Prepare to move a box or rule node to the current page,
                                        // then goto contribute
                                        self.set_page_total(self.page_total() + self.page_depth() + self.height(p) as i64);
                                        self.set_page_depth(self.depth(p) as i64);
                                        break 'contribute;
                                    }
                                }
                                WHATSIT_NODE => {
                                    if self.page_contents < BOX_THERE
                                        && (self.subtype(p) == PDF_SNAPY_NODE
                                            || self.subtype(p) == PDF_SNAPY_COMP_NODE)
                                    {
                                        self.print_s(b"snap node being discarded");
                                        break 'done1;
                                    } else {
                                        // §1611 Prepare to move whatsit p to the current page, then goto contribute
                                        if self.subtype(p) == PDF_REFXFORM_NODE
                                            || self.subtype(p) == PDF_REFXIMAGE_NODE
                                        {
                                            self.set_page_total(self.page_total() + self.page_depth() + self.pdf_height(p) as i64);
                                            self.set_page_depth(self.pdf_depth(p) as i64);
                                        }
                                        break 'contribute;
                                    }
                                }
                                GLUE_NODE => {
                                    if self.page_contents < BOX_THERE {
                                        break 'done1;
                                    } else if self.precedes_break(self.page_tail) {
                                        pi = 0;
                                    } else {
                                        break 'update_heights;
                                    }
                                }
                                KERN_NODE => {
                                    if self.page_contents < BOX_THERE {
                                        break 'done1;
                                    } else if self.link(p) == NULL {
                                        return;
                                    } else if self.ty(self.link(p)) == GLUE_NODE {
                                        pi = 0;
                                    } else {
                                        break 'update_heights;
                                    }
                                }
                                PENALTY_NODE => {
                                    if self.page_contents < BOX_THERE {
                                        break 'done1;
                                    } else {
                                        pi = self.penalty(p);
                                    }
                                }
                                MARK_NODE => break 'contribute,
                                INS_NODE => {
                                    self.page_append_insertion(p);
                                    break 'contribute;
                                }
                                _ => self.confusion(b"page"),
                            }
                            // §1182 Check if node p is a new champion breakpoint; then if it is time for
                            // a page break, prepare for output, and either fire up the user's output
                            // routine and return or ship out the page and goto done
                            if pi < INF_PENALTY {
                                // §1184 Compute the badness, b, of the current page, using awful_bad if
                                // the box is too full
                                let b;
                                if self.page_total() < self.page_goal() {
                                    if self.page_so_far[3] != 0
                                        || self.page_so_far[4] != 0
                                        || self.page_so_far[5] != 0
                                    {
                                        b = 0;
                                    } else {
                                        b = crate::pack::pack_badness(self.page_goal() - self.page_total(), self.page_so_far[2]);
                                    }
                                } else if self.page_total() - self.page_goal() > self.page_shrink() {
                                    b = AWFUL_BAD;
                                } else {
                                    b = crate::pack::pack_badness(self.page_total() - self.page_goal(), self.page_shrink());
                                }
                                // c is a local i64: b+pi+insert_penalties passes -2^31 with a huge negative
                                // \floatingpenalty, and the oracle prints it exactly.
                                let mut c: i64;
                                if b < AWFUL_BAD {
                                    if pi <= EJECT_PENALTY {
                                        c = pi as i64;
                                    } else if b < INF_BAD {
                                        c = b as i64 + pi as i64 + self.insert_penalties;
                                    } else {
                                        c = DEPLORABLE as i64;
                                    }
                                } else {
                                    c = b as i64;
                                }
                                if self.insert_penalties >= 10000 {
                                    c = AWFUL_BAD as i64;
                                }
                                if self.tracing_pages() > 0 {
                                    // §1183 Display the page break cost
                                    self.begin_diagnostic();
                                    self.print_nl(b'%' as i32);
                                    self.print_s(b" t=");
                                    self.print_totals();
                                    self.print_s(b" g=");
                                    self.print_scaled(self.page_goal());
                                    self.print_s(b" b=");
                                    if b == AWFUL_BAD {
                                        self.print_char(b'*' as i32);
                                    } else {
                                        self.print_int(b);
                                    }
                                    self.print_s(b" p=");
                                    self.print_int(pi);
                                    self.print_s(b" c=");
                                    if c == AWFUL_BAD as i64 {
                                        self.print_char(b'*' as i32);
                                    } else {
                                        self.print_int(c);
                                    }
                                    if c <= self.least_page_cost {
                                        self.print_char(b'#' as i32);
                                    }
                                    self.end_diagnostic(false);
                                }
                                if c <= self.least_page_cost {
                                    self.best_page_break = p;
                                    self.best_size = sat32(self.page_goal());
                                    self.least_page_cost = c;
                                    let mut r = self.link(PAGE_INS_HEAD);
                                    while r != PAGE_INS_HEAD {
                                        self.set_best_ins_ptr(r, self.last_ins_ptr(r));
                                        r = self.link(r);
                                    }
                                }
                                if c == AWFUL_BAD as i64 || pi <= EJECT_PENALTY {
                                    self.fire_up(p); // output the current page at the best place
                                    if self.output_active {
                                        return; // user's output routine will act
                                    }
                                    break 'done; // the page has been shipped out by default output routine
                                }
                            }
                            if self.ty(p) < GLUE_NODE || self.ty(p) > KERN_NODE {
                                break 'contribute;
                            }
                        }
                        // update_heights:
                        // §1181 Update the current page measurements with respect to the glue or kern
                        // specified by node p
                        let mut q;
                        if self.ty(p) == KERN_NODE {
                            q = p;
                        } else {
                            q = self.glue_ptr(p);
                            let o = 2 + self.stretch_order(q);
                            self.page_so_far[o] += self.stretch(q) as i64;
                            self.set_page_shrink(self.page_shrink() + self.shrink(q) as i64);
                            if self.shrink_order(q) != NORMAL && self.shrink(q) != 0 {
                                self.print_err_s(b"Infinite glue shrinkage found on current page");
                                self.help(&[
                                    b"The page about to be output contains some infinitely",
                                    b"shrinkable glue, e.g., `\\vss' or `\\vskip 0pt minus 1fil'.",
                                    b"Such glue doesn't belong there; but you can safely proceed,",
                                    b"since the offensive shrinkability has been made finite.",
                                ]);
                                self.error();
                                let r = self.new_spec(q);
                                self.set_shrink_order(r, NORMAL);
                                self.delete_glue_ref(q);
                                self.set_glue_ptr(p, r);
                                q = r;
                            }
                        }
                        self.set_page_total(self.page_total() + self.page_depth() + self.width(q) as i64);
                        self.set_page_depth(0);
                    }
                    // contribute:
                    // §1180 Make sure that page_max_depth is not exceeded
                    if self.page_depth() > self.page_max_depth as i64 {
                        self.set_page_total(self.page_total() + self.page_depth() - self.page_max_depth as i64);
                        self.set_page_depth(self.page_max_depth as i64);
                    }
                    // §1175 Link node p into the current page and goto done
                    self.set_link(self.page_tail, p);
                    self.page_tail = p;
                    self.set_link(CONTRIB_HEAD, self.link(p));
                    self.set_link(p, NULL);
                    break 'done;
                }
                // done1:
                // §1176 Recycle node p
                self.set_link(CONTRIB_HEAD, self.link(p));
                self.set_link(p, NULL);
                if self.saving_vdiscards() > 0 {
                    if self.page_disc() == NULL {
                        self.set_page_disc(p);
                    } else {
                        self.set_link(self.tail_page_disc(), p);
                    }
                    self.set_tail_page_disc(p);
                } else {
                    self.flush_node_list(p);
                }
            }
            // done:
            if self.link(CONTRIB_HEAD) == NULL {
                break;
            }
        }
        // §1172 Make the contribution list empty by setting its tail to contrib_head
        if self.nest_ptr == 0 {
            self.set_tail(CONTRIB_HEAD); // vertical mode
        } else {
            self.set_contrib_tail(CONTRIB_HEAD); // other modes
        }
    }

    /// §1185 Append an insertion to the current page and goto contribute.  The goto is the caller's.
    fn page_append_insertion(&mut self, p: i32) {
        if self.page_contents == EMPTY {
            self.freeze_page_specs(INSERTS_ONLY);
        }
        let n = self.subtype(p);
        let mut r = PAGE_INS_HEAD;
        while n >= self.subtype(self.link(r)) {
            r = self.link(r);
        }
        if self.subtype(r) != n {
            // §1186 Create a page insertion node with subtype(r)=qi(n), and include the glue
            // correction for box n in the current page state
            let mut q = self.get_node(PAGE_INS_NODE_SIZE);
            self.set_link(q, self.link(r));
            self.set_link(r, q);
            r = q;
            self.set_subtype(r, n);
            self.set_ty(r, INSERTING);
            self.ensure_vbox(n);
            if self.box_reg(n) == NULL {
                self.set_height(r, 0);
            } else {
                self.set_height(
                    r,
                    sat32(self.height(self.box_reg(n)) as i64 + self.depth(self.box_reg(n)) as i64),
                );
            }
            self.set_best_ins_ptr(r, NULL);
            q = self.skip(n);
            let h = if self.count(n) == 1000 {
                self.height(r)
            } else {
                sat32(self.x_over_n(self.height(r), 1000) as i64 * self.count(n) as i64)
            };
            self.set_page_goal(self.page_goal() - h as i64 - self.width(q) as i64);
            let o = 2 + self.stretch_order(q);
            self.page_so_far[o] += self.stretch(q) as i64;
            self.set_page_shrink(self.page_shrink() + self.shrink(q) as i64);
            if self.shrink_order(q) != NORMAL && self.shrink(q) != 0 {
                self.print_err_s(b"Infinite glue shrinkage inserted from ");
                self.print_esc_s(b"skip");
                self.print_int(n);
                self.help(&[
                    b"The correction glue for page breaking with insertions",
                    b"must have finite shrinkability. But you may proceed,",
                    b"since the offensive shrinkability has been made finite.",
                ]);
                self.error();
            }
        }
        if self.ty(r) == SPLIT_UP {
            self.insert_penalties += self.float_cost(p) as i64;
        } else {
            self.set_last_ins_ptr(r, p);
            // this much room is left if we shrink the maximum
            let delta = self.page_goal() - self.page_total() - self.page_depth() + self.page_shrink();
            // this much room is needed
            let h = if self.count(n) == 1000 {
                self.height(p)
            } else {
                sat32(self.x_over_n(self.height(p), 1000) as i64 * self.count(n) as i64)
            };
            if (h <= 0 || h as i64 <= delta)
                && self.height(p) as i64 + self.height(r) as i64 <= self.dimen(n) as i64
            {
                self.set_page_goal(self.page_goal() - h as i64);
                self.set_height(r, sat32(self.height(r) as i64 + self.height(p) as i64));
            } else {
                // §1187 Find the best way to split the insertion, and change type(r) to split_up
                let mut w;
                if self.count(n) <= 0 {
                    w = MAX_DIMEN;
                } else {
                    w = sat32(self.page_goal() - self.page_total() - self.page_depth());
                    if self.count(n) != 1000 {
                        w = sat32(self.x_over_n(w, self.count(n)) as i64 * 1000);
                    }
                }
                let room = self.dimen(n) as i64 - self.height(r) as i64;
                if w as i64 > room {
                    w = sat32(room);
                }
                let q = self.vert_break(self.ins_ptr(p), w, self.depth(p));
                self.set_height(
                    r,
                    sat32(self.height(r) as i64 + self.best_height_plus_depth as i64),
                );
                if self.tracing_pages() > 0 {
                    // §1188 Display the insertion split cost
                    self.begin_diagnostic();
                    self.print_nl_s(b"% split");
                    self.print_int(n);
                    self.print_s(b" to ");
                    self.print_scaled(w);
                    self.print_char(b',' as i32);
                    self.print_scaled(self.best_height_plus_depth);
                    self.print_s(b" p=");
                    if q == NULL {
                        self.print_int(EJECT_PENALTY);
                    } else if self.ty(q) == PENALTY_NODE {
                        self.print_int(self.penalty(q));
                    } else {
                        self.print_char(b'0' as i32);
                    }
                    self.end_diagnostic(false);
                }
                if self.count(n) != 1000 {
                    self.best_height_plus_depth = sat32(
                        self.x_over_n(self.best_height_plus_depth, 1000) as i64
                            * self.count(n) as i64,
                    );
                }
                self.set_page_goal(self.page_goal() - self.best_height_plus_depth as i64);
                self.set_ty(r, SPLIT_UP);
                self.set_broken_ptr(r, q);
                self.set_broken_ins(r, p);
                if q == NULL {
                    self.insert_penalties += EJECT_PENALTY as i64;
                } else if self.ty(q) == PENALTY_NODE {
                    self.insert_penalties += self.penalty(q) as i64;
                }
            }
        }
    }

    /// §1189
    pub fn fire_up(&mut self, c: i32) {
        let mut p: i32;
        let mut q: i32;
        let mut r: i32;
        let mut s: i32;
        let mut prev_p: i32;
        let mut n: i32;
        let mut wait: bool;
        // §1190 Set the value of output_penalty
        if self.ty(self.best_page_break) == PENALTY_NODE {
            self.geq_word_define(
                INT_BASE + OUTPUT_PENALTY_CODE,
                self.penalty(self.best_page_break),
            );
            self.set_penalty(self.best_page_break, INF_PENALTY);
        } else {
            self.geq_word_define(INT_BASE + OUTPUT_PENALTY_CODE, INF_PENALTY);
        }
        if self.sa_mark() != NULL {
            if self.do_marks(FIRE_UP_INIT, 0, self.sa_mark()) {
                self.set_sa_mark(NULL);
            }
        }
        if self.bot_mark() != NULL {
            if self.top_mark() != NULL {
                self.delete_token_ref(self.top_mark());
            }
            self.set_top_mark(self.bot_mark());
            self.add_token_ref(self.top_mark());
            self.delete_token_ref(self.first_mark());
            self.set_first_mark(NULL);
        }
        // §1191 Put the optimal current page into box 255, update first_mark and bot_mark, append
        // insertions to their boxes, and put the remaining nodes back on the contribution list
        if c == self.best_page_break {
            self.best_page_break = NULL; // c not yet linked in
        }
        // §1192 Ensure that box 255 is empty before output
        if self.box_reg(255) != NULL {
            self.print_err_s(b"");
            self.print_esc_s(b"box");
            self.print_s(b"255 is not void");
            self.help(&[
                b"You shouldn't use \\box255 except in \\output routines.",
                b"Proceed, and I'll discard its present contents.",
            ]);
            self.box_error(255);
        }
        self.insert_penalties = 0; // this will count the number of insertions held over
        let save_split_top_skip = self.split_top_skip();
        if self.holding_inserts() <= 0 {
            // §1195 Prepare all the boxes involved in insertions to act as queues
            r = self.link(PAGE_INS_HEAD);
            while r != PAGE_INS_HEAD {
                if self.best_ins_ptr(r) != NULL {
                    n = self.subtype(r);
                    self.ensure_vbox(n);
                    if self.box_reg(n) == NULL {
                        let b = self.new_null_box();
                        self.set_box_reg(n, b);
                    }
                    p = self.box_reg(n) + LIST_OFFSET;
                    while self.link(p) != NULL {
                        p = self.link(p);
                    }
                    self.set_last_ins_ptr(r, p);
                }
                r = self.link(r);
            }
        }
        q = HOLD_HEAD;
        self.set_link(q, NULL);
        prev_p = PAGE_HEAD;
        p = self.link(prev_p);
        while p != self.best_page_break {
            if self.ty(p) == INS_NODE {
                if self.holding_inserts() <= 0 {
                    // §1197 Either insert the material specified by node p into the appropriate box,
                    // or hold it for the next page; also delete node p from the current page
                    r = self.link(PAGE_INS_HEAD);
                    while self.subtype(r) != self.subtype(p) {
                        r = self.link(r);
                    }
                    if self.best_ins_ptr(r) == NULL {
                        wait = true;
                    } else {
                        wait = false;
                        s = self.last_ins_ptr(r);
                        self.set_link(s, self.ins_ptr(p));
                        if self.best_ins_ptr(r) == p {
                            // §1198 Wrap up the box specified by node r, splitting node p if called for;
                            // set wait:=true if node p holds a remainder after splitting
                            if self.ty(r) == SPLIT_UP {
                                if self.broken_ins(r) == p && self.broken_ptr(r) != NULL {
                                    while self.link(s) != self.broken_ptr(r) {
                                        s = self.link(s);
                                    }
                                    self.set_link(s, NULL);
                                    self.set_split_top_skip(self.split_top_ptr(p));
                                    let t = self.prune_page_top(self.broken_ptr(r), false);
                                    self.set_ins_ptr(p, t);
                                    if self.ins_ptr(p) != NULL {
                                        self.temp_ptr = self.vpack_natural(self.ins_ptr(p));
                                        self.set_height(
                                            p,
                                            sat32(
                                                self.height(self.temp_ptr) as i64
                                                    + self.depth(self.temp_ptr) as i64,
                                            ),
                                        );
                                        self.free_node(self.temp_ptr, BOX_NODE_SIZE);
                                        wait = true;
                                    }
                                }
                            }
                            self.set_best_ins_ptr(r, NULL);
                            n = self.subtype(r);
                            self.temp_ptr = self.list_ptr(self.box_reg(n));
                            self.free_node(self.box_reg(n), BOX_NODE_SIZE);
                            let b = self.vpack_natural(self.temp_ptr);
                            self.set_box_reg(n, b);
                        } else {
                            while self.link(s) != NULL {
                                s = self.link(s);
                            }
                            self.set_last_ins_ptr(r, s);
                        }
                    }
                    // §1199 Either append the insertion node p after node q, and remove it from the
                    // current page, or delete node(p)
                    self.set_link(prev_p, self.link(p));
                    self.set_link(p, NULL);
                    if wait {
                        self.set_link(q, p);
                        q = p;
                        self.insert_penalties += 1;
                    } else {
                        self.delete_glue_ref(self.split_top_ptr(p));
                        self.free_node(p, INS_NODE_SIZE);
                    }
                    p = prev_p;
                }
            } else if self.ty(p) == MARK_NODE {
                if self.mark_class(p) != 0 {
                    // §1830 Update the current marks for fire_up
                    self.find_sa_element(MARK_VAL, self.mark_class(p), true);
                    if self.sa_first_mark(self.cur_ptr) == NULL {
                        self.set_sa_first_mark(self.cur_ptr, self.mark_ptr(p));
                        self.add_token_ref(self.mark_ptr(p));
                    }
                    if self.sa_bot_mark(self.cur_ptr) != NULL {
                        self.delete_token_ref(self.sa_bot_mark(self.cur_ptr));
                    }
                    self.set_sa_bot_mark(self.cur_ptr, self.mark_ptr(p));
                    self.add_token_ref(self.mark_ptr(p));
                } else {
                    // §1193 Update the values of first_mark and bot_mark
                    if self.first_mark() == NULL {
                        self.set_first_mark(self.mark_ptr(p));
                        self.add_token_ref(self.first_mark());
                    }
                    if self.bot_mark() != NULL {
                        self.delete_token_ref(self.bot_mark());
                    }
                    self.set_bot_mark(self.mark_ptr(p));
                    self.add_token_ref(self.bot_mark());
                }
            }
            prev_p = p;
            p = self.link(prev_p);
        }
        self.set_split_top_skip(save_split_top_skip);
        // §1194 Break the current page at node p, put it in box 255, and put the remaining nodes on the
        // contribution list
        if p != NULL {
            if self.link(CONTRIB_HEAD) == NULL {
                if self.nest_ptr == 0 {
                    self.set_tail(self.page_tail);
                } else {
                    self.set_contrib_tail(self.page_tail);
                }
            }
            self.set_link(self.page_tail, self.link(CONTRIB_HEAD));
            self.set_link(CONTRIB_HEAD, p);
            self.set_link(prev_p, NULL);
        }
        let save_vbadness = self.vbadness();
        self.set_vbadness(INF_BAD);
        let save_vfuzz = self.vfuzz();
        self.set_vfuzz(MAX_DIMEN); // inhibit error messages
        let b = self.vpackage(
            self.link(PAGE_HEAD),
            self.best_size,
            EXACTLY,
            self.page_max_depth,
        );
        self.set_box_reg(255, b);
        self.set_vbadness(save_vbadness);
        self.set_vfuzz(save_vfuzz);
        if self.last_glue != MAX_HALFWORD {
            self.delete_glue_ref(self.last_glue);
        }
        // §1168 Start a new current page
        self.page_contents = EMPTY;
        self.page_tail = PAGE_HEAD;
        self.set_link(PAGE_HEAD, NULL);
        self.last_glue = MAX_HALFWORD;
        self.last_penalty = 0;
        self.last_kern = 0;
        self.last_node_type = -1;
        self.set_page_depth(0);
        self.page_max_depth = 0;
        // this sets last_glue:=max_halfword
        if q != HOLD_HEAD {
            self.set_link(PAGE_HEAD, self.link(HOLD_HEAD));
            self.page_tail = q;
        }
        // §1196 Delete the page-insertion nodes
        r = self.link(PAGE_INS_HEAD);
        while r != PAGE_INS_HEAD {
            q = self.link(r);
            self.free_node(r, PAGE_INS_NODE_SIZE);
            r = q;
        }
        self.set_link(PAGE_INS_HEAD, PAGE_INS_HEAD);
        if self.sa_mark() != NULL {
            if self.do_marks(FIRE_UP_DONE, 0, self.sa_mark()) {
                self.set_sa_mark(NULL);
            }
        }
        if self.top_mark() != NULL && self.first_mark() == NULL {
            self.set_first_mark(self.top_mark());
            self.add_token_ref(self.top_mark());
        }
        if self.output_routine() != NULL {
            if self.dead_cycles >= self.max_dead_cycles() {
                // §1201 Explain that too many dead cycles have occurred in a row
                self.print_err_s(b"Output loop---");
                self.print_int(self.dead_cycles);
                self.print_s(b" consecutive dead cycles");
                self.help(&[
                    b"I've concluded that your \\output is awry; it never does a",
                    b"\\shipout, so I'm shipping \\box255 out myself. Next time",
                    b"increase \\maxdeadcycles if you want me to be more patient!",
                ]);
                self.error();
            } else {
                // §1202 Fire up the user's output routine and return
                self.output_active = true;
                self.dead_cycles += 1;
                self.push_nest();
                self.set_mode(-VMODE);
                self.set_prev_depth(self.pdf_ignored_dimen());
                self.set_mode_line(-self.line);
                self.begin_token_list(self.output_routine(), OUTPUT_TEXT);
                self.new_save_level(OUTPUT_GROUP);
                self.normal_paragraph();
                self.scan_left_brace();
                return;
            }
        }
        // §1200 Perform the default output routine
        if self.link(PAGE_HEAD) != NULL {
            if self.link(CONTRIB_HEAD) == NULL {
                if self.nest_ptr == 0 {
                    self.set_tail(self.page_tail);
                } else {
                    self.set_contrib_tail(self.page_tail);
                }
            } else {
                self.set_link(self.page_tail, self.link(CONTRIB_HEAD));
            }
            self.set_link(CONTRIB_HEAD, self.link(PAGE_HEAD));
            self.set_link(PAGE_HEAD, NULL);
            self.page_tail = PAGE_HEAD;
        }
        self.flush_node_list(self.page_disc());
        self.set_page_disc(NULL);
        self.ship_out(self.box_reg(255));
        self.set_box_reg(255, NULL);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::save::testkit::*;
    use crate::toks::testkit::*;

    fn fresh() -> &'static mut Tex {
        let t = table_machine();
        print_setup(t);
        t.last_glue = MAX_HALFWORD;
        t.set_max_depth(MAX_DIMEN);
        t
    }

    fn pt(s: &str) -> i32 {
        if s == "M" {
            MAX_DIMEN
        } else {
            s.parse::<i32>().unwrap() * 65536
        }
    }

    fn spec(t: &mut Tex, w: &str, st: &str, order: &str, sh: &str) -> i32 {
        let g = t.new_spec(ZERO_GLUE);
        t.set_width(g, pt(w));
        t.set_stretch(g, pt(st));
        t.set_stretch_order(g, order.parse().unwrap());
        t.set_shrink(g, pt(sh));
        g
    }

    fn setting(t: &mut Tex, s: &str) {
        let (k, v) = s.split_once('=').unwrap();
        let f: Vec<&str> = v.split(',').collect();
        let n = k[1..].parse::<i32>().unwrap_or(0);
        match k {
            "vsize" => t.set_vsize(pt(v)),
            "maxdepth" => t.set_max_depth(pt(v)),
            "topskip" => {
                let g = spec(t, v, "0", "0", "0");
                t.set_top_skip(g);
            }
            _ => match k.as_bytes()[0] {
                b'C' => t.set_count(n, v.parse().unwrap()),
                b'D' => t.set_dimen(n, pt(v)),
                b'S' => {
                    let g = spec(t, f[0], f[1], f[2], f[3]);
                    t.set_skip(n, g);
                }
                b'B' => {
                    let b = t.new_null_box();
                    t.set_ty(b, VLIST_NODE);
                    t.set_height(b, pt(f[0]));
                    t.set_depth(b, pt(f[1]));
                    t.set_box_reg(n, b);
                }
                _ => panic!("{}", s),
            },
        }
    }

    /// The node of one item, and its natural size.  An insert's size is what vpack would make of its list.
    fn item(t: &mut Tex, it: &str) -> (i32, i32) {
        let f: Vec<&str> = it[1..].split(',').collect();
        match it.as_bytes()[0] {
            b'r' => {
                let p = t.new_rule();
                t.set_height(p, pt(f[0]));
                t.set_depth(p, pt(f[1]));
                t.set_width(p, 0);
                (p, pt(f[0]) + pt(f[1]))
            }
            b'g' => {
                let g = spec(t, f[0], f[1], f[2], f[3]);
                let p = t.new_glue(g);
                t.set_glue_ref_count(g, NULL);
                (p, pt(f[0]))
            }
            b'k' => (t.new_kern(pt(f[0])), pt(f[0])),
            b'p' => (t.new_penalty(f[0].parse().unwrap()), 0),
            b'i' => {
                let (head, body) = it[1..].split_once(':').unwrap();
                let h: Vec<&str> = head.split(',').collect();
                let (mut first, mut last, mut total) = (NULL, NULL, 0i64);
                for x in body.split('/') {
                    let (p, w) = item(t, x);
                    total += w as i64;
                    if first == NULL {
                        first = p;
                    } else {
                        t.set_link(last, p);
                    }
                    last = p;
                }
                let q = t.get_node(INS_NODE_SIZE);
                t.set_ty(q, INS_NODE);
                t.set_subtype(q, h[0].parse().unwrap());
                t.set_height(q, sat32(total));
                t.set_ins_ptr(q, first);
                t.set_split_top_ptr(q, ZERO_GLUE);
                t.set_depth(q, pt(h[1]));
                t.set_float_cost(q, h[2].parse().unwrap());
                (q, 0)
            }
            _ => panic!("{}", it),
        }
    }

    fn contribute(t: &mut Tex, p: i32) {
        t.set_link(t.tail(), p);
        t.set_tail(p);
    }

    /// Runs a row of page_oracle.tsv; build_page sees all of it at once where the oracle saw it in pieces.
    fn run_row(row: &[&str]) -> (String, String) {
        let t = fresh();
        t.set_tracing_pages(1);
        for s in row[1].split(' ').filter(|s| !s.is_empty()) {
            setting(t, s);
        }
        printed(t);
        for it in row[2].split(' ').filter(|s| !s.is_empty() && *s != "P") {
            let (p, _) = item(t, it);
            contribute(t, p);
        }
        t.build_page();
        let got: Vec<String> = printed(t)
            .lines()
            .filter(|l| !l.is_empty())
            .map(String::from)
            .collect();
        let skip = if t.last_glue == MAX_HALFWORD {
            0
        } else {
            t.width(t.last_glue)
        };
        let state = format!(
            "{},{},{},{}",
            t.last_penalty, t.last_kern, skip, t.last_node_type
        );
        Tex::dealloc(t);
        (got.join("|"), state)
    }

    #[test]
    fn build_page_traces_what_the_oracle_traces() {
        let rows: Vec<Vec<&str>> =
            core::str::from_utf8(include_bytes!("../testdata/page_oracle.tsv"))
                .unwrap()
                .lines()
                .map(|l| l.split('\t').collect())
                .collect();
        assert!(rows.len() >= 45);
        let mut bad = Vec::new();
        for row in &rows {
            let (got, state) = run_row(row);
            if got != row[3] || state != row[4] {
                bad.push(format!(
                    "--- {}\nwant {} {}\ngot  {} {}",
                    row[0], row[3], row[4], got, state
                ));
            }
        }
        assert!(
            bad.is_empty(),
            "{} of {} differ:\n{}",
            bad.len(),
            rows.len(),
            bad.join("\n")
        );
    }

    /// Three maximal rules with no break between them.  The oracle's 64-bit total is above the goal, and a 32-bit
    /// one that wrapped would be negative, and one that clamped would not come back.
    #[test]
    fn page_total_holds_what_would_wrap() {
        let t = fresh();
        t.set_vsize(MAX_DIMEN);
        for _ in 0..3 {
            let p = t.new_rule();
            t.set_height(p, MAX_DIMEN);
            t.set_depth(p, 0);
            contribute(t, p);
        }
        let p = t.new_penalty(INF_PENALTY);
        contribute(t, p);
        t.build_page();
        assert_eq!(t.page_total(), 3 * MAX_DIMEN as i64);
        Tex::dealloc(t);
    }

    #[test]
    fn vert_break_saturates_where_it_would_wrap() {
        let t = fresh();
        let (mut first, mut last) = (NULL, NULL);
        for v in [100, 200, 300] {
            for _ in 0..3 {
                let p = t.new_rule();
                t.set_height(p, MAX_DIMEN);
                t.set_depth(p, 0);
                if first == NULL {
                    first = p;
                } else {
                    t.set_link(last, p);
                }
                last = p;
            }
            let p = t.new_penalty(v);
            t.set_link(last, p);
            last = p;
        }
        let q = t.vert_break(first, MAX_DIMEN, 0);
        assert_eq!(t.ty(q), PENALTY_NODE);
        assert_eq!(
            t.penalty(q),
            100,
            "the first break is too full, and it is also the best"
        );
        assert_eq!(t.best_height_plus_depth, i32::MAX);
        Tex::dealloc(t);
    }

    fn list(t: &mut Tex, items: &[&str]) -> Vec<i32> {
        let nodes: Vec<i32> = items.iter().map(|it| item(t, it).0).collect();
        for w in nodes.windows(2) {
            t.set_link(w[0], w[1]);
        }
        nodes
    }

    #[test]
    fn prune_page_top_discards_up_to_the_first_box() {
        let t = fresh();
        let g = spec(t, "7", "0", "0", "0");
        t.set_split_top_skip(g);
        let n = list(t, &["g1,0,0,0", "k2", "p3", "r5,1", "g1,0,0,0"]);
        let used = t.var_used;
        let q = t.prune_page_top(n[0], false);
        assert_eq!(t.ty(q), GLUE_NODE, "split_top_skip goes before the box");
        assert_eq!(t.subtype(q), SPLIT_TOP_SKIP_CODE + 1);
        assert_eq!(
            t.width(t.glue_ptr(q)),
            pt("2"),
            "less the height of the box"
        );
        assert_eq!(t.link(q), n[3]);
        assert_eq!(t.split_disc(), NULL);
        assert!(
            t.var_used < used + GLUE_SPEC_SIZE + SMALL_NODE_SIZE,
            "the discarded nodes are gone"
        );
        Tex::dealloc(t);
    }

    #[test]
    fn prune_page_top_keeps_the_discards_when_asked() {
        let t = fresh();
        let g = spec(t, "7", "0", "0", "0");
        t.set_split_top_skip(g);
        let n = list(t, &["g1,0,0,0", "k2", "p3", "r9,1"]);
        let q = t.prune_page_top(n[0], true);
        assert_eq!(
            t.width(t.glue_ptr(q)),
            0,
            "a box taller than the skip leaves none"
        );
        assert_eq!(t.split_disc(), n[0]);
        assert_eq!(t.link(n[0]), n[1]);
        assert_eq!(t.link(n[1]), n[2]);
        assert_eq!(t.link(n[2]), NULL);
        Tex::dealloc(t);
    }

    #[test]
    fn prune_page_top_moves_past_marks_and_may_empty_the_list() {
        let t = fresh();
        let m = t.get_node(SMALL_NODE_SIZE);
        t.set_ty(m, MARK_NODE);
        t.set_subtype(m, 0);
        let k = t.new_kern(1);
        t.set_link(m, k);
        let q = t.prune_page_top(m, false);
        assert_eq!(q, m, "a mark stays, the kern after it goes");
        assert_eq!(t.link(m), NULL);
        let k = t.new_kern(1);
        assert_eq!(t.prune_page_top(k, false), NULL);
        Tex::dealloc(t);
    }

    #[test]
    fn freeze_page_specs_and_print_totals() {
        let t = fresh();
        t.set_tracing_pages(1);
        t.set_vsize(pt("100"));
        t.set_max_depth(pt("2"));
        t.page_so_far[1] = 5;
        t.page_so_far[6] = 5;
        t.freeze_page_specs(INSERTS_ONLY);
        assert_eq!(
            (
                t.page_contents,
                t.page_total(),
                t.page_shrink(),
                t.least_page_cost
            ),
            (INSERTS_ONLY, 0, 0, AWFUL_BAD as i64)
        );
        assert_eq!(
            printed(t).trim_start_matches('\n'),
            "%% goal height=100.0, max depth=2.0\n"
        );
        t.set_page_total(pt("1") as i64);
        t.page_so_far[2] = pt("2") as i64;
        t.page_so_far[3] = pt("3") as i64;
        t.page_so_far[4] = pt("4") as i64;
        t.page_so_far[5] = pt("5") as i64;
        t.set_page_shrink(pt("6") as i64);
        t.print_totals();
        assert_eq!(
            printed(t),
            "1.0 plus 2.0 plus 3.0fil plus 4.0fill plus 5.0filll minus 6.0"
        );
        Tex::dealloc(t);
    }
}
