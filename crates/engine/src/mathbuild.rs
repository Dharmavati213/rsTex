//! §1314-1385 Part 48: building math lists.
//!
//! The dimensions of §1324 and §1377 are `i64` locals, section 11's rule for a site not in its table
//! (`w-total_shrink[normal]+q<=z` is in it): a box may be as wide as 2^31-1, the oracle's 64-bit
//! `integer` adds another to that, and its tests and halves see the true sum.
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

/// arith.rs's `half`, on the `i64` locals of the display's position.
#[inline]
fn mathbuild_half(x: i64) -> i64 {
    if x & 1 != 0 { (x + 1) / 2 } else { x / 2 }
}

impl Tex {
    /// §1314
    pub fn push_math(&mut self, c: i32) {
        self.push_nest();
        self.set_mode(-MMODE);
        self.set_incompleat_noad(NULL);
        self.new_save_level(c);
    }

    /// §1317 Go into ordinary math mode
    fn mathbuild_go_into_ordinary_math(&mut self) {
        self.push_math(MATH_SHIFT_GROUP);
        self.eq_word_define(INT_BASE + CUR_FAM_CODE, -1);
        // dropped: src-specials, insert_src_special_every_math
        if self.every_math() != NULL {
            self.begin_token_list(self.every_math(), EVERY_MATH_TEXT);
        }
    }

