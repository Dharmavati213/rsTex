//! §1206-1232 Part 46: the chief executive.
//!
//! `main_control` is one function, a state machine over WEB's labels (PORTING section 9), with every
//! `@<Cases of |main_control|...@>` in place.  `Pc::Reswitch` is the big `case`; what follows it is the character
//! path of §1211.  MLTeX's `effective_char` is the identity here, and src-specials' tests are dropped (PORTING
//! section 19), each marked where the line went away.
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

/// The labels of §1207.
#[derive(Clone, Copy)]
enum Pc {
    BigSwitch,
    Reswitch,
    MainLoop,
    MainLoopWrapup,
    MainLoopMove,
    MainLoopMove1,
    MainLoopMove2,
    MainLoopMoveLig,
    MainLoopLookahead,
    MainLoopLookahead1,
    MainLigLoop,
    MainLigLoop1,
    MainLigLoop2,
    AppendNormalSpace,
}

impl Tex {
    /// §1207
    pub fn main_control(&mut self) {
        // A run restored from a checkpoint comes here with \everyjob behind it.
        if self.every_job() != NULL && !self.v_image.ck.resumed {
            self.begin_token_list(self.every_job(), EVERY_JOB_TEXT);
        }
        let mut pc = Pc::BigSwitch;
        loop {
            match pc {
                Pc::BigSwitch => {
                    if self.v_image.ck.page_due {
                        self.pages_take();
                    }
                    // The one fetch a restore can resume at: crate::checkpoint.
                    self.v_image.ck.at_big_switch = true;
                    self.get_x_token();
                    self.v_image.ck.at_big_switch = false;
                    pc = Pc::Reswitch;
                }
                Pc::Reswitch => {
                    // §1208 Give diagnostic information, if requested
                    if self.interrupt != 0 && self.OK_to_interrupt {
                        self.back_input();
                        self.check_interrupt();
                        pc = Pc::BigSwitch;
                        continue;
                    }
                    if self.tracing_commands() > 0 {
                        self.show_cur_cmd_chr();
                    }
                    // The `goto big_switch` that follows the case, unless an arm goes elsewhere.
                    pc = Pc::BigSwitch;
                    match (abs(self.mode()), self.cur_cmd) {
                        (HMODE, LETTER) | (HMODE, OTHER_CHAR) | (HMODE, CHAR_GIVEN) => pc = Pc::MainLoop,
                        (HMODE, CHAR_NUM) => {
                            self.scan_char_num();
                            self.cur_chr = self.cur_val;
                            pc = Pc::MainLoop;
                        }
                        (HMODE, NO_BOUNDARY) => {
                            self.get_x_token();
                            if self.cur_cmd == LETTER
                                || self.cur_cmd == OTHER_CHAR
                                || self.cur_cmd == CHAR_GIVEN
                                || self.cur_cmd == CHAR_NUM
                            {
                                self.cancel_boundary = true;
                            }
                            pc = Pc::Reswitch;
                        }
                        (HMODE, SPACER) => {
                            if self.space_factor() == 1000 || self.pdf_adjust_interword_glue() > 0 {
                                pc = Pc::AppendNormalSpace;
                            } else {
                                self.app_space();
                            }
                        }
                        (HMODE, EX_SPACE) | (MMODE, EX_SPACE) => pc = Pc::AppendNormalSpace,

                        // §1223 Cases of main_control that are not part of the inner loop
                        (_, RELAX) | (VMODE, SPACER) | (MMODE, SPACER) | (MMODE, NO_BOUNDARY) => {}
                        (_, IGNORE_SPACES) => {
                            if self.cur_chr == 0 {
                                // §432 Get the next non-blank non-call token
                                loop {
                                    self.get_x_token();
                                    if self.cur_cmd != SPACER {
                                        break;
                                    }
                                }
                                pc = Pc::Reswitch;
                            } else {
                                let t = self.scanner_status;
                                self.scanner_status = NORMAL;
                                self.get_next();
                                self.scanner_status = t;
                                if self.cur_cs < HASH_BASE {
                                    self.cur_cs = self.prim_lookup(self.cur_cs - SINGLE_BASE);
                                } else {
                                    self.cur_cs = self.prim_lookup(self.text(self.cur_cs));
                                }
                                if self.cur_cs != UNDEFINED_PRIMITIVE {
                                    self.cur_cmd = self.prim_eq_type(self.cur_cs);
                                    self.cur_chr = self.prim_equiv(self.cur_cs);
                                    self.cur_tok = CS_TOKEN_FLAG + PRIM_EQTB_BASE + self.cur_cs;
                                    pc = Pc::Reswitch;
                                }
                            }
                        }
                        (VMODE, STOP) => {
                            // this is the only way out
                            if self.its_all_over() {
                                return;
                            }
                        }
                        // §1226, §1276, §1289, §1322 Forbidden cases detected in main_control
                        (VMODE, VMOVE)
                        | (HMODE, HMOVE)
                        | (MMODE, HMOVE)
                        | (_, LAST_ITEM)
                        | (VMODE, VADJUST)
                        | (VMODE, ITAL_CORR)
                        | (VMODE, EQ_NO)
                        | (HMODE, EQ_NO)
                        | (_, MAC_PARAM) => self.report_illegal_case(),
                        // §1224 Math-only cases in non-math modes, or vice versa
                        (VMODE, SUP_MARK) | (HMODE, SUP_MARK)
                        | (VMODE, SUB_MARK) | (HMODE, SUB_MARK)
                        | (VMODE, MATH_CHAR_NUM) | (HMODE, MATH_CHAR_NUM)
                        | (VMODE, MATH_GIVEN) | (HMODE, MATH_GIVEN)
                        | (VMODE, MATH_COMP) | (HMODE, MATH_COMP)
                        | (VMODE, DELIM_NUM) | (HMODE, DELIM_NUM)
                        | (VMODE, LEFT_RIGHT) | (HMODE, LEFT_RIGHT)
                        | (VMODE, ABOVE) | (HMODE, ABOVE)
                        | (VMODE, RADICAL) | (HMODE, RADICAL)
                        | (VMODE, MATH_STYLE) | (HMODE, MATH_STYLE)
                        | (VMODE, MATH_CHOICE) | (HMODE, MATH_CHOICE)
                        | (VMODE, VCENTER) | (HMODE, VCENTER)
                        | (VMODE, NON_SCRIPT) | (HMODE, NON_SCRIPT)
                        | (VMODE, MKERN) | (HMODE, MKERN)
                        | (VMODE, LIMIT_SWITCH) | (HMODE, LIMIT_SWITCH)
                        | (VMODE, MSKIP) | (HMODE, MSKIP)
                        | (VMODE, MATH_ACCENT) | (HMODE, MATH_ACCENT)
                        | (MMODE, ENDV)
                        | (MMODE, PAR_END)
                        | (MMODE, STOP)
                        | (MMODE, VSKIP)
                        | (MMODE, UN_VBOX)
                        | (MMODE, VALIGN)
                        | (MMODE, HRULE) => self.insert_dollar_sign(),
                        // §1234 Cases of main_control that build boxes and lists
                        (VMODE, HRULE) | (HMODE, VRULE) | (MMODE, VRULE) => {
                            let p = self.scan_rule_spec();
                            self.tail_append(p);
                            if abs(self.mode()) == VMODE {
                                self.set_prev_depth(self.pdf_ignored_dimen());
                            } else if abs(self.mode()) == HMODE {
                                self.set_space_factor(1000);
                            }
                        }
                        // §1235
                        (VMODE, VSKIP) | (HMODE, HSKIP) | (MMODE, HSKIP) | (MMODE, MSKIP) => self.append_glue(),
                        (_, KERN) | (MMODE, MKERN) => self.append_kern(),
                        // §1241
                        (VMODE, LEFT_BRACE) | (HMODE, LEFT_BRACE) => self.new_save_level(SIMPLE_GROUP),
                        (_, BEGIN_GROUP) => self.new_save_level(SEMI_SIMPLE_GROUP),
                        (_, END_GROUP) => {
                            if self.cur_group == SEMI_SIMPLE_GROUP {
                                self.unsave();
                            } else {
                                self.off_save();
                            }
                        }
                        // §1245
                        (_, RIGHT_BRACE) => self.handle_right_brace(),
                        // §1251
                        (VMODE, HMOVE) | (HMODE, VMOVE) | (MMODE, VMOVE) => {
                            let t = self.cur_chr;
                            self.scan_normal_dimen();
                            if t == 0 {
                                self.scan_box(self.cur_val);
                            } else {
                                self.scan_box(-self.cur_val);
                            }
                        }
                        (_, LEADER_SHIP) => self.scan_box(LEADER_FLAG - A_LEADERS + self.cur_chr),
                        (_, MAKE_BOX) => self.begin_box(0),
                        // §1268
                        (VMODE, START_PAR) => self.new_graf(self.cur_chr > 0),
                        (VMODE, LETTER)
                        | (VMODE, OTHER_CHAR)
                        | (VMODE, CHAR_NUM)
                        | (VMODE, CHAR_GIVEN)
                        | (VMODE, MATH_SHIFT)
                        | (VMODE, UN_HBOX)
                        | (VMODE, VRULE)
                        | (VMODE, ACCENT)
                        | (VMODE, DISCRETIONARY)
                        | (VMODE, HSKIP)
                        | (VMODE, VALIGN)
                        | (VMODE, EX_SPACE)
                        | (VMODE, NO_BOUNDARY) => {
                            self.back_input();
                            self.new_graf(true);
                        }
                        // §1270
                        (HMODE, START_PAR) | (MMODE, START_PAR) => {
                            if self.cur_chr != 2 {
                                self.indent_in_hmode();
                            }
                        }
                        // §1272
                        (VMODE, PAR_END) => {
                            self.normal_paragraph();
                            if self.mode() > 0 {
                                self.build_page();
                            }
                        }
                        (HMODE, PAR_END) => {
                            // this tries to recover from an alignment that didn't end properly
                            if self.align_state < 0 {
                                self.off_save();
                            }
                            // this takes us to the enclosing mode, if mode>0
                            self.end_graf();
                            if self.mode() == VMODE {
                                self.build_page();
                            }
                        }
                        (HMODE, STOP) | (HMODE, VSKIP) | (HMODE, HRULE) | (HMODE, UN_VBOX) | (HMODE, HALIGN) => {
                            self.head_for_vmode();
                        }
                        // §1275
                        (_, INSERT) | (HMODE, VADJUST) | (MMODE, VADJUST) => self.begin_insert_or_adjust(),
                        (_, MARK) => self.make_mark(),
                        // §1280
                        (_, BREAK_PENALTY) => self.append_penalty(),
                        // §1282
                        (_, REMOVE_ITEM) => self.delete_last(),
                        // §1287
                        (VMODE, UN_VBOX) | (HMODE, UN_HBOX) | (MMODE, UN_HBOX) => self.unpackage(),
                        // §1290
                        (HMODE, ITAL_CORR) => self.append_italic_correction(),
                        (MMODE, ITAL_CORR) => {
                            let p = self.new_kern(0);
                            self.tail_append(p);
                        }
                        // §1294
                        (HMODE, DISCRETIONARY) | (MMODE, DISCRETIONARY) => self.append_discretionary(),
                        // §1300
                        (HMODE, ACCENT) => self.make_accent(),
                        // §1304
                        (_, CAR_RET) | (_, TAB_MARK) => self.align_error(),
                        (_, NO_ALIGN) => self.no_align_error(),
                        (_, OMIT) => self.omit_error(),
                        // §1308
                        (VMODE, HALIGN) => self.init_align(),
                        (HMODE, VALIGN) => {
                            // §1703 Cases of main_control for hmode+valign
                            if self.cur_chr > 0 {
                                if self.eTeX_enabled(self.TeXXeT_en(), self.cur_cmd, self.cur_chr) {
                                    let p = self.new_math(0, self.cur_chr);
                                    self.tail_append(p);
                                }
                            } else {
                                self.init_align();
                            }
                        }
                        (MMODE, HALIGN) => {
                            if self.privileged() {
                                if self.cur_group == MATH_SHIFT_GROUP {
                                    self.init_align();
                                } else {
                                    self.off_save();
                                }
                            }
                        }
                        (VMODE, ENDV) | (HMODE, ENDV) => {
                            if self.partoken_context() > 1 && self.mode() == HMODE {
                                self.back_input();
                                self.cur_tok = self.par_token;
                                self.back_input();
                                self.set_token_type(INSERTED);
                            } else {
                                self.do_endv();
                            }
                        }
                        // §1312
                        (_, END_CS_NAME) => self.cs_error(),
                        // §1315
                        (HMODE, MATH_SHIFT) => self.init_math(),
                        // §1318
                        (MMODE, EQ_NO) => {
                            if self.privileged() {
                                if self.cur_group == MATH_SHIFT_GROUP {
                                    self.start_eq_no();
                                } else {
                                    self.off_save();
                                }
                            }
                        }
                        // §1328
                        (MMODE, LEFT_BRACE) => {
                            let p = self.new_noad();
                            self.tail_append(p);
                            self.back_input();
                            self.scan_math(self.nucleus(self.tail()));
                        }
                        // §1332
                        (MMODE, LETTER) | (MMODE, OTHER_CHAR) | (MMODE, CHAR_GIVEN) => {
                            self.set_math_char(self.math_code(self.cur_chr));
                        }
                        (MMODE, CHAR_NUM) => {
                            self.scan_char_num();
                            self.cur_chr = self.cur_val;
                            self.set_math_char(self.math_code(self.cur_chr));
                        }
                        (MMODE, MATH_CHAR_NUM) => {
                            self.scan_fifteen_bit_int();
                            self.set_math_char(self.cur_val);
                        }
                        (MMODE, MATH_GIVEN) => self.set_math_char(self.cur_chr),
                        (MMODE, DELIM_NUM) => {
                            self.scan_twenty_seven_bit_int();
                            self.set_math_char(self.cur_val / 0o10000);
                        }
                        // §1336
                        (MMODE, MATH_COMP) => {
                            let p = self.new_noad();
                            self.tail_append(p);
                            self.set_ty(self.tail(), self.cur_chr);
                            self.scan_math(self.nucleus(self.tail()));
                        }
                        (MMODE, LIMIT_SWITCH) => self.math_limit_switch(),
                        // §1340
                        (MMODE, RADICAL) => self.math_radical(),
                        // §1342
                        (MMODE, ACCENT) | (MMODE, MATH_ACCENT) => self.math_ac(),
                        // §1345
                        (MMODE, VCENTER) => {
                            self.scan_spec(VCENTER_GROUP, false);
                            self.normal_paragraph();
                            self.push_nest();
                            self.set_mode(-VMODE);
                            self.set_prev_depth(self.pdf_ignored_dimen());
                            // dropped: src-specials
                            if self.every_vbox() != NULL {
                                self.begin_token_list(self.every_vbox(), EVERY_VBOX_TEXT);
                            }
                        }
                        // §1349
                        (MMODE, MATH_STYLE) => {
                            let p = self.new_style(self.cur_chr);
                            self.tail_append(p);
                        }
                        (MMODE, NON_SCRIPT) => {
                            let p = self.new_glue(ZERO_GLUE);
                            self.tail_append(p);
                            self.set_subtype(self.tail(), COND_MATH_GLUE);
                        }
                        (MMODE, MATH_CHOICE) => self.append_choices(),
                        // §1353
                        (MMODE, SUB_MARK) | (MMODE, SUP_MARK) => self.sub_sup(),
                        // §1358
                        (MMODE, ABOVE) => self.math_fraction(),
                        // §1368
                        (MMODE, LEFT_RIGHT) => self.math_left_right(),
                        // §1371
                        (MMODE, MATH_SHIFT) => {
                            if self.cur_group == MATH_SHIFT_GROUP {
                                self.after_math();
                            } else {
                                self.off_save();
                            }
                        }
                        // §1388 Cases of main_control that don't depend on mode
                        (_, TOKS_REGISTER)
                        | (_, ASSIGN_TOKS)
                        | (_, ASSIGN_INT)
                        | (_, ASSIGN_DIMEN)
                        | (_, ASSIGN_GLUE)
                        | (_, ASSIGN_MU_GLUE)
                        | (_, ASSIGN_FONT_DIMEN)
                        | (_, ASSIGN_FONT_INT)
                        | (_, SET_AUX)
                        | (_, SET_PREV_GRAF)
                        | (_, SET_PAGE_DIMEN)
                        | (_, SET_PAGE_INT)
                        | (_, SET_BOX_DIMEN)
                        | (_, SET_SHAPE)
                        | (_, DEF_CODE)
                        | (_, DEF_FAMILY)
                        | (_, SET_FONT)
                        | (_, DEF_FONT)
                        | (_, LETTERSPACE_FONT)
                        | (_, PDF_COPY_FONT)
                        | (_, REGISTER)
                        | (_, ADVANCE)
                        | (_, MULTIPLY)
                        | (_, DIVIDE)
                        | (_, PREFIX)
                        | (_, LET)
                        | (_, SHORTHAND_DEF)
                        | (_, READ_TO_CS)
                        | (_, DEF)
                        | (_, SET_BOX)
                        | (_, HYPH_DATA)
                        | (_, SET_INTERACTION) => self.prefixed_command(),
                        // §1446
                        (_, AFTER_ASSIGNMENT) => {
                            self.get_token();
                            self.after_token = self.cur_tok;
                        }
                        // §1449
                        (_, AFTER_GROUP) => {
                            self.get_token();
                            self.save_for_after(self.cur_tok);
                        }
                        (_, PARTOKEN_NAME) => {
                            self.get_token();
                            if self.cur_cs > 0 {
                                self.par_loc = self.cur_cs;
                                self.par_token = self.cur_tok;
                            }
                        }
                        // §1452
                        (_, IN_STREAM) => self.open_or_close_in(),
                        // §1454
                        (_, MESSAGE) => self.issue_message(),
                        // §1463
                        (_, CASE_SHIFT) => self.shift_case(),
                        // §1468
                        (_, XRAY) => self.show_whatever(),
                        // §1527 Cases of main_control that are for extensions to TeX
                        (_, EXTENSION) => self.do_extension(),
                        _ => {}
                    }
                }
                Pc::MainLoop => {
                    // §1211 Append character cur_chr and the following characters (if any) to the current hlist
                    // dropped: src-specials
                    self.adjust_space_factor();
                    self.save_tail = NULL;
                    self.main_f = self.cur_font();
                    self.bchar = self.font_bchar[self.main_f];
                    self.false_bchar = self.font_false_bchar[self.main_f];
                    if self.mode() > 0 && self.language() != self.clang() {
                        self.fix_language();
                    }
                    self.lig_stack = self.fast_get_avail();
                    self.set_font(self.lig_stack, self.main_f);
                    self.cur_l = self.cur_chr;
                    self.set_character(self.lig_stack, self.cur_l);
                    self.cur_q = self.tail();
                    let tmp_k1 = self.get_auto_kern(self.main_f, NON_CHAR, self.cur_l);
                    if tmp_k1 != NULL {
                        self.control_append_auto_kern(tmp_k1);
                        pc = Pc::MainLoopMove;
                        continue;
                    }
                    if self.cancel_boundary {
                        self.cancel_boundary = false;
                        self.main_k = NON_ADDRESS;
                    } else {
                        self.main_k = self.bchar_label[self.main_f];
                    }
                    if self.main_k == NON_ADDRESS {
                        // no left boundary processing
                        pc = Pc::MainLoopMove2;
                        continue;
                    }
                    self.cur_r = self.cur_l;
                    self.cur_l = NON_CHAR;
                    // begin with cursor after left boundary
                    pc = Pc::MainLigLoop1;
                }
                Pc::MainLoopWrapup => {
                    // §1212 Make a ligature node, if ligature_present; insert a null discretionary, if appropriate
                    self.wrapup(self.rt_hit);
                    pc = Pc::MainLoopMove;
                }
                Pc::MainLoopMove => {
                    // §1213 If the cursor is immediately followed by the right boundary, goto reswitch; if it's
                    // followed by an invalid character, goto big_switch; otherwise move the cursor one step to the
                    // right and goto main_lig_loop
                    if self.lig_stack == NULL {
                        pc = Pc::Reswitch;
                        continue;
                    }
                    self.cur_q = self.tail();
                    self.cur_l = self.character(self.lig_stack);
                    pc = Pc::MainLoopMove1;
                }
                Pc::MainLoopMove1 => {
                    pc = if !self.is_char_node(self.lig_stack) { Pc::MainLoopMoveLig } else { Pc::MainLoopMove2 };
                }
                Pc::MainLoopMove2 => {
                    // dropped: MLTeX.  effective_char(false,main_f,cur_chr) is cur_chr.
                    if self.cur_chr > self.font_ec[self.main_f] || self.cur_chr < self.font_bc[self.main_f] {
                        self.char_warning(self.main_f, self.cur_chr);
                        self.free_avail(self.lig_stack);
                        pc = Pc::BigSwitch;
                        continue;
                    }
                    // dropped: MLTeX.  effective_char_info is char_info.
                    self.main_i = self.char_info(self.main_f, self.cur_l);
                    if !self.char_exists(self.main_i) {
                        self.char_warning(self.main_f, self.cur_chr);
                        self.free_avail(self.lig_stack);
                        pc = Pc::BigSwitch;
                        continue;
                    }
                    self.set_link(self.tail(), self.lig_stack);
                    self.set_tail(self.lig_stack);
                    pc = Pc::MainLoopLookahead;
                }
                Pc::MainLoopLookahead => {
                    // §1215 Look ahead for another character, or leave lig_stack empty
                    self.get_next(); // set only cur_cmd and cur_chr, for speed
                    if self.cur_cmd == LETTER || self.cur_cmd == OTHER_CHAR || self.cur_cmd == CHAR_GIVEN {
                        pc = Pc::MainLoopLookahead1;
                        continue;
                    }
                    self.x_token(); // now expand and set cur_cmd, cur_chr, cur_tok
                    if self.cur_cmd == LETTER || self.cur_cmd == OTHER_CHAR || self.cur_cmd == CHAR_GIVEN {
                        pc = Pc::MainLoopLookahead1;
                        continue;
                    }
                    if self.cur_cmd == CHAR_NUM {
                        self.scan_char_num();
                        self.cur_chr = self.cur_val;
                        pc = Pc::MainLoopLookahead1;
                        continue;
                    }
                    if self.cur_cmd == NO_BOUNDARY {
                        self.bchar = NON_CHAR;
                    }
                    self.cur_r = self.bchar;
                    self.lig_stack = NULL;
                    pc = Pc::MainLigLoop;
                }
                Pc::MainLoopLookahead1 => {
                    self.adjust_space_factor();
                    self.lig_stack = self.fast_get_avail();
                    self.set_font(self.lig_stack, self.main_f);
                    self.cur_r = self.cur_chr;
                    self.set_character(self.lig_stack, self.cur_r);
                    if self.cur_r == self.false_bchar {
                        self.cur_r = NON_CHAR; // this prevents spurious ligatures
                    }
                    pc = Pc::MainLigLoop;
                }
                Pc::MainLigLoop => {
                    // §1216 If there's a ligature/kern command relevant to cur_l and cur_r, adjust the text
                    // appropriately; exit to main_loop_wrapup
                    let tmp_k1 = self.get_auto_kern(self.main_f, self.cur_l, self.cur_r);
                    if tmp_k1 != NULL {
                        self.control_append_auto_kern(tmp_k1);
                        pc = Pc::MainLoopMove;
                        continue;
                    }
                    if self.char_tag(self.main_i) != LIG_TAG || self.cur_r == NON_CHAR {
                        pc = Pc::MainLoopWrapup;
                        continue;
                    }
                    self.main_k = self.lig_kern_start(self.main_f, self.main_i);
                    self.main_j = self.font_info[self.main_k].qqqq();
                    if self.skip_byte(self.main_j) <= STOP_FLAG {
                        pc = Pc::MainLigLoop2;
                        continue;
                    }
                    self.main_k = self.lig_kern_restart(self.main_f, self.main_j);
                    pc = Pc::MainLigLoop1;
                }
                Pc::MainLigLoop1 => {
                    self.main_j = self.font_info[self.main_k].qqqq();
                    pc = Pc::MainLigLoop2;
                }
                Pc::MainLigLoop2 => {
                    if self.next_char(self.main_j) == self.cur_r && self.skip_byte(self.main_j) <= STOP_FLAG {
                        // §1218 Do ligature or kern command, returning to main_lig_loop or main_loop_wrapup or
                        // main_loop_move
                        if self.op_byte(self.main_j) >= KERN_FLAG {
                            self.wrapup(self.rt_hit);
                            let k = self.new_kern(self.char_kern(self.main_f, self.main_j));
                            self.tail_append(k);
                            pc = Pc::MainLoopMove;
                            continue;
                        }
                        if self.cur_l == NON_CHAR {
                            self.lft_hit = true;
                        } else if self.lig_stack == NULL {
                            self.rt_hit = true;
                        }
                        self.check_interrupt(); // allow a way out in case there's an infinite ligature loop
                        match self.op_byte(self.main_j) {
                            // =:|, =:|>
                            1 | 5 => {
                                self.cur_l = self.rem_byte(self.main_j);
                                self.main_i = self.char_info(self.main_f, self.cur_l);
                                self.ligature_present = true;
                            }
                            // |=:, |=:>
                            2 | 6 => {
                                self.cur_r = self.rem_byte(self.main_j);
                                if self.lig_stack == NULL {
                                    // right boundary character is being consumed
                                    self.lig_stack = self.new_lig_item(self.cur_r);
                                    self.bchar = NON_CHAR;
                                } else if self.is_char_node(self.lig_stack) {
                                    // link(lig_stack)=null
                                    self.main_p = self.lig_stack;
                                    self.lig_stack = self.new_lig_item(self.cur_r);
                                    self.set_lig_ptr(self.lig_stack, self.main_p);
                                } else {
                                    self.set_character(self.lig_stack, self.cur_r);
                                }
                            }
                            // |=:|
                            3 => {
                                self.cur_r = self.rem_byte(self.main_j);
                                self.main_p = self.lig_stack;
                                self.lig_stack = self.new_lig_item(self.cur_r);
                                self.set_link(self.lig_stack, self.main_p);
                            }
                            // |=:|>, |=:|>>
                            7 | 11 => {
                                self.wrapup(false);
                                self.cur_q = self.tail();
                                self.cur_l = self.rem_byte(self.main_j);
                                self.main_i = self.char_info(self.main_f, self.cur_l);
                                self.ligature_present = true;
                            }
                            // =:
                            _ => {
                                self.cur_l = self.rem_byte(self.main_j);
                                self.ligature_present = true;
                                pc = if self.lig_stack == NULL { Pc::MainLoopWrapup } else { Pc::MainLoopMove1 };
                                continue;
                            }
                        }
                        if self.op_byte(self.main_j) > 4 && self.op_byte(self.main_j) != 7 {
                            pc = Pc::MainLoopWrapup;
                            continue;
                        }
                        if self.cur_l < NON_CHAR {
                            pc = Pc::MainLigLoop;
                            continue;
                        }
                        self.main_k = self.bchar_label[self.main_f];
                        pc = Pc::MainLigLoop1;
                        continue;
                    }
                    if self.skip_byte(self.main_j) == 0 {
                        self.main_k += 1;
                    } else {
                        if self.skip_byte(self.main_j) >= STOP_FLAG {
                            pc = Pc::MainLoopWrapup;
                            continue;
                        }
                        self.main_k = self.main_k + self.skip_byte(self.main_j) + 1;
                    }
                    pc = Pc::MainLigLoop1;
                }
                Pc::MainLoopMoveLig => {
                    // §1214 Move the cursor past a pseudo-ligature, then goto main_loop_lookahead or main_lig_loop
                    self.main_p = self.lig_ptr(self.lig_stack);
                    if self.main_p > NULL {
                        self.tail_append(self.main_p); // append a single character
                    }
                    self.temp_ptr = self.lig_stack;
                    self.lig_stack = self.link(self.temp_ptr);
                    self.free_node(self.temp_ptr, SMALL_NODE_SIZE);
                    self.main_i = self.char_info(self.main_f, self.cur_l);
                    self.ligature_present = true;
                    if self.lig_stack == NULL {
                        if self.main_p > NULL {
                            pc = Pc::MainLoopLookahead;
                            continue;
                        }
                        self.cur_r = self.bchar;
                    } else {
                        self.cur_r = self.character(self.lig_stack);
                    }
                    pc = Pc::MainLigLoop;
                }
                Pc::AppendNormalSpace => {
                    // §1219 Append a normal inter-word space to the current list, then goto big_switch
                    if self.space_skip() == ZERO_GLUE {
                        self.control_find_space_glue();
                        self.temp_ptr = self.new_glue(self.main_p);
                    } else {
                        self.temp_ptr = self.new_param_glue(SPACE_SKIP_CODE);
                    }
                    if self.pdf_adjust_interword_glue() > 0 {
                        self.adjust_interword_glue(self.tail(), self.temp_ptr);
                    }
                    self.set_link(self.tail(), self.temp_ptr);
                    self.set_tail(self.temp_ptr);
                    pc = Pc::BigSwitch;
                }
            }
        }
    }

