//! §1386-1476 Part 49: mode-independent processing.
//!
//! `prefixed_command` is one function with all of `@<Assignments@>`, in section order, as tangle has it.
//! Token lists are section 15 of docs/PORTING.md: §1396, §1404, §1405, §1457, §1466, §1475.  The overflow
//! sites of section 11 are §1416 and §1417, and `get_node(2*n+1)` in §1426, which no table lists.
//! §1402's `\charsubdef` branch is MLTeX, dropped with its primitive.  §1443's
//! `kpse_make_tex_discard_errors` is only read by mktex, which is dropped.
//!
//! DROPPED §1427: a check of constants; capacities are checked at compile time.
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
    /// §430 Get the next non-blank non-relax non-call token.
    #[inline(always)]
    fn prefixed_next_non_blank_non_relax(&mut self) {
        loop {
            self.get_x_token();
            if self.cur_cmd != SPACER && self.cur_cmd != RELAX {
                break;
            }
        }
    }

    /// §75 Initialize the print `selector` based on `interaction`.
    #[inline(always)]
    fn prefixed_init_selector(&mut self) {
        if self.interaction == BATCH_MODE {
            self.selector = NO_PRINT;
        } else {
            self.selector = TERM_ONLY;
        }
    }

    /// §1932 Adjust `selector` based on `show_stream`.
    #[inline(always)]
    fn prefixed_adjust_selector_for_show_stream(&mut self) {
        let s = self.show_stream();
        if s >= 0 && s < NO_PRINT && self.write_open[s] {
            self.selector = s;
        }
    }

    /// §1389
    pub fn prefixed_command(&mut self) {
        let mut a = 0;
        let mut f: i32;
        let mut j: i32;
        let mut k: i32;
        let mut p: i32;
        let mut q: i32;
        let mut n: i32;
        let mut e: bool;
        while self.cur_cmd == PREFIX {
            if !odd(a / self.cur_chr) {
                a += self.cur_chr;
            }
            // §430
            self.prefixed_next_non_blank_non_relax();
            if self.cur_cmd <= MAX_NON_PREFIXED_COMMAND {
                // §1390 Discard erroneous prefixes and return
                self.print_err_s(b"You can't use a prefix with `");
                self.print_cmd_chr(self.cur_cmd, self.cur_chr);
                self.print_char(b'\'' as i32);
                self.help(&[b"I'll pretend you didn't say \\long or \\outer or \\global."]);
                if self.eTeX_ex() {
                    self.set_help_line(0, b"I'll pretend you didn't say \\long or \\outer or \\global or \\protected.");
                }
                self.back_error();
                return;
            }
            if self.tracing_commands() > 2 && self.eTeX_ex() {
                self.show_cur_cmd_chr();
            }
        }
        // §1391 Discard the prefixes \long and \outer if they are irrelevant
        if a >= 8 {
            j = PROTECTED_TOKEN;
            a -= 8;
        } else {
            j = 0;
        }
        if self.cur_cmd != DEF && (a % 4 != 0 || j != 0) {
            self.print_err_s(b"You can't use `");
            self.print_esc_s(b"long");
            self.print_s(b"' or `");
            self.print_esc_s(b"outer");
            self.help(&[b"I'll pretend you didn't say \\long or \\outer here."]);
            if self.eTeX_ex() {
                self.set_help_line(0, b"I'll pretend you didn't say \\long or \\outer or \\protected here.");
                self.print_s(b"' or `");
                self.print_esc_s(b"protected");
            }
            self.print_s(b"' with `");
            self.print_cmd_chr(self.cur_cmd, self.cur_chr);
            self.print_char(b'\'' as i32);
            self.error();
        }
        // §1392 Adjust for the setting of \globaldefs
        if self.global_defs() != 0 {
            if self.global_defs() < 0 {
                if a >= 4 {
                    a -= 4;
                }
            } else if a < 4 {
                a += 4;
            }
        }
        'done: {
            match self.cur_cmd {
                // §1395
                SET_FONT => self.define(a, CUR_FONT_LOC, DATA, self.cur_chr),
                // §1396
                DEF => {
                    if odd(self.cur_chr) && a < 4 && self.global_defs() >= 0 {
                        a += 4;
                    }
                    e = self.cur_chr >= 2;
                    self.get_r_token();
                    p = self.cur_cs;
                    self.scan_toks(true, e);
                    if j != 0 {
                        self.def_ref = self.tok_prepend(self.def_ref, j);
                    }
                    self.define(a, p, CALL + (a % 4), self.def_ref);
                }
                // §1399
                LET => {
                    n = self.cur_chr;
                    self.get_r_token();
                    p = self.cur_cs;
                    if n == NORMAL {
                        loop {
                            self.get_token();
                            if self.cur_cmd != SPACER {
                                break;
                            }
                        }
                        if self.cur_tok == OTHER_TOKEN + b'=' as i32 {
                            self.get_token();
                            if self.cur_cmd == SPACER {
                                self.get_token();
                            }
                        }
                    } else {
                        self.get_token();
                        q = self.cur_tok;
                        self.get_token();
                        self.back_input();
                        self.cur_tok = q;
                        self.back_input(); // look ahead, then back up
                    } // note that back_input doesn't affect cur_cmd, cur_chr
                    if self.cur_cmd >= CALL {
                        self.add_token_ref(self.cur_chr);
                    } else if self.cur_cmd == REGISTER || self.cur_cmd == TOKS_REGISTER {
                        if self.cur_chr < MEM_BOT || self.cur_chr > LO_MEM_STAT_MAX {
                            self.add_sa_ref(self.cur_chr);
                        }
                    }
                    self.define(a, p, self.cur_cmd, self.cur_chr);
                }
                // §1402.  dropped: MLTeX, cur_chr=char_sub_def_code, whose primitive does not exist.
                SHORTHAND_DEF => {
                    n = self.cur_chr;
                    self.get_r_token();
                    p = self.cur_cs;
                    self.define(a, p, RELAX, 256);
                    self.scan_optional_equals();
                    match n {
                        CHAR_DEF_CODE => {
                            self.scan_char_num();
                            self.define(a, p, CHAR_GIVEN, self.cur_val);
                        }
                        MATH_CHAR_DEF_CODE => {
                            self.scan_fifteen_bit_int();
                            self.define(a, p, MATH_GIVEN, self.cur_val);
                        }
                        _ => {
                            self.scan_register_num();
                            if self.cur_val > 255 {
                                j = n - COUNT_DEF_CODE; // int_val..box_val
                                if j > MU_VAL {
                                    j = TOK_VAL; // int_val..mu_val or tok_val
                                }
                                self.find_sa_element(j, self.cur_val, true);
                                self.add_sa_ref(self.cur_ptr);
                                if j == TOK_VAL {
                                    j = TOKS_REGISTER;
                                } else {
                                    j = REGISTER;
                                }
                                self.define(a, p, j, self.cur_ptr);
                            } else {
                                match n {
                                    COUNT_DEF_CODE => self.define(a, p, ASSIGN_INT, COUNT_BASE + self.cur_val),
                                    DIMEN_DEF_CODE => self.define(a, p, ASSIGN_DIMEN, SCALED_BASE + self.cur_val),
                                    SKIP_DEF_CODE => self.define(a, p, ASSIGN_GLUE, SKIP_BASE + self.cur_val),
                                    MU_SKIP_DEF_CODE => self.define(a, p, ASSIGN_MU_GLUE, MU_SKIP_BASE + self.cur_val),
                                    TOKS_DEF_CODE => self.define(a, p, ASSIGN_TOKS, TOKS_BASE + self.cur_val),
                                    _ => {} // there are no other cases
                                }
                            }
                        }
                    }
                }
                // §1403
                READ_TO_CS => {
                    j = self.cur_chr;
                    self.scan_int();
                    n = self.cur_val;
                    if !self.scan_keyword(b"to") {
                        self.print_err_s(b"Missing `to' inserted");
                        self.help(&[
                            b"You should have said `\\read<number> to \\cs'.",
                            b"I'm going to look for the \\cs now.",
                        ]);
                        self.error();
                    }
                    self.get_r_token();
                    p = self.cur_cs;
                    self.read_toks(n, p, j);
                    self.define(a, p, CALL, self.cur_val);
                }
                // §1404
                TOKS_REGISTER | ASSIGN_TOKS => {
                    q = self.cur_cs;
                    e = false; // just in case, will be set true for sparse array elements
                    if self.cur_cmd == TOKS_REGISTER {
                        if self.cur_chr == MEM_BOT {
                            self.scan_register_num();
                            if self.cur_val > 255 {
                                self.find_sa_element(TOK_VAL, self.cur_val, true);
                                self.cur_chr = self.cur_ptr;
                                e = true;
                            } else {
                                self.cur_chr = TOKS_BASE + self.cur_val;
                            }
                        } else {
                            e = true;
                        }
                    }
                    p = self.cur_chr; // p=every_par_loc or output_routine_loc or ...
                    self.scan_optional_equals();
                    // §430
                    self.prefixed_next_non_blank_non_relax();
                    if self.cur_cmd != LEFT_BRACE {
                        // §1405 If the right-hand side is a token parameter or token register, finish the assignment and goto done
                        if self.cur_cmd == TOKS_REGISTER || self.cur_cmd == ASSIGN_TOKS {
                            if self.cur_cmd == TOKS_REGISTER {
                                if self.cur_chr == MEM_BOT {
                                    self.scan_register_num();
                                    if self.cur_val < 256 {
                                        q = self.equiv(TOKS_BASE + self.cur_val);
                                    } else {
                                        self.find_sa_element(TOK_VAL, self.cur_val, false);
                                        if self.cur_ptr == NULL {
                                            q = NULL;
                                        } else {
                                            q = self.sa_ptr(self.cur_ptr);
                                        }
                                    }
                                } else {
                                    q = self.sa_ptr(self.cur_chr);
                                }
                            } else {
                                q = self.equiv(self.cur_chr);
                            }
                            if q == NULL {
                                self.sa_define(a, e, p, NULL, p, UNDEFINED_CS, NULL);
                            } else {
                                self.add_token_ref(q);
                                self.sa_define(a, e, p, q, p, CALL, q);
                            }
                            break 'done;
                        }
                    }
                    self.back_input();
                    self.cur_cs = q;
                    self.scan_toks(false, false);
                    if self.tok_is_empty(self.def_ref) {
                        // empty list: revert to the default
                        self.sa_define(a, e, p, NULL, p, UNDEFINED_CS, NULL);
                        self.flush_list(self.def_ref);
                    } else {
                        if p == OUTPUT_ROUTINE_LOC && !e {
                            // enclose in curlies
                            let d = self.tok_append(self.def_ref, RIGHT_BRACE_TOKEN + b'}' as i32);
                            self.def_ref = self.tok_prepend(d, LEFT_BRACE_TOKEN + b'{' as i32);
                        }
                        self.sa_define(a, e, p, self.def_ref, p, CALL, self.def_ref);
                    }
                }
                // §1406
                ASSIGN_INT => {
                    p = self.cur_chr;
                    self.scan_optional_equals();
                    self.scan_int();
                    self.word_define(a, p, self.cur_val);
                }
                ASSIGN_DIMEN => {
                    p = self.cur_chr;
                    self.scan_optional_equals();
                    self.scan_normal_dimen();
                    self.word_define(a, p, self.cur_val);
                }
                ASSIGN_GLUE | ASSIGN_MU_GLUE => {
                    p = self.cur_chr;
                    n = self.cur_cmd;
                    self.scan_optional_equals();
                    if n == ASSIGN_MU_GLUE {
                        self.scan_glue(MU_VAL);
                    } else {
                        self.scan_glue(GLUE_VAL);
                    }
                    self.trap_zero_glue();
                    self.define(a, p, GLUE_REF, self.cur_val);
                }
                // §1410
                DEF_CODE => {
                    // §1411 Let n be the largest legal code value, based on cur_chr
                    if self.cur_chr == CAT_CODE_BASE {
                        n = MAX_CHAR_CODE;
                    } else if self.cur_chr == MATH_CODE_BASE {
                        n = 0o100000;
                    } else if self.cur_chr == SF_CODE_BASE {
                        n = 0o77777;
                    } else if self.cur_chr == DEL_CODE_BASE {
                        n = 0o77777777;
                    } else {
                        n = 255;
                    }
                    p = self.cur_chr;
                    self.scan_char_num();
                    p += self.cur_val;
                    self.scan_optional_equals();
                    self.scan_int();
                    if (self.cur_val < 0 && p < DEL_CODE_BASE) || self.cur_val > n {
                        self.print_err_s(b"Invalid code (");
                        self.print_int(self.cur_val);
                        if p < DEL_CODE_BASE {
                            self.print_s(b"), should be in the range 0..");
                        } else {
                            self.print_s(b"), should be at most ");
                        }
                        self.print_int(n);
                        self.help(&[b"I'm going to use 0 instead of that illegal code value."]);
                        self.error();
                        self.cur_val = 0;
                    }
                    if p < DEL_CODE_BASE {
                        // WEB has two branches here, at math_code_base, differing only in hi(), the identity.
                        self.define(a, p, DATA, self.cur_val);
                    } else {
                        self.word_define(a, p, self.cur_val);
                    }
                }
                // §1412
                DEF_FAMILY => {
                    p = self.cur_chr;
                    self.scan_four_bit_int();
                    p += self.cur_val;
                    self.scan_optional_equals();
                    self.scan_font_ident();
                    self.define(a, p, DATA, self.cur_val);
                }
                // §1413
                REGISTER | ADVANCE | MULTIPLY | DIVIDE => self.do_register_command(a),
                // §1419
                SET_BOX => {
                    self.scan_register_num();
                    if a >= 4 {
                        n = GLOBAL_BOX_FLAG + self.cur_val;
                    } else {
                        n = BOX_FLAG + self.cur_val;
                    }
                    self.scan_optional_equals();
                    if self.set_box_allowed {
                        self.scan_box(n);
                    } else {
                        self.print_err_s(b"Improper ");
                        self.print_esc_s(b"setbox");
                        self.help(&[
                            b"Sorry, \\setbox is not allowed after \\halign in a display,",
                            b"or between \\accent and an accented character.",
                        ]);
                        self.error();
                    }
                }
                // §1420
                SET_AUX => self.alter_aux(),
                SET_PREV_GRAF => self.alter_prev_graf(),
                SET_PAGE_DIMEN => self.alter_page_so_far(),
                SET_PAGE_INT => self.alter_integer(),
                SET_BOX_DIMEN => self.alter_box_dimen(),
                // §1426
                SET_SHAPE => {
                    q = self.cur_chr;
                    self.scan_optional_equals();
                    self.scan_int();
                    n = self.cur_val;
                    if n <= 0 {
                        p = NULL;
                    } else if q > PAR_SHAPE_LOC {
                        n = self.cur_val / 2 + 1;
                        // The oracle's 2*n+1 is 64 bits, so it asks for too much and overflows; i32 would wrap.
                        p = self.get_node(sat32(2 * n as i64 + 1));
                        self.set_info(p, n);
                        n = self.cur_val;
                        self.mem[p + 1].set_int(n); // number of penalties
                        for j in (p + 2)..=(p + n + 1) {
                            self.scan_int();
                            self.mem[j].set_int(self.cur_val); // penalty values
                        }
                        if !odd(n) {
                            self.mem[p + n + 2].set_int(0); // unused
                        }
                    } else {
                        p = self.get_node(sat32(2 * n as i64 + 1));
                        self.set_info(p, n);
                        for j in 1..=n {
                            self.scan_normal_dimen();
                            self.mem[p + 2 * j - 1].set_sc(self.cur_val); // indentation
                            self.scan_normal_dimen();
                            self.mem[p + 2 * j].set_sc(self.cur_val); // width
                        }
                    }
                    self.define(a, q, SHAPE_REF, p);
                }
                // §1430
                HYPH_DATA => {
                    if self.cur_chr == 1 {
                        if self.ini_version {
                            self.new_patterns();
                            break 'done;
                        }
                        self.print_err_s(b"Patterns can be loaded only by INITEX");
                        self.help(&[]);
                        self.error();
                        loop {
                            self.get_token();
                            if self.cur_cmd == RIGHT_BRACE {
                                break; // flush the patterns
                            }
                        }
                        return;
                    } else {
                        self.new_hyph_exceptions();
                        break 'done;
                    }
                }
                // §1431
                ASSIGN_FONT_DIMEN => {
                    self.find_font_dimen(true);
                    k = self.cur_val;
                    self.scan_optional_equals();
                    self.scan_normal_dimen();
                    self.font_info[k].set_sc(self.cur_val);
                }
                ASSIGN_FONT_INT => {
                    n = self.cur_chr;
                    self.scan_font_ident();
                    f = self.cur_val;
                    if n == NO_LIG_CODE {
                        self.set_no_ligatures(f);
                    } else if n < LP_CODE_BASE {
                        self.scan_optional_equals();
                        self.scan_int();
                        if n == 0 {
                            self.hyphen_char[f] = self.cur_val;
                        } else {
                            self.skew_char[f] = self.cur_val;
                        }
                    } else {
                        self.scan_char_num();
                        p = self.cur_val;
                        self.scan_optional_equals();
                        self.scan_int();
                        match n {
                            LP_CODE_BASE => self.set_lp_code(f, p, self.cur_val),
                            RP_CODE_BASE => self.set_rp_code(f, p, self.cur_val),
                            EF_CODE_BASE => self.set_ef_code(f, p, self.cur_val),
                            TAG_CODE => self.set_tag_code(f, p, self.cur_val),
                            KN_BS_CODE_BASE => self.set_kn_bs_code(f, p, self.cur_val),
                            ST_BS_CODE_BASE => self.set_st_bs_code(f, p, self.cur_val),
                            SH_BS_CODE_BASE => self.set_sh_bs_code(f, p, self.cur_val),
                            KN_BC_CODE_BASE => self.set_kn_bc_code(f, p, self.cur_val),
                            KN_AC_CODE_BASE => self.set_kn_ac_code(f, p, self.cur_val),
                            _ => {}
                        }
                    }
                }
                // §1434
                DEF_FONT => self.new_font(a),
                LETTERSPACE_FONT => self.new_letterspaced_font(a),
                PDF_COPY_FONT => self.make_font_copy(a),
                // §1442
                SET_INTERACTION => self.new_interaction(),
                _ => self.confusion(b"prefix"),
            }
        }
        // done: §1447 Insert a token saved by \afterassignment, if any
        if self.after_token != 0 {
            self.cur_tok = self.after_token;
            self.back_input();
            self.after_token = 0;
        }
    }

    /// §1393
    pub fn get_r_token(&mut self) {
        loop {
            loop {
                self.get_token();
                if self.cur_tok != SPACE_TOKEN {
                    break;
                }
            }
            if self.cur_cs == 0
                || self.cur_cs > self.eqtb_top
                || (self.cur_cs > FROZEN_CONTROL_SEQUENCE && self.cur_cs <= EQTB_SIZE)
            {
                self.print_err_s(b"Missing control sequence inserted");
                self.help(&[
                    b"Please don't say `\\def cs{...}', say `\\def\\cs{...}'.",
                    b"I've inserted an inaccessible control sequence so that your",
                    b"definition will be completed without mixing me up too badly.",
                    b"You can recover graciously from this error, if you're",
                    b"careful; see exercise 27.2 in The TeXbook.",
                ]);
                if self.cur_cs == 0 {
                    self.back_input();
                }
                self.cur_tok = CS_TOKEN_FLAG + FROZEN_PROTECTION;
                self.ins_error();
                continue; // restart
            }
            break;
        }
    }

    /// §1407
    pub fn trap_zero_glue(&mut self) {
        if self.width(self.cur_val) == 0 && self.stretch(self.cur_val) == 0 && self.shrink(self.cur_val) == 0 {
            self.add_glue_ref(ZERO_GLUE);
            self.delete_glue_ref(self.cur_val);
            self.cur_val = ZERO_GLUE;
        }
    }

    /// §1414
    pub fn do_register_command(&mut self, a: i32) {
        let mut l = 0;
        let mut q: i32;
        let mut r: i32;
        let mut s = 0;
        let mut p: i32;
        let mut e: bool;
        let mut w = 0;
        q = self.cur_cmd;
        e = false; // just in case, will be set true for sparse array elements
        // §1415 Compute the register location l and its type p; but return if invalid
        'found: {
            if q != REGISTER {
                self.get_x_token();
                if self.cur_cmd >= ASSIGN_INT && self.cur_cmd <= ASSIGN_MU_GLUE {
                    l = self.cur_chr;
                    p = self.cur_cmd - ASSIGN_INT;
                    break 'found;
                }
                if self.cur_cmd != REGISTER {
                    self.print_err_s(b"You can't use `");
                    self.print_cmd_chr(self.cur_cmd, self.cur_chr);
                    self.print_s(b"' after ");
                    self.print_cmd_chr(q, 0);
                    self.help(&[b"I'm forgetting what you said and not changing anything."]);
                    self.error();
                    return;
                }
            }
            if self.cur_chr < MEM_BOT || self.cur_chr > LO_MEM_STAT_MAX {
                l = self.cur_chr;
                p = self.sa_type(l);
                e = true;
            } else {
                p = self.cur_chr - MEM_BOT;
                self.scan_register_num();
                if self.cur_val > 255 {
                    self.find_sa_element(p, self.cur_val, true);
                    l = self.cur_ptr;
                    e = true;
                } else {
                    match p {
                        INT_VAL => l = self.cur_val + COUNT_BASE,
                        DIMEN_VAL => l = self.cur_val + SCALED_BASE,
                        GLUE_VAL => l = self.cur_val + SKIP_BASE,
                        MU_VAL => l = self.cur_val + MU_SKIP_BASE,
                        _ => {} // there are no other cases
                    }
                }
            }
        }
        // found:
        if p < GLUE_VAL {
            if e {
                w = self.sa_int(l);
            } else {
                w = self.eqtb[l].int();
            }
        } else if e {
            s = self.sa_ptr(l);
        } else {
            s = self.equiv(l);
        }
        if q == REGISTER {
            self.scan_optional_equals();
        } else if self.scan_keyword(b"by") {
            // optional `by'
        }
        self.arith_error = false;
        if q < MULTIPLY {
            // §1416 Compute result of register or advance, put it in cur_val
            if p < GLUE_VAL {
                if p == INT_VAL {
                    self.scan_int();
                } else {
                    self.scan_normal_dimen();
                }
                if q == ADVANCE {
                    self.cur_val = sat32(self.cur_val as i64 + w as i64);
                }
            } else {
                self.scan_glue(p);
                if q == ADVANCE {
                    // §1417 Compute the sum of two glue specs
                    q = self.new_spec(self.cur_val);
                    r = s;
                    self.delete_glue_ref(self.cur_val);
                    self.set_width(q, sat32(self.width(q) as i64 + self.width(r) as i64));
                    if self.stretch(q) == 0 {
                        self.set_stretch_order(q, NORMAL);
                    }
                    if self.stretch_order(q) == self.stretch_order(r) {
                        self.set_stretch(q, sat32(self.stretch(q) as i64 + self.stretch(r) as i64));
                    } else if self.stretch_order(q) < self.stretch_order(r) && self.stretch(r) != 0 {
                        self.set_stretch(q, self.stretch(r));
                        self.set_stretch_order(q, self.stretch_order(r));
                    }
                    if self.shrink(q) == 0 {
                        self.set_shrink_order(q, NORMAL);
                    }
                    if self.shrink_order(q) == self.shrink_order(r) {
                        self.set_shrink(q, sat32(self.shrink(q) as i64 + self.shrink(r) as i64));
                    } else if self.shrink_order(q) < self.shrink_order(r) && self.shrink(r) != 0 {
                        self.set_shrink(q, self.shrink(r));
                        self.set_shrink_order(q, self.shrink_order(r));
                    }
                    self.cur_val = q;
                }
            }
        } else {
            // §1418 Compute result of multiply or divide, put it in cur_val
            self.scan_int();
            if p < GLUE_VAL {
                if q == MULTIPLY {
                    if p == INT_VAL {
                        self.cur_val = self.mult_integers(w, self.cur_val);
                    } else {
                        self.cur_val = self.nx_plus_y(w, self.cur_val, 0);
                    }
                } else {
                    self.cur_val = self.x_over_n(w, self.cur_val);
                }
            } else {
                r = self.new_spec(s);
                if q == MULTIPLY {
                    let v = self.nx_plus_y(self.width(s), self.cur_val, 0);
                    self.set_width(r, v);
                    let v = self.nx_plus_y(self.stretch(s), self.cur_val, 0);
                    self.set_stretch(r, v);
                    let v = self.nx_plus_y(self.shrink(s), self.cur_val, 0);
                    self.set_shrink(r, v);
                } else {
                    let v = self.x_over_n(self.width(s), self.cur_val);
                    self.set_width(r, v);
                    let v = self.x_over_n(self.stretch(s), self.cur_val);
                    self.set_stretch(r, v);
                    let v = self.x_over_n(self.shrink(s), self.cur_val);
                    self.set_shrink(r, v);
                }
                self.cur_val = r;
            }
        }
        if self.arith_error {
            self.print_err_s(b"Arithmetic overflow");
            self.help(&[
                b"I can't carry out that multiplication or division,",
                b"since the result is out of range.",
            ]);
            if p >= GLUE_VAL {
                self.delete_glue_ref(self.cur_val);
            }
            self.error();
            return;
        }
        if p < GLUE_VAL {
            self.sa_word_define(a, e, l, self.cur_val);
        } else {
            self.trap_zero_glue();
            self.sa_define(a, e, l, self.cur_val, l, GLUE_REF, self.cur_val);
        }
    }

    /// §1421
    pub fn alter_aux(&mut self) {
        if self.cur_chr != abs(self.mode()) {
            self.report_illegal_case();
        } else {
            let c = self.cur_chr;
            self.scan_optional_equals();
            if c == VMODE {
                self.scan_normal_dimen();
                self.set_prev_depth(self.cur_val);
            } else {
                self.scan_int();
                if self.cur_val <= 0 || self.cur_val > 32767 {
                    self.print_err_s(b"Bad space factor");
                    self.help(&[b"I allow only values in the range 1..32767 here."]);
                    self.int_error(self.cur_val);
                } else {
                    self.set_space_factor(self.cur_val);
                }
            }
        }
    }

    /// §1422
    pub fn alter_prev_graf(&mut self) {
        self.nest[self.nest_ptr] = self.cur_list;
        let mut p = self.nest_ptr;
        while abs(self.nest[p].mode_field) != VMODE {
            p -= 1;
        }
        self.scan_optional_equals();
        self.scan_int();
        if self.cur_val < 0 {
            self.print_err_s(b"Bad ");
            self.print_esc_s(b"prevgraf");
            self.help(&[b"I allow only nonnegative values here."]);
            self.int_error(self.cur_val);
        } else {
            self.nest[p].pg_field = self.cur_val;
            self.cur_list = self.nest[self.nest_ptr];
        }
    }

    /// §1423
    pub fn alter_page_so_far(&mut self) {
        let c = self.cur_chr;
        self.scan_optional_equals();
        self.scan_normal_dimen();
        self.page_so_far[c] = self.cur_val;
    }

    /// §1424
    pub fn alter_integer(&mut self) {
        let c = self.cur_chr;
        self.scan_optional_equals();
        self.scan_int();
        if c == 0 {
            self.dead_cycles = self.cur_val;
        } else if c == 2 {
            // §1696 Cases for alter_integer
            if self.cur_val < BATCH_MODE || self.cur_val > ERROR_STOP_MODE {
                self.print_err_s(b"Bad interaction mode");
                self.help(&[
                    b"Modes are 0=batch, 1=nonstop, 2=scroll, and",
                    b"3=errorstop. Proceed, and I'll ignore this case.",
                ]);
                self.int_error(self.cur_val);
            } else {
                self.cur_chr = self.cur_val;
                self.new_interaction();
            }
        } else {
            self.insert_penalties = self.cur_val;
        }
    }

    /// §1425
    pub fn alter_box_dimen(&mut self) {
        let c = self.cur_chr;
        self.scan_register_num();
        let b = self.fetch_box();
        self.scan_optional_equals();
        self.scan_normal_dimen();
        if b != NULL {
            self.mem[b + c].set_sc(self.cur_val);
        }
    }

    /// §1435
    pub fn new_font(&mut self, a: i32) {
        let t: i32;
        let mut s: i32;
        let mut f: i32;
        if self.job_name == 0 {
            self.open_log_file(); // avoid confusing texput with the font name
        }
        self.get_r_token();
        let u = self.cur_cs;
        if u >= HASH_BASE {
            t = self.text(u);
        } else if u >= SINGLE_BASE {
            if u == NULL_CS {
                t = S_FONT;
            } else {
                t = u - SINGLE_BASE;
            }
        } else {
            let old_setting = self.selector;
            self.selector = NEW_STRING;
            self.print_s(b"FONT");
            self.print(u - ACTIVE_BASE);
            self.selector = old_setting;
            self.str_room(1);
            t = self.make_string();
        }
        self.define(a, u, SET_FONT, NULL_FONT);
        self.scan_optional_equals();
        self.scan_file_name();
        // §1436 Scan the font size specification
        self.name_in_progress = true; // this keeps cur_name from being changed
        if self.scan_keyword(b"at") {
            // §1437 Put the (positive) `at' size into s
            self.scan_normal_dimen();
            s = self.cur_val;
            if s <= 0 || s >= 0o1000000000 {
                self.print_err_s(b"Improper `at' size (");
                self.print_scaled(s);
                self.print_s(b"pt), replaced by 10pt");
                self.help(&[
                    b"I can only handle fonts at positive sizes that are",
                    b"less than 2048pt, so I've changed what you said to 10pt.",
                ]);
                self.error();
                s = 10 * UNITY;
            }
        } else if self.scan_keyword(b"scaled") {
            self.scan_int();
            s = -self.cur_val;
            if self.cur_val <= 0 || self.cur_val > 32768 {
                self.print_err_s(b"Illegal magnification has been changed to 1000");
                self.help(&[b"The magnification ratio must be between 1 and 32768."]);
                self.int_error(self.cur_val);
                s = -1000;
            }
        } else {
            s = -1000;
        }
        self.name_in_progress = false;
        'common_ending: {
            // §1438 If this font has already been loaded, set f to the internal font number and goto common_ending
            f = FONT_BASE + 1;
            let font_end = self.font_ptr;
            while f <= font_end {
                if self.str_eq_str(self.font_name[f], self.cur_name) && self.str_eq_str(self.font_area[f], self.cur_area) {
                    if self.pdf_font_step[f] == 0 {
                        if s > 0 {
                            if s == self.font_size[f] {
                                break 'common_ending;
                            }
                        } else {
                            self.arith_error = false;
                            if self.font_size[f] == self.xn_over_d(self.font_dsize[f], -s, 1000) && !self.arith_error {
                                break 'common_ending;
                            }
                        }
                    }
                }
                f += 1;
            }
            f = self.read_font_info(u, self.cur_name, self.cur_area, s);
        }
        // common_ending:
        self.define(a, u, SET_FONT, f);
        self.eqtb[FONT_ID_BASE + f] = self.eqtb[u];
        self.set_font_id_text(f, t);
    }

    /// §1443
    pub fn new_interaction(&mut self) {
        self.print_ln();
        self.interaction = self.cur_chr;
        // dropped: kpse_make_tex_discard_errors, read only by mktex, section 19.
        // §75
        self.prefixed_init_selector();
        if self.log_opened {
            self.selector += 2;
        }
    }

    /// §1448
    pub fn do_assignments(&mut self) {
        loop {
            // §430
            self.prefixed_next_non_blank_non_relax();
            if self.cur_cmd <= MAX_NON_PREFIXED_COMMAND {
                return;
            }
            self.set_box_allowed = false;
            self.prefixed_command();
            self.set_box_allowed = true;
        }
    }

    /// §1453
    pub fn open_or_close_in(&mut self) {
        let c = self.cur_chr;
        self.scan_four_bit_int();
        let n = self.cur_val;
        if self.read_open[n] != CLOSED {
            self.a_close(read_file(n));
            self.read_open[n] = CLOSED;
        }
        if c != 0 {
            self.scan_optional_equals();
            self.scan_file_name();
            self.pack_cur_name();
            self.tex_input_type = 0; // Tell open_input we are \openin.
            if self.kpse_in_name_ok() && self.a_open_in(read_file(n), KPSE_TEX_FORMAT) {
                self.read_open[n] = JUST_OPEN;
            }
        }
    }

    /// §1457
    pub fn issue_message(&mut self) {
        let c = self.cur_chr;
        self.scan_toks(false, true);
        let old_setting = self.selector;
        self.selector = NEW_STRING;
        self.message_printing = true;
        self.token_show(self.def_ref);
        self.message_printing = false;
        self.selector = old_setting;
        self.flush_list(self.def_ref);
        self.str_room(1);
        let s = self.make_string();
        if c == 0 {
            // §1458 Print string s on the terminal
            if self.term_offset + self.length(s) > self.max_print_line - 2 {
                self.print_ln();
            } else if self.term_offset > 0 || self.file_offset > 0 {
                self.print_char(b' ' as i32);
            }
            self.print(s);
            self.update_terminal();
        } else {
            // §1461 Print string s as an error message
            self.print_err_s(b"");
            self.print(s);
            if self.err_help() != NULL {
                self.use_err_help = true;
            } else if self.long_help_seen {
                self.help(&[b"(That was another \\errmessage.)"]);
            } else {
                if self.interaction < ERROR_STOP_MODE {
                    self.long_help_seen = true;
                }
                self.help(&[
                    b"This error message was generated by an \\errmessage",
                    b"command, so I can't give any explicit help.",
                    b"Pretend that you're Hercule Poirot: Examine all clues,",
                    b"and deduce the truth by order and method.",
                ]);
            }
            self.error();
            self.use_err_help = false;
        }
        self.flush_string();
    }

    /// §1462
    #[cold]
    pub fn give_err_help(&mut self) {
        self.token_show(self.err_help());
    }

    /// §1466
    pub fn shift_case(&mut self) {
        let b = self.cur_chr;
        self.scan_toks(false, false);
        let mut p = self.def_ref + 1;
        while self.toks[p] != 0 {
            // §1467 Change the case of the token in p, if a change is appropriate
            let t = self.toks[p];
            if t < CS_TOKEN_FLAG + SINGLE_BASE {
                let c = t % 256;
                if self.equiv(b + c) != 0 {
                    self.toks[p] = t - c + self.equiv(b + c);
                }
            }
            p += 1;
        }
        self.back_list(self.def_ref); // the block carries its own header, which end_token_list frees
    }

    /// §1471
    pub fn show_whatever(&mut self) {
        let mut p: i32;
        let mut t: i32;
        let mut m: i32;
        let mut l: i32;
        let mut n: i32;
        'common_ending: {
            match self.cur_chr {
                SHOW_LISTS_CODE => {
                    // §1932
                    self.prefixed_adjust_selector_for_show_stream();
                    self.begin_diagnostic();
                    self.show_activities();
                }
                SHOW_BOX_CODE => {
                    // §1474 Show the current contents of a box
                    self.scan_register_num();
                    p = self.fetch_box();
                    // §1932
                    self.prefixed_adjust_selector_for_show_stream();
                    self.begin_diagnostic();
                    self.print_nl_s(b"> \\box");
                    self.print_int(self.cur_val);
                    self.print_char(b'=' as i32);
                    if p == NULL {
                        self.print_s(b"void");
                    } else {
                        self.show_box(p);
                    }
                }
                SHOW_CODE => {
                    // §1472 Show the current meaning of a token, then goto common_ending
                    self.get_token();
                    // §1932
                    self.prefixed_adjust_selector_for_show_stream();
                    if self.interaction == ERROR_STOP_MODE {
                        self.wake_up_terminal();
                    }
                    self.print_nl_s(b"> ");
                    if self.cur_cs != 0 {
                        self.sprint_cs(self.cur_cs);
                        self.print_char(b'=' as i32);
                    }
                    self.print_meaning();
                    break 'common_ending;
                }
                // §1677 Cases for show_whatever
                SHOW_GROUPS => {
                    // §1932
                    self.prefixed_adjust_selector_for_show_stream();
                    self.begin_diagnostic();
                    self.show_save_groups();
                }
                // §1691 Cases for show_whatever
                SHOW_IFS => {
                    // §1932
                    self.prefixed_adjust_selector_for_show_stream();
                    self.begin_diagnostic();
                    self.print_nl_s(b"");
                    self.print_ln();
                    if self.cond_ptr == NULL {
                        self.print_nl_s(b"### ");
                        self.print_s(b"no active conditionals");
                    } else {
                        p = self.cond_ptr;
                        n = 0;
                        loop {
                            n += 1;
                            p = self.link(p);
                            if p == NULL {
                                break;
                            }
                        }
                        p = self.cond_ptr;
                        t = self.cur_if;
                        l = self.if_line;
                        m = self.if_limit;
                        loop {
                            self.print_nl_s(b"### level ");
                            self.print_int(n);
                            self.print_s(b": ");
                            self.print_cmd_chr(IF_TEST, t);
                            if m == FI_CODE {
                                self.print_esc_s(b"else");
                            }
                            self.print_if_line(l);
                            n -= 1;
                            t = self.subtype(p);
                            l = self.if_line_field(p);
                            m = self.ty(p);
                            p = self.link(p);
                            if p == NULL {
                                break;
                            }
                        }
                    }
                }
                _ => {
                    // §1475 Show the current value of some parameter or register, then goto common_ending
                    let mk = self.tb_mark();
                    self.the_toks();
                    let h = self.tb_commit(mk);
                    // §1932
                    self.prefixed_adjust_selector_for_show_stream();
                    if self.interaction == ERROR_STOP_MODE {
                        self.wake_up_terminal();
                    }
                    self.print_nl_s(b"> ");
                    self.token_show(h);
                    self.flush_list(h);
                    break 'common_ending;
                }
            }
            // §1476 Complete a potentially long \show command
            self.end_diagnostic(true);
            self.print_err_s(b"OK");
            if self.selector == TERM_AND_LOG && self.tracing_online() <= 0 {
                self.selector = TERM_ONLY;
                self.print_s(b" (see the transcript file)");
                self.selector = TERM_AND_LOG;
            }
        }
        // common_ending:
        if self.selector < NO_PRINT {
            self.print_ln();
            // §75
            self.prefixed_init_selector();
            if self.log_opened {
                self.selector += 2;
            }
        } else {
            if self.interaction < ERROR_STOP_MODE {
                self.help(&[]);
                self.error_count -= 1;
            } else if self.tracing_online() > 0 {
                self.help(&[
                    b"This isn't an error message; I'm just \\showing something.",
                    b"Type `I\\show...' to show more (e.g., \\show\\cs,",
                    b"\\showthe\\count10, \\showbox255, \\showlists).",
                ]);
            } else {
                self.help(&[
                    b"This isn't an error message; I'm just \\showing something.",
                    b"Type `I\\show...' to show more (e.g., \\show\\cs,",
                    b"\\showthe\\count10, \\showbox255, \\showlists).",
                    b"And type `I\\tracingonline=1\\show...' to show boxes and",
                    b"lists on your terminal as well as in the transcript file.",
                ]);
            }
            self.error();
        }
    }
}

