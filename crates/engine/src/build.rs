//! §1233-1313 Part 47: building boxes and lists.
//!
//! Every `@<Cases of |handle_right_brace|...@>` is in place in `handle_right_brace`.
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
    /// §1238
    pub fn append_glue(&mut self) {
        let s = self.cur_chr;
        match s {
            FIL_CODE => self.cur_val = FIL_GLUE,
            FILL_CODE => self.cur_val = FILL_GLUE,
            SS_CODE => self.cur_val = SS_GLUE,
            FIL_NEG_CODE => self.cur_val = FIL_NEG_GLUE,
            SKIP_CODE => self.scan_glue(GLUE_VAL),
            MSKIP_CODE => self.scan_glue(MU_VAL),
            _ => {}
        } // now cur_val points to the glue specification
        let p = self.new_glue(self.cur_val);
        self.tail_append(p);
        if s >= SKIP_CODE {
            self.set_glue_ref_count(self.cur_val, self.glue_ref_count(self.cur_val) - 1);
            if s > SKIP_CODE {
                self.set_subtype(self.tail(), MU_GLUE);
            }
        }
    }

    /// §1239
    pub fn append_kern(&mut self) {
        let s = self.cur_chr;
        self.scan_dimen(s == MU_GLUE, false, false);
        let p = self.new_kern(self.cur_val);
        self.tail_append(p);
        self.set_subtype(self.tail(), s);
    }

    /// §1242
    #[cold]
    pub fn off_save(&mut self) {
        if self.cur_group == BOTTOM_LEVEL {
            // §1244 Drop current token and complain that it was unmatched
            self.print_err_s(b"Extra ");
            self.print_cmd_chr(self.cur_cmd, self.cur_chr);
            self.help(&[b"Things are pretty mixed up, but I think the worst is over."]);
            self.error();
        } else {
            self.back_input();
            self.print_err_s(b"Missing ");
            // §1243 Prepare to insert a token that matches cur_group, and print what it is
            let p = match self.cur_group {
                SEMI_SIMPLE_GROUP => {
                    self.print_esc_s(b"endgroup");
                    self.tok_one(CS_TOKEN_FLAG + FROZEN_END_GROUP)
                }
                MATH_SHIFT_GROUP => {
                    self.print_char(b'$' as i32);
                    self.tok_one(MATH_SHIFT_TOKEN + b'$' as i32)
                }
                MATH_LEFT_GROUP => {
                    self.print_esc_s(b"right.");
                    self.tok_two(CS_TOKEN_FLAG + FROZEN_RIGHT, OTHER_TOKEN + b'.' as i32)
                }
                _ => {
                    self.print_char(b'}' as i32);
                    self.tok_one(RIGHT_BRACE_TOKEN + b'}' as i32)
                }
            };
            self.print_s(b" inserted");
            self.ins_list(p);
            self.help(&[
                b"I've inserted something that you may have forgotten.",
                b"(See the <inserted text> above.)",
                b"With luck, this will get me unwedged. But if you",
                b"really didn't forget anything, try typing `2' now; then",
                b"my insertion and my current dilemma will both disappear.",
            ]);
            self.error();
        }
    }

    /// `back_input; cur_tok:=par_token; back_input; token_type:=inserted`, which §1263, §1278, §1311 and §1346 all do.
    fn build_insert_par_token(&mut self) {
        self.back_input();
        self.cur_tok = self.par_token;
        self.back_input();
        self.set_token_type(INSERTED);
    }

    /// §1246
    pub fn handle_right_brace(&mut self) {
        match self.cur_group {
            SIMPLE_GROUP => self.unsave(),
            BOTTOM_LEVEL => {
                self.print_err_s(b"Too many }'s");
                self.help(&[
                    b"You've closed more groups than you opened.",
                    b"Such booboos are generally harmless, so keep going.",
                ]);
                self.error();
            }
            SEMI_SIMPLE_GROUP | MATH_SHIFT_GROUP | MATH_LEFT_GROUP => self.extra_right_brace(),
            // §1263 Cases of handle_right_brace where a right_brace triggers a delayed action
            HBOX_GROUP => self.package(0),
            ADJUSTED_HBOX_GROUP => {
                self.adjust_tail = ADJUST_HEAD;
                self.pre_adjust_tail = PRE_ADJUST_HEAD;
                self.package(0);
            }
            VBOX_GROUP => {
                if self.partoken_context() > 0 && self.mode() == HMODE {
                    self.build_insert_par_token();
                } else {
                    self.end_graf();
                    self.package(0);
                }
            }
            VTOP_GROUP => {
                if self.partoken_context() > 0 && self.mode() == HMODE {
                    self.build_insert_par_token();
                } else {
                    self.end_graf();
                    self.package(VTOP_CODE);
                }
            }
            // §1278
            INSERT_GROUP => {
                if self.partoken_context() > 1 && self.mode() == HMODE {
                    self.build_insert_par_token();
                } else {
                    self.end_graf();
                    let q = self.split_top_skip();
                    self.add_glue_ref(q);
                    let d = self.split_max_depth();
                    let f = self.floating_penalty();
                    self.unsave();
                    self.save_ptr -= 2; // now saved(0) is the insertion number, or 255 for vadjust
                    let p = self.vpack_natural(self.link(self.head()));
                    self.pop_nest();
                    if self.saved(0) < 255 {
                        let g = self.get_node(INS_NODE_SIZE);
                        self.tail_append(g);
                        self.set_ty(self.tail(), INS_NODE);
                        self.set_subtype(self.tail(), self.saved(0));
                        // A sum of two box dimensions, each of which can be clamped near 2^31: PORTING section 11.
                        let h = self.height(p) as i64 + self.depth(p) as i64;
                        self.set_height(self.tail(), sat32(h));
                        self.set_ins_ptr(self.tail(), self.list_ptr(p));
                        self.set_split_top_ptr(self.tail(), q);
                        self.set_depth(self.tail(), d);
                        self.set_float_cost(self.tail(), f);
                    } else {
                        let g = self.get_node(SMALL_NODE_SIZE);
                        self.tail_append(g);
                        self.set_ty(self.tail(), ADJUST_NODE);
                        self.set_adjust_pre(self.tail(), self.saved(1)); // the subtype is used for adjust_pre
                        self.set_adjust_ptr(self.tail(), self.list_ptr(p));
                        self.delete_glue_ref(q);
                    }
                    self.free_node(p, BOX_NODE_SIZE);
                    if self.nest_ptr == 0 {
                        self.build_page();
                    }
                }
            }
            OUTPUT_GROUP => {
                if self.partoken_context() > 1 && self.mode() == HMODE {
                    self.build_insert_par_token();
                } else {
                    // §1203 Resume the page builder after an output routine has come to an end
                    while self.at_list_end() && self.token_type() == BACKED_UP {
                        self.end_token_list(); // output-ending brace may have been backed-up
                    }
                    if !self.at_list_end() || self.token_type() != OUTPUT_TEXT {
                        // §1204 Recover from an unbalanced output routine
                        self.print_err_s(b"Unbalanced output routine");
                        self.help(&[
                            b"Your sneaky output routine has problematic {'s and/or }'s.",
                            b"I can't handle that very well; good luck.",
                        ]);
                        self.error();
                        // loops forever if reading from a file, since null=min_halfword<=0
                        loop {
                            self.get_token();
                            if self.at_list_end() {
                                break;
                            }
                        }
                    }
                    self.output_can_end = true;
                    self.end_token_list(); // conserve stack space in case more outputs are triggered
                    self.output_can_end = false;
                    self.end_graf();
                    self.unsave();
                    self.output_active = false;
                    self.insert_penalties = 0;
                    // §1205 Ensure that box 255 is empty after output
                    if self.box_reg(255) != NULL {
                        self.print_err_s(b"Output routine didn't use all of ");
                        self.print_esc_s(b"box");
                        self.print_int(255);
                        self.help(&[
                            b"Your \\output commands should empty \\box255,",
                            b"e.g., by saying `\\shipout\\box255'.",
                            b"Proceed; I'll discard its present contents.",
                        ]);
                        self.box_error(255);
                    }
                    if self.tail() != self.head() {
                        // current list goes after heldover insertions
                        self.set_link(self.page_tail, self.link(self.head()));
                        self.page_tail = self.tail();
                    }
                    if self.link(PAGE_HEAD) != NULL {
                        // and both go before heldover contributions
                        if self.link(CONTRIB_HEAD) == NULL {
                            self.set_contrib_tail(self.page_tail);
                        }
                        self.set_link(self.page_tail, self.link(CONTRIB_HEAD));
                        self.set_link(CONTRIB_HEAD, self.link(PAGE_HEAD));
                        self.set_link(PAGE_HEAD, NULL);
                        self.page_tail = PAGE_HEAD;
                    }
                    self.flush_node_list(self.page_disc());
                    self.set_page_disc(NULL);
                    self.pop_nest();
                    self.build_page();
                }
            }
            // §1296
            DISC_GROUP => self.build_discretionary(),
            // §1310
            ALIGN_GROUP => {
                self.back_input();
                self.cur_tok = CS_TOKEN_FLAG + FROZEN_CR;
                self.print_err_s(b"Missing ");
                self.print_esc_s(b"cr");
                self.print_s(b" inserted");
                self.help(&[b"I'm guessing that you meant to end an alignment here."]);
                self.ins_error();
            }
            // §1311
            NO_ALIGN_GROUP => {
                if self.partoken_context() > 1 && self.mode() == HMODE {
                    self.build_insert_par_token();
                } else {
                    self.end_graf();
                    self.unsave();
                    self.align_peek();
                }
            }
            // §1346
            VCENTER_GROUP => {
                if self.partoken_context() > 0 && self.mode() == HMODE {
                    self.build_insert_par_token();
                } else {
                    self.end_graf();
                    self.unsave();
                    self.save_ptr -= 2;
                    let p = self.vpack(self.link(self.head()), self.saved(1), self.saved(0));
                    self.pop_nest();
                    let n = self.new_noad();
                    self.tail_append(n);
                    self.set_ty(self.tail(), VCENTER_NOAD);
                    self.set_math_type(self.nucleus(self.tail()), SUB_BOX);
                    self.set_info(self.nucleus(self.tail()), p);
                }
            }
            // §1351
            MATH_CHOICE_GROUP => self.build_choices(),
            // §1364
            MATH_GROUP => {
                self.unsave();
                self.save_ptr -= 1;
                self.set_math_type(self.saved(0), SUB_MLIST);
                let p = self.fin_mlist(NULL);
                self.set_info(self.saved(0), p);
                if p != NULL && self.link(p) == NULL {
                    if self.ty(p) == ORD_NOAD {
                        if self.math_type(self.subscr(p)) == EMPTY && self.math_type(self.supscr(p)) == EMPTY {
                            let s = self.saved(0);
                            let n = self.nucleus(p);
                            self.mem[s] = self.mem[n];
                            self.free_node(p, NOAD_SIZE);
                        }
                    } else if self.ty(p) == ACCENT_NOAD
                        && self.saved(0) == self.nucleus(self.tail())
                        && self.ty(self.tail()) == ORD_NOAD
                    {
                        // §1365 Replace the tail of the list by p
                        let mut q = self.head();
                        while self.link(q) != self.tail() {
                            q = self.link(q);
                        }
                        self.set_link(q, p);
                        self.free_node(self.tail(), NOAD_SIZE);
                        self.set_tail(p);
                    }
                }
            }
            _ => self.confusion(b"rightbrace"),
        }
    }

    /// §1247
    #[cold]
    pub fn extra_right_brace(&mut self) {
        self.print_err_s(b"Extra }, or forgotten ");
        match self.cur_group {
            SEMI_SIMPLE_GROUP => self.print_esc_s(b"endgroup"),
            MATH_SHIFT_GROUP => self.print_char(b'$' as i32),
            MATH_LEFT_GROUP => self.print_esc_s(b"right"),
            _ => {}
        }
        self.help(&[
            b"I've deleted a group-closing symbol because it seems to be",
            b"spurious, as in `$x}$'. But perhaps the } is legitimate and",
            b"you forgot something else, as in `\\hbox{$x}'. In such cases",
            b"the way to recover is to insert both the forgotten and the",
            b"deleted material, e.g., by typing `I$}'.",
        ]);
        self.error();
        self.align_state += 1;
    }

    /// §1248
    pub fn normal_paragraph(&mut self) {
        if self.looseness() != 0 {
            self.eq_word_define(INT_BASE + LOOSENESS_CODE, 0);
        }
        if self.hang_indent() != 0 {
            self.eq_word_define(DIMEN_BASE + HANG_INDENT_CODE, 0);
        }
        if self.hang_after() != 1 {
            self.eq_word_define(INT_BASE + HANG_AFTER_CODE, 1);
        }
        if self.par_shape_ptr() != NULL {
            self.eq_define(PAR_SHAPE_LOC, SHAPE_REF, NULL);
        }
        if self.inter_line_penalties_ptr() != NULL {
            self.eq_define(INTER_LINE_PENALTIES_LOC, SHAPE_REF, NULL);
        }
    }

    /// §1253
    pub fn box_end(&mut self, box_context: i32) {
        if box_context < BOX_FLAG {
            // §1254 Append box cur_box to the current list, shifted by box_context
            if self.cur_box != NULL {
                self.set_shift_amount(self.cur_box, box_context);
                if abs(self.mode()) == VMODE {
                    if self.pre_adjust_tail != NULL {
                        if PRE_ADJUST_HEAD != self.pre_adjust_tail {
                            self.append_list(PRE_ADJUST_HEAD, self.pre_adjust_tail);
                        }
                        self.pre_adjust_tail = NULL;
                    }
                    self.append_to_vlist(self.cur_box);
                    if self.adjust_tail != NULL {
                        if ADJUST_HEAD != self.adjust_tail {
                            self.append_list(ADJUST_HEAD, self.adjust_tail);
                        }
                        self.adjust_tail = NULL;
                    }
                    if self.mode() > 0 {
                        self.build_page();
                    }
                } else {
                    if abs(self.mode()) == HMODE {
                        self.set_space_factor(1000);
                    } else {
                        let p = self.new_noad();
                        self.set_math_type(self.nucleus(p), SUB_BOX);
                        self.set_info(self.nucleus(p), self.cur_box);
                        self.cur_box = p;
                    }
                    self.set_link(self.tail(), self.cur_box);
                    self.set_tail(self.cur_box);
                }
            }
        } else if box_context < SHIP_OUT_FLAG {
            // §1255 Store cur_box in a box register
            let a;
            if box_context < GLOBAL_BOX_FLAG {
                self.cur_val = box_context - BOX_FLAG;
                a = 0;
            } else {
                self.cur_val = box_context - GLOBAL_BOX_FLAG;
                a = 4;
            }
            if self.cur_val < 256 {
                self.define(a, BOX_BASE + self.cur_val, BOX_REF, self.cur_box);
            } else {
                self.sa_def_box(a);
            }
        } else if self.cur_box != NULL {
            if box_context > SHIP_OUT_FLAG {
                // §1256 Append a new leader node that uses cur_box
                // §430 Get the next non-blank non-relax non-call token
                loop {
                    self.get_x_token();
                    if self.cur_cmd != SPACER && self.cur_cmd != RELAX {
                        break;
                    }
                }
                if (self.cur_cmd == HSKIP && abs(self.mode()) != VMODE)
                    || (self.cur_cmd == VSKIP && abs(self.mode()) == VMODE)
                {
                    self.append_glue();
                    self.set_subtype(self.tail(), box_context - (LEADER_FLAG - A_LEADERS));
                    self.set_leader_ptr(self.tail(), self.cur_box);
                } else {
                    self.print_err_s(b"Leaders not followed by proper glue");
                    self.help(&[
                        b"You should say `\\leaders <box or rule><hskip or vskip>'.",
                        b"I found the <box or rule>, but there's no suitable",
                        b"<hskip or vskip>, so I'm ignoring these leaders.",
                    ]);
                    self.back_error();
                    self.flush_node_list(self.cur_box);
                }
            } else {
                self.ship_out(self.cur_box);
            }
        }
    }

    /// §450 `find_effective_tail_eTeX`: the result is `tx`.
    fn build_find_effective_tail(&self) -> i32 {
        let mut tx = self.tail();
        if !self.is_char_node(tx) && self.ty(tx) == MATH_NODE && self.subtype(tx) == END_M_CODE {
            let mut r = self.head();
            let mut q;
            loop {
                q = r;
                r = self.link(q);
                if r == tx {
                    break;
                }
            }
            tx = q;
        }
        tx
    }

    /// §1258 `fetch_effective_tail_eTeX(#)`.  False is the macro's `#`: `tx` is in a discretionary's replacement.
    fn build_fetch_effective_tail(&mut self, tx: i32) -> bool {
        let mut q = self.head();
        let mut p = NULL;
        let mut r;
        let mut fm;
        loop {
            r = p;
            p = q;
            fm = false;
            if !self.is_char_node(q) {
                if self.ty(q) == DISC_NODE {
                    for _ in 1..=self.replace_count(q) {
                        p = self.link(p);
                    }
                    if p == tx {
                        return false;
                    }
                } else if self.ty(q) == MATH_NODE && self.subtype(q) == BEGIN_M_CODE {
                    fm = true;
                }
            }
            q = self.link(p);
            if q == tx {
                break;
            }
        }
        q = self.link(tx);
        self.set_link(p, q);
        self.set_link(tx, NULL);
        if q == NULL {
            if fm {
                self.confusion(b"tail1");
            } else {
                self.set_tail(p);
            }
        } else if fm {
            self.set_tail(r);
            self.set_link(r, NULL);
            self.flush_node_list(p);
        }
        true
    }

    /// §1257
    pub fn begin_box(&mut self, box_context: i32) {
        match self.cur_chr {
            BOX_CODE => {
                self.scan_register_num();
                self.cur_box = self.fetch_box();
                self.change_box(NULL); // the box becomes void, at the same level
            }
            COPY_CODE => {
                self.scan_register_num();
                let q = self.fetch_box();
                self.cur_box = self.copy_node_list(q);
            }
            LAST_BOX_CODE => {
                // §1258 If the current list ends with a box node, delete it from the list and make cur_box point to
                // it; otherwise set cur_box:=null
                self.cur_box = NULL;
                if abs(self.mode()) == MMODE {
                    self.you_cant();
                    self.help(&[b"Sorry; this \\lastbox will be void."]);
                    self.error();
                } else if self.mode() == VMODE && self.head() == self.tail() {
                    self.you_cant();
                    self.help(&[
                        b"Sorry...I usually can't take things from the current page.",
                        b"This \\lastbox will therefore be void.",
                    ]);
                    self.error();
                } else {
                    let tx = self.build_find_effective_tail();
                    if !self.is_char_node(tx) && (self.ty(tx) == HLIST_NODE || self.ty(tx) == VLIST_NODE) {
                        // §1259 Remove the last box, unless it's part of a discretionary
                        if self.build_fetch_effective_tail(tx) {
                            self.cur_box = tx;
                            self.set_shift_amount(self.cur_box, 0);
                        }
                    }
                }
            }
            VSPLIT_CODE => {
                // §1260 Split off part of a vertical box, make cur_box point to it
                self.scan_register_num();
                let n = self.cur_val;
                if !self.scan_keyword(b"to") {
                    self.print_err_s(b"Missing `to' inserted");
                    self.help(&[
                        b"I'm working on `\\vsplit<box number> to <dimen>';",
                        b"will look for the <dimen> next.",
                    ]);
                    self.error();
                }
                self.scan_normal_dimen();
                self.cur_box = self.vsplit(n, self.cur_val);
            }
            _ => {
                // §1261 Initiate the construction of an hbox or vbox, then return
                let mut k = self.cur_chr - VTOP_CODE;
                self.set_saved(0, box_context);
                if k == HMODE {
                    if box_context < BOX_FLAG && abs(self.mode()) == VMODE {
                        self.scan_spec(ADJUSTED_HBOX_GROUP, true);
                    } else {
                        self.scan_spec(HBOX_GROUP, true);
                    }
                } else {
                    if k == VMODE {
                        self.scan_spec(VBOX_GROUP, true);
                    } else {
                        self.scan_spec(VTOP_GROUP, true);
                        k = VMODE;
                    }
                    self.normal_paragraph();
                }
                self.push_nest();
                self.set_mode(-k);
                if k == VMODE {
                    self.set_prev_depth(self.pdf_ignored_dimen());
                    if self.every_vbox() != NULL {
                        self.begin_token_list(self.every_vbox(), EVERY_VBOX_TEXT);
                    }
                } else {
                    self.set_space_factor(1000);
                    if self.every_hbox() != NULL {
                        self.begin_token_list(self.every_hbox(), EVERY_HBOX_TEXT);
                    }
                }
                return;
            }
        }
        self.box_end(box_context); // in simple cases, we use the box immediately
    }

    /// §1262
    pub fn scan_box(&mut self, box_context: i32) {
        // §430 Get the next non-blank non-relax non-call token
        loop {
            self.get_x_token();
            if self.cur_cmd != SPACER && self.cur_cmd != RELAX {
                break;
            }
        }
        if self.cur_cmd == MAKE_BOX {
            self.begin_box(box_context);
        } else if box_context >= LEADER_FLAG && (self.cur_cmd == HRULE || self.cur_cmd == VRULE) {
            self.cur_box = self.scan_rule_spec();
            self.box_end(box_context);
        } else {
            self.print_err_s(b"A <box> was supposed to be here");
            self.help(&[
                b"I was expecting to see \\hbox or \\vbox or \\copy or \\box or",
                b"something like that. So you might find something missing in",
                b"your output. But keep trying; you can fix this later.",
            ]);
            self.back_error();
        }
    }

    /// §1264
    pub fn package(&mut self, c: i32) {
        let d = self.box_max_depth();
        self.unsave();
        self.save_ptr -= 3;
        if self.mode() == -HMODE {
            self.cur_box = self.hpack(self.link(self.head()), self.saved(2), self.saved(1));
        } else {
            self.cur_box = self.vpackage(self.link(self.head()), self.saved(2), self.saved(1), d);
            if c == VTOP_CODE {
                // §1265 Readjust the height and depth of cur_box, for \vtop
                let mut h = 0;
                let p = self.list_ptr(self.cur_box);
                if p != NULL && self.ty(p) <= RULE_NODE {
                    h = self.height(p);
                }
                // Three box dimensions, each of which can be clamped near 2^31: PORTING section 11.
                let v = self.depth(self.cur_box) as i64 - h as i64 + self.height(self.cur_box) as i64;
                self.set_depth(self.cur_box, sat32(v));
                self.set_height(self.cur_box, h);
            }
        }
        self.pop_nest();
        self.box_end(self.saved(0));
    }

    /// §1269
    pub fn norm_min(&mut self, h: i32) -> i32 {
        if h <= 0 {
            1
        } else if h >= 63 {
            63
        } else {
            h
        }
    }

    /// §1269
    pub fn new_graf(&mut self, indented: bool) {
        self.set_prev_graf(0);
        if self.mode() == VMODE || self.head() != self.tail() {
            let p = self.new_param_glue(PAR_SKIP_CODE);
            self.tail_append(p);
        }
        self.push_nest();
        self.set_mode(HMODE);
        self.set_space_factor(1000);
        self.set_cur_lang();
        self.set_clang(self.cur_lang);
        let l = self.norm_min(self.left_hyphen_min());
        let r = self.norm_min(self.right_hyphen_min());
        self.set_prev_graf((l * 0o100 + r) * 0o200000 + self.cur_lang);
        if indented {
            let p = self.new_null_box();
            self.set_tail(p);
            self.set_link(self.head(), p);
            self.set_width(p, self.par_indent());
            // dropped: src-specials
        }
        if self.every_par() != NULL {
            self.begin_token_list(self.every_par(), EVERY_PAR_TEXT);
        }
        if self.nest_ptr == 1 {
            self.build_page(); // put par_skip glue on current page
        }
    }

    /// §1271
    pub fn indent_in_hmode(&mut self) {
        if self.cur_chr > 0 {
            // \indent
            let mut p = self.new_null_box();
            self.set_width(p, self.par_indent());
            if abs(self.mode()) == HMODE {
                self.set_space_factor(1000);
            } else {
                let q = self.new_noad();
                self.set_math_type(self.nucleus(q), SUB_BOX);
                self.set_info(self.nucleus(q), p);
                p = q;
            }
            self.tail_append(p);
        }
    }

    /// §1273
    pub fn head_for_vmode(&mut self) {
        if self.mode() < 0 {
            if self.cur_cmd != HRULE {
                self.off_save();
            } else {
                self.print_err_s(b"You can't use `");
                self.print_esc_s(b"hrule");
                self.print_s(b"' here except with leaders");
                self.help(&[
                    b"To put a horizontal rule in an hbox or an alignment,",
                    b"you should use \\leaders or \\hrulefill (see The TeXbook).",
                ]);
                self.error();
            }
        } else {
            self.build_insert_par_token();
        }
    }

    /// §1274
    pub fn end_graf(&mut self) {
        if self.mode() == HMODE {
            if self.head() == self.tail() {
                self.pop_nest(); // null paragraphs are ignored
            } else {
                self.line_break(false);
            }
            if self.LR_save() != NULL {
                self.mem_flush_list(self.LR_save());
                self.set_LR_save(NULL);
            }
            self.normal_paragraph();
            self.error_count = 0;
        }
    }

    /// §1277
    pub fn begin_insert_or_adjust(&mut self) {
        if self.cur_cmd == VADJUST {
            self.cur_val = 255;
        } else {
            self.scan_eight_bit_int();
            if self.cur_val == 255 {
                self.print_err_s(b"You can't ");
                self.print_esc_s(b"insert");
                self.print_int(255);
                self.help(&[b"I'm changing to \\insert0; box 255 is special."]);
                self.error();
                self.cur_val = 0;
            }
        }
        self.set_saved(0, self.cur_val);
        if self.cur_cmd == VADJUST && self.scan_keyword(b"pre") {
            self.set_saved(1, 1);
        } else {
            self.set_saved(1, 0);
        }
        self.save_ptr += 2;
        self.new_save_level(INSERT_GROUP);
        self.scan_left_brace();
        self.normal_paragraph();
        self.push_nest();
        self.set_mode(-VMODE);
        self.set_prev_depth(self.pdf_ignored_dimen());
    }

    /// §1279
    pub fn make_mark(&mut self) {
        let c;
        if self.cur_chr == 0 {
            c = 0;
        } else {
            self.scan_register_num();
            c = self.cur_val;
        }
        self.scan_toks(false, true);
        let p = self.get_node(SMALL_NODE_SIZE);
        self.set_mark_class(p, c);
        self.set_ty(p, MARK_NODE);
        self.set_subtype(p, 0); // the subtype is not used
        self.set_mark_ptr(p, self.def_ref);
        self.set_link(self.tail(), p);
        self.set_tail(p);
    }

    /// §1281
    pub fn append_penalty(&mut self) {
        self.scan_int();
        let p = self.new_penalty(self.cur_val);
        self.tail_append(p);
        if self.mode() == VMODE {
            self.build_page();
        }
    }

    /// §1283
    pub fn delete_last(&mut self) {
        if self.mode() == VMODE && self.tail() == self.head() {
            // §1284 Apologize for inability to do the operation now, unless \unskip follows non-glue
            if self.cur_chr != GLUE_NODE || self.last_glue != MAX_HALFWORD {
                self.you_cant();
                self.help(&[
                    b"Sorry...I usually can't take things from the current page.",
                    b"Try `I\\vskip-\\lastskip' instead.",
                ]);
                if self.cur_chr == KERN_NODE {
                    self.set_help_line(0, b"Try `I\\kern-\\lastkern' instead.");
                } else if self.cur_chr != GLUE_NODE {
                    self.set_help_line(0, b"Perhaps you can make the output routine do it.");
                }
                self.error();
            }
        } else {
            let tx = self.build_find_effective_tail();
            if !self.is_char_node(tx) && self.ty(tx) == self.cur_chr {
                if !self.build_fetch_effective_tail(tx) {
                    return;
                }
                self.flush_node_list(tx);
            }
        }
    }

    /// §1288
    pub fn unpackage(&mut self) {
        'done: {
            if self.cur_chr > COPY_CODE {
                // §1863 Handle saved items and goto done
                self.set_link(self.tail(), self.disc_ptr[self.cur_chr]);
                self.disc_ptr[self.cur_chr] = NULL;
                break 'done;
            }
            let c = self.cur_chr;
            self.scan_register_num();
            let p = self.fetch_box();
            if p == NULL {
                return;
            }
            if abs(self.mode()) == MMODE
                || (abs(self.mode()) == VMODE && self.ty(p) != VLIST_NODE)
                || (abs(self.mode()) == HMODE && self.ty(p) != HLIST_NODE)
            {
                self.print_err_s(b"Incompatible list can't be unboxed");
                self.help(&[
                    b"Sorry, Pandora. (You sneaky devil.)",
                    b"I refuse to unbox an \\hbox in vertical mode or vice versa.",
                    b"And I can't open any boxes in math mode.",
                ]);
                self.error();
                return;
            }
            if c == COPY_CODE {
                let q = self.copy_node_list(self.list_ptr(p));
                self.set_link(self.tail(), q);
            } else {
                self.set_link(self.tail(), self.list_ptr(p));
                self.change_box(NULL);
                self.free_node(p, BOX_NODE_SIZE);
            }
        }
        // done:
        while self.link(self.tail()) != NULL {
            let r = self.link(self.tail());
            if !self.is_char_node(r) && self.ty(r) == MARGIN_KERN_NODE {
                self.set_link(self.tail(), self.link(r));
                self.free_avail(self.margin_char(r));
                self.free_node(r, MARGIN_KERN_NODE_SIZE);
            }
            self.set_tail(self.link(self.tail()));
        }
    }

    /// §1291
    pub fn append_italic_correction(&mut self) {
        if self.tail() != self.head() {
            let p;
            if self.is_char_node(self.tail()) {
                p = self.tail();
            } else if self.ty(self.tail()) == LIGATURE_NODE {
                p = self.lig_char(self.tail());
            } else {
                return;
            }
            let f = self.font(p);
            let k = self.new_kern(self.char_italic(f, self.char_info(f, self.character(p))));
            self.tail_append(k);
            self.set_subtype(self.tail(), EXPLICIT);
        }
    }

    /// §1295
    pub fn append_discretionary(&mut self) {
        let d = self.new_disc();
        self.tail_append(d);
        if self.cur_chr == 1 {
            let c = self.hyphen_char[self.cur_font()];
            if c >= 0 && c < 256 {
                let pre_kern = self.get_auto_kern(self.cur_font(), NON_CHAR, c);
                let app_kern = self.get_auto_kern(self.cur_font(), c, NON_CHAR);
                let c_node = self.new_character(self.cur_font(), c);
                if app_kern == NULL && pre_kern == NULL {
                    // no auto-kern
                    self.set_pre_break(self.tail(), c_node);
                } else {
                    if pre_kern == NULL {
                        self.set_pre_break(self.tail(), c_node);
                    } else {
                        self.set_pre_break(self.tail(), pre_kern);
                        self.set_link(pre_kern, c_node);
                    }
                    if app_kern != NULL {
                        self.set_link(c_node, app_kern);
                    }
                }
            }
        } else {
            self.save_ptr += 1;
            self.set_saved(-1, 0);
            self.new_save_level(DISC_GROUP);
            self.scan_left_brace();
            self.push_nest();
            self.set_mode(-HMODE);
            self.set_space_factor(1000);
        }
    }

    /// §1297
    pub fn build_discretionary(&mut self) {
        self.unsave();
        // §1299 Prune the current list, if necessary, until it contains only char_node, kern_node, hlist_node,
        // vlist_node, rule_node, and ligature_node items; set n to the length of the list, and set q to the list's tail
        let mut q = self.head();
        let mut p = self.link(q);
        let mut n = 0;
        while p != NULL {
            if !self.is_char_node(p)
                && self.ty(p) > RULE_NODE
                && self.ty(p) != KERN_NODE
                && self.ty(p) != LIGATURE_NODE
            {
                self.print_err_s(b"Improper discretionary list");
                self.help(&[b"Discretionary lists must contain only boxes and kerns."]);
                self.error();
                self.begin_diagnostic();
                self.print_nl_s(b"The following discretionary sublist has been deleted:");
                self.show_box(p);
                self.end_diagnostic(true);
                self.flush_node_list(p);
                self.set_link(q, NULL);
                break; // goto done
            }
            q = p;
            p = self.link(q);
            n += 1;
        }
        // done:
        p = self.link(self.head());
        self.pop_nest();
        match self.saved(-1) {
            0 => self.set_pre_break(self.tail(), p),
            1 => self.set_post_break(self.tail(), p),
            2 => {
                // §1298 Attach list p to the current list, and record its length; then finish up and return
                if n > 0 && abs(self.mode()) == MMODE {
                    self.print_err_s(b"Illegal math ");
                    self.print_esc_s(b"discretionary");
                    self.help(&[
                        b"Sorry: The third part of a discretionary break must be",
                        b"empty, in math formulas. I had to delete your third part.",
                    ]);
                    self.flush_node_list(p);
                    n = 0;
                    self.error();
                } else {
                    self.set_link(self.tail(), p);
                }
                if n <= WEB_MAX_QUARTERWORD {
                    self.set_replace_count(self.tail(), n);
                } else {
                    self.print_err_s(b"Discretionary list is too long");
                    self.help(&[
                        b"Wow---I never thought anybody would tweak me here.",
                        b"You can't seriously need such a huge discretionary list?",
                    ]);
                    self.error();
                }
                if n > 0 {
                    self.set_tail(q);
                }
                self.save_ptr -= 1;
                return;
            }
            _ => {} // there are no other cases
        }
        self.set_saved(-1, self.saved(-1) + 1);
        self.new_save_level(DISC_GROUP);
        self.scan_left_brace();
        self.push_nest();
        self.set_mode(-HMODE);
        self.set_space_factor(1000);
    }

    /// §1301
    pub fn make_accent(&mut self) {
        self.scan_char_num();
        let mut f = self.cur_font();
        let mut p = self.new_character(f, self.cur_val);
        if p != NULL {
            let x = self.x_height(f);
            let s = self.slant(f) as f64 / 65536.0;
            let a = self.char_width(f, self.char_info(f, self.character(p)));
            self.do_assignments();
            // §1302 Create a character node q for the next character, but set q:=null if problems arise
            let mut q = NULL;
            f = self.cur_font();
            if self.cur_cmd == LETTER || self.cur_cmd == OTHER_CHAR || self.cur_cmd == CHAR_GIVEN {
                q = self.new_character(f, self.cur_chr);
            } else if self.cur_cmd == CHAR_NUM {
                self.scan_char_num();
                q = self.new_character(f, self.cur_val);
            } else {
                self.back_input();
            }
            if q != NULL {
                // §1303 Append the accent with appropriate kerns, then set p:=q
                let t = self.slant(f) as f64 / 65536.0;
                let i = self.char_info(f, self.character(q));
                let w = self.char_width(f, i);
                let h = self.char_height(f, self.height_depth(i));
                if h != x {
                    // the accent must be shifted up or down
                    p = self.hpack_natural(p);
                    self.set_shift_amount(p, sat32(x as i64 - h as i64));
                }
                let delta = round((w as i64 - a as i64) as f64 / 2.0 + h as f64 * t - x as f64 * s);
                let r = self.new_kern(delta);
                self.set_subtype(r, ACC_KERN);
                self.set_link(self.tail(), r);
                self.set_link(r, p);
                // delta is clamped at the limit of an i32, and a is added to it: PORTING section 11.
                let k = self.new_kern(sat32(-(a as i64) - delta as i64));
                self.set_tail(k);
                self.set_subtype(k, ACC_KERN);
                self.set_link(p, k);
                p = q;
            }
            self.set_link(self.tail(), p);
            self.set_tail(p);
            self.set_space_factor(1000);
        }
    }

    /// §1305
    #[cold]
    pub fn align_error(&mut self) {
        if abs(self.align_state) > 2 {
            // §1306 Express consternation over the fact that no alignment is in progress
            self.print_err_s(b"Misplaced ");
            self.print_cmd_chr(self.cur_cmd, self.cur_chr);
            if self.cur_tok == TAB_TOKEN + b'&' as i32 {
                self.help(&[
                    b"I can't figure out why you would want to use a tab mark",
                    b"here. If you just want an ampersand, the remedy is",
                    b"simple: Just type `I\\&' now. But if some right brace",
                    b"up above has ended a previous alignment prematurely,",
                    b"you're probably due for more error messages, and you",
                    b"might try typing `S' now just to see what is salvageable.",
                ]);
            } else {
                self.help(&[
                    b"I can't figure out why you would want to use a tab mark",
                    b"or \\cr or \\span just now. If something like a right brace",
                    b"up above has ended a previous alignment prematurely,",
                    b"you're probably due for more error messages, and you",
                    b"might try typing `S' now just to see what is salvageable.",
                ]);
            }
            self.error();
        } else {
            self.back_input();
            if self.align_state < 0 {
                self.print_err_s(b"Missing { inserted");
                self.align_state += 1;
                self.cur_tok = LEFT_BRACE_TOKEN + b'{' as i32;
            } else {
                self.print_err_s(b"Missing } inserted");
                self.align_state -= 1;
                self.cur_tok = RIGHT_BRACE_TOKEN + b'}' as i32;
            }
            self.help(&[
                b"I've put in what seems to be necessary to fix",
                b"the current column of the current alignment.",
                b"Try to go on, since this might almost work.",
            ]);
            self.ins_error();
        }
    }

    /// §1307
    #[cold]
    pub fn no_align_error(&mut self) {
        self.print_err_s(b"Misplaced ");
        self.print_esc_s(b"noalign");
        self.help(&[
            b"I expect to see \\noalign only after the \\cr of",
            b"an alignment. Proceed, and I'll ignore this case.",
        ]);
        self.error();
    }

    /// §1307
    #[cold]
    pub fn omit_error(&mut self) {
        self.print_err_s(b"Misplaced ");
        self.print_esc_s(b"omit");
        self.help(&[
            b"I expect to see \\omit only after tab marks or the \\cr of",
            b"an alignment. Proceed, and I'll ignore this case.",
        ]);
        self.error();
    }

    /// §1309
    pub fn do_endv(&mut self) {
        self.base_ptr = self.input_ptr;
        self.input_stack[self.base_ptr] = self.cur_input;
        let mut r = self.input_stack[self.base_ptr];
        while r.index_field != V_TEMPLATE && self.rec_at_list_end(r) {
            self.base_ptr -= 1;
            r = self.input_stack[self.base_ptr];
        }
        if r.index_field != V_TEMPLATE || !self.rec_at_list_end(r) {
            self.fatal_error(b"(interwoven alignment preambles are not allowed)");
        }
        if self.cur_group == ALIGN_GROUP {
            self.end_graf();
            if self.fin_col() {
                self.fin_row();
            }
        } else {
            self.off_save();
        }
    }

    /// §1313
    #[cold]
    pub fn cs_error(&mut self) {
        self.print_err_s(b"Extra ");
        self.print_esc_s(b"endcsname");
        self.help(&[b"I'm ignoring this, since I wasn't doing a \\csname."]);
        self.error();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::save::testkit::*;
    use crate::toks::testkit::*;

    fn tex() -> &'static mut Tex {
        let t = table_machine();
        print_setup(t);
        t.interaction = BATCH_MODE; // so that help lines go to the terminal buffer with the rest
        t
    }

    fn nodes(t: &Tex, mut p: i32) -> Vec<i32> {
        let mut v = Vec::new();
        while p != NULL {
            v.push(p);
            p = t.link(p);
        }
        v
    }

    fn after_head(t: &Tex) -> Vec<i32> {
        nodes(t, t.link(t.head()))
    }

    /// A new list at nest_ptr>0, so that nothing calls the page builder.
    fn enter(t: &mut Tex, mode: i32) {
        t.push_nest();
        t.set_mode(mode);
    }

    /// The tokens waiting on the current input level.
    fn level(t: &Tex) -> Vec<i32> {
        let mut v = Vec::new();
        let mut i = t.loc();
        while t.toks[i] != 0 {
            v.push(t.toks[i]);
            i += 1;
        }
        v
    }

    fn assert_message(key: &str, out: &str) {
        let rows = table(include_str!("../testdata/build_messages_oracle.tsv"));
        let r = rows.iter().find(|r| r[0] == key).unwrap();
        assert!(out.starts_with(&r[1]), "{key}: {out:?} does not begin with {:?}", r[1]);
        let help = format!("{}\n\n", r[2]);
        assert!(out.ends_with(&help), "{key}: {out:?} does not end with {help:?}");
    }

    #[test]
    fn norm_min_keeps_to_one_and_sixty_three() {
        let t = machine();
        for (h, want) in [(-5, 1), (0, 1), (1, 1), (2, 2), (62, 62), (63, 63), (64, 63), (1000000, 63)] {
            assert_eq!(t.norm_min(h), want, "norm_min({h})");
        }
        Tex::dealloc(t);
    }

    #[test]
    fn prev_graf_of_a_new_paragraph_against_the_oracle() {
        for r in table(include_str!("../testdata/build_prevgraf_oracle.tsv")) {
            let t = tex();
            t.set_left_hyphen_min(r[0].parse().unwrap());
            t.set_right_hyphen_min(r[1].parse().unwrap());
            t.set_language(r[2].parse().unwrap());
            enter(t, -VMODE);
            t.new_graf(false);
            assert_eq!(t.mode(), HMODE);
            assert_eq!(t.space_factor(), 1000);
            let pg = t.prev_graf();
            let got = format!("(language{}:hyphenmin{},{})", pg % 0o200000, pg / 0o20000000, (pg / 0o200000) % 0o100);
            assert_eq!(got, r[3], "{r:?}");
            assert_eq!(t.clang(), pg % 0o200000);
            Tex::dealloc(t);
        }
    }

    #[test]
    fn new_graf_appends_parskip_and_the_indent_box() {
        let t = tex();
        enter(t, -VMODE);
        // Internal vertical, empty: \parskip only when the list has something.
        t.new_graf(true);
        assert_eq!(t.nest[t.nest_ptr - 1].tail_field, t.nest[t.nest_ptr - 1].head_field);
        let b = t.link(t.head());
        assert_eq!((t.tail(), t.ty(b)), (b, HLIST_NODE));
        Tex::dealloc(t);

        let t = tex();
        enter(t, -VMODE);
        t.set_mode(VMODE); // the test that goes by mode, with a nest_ptr that is not 1
        t.set_par_indent(5 * 65536);
        t.new_graf(true);
        let outer = t.nest[t.nest_ptr - 1];
        assert_eq!(t.ty(outer.tail_field), GLUE_NODE);
        assert_eq!(t.subtype(outer.tail_field), PAR_SKIP_CODE + 1);
        assert_eq!(t.width(t.tail()), 5 * 65536);
        Tex::dealloc(t);

        let t = tex();
        enter(t, -VMODE);
        t.new_graf(false);
        assert_eq!(t.head(), t.tail());
        Tex::dealloc(t);
    }

    #[test]
    fn off_save_at_the_bottom_says_extra() {
        let t = tex();
        t.cur_group = BOTTOM_LEVEL;
        t.cur_cmd = END_GROUP;
        t.cur_chr = 0;
        let before = t.input_ptr;
        t.off_save();
        assert_message("extra_endgroup", &printed(t));
        assert_eq!(t.input_ptr, before, "the token is dropped and nothing goes in");
        Tex::dealloc(t);
    }

    #[test]
    fn off_save_inserts_what_matches_the_group() {
        for (key, group, want) in [
            ("missing_right_brace", SIMPLE_GROUP, vec![RIGHT_BRACE_TOKEN + b'}' as i32]),
            ("missing_endgroup", SEMI_SIMPLE_GROUP, vec![CS_TOKEN_FLAG + FROZEN_END_GROUP]),
            ("missing_dollar", MATH_SHIFT_GROUP, vec![MATH_SHIFT_TOKEN + b'$' as i32]),
            ("missing_right_dot", MATH_LEFT_GROUP, vec![CS_TOKEN_FLAG + FROZEN_RIGHT, OTHER_TOKEN + b'.' as i32]),
        ] {
            let t = tex();
            t.cur_group = group;
            t.cur_cmd = END_GROUP;
            t.cur_tok = CS_TOKEN_FLAG + cs(b'x');
            t.off_save();
            assert_message(key, &printed(t));
            assert_eq!(t.token_type(), INSERTED, "{key}");
            assert_eq!(level(t), want, "{key}");
            // The token is on the level below, to be read again after what was inserted.
            let below = t.input_stack[t.input_ptr - 1];
            assert_eq!(below.index_field, BACKED_UP, "{key}");
            assert_eq!(t.toks[below.loc_field], CS_TOKEN_FLAG + cs(b'x'), "{key}");
            Tex::dealloc(t);
        }
    }

    #[test]
    fn too_many_and_extra_right_braces() {
        for (key, group, delta) in [
            ("too_many", BOTTOM_LEVEL, 0),
            ("extra_endgroup_brace", SEMI_SIMPLE_GROUP, 1),
            ("extra_dollar_brace", MATH_SHIFT_GROUP, 1),
            ("extra_right_brace", MATH_LEFT_GROUP, 1),
        ] {
            let t = tex();
            t.cur_group = group;
            t.align_state = 7;
            t.handle_right_brace();
            assert_message(key, &printed(t));
            assert_eq!(t.align_state, 7 + delta, "{key}");
            Tex::dealloc(t);
        }
    }

    #[test]
    fn a_right_brace_of_a_simple_group_only_ends_it() {
        let t = tex();
        t.cur_tok = RIGHT_BRACE_TOKEN + b'}' as i32;
        t.new_save_level(SIMPLE_GROUP);
        assert_eq!(t.cur_group, SIMPLE_GROUP);
        t.handle_right_brace();
        assert_eq!(t.cur_group, BOTTOM_LEVEL);
        assert_eq!(printed(t), "");
        Tex::dealloc(t);
    }

    #[test]
    fn missing_cr_in_an_alignment() {
        let t = tex();
        t.cur_group = ALIGN_GROUP;
        t.cur_tok = RIGHT_BRACE_TOKEN + b'}' as i32;
        t.handle_right_brace();
        assert_message("missing_cr", &printed(t));
        // ins_error: the frozen \cr is what is read first, then the brace.
        assert_eq!(level(t), vec![CS_TOKEN_FLAG + FROZEN_CR]);
        assert_eq!(t.token_type(), INSERTED);
        assert_eq!(t.toks[t.input_stack[t.input_ptr - 1].loc_field], RIGHT_BRACE_TOKEN + b'}' as i32);
        Tex::dealloc(t);
    }

    #[test]
    fn partoken_context_decides_at_a_brace() {
        // (group, smallest context at which a paragraph is ended by \par's token)
        for (group, least) in
            [(VBOX_GROUP, 1), (VTOP_GROUP, 1), (VCENTER_GROUP, 1), (INSERT_GROUP, 2), (OUTPUT_GROUP, 2), (NO_ALIGN_GROUP, 2)]
        {
            let t = tex();
            enter(t, HMODE);
            t.cur_group = group;
            t.set_partoken_context(least);
            t.cur_tok = RIGHT_BRACE_TOKEN + b'}' as i32;
            t.handle_right_brace();
            assert_eq!(t.token_type(), INSERTED, "group {group}");
            assert_eq!(level(t), vec![t.par_token], "group {group}");
            assert_eq!(t.toks[t.input_stack[t.input_ptr - 1].loc_field], RIGHT_BRACE_TOKEN + b'}' as i32, "group {group}");
            Tex::dealloc(t);
        }
    }

    #[test]
    fn head_for_vmode_inserts_par_unless_restricted() {
        let t = tex();
        enter(t, HMODE);
        t.cur_cmd = VSKIP;
        t.cur_tok = CS_TOKEN_FLAG + cs(b'v');
        t.head_for_vmode();
        assert_eq!(level(t), vec![t.par_token]);
        assert_eq!(t.token_type(), INSERTED);
        assert_eq!(t.toks[t.input_stack[t.input_ptr - 1].loc_field], CS_TOKEN_FLAG + cs(b'v'));
        assert_eq!(printed(t), "");
        Tex::dealloc(t);

        let t = tex();
        enter(t, -HMODE);
        t.cur_cmd = HRULE;
        t.head_for_vmode();
        assert_message("hrule_in_hbox", &printed(t));
        Tex::dealloc(t);

        let t = tex();
        enter(t, -HMODE);
        t.cur_group = HBOX_GROUP;
        t.cur_cmd = VSKIP;
        t.head_for_vmode();
        assert_message("missing_right_brace", &printed(t));
        Tex::dealloc(t);
    }

    #[test]
    fn glue_of_the_four_fixed_kinds_takes_a_reference() {
        for (s, spec) in [(FIL_CODE, FIL_GLUE), (FILL_CODE, FILL_GLUE), (SS_CODE, SS_GLUE), (FIL_NEG_CODE, FIL_NEG_GLUE)] {
            let t = tex();
            enter(t, -HMODE);
            t.cur_chr = s;
            let before = t.glue_ref_count(spec);
            t.append_glue();
            assert_eq!(t.glue_ptr(t.tail()), spec);
            assert_eq!(t.subtype(t.tail()), NORMAL);
            assert_eq!(t.glue_ref_count(spec), before + 1);
            assert_eq!(t.cur_val, spec);
            Tex::dealloc(t);
        }
    }

    #[test]
    fn leaders_need_proper_glue() {
        for (mode, cmd, chr, want_glue) in [
            (-HMODE, HSKIP, FIL_CODE, true),
            (-VMODE, VSKIP, FIL_CODE, true),
            (-MMODE, HSKIP, FILL_CODE, true),
            (-HMODE, VSKIP, FIL_CODE, false),
            (-VMODE, HSKIP, FIL_CODE, false),
        ] {
            for (extra, sub) in [(0, A_LEADERS), (1, A_LEADERS + 1), (2, A_LEADERS + 2)] {
                let t = tex();
                enter(t, mode);
                t.set_eq_type(cs(b'g'), cmd);
                t.set_equiv(cs(b'g'), chr);
                t.set_eq_type(cs(b'r'), RELAX);
                // A space and a \relax are passed over, as in the module that finds the glue.
                feed(t, &[SPACE_TOKEN, cs_tok(b'r'), cs_tok(b'g')]);
                let r = t.new_rule();
                t.cur_box = r;
                t.box_end(LEADER_FLAG + extra);
                if want_glue {
                    assert_eq!((t.ty(t.tail()), t.subtype(t.tail()), t.leader_ptr(t.tail())), (GLUE_NODE, sub, r));
                    assert_eq!(printed(t), "");
                } else {
                    assert_eq!(t.head(), t.tail());
                    assert_message("leaders", &printed(t));
                    assert_eq!(t.toks[t.loc()], cs_tok(b'g'), "the token is read again");
                }
                Tex::dealloc(t);
            }
        }
    }

    #[test]
    fn scan_box_refuses_other_things() {
        let t = tex();
        t.set_eq_type(cs(b'r'), RELAX);
        feed(t, &[SPACE_TOKEN, cs_tok(b'r'), LETTER_TOKEN + b'x' as i32]);
        t.scan_box(BOX_FLAG);
        assert_message("no_box", &printed(t));
        assert_eq!(t.toks[t.loc()], LETTER_TOKEN + b'x' as i32);
        Tex::dealloc(t);
    }

    #[test]
    fn box_end_by_range_of_context() {
        // A shift in hmode: the list gets it, and the space factor is 1000.
        let t = tex();
        enter(t, -HMODE);
        t.set_space_factor(3000);
        let b = t.new_null_box();
        t.cur_box = b;
        t.box_end(77);
        assert_eq!((t.tail(), t.shift_amount(b), t.space_factor()), (b, 77, 1000));
        Tex::dealloc(t);
        // A local and a global register: the last in box(n) range.
        let t = tex();
        t.new_save_level(SIMPLE_GROUP);
        let b = t.new_null_box();
        t.cur_box = b;
        t.box_end(BOX_FLAG + 255);
        assert_eq!(t.box_reg(255), b);
        assert_eq!(t.eq_level(BOX_BASE + 255), LEVEL_ONE + 1);
        let c = t.new_null_box();
        t.cur_box = c;
        t.box_end(GLOBAL_BOX_FLAG + 3);
        assert_eq!(t.box_reg(3), c);
        assert_eq!(t.eq_level(BOX_BASE + 3), LEVEL_ONE);
        Tex::dealloc(t);
        // cur_box=null does nothing in the shift range, or at leaders, or at shipping out.
        let t = tex();
        enter(t, -HMODE);
        t.cur_box = NULL;
        t.box_end(5);
        t.box_end(SHIP_OUT_FLAG);
        t.box_end(LEADER_FLAG);
        assert_eq!(t.head(), t.tail());
        Tex::dealloc(t);
    }

    #[test]
    fn lastbox_refused_in_math_and_on_the_page() {
        let t = tex();
        enter(t, -MMODE);
        t.cur_cmd = MAKE_BOX;
        t.cur_chr = LAST_BOX_CODE;
        let n = t.new_null_box();
        t.tail_append(n);
        t.begin_box(0);
        assert_message("lastbox_math", &printed(t));
        assert_eq!((t.tail(), t.cur_box), (n, NULL));
        Tex::dealloc(t);

        let t = tex();
        t.cur_cmd = MAKE_BOX;
        t.cur_chr = LAST_BOX_CODE;
        t.begin_box(0);
        assert_message("lastbox_page", &printed(t));
        assert_eq!(t.cur_box, NULL);
        Tex::dealloc(t);
    }

    /// `\lastbox` in restricted horizontal mode, with cur_box left as a register would take it.
    fn lastbox(t: &mut Tex) -> i32 {
        t.cur_chr = LAST_BOX_CODE;
        t.begin_box(BOX_FLAG + 1);
        t.box_reg(1)
    }

    #[test]
    fn lastbox_takes_a_box_off_and_clears_its_shift() {
        let t = tex();
        enter(t, -HMODE);
        let (a, b) = (t.new_null_box(), t.new_null_box());
        t.tail_append(a);
        t.tail_append(b);
        t.set_shift_amount(b, 500);
        assert_eq!(lastbox(t), b);
        assert_eq!((t.shift_amount(b), t.link(b), t.tail(), t.link(a)), (0, NULL, a, NULL));
        // A kern is not a box, and an empty list has nothing.
        let k = t.new_kern(1);
        t.tail_append(k);
        assert_eq!(lastbox(t), NULL);
        assert_eq!(t.tail(), k);
        Tex::dealloc(t);
    }

    /// The box inside a discretionary's replacement: the tail may not be taken from there.
    #[test]
    fn lastbox_leaves_a_box_in_a_replacement() {
        let t = tex();
        enter(t, -HMODE);
        let d = t.new_disc();
        t.tail_append(d);
        let b = t.new_null_box();
        t.set_replace_count(d, 1);
        t.set_link(d, b);
        t.set_tail(b);
        assert_eq!(lastbox(t), NULL);
        assert_eq!((t.tail(), t.link(d)), (b, b));
        // With a node after the replacement the box is free.
        let c = t.new_null_box();
        t.tail_append(c);
        assert_eq!(lastbox(t), c);
        assert_eq!((t.tail(), t.link(b)), (b, NULL));
        // And then the box is in the replacement again.
        assert_eq!(lastbox(t), NULL);
        Tex::dealloc(t);
    }

    #[test]
    fn delete_last_apologizes_on_an_empty_page_list() {
        for (key, chr) in [("unskip", GLUE_NODE), ("unkern", KERN_NODE), ("unpenalty", PENALTY_NODE)] {
            let t = tex();
            t.last_glue = ZERO_GLUE;
            t.cur_cmd = REMOVE_ITEM;
            t.cur_chr = chr;
            t.delete_last();
            assert_message(key, &printed(t));
            Tex::dealloc(t);
        }
        // \unskip says nothing while last_glue is max_halfword.
        let t = tex();
        t.last_glue = MAX_HALFWORD;
        t.cur_chr = GLUE_NODE;
        t.delete_last();
        assert_eq!(printed(t), "");
        Tex::dealloc(t);
    }

    #[test]
    fn delete_last_by_type_of_the_tail() {
        let t = tex();
        enter(t, -HMODE);
        // tail=head is not refused here, and head is a character node.
        t.cur_chr = KERN_NODE;
        t.delete_last();
        assert_eq!(t.tail(), t.head());
        let (k, p) = (t.new_kern(1), t.new_penalty(2));
        t.tail_append(k);
        t.tail_append(p);
        t.cur_chr = KERN_NODE;
        t.delete_last();
        assert_eq!(t.tail(), p, "the tail is a penalty");
        t.cur_chr = PENALTY_NODE;
        t.delete_last();
        assert_eq!((t.tail(), t.link(k)), (k, NULL));
        t.cur_chr = KERN_NODE;
        t.delete_last();
        assert_eq!((t.tail(), t.link(t.head())), (t.head(), NULL));
        // A character at the tail is left.
        let c = t.fast_get_avail();
        t.set_link(t.tail(), c);
        t.set_tail(c);
        t.delete_last();
        assert_eq!(t.tail(), c);
        Tex::dealloc(t);
    }

    /// Effective tails: the display line of the case lastbox_etex.tex: begin_M, a kern, end_M.
    fn m_list(t: &mut Tex, before: bool) -> (i32, i32, i32, i32) {
        enter(t, -HMODE);
        let x = if before {
            let x = t.new_penalty(1);
            t.tail_append(x);
            x
        } else {
            NULL
        };
        let (b, k, e) = (t.new_math(0, BEGIN_M_CODE), t.new_kern(5), t.new_math(0, END_M_CODE));
        t.tail_append(b);
        t.tail_append(k);
        t.tail_append(e);
        (x, b, k, e)
    }

    #[test]
    fn delete_last_looks_past_end_m_and_removes_a_lone_pair() {
        let t = tex();
        m_list(t, false);
        t.cur_chr = KERN_NODE;
        t.delete_last();
        assert_eq!(t.tail(), t.head(), "begin_M and end_M go with the kern");
        assert_eq!(t.link(t.head()), NULL);
        Tex::dealloc(t);

        let t = tex();
        let (x, _, _, _) = m_list(t, true);
        t.cur_chr = KERN_NODE;
        t.delete_last();
        assert_eq!((t.tail(), t.link(x)), (x, NULL));
        Tex::dealloc(t);

        // The tail's own type is not looked at: an \unskip finds a kern before end_M, and does nothing.
        let t = tex();
        let (_, b, k, e) = m_list(t, false);
        t.cur_chr = GLUE_NODE;
        t.delete_last();
        assert_eq!(after_head(t), vec![b, k, e]);
        Tex::dealloc(t);

        // With no begin_M just before it, only the item goes.
        let t = tex();
        enter(t, -HMODE);
        let (k, e) = (t.new_kern(5), t.new_math(0, END_M_CODE));
        t.tail_append(k);
        t.tail_append(e);
        t.cur_chr = KERN_NODE;
        t.delete_last();
        assert_eq!(after_head(t), vec![e]);
        assert_eq!(t.tail(), e);
        Tex::dealloc(t);

        // A math node that is not end_M is the tail itself.
        let t = tex();
        enter(t, -HMODE);
        let (k, e) = (t.new_kern(5), t.new_math(0, END_L_CODE));
        t.tail_append(k);
        t.tail_append(e);
        t.cur_chr = KERN_NODE;
        t.delete_last();
        assert_eq!(after_head(t), vec![k, e]);
        Tex::dealloc(t);
    }

    #[test]
    fn discards_are_appended_and_margin_kerns_removed() {
        let t = tex();
        enter(t, -VMODE);
        let (a, m, b) = (t.new_kern(1), t.get_node(MARGIN_KERN_NODE_SIZE), t.new_penalty(2));
        t.set_ty(m, MARGIN_KERN_NODE);
        let c = t.fast_get_avail();
        t.set_margin_char(m, c);
        t.set_link(a, m);
        t.set_link(m, b);
        t.disc_ptr[LAST_BOX_CODE] = a;
        t.cur_chr = LAST_BOX_CODE;
        let used = t.var_used;
        t.unpackage();
        assert_eq!(t.disc_ptr[LAST_BOX_CODE], NULL);
        assert_eq!(after_head(t), vec![a, b]);
        assert_eq!(t.tail(), b);
        assert_eq!(t.var_used, used - MARGIN_KERN_NODE_SIZE);
        assert_eq!(t.avail, c, "the margin character is back on the avail stack");
        Tex::dealloc(t);
    }

    /// A font whose only data are a character with italic correction and one without.
    fn font_for_italics(t: &mut Tex) -> i32 {
        let f = 1;
        t.char_base[f] = 10;
        t.italic_base[f] = 400;
        t.font_info[400].set_sc(0);
        t.font_info[401].set_sc(65536);
        t.font_info[10 + b'f' as i32].set_qqqq(FourQuarters { b0: 1, b1: 0, b2: 4, b3: 0 });
        t.font_info[10 + b'x' as i32].set_qqqq(FourQuarters { b0: 1, b1: 0, b2: 0, b3: 0 });
        f
    }

    #[test]
    fn italic_correction_by_what_is_at_the_tail() {
        let t = tex();
        enter(t, -HMODE);
        let f = font_for_italics(t);
        t.append_italic_correction(); // head=tail
        assert_eq!(t.head(), t.tail());
        let c = t.fast_get_avail();
        t.set_font(c, f);
        t.set_character(c, b'f' as i32);
        t.set_link(t.tail(), c);
        t.set_tail(c);
        t.append_italic_correction();
        assert_eq!((t.ty(t.tail()), t.subtype(t.tail()), t.width(t.tail())), (KERN_NODE, EXPLICIT, 65536));
        // After a kern: nothing.
        let k = t.tail();
        t.append_italic_correction();
        assert_eq!(t.tail(), k);
        // After a ligature: from its lig_char, and zero for a character without.
        let l = t.new_ligature(f, b'x' as i32, NULL);
        t.tail_append(l);
        t.append_italic_correction();
        assert_eq!((t.ty(t.tail()), t.width(t.tail())), (KERN_NODE, 0));
        Tex::dealloc(t);
    }

    fn enter_disc(t: &mut Tex, part: i32) -> i32 {
        let d = t.new_disc();
        t.tail_append(d);
        t.save_ptr += 1;
        t.set_saved(-1, part);
        t.new_save_level(DISC_GROUP);
        enter(t, -HMODE);
        d
    }

    fn chars(t: &mut Tex, n: i32) {
        for _ in 0..n {
            let c = t.fast_get_avail();
            t.set_font(c, NULL_FONT);
            t.set_character(c, b'a' as i32);
            t.set_link(t.tail(), c);
            t.set_tail(c);
        }
    }

    #[test]
    fn third_part_records_its_length_up_to_the_quarterword() {
        for (n, ok) in [(0, true), (1, true), (254, true), (255, true), (256, false)] {
            let t = tex();
            enter(t, -HMODE);
            let sp = t.save_ptr;
            let d = enter_disc(t, 2);
            chars(t, n);
            let last = t.tail();
            t.handle_right_brace();
            assert_eq!(t.save_ptr, sp, "n={n}");
            assert_eq!(t.replace_count(d), if ok { n } else { 0 }, "n={n}");
            assert_eq!(t.tail(), if n > 0 { last } else { d }, "n={n}");
            let out = printed(t);
            if ok {
                assert_eq!(out, "", "n={n}");
            } else {
                assert!(out.starts_with("! Discretionary list is too long."), "{out:?}");
                assert!(out.ends_with(
                    "Wow---I never thought anybody would tweak me here.\nYou can't seriously need such a huge discretionary list?\n\n"
                ));
            }
            Tex::dealloc(t);
        }
    }

    #[test]
    fn third_part_is_refused_in_math() {
        let t = tex();
        enter(t, -MMODE);
        let d = enter_disc(t, 2);
        chars(t, 2);
        t.handle_right_brace();
        assert_message("illegal_math_disc", &printed(t));
        assert_eq!((t.replace_count(d), t.tail(), t.link(d)), (0, d, NULL));
        Tex::dealloc(t);
        // An empty third part is not.
        let t = tex();
        enter(t, -MMODE);
        let d = enter_disc(t, 2);
        t.handle_right_brace();
        assert_eq!((t.replace_count(d), t.tail(), printed(t)), (0, d, String::new()));
        Tex::dealloc(t);
    }

    #[test]
    fn improper_items_are_deleted_with_what_follows() {
        let t = tex();
        enter(t, -HMODE);
        let d = enter_disc(t, 2);
        chars(t, 1);
        let keep = t.tail();
        let (p, a) = (t.new_penalty(1), t.new_kern(1));
        t.tail_append(p);
        t.tail_append(a);
        t.handle_right_brace();
        let out = printed(t);
        assert!(out.starts_with("! Improper discretionary list."), "{out:?}");
        assert!(out.contains("Discretionary lists must contain only boxes and kerns."));
        assert!(out.contains("The following discretionary sublist has been deleted:"));
        assert!(out.contains("\\penalty 1"), "{out:?}");
        assert_eq!((t.replace_count(d), t.tail(), t.link(keep)), (1, keep, NULL));
        Tex::dealloc(t);
    }

    #[test]
    fn align_error_by_align_state() {
        for (key, cmd, chr, tok) in [
            ("misplaced_tab", TAB_MARK, b'&' as i32, TAB_TOKEN + b'&' as i32),
            ("misplaced_span", TAB_MARK, SPAN_CODE, CS_TOKEN_FLAG + cs(b's')),
            ("misplaced_cr", CAR_RET, CR_CODE, CS_TOKEN_FLAG + cs(b'c')),
            ("misplaced_crcr", CAR_RET, CR_CR_CODE, CS_TOKEN_FLAG + cs(b'c')),
        ] {
            for state in [1000000, 3, -3] {
                let t = tex();
                t.align_state = state;
                (t.cur_cmd, t.cur_chr, t.cur_tok) = (cmd, chr, tok);
                t.align_error();
                assert_message(key, &printed(t));
                assert_eq!(t.align_state, state);
                Tex::dealloc(t);
            }
        }
        for (key, state, inserted) in [
            ("missing_right_in_cell", 0, RIGHT_BRACE_TOKEN + b'}' as i32),
            ("missing_right_in_cell", 1, RIGHT_BRACE_TOKEN + b'}' as i32),
            ("missing_right_in_cell", 2, RIGHT_BRACE_TOKEN + b'}' as i32),
            ("missing_left_in_cell", -1, LEFT_BRACE_TOKEN + b'{' as i32),
            ("missing_left_in_cell", -2, LEFT_BRACE_TOKEN + b'{' as i32),
        ] {
            let t = tex();
            t.align_state = state;
            (t.cur_cmd, t.cur_chr, t.cur_tok) = (TAB_MARK, b'&' as i32, TAB_TOKEN + b'&' as i32);
            t.align_error();
            assert_message(key, &printed(t));
            // The first change is align_error's; ins_error's back_input of the brace undoes it, for get_next to count again.
            assert_eq!(t.align_state, state, "state {state}");
            assert_eq!(level(t), vec![inserted]);
            assert_eq!(t.toks[t.input_stack[t.input_ptr - 1].loc_field], TAB_TOKEN + b'&' as i32);
            Tex::dealloc(t);
        }
    }

    #[test]
    fn misplaced_noalign_omit_and_endcsname() {
        let t = tex();
        t.no_align_error();
        assert_message("noalign", &printed(t));
        t.omit_error();
        assert_message("omit", &printed(t));
        t.cs_error();
        assert_message("endcsname", &printed(t));
        Tex::dealloc(t);
    }

    #[test]
    fn normal_paragraph_resets_only_what_is_set() {
        let t = tex();
        t.new_save_level(SIMPLE_GROUP);
        let sp = t.save_ptr;
        t.normal_paragraph();
        assert_eq!(t.save_ptr, sp, "nothing was changed, so nothing was saved");
        t.set_looseness(3);
        t.set_hang_indent(5);
        t.set_hang_after(7);
        let shape = t.get_node(3);
        t.set_info(shape, 1);
        t.eqtb[PAR_SHAPE_LOC].set_rh(shape);
        let pens = t.get_node(3);
        t.set_info(pens, 1);
        t.eqtb[INTER_LINE_PENALTIES_LOC].set_rh(pens);
        t.normal_paragraph();
        assert_eq!(t.looseness(), 0);
        assert_eq!(t.hang_indent(), 0);
        assert_eq!(t.hang_after(), 1);
        assert_eq!(t.par_shape_ptr(), NULL);
        assert_eq!(t.inter_line_penalties_ptr(), NULL);
        assert!(t.save_ptr > sp, "each one is saved at this level");
        Tex::dealloc(t);
    }

    #[test]
    fn a_null_paragraph_is_dropped_and_lr_save_flushed() {
        let t = tex();
        enter(t, -VMODE);
        enter(t, HMODE);
        t.error_count = 50;
        let c = t.fast_get_avail();
        t.set_LR_save(c);
        t.set_looseness(2);
        t.end_graf();
        assert_eq!(t.mode(), -VMODE);
        assert_eq!(t.nest_ptr, 1);
        assert_eq!(t.error_count, 0);
        assert_eq!(t.looseness(), 0);
        assert_eq!(t.LR_save(), NULL);
        // In any other mode nothing happens.
        t.error_count = 50;
        t.set_looseness(2);
        t.end_graf();
        assert_eq!((t.error_count, t.looseness()), (50, 2));
        Tex::dealloc(t);
    }

    /// A v-template level at its end, which is what do_endv expects, under levels already read.
    fn at_end_of_v_template(t: &mut Tex, extra_levels: i32) {
        t.begin_token_list(OMIT_TEMPLATE, V_TEMPLATE);
        t.set_loc(OMIT_TEMPLATE + 2); // past end_template_token
        assert!(t.at_list_end());
        for _ in 0..extra_levels {
            let p = t.tok_one(LETTER_TOKEN + b'x' as i32);
            t.ins_list(p);
            t.set_loc(p + 2);
            assert!(t.at_list_end());
        }
    }

    #[test]
    fn do_endv_finds_its_v_template() {
        for extra in [0, 1, 3] {
            let t = tex();
            at_end_of_v_template(t, extra);
            t.cur_group = SIMPLE_GROUP; // not align_group: off_save, which is a real routine here
            t.cur_cmd = ENDV;
            t.cur_tok = CS_TOKEN_FLAG + FROZEN_ENDV;
            t.do_endv(); // the fatal error would end the test process here
            assert_message("missing_right_brace", &printed(t));
            Tex::dealloc(t);
        }
    }

    /// The fatal error of do_endv ends the process, so this goes by what its test sees.
    #[test]
    fn rec_at_list_end_sees_a_level_with_tokens_left() {
        let t = tex();
        t.begin_token_list(OMIT_TEMPLATE, V_TEMPLATE);
        assert!(!t.at_list_end(), "end_template_token is still to be read");
        t.input_stack[t.input_ptr] = t.cur_input;
        assert!(!t.rec_at_list_end(t.input_stack[t.input_ptr]));
        Tex::dealloc(t);
    }
}