    /// §1212 `pack_lig(#)`.  `tmp_k2` is local here: nothing else in main_control reads it.
    fn pack_lig(&mut self, z: bool) {
        self.main_p = self.new_ligature(self.main_f, self.cur_l, self.link(self.cur_q));
        if self.lft_hit {
            self.set_subtype(self.main_p, 2);
            self.lft_hit = false;
        }
        if z && self.lig_stack == NULL {
            self.set_subtype(self.main_p, self.subtype(self.main_p) + 1);
            self.rt_hit = false;
        }
        let tmp_k2 = if self.pdf_prepend_kern() > 0 {
            self.get_auto_kern(self.main_f, NON_CHAR, self.cur_l)
        } else {
            NULL
        };
        if tmp_k2 == NULL {
            self.set_link(self.cur_q, self.main_p);
        } else {
            self.set_link(self.cur_q, tmp_k2);
            self.set_link(tmp_k2, self.main_p);
        }
        self.set_tail(self.main_p);
        self.ligature_present = false;
    }

    /// §1212 `wrapup(#)`
    fn wrapup(&mut self, z: bool) {
        if self.cur_l < NON_CHAR {
            if self.link(self.cur_q) > NULL && self.character(self.tail()) == self.hyphen_char[self.main_f] {
                self.ins_disc = true;
            }
            if self.ligature_present {
                self.pack_lig(z);
            }
            if self.ins_disc {
                self.ins_disc = false;
                if self.mode() > 0 {
                    let d = self.new_disc();
                    self.tail_append(d);
                }
            }
        }
    }