/// Only what runs with real callees in this tree: the procedures that read no number, since the scanners are other modules'.
/// The rest is in tests/integration/prefixed/.  `testdata/prefixed_oracle.tsv` comes from tests/oracle/gen_prefixed_tables.py.
#[cfg(test)]
mod tests {
    use super::*;
    use crate::save::testkit::table_machine;
    use crate::toks::testkit::*;

    /// `table_machine` with the primitives of the WEB, printing to the terminal in scroll mode.
    fn prim_machine() -> &'static mut Tex {
        let t = table_machine();
        print_setup(t);
        t.no_new_control_sequence = false;
        t.prim_used = PRIM_SIZE;
        for l in include_str!("../testdata/primitives.tsv").lines() {
            let f: Vec<&str> = l.split('\t').collect();
            let (cmd, chr): (i32, i32) = (f[1].parse().unwrap(), f[2].parse().unwrap());
            let s = f[0].as_bytes().to_vec();
            t.primitive(&s, cmd, chr);
        }
        t.no_new_control_sequence = false;
        t.set_text(FROZEN_PROTECTION, S_INACCESSIBLE);
        t
    }

    /// Reads the line as main_control would, for the commands that can run here; anything else is dropped.
    fn run_line(t: &mut Tex, line: &str) {
        t.first = 1;
        t.error_count = 0;
        // input_ln removes the spaces at the end of a line.
        set_line(t, line.trim_end_matches(' ').as_bytes(), NEW_LINE);
        loop {
            t.get_x_token();
            if t.cur_tok == 0 {
                return;
            }
            if t.cur_cmd == XRAY {
                t.show_whatever();
            } else if t.cur_cmd > MAX_NON_PREFIXED_COMMAND {
                t.prefixed_command();
            }
        }
    }

    #[test]
    fn lines_against_the_oracle() {
        let t = prim_machine();
        let mut n = 0;
        for row in include_str!("../testdata/prefixed_oracle.tsv").lines() {
            let f: Vec<&str> = row.split('\t').collect();
            run_line(t, f[0]);
            let s = printed(t);
            let got: Vec<&str> = s.lines().filter(|l| l.starts_with("> ") || l.starts_with("! ") || l.starts_with("### ")).collect();
            assert_eq!(got, &f[1..], "for {}\nall of it: {}", f[0], s);
            n += 1;
        }
        assert!(n > 70, "{} rows", n);
        Tex::dealloc(t);
    }

    #[test]
    fn get_r_token_inserts_the_inaccessible_control_sequence() {
        let t = prim_machine();
        // cur_cs=0: the token goes back after the insertion, so it is the next one read.
        set_line(t, b"1", NEW_LINE);
        t.get_r_token();
        assert_eq!(t.cur_cs, FROZEN_PROTECTION);
        assert_eq!(t.cur_tok, CS_TOKEN_FLAG + FROZEN_PROTECTION);
        t.get_token();
        assert_eq!(t.cur_tok, OTHER_TOKEN + b'1' as i32);
        let s = printed(t);
        assert!(s.starts_with("! Missing control sequence inserted.\n"), "got {:?}", s);
        // A frozen control sequence: nothing goes back.
        t.first = 1;
        feed(t, &[CS_TOKEN_FLAG + FROZEN_END_TEMPLATE, LETTER_TOKEN + b'x' as i32]);
        t.get_r_token();
        assert_eq!(t.cur_cs, FROZEN_PROTECTION);
        t.get_token();
        assert_eq!(t.cur_tok, LETTER_TOKEN + b'x' as i32);
        printed(t);
        // space_token is skipped; a space with another code is an error.
        feed(t, &[SPACE_TOKEN, SPACE_TOKEN, cs_tok(b'q')]);
        t.get_r_token();
        assert_eq!(t.cur_cs, cs(b'q'));
        assert_eq!(printed(t), "");
        feed(t, &[SPACE_TOKEN + 1, cs_tok(b'q')]);
        t.get_r_token();
        assert_eq!(t.cur_cs, FROZEN_PROTECTION);
        t.get_token();
        assert_eq!(t.cur_tok, SPACE_TOKEN + 1);
        Tex::dealloc(t);
    }

    #[test]
    fn let_counts_a_reference_to_a_macro_and_gives_it_back() {
        let t = prim_machine();
        let h = def_macro(t, cs(b'm'), CALL, &[], &[LETTER_TOKEN + b'x' as i32]);
        assert_eq!(t.token_ref_count(h), 0);
        run_line(t, "\\let\\b\\m");
        assert_eq!((t.eq_type(cs(b'b')), t.equiv(cs(b'b'))), (CALL, h));
        assert_eq!(t.token_ref_count(h), 1);
        run_line(t, "\\futurelet\\c\\relax\\m");
        assert_eq!(t.token_ref_count(h), 2);
        run_line(t, "\\let\\b\\relax");
        assert_eq!(t.token_ref_count(h), 1);
        run_line(t, "\\let\\c\\relax");
        assert_eq!(t.token_ref_count(h), 0);
        assert_eq!(t.toks[h + 2], LETTER_TOKEN + b'x' as i32, "the list is still \\m's");
        Tex::dealloc(t);
    }

    #[test]
    fn let_of_a_register_counts_only_a_sparse_element() {
        let t = prim_machine();
        // The primitive's cur_chr is in mem_bot..lo_mem_stat_max: no element.
        run_line(t, "\\let\\b\\count");
        assert_eq!((t.eq_type(cs(b'b')), t.equiv(cs(b'b'))), (REGISTER, MEM_BOT + INT_VAL));
        run_line(t, "\\let\\c\\toks");
        assert_eq!((t.eq_type(cs(b'c')), t.equiv(cs(b'c'))), (TOKS_REGISTER, MEM_BOT));
        // An element.
        for cmd in [REGISTER, TOKS_REGISTER] {
            let p = t.get_node(WORD_NODE_SIZE);
            t.set_sa_ref(p, 0);
            t.set_eq_type(cs(b's'), cmd);
            t.set_equiv(cs(b's'), p);
            run_line(t, "\\let\\d\\s");
            assert_eq!(t.sa_ref(p), 1);
            assert_eq!((t.eq_type(cs(b'd')), t.equiv(cs(b'd'))), (cmd, p));
            // So that the next round's \let\d does not destroy the element: eq_destroy would call etex.rs.
            t.set_eq_type(cs(b'd'), UNDEFINED_CS);
            t.set_equiv(cs(b'd'), NULL);
            t.set_eq_level(cs(b'd'), LEVEL_ZERO);
        }
        Tex::dealloc(t);
    }

    /// The font is the simplest assignment that needs no number: the adjustment for \globaldefs and the choice of define.
    #[test]
    fn global_defs_and_the_global_prefix() {
        let t = prim_machine();
        t.set_eq_type(cs(b'f'), SET_FONT);
        t.set_equiv(cs(b'f'), 7);
        for (global_defs, line, kept) in [
            (0, "\\f", false),
            (0, "\\global\\f", true),
            (0, "\\global\\global\\f", true),
            (1, "\\f", true),
            (1, "\\global\\f", true),
            (5, "\\f", true),
            (-1, "\\f", false),
            (-1, "\\global\\f", false),
            (-5, "\\global\\global\\f", false),
        ] {
            t.set_global_defs(global_defs);
            t.new_save_level(SIMPLE_GROUP);
            run_line(t, line);
            assert_eq!(t.cur_font(), 7, "{}", line);
            t.unsave();
            assert_eq!(t.cur_font(), if kept { 7 } else { 0 }, "defs {} {}", global_defs, line);
            t.set_global_defs(0);
            t.geq_define(CUR_FONT_LOC, DATA, 0);
        }
        Tex::dealloc(t);
    }

    #[test]
    fn after_token_goes_back_at_done_and_stays_at_a_return() {
        let t = prim_machine();
        t.set_eq_type(cs(b'f'), SET_FONT);
        let z = LETTER_TOKEN + b'z' as i32;
        // done
        t.after_token = z;
        t.first = 1;
        set_line(t, b"\\f y", NEW_LINE);
        t.get_x_token();
        t.prefixed_command();
        assert_eq!(t.after_token, 0);
        t.get_token();
        assert_eq!(t.cur_tok, z);
        t.get_token();
        assert_eq!(t.cur_tok, LETTER_TOKEN + b'y' as i32);
        // The return of a prefix before a command that takes none.
        t.after_token = z;
        run_line(t, "\\long\\hrule");
        assert_eq!(t.after_token, z);
        // do_register_command's own return comes back to prefixed_command, which goes on to done.
        run_line(t, "\\advance x");
        assert_eq!(t.after_token, 0);
        printed(t);
        Tex::dealloc(t);
    }

    #[test]
    fn trap_zero_glue_replaces_only_a_zero_spec() {
        let t = prim_machine();
        let before = t.glue_ref_count(ZERO_GLUE);
        let p = t.new_spec(ZERO_GLUE);
        t.set_stretch_order(p, FILLL);
        t.set_shrink_order(p, FIL);
        t.cur_val = p;
        t.trap_zero_glue();
        assert_eq!(t.cur_val, ZERO_GLUE, "the orders do not matter");
        assert_eq!(t.glue_ref_count(ZERO_GLUE), before + 1);
        for (w, y, z) in [(1, 0, 0), (0, 1, 0), (0, 0, 1), (-1, 0, 0)] {
            let p = t.new_spec(ZERO_GLUE);
            t.set_width(p, w);
            t.set_stretch(p, y);
            t.set_shrink(p, z);
            t.cur_val = p;
            t.trap_zero_glue();
            assert_eq!(t.cur_val, p);
            assert_eq!(t.glue_ref_count(ZERO_GLUE), before + 1);
        }
        Tex::dealloc(t);
    }

    #[test]
    fn new_interaction_sets_the_selector() {
        let t = prim_machine();
        for (mode, log_opened, selector) in [
            (BATCH_MODE, false, NO_PRINT),
            (NONSTOP_MODE, false, TERM_ONLY),
            (SCROLL_MODE, false, TERM_ONLY),
            (ERROR_STOP_MODE, false, TERM_ONLY),
            (BATCH_MODE, true, LOG_ONLY),
            (NONSTOP_MODE, true, TERM_AND_LOG),
            (SCROLL_MODE, true, TERM_AND_LOG),
            (ERROR_STOP_MODE, true, TERM_AND_LOG),
        ] {
            t.selector = TERM_ONLY;
            t.cur_chr = mode;
            t.log_opened = log_opened;
            t.new_interaction();
            assert_eq!((t.interaction, t.selector), (mode, selector));
        }
        Tex::dealloc(t);
    }

    #[test]
    fn give_err_help_shows_the_list() {
        let t = prim_machine();
        t.give_err_help();
        assert_eq!(printed(t), "", "null shows nothing");
        let m = t.tb_mark();
        for c in *b"help" {
            t.tb_push(LETTER_TOKEN + c as i32);
        }
        let p = t.tb_commit(m);
        t.set_equiv(ERR_HELP_LOC, p);
        t.give_err_help();
        assert_eq!(printed(t), "help");
        Tex::dealloc(t);
    }

    /// What the oracle's log has for `\iftrue\iffalse\else\iftrue\showifs`: tests/integration/prefixed/show_etex.tex.
    #[test]
    fn showifs_walks_the_conditions() {
        let t = prim_machine();
        let push = |t: &mut Tex, code: i32, limit: i32| {
            let p = t.get_node(IF_NODE_SIZE);
            t.set_link(p, t.cond_ptr);
            t.set_ty(p, t.if_limit);
            t.set_subtype(p, t.cur_if);
            t.set_if_line_field(p, t.if_line);
            t.cond_ptr = p;
            t.cur_if = code;
            t.if_limit = limit;
            t.if_line = 12;
        };
        push(t, IF_TRUE_CODE, ELSE_CODE);
        push(t, IF_FALSE_CODE, FI_CODE);
        push(t, IF_TRUE_CODE, ELSE_CODE);
        run_line(t, "\\showifs");
        let s = printed(t);
        let got: Vec<&str> = s.lines().filter(|l| l.starts_with("###")).collect();
        assert_eq!(
            got,
            [
                "### level 3: \\iftrue entered on line 12",
                "### level 2: \\iffalse\\else entered on line 12",
                "### level 1: \\iftrue entered on line 12",
            ]
        );
        Tex::dealloc(t);
    }
}