    /// §1316
    pub fn init_math(&mut self) {
        self.get_token(); // get_x_token would fail on \ifmmode!
        if self.cur_cmd == MATH_SHIFT && self.mode() > 0 {
            // §1323 Go into display math mode
            let mut j = NULL;
            let mut w: i64 = -(MAX_DIMEN as i64);
            let mut x;
            if self.head() == self.tail() {
                // `\noindent$$' or `$${ }$$'
                // §1732 Prepare for display after an empty paragraph
                self.pop_nest();
                // §1731 Set the value of x to the text direction before the display
                x = if self.LR_save() == NULL {
                    0
                } else if self.info(self.LR_save()) >= R_CODE {
                    -1
                } else {
                    1
                };
            } else {
                self.line_break(true);
                // §1324 Calculate the natural width, w, by which the characters of the final line extend
                // §1734 Prepare for display after a non-empty paragraph
                let mut p;
                if self.eTeX_ex() {
                    // §1740 Let j be the prototype box for the display
                    j = if self.right_skip() == ZERO_GLUE {
                        self.new_kern(0)
                    } else {
                        self.new_param_glue(RIGHT_SKIP_CODE)
                    };
                    p = if self.left_skip() == ZERO_GLUE {
                        self.new_kern(0)
                    } else {
                        self.new_param_glue(LEFT_SKIP_CODE)
                    };
                    self.set_link(p, j);
                    j = self.new_null_box();
                    self.set_width(j, self.width(self.just_box));
                    self.set_shift_amount(j, self.shift_amount(self.just_box));
                    self.set_list_ptr(j, p);
                    self.set_glue_order(j, self.glue_order(self.just_box));
                    self.set_glue_sign(j, self.glue_sign(self.just_box));
                    self.set_glue_set(j, self.glue_set(self.just_box));
                }
                let mut v = self.shift_amount(self.just_box) as i64;
                // §1731 Set the value of x to the text direction before the display
                x = if self.LR_save() == NULL {
                    0
                } else if self.info(self.LR_save()) >= R_CODE {
                    -1
                } else {
                    1
                };
                if x >= 0 {
                    p = self.list_ptr(self.just_box);
                    self.set_link(TEMP_HEAD, NULL);
                } else {
                    v = -v - self.width(self.just_box) as i64;
                    p = self.new_math(0, BEGIN_L_CODE);
                    self.set_link(TEMP_HEAD, p);
                    let e = self.new_math(0, END_L_CODE);
                    self.just_copy(self.list_ptr(self.just_box), p, e);
                    self.cur_dir = RIGHT_TO_LEFT;
                }
                v += 2 * self.quad(self.cur_font()) as i64;
                if self.TeXXeT_en() {
                    // §1710 Initialize the LR stack
                    self.put_LR(BEFORE); // this will never match
                }
                'done: {
                    while p != NULL {
                        // §1325 Let d be the natural width of node p; if the node is ``visible,'' goto found
                        let mut d;
                        let mut found = false;
                        loop {
                            // reswitch:
                            if self.is_char_node(p) {
                                let f = self.font(p);
                                let i = self.char_info(f, self.character(p));
                                d = self.char_width(f, i);
                                found = true;
                                break;
                            }
                            match self.ty(p) {
                                HLIST_NODE | VLIST_NODE | RULE_NODE => {
                                    d = self.width(p);
                                    found = true;
                                }
                                LIGATURE_NODE => {
                                    // §826 Make node p look like a char_node and goto reswitch
                                    self.mem[LIG_TRICK] = self.mem[self.lig_char(p)];
                                    self.set_link(LIG_TRICK, self.link(p));
                                    p = LIG_TRICK;
                                    continue;
                                }
                                MARGIN_KERN_NODE => {
                                    d = self.width(p);
                                }
                                KERN_NODE => {
                                    d = self.width(p);
                                }
                                // §1736 Cases of `Let d be the natural width' that need special treatment
                                MATH_NODE => {
                                    d = self.width(p);
                                    if self.TeXXeT_en() {
                                        // §1737 Adjust the LR stack for the init_math routine
                                        if self.end_LR(p) {
                                            if self.info(self.LR_ptr) == self.end_LR_type(p) {
                                                self.pop_LR();
                                            } else if self.subtype(p) > L_CODE {
                                                w = MAX_DIMEN as i64;
                                                break 'done;
                                            }
                                        } else {
                                            self.push_LR(p);
                                            if self.LR_dir(p) != self.cur_dir {
                                                self.just_reverse(p);
                                                p = TEMP_HEAD;
                                            }
                                        }
                                    } else if self.subtype(p) >= L_CODE {
                                        w = MAX_DIMEN as i64;
                                        break 'done;
                                    }
                                }
                                EDGE_NODE => {
                                    d = self.width(p);
                                    self.cur_dir = self.subtype(p);
                                }
                                GLUE_NODE => {
                                    // §1326 Let d be the natural width of this glue; if stretching or shrinking, set v
                                    let q = self.glue_ptr(p);
                                    d = self.width(q);
                                    if self.glue_sign(self.just_box) == STRETCHING {
                                        if self.glue_order(self.just_box) == self.stretch_order(q)
                                            && self.stretch(q) != 0
                                        {
                                            v = MAX_DIMEN as i64;
                                        }
                                    } else if self.glue_sign(self.just_box) == SHRINKING {
                                        if self.glue_order(self.just_box) == self.shrink_order(q)
                                            && self.shrink(q) != 0
                                        {
                                            v = MAX_DIMEN as i64;
                                        }
                                    }
                                    if self.subtype(p) >= A_LEADERS {
                                        found = true;
                                    }
                                }
                                WHATSIT_NODE => {
                                    // §1608 Let d be the width of the whatsit p
                                    d = if self.subtype(p) == PDF_REFXFORM_NODE
                                        || self.subtype(p) == PDF_REFXIMAGE_NODE
                                    {
                                        self.pdf_width(p)
                                    } else {
                                        0
                                    };
                                }
                                _ => {
                                    d = 0;
                                }
                            }
                            break;
                        }
                        if found {
                            if v < MAX_DIMEN as i64 {
                                v += d as i64;
                                w = v;
                            } else {
                                w = MAX_DIMEN as i64;
                                break 'done;
                            }
                        } else if v < MAX_DIMEN as i64 {
                            v += d as i64;
                        }
                        p = self.link(p); // not_found:
                    }
                }
                // done:
                // §1735 Finish the natural width computation
                if self.TeXXeT_en() {
                    while self.LR_ptr != NULL {
                        self.pop_LR();
                    }
                    if self.LR_problems != 0 {
                        w = MAX_DIMEN as i64;
                        self.LR_problems = 0;
                    }
                }
                self.cur_dir = LEFT_TO_RIGHT;
                self.flush_node_list(self.link(TEMP_HEAD));
            }
            // now we are in vertical mode, working on the list that will contain the display
            // §1327 Calculate the length, l, and the shift amount, s, of the display lines
            let (l, s);
            let pg = self.prev_graf() as i64;
            if self.par_shape_ptr() == NULL {
                let a = self.hang_after() as i64;
                if self.hang_indent() != 0 && ((a >= 0 && pg + 2 > a) || pg + 1 < -a) {
                    l = self.hsize().wrapping_sub(abs(self.hang_indent()));
                    s = if self.hang_indent() > 0 { self.hang_indent() } else { 0 };
                } else {
                    l = self.hsize();
                    s = 0;
                }
            } else {
                let n = self.info(self.par_shape_ptr());
                let p = if pg + 2 >= n as i64 {
                    self.par_shape_ptr() + 2 * n
                } else {
                    self.par_shape_ptr() + 2 * (self.prev_graf() + 2)
                };
                s = self.mem[p - 1].sc();
                l = self.mem[p].sc();
            }
            self.push_math(MATH_SHIFT_GROUP);
            self.set_mode(MMODE);
            self.eq_word_define(INT_BASE + CUR_FAM_CODE, -1);
            let v = self.store32(w);
            self.eq_word_define(DIMEN_BASE + PRE_DISPLAY_SIZE_CODE, v);
            self.set_LR_box(j);
            if self.eTeX_ex() {
                self.eq_word_define(INT_BASE + PRE_DISPLAY_DIRECTION_CODE, x);
            }
            self.eq_word_define(DIMEN_BASE + DISPLAY_WIDTH_CODE, l);
            self.eq_word_define(DIMEN_BASE + DISPLAY_INDENT_CODE, s);
            if self.every_display() != NULL {
                self.begin_token_list(self.every_display(), EVERY_DISPLAY_TEXT);
            }
            if self.nest_ptr == 1 {
                self.build_page();
            }
        } else {
            self.back_input();
            self.mathbuild_go_into_ordinary_math();
        }
    }

    /// §1320
    pub fn start_eq_no(&mut self) {
        self.set_saved(0, self.cur_chr);
        self.save_ptr += 1;
        self.mathbuild_go_into_ordinary_math();
    }

    /// §1330 Treat cur_chr as an active character
    fn mathbuild_treat_as_active(&mut self) {
        self.cur_cs = self.cur_chr + ACTIVE_BASE;
        self.cur_cmd = self.eq_type(self.cur_cs);
        self.cur_chr = self.equiv(self.cur_cs);
        self.x_token();
        self.back_input();
    }

    /// §1329
    pub fn scan_math(&mut self, p: i32) {
        let mut c; // math character code
        'restart: loop {
            // §430 Get the next non-blank non-relax non-call token
            loop {
                self.get_x_token();
                if self.cur_cmd != SPACER && self.cur_cmd != RELAX {
                    break;
                }
            }
            loop {
                // reswitch:
                match self.cur_cmd {
                    LETTER | OTHER_CHAR | CHAR_GIVEN => {
                        c = self.math_code(self.cur_chr);
                        if c == 0o100000 {
                            self.mathbuild_treat_as_active();
                            continue 'restart;
                        }
                    }
                    CHAR_NUM => {
                        self.scan_char_num();
                        self.cur_chr = self.cur_val;
                        self.cur_cmd = CHAR_GIVEN;
                        continue;
                    }
                    MATH_CHAR_NUM => {
                        self.scan_fifteen_bit_int();
                        c = self.cur_val;
                    }
                    MATH_GIVEN => {
                        c = self.cur_chr;
                    }
                    DELIM_NUM => {
                        self.scan_twenty_seven_bit_int();
                        c = self.cur_val / 0o10000;
                    }
                    _ => {
                        // §1331 Scan a subformula enclosed in braces and return
                        self.back_input();
                        self.scan_left_brace();
                        self.set_saved(0, p);
                        self.save_ptr += 1;
                        self.push_math(MATH_GROUP);
                        return;
                    }
                }
                break 'restart;
            }
        }
        self.set_math_type(p, MATH_CHAR);
        self.set_character(p, c % 256);
        if c >= VAR_CODE && self.fam_in_range() {
            self.set_fam(p, self.cur_fam());
        } else {
            self.set_fam(p, (c / 256) % 16);
        }
    }

    /// §1333
    pub fn set_math_char(&mut self, c: i32) {
        if c >= 0o100000 {
            self.mathbuild_treat_as_active();
        } else {
            let p = self.new_noad();
            let n = self.nucleus(p);
            self.set_math_type(n, MATH_CHAR);
            self.set_character(n, c % 256);
            self.set_fam(n, (c / 256) % 16);
            if c >= VAR_CODE {
                if self.fam_in_range() {
                    self.set_fam(n, self.cur_fam());
                }
                self.set_ty(p, ORD_NOAD);
            } else {
                self.set_ty(p, ORD_NOAD + (c / 0o10000));
            }
            self.set_link(self.tail(), p);
            self.set_tail(p);
        }
    }

    /// §1337
    pub fn math_limit_switch(&mut self) {
        if self.head() != self.tail() && self.ty(self.tail()) == OP_NOAD {
            self.set_subtype(self.tail(), self.cur_chr);
            return;
        }
        self.print_err_s(b"Limit controls must follow a math operator");
        self.help(&[b"I'm ignoring this misplaced \\limits or \\nolimits command."]);
        self.error();
    }

    /// §1338
    pub fn scan_delimiter(&mut self, p: i32, r: bool) {
        if r {
            self.scan_twenty_seven_bit_int();
        } else {
            // §430 Get the next non-blank non-relax non-call token
            loop {
                self.get_x_token();
                if self.cur_cmd != SPACER && self.cur_cmd != RELAX {
                    break;
                }
            }
            match self.cur_cmd {
                LETTER | OTHER_CHAR => {
                    self.cur_val = self.del_code(self.cur_chr);
                }
                DELIM_NUM => {
                    self.scan_twenty_seven_bit_int();
                }
                _ => {
                    self.cur_val = -1;
                }
            }
        }
        if self.cur_val < 0 {
            // §1339 Report that an invalid delimiter code is being changed to null; set cur_val:=0
            self.print_err_s(b"Missing delimiter (. inserted)");
            self.help(&[
                b"I was expecting to see something like `(' or `\\{' or",
                b"`\\}' here. If you typed, e.g., `{' instead of `\\{', you",
                b"should probably delete the `{' by typing `1' now, so that",
                b"braces don't get unbalanced. Otherwise just proceed.",
                b"Acceptable delimiters are characters whose \\delcode is",
                b"nonnegative, or you can use `\\delimiter <delimiter code>'.",
            ]);
            self.back_error();
            self.cur_val = 0;
        }
        self.set_small_fam(p, (self.cur_val / 0o4000000) % 16);
        self.set_small_char(p, (self.cur_val / 0o10000) % 256);
        self.set_large_fam(p, (self.cur_val / 256) % 16);
        self.set_large_char(p, self.cur_val % 256);
    }

    /// §1341
    pub fn math_radical(&mut self) {
        let n = self.get_node(RADICAL_NOAD_SIZE);
        self.tail_append(n);
        let t = self.tail();
        self.set_ty(t, RADICAL_NOAD);
        self.set_subtype(t, NORMAL);
        let (a, b, c) = (self.nucleus(t), self.subscr(t), self.supscr(t));
        self.mem[a] = self.empty_field;
        self.mem[b] = self.empty_field;
        self.mem[c] = self.empty_field;
        self.scan_delimiter(self.left_delimiter(t), true);
        self.scan_math(self.nucleus(self.tail()));
    }

    /// §1343
    pub fn math_ac(&mut self) {
        if self.cur_cmd == ACCENT {
            // §1344 Complain that the user should have said \mathaccent
            self.print_err_s(b"Please use ");
            self.print_esc_s(b"mathaccent");
            self.print_s(b" for accents in math mode");
            self.help(&[
                b"I'm changing \\accent to \\mathaccent here; wish me luck.",
                b"(Accents are not the same in formulas as they are in text.)",
            ]);
            self.error();
        }
        let n = self.get_node(ACCENT_NOAD_SIZE);
        self.tail_append(n);
        let t = self.tail();
        self.set_ty(t, ACCENT_NOAD);
        self.set_subtype(t, NORMAL);
        let (a, b, c) = (self.nucleus(t), self.subscr(t), self.supscr(t));
        self.mem[a] = self.empty_field;
        self.mem[b] = self.empty_field;
        self.mem[c] = self.empty_field;
        let ac = self.accent_chr(t);
        self.set_math_type(ac, MATH_CHAR);
        self.scan_fifteen_bit_int();
        self.set_character(ac, self.cur_val % 256);
        if self.cur_val >= VAR_CODE && self.fam_in_range() {
            self.set_fam(ac, self.cur_fam());
        } else {
            self.set_fam(ac, (self.cur_val / 256) % 16);
        }
        self.scan_math(self.nucleus(self.tail()));
    }

    /// §1350
    pub fn append_choices(&mut self) {
        let n = self.new_choice();
        self.tail_append(n);
        self.save_ptr += 1;
        self.set_saved(-1, 0);
        self.push_math(MATH_CHOICE_GROUP);
        self.scan_left_brace();
    }

    /// §1352
    pub fn build_choices(&mut self) {
        self.unsave();
        let p = self.fin_mlist(NULL);
        match self.saved(-1) {
            0 => self.set_display_mlist(self.tail(), p),
            1 => self.set_text_mlist(self.tail(), p),
            2 => self.set_script_mlist(self.tail(), p),
            3 => {
                self.set_script_script_mlist(self.tail(), p);
                self.save_ptr -= 1;
                return;
            }
            _ => {} // there are no other cases
        }
        self.set_saved(-1, self.saved(-1) + 1);
        self.push_math(MATH_CHOICE_GROUP);
        self.scan_left_brace();
    }

    /// §1354
    pub fn sub_sup(&mut self) {
        let mut t = EMPTY; // type of previous sub/superscript
        let mut p = NULL; // field to be filled by scan_math
        if self.tail() != self.head() && self.scripts_allowed(self.tail()) {
            p = self.supscr(self.tail()) + self.cur_cmd - SUP_MARK; // supscr or subscr
            t = self.math_type(p);
        }
        if p == NULL || t != EMPTY {
            // §1355 Insert a dummy noad to be sub/superscripted
            let n = self.new_noad();
            self.tail_append(n);
            p = self.supscr(self.tail()) + self.cur_cmd - SUP_MARK; // supscr or subscr
            if t != EMPTY {
                if self.cur_cmd == SUP_MARK {
                    self.print_err_s(b"Double superscript");
                    self.help(&[b"I treat `x^1^2' essentially like `x^1{}^2'."]);
                } else {
                    self.print_err_s(b"Double subscript");
                    self.help(&[b"I treat `x_1_2' essentially like `x_1{}_2'."]);
                }
                self.error();
            }
        }
        self.scan_math(p);
    }

    /// §1359
    pub fn math_fraction(&mut self) {
        let c = self.cur_chr; // the type of generalized fraction we are scanning
        if self.incompleat_noad() != NULL {
            // §1361 Ignore the fraction operation and complain about this ambiguous case
            if c >= DELIMITED_CODE {
                self.scan_delimiter(GARBAGE, false);
                self.scan_delimiter(GARBAGE, false);
            }
            if c % DELIMITED_CODE == ABOVE_CODE {
                self.scan_normal_dimen();
            }
            self.print_err_s(b"Ambiguous; you need another { and }");
            self.help(&[
                b"I'm ignoring this fraction specification, since I don't",
                b"know whether a construction like `x \\over y \\over z'",
                b"means `{x \\over y} \\over z' or `x \\over {y \\over z}'.",
            ]);
            self.error();
        } else {
            let f = self.get_node(FRACTION_NOAD_SIZE);
            self.set_incompleat_noad(f);
            self.set_ty(f, FRACTION_NOAD);
            self.set_subtype(f, NORMAL);
            let n = self.numerator(f);
            self.set_math_type(n, SUB_MLIST);
            self.set_info(n, self.link(self.head()));
            let d = self.denominator(f);
            self.mem[d] = self.empty_field;
            let nd = self.null_delimiter;
            let x = self.left_delimiter(f);
            self.mem[x].set_qqqq(nd);
            let x = self.right_delimiter(f);
            self.mem[x].set_qqqq(nd);
            self.set_link(self.head(), NULL);
            self.set_tail(self.head());
            // §1360 Use code c to distinguish between generalized fractions
            if c >= DELIMITED_CODE {
                self.scan_delimiter(self.left_delimiter(f), false);
                self.scan_delimiter(self.right_delimiter(f), false);
            }
            match c % DELIMITED_CODE {
                ABOVE_CODE => {
                    self.scan_normal_dimen();
                    self.set_thickness(f, self.cur_val);
                }
                OVER_CODE => self.set_thickness(f, DEFAULT_CODE),
                ATOP_CODE => self.set_thickness(f, 0),
                _ => {} // there are no other cases
            }
        }
    }

    /// §1362
    pub fn fin_mlist(&mut self, p: i32) -> i32 {
        let q; // the mlist to return
        let f = self.incompleat_noad();
        if f != NULL {
            // §1363 Compleat the incompleat noad
            let d = self.denominator(f);
            self.set_math_type(d, SUB_MLIST);
            self.set_info(d, self.link(self.head()));
            if p == NULL {
                q = f;
            } else {
                q = self.info(self.numerator(f));
                if self.ty(q) != LEFT_NOAD || self.delim_ptr() == NULL {
                    self.confusion(b"right");
                }
                self.set_info(self.numerator(f), self.link(self.delim_ptr()));
                self.set_link(self.delim_ptr(), f);
                self.set_link(f, p);
            }
        } else {
            self.set_link(self.tail(), p);
            q = self.link(self.head());
        }
        self.pop_nest();
        q
    }

    /// §1369
    pub fn math_left_right(&mut self) {
        let t = self.cur_chr; // left_noad or right_noad
        if t != LEFT_NOAD && self.cur_group != MATH_LEFT_GROUP {
            // §1370 Try to recover from mismatched \right
            if self.cur_group == MATH_SHIFT_GROUP {
                self.scan_delimiter(GARBAGE, false);
                self.print_err_s(b"Extra ");
                if t == MIDDLE_NOAD {
                    self.print_esc_s(b"middle");
                    self.help(&[b"I'm ignoring a \\middle that had no matching \\left."]);
                } else {
                    self.print_esc_s(b"right");
                    self.help(&[b"I'm ignoring a \\right that had no matching \\left."]);
                }
                self.error();
            } else {
                self.off_save();
            }
        } else {
            let p = self.new_noad();
            self.set_ty(p, t);
            self.scan_delimiter(self.delimiter(p), false);
            if t == MIDDLE_NOAD {
                self.set_ty(p, RIGHT_NOAD);
                self.set_subtype(p, MIDDLE_NOAD);
            }
            let q;
            if t == LEFT_NOAD {
                q = p;
            } else {
                q = self.fin_mlist(p);
                self.unsave(); // end of math_left_group
            }
            if t != RIGHT_NOAD {
                self.push_math(MATH_LEFT_GROUP);
                self.set_link(self.head(), q);
                self.set_tail(p);
                self.set_delim_ptr(p);
            } else {
                let n = self.new_noad();
                self.tail_append(n);
                self.set_ty(self.tail(), INNER_NOAD);
                let n = self.nucleus(self.tail());
                self.set_math_type(n, SUB_MLIST);
                self.set_info(n, q);
            }
        }
    }

    /// §1373 Check that the necessary fonts for math symbols are present, or return true (`danger`)
    /// after deleting the formula.
    fn mathbuild_math_fonts_missing(&mut self) -> bool {
        if self.font_params[self.fam_fnt(2 + TEXT_SIZE)] < TOTAL_MATHSY_PARAMS
            || self.font_params[self.fam_fnt(2 + SCRIPT_SIZE)] < TOTAL_MATHSY_PARAMS
            || self.font_params[self.fam_fnt(2 + SCRIPT_SCRIPT_SIZE)] < TOTAL_MATHSY_PARAMS
        {
            self.print_err_s(b"Math formula deleted: Insufficient symbol fonts");
            self.help(&[
                b"Sorry, but I can't typeset math unless \\textfont 2",
                b"and \\scriptfont 2 and \\scriptscriptfont 2 have all",
                b"the \\fontdimen values needed in math symbol fonts.",
            ]);
            self.error();
            self.flush_math();
            true
        } else if self.font_params[self.fam_fnt(3 + TEXT_SIZE)] < TOTAL_MATHEX_PARAMS
            || self.font_params[self.fam_fnt(3 + SCRIPT_SIZE)] < TOTAL_MATHEX_PARAMS
            || self.font_params[self.fam_fnt(3 + SCRIPT_SCRIPT_SIZE)] < TOTAL_MATHEX_PARAMS
        {
            self.print_err_s(b"Math formula deleted: Insufficient extension fonts");
            self.help(&[
                b"Sorry, but I can't typeset math unless \\textfont 3",
                b"and \\scriptfont 3 and \\scriptscriptfont 3 have all",
                b"the \\fontdimen values needed in math extension fonts.",
            ]);
            self.error();
            self.flush_math();
            true
        } else {
            false
        }
    }

    /// §1375 Check that another $ follows
    fn mathbuild_check_second_dollar(&mut self) {
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

    /// §1372
    pub fn after_math(&mut self) {
        // §1376, §1741 Local variables for finishing a displayed formula are declared where they
        // are first set; the dimensions are i64: see the header of this file.
        let mut j = NULL; // prototype box
        // §1742 Retrieve the prototype box
        if self.mode() == MMODE {
            j = self.LR_box();
        }
        let mut danger = self.mathbuild_math_fonts_missing(); // not enough symbol fonts are present
        let mut m = self.mode(); // mmode or -mmode
        let mut l = false; // `\leqno' instead of `\eqno'
        let mut p = self.fin_mlist(NULL); // this pops the nest; the formula
        let a; // box containing equation number
        if self.mode() == -m {
            // end of equation number
            self.mathbuild_check_second_dollar();
            self.cur_mlist = p;
            self.cur_style = TEXT_STYLE;
            self.mlist_penalties = false;
            self.mlist_to_hlist();
            a = self.hpack_natural(self.link(TEMP_HEAD));
            self.set_box_lr(a, DLIST);
            self.unsave();
            self.save_ptr -= 1; // now cur_group=math_shift_group
            if self.saved(0) == 1 {
                l = true;
            }
            // §1742 Retrieve the prototype box
            if self.mode() == MMODE {
                j = self.LR_box();
            }
            danger = self.mathbuild_math_fonts_missing();
            m = self.mode();
            p = self.fin_mlist(NULL);
        } else {
            a = NULL;
        }
        if m < 0 {
            // §1374 Finish math in text
            let n = self.new_math(self.math_surround(), BEFORE);
            self.tail_append(n);
            self.cur_mlist = p;
            self.cur_style = TEXT_STYLE;
            self.mlist_penalties = self.mode() > 0;
            self.mlist_to_hlist();
            self.set_link(self.tail(), self.link(TEMP_HEAD));
            while self.link(self.tail()) != NULL {
                self.set_tail(self.link(self.tail()));
            }
            let n = self.new_math(self.math_surround(), AFTER);
            self.tail_append(n);
            self.set_space_factor(1000);
            self.unsave();
        } else {
            if a == NULL {
                self.mathbuild_check_second_dollar();
            }
            // §1377 Finish displayed math
            self.cur_mlist = p;
            self.cur_style = DISPLAY_STYLE;
            self.mlist_penalties = false;
            self.mlist_to_hlist();
            p = self.link(TEMP_HEAD);
            self.adjust_tail = ADJUST_HEAD;
            self.pre_adjust_tail = PRE_ADJUST_HEAD;
            let mut b = self.hpack_natural(p); // box containing the equation
            p = self.list_ptr(b);
            let t = self.adjust_tail; // tail of adjustment list
            self.adjust_tail = NULL;
            let pre_t = self.pre_adjust_tail; // tail of pre-adjustment list
            self.pre_adjust_tail = NULL;
            let mut w = self.width(b) as i64; // width of the equation
            let z = self.display_width() as i64; // width of the line
            let mut s = self.display_indent() as i64; // move the line right this much
            if self.pre_display_direction() < 0 {
                s = -s - z;
            }
            let mut e: i64; // width of equation number
            let q: i64; // width of equation number plus space to separate from equation
            if a == NULL || danger {
                e = 0;
                q = 0;
            } else {
                e = self.width(a) as i64;
                q = e + self.math_quad(TEXT_SIZE) as i64;
            }
            if self.held(w + q) > z {
                // §1379 Squeeze the equation as much as possible; if there is an equation number
                // that should go on a separate line by itself, set e:=0
                if e != 0
                    && (self.held(w - self.total_shrink[NORMAL] + q) <= z
                        || self.total_shrink[FIL] != 0
                        || self.total_shrink[FILL] != 0
                        || self.total_shrink[FILLL] != 0)
                {
                    self.free_node(b, BOX_NODE_SIZE);
                    b = self.hpack_wide(p, self.held(z - q), EXACTLY);
                } else {
                    e = 0;
                    if w > z {
                        self.free_node(b, BOX_NODE_SIZE);
                        b = self.hpack_wide(p, z, EXACTLY);
                    }
                }
                w = self.width(b) as i64;
            }
            // §1380 Determine the displacement, d, of the left edge of the equation
            self.set_box_lr(b, DLIST);
            let mut d = mathbuild_half(self.held(z - w)); // displacement of equation in the line
            if e > 0 && d < self.held(2 * e) {
                // too close
                d = mathbuild_half(self.held(z - w - e));
                if p != NULL && !self.is_char_node(p) && self.ty(p) == GLUE_NODE {
                    d = 0;
                }
            }
            // §1381 Append the glue or equation number preceding the display
            let n = self.new_penalty(self.pre_display_penalty());
            self.tail_append(n);
            let g1; // glue parameter codes for before and after
            let mut g2;
            if self.held(d + s) <= self.pre_display_size() as i64 || l {
                // not enough clearance
                g1 = ABOVE_DISPLAY_SKIP_CODE;
                g2 = BELOW_DISPLAY_SKIP_CODE;
            } else {
                g1 = ABOVE_DISPLAY_SHORT_SKIP_CODE;
                g2 = BELOW_DISPLAY_SHORT_SKIP_CODE;
            }
            if l && e == 0 {
                // it follows that type(a)=hlist_node
                self.app_display(j, a, 0);
                let n = self.new_penalty(INF_PENALTY);
                self.tail_append(n);
            } else {
                let n = self.new_param_glue(g1);
                self.tail_append(n);
            }
            // §1382 Append the display and perhaps also the equation number
            if e != 0 {
                let v = self.store32(z - w - e - d);
                let r = self.new_kern(v); // kern node used to position the display
                if l {
                    self.set_link(a, r);
                    self.set_link(r, b);
                    b = a;
                    d = 0;
                } else {
                    self.set_link(b, r);
                    self.set_link(r, a);
                }
                b = self.hpack_natural(b);
            }
            let v = self.store32(d);
            self.app_display(j, b, v);
            // §1383 Append the glue or equation number following the display
            if a != NULL && e == 0 && !l {
                let n = self.new_penalty(INF_PENALTY);
                self.tail_append(n);
                let v = self.store32(z - self.width(a) as i64);
                self.app_display(j, a, v);
                g2 = 0;
            }
            if t != ADJUST_HEAD {
                // migrating material comes after equation number
                self.set_link(self.tail(), self.link(ADJUST_HEAD));
                self.set_tail(t);
            }
            if pre_t != PRE_ADJUST_HEAD {
                self.set_link(self.tail(), self.link(PRE_ADJUST_HEAD));
                self.set_tail(pre_t);
            }
            let n = self.new_penalty(self.post_display_penalty());
            self.tail_append(n);
            if g2 > 0 {
                let n = self.new_param_glue(g2);
                self.tail_append(n);
            }
            // §1743 Flush the prototype box
            self.flush_node_list(j);
            self.resume_after_display();
        }
    }

    /// §1378
    pub fn resume_after_display(&mut self) {
        if self.cur_group != MATH_SHIFT_GROUP {
            self.confusion(b"display");
        }
        self.unsave();
        self.set_prev_graf(self.prev_graf().wrapping_add(3));
        self.push_nest();
        self.set_mode(HMODE);
        self.set_space_factor(1000);
        self.set_cur_lang();
        self.set_clang(self.cur_lang);
        let (lh, rh) = (self.left_hyphen_min(), self.right_hyphen_min());
        let g = (self.norm_min(lh) * 0o100 + self.norm_min(rh)) * 0o200000 + self.cur_lang;
        self.set_prev_graf(g);
        // §469 Scan an optional space
        self.get_x_token();
        if self.cur_cmd != SPACER {
            self.back_input();
        }
        if self.nest_ptr == 1 {
            self.build_page();
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

    const DOLLAR: i32 = MATH_SHIFT * 256 + b'$' as i32;

    /// `table_machine` with printing, a file level at line 7, and a formula in progress.
    fn math_machine() -> &'static mut Tex {
        let t = table_machine();
        print_setup(t);
        t.line = 7;
        t.set_name(20);
        t.push_math(MATH_SHIFT_GROUP);
        t
    }

    /// A noad with empty fields, as new_noad makes, which is not ported in this tree.
    fn noad(t: &mut Tex, ty: i32) -> i32 {
        let p = t.get_node(NOAD_SIZE);
        t.set_link(p, NULL);
        t.set_ty(p, ty);
        t.set_subtype(p, NORMAL);
        let (a, b, c) = (t.nucleus(p), t.subscr(p), t.supscr(p));
        t.mem[a] = t.empty_field;
        t.mem[b] = t.empty_field;
        t.mem[c] = t.empty_field;
        p
    }

    fn append(t: &mut Tex, p: i32) {
        t.set_link(t.tail(), p);
        t.set_tail(p);
    }

    /// The fields of a math_char: its type, fam and character.
    fn field(t: &Tex, p: i32) -> (i32, i32, i32) {
        (t.math_type(p), t.fam(p), t.character(p))
    }

    #[test]
    fn half_on_i64_is_half_below_2_31_and_goes_on_above() {
        let t = machine();
        for x in [0, 1, -1, 2, -2, 3, -3, 7, -7, 1073741823, -1073741823, 2147483646, 2147483647, -2147483647] {
            assert_eq!(mathbuild_half(x as i64), t.half(x) as i64, "{x}");
        }
        assert_eq!(mathbuild_half(3221225469), 1610612735);
        assert_eq!(mathbuild_half(-3221225469), -1610612734);
        assert_eq!(mathbuild_half(4294967294), 2147483647);
        Tex::dealloc(t);
    }

    #[test]
    fn push_math_starts_an_empty_formula() {
        let t = table_machine();
        t.set_mode(HMODE);
        let outer = t.nest_ptr;
        t.set_incompleat_noad(77);
        t.push_math(MATH_LEFT_GROUP);
        assert_eq!(t.nest_ptr, outer + 1);
        assert_eq!((t.mode(), t.incompleat_noad(), t.cur_group), (-MMODE, NULL, MATH_LEFT_GROUP));
        assert_eq!(t.head(), t.tail());
        Tex::dealloc(t);
    }

    #[test]
    fn scan_math_takes_a_character_and_the_family_rule() {
        let t = math_machine();
        let p = t.get_node(1);
        t.set_cur_fam(-1);
        t.set_math_code(b'a' as i32, 0x7161);
        feed(t, &[letter(b'a')]);
        t.scan_math(p);
        assert_eq!(field(t, p), (MATH_CHAR, 1, 0x61));
        // Class 7 takes \fam when it is in 0..15, and no other class does.
        for (fam, code, want) in [
            (0, 0x7161, 0),
            (3, 0x7161, 3),
            (15, 0x7161, 15),
            (16, 0x7161, 1),
            (-1, 0x7161, 1),
            (3, 0x6161, 1),
            (3, 0x0261, 2),
            (3, 0x7F61, 3),
            (16, 0x7F61, 15),
            (3, 0x6F61, 15),
        ] {
            t.set_cur_fam(fam);
            t.set_math_code(b'a' as i32, code);
            feed(t, &[letter(b'a')]);
            t.scan_math(p);
            assert_eq!(field(t, p), (MATH_CHAR, want, code % 256), "fam {fam} code {code:x}");
        }
        t.set_cur_fam(-1);
        // Spaces and \relax are passed over, a macro is expanded, a \mathchardef gives the code.
        t.set_eq_type(cs(b'r'), RELAX);
        t.set_equiv(cs(b'r'), 256);
        t.set_math_code(b'b' as i32, 0x0362);
        def_macro(t, cs(b'm'), CALL, &[], &[letter(b'b')]);
        feed(t, &[SPACE_TOKEN, cs_tok(b'r'), SPACE_TOKEN, cs_tok(b'm')]);
        t.scan_math(p);
        assert_eq!(field(t, p), (MATH_CHAR, 3, 0x62));
        t.set_eq_type(cs(b'g'), MATH_GIVEN);
        t.set_equiv(cs(b'g'), 0x2563);
        feed(t, &[cs_tok(b'g')]);
        t.scan_math(p);
        assert_eq!(field(t, p), (MATH_CHAR, 5, 0x63));
        Tex::dealloc(t);
    }

    #[test]
    fn scan_math_treats_mathcode_8000_as_active() {
        let t = math_machine();
        let p = t.get_node(1);
        t.set_cur_fam(-1);
        t.set_math_code(b'!' as i32, 0o100000);
        t.set_math_code(b'b' as i32, 0x0162);
        def_macro(t, ACTIVE_BASE + b'!' as i32, CALL, &[], &[letter(b'b'), letter(b'c')]);
        feed(t, &[OTHER_TOKEN + b'!' as i32, letter(b'z')]);
        t.scan_math(p);
        assert_eq!(field(t, p), (MATH_CHAR, 1, 0x62));
        // The rest of the macro's body, then what followed, are still to come.
        t.get_token();
        assert_eq!(t.cur_tok, letter(b'c'));
        t.get_token();
        assert_eq!(t.cur_tok, letter(b'z'));
        Tex::dealloc(t);
    }

    #[test]
    fn set_math_char_of_8000_backs_up_the_meaning() {
        let t = math_machine();
        // An active character that means \relax, so that x_token keeps it.
        t.set_eq_type(ACTIVE_BASE + b'!' as i32, RELAX);
        t.set_equiv(ACTIVE_BASE + b'!' as i32, 256);
        t.cur_chr = b'!' as i32;
        let before = t.tail();
        t.set_math_char(0o100000);
        assert_eq!(t.tail(), before, "no noad");
        t.get_token();
        assert_eq!((t.cur_cmd, t.cur_chr, t.cur_cs), (RELAX, 256, ACTIVE_BASE + b'!' as i32));
        Tex::dealloc(t);
    }

    #[test]
    fn scan_delimiter_splits_the_code() {
        let t = math_machine();
        let p = t.get_node(1);
        for (code, want) in [
            (0, (0, 0, 0, 0)),
            (0x28300, (0, 0x28, 3, 0)),
            (0xFFFFFF, (15, 255, 15, 255)),
            (0x123456, (1, 0x23, 4, 0x56)),
        ] {
            t.set_del_code(b'(' as i32, code);
            feed(t, &[SPACE_TOKEN, OTHER_TOKEN + b'(' as i32]);
            t.scan_delimiter(p, false);
            assert_eq!((t.small_fam(p), t.small_char(p), t.large_fam(p), t.large_char(p)), want, "{code:x}");
        }
        Tex::dealloc(t);
    }

    #[test]
    fn scan_delimiter_refuses_with_the_oracles_message() {
        let t = math_machine();
        let p = t.get_node(1);
        t.set_small_fam(p, 9);
        // A character whose \delcode is -1, as `a` is in INITEX.
        feed(t, &[letter(b'a')]);
        t.scan_delimiter(p, false);
        let s = printed(t);
        assert!(s.contains("! Missing delimiter (. inserted)."), "got {s:?}");
        assert_eq!((t.small_fam(p), t.small_char(p), t.large_fam(p), t.large_char(p)), (0, 0, 0, 0));
        t.get_token();
        assert_eq!(t.cur_tok, letter(b'a'), "the token is read again");
        // A token of another kind altogether.
        feed(t, &[DOLLAR]);
        t.scan_delimiter(p, false);
        assert!(printed(t).contains("! Missing delimiter (. inserted)."));
        t.get_token();
        assert_eq!(t.cur_cmd, MATH_SHIFT);
        Tex::dealloc(t);
    }

    #[test]
    fn limit_switch_takes_an_operator_and_nothing_else() {
        let t = math_machine();
        // At the head of the list.
        t.cur_chr = LIMITS;
        t.math_limit_switch();
        assert!(printed(t).contains("! Limit controls must follow a math operator."));
        // After another noad.
        let q = noad(t, BIN_NOAD);
        append(t, q);
        t.math_limit_switch();
        assert!(printed(t).contains("! Limit controls must follow a math operator."));
        assert_eq!(t.subtype(q), NORMAL);
        // After an operator.
        let q = noad(t, OP_NOAD);
        append(t, q);
        for c in [LIMITS, NO_LIMITS, NORMAL] {
            t.cur_chr = c;
            t.math_limit_switch();
            assert_eq!(t.subtype(q), c);
        }
        assert_eq!(printed(t), "");
        Tex::dealloc(t);
    }

    #[test]
    fn fin_mlist_without_a_fraction_returns_the_list_with_p_on_the_end() {
        let t = math_machine();
        let a = noad(t, ORD_NOAD);
        append(t, a);
        let n = t.nest_ptr;
        assert_eq!(t.fin_mlist(NULL), a);
        assert_eq!(t.nest_ptr, n - 1);
        assert_eq!(t.link(a), NULL);
        t.push_math(MATH_SHIFT_GROUP);
        let b = noad(t, ORD_NOAD);
        append(t, a);
        assert_eq!(t.fin_mlist(b), a);
        assert_eq!(t.link(a), b);
        // Nothing at all but p: the head's link was null, and p is the list.
        t.push_math(MATH_SHIFT_GROUP);
        assert_eq!(t.fin_mlist(NULL), NULL);
        t.push_math(MATH_SHIFT_GROUP);
        assert_eq!(t.fin_mlist(b), b);
        Tex::dealloc(t);
    }

    #[test]
    fn fin_mlist_with_a_right_delimiter_keeps_the_left_outside_the_fraction() {
        // `\left( a \over b \right)`: math_left_right has put the \left noad at the head of the numerator,
        // with delim_ptr at it.  The list comes out as left, fraction, right.
        let t = math_machine();
        t.push_math(MATH_LEFT_GROUP);
        let l = noad(t, LEFT_NOAD);
        let a = noad(t, ORD_NOAD);
        t.set_link(t.head(), l);
        t.set_link(l, a);
        t.set_tail(a);
        t.set_delim_ptr(l);
        t.cur_chr = OVER_CODE;
        t.math_fraction();
        let f = t.incompleat_noad();
        let b = noad(t, ORD_NOAD);
        append(t, b);
        let r = noad(t, RIGHT_NOAD);
        assert_eq!(t.fin_mlist(r), l);
        assert_eq!(t.link(l), f);
        assert_eq!(t.link(f), r);
        assert_eq!(t.info(t.numerator(f)), a);
        assert_eq!(t.info(t.denominator(f)), b);
        Tex::dealloc(t);
    }

    #[test]
    fn math_fraction_takes_the_list_as_numerator() {
        let t = math_machine();
        let a = noad(t, ORD_NOAD);
        append(t, a);
        t.cur_chr = OVER_CODE;
        t.math_fraction();
        let f = t.incompleat_noad();
        assert_eq!((t.ty(f), t.subtype(f), t.thickness(f)), (FRACTION_NOAD, NORMAL, DEFAULT_CODE));
        assert_eq!((t.math_type(t.numerator(f)), t.info(t.numerator(f))), (SUB_MLIST, a));
        assert_eq!((t.math_type(t.denominator(f)), t.info(t.denominator(f))), (EMPTY, NULL));
        assert_eq!((t.tail(), t.link(t.head())), (t.head(), NULL));
        // fin_mlist completes it with what came after.
        let b = noad(t, ORD_NOAD);
        append(t, b);
        assert_eq!(t.fin_mlist(NULL), f);
        assert_eq!((t.math_type(t.denominator(f)), t.info(t.denominator(f))), (SUB_MLIST, b));
        Tex::dealloc(t);
    }

    #[test]
    fn math_fraction_with_delimiters_and_atop() {
        let t = math_machine();
        t.set_del_code(b'(' as i32, 0x12345);
        t.set_del_code(b')' as i32, 0x6789A);
        feed(t, &[OTHER_TOKEN + b'(' as i32, OTHER_TOKEN + b')' as i32]);
        t.cur_chr = ATOP_CODE + DELIMITED_CODE;
        t.math_fraction();
        let f = t.incompleat_noad();
        assert_eq!(t.thickness(f), 0);
        let (l, r) = (t.left_delimiter(f), t.right_delimiter(f));
        assert_eq!((t.small_fam(l), t.small_char(l), t.large_fam(l), t.large_char(l)), (0, 0x12, 3, 0x45));
        assert_eq!((t.small_fam(r), t.small_char(r), t.large_fam(r), t.large_char(r)), (0, 0x67, 8, 0x9A));
        Tex::dealloc(t);
    }

    #[test]
    fn a_second_fraction_is_ambiguous_and_still_reads_its_delimiters() {
        let t = math_machine();
        t.cur_chr = OVER_CODE;
        t.math_fraction();
        let f = t.incompleat_noad();
        t.set_del_code(b'(' as i32, 0x12345);
        feed(t, &[OTHER_TOKEN + b'(' as i32, OTHER_TOKEN + b'(' as i32, letter(b'z')]);
        t.cur_chr = OVER_CODE + DELIMITED_CODE;
        t.math_fraction();
        assert!(printed(t).contains("! Ambiguous; you need another { and }."));
        assert_eq!(t.incompleat_noad(), f);
        assert_eq!(t.thickness(f), DEFAULT_CODE);
        t.get_token();
        assert_eq!(t.cur_tok, letter(b'z'));
        Tex::dealloc(t);
    }

    #[test]
    fn extra_right_and_middle() {
        let t = math_machine();
        t.set_del_code(b')' as i32, 1);
        feed(t, &[OTHER_TOKEN + b')' as i32]);
        t.cur_chr = RIGHT_NOAD;
        t.math_left_right();
        assert!(printed(t).contains("! Extra \\right."));
        feed(t, &[OTHER_TOKEN + b')' as i32]);
        t.cur_chr = MIDDLE_NOAD;
        t.math_left_right();
        assert!(printed(t).contains("! Extra \\middle."));
        assert_eq!(t.tail(), t.head(), "nothing was added");
        Tex::dealloc(t);
    }

    #[test]
    fn sub_sup_fills_the_field_of_the_noad_before_it() {
        let t = math_machine();
        t.set_cur_fam(-1);
        t.set_math_code(b'a' as i32, 0x0161);
        t.set_math_code(b'b' as i32, 0x0262);
        let q = noad(t, ORD_NOAD);
        append(t, q);
        feed(t, &[letter(b'a')]);
        t.cur_cmd = SUP_MARK;
        t.sub_sup();
        assert_eq!(field(t, t.supscr(q)), (MATH_CHAR, 1, 0x61));
        feed(t, &[letter(b'b')]);
        t.cur_cmd = SUB_MARK;
        t.sub_sup();
        assert_eq!(field(t, t.subscr(q)), (MATH_CHAR, 2, 0x62));
        assert_eq!(t.tail(), q, "no dummy noad");
        Tex::dealloc(t);
    }

    #[test]
    fn build_choices_finishes_on_the_fourth() {
        let t = math_machine();
        let c = t.get_node(STYLE_NODE_SIZE);
        t.set_ty(c, CHOICE_NODE);
        append(t, c);
        t.save_ptr += 1;
        t.set_saved(-1, 3);
        t.push_math(MATH_CHOICE_GROUP);
        let a = noad(t, ORD_NOAD);
        append(t, a);
        let sp = t.save_ptr;
        t.build_choices();
        assert_eq!(t.script_script_mlist(c), a);
        assert_eq!(t.tail(), c);
        assert!(t.save_ptr < sp);
        assert_eq!(t.cur_group, MATH_SHIFT_GROUP);
        Tex::dealloc(t);
    }

    #[test]
    fn init_math_starts_ordinary_math_on_one_dollar_in_a_paragraph() {
        let t = table_machine();
        print_setup(t);
        t.set_mode(HMODE);
        feed(t, &[letter(b'z')]);
        t.init_math();
        assert_eq!((t.mode(), t.cur_fam(), t.cur_group), (-MMODE, -1, MATH_SHIFT_GROUP));
        t.get_token();
        assert_eq!(t.cur_tok, letter(b'z'));
        Tex::dealloc(t);
    }

    #[test]
    fn init_math_with_every_math_reads_it_before_the_formula() {
        let t = table_machine();
        print_setup(t);
        t.set_mode(HMODE);
        let m = t.tb_mark();
        t.tb_push(letter(b'e'));
        let p = t.tb_commit(m);
        t.set_every_math(p);
        feed(t, &[letter(b'z')]);
        t.init_math();
        t.get_token();
        assert_eq!(t.cur_tok, letter(b'e'));
        t.get_token();
        assert_eq!(t.cur_tok, letter(b'z'));
        Tex::dealloc(t);
    }

    #[test]
    fn init_math_in_restricted_mode_is_ordinary_on_two_dollars() {
        let t = table_machine();
        print_setup(t);
        t.set_mode(-HMODE);
        feed(t, &[DOLLAR]);
        t.init_math();
        assert_eq!(t.mode(), -MMODE);
        t.get_token();
        assert_eq!(t.cur_cmd, MATH_SHIFT);
        Tex::dealloc(t);
    }

    /// Section 1327 against the oracle: the display after an empty paragraph, in a \vbox.
    #[test]
    fn display_shape_against_the_oracle() {
        let rows = table(include_str!("../testdata/mathbuild_shape_oracle.tsv"));
        assert!(rows.len() > 70);
        for r in rows {
            let v: Vec<i32> = [&r[0], &r[1], &r[2], &r[3], &r[5], &r[6], &r[7]].iter().map(|f| f.parse().unwrap()).collect();
            let t = table_machine();
            print_setup(t);
            t.set_hsize(v[0]);
            t.set_hang_indent(v[1]);
            t.set_hang_after(v[2]);
            if r[4] != "-" {
                let pairs: Vec<&str> = r[4].split(';').collect();
                let n = pairs.len() as i32;
                let p = t.get_node(2 * n + 1);
                t.set_info(p, n);
                for (j, pr) in pairs.iter().enumerate() {
                    let j = j as i32 + 1;
                    let mut f = pr.split(' ').map(|x| x.parse::<i32>().unwrap());
                    t.mem[p + 2 * j - 1].set_sc(f.next().unwrap());
                    t.mem[p + 2 * j].set_sc(f.next().unwrap());
                }
                t.set_par_shape_ptr(p);
            }
            // The \vbox, and the empty paragraph in it.
            t.set_mode(VMODE);
            t.push_nest();
            t.set_mode(-VMODE);
            t.new_save_level(VBOX_GROUP);
            t.set_prev_graf(v[3]);
            t.push_nest();
            t.set_mode(HMODE);
            t.set_space_factor(1000);
            feed(t, &[DOLLAR]);
            t.init_math();
            assert_eq!(t.mode(), MMODE, "{r:?}");
            assert_eq!((t.display_width(), t.display_indent(), t.pre_display_size()), (v[4], v[5], v[6]), "{r:?}");
            assert_eq!(t.nest_ptr, 2, "the empty paragraph was popped, and the display pushed");
            Tex::dealloc(t);
        }
    }

    #[test]
    fn start_eq_no_saves_which_of_eqno_and_leqno_it_was() {
        let t = math_machine();
        for c in [0, 1] {
            t.cur_chr = c;
            let sp = t.save_ptr;
            let n = t.nest_ptr;
            t.start_eq_no();
            assert_eq!((t.mode(), t.nest_ptr, t.cur_group, t.cur_fam()), (-MMODE, n + 1, MATH_SHIFT_GROUP, -1));
            t.unsave();
            t.pop_nest();
            t.save_ptr -= 1;
            assert_eq!(t.save_ptr, sp);
            assert_eq!(t.saved(0), c);
        }
        Tex::dealloc(t);
    }
}