    /// §1217 If tmp_k1 is not null then append that kern.  The caller has the test and the `goto main_loop_move`.
    fn control_append_auto_kern(&mut self, tmp_k1: i32) {
        self.wrapup(self.rt_hit); // wrapup might insert a null discretionary
        self.save_tail = self.tail();
        // insert auto-kern before a null discretionary inserted by wrapup if appropriate
        let t = self.tail();
        if !self.is_char_node(t)
            && self.ty(t) == DISC_NODE
            && self.replace_count(t) == 0
            && self.pre_break(t) == NULL
            && self.post_break(t) == NULL
            && self.link(self.prev_tail) == t
        {
            self.insert_before_tail(tmp_k1);
        } else {
            self.tail_append(tmp_k1);
        }
    }

    /// §1220 Find the glue specification, main_p, for text spaces in the current font
    fn control_find_space_glue(&mut self) {
        self.main_p = self.font_glue[self.cur_font()];
        if self.main_p == NULL {
            self.main_p = self.new_spec(ZERO_GLUE);
            self.main_k = self.param_base[self.cur_font()] + SPACE_CODE;
            self.set_width(self.main_p, self.font_info[self.main_k].sc()); // that's space(cur_font)
            self.set_stretch(self.main_p, self.font_info[self.main_k + 1].sc()); // and space_stretch(cur_font)
            self.set_shrink(self.main_p, self.font_info[self.main_k + 2].sc()); // and space_shrink(cur_font)
            let f = self.cur_font();
            self.font_glue[f] = self.main_p;
        }
    }

