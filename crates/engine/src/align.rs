//! §944-988 Part 37: alignment.
//!
//! The templates are token lists in `self.toks`, with a header or 0 in `u_part` and `v_part`; the
//! rest of the alignment's lists are nodes in `mem`, as written.  docs/PORTING.md section 15,
//! "Templates".
//!
//! `fin_align`'s `t` is an `i64`, section 11's rule for a site not in its table: it accumulates
//! widths of columns and tabskips, which the oracle's 64-bit `integer` holds past 2^31.
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
    /// §948
    pub fn push_alignment(&mut self) {
        let p = self.get_node(ALIGN_STACK_NODE_SIZE);
        self.set_link(p, self.align_ptr);
        self.set_info(p, self.cur_align);
        self.set_llink(p, self.preamble());
        self.set_rlink(p, self.cur_span);
        let v = self.cur_loop;
        self.mem[p + 2].set_int(v);
        let v = self.align_state;
        self.mem[p + 3].set_int(v);
        self.set_info(p + 4, self.cur_head);
        self.set_link(p + 4, self.cur_tail);
        self.set_info(p + 5, self.cur_pre_head);
        self.set_link(p + 5, self.cur_pre_tail);
        self.align_ptr = p;
        self.cur_head = self.get_avail();
        self.cur_pre_head = self.get_avail();
    }

    /// §948
    pub fn pop_alignment(&mut self) {
        self.free_avail(self.cur_head);
        self.free_avail(self.cur_pre_head);
        let p = self.align_ptr;
        self.cur_tail = self.link(p + 4);
        self.cur_head = self.info(p + 4);
        self.cur_pre_tail = self.link(p + 5);
        self.cur_pre_head = self.info(p + 5);
        self.align_state = self.mem[p + 3].int();
        self.cur_loop = self.mem[p + 2].int();
        self.cur_span = self.rlink(p);
        self.set_preamble(self.llink(p));
        self.cur_align = self.info(p);
        self.align_ptr = self.link(p);
        self.free_node(p, ALIGN_STACK_NODE_SIZE);
    }

    /// §950
    pub fn init_align(&mut self) {
        let save_cs_ptr = self.cur_cs; // `\halign` or `\valign`, usually
        self.push_alignment();
        self.align_state = -1000000; // enter a new alignment level
        // §952 Check for improper alignment in displayed math
        if self.mode() == MMODE && (self.tail() != self.head() || self.incompleat_noad() != NULL) {
            self.print_err_s(b"Improper ");
            self.print_esc_s(b"halign");
            self.print_s(b" inside $$'s");
            self.help(&[
                b"Displays can use special alignments (like \\eqalignno)",
                b"only if nothing but the alignment itself is between $$'s.",
                b"So I've deleted the formulas that preceded this alignment.",
            ]);
            self.error();
            self.flush_math();
        }
        self.push_nest(); // enter a new semantic level
        // §951 Change current mode to -vmode for \halign, -hmode for \valign
        if self.mode() == MMODE {
            self.set_mode(-VMODE);
            let v = self.nest[self.nest_ptr - 2].aux_field.sc();
            self.set_prev_depth(v);
        } else if self.mode() > 0 {
            self.set_mode(-self.mode());
        }
        self.scan_spec(ALIGN_GROUP, false);
        // §953 Scan the preamble and record it in the preamble list
        self.set_preamble(NULL);
        self.cur_align = ALIGN_HEAD;
        self.cur_loop = NULL;
        self.scanner_status = ALIGNING;
        self.warning_index = save_cs_ptr;
        self.align_state = -1000000; // at this point, cur_cmd=left_brace
        loop {
            // §954 Append the current tabskip glue to the preamble list
            let g = self.new_param_glue(TAB_SKIP_CODE);
            self.set_link(self.cur_align, g);
            self.cur_align = g;
            if self.cur_cmd == CAR_RET {
                break; // \cr ends the preamble
            }
            // §955 Scan preamble text until cur_cmd is tab_mark or car_ret
            // §959 Scan the template u_j, putting the resulting token list in hold_head
            let m = self.tb_mark();
            self.tb_align_mark = m;
            loop {
                self.get_preamble_token();
                if self.cur_cmd == MAC_PARAM {
                    break; // done1
                }
                if self.cur_cmd <= CAR_RET && self.cur_cmd >= TAB_MARK && self.align_state == -1000000 {
                    if self.tb_len(m) == 0 && self.cur_loop == NULL && self.cur_cmd == TAB_MARK {
                        self.cur_loop = self.cur_align;
                    } else {
                        self.print_err_s(b"Missing # inserted in alignment preamble");
                        self.help(&[
                            b"There should be exactly one # between &'s, when an",
                            b"\\halign or \\valign is being set up. In this case you had",
                            b"none, so I've put one in; maybe that will work.",
                        ]);
                        self.back_error();
                        break; // done1
                    }
                } else if self.cur_cmd != SPACER || self.tb_len(m) != 0 {
                    self.tb_push(self.cur_tok);
                }
            }
            // done1:
            let n = self.new_null_box(); // a new alignrecord
            self.set_link(self.cur_align, n);
            self.cur_align = n;
            self.set_info(self.cur_align, END_SPAN);
            self.set_width(self.cur_align, NULL_FLAG);
            let u = self.tb_commit(m);
            self.set_u_part(self.cur_align, u);
            // §960 Scan the template v_j, putting the resulting token list in hold_head
            let m = self.tb_mark();
            self.tb_align_mark = m;
            loop {
                self.get_preamble_token(); // continue:
                if self.cur_cmd <= CAR_RET && self.cur_cmd >= TAB_MARK && self.align_state == -1000000 {
                    break; // done2
                }
                if self.cur_cmd == MAC_PARAM {
                    self.print_err_s(b"Only one # is allowed per tab");
                    self.help(&[
                        b"There should be exactly one # between &'s, when an",
                        b"\\halign or \\valign is being set up. In this case you had",
                        b"more than one, so I'm ignoring all but the first.",
                    ]);
                    self.error();
                    continue;
                }
                self.tb_push(self.cur_tok);
            }
            // done2: put \endtemplate at the end
            self.tb_push(END_TEMPLATE_TOKEN);
            let v = self.tb_commit(m);
            self.set_v_part(self.cur_align, v);
        }
        // done:
        self.scanner_status = NORMAL;
        self.new_save_level(ALIGN_GROUP);
        if self.every_cr() != NULL {
            self.begin_token_list(self.every_cr(), EVERY_CR_TEXT);
        }
        self.align_peek(); // look for \noalign or \omit
    }

    /// §958
    pub fn get_preamble_token(&mut self) {
        loop {
            // restart:
            self.get_token();
            while self.cur_chr == SPAN_CODE && self.cur_cmd == TAB_MARK {
                self.get_token(); // this token will be expanded once
                if self.cur_cmd > MAX_COMMAND {
                    self.expand();
                    self.get_token();
                }
            }
            if self.cur_cmd == ENDV {
                self.fatal_error(b"(interwoven alignment preambles are not allowed)");
            }
            if self.cur_cmd == ASSIGN_GLUE && self.cur_chr == GLUE_BASE + TAB_SKIP_CODE {
                self.scan_optional_equals();
                self.scan_glue(GLUE_VAL);
                if self.global_defs() > 0 {
                    self.geq_define(GLUE_BASE + TAB_SKIP_CODE, GLUE_REF, self.cur_val);
                } else {
                    self.eq_define(GLUE_BASE + TAB_SKIP_CODE, GLUE_REF, self.cur_val);
                }
                continue;
            }
            break;
        }
    }

    /// §961
    pub fn align_peek(&mut self) {
        loop {
            // restart:
            self.align_state = 1000000;
            loop {
                self.get_x_or_protected();
                if self.cur_cmd != SPACER {
                    break;
                }
            }
            if self.cur_cmd == NO_ALIGN {
                self.scan_left_brace();
                self.new_save_level(NO_ALIGN_GROUP);
                if self.mode() == -VMODE {
                    self.normal_paragraph();
                }
            } else if self.cur_cmd == RIGHT_BRACE {
                self.fin_align();
            } else if self.cur_cmd == CAR_RET && self.cur_chr == CR_CR_CODE {
                continue; // ignore \crcr
            } else {
                self.init_row(); // start a new row
                self.init_col(); // start a new column and replace what we peeked at
            }
            break;
        }
    }

    /// §962
    pub fn init_row(&mut self) {
        self.push_nest();
        self.set_mode((-HMODE - VMODE) - self.mode());
        if self.mode() == -HMODE {
            self.set_space_factor(0);
        } else {
            self.set_prev_depth(0);
        }
        let g = self.new_glue(self.glue_ptr(self.preamble()));
        self.tail_append(g);
        self.set_subtype(self.tail(), TAB_SKIP_CODE + 1);
        self.cur_align = self.link(self.preamble());
        self.cur_tail = self.cur_head;
        self.cur_pre_tail = self.cur_pre_head;
        self.init_span(self.cur_align);
    }

    /// §963
    pub fn init_span(&mut self, p: i32) {
        self.push_nest();
        if self.mode() == -HMODE {
            self.set_space_factor(1000);
        } else {
            self.set_prev_depth(self.pdf_ignored_dimen());
            self.normal_paragraph();
        }
        self.cur_span = p;
    }

    /// §964
    pub fn init_col(&mut self) {
        self.set_extra_info(self.cur_align, self.cur_cmd);
        if self.cur_cmd == OMIT {
            self.align_state = 0;
        } else {
            self.back_input();
            self.begin_token_list(self.u_part(self.cur_align), U_TEMPLATE);
        } // now align_state=1000000
    }

    /// §967
    pub fn fin_col(&mut self) -> bool {
        if self.cur_align == NULL {
            self.confusion(b"endv");
        }
        let mut q = self.link(self.cur_align);
        if q == NULL {
            self.confusion(b"endv");
        }
        if self.align_state < 500000 {
            self.fatal_error(b"(interwoven alignment preambles are not allowed)");
        }
        let mut p = self.link(q);
        // §968 If the preamble list has been traversed, check that the row has ended
        if p == NULL && self.extra_info(self.cur_align) < CR_CODE {
            if self.cur_loop != NULL {
                // §969 Lengthen the preamble periodically
                p = self.new_null_box(); // a new alignrecord
                self.set_link(q, p);
                self.set_info(p, END_SPAN);
                self.set_width(p, NULL_FLAG);
                self.cur_loop = self.link(self.cur_loop);
                // §970 Copy the templates from node cur_loop into node p
                let l = self.tok_copy(self.u_part(self.cur_loop));
                self.set_u_part(p, l);
                let l = self.tok_copy(self.v_part(self.cur_loop));
                self.set_v_part(p, l);
                self.cur_loop = self.link(self.cur_loop);
                let g = self.new_glue(self.glue_ptr(self.cur_loop));
                self.set_link(p, g);
                self.set_subtype(self.link(p), TAB_SKIP_CODE + 1);
            } else {
                self.print_err_s(b"Extra alignment tab has been changed to ");
                self.print_esc_s(b"cr");
                self.help(&[
                    b"You have given more \\span or & marks than there were",
                    b"in the preamble to the \\halign or \\valign now in progress.",
                    b"So I'll assume that you meant to type \\cr instead.",
                ]);
                self.set_extra_info(self.cur_align, CR_CODE);
                self.error();
            }
        }
        if self.extra_info(self.cur_align) != SPAN_CODE {
            self.unsave();
            self.new_save_level(ALIGN_GROUP);
            // §972 Package an unset box for the current column and record its width
            let u;
            let w;
            if self.mode() == -HMODE {
                self.adjust_tail = self.cur_tail;
                self.pre_adjust_tail = self.cur_pre_tail;
                u = self.hpack_natural(self.link(self.head()));
                w = self.width(u);
                self.cur_tail = self.adjust_tail;
                self.adjust_tail = NULL;
                self.cur_pre_tail = self.pre_adjust_tail;
                self.pre_adjust_tail = NULL;
            } else {
                u = self.vpackage(self.link(self.head()), 0, ADDITIONAL, 0);
                w = self.height(u);
            }
            let mut n = 0; // this represents a span count of 1
            if self.cur_span != self.cur_align {
                // §974 Update width entry for spanned columns
                q = self.cur_span;
                loop {
                    n += 1;
                    q = self.link(self.link(q));
                    if q == self.cur_align {
                        break;
                    }
                }
                if n > WEB_MAX_QUARTERWORD {
                    self.confusion(b"256 spans"); // this can happen, but won't
                }
                q = self.cur_span;
                while self.link(self.info(q)) < n {
                    q = self.info(q);
                }
                if self.link(self.info(q)) > n {
                    let s = self.get_node(SPAN_NODE_SIZE);
                    self.set_info(s, self.info(q));
                    self.set_link(s, n);
                    self.set_info(q, s);
                    self.set_width(s, w);
                } else if self.width(self.info(q)) < w {
                    self.set_width(self.info(q), w);
                }
            } else if w > self.width(self.cur_align) {
                self.set_width(self.cur_align, w);
            }
            self.set_ty(u, UNSET_NODE);
            self.set_span_count(u, n);
            // §835 Determine the stretch order
            let mut o = if self.total_stretch[FILLL] != 0 {
                FILLL
            } else if self.total_stretch[FILL] != 0 {
                FILL
            } else if self.total_stretch[FIL] != 0 {
                FIL
            } else {
                NORMAL
            };
            self.set_glue_order(u, o);
            self.set_glue_stretch(u, sat32(self.total_stretch[o]));
            // §841 Determine the shrink order
            o = if self.total_shrink[FILLL] != 0 {
                FILLL
            } else if self.total_shrink[FILL] != 0 {
                FILL
            } else if self.total_shrink[FIL] != 0 {
                FIL
            } else {
                NORMAL
            };
            self.set_glue_sign(u, o);
            self.set_glue_shrink(u, sat32(self.total_shrink[o]));
            self.pop_nest();
            self.set_link(self.tail(), u);
            self.set_tail(u);
            // §971 Copy the tabskip glue between columns
            let g = self.new_glue(self.glue_ptr(self.link(self.cur_align)));
            self.tail_append(g);
            self.set_subtype(self.tail(), TAB_SKIP_CODE + 1);
            if self.extra_info(self.cur_align) >= CR_CODE {
                return true;
            }
            self.init_span(p);
        }
        self.align_state = 1000000;
        loop {
            self.get_x_or_protected();
            if self.cur_cmd != SPACER {
                break;
            }
        }
        self.cur_align = p;
        self.init_col();
        false
    }

    /// §975
    pub fn fin_row(&mut self) {
        let p; // the new unset box
        if self.mode() == -HMODE {
            p = self.hpack_natural(self.link(self.head()));
            self.pop_nest();
            if self.cur_pre_head != self.cur_pre_tail {
                self.append_list(self.cur_pre_head, self.cur_pre_tail);
            }
            self.append_to_vlist(p);
            if self.cur_head != self.cur_tail {
                self.append_list(self.cur_head, self.cur_tail);
            }
        } else {
            p = self.vpack_natural(self.link(self.head()));
            self.pop_nest();
            self.set_link(self.tail(), p);
            self.set_tail(p);
            self.set_space_factor(1000);
        }
        self.set_ty(p, UNSET_NODE);
        self.set_glue_stretch(p, 0);
        if self.every_cr() != NULL {
            self.begin_token_list(self.every_cr(), EVERY_CR_TEXT);
        }
        self.align_peek();
    } // note that glue_shrink(p)=0 since glue_shrink==shift_amount

    /// §976
    pub fn fin_align(&mut self) {
        if self.cur_group != ALIGN_GROUP {
            self.confusion(b"align1");
        }
        self.unsave(); // that align_group was for individual entries
        if self.cur_group != ALIGN_GROUP {
            self.confusion(b"align0");
        }
        self.unsave(); // that align_group was for the whole alignment
        let o = if self.nest[self.nest_ptr - 1].mode_field == MMODE {
            self.display_indent()
        } else {
            0
        };
        // §977 Go through the preamble list, determining the column widths
        let mut q = self.link(self.preamble());
        let mut p;
        loop {
            self.flush_list(self.u_part(q));
            self.flush_list(self.v_part(q));
            p = self.link(self.link(q));
            if self.width(q) == NULL_FLAG {
                // §978 Nullify width(q) and the tabskip glue following this column
                self.set_width(q, 0);
                let r = self.link(q);
                let s = self.glue_ptr(r);
                if s != ZERO_GLUE {
                    self.add_glue_ref(ZERO_GLUE);
                    self.delete_glue_ref(s);
                    self.set_glue_ptr(r, ZERO_GLUE);
                }
            }
            if self.info(q) != END_SPAN {
                // §979 Merge the widths in the span nodes of q with those of p
                let t = self.width(q) as i64 + self.width(self.glue_ptr(self.link(q))) as i64;
                let mut r = self.info(q);
                let mut s = END_SPAN;
                self.set_info(s, p);
                let mut n = 1;
                loop {
                    self.set_width(r, sat32(self.width(r) as i64 - t));
                    let u = self.info(r);
                    while self.link(r) > n {
                        s = self.info(s);
                        n = self.link(self.info(s)) + 1;
                    }
                    if self.link(r) < n {
                        self.set_info(r, self.info(s));
                        self.set_info(s, r);
                        self.set_link(r, self.link(r) - 1);
                        s = r;
                    } else {
                        if self.width(r) > self.width(self.info(s)) {
                            self.set_width(self.info(s), self.width(r));
                        }
                        self.free_node(r, SPAN_NODE_SIZE);
                    }
                    r = u;
                    if r == END_SPAN {
                        break;
                    }
                }
            }
            self.set_ty(q, UNSET_NODE);
            self.set_span_count(q, 0);
            self.set_height(q, 0);
            self.set_depth(q, 0);
            self.set_glue_order(q, NORMAL);
            self.set_glue_sign(q, NORMAL);
            self.set_glue_stretch(q, 0);
            self.set_glue_shrink(q, 0);
            q = p;
            if q == NULL {
                break;
            }
        }
        // §980 Package the preamble list, to determine the actual tabskip glue amounts
        self.save_ptr -= 2;
        self.pack_begin_line = -self.mode_line();
        if self.mode() == -VMODE {
            let rule_save = self.overfull_rule();
            self.set_overfull_rule(0); // prevent rule from being packaged
            p = self.hpack(self.preamble(), self.saved(1), self.saved(0));
            self.set_overfull_rule(rule_save);
        } else {
            q = self.link(self.preamble());
            loop {
                self.set_height(q, self.width(q));
                self.set_width(q, 0);
                q = self.link(self.link(q));
                if q == NULL {
                    break;
                }
            }
            p = self.vpack(self.preamble(), self.saved(1), self.saved(0));
            q = self.link(self.preamble());
            loop {
                self.set_width(q, self.height(q));
                self.set_height(q, 0);
                q = self.link(self.link(q));
                if q == NULL {
                    break;
                }
            }
        }
        self.pack_begin_line = 0;
        // §981 Set the glue in all the unset boxes of the current list
        q = self.link(self.head());
        let mut s = self.head();
        while q != NULL {
            if !self.is_char_node(q) {
                if self.ty(q) == UNSET_NODE {
                    // §983 Set the unset box q and the unset boxes in it
                    if self.mode() == -VMODE {
                        self.set_ty(q, HLIST_NODE);
                        self.set_width(q, self.width(p));
                        if self.nest[self.nest_ptr - 1].mode_field == MMODE {
                            self.set_box_lr(q, DLIST); // for ship_out
                        }
                    } else {
                        self.set_ty(q, VLIST_NODE);
                        self.set_height(q, self.height(p));
                    }
                    self.set_glue_order(q, self.glue_order(p));
                    self.set_glue_sign(q, self.glue_sign(p));
                    self.set_glue_set(q, self.glue_set(p));
                    self.set_shift_amount(q, o);
                    let mut r = self.link(self.list_ptr(q));
                    s = self.link(self.list_ptr(p));
                    loop {
                        // §984 Set the glue in node r and change it from an unset node
                        let mut n = self.span_count(r);
                        let mut t = self.width(s) as i64;
                        let w = self.width(s);
                        let mut u = HOLD_HEAD;
                        self.set_box_lr(r, 0); // for ship_out
                        while n > 0 {
                            n -= 1;
                            // §985 Append tabskip glue and an empty box to list u, and update s and t
                            s = self.link(s);
                            let v = self.glue_ptr(s);
                            let g = self.new_glue(v);
                            self.set_link(u, g);
                            u = g;
                            self.set_subtype(u, TAB_SKIP_CODE + 1);
                            t += self.width(v) as i64;
                            if self.glue_sign(p) == STRETCHING {
                                if self.stretch_order(v) == self.glue_order(p) {
                                    t += round(self.glue_set(p) * self.stretch(v) as f64) as i64;
                                }
                            } else if self.glue_sign(p) == SHRINKING {
                                if self.shrink_order(v) == self.glue_order(p) {
                                    t -= round(self.glue_set(p) * self.shrink(v) as f64) as i64;
                                }
                            }
                            s = self.link(s);
                            let b = self.new_null_box();
                            self.set_link(u, b);
                            u = b;
                            t += self.width(s) as i64;
                            if self.mode() == -VMODE {
                                self.set_width(u, self.width(s));
                            } else {
                                self.set_ty(u, VLIST_NODE);
                                self.set_height(u, self.width(s));
                            }
                        }
                        if self.mode() == -VMODE {
                            // §986 Make the unset node r into an hlist_node of width w
                            self.set_height(r, self.height(q));
                            self.set_depth(r, self.depth(q));
                            let x = self.width(r) as i64;
                            self.align_set_unset_glue(r, t, x);
                            self.set_width(r, w);
                            self.set_ty(r, HLIST_NODE);
                        } else {
                            // §987 Make the unset node r into a vlist_node of height w
                            self.set_width(r, self.width(q));
                            let x = self.height(r) as i64;
                            self.align_set_unset_glue(r, t, x);
                            self.set_height(r, w);
                            self.set_ty(r, VLIST_NODE);
                        }
                        self.set_shift_amount(r, 0);
                        if u != HOLD_HEAD {
                            // append blank boxes to account for spanned nodes
                            self.set_link(u, self.link(r));
                            self.set_link(r, self.link(HOLD_HEAD));
                            r = u;
                        }
                        r = self.link(self.link(r));
                        s = self.link(self.link(s));
                        if r == NULL {
                            break;
                        }
                    }
                } else if self.ty(q) == RULE_NODE {
                    // §982 Make the running dimensions in rule q extend to the boundaries
                    if self.is_running(self.width(q)) {
                        self.set_width(q, self.width(p));
                    }
                    if self.is_running(self.height(q)) {
                        self.set_height(q, self.height(p));
                    }
                    if self.is_running(self.depth(q)) {
                        self.set_depth(q, self.depth(p));
                    }
                    if o != 0 {
                        let r = self.link(q);
                        self.set_link(q, NULL);
                        q = self.hpack_natural(q);
                        self.set_shift_amount(q, o);
                        self.set_link(q, r);
                        self.set_link(s, q);
                    }
                }
            }
            s = q;
            q = self.link(q);
        }
        self.flush_node_list(p);
        self.pop_alignment();
        // §988 Insert the current list into its environment
        let aux_save = self.aux();
        p = self.link(self.head());
        q = self.tail();
        self.pop_nest();
        if self.mode() == MMODE {
            // §1384 Finish an alignment in a display
            self.do_assignments();
            if self.cur_cmd != MATH_SHIFT {
                // §1385 Pontificate about improper alignment in display
                self.print_err_s(b"Missing $$ inserted");
                self.help(&[
                    b"Displays can use special alignments (like \\eqalignno)",
                    b"only if nothing but the alignment itself is between $$'s.",
                ]);
                self.back_error();
            } else {
                // §1375 Check that another $ follows
                self.get_x_token();
                if self.cur_cmd != MATH_SHIFT {
                    self.print_err_s(b"Display math should end with $$");
                    self.help(&[
                        b"The `$' that I just saw supposedly matches a previous `$$'.",
                        b"So I shall assume that you typed `$$' both times.",
                    ]);
                    self.back_error();
                }
            }
            self.flush_node_list(self.LR_box());
            self.pop_nest();
            let n = self.new_penalty(self.pre_display_penalty());
            self.tail_append(n);
            let n = self.new_param_glue(ABOVE_DISPLAY_SKIP_CODE);
            self.tail_append(n);
            self.set_link(self.tail(), p);
            if p != NULL {
                self.set_tail(q);
            }
            let n = self.new_penalty(self.post_display_penalty());
            self.tail_append(n);
            let n = self.new_param_glue(BELOW_DISPLAY_SKIP_CODE);
            self.tail_append(n);
            self.set_prev_depth(aux_save.sc());
            self.resume_after_display();
        } else {
            self.set_aux(aux_save);
            self.set_link(self.tail(), p);
            if p != NULL {
                self.set_tail(q);
            }
            if self.mode() == VMODE {
                self.build_page();
            }
        }
    }

    /// The part §986 and §987 share: set the glue of the unset node `r`, whose natural size in the
    /// direction of the alignment is `x`, so that it comes out as `t`.
    fn align_set_unset_glue(&mut self, r: i32, t: i64, x: i64) {
        if t == x {
            self.set_glue_sign(r, NORMAL);
            self.set_glue_order(r, NORMAL);
            self.set_glue_set(r, 0.0);
        } else if t > x {
            self.set_glue_sign(r, STRETCHING);
            if self.glue_stretch(r) == 0 {
                self.set_glue_set(r, 0.0);
            } else {
                self.set_glue_set(r, (t - x) as f64 / self.glue_stretch(r) as f64);
            }
        } else {
            self.set_glue_order(r, self.glue_sign(r));
            self.set_glue_sign(r, SHRINKING);
            if self.glue_shrink(r) == 0 {
                self.set_glue_set(r, 0.0);
            } else if self.glue_order(r) == NORMAL && x - t > self.glue_shrink(r) as i64 {
                self.set_glue_set(r, 1.0);
            } else {
                self.set_glue_set(r, (x - t) as f64 / self.glue_shrink(r) as f64);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::save::testkit::*;
    use crate::toks::testkit::*;

    fn letter(c: u8) -> i32 {
        LETTER_TOKEN + c as i32
    }

    /// A preamble as init_align leaves it: `glue record glue`, with the templates given.
    fn preamble_of_one(t: &mut Tex, u: i32, v: i32) -> i32 {
        let g = t.new_param_glue(TAB_SKIP_CODE);
        t.set_preamble(g);
        let r = t.new_null_box();
        t.set_link(g, r);
        t.set_info(r, END_SPAN);
        t.set_width(r, NULL_FLAG);
        t.set_u_part(r, u);
        t.set_v_part(r, v);
        let g2 = t.new_param_glue(TAB_SKIP_CODE);
        t.set_link(r, g2);
        r
    }

    #[test]
    fn pop_alignment_gives_back_what_push_alignment_saved() {
        let t = table_machine();
        t.push_alignment(); // an outermost level, for cur_head and cur_pre_head to exist
        t.cur_align = 11;
        t.cur_span = 12;
        t.cur_loop = 13;
        t.align_state = -999;
        t.set_preamble(14);
        t.cur_tail = 15;
        t.cur_pre_tail = 16;
        let (head, pre_head, ptr) = (t.cur_head, t.cur_pre_head, t.align_ptr);
        let (var, dy) = (t.var_used, t.dyn_used);
        t.push_alignment();
        assert_ne!(t.align_ptr, ptr);
        assert_eq!(t.link(t.align_ptr), ptr);
        assert_ne!(t.cur_head, head);
        assert_ne!(t.cur_pre_head, pre_head);
        assert_ne!(t.cur_head, t.cur_pre_head);
        t.cur_align = 21;
        t.cur_span = 22;
        t.cur_loop = 23;
        t.align_state = 1000000;
        t.set_preamble(24);
        t.cur_tail = 25;
        t.cur_pre_tail = 26;
        t.pop_alignment();
        assert_eq!(
            (t.cur_align, t.cur_span, t.cur_loop, t.align_state, t.preamble(), t.cur_tail, t.cur_pre_tail),
            (11, 12, 13, -999, 14, 15, 16)
        );
        assert_eq!((t.cur_head, t.cur_pre_head, t.align_ptr), (head, pre_head, ptr));
        assert_eq!((t.var_used, t.dyn_used), (var, dy), "the stack node and the two heads are free again");
        Tex::dealloc(t);
    }

    fn preamble_tokens(t: &mut Tex, input: &[i32]) -> Vec<i32> {
        feed(t, input);
        let mut v = Vec::new();
        for _ in 0..input.len() + 5 {
            t.get_preamble_token();
            if t.cur_tok == 0 {
                break;
            }
            v.push(t.cur_tok);
        }
        v
    }

    #[test]
    fn get_preamble_token_expands_once_after_span() {
        let t = table_machine();
        print_setup(t);
        t.align_state = -1000000;
        t.scanner_status = ALIGNING;
        t.set_eq_type(cs(b's'), TAB_MARK);
        t.set_equiv(cs(b's'), SPAN_CODE);
        def_macro(t, cs(b'm'), CALL, &[], &[letter(b'a'), letter(b'b')]);
        def_macro(t, cs(b'n'), CALL, &[], &[cs_tok(b'm'), letter(b'c')]);
        let (s, m, n) = (cs_tok(b's'), cs_tok(b'm'), cs_tok(b'n'));
        // Not after \span, a macro stays a token: the preamble is not expanded.
        assert_eq!(preamble_tokens(t, &[m, letter(b'z')]), vec![m, letter(b'z')]);
        // After \span, one level.
        assert_eq!(preamble_tokens(t, &[s, m, letter(b'z')]), vec![letter(b'a'), letter(b'b'), letter(b'z')]);
        assert_eq!(preamble_tokens(t, &[s, n, letter(b'z')]), vec![m, letter(b'c'), letter(b'z')]);
        // A run of \span still expands only what follows it, once.
        assert_eq!(preamble_tokens(t, &[s, s, n, letter(b'z')]), vec![m, letter(b'c'), letter(b'z')]);
        // The token after the expansion may be \span again.
        assert_eq!(preamble_tokens(t, &[s, letter(b'y'), s, m]), vec![letter(b'y'), letter(b'a'), letter(b'b')]);
        // A \span whose next token is not expandable gives that token, whatever it is.
        assert_eq!(preamble_tokens(t, &[s, s, letter(b'y')]), vec![letter(b'y')]);
        Tex::dealloc(t);
    }

    #[test]
    fn init_row_starts_a_row_and_a_span_in_the_other_direction() {
        let t = table_machine();
        print_setup(t);
        t.push_alignment();
        let r = preamble_of_one(t, NULL, NULL);
        t.set_mode(-VMODE);
        t.set_prev_depth(5);
        let n = t.nest_ptr;
        t.init_row();
        assert_eq!(t.nest_ptr, n + 2, "the row and the entry");
        assert_eq!((t.mode(), t.space_factor(), t.cur_align, t.cur_span), (-HMODE, 1000, r, r));
        assert_eq!((t.cur_tail, t.cur_pre_tail), (t.cur_head, t.cur_pre_head));
        // The row's list has the tabskip glue, of the subtype that says so.
        let row = t.nest[t.nest_ptr - 1].tail_field;
        assert_eq!(t.ty(row), GLUE_NODE);
        assert_eq!(t.subtype(row), TAB_SKIP_CODE + 1);
        assert_eq!(t.glue_ptr(row), t.glue_ptr(t.preamble()));
        Tex::dealloc(t);
    }

    #[test]
    fn init_col_starts_the_template_or_omits_it() {
        let t = table_machine();
        print_setup(t);
        t.push_alignment();
        let u = t.tok_one(letter(b'u'));
        let r = preamble_of_one(t, u, NULL);
        t.cur_align = r;
        // A token read, and not \omit: it comes back after the u part.
        feed(t, &[letter(b'z')]);
        t.get_token();
        t.align_state = 1000000;
        t.init_col();
        assert_eq!((t.extra_info(r), t.align_state), (LETTER, 1000000));
        t.get_token();
        assert_eq!(t.cur_tok, letter(b'u'));
        t.get_token();
        assert_eq!(t.cur_tok, letter(b'z'));
        assert_eq!(t.align_state, 0, "section 346, at the end of the u part");
        // A null u part is an empty level: the token comes back at once.
        t.set_u_part(r, NULL);
        feed(t, &[letter(b'y')]);
        t.get_token();
        t.align_state = 1000000;
        t.init_col();
        t.get_token();
        assert_eq!(t.cur_tok, letter(b'y'));
        // \omit: not put back, no template, and the v part will be found when align_state is 0.
        t.set_u_part(r, u);
        t.cur_cmd = OMIT;
        t.init_col();
        assert_eq!((t.extra_info(r), t.align_state), (OMIT, 0));
        t.get_token();
        assert_eq!(t.cur_tok, 0, "nothing was put in front of the input");
        t.flush_list(u);
        Tex::dealloc(t);
    }

    /// What the oracle's log of tests/integration/align/glue_set_branches.tex has, and limits_big.tex.
    #[test]
    fn set_unset_glue_takes_each_branch() {
        let t = table_machine();
        let r = t.get_node(BOX_NODE_SIZE);
        let pt = 65536i64;
        // t=x.
        t.set_glue_stretch(r, 3);
        t.set_glue_shrink(r, 3);
        t.set_glue_order(r, FIL);
        t.set_glue_sign(r, FILL);
        t.align_set_unset_glue(r, 10 * pt, 10 * pt);
        assert_eq!((t.glue_sign(r), t.glue_order(r), t.glue_set(r)), (NORMAL, NORMAL, 0.0));
        // t>x, no stretch and stretch.
        t.set_glue_stretch(r, 0);
        t.set_glue_order(r, FIL);
        t.align_set_unset_glue(r, 20 * pt, 5 * pt);
        assert_eq!((t.glue_sign(r), t.glue_order(r), t.glue_set(r)), (STRETCHING, FIL, 0.0));
        t.set_glue_stretch(r, 2 * 65536);
        t.set_glue_order(r, FIL);
        t.align_set_unset_glue(r, 20 * pt, 5 * pt);
        assert_eq!((t.glue_sign(r), t.glue_order(r), t.glue_set(r)), (STRETCHING, FIL, 7.5));
        // t<x: no shrink.  The shrink order was kept in glue_sign.
        t.set_glue_shrink(r, 0);
        t.set_glue_sign(r, NORMAL);
        t.align_set_unset_glue(r, 18 * pt, 20 * pt);
        assert_eq!((t.glue_sign(r), t.glue_order(r), t.glue_set(r)), (SHRINKING, NORMAL, 0.0));
        // Too little finite shrink: 1.0.  Exactly enough: 1.0 too, by division.
        t.set_glue_shrink(r, 65536);
        t.set_glue_sign(r, NORMAL);
        t.align_set_unset_glue(r, 18 * pt, 20 * pt);
        assert_eq!((t.glue_sign(r), t.glue_order(r), t.glue_set(r)), (SHRINKING, NORMAL, 1.0));
        t.set_glue_shrink(r, 2 * 65536);
        t.set_glue_sign(r, NORMAL);
        t.align_set_unset_glue(r, 18 * pt, 20 * pt);
        assert_eq!(t.glue_set(r), 1.0);
        t.set_glue_shrink(r, 100 * 65536);
        t.set_glue_sign(r, NORMAL);
        t.align_set_unset_glue(r, 18 * pt, 20 * pt);
        assert_eq!(t.glue_set(r), 0.02);
        // Infinite shrink is no limit.
        t.set_glue_shrink(r, 65536);
        t.set_glue_sign(r, FIL);
        t.align_set_unset_glue(r, 18 * pt, 20 * pt);
        assert_eq!((t.glue_sign(r), t.glue_order(r), t.glue_set(r)), (SHRINKING, FIL, 2.0));
        // t past 2^31: from a span of three columns of \maxdimen, on the oracle's log.
        t.set_glue_stretch(r, 1073741823);
        t.set_glue_order(r, NORMAL);
        t.align_set_unset_glue(r, 3221225469, 0);
        assert_eq!(t.glue_set(r), 3.0);
        Tex::dealloc(t);
    }
}