    /// §1221
    pub fn app_space(&mut self) {
        let q;
        if self.space_factor() >= 2000 && self.xspace_skip() != ZERO_GLUE {
            q = self.new_param_glue(XSPACE_SKIP_CODE);
        } else {
            if self.space_skip() != ZERO_GLUE {
                self.main_p = self.space_skip();
            } else {
                self.control_find_space_glue();
            }
            self.main_p = self.new_spec(self.main_p);
            // §1222 Modify the glue specification in main_p according to the space factor
            if self.space_factor() >= 2000 {
                // A \spaceskip built with \advance can be near 2^31: PORTING section 11.
                let w = self.width(self.main_p) as i64 + self.extra_space(self.cur_font()) as i64;
                self.set_width(self.main_p, sat32(w));
            }
            let s = self.xn_over_d(self.stretch(self.main_p), self.space_factor(), 1000);
            self.set_stretch(self.main_p, s);
            let s = self.xn_over_d(self.shrink(self.main_p), 1000, self.space_factor());
            self.set_shrink(self.main_p, s);
            q = self.new_glue(self.main_p);
            self.set_glue_ref_count(self.main_p, NULL);
        }
        self.set_link(self.tail(), q);
        self.set_tail(q);
    }

    /// §1225
    #[cold]
    pub fn insert_dollar_sign(&mut self) {
        self.back_input();
        self.cur_tok = MATH_SHIFT_TOKEN + b'$' as i32;
        self.print_err_s(b"Missing $ inserted");
        self.help(&[
            b"I've inserted a begin-math/end-math symbol since I think",
            b"you left one out. Proceed, with fingers crossed.",
        ]);
        self.ins_error();
    }

    /// §1227
    #[cold]
    pub fn you_cant(&mut self) {
        self.print_err_s(b"You can't use `");
        self.print_cmd_chr(self.cur_cmd, self.cur_chr);
        self.print_in_mode(self.mode());
    }

    /// §1228
    #[cold]
    pub fn report_illegal_case(&mut self) {
        self.you_cant();
        self.help(&[
            b"Sorry, but I'm not programmed to handle this case;",
            b"I'll just pretend that you didn't ask for it.",
            b"If you're in the wrong mode, you might be able to",
            b"return to the right one by typing `I}' or `I$' or `I\\par'.",
        ]);
        self.error();
    }

    /// §1229
    pub fn privileged(&mut self) -> bool {
        if self.mode() > 0 {
            true
        } else {
            self.report_illegal_case();
            false
        }
    }

    /// §1232
    pub fn its_all_over(&mut self) -> bool {
        if self.privileged() {
            if PAGE_HEAD == self.page_tail && self.head() == self.tail() && self.dead_cycles == 0 {
                return true;
            }
            self.back_input(); // we will try to end again after ejecting residual material
            let p = self.new_null_box();
            self.tail_append(p);
            self.set_width(self.tail(), self.hsize());
            let p = self.new_glue(FILL_GLUE);
            self.tail_append(p);
            let p = self.new_penalty(-0o10000000000);
            self.tail_append(p);
            self.build_page(); // append \hbox to \hsize{}\vfill\penalty-'10000000000
        }
        false
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

    fn assert_message(key: &str, out: &str) {
        let rows = table(include_str!("../testdata/build_messages_oracle.tsv"));
        let r = rows.iter().find(|r| r[0] == key).unwrap();
        assert!(out.starts_with(&r[1]), "{key}: {out:?} does not begin with {:?}", r[1]);
        let help = format!("{}\n\n", r[2]);
        assert!(out.ends_with(&help), "{key}: {out:?} does not end with {help:?}");
    }

    fn nodes(t: &Tex, mut p: i32) -> Vec<i32> {
        let mut v = Vec::new();
        while p != NULL {
            v.push(p);
            p = t.link(p);
        }
        v
    }

    #[test]
    fn privileged_only_in_a_positive_mode() {
        let t = tex();
        assert!(t.privileged());
        assert_eq!(printed(t), "");
        t.set_mode(-VMODE);
        (t.cur_cmd, t.cur_chr) = (STOP, 0);
        assert!(!t.privileged());
        assert_message("illegal_end_in_vbox", &printed(t));
        Tex::dealloc(t);
    }

    #[test]
    fn its_all_over_when_everything_is_empty() {
        let t = tex();
        (t.cur_cmd, t.cur_chr) = (STOP, 0);
        assert!(t.its_all_over());
        assert_eq!(printed(t), "");
        Tex::dealloc(t);
    }

    #[test]
    fn report_illegal_case_against_the_oracle() {
        for (key, cmd, chr) in [
            ("illegal_mac_param", MAC_PARAM, b'#' as i32),
            ("illegal_raise", VMOVE, 1),
            ("illegal_vadjust", VADJUST, 0),
            ("illegal_ital_corr", ITAL_CORR, 0),
        ] {
            let t = tex();
            (t.cur_cmd, t.cur_chr) = (cmd, chr);
            t.report_illegal_case();
            assert_message(key, &printed(t));
            Tex::dealloc(t);
        }
    }

    #[test]
    fn insert_dollar_sign_puts_it_before_the_token() {
        let t = tex();
        (t.cur_cmd, t.cur_chr) = (SUP_MARK, b'^' as i32);
        let tok = SUP_MARK * 256 + b'^' as i32;
        t.cur_tok = tok;
        t.insert_dollar_sign();
        assert_message("dollar", &printed(t));
        assert_eq!(t.token_type(), INSERTED);
        assert_eq!(t.toks[t.loc()], MATH_SHIFT_TOKEN + b'$' as i32);
        assert_eq!(t.toks[t.input_stack[t.input_ptr - 1].loc_field], tok);
        Tex::dealloc(t);
    }

    /// The parameters of the font from the oracle's first row: space, stretch, shrink, extra_space.
    fn set_font(t: &mut Tex, row: &[String]) {
        let f = 1;
        t.param_base[f] = 500;
        t.font_info[500 + SPACE_CODE].set_sc(row[0].parse().unwrap());
        t.font_info[500 + SPACE_STRETCH_CODE].set_sc(row[1].parse().unwrap());
        t.font_info[500 + SPACE_SHRINK_CODE].set_sc(row[2].parse().unwrap());
        t.font_info[500 + EXTRA_SPACE_CODE].set_sc(row[3].parse().unwrap());
        t.eqtb[CUR_FONT_LOC].set_rh(f);
    }

    /// `3pt plus 2fil minus 1filll`, as a glue spec, or zero_glue as trap_zero_glue would.
    fn spec(t: &mut Tex, text: &str) -> i32 {
        if text == "0pt" {
            return ZERO_GLUE;
        }
        fn value(s: &str) -> (i32, i32) {
            for (u, o) in [("pt", NORMAL), ("filll", FILLL), ("fill", FILL), ("fil", FIL)] {
                if let Some(n) = s.strip_suffix(u) {
                    return (n.parse::<i32>().unwrap() * 65536, o);
                }
            }
            panic!("{s}")
        }
        let w: Vec<&str> = text.split(' ').collect();
        let q = t.new_spec(ZERO_GLUE);
        t.set_width(q, value(w[0]).0);
        let mut k = 1;
        while k < w.len() {
            let (v, o) = value(w[k + 1]);
            if w[k] == "plus" {
                t.set_stretch(q, v);
                t.set_stretch_order(q, o);
            } else {
                t.set_shrink(q, v);
                t.set_shrink_order(q, o);
            }
            k += 2;
        }
        q
    }

    #[test]
    fn app_space_against_the_oracle() {
        let rows = table(include_str!("../testdata/build_space_oracle.tsv"));
        for r in &rows[1..] {
            let t = tex();
            set_font(t, &rows[0]);
            t.push_nest();
            t.set_mode(-HMODE);
            t.set_space_factor(r[0].parse().unwrap());
            let (sk, xs) = (spec(t, &r[1]), spec(t, &r[2]));
            t.eqtb[GLUE_BASE + SPACE_SKIP_CODE].set_rh(sk);
            t.eqtb[GLUE_BASE + XSPACE_SKIP_CODE].set_rh(xs);
            t.app_space();
            let g = t.tail();
            assert_eq!(t.ty(g), GLUE_NODE, "{r:?}");
            t.print_spec(t.glue_ptr(g), b"pt");
            assert_eq!(printed(t), r[3], "{r:?}");
            Tex::dealloc(t);
        }
    }

    #[test]
    fn app_space_keeps_the_cached_spec_and_makes_its_own() {
        let rows = table(include_str!("../testdata/build_space_oracle.tsv"));
        let t = tex();
        set_font(t, &rows[0]);
        t.push_nest();
        t.set_mode(-HMODE);
        t.set_space_factor(1500);
        t.app_space();
        let cached = t.font_glue[1];
        assert_ne!(cached, NULL);
        assert_eq!(t.width(cached), t.font_info[500 + SPACE_CODE].sc());
        assert_eq!(t.glue_ref_count(cached), NULL, "only the font table holds it");
        let own = t.glue_ptr(t.tail());
        assert_ne!(own, cached);
        assert_eq!(t.glue_ref_count(own), NULL, "the node's reference is the one reference");
        t.app_space();
        assert_eq!(t.font_glue[1], cached);
        assert_ne!(t.glue_ptr(t.tail()), own);
        Tex::dealloc(t);
    }

    fn hlist(t: &mut Tex, mode: i32) {
        t.push_nest();
        t.set_mode(mode);
    }

    /// The state wrapup sees after a run of characters with cur_q at `q`.
    fn run_of(t: &mut Tex, q: i32, l: i32) -> i32 {
        let f = 1;
        t.main_f = f;
        t.hyphen_char[f] = b'-' as i32;
        t.cur_q = q;
        t.cur_l = l;
        let c = t.fast_get_avail();
        t.set_font(c, f);
        t.set_character(c, b'f' as i32);
        t.set_link(q, c);
        t.set_tail(c);
        c
    }

    #[test]
    fn wrapup_makes_a_ligature_from_what_follows_cur_q() {
        let t = tex();
        hlist(t, -HMODE);
        let q = t.head();
        let c = run_of(t, q, 12);
        t.ligature_present = true;
        t.lig_stack = NULL;
        t.wrapup(false);
        let l = t.link(q);
        assert_eq!((t.tail(), t.ty(l), t.subtype(l)), (l, LIGATURE_NODE, 0));
        assert_eq!((t.font(l + 1), t.character(l + 1), t.lig_ptr(l)), (1, 12, c));
        assert!(!t.ligature_present);
        Tex::dealloc(t);
    }

    #[test]
    fn pack_lig_records_the_boundaries_in_the_subtype() {
        for (lft, z, stack, want, rt_after) in [
            (false, false, NULL, 0, true),
            (true, false, NULL, 2, true),
            (false, true, NULL, 1, false),
            (true, true, NULL, 3, false),
            (false, true, 5000, 0, true), // a right boundary counts only when lig_stack is empty
        ] {
            let t = tex();
            hlist(t, -HMODE);
            let q = t.head();
            run_of(t, q, 12);
            t.ligature_present = true;
            t.lft_hit = lft;
            t.rt_hit = true;
            t.lig_stack = stack;
            t.pack_lig(z);
            assert_eq!(t.subtype(t.link(q)), want);
            assert!(!t.lft_hit);
            assert_eq!(t.rt_hit, rt_after);
            Tex::dealloc(t);
        }
    }

    #[test]
    fn wrapup_adds_the_discretionary_only_in_unrestricted_hmode() {
        for (mode, l, want) in [(HMODE, b'-' as i32, true), (-HMODE, b'-' as i32, false), (HMODE, NON_CHAR, false)] {
            let t = tex();
            hlist(t, mode);
            let q = t.head();
            let c = run_of(t, q, l);
            t.set_character(c, b'-' as i32);
            t.ligature_present = false;
            t.wrapup(false);
            if want {
                assert_eq!(t.ty(t.tail()), DISC_NODE);
                assert_eq!(t.link(c), t.tail());
            } else {
                assert_eq!(t.tail(), c);
            }
            assert!(!t.ins_disc);
            Tex::dealloc(t);
        }
    }

    #[test]
    fn wrapup_does_nothing_at_the_left_boundary() {
        let t = tex();
        hlist(t, HMODE);
        let q = t.head();
        let c = run_of(t, q, NON_CHAR);
        t.ligature_present = true;
        t.wrapup(true);
        assert_eq!((t.tail(), t.ligature_present), (c, true));
        Tex::dealloc(t);
    }

    #[test]
    fn auto_kern_goes_before_a_null_discretionary_from_wrapup() {
        // The hyphen_char at tail makes wrapup append a null discretionary; prev_tail is then the hyphen.
        let t = tex();
        hlist(t, HMODE);
        let q = t.head();
        let c = run_of(t, q, b'-' as i32);
        t.set_character(c, b'-' as i32);
        let k = t.new_kern(7);
        t.control_append_auto_kern(k);
        let d = t.tail();
        assert_eq!(t.ty(d), DISC_NODE);
        assert_eq!(nodes(t, t.link(q)), vec![c, k, d]);
        assert_eq!(t.prev_tail, k);
        assert_eq!(t.save_tail, d, "before the kern goes in");
        Tex::dealloc(t);
        // No discretionary: it just goes at the end.
        let t = tex();
        hlist(t, HMODE);
        let q = t.head();
        let c = run_of(t, q, b'x' as i32);
        let k = t.new_kern(7);
        t.control_append_auto_kern(k);
        assert_eq!(nodes(t, t.link(q)), vec![c, k]);
        assert_eq!((t.tail(), t.save_tail), (k, c));
        Tex::dealloc(t);
        // A discretionary with something in it, not from wrapup: at the end too.
        let t = tex();
        hlist(t, HMODE);
        let q = t.head();
        let d = t.new_disc();
        t.tail_append(d);
        t.set_replace_count(d, 1);
        t.cur_l = NON_CHAR;
        t.cur_q = q;
        let k = t.new_kern(7);
        t.control_append_auto_kern(k);
        assert_eq!(nodes(t, t.link(q)), vec![d, k]);
        Tex::dealloc(t);
    }

    /// What main_control does in vertical mode with what needs no other module.  The last token is \end.
    fn run(t: &mut Tex, toks: &[i32]) -> String {
        let mut v = toks.to_vec();
        v.push(cs_tok(b'E'));
        t.set_eq_type(cs(b'E'), STOP);
        t.set_equiv(cs(b'E'), 0);
        feed(t, &v);
        t.main_control();
        printed(t)
    }

    #[test]
    fn main_control_ends_at_end_on_an_empty_page() {
        let t = tex();
        assert_eq!(run(t, &[]), "");
        Tex::dealloc(t);
    }

    #[test]
    fn main_control_groups_and_ignorable_things() {
        let t = tex();
        for (c, cmd, chr) in [
            (b'r', RELAX, 256),
            (b'i', IGNORE_SPACES, 0),
            (b'b', BEGIN_GROUP, 0),
            (b'e', END_GROUP, 0),
            (b'a', AFTER_ASSIGNMENT, 0),
            (b'g', AFTER_GROUP, 0),
            (b'P', PARTOKEN_NAME, 0),
        ] {
            t.set_eq_type(cs(c), cmd);
            t.set_equiv(cs(c), chr);
        }
        let out = run(
            t,
            &[
                SPACE_TOKEN,
                cs_tok(b'r'),
                cs_tok(b'i'),
                SPACE_TOKEN,
                SPACE_TOKEN,
                LEFT_BRACE_TOKEN + b'{' as i32,
                cs_tok(b'b'),
                cs_tok(b'g'),
                cs_tok(b'r'),
                cs_tok(b'e'),
                RIGHT_BRACE_TOKEN + b'}' as i32,
                cs_tok(b'a'),
                cs_tok(b'r'),
                cs_tok(b'P'),
                cs_tok(b'q'),
            ],
        );
        assert_eq!(out, "");
        assert_eq!(t.cur_group, BOTTOM_LEVEL);
        assert_eq!(t.after_token, cs_tok(b'r'));
        assert_eq!((t.par_loc, t.par_token), (cs(b'q'), cs_tok(b'q')));
        Tex::dealloc(t);
    }

    #[test]
    fn main_control_partokenname_of_a_character_is_ignored() {
        let t = tex();
        t.set_eq_type(cs(b'P'), PARTOKEN_NAME);
        let (l, k) = (t.par_loc, t.par_token);
        assert_eq!(run(t, &[cs_tok(b'P'), LETTER_TOKEN + b'x' as i32]), "");
        assert_eq!((t.par_loc, t.par_token), (l, k));
        Tex::dealloc(t);
    }

    #[test]
    fn main_control_reports_the_forbidden_and_the_misplaced() {
        // Each stays in vertical mode.
        for (cmd, chr, key) in [
            (VMOVE, 1, "illegal_raise"),
            (VADJUST, 0, "illegal_vadjust"),
            (ITAL_CORR, 0, "illegal_ital_corr"),
            (MAC_PARAM, b'#' as i32, "illegal_mac_param"),
            (END_CS_NAME, 0, "endcsname"),
            (NO_ALIGN, 0, "noalign"),
            (OMIT, 0, "omit"),
            (CAR_RET, CR_CODE, "misplaced_cr"),
            (TAB_MARK, SPAN_CODE, "misplaced_span"),
        ] {
            let t = tex();
            t.set_eq_type(cs(b'x'), cmd);
            t.set_equiv(cs(b'x'), chr);
            let out = run(t, &[cs_tok(b'x')]);
            assert_message(key, &out);
            Tex::dealloc(t);
        }
    }

    #[test]
    fn main_control_appends_fil_glue_in_vertical_mode() {
        let t = tex();
        t.set_eq_type(cs(b'v'), VSKIP);
        t.set_equiv(cs(b'v'), FIL_CODE);
        t.set_eq_type(cs(b'u'), REMOVE_ITEM);
        t.set_equiv(cs(b'u'), GLUE_NODE);
        // \vfil\unskip leaves the list empty again, and so \end can finish.
        assert_eq!(run(t, &[cs_tok(b'v'), cs_tok(b'u')]), "");
        assert_eq!(t.head(), t.tail());
        Tex::dealloc(t);
    }

    #[test]
    fn main_control_ignorespaces_expands_as_it_goes() {
        let t = tex();
        t.set_eq_type(cs(b'i'), IGNORE_SPACES);
        def_macro(t, cs(b's'), CALL, &[], &[SPACE_TOKEN]);
        assert_eq!(run(t, &[cs_tok(b'i'), cs_tok(b's'), SPACE_TOKEN, cs_tok(b's')]), "");
        Tex::dealloc(t);
    }

    #[test]
    fn tracing_commands_shows_each_at_reswitch() {
        let t = tex();
        t.set_tracing_commands(1);
        t.set_eq_type(cs(b'r'), RELAX);
        t.set_equiv(cs(b'r'), 256);
        let out = run(t, &[cs_tok(b'r'), SPACE_TOKEN]);
        let shown: Vec<&str> = out.lines().filter(|l| l.starts_with('{')).collect();
        assert_eq!(shown, vec!["{vertical mode: \\relax}", "{blank space  }", "{\\end}"]);
        Tex::dealloc(t);
    }
}
