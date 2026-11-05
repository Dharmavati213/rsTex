//! §428-489 Part 26: basic scanning subroutines.
//!
//! A procedure includes every module reachable from it, wherever defined: `tools/web.py tangle NAME`.
//! §478's digit stack stays in `mem`, PORTING section 16.
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

/// The first and last of the date parameters, read by `\the\time` and the like: volatile reads.
const TIME_LOC: i32 = INT_BASE + TIME_CODE;
const YEAR_LOC: i32 = INT_BASE + YEAR_CODE;

impl Tex {
    /// §429
    pub fn scan_left_brace(&mut self) {
        // §430 Get the next non-blank non-relax non-call token
        loop {
            self.get_x_token();
            if self.cur_cmd != SPACER && self.cur_cmd != RELAX {
                break;
            }
        }
        if self.cur_cmd != LEFT_BRACE {
            self.print_err_s(b"Missing { inserted");
            self.help(&[
                b"A left brace was mandatory here, so I've put one in.",
                b"You might want to delete and/or insert some corrections",
                b"so that I will find a matching right brace soon.",
                b"(If you're confused by all this, try typing `I}' now.)",
            ]);
            self.back_error();
            self.cur_tok = LEFT_BRACE_TOKEN + b'{' as i32;
            self.cur_cmd = LEFT_BRACE;
            self.cur_chr = b'{' as i32;
            self.align_state += 1;
        }
    }

    /// §431
    pub fn scan_optional_equals(&mut self) {
        // §432 Get the next non-blank non-call token
        loop {
            self.get_x_token();
            if self.cur_cmd != SPACER {
                break;
            }
        }
        if self.cur_tok != OTHER_TOKEN + b'=' as i32 {
            self.back_input();
        }
    }

    /// §433
    /// The tokens go on the tx stack, not on `tb`: see Tex::tx_mark.
    pub fn scan_keyword(&mut self, s: &[u8]) -> bool {
        // scan_dimen's units and scan_glue's plus and minus each look at the same token, in the one-token level the one
        // before it made, and most are told no by its first letter.  Reading it and backing it up again would
        // give that level again, unless the one below has ended and would go too.
        if self.state() == TOKEN_LIST && self.token_type() == BACKED_UP && self.loc() == self.start() + 1 {
            let t = self.toks[self.loc()];
            let (cmd, chr, c) = (t >> 8, t & 255, s[0] as i32);
            if (cmd == LETTER || cmd == OTHER_CHAR)
                && self.toks[self.loc() + 1] == 0
                && chr != c
                && chr != c - b'a' as i32 + b'A' as i32
                && !self.rec_at_list_end(self.input_stack[self.input_ptr - 1])
            {
                self.cur_cmd = cmd;
                self.cur_chr = chr;
                self.cur_tok = t;
                return false;
            }
        }
        let m = self.tx_mark();
        let save_cur_cs = self.cur_cs;
        let mut k = 0;
        while k < s.len() {
            self.get_x_token(); // recursion is possible here
            let c = s[k] as i32;
            if self.cur_cs == 0 && (self.cur_chr == c || self.cur_chr == c - b'a' as i32 + b'A' as i32) {
                self.tx_push(self.cur_tok);
                k += 1;
            } else if self.cur_cmd != SPACER || self.tx_len(m) != 0 {
                self.back_input();
                if self.tx_len(m) != 0 {
                    let p = self.tx_commit(m);
                    self.back_list(p);
                }
                self.cur_cs = save_cur_cs;
                return false;
            }
        }
        self.tx_discard(m);
        true
    }

    /// §434
    #[cold]
    pub fn mu_error(&mut self) {
        self.print_err_s(b"Incompatible glue units");
        self.help(&[b"I'm going to assume that 1mu=1pt when they're mixed."]);
        self.error();
    }

    /// §439.  The commonest commands, for which §455 only lowers the level and §456 only negates; the others, and
    /// `\\time` and its like, are in `scan_internal_others`.
    pub fn scan_something_internal(&mut self, level: i32, negative: bool) {
        let m = self.cur_chr;
        match self.cur_cmd {
            ASSIGN_DIMEN => self.scanned_result(self.eqtb[m].sc(), DIMEN_VAL),
            ASSIGN_INT if m < TIME_LOC || m > YEAR_LOC => self.scanned_result(self.eqtb[m].int(), INT_VAL),
            CHAR_GIVEN | MATH_GIVEN => self.scanned_result(m, INT_VAL),
            _ => {
                self.scan_internal_others(level, negative);
                return;
            }
        }
        if self.cur_val_level > level {
            self.cur_val_level = level;
        }
        if negative {
            self.cur_val = self.cur_val.wrapping_neg();
        }
    }

    /// §439 as WEB has it.
    fn scan_internal_others(&mut self, level: i32, negative: bool) {
        let mut m;
        'restart: loop {
            m = self.cur_chr;
            match self.cur_cmd {
                DEF_CODE => {
                    // §440 Fetch a character code from some table
                    self.scan_char_num();
                    if m == MATH_CODE_BASE {
                        self.scanned_result(self.math_code(self.cur_val), INT_VAL);
                    } else if m < MATH_CODE_BASE {
                        self.scanned_result(self.equiv(m + self.cur_val), INT_VAL);
                    } else {
                        self.scanned_result(self.eqtb[m + self.cur_val].int(), INT_VAL);
                    }
                }
                TOKS_REGISTER | ASSIGN_TOKS | DEF_FAMILY | SET_FONT | DEF_FONT | LETTERSPACE_FONT | PDF_COPY_FONT => {
                    // §441 Fetch a token list or font identifier, provided that level=tok_val
                    if level != TOK_VAL {
                        self.print_err_s(b"Missing number, treated as zero");
                        self.help(&[
                            b"A number should have been here; I inserted `0'.",
                            b"(If you can't figure out why I needed to see a number,",
                            b"look up `weird error' in the index to The TeXbook.)",
                        ]);
                        self.back_error();
                        self.scanned_result(0, DIMEN_VAL);
                    } else if self.cur_cmd <= ASSIGN_TOKS {
                        if self.cur_cmd < ASSIGN_TOKS {
                            // cur_cmd=toks_register
                            if m == MEM_BOT {
                                self.scan_register_num();
                                if self.cur_val < 256 {
                                    self.cur_val = self.equiv(TOKS_BASE + self.cur_val);
                                } else {
                                    self.find_sa_element(TOK_VAL, self.cur_val, false);
                                    if self.cur_ptr == NULL {
                                        self.cur_val = NULL;
                                    } else {
                                        self.cur_val = self.sa_ptr(self.cur_ptr);
                                    }
                                }
                            } else {
                                self.cur_val = self.sa_ptr(m);
                            }
                        } else {
                            self.cur_val = self.equiv(m);
                        }
                        self.cur_val_level = TOK_VAL;
                    } else {
                        self.back_input();
                        self.scan_font_ident();
                        self.scanned_result(FONT_ID_BASE + self.cur_val, IDENT_VAL);
                    }
                }
                ASSIGN_INT => {
                    if m >= TIME_LOC && m <= YEAR_LOC {
                        self.scan_date_read(m);
                    }
                    self.scanned_result(self.eqtb[m].int(), INT_VAL);
                }
                ASSIGN_DIMEN => self.scanned_result(self.eqtb[m].sc(), DIMEN_VAL),
                ASSIGN_GLUE => self.scanned_result(self.equiv(m), GLUE_VAL),
                ASSIGN_MU_GLUE => self.scanned_result(self.equiv(m), MU_VAL),
                SET_AUX => {
                    // §444 Fetch the space_factor or the prev_depth
                    if abs(self.mode()) != m {
                        self.print_err_s(b"Improper ");
                        self.print_cmd_chr(SET_AUX, m);
                        self.help(&[
                            b"You can refer to \\spacefactor only in horizontal mode;",
                            b"you can refer to \\prevdepth only in vertical mode; and",
                            b"neither of these is meaningful inside \\write. So",
                            b"I'm forgetting what you said and using zero instead.",
                        ]);
                        self.error();
                        if level != TOK_VAL {
                            self.scanned_result(0, DIMEN_VAL);
                        } else {
                            self.scanned_result(0, INT_VAL);
                        }
                    } else if m == VMODE {
                        self.scanned_result(self.prev_depth(), DIMEN_VAL);
                    } else {
                        self.scanned_result(self.space_factor(), INT_VAL);
                    }
                }
                SET_PREV_GRAF => {
                    // §448 Fetch the prev_graf
                    if self.mode() == 0 {
                        self.scanned_result(0, INT_VAL); // prev_graf=0 within \write
                    } else {
                        self.nest[self.nest_ptr] = self.cur_list;
                        let mut p = self.nest_ptr;
                        while abs(self.nest[p].mode_field) != VMODE {
                            p -= 1;
                        }
                        self.scanned_result(self.nest[p].pg_field, INT_VAL);
                    }
                }
                SET_PAGE_INT => {
                    // §445 Fetch the dead_cycles or the insert_penalties
                    if m == 0 {
                        self.cur_val = self.dead_cycles;
                    } else if m == 2 {
                        // §1694
                        self.cur_val = self.interaction;
                    } else {
                        self.cur_val = wrap32(self.insert_penalties);
                    }
                    self.cur_val_level = INT_VAL;
                }
                SET_PAGE_DIMEN => {
                    // §447 Fetch something on the page_so_far
                    if self.page_contents == EMPTY && !self.output_active {
                        if m == 0 {
                            self.cur_val = MAX_DIMEN;
                        } else {
                            self.cur_val = 0;
                        }
                    } else {
                        self.cur_val = wrap32(self.page_so_far[m]);
                    }
                    self.cur_val_level = DIMEN_VAL;
                }
                SET_SHAPE => {
                    // §449 Fetch the par_shape size
                    if m > PAR_SHAPE_LOC {
                        // §1866 Fetch a penalties array element
                        self.scan_int();
                        if self.equiv(m) == NULL || self.cur_val < 0 {
                            self.cur_val = 0;
                        } else {
                            if self.cur_val > self.penalty(self.equiv(m)) {
                                self.cur_val = self.penalty(self.equiv(m));
                            }
                            self.cur_val = self.penalty(self.equiv(m) + self.cur_val);
                        }
                    } else if self.par_shape_ptr() == NULL {
                        self.cur_val = 0;
                    } else {
                        self.cur_val = self.info(self.par_shape_ptr());
                    }
                    self.cur_val_level = INT_VAL;
                }
                SET_BOX_DIMEN => {
                    // §446 Fetch a box dimension
                    self.scan_register_num();
                    let q = self.fetch_box();
                    if q == NULL {
                        self.cur_val = 0;
                    } else {
                        self.cur_val = self.mem[q + m].sc();
                    }
                    self.cur_val_level = DIMEN_VAL;
                }
                CHAR_GIVEN | MATH_GIVEN => self.scanned_result(self.cur_chr, INT_VAL),
                ASSIGN_FONT_DIMEN => {
                    // §451 Fetch a font dimension
                    self.find_font_dimen(false);
                    let k = self.fmem_ptr;
                    self.font_info[k].set_sc(0);
                    self.scanned_result(self.font_info[self.cur_val].sc(), DIMEN_VAL);
                }
                ASSIGN_FONT_INT => {
                    // §452 Fetch a font integer
                    self.scan_font_ident();
                    if m == 0 {
                        self.scanned_result(self.hyphen_char[self.cur_val], INT_VAL);
                    } else if m == 1 {
                        self.scanned_result(self.skew_char[self.cur_val], INT_VAL);
                    } else if m == NO_LIG_CODE {
                        let v = self.test_no_ligatures(self.cur_val);
                        self.scanned_result(v, INT_VAL);
                    } else {
                        let n = self.cur_val;
                        self.scan_char_num();
                        let k = self.cur_val;
                        match m {
                            LP_CODE_BASE => {
                                let v = self.get_lp_code(n, k);
                                self.scanned_result(v, INT_VAL);
                            }
                            RP_CODE_BASE => {
                                let v = self.get_rp_code(n, k);
                                self.scanned_result(v, INT_VAL);
                            }
                            EF_CODE_BASE => {
                                let v = self.get_ef_code(n, k);
                                self.scanned_result(v, INT_VAL);
                            }
                            TAG_CODE => {
                                let v = self.get_tag_code(n, k);
                                self.scanned_result(v, INT_VAL);
                            }
                            KN_BS_CODE_BASE => {
                                let v = self.get_kn_bs_code(n, k);
                                self.scanned_result(v, INT_VAL);
                            }
                            ST_BS_CODE_BASE => {
                                let v = self.get_st_bs_code(n, k);
                                self.scanned_result(v, INT_VAL);
                            }
                            SH_BS_CODE_BASE => {
                                let v = self.get_sh_bs_code(n, k);
                                self.scanned_result(v, INT_VAL);
                            }
                            KN_BC_CODE_BASE => {
                                let v = self.get_kn_bc_code(n, k);
                                self.scanned_result(v, INT_VAL);
                            }
                            KN_AC_CODE_BASE => {
                                let v = self.get_kn_ac_code(n, k);
                                self.scanned_result(v, INT_VAL);
                            }
                            _ => {}
                        }
                    }
                }
                REGISTER => {
                    // §453 Fetch a register
                    if m < MEM_BOT || m > LO_MEM_STAT_MAX {
                        self.cur_val_level = self.sa_type(m);
                        if self.cur_val_level < GLUE_VAL {
                            self.cur_val = self.sa_int(m);
                        } else {
                            self.cur_val = self.sa_ptr(m);
                        }
                    } else {
                        self.scan_register_num();
                        self.cur_val_level = m - MEM_BOT;
                        if self.cur_val > 255 {
                            self.find_sa_element(self.cur_val_level, self.cur_val, false);
                            if self.cur_ptr == NULL {
                                if self.cur_val_level < GLUE_VAL {
                                    self.cur_val = 0;
                                } else {
                                    self.cur_val = ZERO_GLUE;
                                }
                            } else if self.cur_val_level < GLUE_VAL {
                                self.cur_val = self.sa_int(self.cur_ptr);
                            } else {
                                self.cur_val = self.sa_ptr(self.cur_ptr);
                            }
                        } else {
                            match self.cur_val_level {
                                INT_VAL => self.cur_val = self.count(self.cur_val),
                                DIMEN_VAL => self.cur_val = self.dimen(self.cur_val),
                                GLUE_VAL => self.cur_val = self.skip(self.cur_val),
                                MU_VAL => self.cur_val = self.mu_skip(self.cur_val),
                                _ => {} // there are no other cases
                            }
                        }
                    }
                }
                LAST_ITEM => {
                    // §450 Fetch an item in the current node, if appropriate
                    if m >= INPUT_LINE_NO_CODE {
                        if m >= ETEX_GLUE {
                            // §1780 Process an expression and return
                            if m < ETEX_MU {
                                // §1807
                                if m == MU_TO_GLUE_CODE {
                                    self.scan_mu_glue();
                                }
                                self.cur_val_level = GLUE_VAL;
                            } else if m < ETEX_EXPR {
                                // §1808
                                if m == GLUE_TO_MU_CODE {
                                    self.scan_normal_glue();
                                }
                                self.cur_val_level = MU_VAL;
                            } else {
                                self.cur_val_level = m - ETEX_EXPR + INT_VAL;
                                self.scan_expr();
                            }
                            while self.cur_val_level > level {
                                if self.cur_val_level == GLUE_VAL {
                                    m = self.cur_val;
                                    self.cur_val = self.width(m);
                                    self.delete_glue_ref(m);
                                } else if self.cur_val_level == MU_VAL {
                                    self.mu_error();
                                }
                                self.cur_val_level -= 1;
                            }
                            if negative {
                                if self.cur_val_level >= GLUE_VAL {
                                    m = self.cur_val;
                                    self.cur_val = self.new_spec(m);
                                    self.delete_glue_ref(m);
                                    self.scan_negate_glue();
                                } else {
                                    self.cur_val = self.cur_val.wrapping_neg();
                                }
                            }
                            return;
                        } else if m >= ETEX_DIM {
                            match m {
                                // §1671
                                FONT_CHAR_WD_CODE | FONT_CHAR_HT_CODE | FONT_CHAR_DP_CODE | FONT_CHAR_IC_CODE => {
                                    self.scan_font_ident();
                                    let q = self.cur_val;
                                    self.scan_char_num();
                                    if self.font_bc[q] <= self.cur_val && self.font_ec[q] >= self.cur_val {
                                        let i = self.char_info(q, self.cur_val);
                                        match m {
                                            FONT_CHAR_WD_CODE => self.cur_val = self.char_width(q, i),
                                            FONT_CHAR_HT_CODE => {
                                                self.cur_val = self.char_height(q, self.height_depth(i))
                                            }
                                            FONT_CHAR_DP_CODE => {
                                                self.cur_val = self.char_depth(q, self.height_depth(i))
                                            }
                                            FONT_CHAR_IC_CODE => self.cur_val = self.char_italic(q, i),
                                            _ => {} // there are no other cases
                                        }
                                    } else {
                                        self.cur_val = 0;
                                    }
                                }
                                // §1674
                                PAR_SHAPE_LENGTH_CODE | PAR_SHAPE_INDENT_CODE | PAR_SHAPE_DIMEN_CODE => {
                                    let mut q = self.cur_chr - PAR_SHAPE_LENGTH_CODE;
                                    self.scan_int();
                                    if self.par_shape_ptr() == NULL || self.cur_val <= 0 {
                                        self.cur_val = 0;
                                    } else {
                                        if q == 2 {
                                            q = self.cur_val % 2;
                                            // cur_val may be 2^31-1: the oracle's integer is 64 bits
                                            self.cur_val = ((self.cur_val as i64 + q as i64) / 2) as i32;
                                        }
                                        if self.cur_val > self.info(self.par_shape_ptr()) {
                                            self.cur_val = self.info(self.par_shape_ptr());
                                        }
                                        self.cur_val = self.mem[self.par_shape_ptr() + 2 * self.cur_val - q].sc();
                                    }
                                    self.cur_val_level = DIMEN_VAL;
                                }
                                // §1804
                                GLUE_STRETCH_CODE | GLUE_SHRINK_CODE => {
                                    self.scan_normal_glue();
                                    let q = self.cur_val;
                                    if m == GLUE_STRETCH_CODE {
                                        self.cur_val = self.stretch(q);
                                    } else {
                                        self.cur_val = self.shrink(q);
                                    }
                                    self.delete_glue_ref(q);
                                }
                                _ => {} // there are no other cases
                            }
                            self.cur_val_level = DIMEN_VAL;
                        } else {
                            match m {
                                INPUT_LINE_NO_CODE => self.cur_val = self.line,
                                BADNESS_CODE => self.cur_val = self.last_badness,
                                PDFTEX_VERSION_CODE => self.cur_val = PDFTEX_VERSION,
                                PDF_LAST_OBJ_CODE => self.cur_val = self.pdf_last_obj,
                                PDF_LAST_XFORM_CODE => self.cur_val = self.pdf_last_xform,
                                PDF_LAST_XIMAGE_CODE => self.cur_val = self.pdf_last_ximage,
                                PDF_LAST_XIMAGE_PAGES_CODE => self.cur_val = self.pdf_last_ximage_pages,
                                PDF_LAST_ANNOT_CODE => self.cur_val = self.pdf_last_annot,
                                PDF_LAST_X_POS_CODE => self.cur_val = self.pdf_last_x_pos,
                                PDF_LAST_Y_POS_CODE => self.cur_val = self.pdf_last_y_pos,
                                PDF_RETVAL_CODE => self.cur_val = self.pdf_retval,
                                PDF_LAST_XIMAGE_COLORDEPTH_CODE => self.cur_val = self.pdf_last_ximage_colordepth,
                                ELAPSED_TIME_CODE => {
                                    self.scan_volatile_read();
                                    self.cur_val = self.get_microinterval();
                                }
                                RANDOM_SEED_CODE => {
                                    self.scan_volatile_read();
                                    self.cur_val = self.random_seed;
                                }
                                PDF_SHELL_ESCAPE_CODE => {
                                    if self.shellenabledp != 0 {
                                        if self.restrictedshell != 0 {
                                            self.cur_val = 2;
                                        } else {
                                            self.cur_val = 1;
                                        }
                                    } else {
                                        self.cur_val = 0;
                                    }
                                }
                                PDF_LAST_LINK_CODE => self.cur_val = self.pdf_last_link,
                                // §1651
                                ETEX_VERSION_CODE => self.cur_val = ETEX_VERSION,
                                // §1665
                                CURRENT_GROUP_LEVEL_CODE => self.cur_val = self.cur_level - LEVEL_ONE,
                                CURRENT_GROUP_TYPE_CODE => self.cur_val = self.cur_group,
                                // §1668
                                CURRENT_IF_LEVEL_CODE => {
                                    let mut q = self.cond_ptr;
                                    self.cur_val = 0;
                                    while q != NULL {
                                        self.cur_val += 1;
                                        q = self.link(q);
                                    }
                                }
                                CURRENT_IF_TYPE_CODE => {
                                    if self.cond_ptr == NULL {
                                        self.cur_val = 0;
                                    } else if self.cur_if < UNLESS_CODE {
                                        self.cur_val = self.cur_if + 1;
                                    } else {
                                        self.cur_val = -(self.cur_if - UNLESS_CODE + 1);
                                    }
                                }
                                CURRENT_IF_BRANCH_CODE => {
                                    if self.if_limit == OR_CODE || self.if_limit == ELSE_CODE {
                                        self.cur_val = 1;
                                    } else if self.if_limit == FI_CODE {
                                        self.cur_val = -1;
                                    } else {
                                        self.cur_val = 0;
                                    }
                                }
                                // §1803
                                GLUE_STRETCH_ORDER_CODE | GLUE_SHRINK_ORDER_CODE => {
                                    self.scan_normal_glue();
                                    let q = self.cur_val;
                                    if m == GLUE_STRETCH_ORDER_CODE {
                                        self.cur_val = self.stretch_order(q);
                                    } else {
                                        self.cur_val = self.shrink_order(q);
                                    }
                                    self.delete_glue_ref(q);
                                }
                                _ => {} // there are no other cases
                            }
                            self.cur_val_level = INT_VAL;
                        }
                    } else {
                        if self.cur_chr == GLUE_VAL {
                            self.cur_val = ZERO_GLUE;
                        } else {
                            self.cur_val = 0;
                        }
                        let tx = self.scan_find_effective_tail();
                        if self.cur_chr == LAST_NODE_TYPE_CODE {
                            self.cur_val_level = INT_VAL;
                            if tx == self.head() || self.mode() == 0 {
                                self.cur_val = -1;
                            }
                        } else {
                            self.cur_val_level = self.cur_chr;
                        }
                        if !self.is_char_node(tx) && self.mode() != 0 {
                            match self.cur_chr {
                                INT_VAL => {
                                    if self.ty(tx) == PENALTY_NODE {
                                        self.cur_val = self.penalty(tx);
                                    }
                                }
                                DIMEN_VAL => {
                                    if self.ty(tx) == KERN_NODE {
                                        self.cur_val = self.width(tx);
                                    }
                                }
                                GLUE_VAL => {
                                    if self.ty(tx) == GLUE_NODE {
                                        self.cur_val = self.glue_ptr(tx);
                                        if self.subtype(tx) == MU_GLUE {
                                            self.cur_val_level = MU_VAL;
                                        }
                                    }
                                }
                                LAST_NODE_TYPE_CODE => {
                                    if self.ty(tx) <= UNSET_NODE {
                                        self.cur_val = self.ty(tx) + 1;
                                    } else {
                                        self.cur_val = UNSET_NODE + 2;
                                    }
                                }
                                _ => {} // there are no other cases
                            }
                        } else if self.mode() == VMODE && tx == self.head() {
                            match self.cur_chr {
                                INT_VAL => self.cur_val = self.last_penalty,
                                DIMEN_VAL => self.cur_val = self.last_kern,
                                GLUE_VAL => {
                                    if self.last_glue != MAX_HALFWORD {
                                        self.cur_val = self.last_glue;
                                    }
                                }
                                LAST_NODE_TYPE_CODE => self.cur_val = self.last_node_type,
                                _ => {} // there are no other cases
                            }
                        }
                    }
                }
                IGNORE_SPACES => {
                    // trap unexpandable primitives
                    if self.cur_chr == 1 {
                        self.scan_reset_primitive(); // §395
                        continue 'restart;
                    }
                }
                // §454 Complain that \the can't do this; give zero result
                _ => self.scan_cannot_use_after_the(level),
            }
            break;
        }
        while self.cur_val_level > level {
            // §455 Convert cur_val to a lower level
            if self.cur_val_level == GLUE_VAL {
                self.cur_val = self.width(self.cur_val);
            } else if self.cur_val_level == MU_VAL {
                self.mu_error();
            }
            self.cur_val_level -= 1;
        }
        // §456 Fix the reference count, if any, and negate cur_val if negative
        if negative {
            if self.cur_val_level >= GLUE_VAL {
                self.cur_val = self.new_spec(self.cur_val);
                self.scan_negate_glue();
            } else {
                self.cur_val = self.cur_val.wrapping_neg();
            }
        } else if self.cur_val_level >= GLUE_VAL && self.cur_val_level <= MU_VAL {
            self.add_glue_ref(self.cur_val);
        }
    }

    /// §457 Negate all three glue components of cur_val
    fn scan_negate_glue(&mut self) {
        let q = self.cur_val;
        self.set_width(q, self.width(q).wrapping_neg());
        self.set_stretch(q, self.stretch(q).wrapping_neg());
        self.set_shrink(q, self.shrink(q).wrapping_neg());
    }

    /// §450 `find_effective_tail`, which is `find_effective_tail_eTeX`; returns tx.
    #[inline]
    fn scan_find_effective_tail(&mut self) -> i32 {
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

    /// §395 Reset cur_tok for unexpandable primitives; the caller does the `goto restart`.
    fn scan_reset_primitive(&mut self) {
        self.get_token();
        if self.cur_cs < HASH_BASE {
            self.cur_cs = self.prim_lookup(self.cur_cs - SINGLE_BASE);
        } else {
            self.cur_cs = self.prim_lookup(self.text(self.cur_cs));
        }
        if self.cur_cs != UNDEFINED_PRIMITIVE {
            self.cur_cmd = self.prim_eq_type(self.cur_cs);
            self.cur_chr = self.prim_equiv(self.cur_cs);
            self.cur_cs = PRIM_EQTB_BASE + self.cur_cs;
            self.cur_tok = CS_TOKEN_FLAG + self.cur_cs;
        } else {
            self.cur_cmd = RELAX;
            self.cur_chr = 0;
            self.cur_tok = CS_TOKEN_FLAG + FROZEN_RELAX;
            self.cur_cs = FROZEN_RELAX;
        }
    }

    /// §454 Complain that \the can't do this; give zero result
    #[cold]
    fn scan_cannot_use_after_the(&mut self, level: i32) {
        self.print_err_s(b"You can't use `");
        self.print_cmd_chr(self.cur_cmd, self.cur_chr);
        self.print_s(b"' after ");
        self.print_esc_s(b"the");
        self.help(&[b"I'm forgetting what you said and using zero instead."]);
        self.error();
        if level != TOK_VAL {
            self.scanned_result(0, DIMEN_VAL);
        } else {
            self.scanned_result(0, INT_VAL);
        }
    }

    /// A read of `\time`, `\day`, `\month` or `\year`: DESIGN section 7.
    #[cold]
    fn scan_date_read(&mut self, m: i32) {
        self.checkpoint_date_read(m);
    }

    /// `\pdfelapsedtime` and `\pdfrandomseed`: a hard taint, DESIGN section 7.
    #[cold]
    fn scan_volatile_read(&mut self) {
        self.host.world.mark_taint_because(Taint::Hard, "\\pdfelapsedtime or \\pdfrandomseed was read");
    }

    /// §459
    pub fn scan_eight_bit_int(&mut self) {
        self.scan_int();
        if self.cur_val < 0 || self.cur_val > 255 {
            self.print_err_s(b"Bad register code");
            self.help(&[b"A register number must be between 0 and 255.", b"I changed this one to zero."]);
            self.int_error(self.cur_val);
            self.cur_val = 0;
        }
    }

    /// §460
    pub fn scan_char_num(&mut self) {
        self.scan_int();
        if self.cur_val < 0 || self.cur_val > 255 {
            self.print_err_s(b"Bad character code");
            self.help(&[b"A character number must be between 0 and 255.", b"I changed this one to zero."]);
            self.int_error(self.cur_val);
            self.cur_val = 0;
        }
    }

    /// §461
    pub fn scan_four_bit_int(&mut self) {
        self.scan_int();
        if self.cur_val < 0 || self.cur_val > 15 {
            self.print_err_s(b"Bad number");
            self.help(&[b"Since I expected to read a number between 0 and 15,", b"I changed this one to zero."]);
            self.int_error(self.cur_val);
            self.cur_val = 0;
        }
    }

    /// §462
    pub fn scan_fifteen_bit_int(&mut self) {
        self.scan_int();
        if self.cur_val < 0 || self.cur_val > 0o77777 {
            self.print_err_s(b"Bad mathchar");
            self.help(&[b"A mathchar number must be between 0 and 32767.", b"I changed this one to zero."]);
            self.int_error(self.cur_val);
            self.cur_val = 0;
        }
    }

    /// §463
    pub fn scan_twenty_seven_bit_int(&mut self) {
        self.scan_int();
        if self.cur_val < 0 || self.cur_val > 0o777777777 {
            self.print_err_s(b"Bad delimiter code");
            self.help(&[b"A numeric delimiter code must be between 0 and 2^{27}-1.", b"I changed this one to zero."]);
            self.int_error(self.cur_val);
            self.cur_val = 0;
        }
    }

    /// §467 Get the next non-blank non-sign token; returns negative.
    #[inline(always)]
    fn scan_sign(&mut self) -> bool {
        let mut negative = false;
        loop {
            // §432 Get the next non-blank non-call token
            loop {
                self.get_x_token();
                if self.cur_cmd != SPACER {
                    break;
                }
            }
            if self.cur_tok == OTHER_TOKEN + b'-' as i32 {
                negative = !negative;
                self.cur_tok = OTHER_TOKEN + b'+' as i32;
            }
            if self.cur_tok != OTHER_TOKEN + b'+' as i32 {
                break;
            }
        }
        negative
    }

    /// §469 Scan an optional space
    #[inline(always)]
    fn scan_optional_space(&mut self) {
        self.get_x_token();
        if self.cur_cmd != SPACER {
            self.back_input();
        }
    }

    /// §466
    pub fn scan_int(&mut self) {
        self.scan_int_general::<false>();
    }

    /// `scan_int` for a caller whose next step is `get_x_token`, with nothing before it that shows the input
    /// stack: §529's first operand, and §1785's factor through `scan_int_from_cur_tok`.  The token after a constant,
    /// which `scan_int` would read and back up, may stay unread: see `scan_int_next_may_stay`.
    #[inline]
    pub fn scan_int_keeping_next(&mut self) {
        self.scan_int_general::<true>();
    }

    /// `back_input` of the digit in `cur_tok`, just read, and `scan_int` or `scan_int_keeping_next`.  The level would
    /// be gone with the next digit or terminator, so this is for when that is the only thing it could do: when
    /// `scan_int_digit_may_skip_level` says that it would add nothing to the stack's high water mark.
    #[inline]
    pub fn scan_int_from_cur_tok<const KEEP: bool>(&mut self) {
        self.radix = 10;
        self.cur_val = 0;
        self.scan_int_accumulate::<KEEP>(214748364, true);
    }

    #[inline]
    pub fn scan_int_digit_may_skip_level(&self) -> bool {
        (self.cur_tok - ZERO_TOKEN) as u32 <= 9 && self.input_ptr <= self.max_in_stack
    }

    #[inline(always)]
    fn scan_int_general<const KEEP: bool>(&mut self) {
        self.radix = 0;
        let negative = self.scan_sign(); // §467
        'restart: loop {
            if self.cur_tok == ALPHA_TOKEN {
                // §468 Scan an alphabetic character code into cur_val
                self.get_token(); // suppress macro expansion
                if self.cur_tok < CS_TOKEN_FLAG {
                    self.cur_val = self.cur_chr;
                    if self.cur_cmd <= RIGHT_BRACE {
                        if self.cur_cmd == RIGHT_BRACE {
                            self.align_state += 1;
                        } else {
                            self.align_state -= 1;
                        }
                    }
                } else if self.cur_tok < CS_TOKEN_FLAG + SINGLE_BASE {
                    self.cur_val = self.cur_tok - CS_TOKEN_FLAG - ACTIVE_BASE;
                } else {
                    self.cur_val = self.cur_tok - CS_TOKEN_FLAG - SINGLE_BASE;
                }
                if self.cur_val > 255 {
                    self.print_err_s(b"Improper alphabetic constant");
                    self.help(&[
                        b"A one-character control sequence belongs after a ` mark.",
                        b"So I'm essentially inserting \\0 here.",
                    ]);
                    self.cur_val = b'0' as i32;
                    self.back_error();
                } else {
                    self.scan_optional_space(); // §469
                }
            } else if self.cur_tok == CS_TOKEN_FLAG + FROZEN_PRIMITIVE {
                self.scan_reset_primitive(); // §395
                continue 'restart;
            } else if self.cur_cmd >= MIN_INTERNAL && self.cur_cmd <= MAX_INTERNAL {
                self.scan_something_internal(INT_VAL, false);
            } else {
                // §470 Scan a numeric constant
                self.radix = 10;
                let mut m = 214748364;
                if self.cur_tok == OCTAL_TOKEN {
                    self.radix = 8;
                    m = 0o2000000000;
                    self.get_x_token();
                } else if self.cur_tok == HEX_TOKEN {
                    self.radix = 16;
                    m = 0o1000000000;
                    self.get_x_token();
                }
                self.cur_val = 0;
                self.scan_int_accumulate::<KEEP>(m, true);
            }
            break;
        }
        if negative {
            self.cur_val = self.cur_val.wrapping_neg();
        }
    }

    /// §471 and what follows it in §470, with `cur_tok` the next character.
    #[inline(always)]
    fn scan_int_accumulate<const KEEP: bool>(&mut self, m: i32, mut vacuous: bool) {
        let mut OK_so_far = true;
        // §471 Accumulate the constant until cur_tok is not a suitable digit
        loop {
            let d;
            if self.cur_tok < ZERO_TOKEN + self.radix && self.cur_tok >= ZERO_TOKEN && self.cur_tok <= ZERO_TOKEN + 9 {
                d = self.cur_tok - ZERO_TOKEN;
            } else if self.radix == 16 {
                if self.cur_tok <= A_TOKEN + 5 && self.cur_tok >= A_TOKEN {
                    d = self.cur_tok - A_TOKEN + 10;
                } else if self.cur_tok <= OTHER_A_TOKEN + 5 && self.cur_tok >= OTHER_A_TOKEN {
                    d = self.cur_tok - OTHER_A_TOKEN + 10;
                } else {
                    break;
                }
            } else {
                break;
            }
            vacuous = false;
            if self.cur_val >= m && (self.cur_val > m || d > 7 || self.radix != 10) {
                if OK_so_far {
                    self.print_err_s(b"Number too big");
                    self.help(&[
                        b"I can only go up to 2147483647='17777777777=\"7FFFFFFF,",
                        b"so I'm using that number instead of yours.",
                    ]);
                    self.error();
                    self.cur_val = INFINITY;
                    OK_so_far = false;
                }
            } else {
                self.cur_val = self.cur_val * self.radix + d;
            }
            if self.radix == 10 && self.state() == TOKEN_LIST {
                // More digits in the list, up to where this test could matter: all that get_x_token would
                // do is read them, and it is not called for cur_tok to see.
                let mut l = self.loc();
                let mut v = self.cur_val;
                loop {
                    let d = self.toks[l] - ZERO_TOKEN;
                    if d as u32 > 9 || v >= m {
                        break;
                    }
                    v = v * 10 + d;
                    l += 1;
                }
                self.cur_val = v;
                self.set_loc(l);
                if KEEP && self.scan_int_next_may_stay(l) {
                    return;
                }
            }
            self.get_x_token();
        }
        if vacuous {
            // §472 Express astonishment that no number was here
            self.print_err_s(b"Missing number, treated as zero");
            self.help(&[
                b"A number should have been here; I inserted `0'.",
                b"(If you can't figure out why I needed to see a number,",
                b"look up `weird error' in the index to The TeXbook.)",
            ]);
            self.back_error();
        } else if self.cur_cmd != SPACER {
            self.back_input();
        }
    }

    /// Whether the constant may end here with the token at `l` unread, for `scan_int_keeping_next`: reading it would
    /// expand nothing and change nothing but `cur_cmd` and its like, which the caller's `get_x_token` will then
    /// set, and `back_input` would give the level only its caller's reading makes finished.  A finished level does
    /// show, as `<recently read>`, if an error comes before the next read.  So not `\relax`, which §1784 swallows, and
    /// after which `Arithmetic overflow` or the error of the caller's caller can come.  Any other token
    /// goes through `back_input` again or leads to the next read.  With `input_ptr` beyond `max_in_stack` the
    /// level could overflow the stack, and it is left to do that.  A space is simply dropped, as §470 does.
    #[inline(always)]
    fn scan_int_next_may_stay(&mut self, l: i32) -> bool {
        let t = self.toks[l];
        if t < CS_TOKEN_FLAG {
            let c = t >> 8;
            if c == SPACER {
                self.set_loc(l + 1);
                return true;
            }
            // A digit here is one that §471's test has to see.  A point is what §474 looks for in cur_tok.
            (c == LETTER || c == OTHER_CHAR)
                && (t - ZERO_TOKEN) as u32 > 9
                && t != POINT_TOKEN
                && t != CONTINENTAL_POINT_TOKEN
                && self.input_ptr <= self.max_in_stack
        } else {
            // get_next's §364 is for commands from tab_mark to car_ret
            let c = self.eq_type(t - CS_TOKEN_FLAG);
            c <= MAX_COMMAND && c != RELAX && (c < TAB_MARK || c > CAR_RET) && self.input_ptr <= self.max_in_stack
        }
    }

    /// §477 Coerce glue to a dimension
    #[inline]
    fn scan_coerce_glue(&mut self) {
        if self.cur_val_level >= GLUE_VAL {
            let v = self.width(self.cur_val);
            self.delete_glue_ref(self.cur_val);
            self.cur_val = v;
        }
    }

    /// §474, for a constant in the current token list in `pt`, at once followed by nothing else of the number, with
    /// `cur_tok` the first token of it: its value goes in `cur_val`, the digits after the point give `f`, and
    /// the list goes on after the unit.  The literal way there reads each of these tokens through `get_x_token` and
    /// leaves six levels behind it, none of them still there when §469 reads the next token, which
    /// it does as it would have.  Only the high water mark of the stack could tell, and it is left to do that.
    #[inline]
    fn scan_dimen_in_pt(&mut self) -> Option<i32> {
        if self.state() != TOKEN_LIST || self.input_ptr >= self.max_in_stack {
            return None;
        }
        let mut t = self.cur_tok;
        let is_point = |t: i32| t == POINT_TOKEN || t == CONTINENTAL_POINT_TOKEN;
        if (t - ZERO_TOKEN) as u32 > 9 && !is_point(t) {
            return None;
        }
        let mut l = self.loc();
        let mut v = 0;
        while (t - ZERO_TOKEN) as u32 <= 9 {
            if v >= 214748364 {
                return None; // §471's test
            }
            v = v * 10 + (t - ZERO_TOKEN);
            t = self.toks[l];
            l += 1;
        }
        let mut k = 0;
        if is_point(t) {
            loop {
                t = self.toks[l];
                l += 1;
                let d = t - ZERO_TOKEN;
                if d as u32 > 9 {
                    break;
                }
                if k < 17 {
                    // digits for k>=17 cannot affect the result
                    self.dig[k] = d;
                    k += 1;
                }
            }
        }
        // scan_keyword looks at cur_chr of anything: letters and others are enough here.  It is told no by the p for
        // fil, em, ex and true, and px gets no further than the t.
        let unit_char = |t: i32, c: u8| (t >> 8 == LETTER || t >> 8 == OTHER_CHAR) && (t & 255 | 32) == c as i32;
        if !unit_char(t, b'p') || !unit_char(self.toks[l], b't') {
            return None;
        }
        self.set_loc(l + 1);
        self.cur_val = v;
        self.radix = 10;
        Some(self.round_decimals(k))
    }

    /// §474
    pub fn scan_dimen(&mut self, mu: bool, inf: bool, shortcut: bool) {
        // §476 Local variables for dimension calculations: num, denom, k, kk, p, q, v and
        // save_cur_val are declared where they are first set.
        let mut f = 0;
        let mut in_pt = false;
        self.arith_error = false;
        self.cur_order = NORMAL;
        let mut negative = false;
        'attach_sign: {
            if !shortcut {
                negative = self.scan_sign(); // §467
                if self.cur_cmd >= MIN_INTERNAL && self.cur_cmd <= MAX_INTERNAL {
                    // §475 Fetch an internal dimension and goto attach_sign, or fetch an internal integer
                    if mu {
                        self.scan_something_internal(MU_VAL, false);
                        if self.cur_val_level != INT_VAL {
                            self.scan_coerce_glue();
                            if self.cur_val_level != MU_VAL {
                                self.mu_error();
                            }
                            break 'attach_sign;
                        }
                    } else {
                        self.scan_something_internal(DIMEN_VAL, false);
                        if self.cur_val_level == DIMEN_VAL {
                            break 'attach_sign;
                        }
                    }
                } else if let Some(g) = if mu { None } else { self.scan_dimen_in_pt() } {
                    f = g;
                    in_pt = true;
                } else {
                    if self.scan_int_digit_may_skip_level() {
                        self.scan_int_from_cur_tok::<false>();
                    } else {
                        self.back_input();
                        if self.cur_tok == CONTINENTAL_POINT_TOKEN {
                            self.cur_tok = POINT_TOKEN;
                        }
                        if self.cur_tok != POINT_TOKEN {
                            self.scan_int();
                        } else {
                            self.radix = 10;
                            self.cur_val = 0;
                        }
                    }
                    if self.cur_tok == CONTINENTAL_POINT_TOKEN {
                        self.cur_tok = POINT_TOKEN;
                    }
                    if self.radix == 10 && self.cur_tok == POINT_TOKEN {
                        // §478 Scan decimal fraction
                        let mut k = 0;
                        let mut p = NULL;
                        self.get_token(); // point_token is being re-scanned
                        loop {
                            self.get_x_token();
                            if self.cur_tok > ZERO_TOKEN + 9 || self.cur_tok < ZERO_TOKEN {
                                break;
                            }
                            if k < 17 {
                                // digits for k>=17 cannot affect the result
                                let q = self.get_avail();
                                self.set_link(q, p);
                                self.set_info(q, self.cur_tok - ZERO_TOKEN);
                                p = q;
                                k += 1;
                            }
                        }
                        for kk in (1..=k).rev() {
                            self.dig[kk - 1] = self.info(p);
                            let q = p;
                            p = self.link(p);
                            self.free_avail(q);
                        }
                        f = self.round_decimals(k);
                        if self.cur_cmd != SPACER {
                            self.back_input();
                        }
                    }
                }
            }
            if self.cur_val < 0 {
                // in this case f=0
                negative = !negative;
                self.cur_val = self.cur_val.wrapping_neg();
            }
            // §479 Scan units and set cur_val to x*(cur_val+f/2^16), where there are x sp per unit
            'done: {
                'attach_fraction: {
                    if in_pt {
                        break 'attach_fraction;
                    }
                    if inf {
                        // §480 Scan for fil units; goto attach_fraction if found
                        if self.scan_keyword(b"fil") {
                            self.cur_order = FIL;
                            while self.scan_keyword(b"l") {
                                if self.cur_order == FILLL {
                                    self.print_err_s(b"Illegal unit of measure (");
                                    self.print_s(b"replaced by filll)");
                                    self.help(&[b"I dddon't go any higher than filll."]);
                                    self.error();
                                } else {
                                    self.cur_order += 1;
                                }
                            }
                            break 'attach_fraction;
                        }
                    }
                    // §481 Scan for units that are internal dimensions; goto attach_sign with cur_val set
                    let save_cur_val = self.cur_val;
                    // §432 Get the next non-blank non-call token
                    loop {
                        self.get_x_token();
                        if self.cur_cmd != SPACER {
                            break;
                        }
                    }
                    'not_found: {
                        let v;
                        if self.cur_cmd < MIN_INTERNAL || self.cur_cmd > MAX_INTERNAL {
                            self.back_input();
                            if mu {
                                break 'not_found;
                            }
                            if self.scan_keyword(b"em") {
                                v = self.quad(self.cur_font()); // §584 The em width for cur_font
                            } else if self.scan_keyword(b"ex") {
                                v = self.x_height(self.cur_font()); // §585 The x-height for cur_font
                            } else if self.scan_keyword(b"px") {
                                v = self.pdf_px_dimen();
                            } else {
                                break 'not_found;
                            }
                            self.scan_optional_space(); // §469
                        } else {
                            if mu {
                                self.scan_something_internal(MU_VAL, false);
                                self.scan_coerce_glue();
                                if self.cur_val_level != MU_VAL {
                                    self.mu_error();
                                }
                            } else {
                                self.scan_something_internal(DIMEN_VAL, false);
                            }
                            v = self.cur_val;
                        }
                        // found:
                        let y = self.xn_over_d(v, f, 0o200000);
                        self.cur_val = self.nx_plus_y(save_cur_val, v, y);
                        break 'attach_sign;
                    }
                    // not_found:
                    if mu {
                        // §482 Scan for mu units and goto attach_fraction
                        if self.scan_keyword(b"mu") {
                            break 'attach_fraction;
                        }
                        self.print_err_s(b"Illegal unit of measure (");
                        self.print_s(b"mu inserted)");
                        self.help(&[
                            b"The unit of measurement in math glue must be mu.",
                            b"To recover gracefully from this error, it's best to",
                            b"delete the erroneous units; e.g., type `2' to delete",
                            b"two letters. (See Chapter 27 of The TeXbook.)",
                        ]);
                        self.error();
                        break 'attach_fraction;
                    }
                    if self.scan_keyword(b"true") {
                        // §483 Adjust f for the magnification ratio
                        self.prepare_mag();
                        if self.mag() != 1000 {
                            self.cur_val = self.xn_over_d(self.cur_val, 1000, self.mag());
                            f = ((1000 * f as i64 + 65536 * self.remainder as i64) / self.mag() as i64) as i32;
                            self.cur_val = self.cur_val.wrapping_add(f / 0o200000);
                            f %= 0o200000;
                        }
                    }
                    if self.scan_keyword(b"pt") {
                        break 'attach_fraction; // the easy case
                    }
                    // §484 Scan for all other units and adjust cur_val and f accordingly; goto done
                    // in the case of scaled points
                    'done2: {
                        let (num, denom);
                        if self.scan_keyword(b"in") {
                            (num, denom) = (7227, 100);
                        } else if self.scan_keyword(b"pc") {
                            (num, denom) = (12, 1);
                        } else if self.scan_keyword(b"cm") {
                            (num, denom) = (7227, 254);
                        } else if self.scan_keyword(b"mm") {
                            (num, denom) = (7227, 2540);
                        } else if self.scan_keyword(b"bp") {
                            (num, denom) = (7227, 7200);
                        } else if self.scan_keyword(b"dd") {
                            (num, denom) = (1238, 1157);
                        } else if self.scan_keyword(b"cc") {
                            (num, denom) = (14856, 1157);
                        } else if self.scan_keyword(b"nd") {
                            (num, denom) = (685, 642);
                        } else if self.scan_keyword(b"nc") {
                            (num, denom) = (1370, 107);
                        } else if self.scan_keyword(b"sp") {
                            break 'done;
                        } else {
                            // §485 Complain about unknown unit and goto done2
                            self.print_err_s(b"Illegal unit of measure (");
                            self.print_s(b"pt inserted)");
                            self.help(&[
                                b"Dimensions can be in units of em, ex, in, pt, pc,",
                                b"cm, mm, dd, cc, nd, nc, bp, or sp; but yours is a new one!",
                                b"I'll assume that you meant to say pt, for printer's points.",
                                b"To recover gracefully from this error, it's best to",
                                b"delete the erroneous units; e.g., type `2' to delete",
                                b"two letters. (See Chapter 27 of The TeXbook.)",
                            ]);
                            self.error();
                            break 'done2;
                        }
                        self.cur_val = self.xn_over_d(self.cur_val, num, denom);
                        f = (num * f + 0o200000 * self.remainder) / denom;
                        self.cur_val = self.cur_val.wrapping_add(f / 0o200000);
                        f %= 0o200000;
                    }
                }
                // attach_fraction:
                if self.cur_val >= 0o40000 {
                    self.arith_error = true;
                } else {
                    self.cur_val = self.cur_val * UNITY + f;
                }
            }
            // done:
            self.scan_optional_space(); // §469
        }
        // attach_sign:
        if self.arith_error || abs(self.cur_val) >= 0o10000000000 {
            // §486 Report that this dimension is out of range
            self.print_err_s(b"Dimension too large");
            self.help(&[
                b"I can't work with sizes bigger than about 19 feet.",
                b"Continue and I'll use the largest value I can.",
            ]);
            self.error();
            self.cur_val = MAX_DIMEN;
            self.arith_error = false;
        }
        if negative {
            self.cur_val = self.cur_val.wrapping_neg();
        }
    }

    /// §487
    pub fn scan_glue(&mut self, level: i32) {
        let mu = level == MU_VAL;
        let negative = self.scan_sign(); // §467
        if self.cur_cmd >= MIN_INTERNAL && self.cur_cmd <= MAX_INTERNAL {
            self.scan_something_internal(level, negative);
            if self.cur_val_level >= GLUE_VAL {
                if self.cur_val_level != level {
                    self.mu_error();
                }
                return;
            }
            if self.cur_val_level == INT_VAL {
                self.scan_dimen(mu, false, true);
            } else if level == MU_VAL {
                self.mu_error();
            }
        } else {
            self.back_input();
            self.scan_dimen(mu, false, false);
            if negative {
                self.cur_val = self.cur_val.wrapping_neg();
            }
        }
        // §488 Create a new glue specification whose width is cur_val; scan for its stretch and
        // shrink components
        let q = self.new_spec(ZERO_GLUE);
        self.set_width(q, self.cur_val);
        if self.scan_keyword(b"plus") {
            self.scan_dimen(mu, true, false);
            self.set_stretch(q, self.cur_val);
            self.set_stretch_order(q, self.cur_order);
        }
        if self.scan_keyword(b"minus") {
            self.scan_dimen(mu, true, false);
            self.set_shrink(q, self.cur_val);
            self.set_shrink_order(q, self.cur_order);
        }
        self.cur_val = q;
    }

    /// §489
    pub fn scan_rule_spec(&mut self) -> i32 {
        let q = self.new_rule(); // width, depth, and height all equal null_flag now
        if self.cur_cmd == VRULE {
            self.set_width(q, DEFAULT_RULE);
        } else {
            self.set_height(q, DEFAULT_RULE);
            self.set_depth(q, 0);
        }
        loop {
            // reswitch:
            if self.scan_keyword(b"width") {
                self.scan_normal_dimen();
                self.set_width(q, self.cur_val);
                continue;
            }
            if self.scan_keyword(b"height") {
                self.scan_normal_dimen();
                self.set_height(q, self.cur_val);
                continue;
            }
            if self.scan_keyword(b"depth") {
                self.scan_normal_dimen();
                self.set_depth(q, self.cur_val);
                continue;
            }
            break;
        }
        q
    }
}

/// What the tests of scan.rs, cond.rs and buildtoks.rs share: the state that the preamble of
/// tests/oracle/gen_scan_tables.py gives the oracle, on a machine with every primitive of the WEB.
#[cfg(test)]
pub(crate) mod testkit {
    use super::*;
    use crate::save::testkit::*;
    use crate::toks::testkit::*;

    /// The line of the case on the oracle: where `\inputlineno` is read.
    const CASE_LINE: i32 = 16;

    /// `\def`, `\long\def` or `\protected\def` by the real scan_toks.
    fn defmac(t: &mut Tex, name: &str, text: &[u8], cmd: i32, protected: bool) {
        let p = cs_named(t, name);
        t.cur_cs = p;
        set_line(t, text, NEW_LINE);
        let mut d = t.scan_toks(true, false);
        if protected {
            d = t.tok_prepend(d, PROTECTED_TOKEN);
        }
        t.set_eq_type(p, cmd);
        t.set_equiv(p, d);
    }

    fn set_glue(t: &mut Tex, code: i32, w: i32, st: i32, so: i32, sh: i32, sho: i32) {
        let q = t.new_spec(ZERO_GLUE);
        t.set_width(q, w);
        t.set_stretch(q, st);
        t.set_stretch_order(q, so);
        t.set_shrink(q, sh);
        t.set_shrink_order(q, sho);
        t.set_glue_par(code, q);
    }

    pub fn scan_machine(mag: i32) -> &'static mut Tex {
        let t = table_machine();
        print_setup(t);
        t.no_new_control_sequence = false;
        t.prim_used = PRIM_SIZE;
        for line in include_str!("../testdata/primitives.tsv").lines() {
            let f: Vec<&str> = line.split('\t').collect();
            t.primitive(f[0].as_bytes(), f[1].parse().unwrap(), f[2].parse().unwrap());
        }
        // Those the table leaves out, of its commands whose names another routine prints.
        for (name, cmd, chr) in [
            ("tolerance", ASSIGN_INT, INT_BASE + TOLERANCE_CODE),
            ("hbadness", ASSIGN_INT, INT_BASE + HBADNESS_CODE),
            ("hsize", ASSIGN_DIMEN, DIMEN_BASE + HSIZE_CODE),
            ("vsize", ASSIGN_DIMEN, DIMEN_BASE + VSIZE_CODE),
            ("baselineskip", ASSIGN_GLUE, GLUE_BASE + BASELINE_SKIP_CODE),
            ("parskip", ASSIGN_GLUE, GLUE_BASE + PAR_SKIP_CODE),
            ("thinmuskip", ASSIGN_MU_GLUE, GLUE_BASE + THIN_MU_SKIP_CODE),
            ("textfont", DEF_FAMILY, MATH_FONT_BASE),
            ("nullfont", SET_FONT, NULL_FONT),
        ] {
            t.primitive(name.as_bytes(), cmd, chr);
        }
        let relax = cs_named(t, "relax");
        t.set_text(FROZEN_RELAX, t.text(relax));
        t.eqtb[FROZEN_RELAX] = t.eqtb[relax];
        let fi = cs_named(t, "fi");
        t.set_text(FROZEN_FI, t.text(fi));
        t.eqtb[FROZEN_FI] = t.eqtb[fi];
        t.set_eq_type(FROZEN_DONT_EXPAND, DONT_EXPAND);
        t.set_text(FROZEN_DONT_EXPAND, S_NOTEXPANDED);
        t.set_eq_type(FROZEN_PRIMITIVE, IGNORE_SPACES);
        t.set_equiv(FROZEN_PRIMITIVE, 1);
        t.set_eq_level(FROZEN_PRIMITIVE, LEVEL_ONE);
        t.set_text(FROZEN_PRIMITIVE, S_PDFPRIMITIVE);
        t.par_loc = cs_named(t, "par");
        t.par_token = CS_TOKEN_FLAG + t.par_loc;
        t.last_glue = MAX_HALFWORD;
        t.last_node_type = -1;
        t.shellenabledp = 1;
        t.restrictedshell = 1;
        t.eTeX_mode = 1;
        t.set_pdf_px_dimen(65782);
        // The preamble's definitions
        defmac(t, "grab", b"{}", CALL, false);
        defmac(t, "m", b"{12}", CALL, false);
        defmac(t, "mm", b"{1}", CALL, false);
        defmac(t, "n", b"{pt}", CALL, false);
        defmac(t, "e", b"{}", CALL, false);
        defmac(t, "s", b"{ }", CALL, false);
        defmac(t, "space", b"{ }", CALL, false);
        defmac(t, "pl", b"{plus}", CALL, false);
        defmac(t, "a", b"{x}", CALL, false);
        defmac(t, "b", b"{x}", CALL, false);
        defmac(t, "cc", b"{y}", CALL, false);
        defmac(t, "la", b"{x}", LONG_CALL, false);
        defmac(t, "aa", b"#1{#1}", CALL, false);
        defmac(t, "ab", b"#1{#1}", CALL, false);
        defmac(t, "ac", b"#1{}", CALL, false);
        defmac(t, "pp", b"{x}", CALL, true);
        defmac(t, "q", b"{\\pp}", CALL, false);
        let (a, z) = (cs_named(t, "a"), cs_named(t, "z"));
        t.eqtb[z] = t.eqtb[a];
        t.add_token_ref(t.equiv(a));
        for (name, cmd, chr) in [
            ("bg", LEFT_BRACE, b'{' as i32),
            ("eg", RIGHT_BRACE, b'}' as i32),
            ("xx", LETTER, b'x' as i32),
            ("chA", CHAR_GIVEN, 65),
            ("mcB", MATH_GIVEN, 1234),
            ("tt", ASSIGN_TOKS, TOKS_BASE),
        ] {
            let p = cs_named(t, name);
            t.set_eq_type(p, cmd);
            t.set_equiv(p, chr);
            t.set_eq_level(p, LEVEL_ONE);
        }
        t.set_tolerance(321);
        t.set_hbadness(-5);
        t.set_hsize(100 * UNITY);
        t.set_vsize(-3 * UNITY);
        set_glue(t, BASELINE_SKIP_CODE, 12 * UNITY, UNITY, NORMAL, 2 * UNITY, NORMAL);
        set_glue(t, PAR_SKIP_CODE, 0, UNITY, FIL, 2 * UNITY, FILL);
        set_glue(t, THIN_MU_SKIP_CODE, 3 * UNITY, UNITY, NORMAL, 2 * UNITY, FILL);
        t.set_mag(mag);
        let list = {
            set_line(t, b"{ab}", NEW_LINE);
            t.cur_cs = 0;
            t.scan_toks(false, false)
        };
        t.set_equiv(EVERY_PAR_LOC, list);
        t.no_new_control_sequence = true;
        t.line = CASE_LINE;
        t.cur_cs = 0;
        t
    }

    /// %XX in a field of the tables stands for a byte.
    pub fn decode(f: &str) -> Vec<u8> {
        let b = f.as_bytes();
        let mut out = Vec::new();
        let mut i = 0;
        while i < b.len() {
            if b[i] == b'%' {
                out.push(u8::from_str_radix(&f[i + 1..i + 3], 16).unwrap());
                i += 3;
            } else {
                out.push(b[i]);
                i += 1;
            }
        }
        out
    }

    /// The lines of what was printed that report an error.
    pub fn error_lines(s: &str) -> String {
        s.lines().filter(|l| l.starts_with("! ")).collect::<Vec<_>>().join("\n")
    }

    /// What the real show_token_list prints for these tokens, in the printing of `\detokenize`,
    /// which does not make characters printable.
    pub fn show_bytes(t: &mut Tex, toks: &[i32]) -> String {
        let m = t.tb_mark();
        for &x in toks {
            t.tb_push(x);
        }
        let p = t.tb_commit(m);
        let (old_setting, b) = (t.selector, t.pool_ptr);
        t.selector = NEW_STRING;
        t.show_token_list(p + 1, NULL, 10000000);
        t.selector = old_setting;
        let s: String = t.str_pool.as_slice()[b as usize..t.pool_ptr as usize].iter().map(|&c| c as char).collect();
        t.pool_ptr = b;
        t.flush_list(p);
        s
    }

    /// Checks every row of testdata/scan_oracle.tsv whose kind is listed.  Returns how many.
    pub fn check_table(kinds: &[&str]) -> usize {
        let (mut n, mut bad) = (0, 0);
        let mut report = String::new();
        for line in include_str!("../testdata/scan_oracle.tsv").lines() {
            let f: Vec<&str> = line.split('\t').collect();
            let kind = f[0];
            if !kinds.contains(&kind) {
                continue;
            }
            n += 1;
            let mag: i32 = f[1].parse().unwrap();
            let text = decode(f[2]);
            let (want_value, want_rest, want_err) = (decode(f[3]), decode(f[4]), decode(f[5]));
            let t = scan_machine(mag);
            let stop = OTHER_TOKEN + b'|' as i32;
            let r = cs_named(t, "r");
            let mut line_text = Vec::new();
            if kind == "edef" {
                line_text.push(b'{');
            }
            line_text.extend_from_slice(&text);
            if kind == "edef" {
                line_text.push(b'}');
            }
            line_text.push(b'|');
            set_line(t, &line_text, NEW_LINE);
            // What `\the\ignorespaces` finds: the last number scanned, the 16 of the oracle's \write16.
            t.scanned_result(16, INT_VAL);
            let value: String;
            let mut errs;
            match kind {
                "int" => {
                    t.scan_int();
                    value = t.cur_val.to_string();
                    errs = printed(t);
                }
                "dimen" => {
                    t.scan_normal_dimen();
                    value = t.cur_val.to_string();
                    errs = printed(t);
                }
                "glue" | "muglue" => {
                    let mu = kind == "muglue";
                    t.scan_glue(if mu { MU_VAL } else { GLUE_VAL });
                    errs = printed(t);
                    t.print_spec(t.cur_val, if mu { b"mu" } else { b"pt" });
                    value = printed(t);
                }
                "edef" => {
                    t.cur_cs = r;
                    let d = t.scan_toks(true, true);
                    errs = printed(t);
                    let l = list(t, d);
                    value = show_bytes(t, &l[1..]);
                }
                _ => {
                    t.cur_cs = r;
                    let d = t.scan_toks(true, kind == "xdef");
                    errs = printed(t);
                    t.token_show(d);
                    value = printed(t);
                }
            }
            let rest = drain(t, stop);
            errs.push_str(&printed(t));
            // The oracle's value and rest went through \write, into the log: the printable forms.
            let in_log = |s: String| -> String {
                let mut o = String::new();
                for c in s.chars() {
                    let k = c as u32;
                    if k >= 128 {
                        o.push_str(&format!("^^{:02x}", k));
                    } else if k < 32 || k == 127 {
                        o.push_str("^^");
                        o.push(if k < 64 { (k + 64) as u8 } else { (k - 64) as u8 } as char);
                    } else {
                        o.push(c);
                    }
                }
                o
            };
            let value = in_log(value);
            let rest = in_log(show_bytes(t, &rest));
            let got_err = error_lines(&errs);
            let lat = |b: &[u8]| b.iter().map(|&c| c as char).collect::<String>();
            if value != lat(&want_value) || rest != lat(&want_rest) || got_err != lat(&want_err) {
                bad += 1;
                if bad <= 40 {
                    report.push_str(&format!(
                        "{} {:?} (mag {})\n  oracle {:?} rest {:?} errors {:?}\n  port   {:?} rest {:?} errors {:?}\n",
                        kind,
                        lat(&text),
                        mag,
                        lat(&want_value),
                        lat(&want_rest),
                        lat(&want_err),
                        value,
                        rest,
                        got_err
                    ));
                }
            }
            Tex::dealloc(t);
        }
        assert_eq!(bad, 0, "{} of {} rows of kinds {:?} differ\n{}", bad, n, kinds, report);
        n
    }
}

#[cfg(test)]
mod tests {
    use super::testkit::*;

    #[test]
    fn scan_int_against_the_oracle() {
        assert!(check_table(&["int"]) > 200);
    }

    #[test]
    fn scan_dimen_against_the_oracle() {
        assert!(check_table(&["dimen"]) > 300);
    }

    #[test]
    fn scan_glue_against_the_oracle() {
        assert!(check_table(&["glue", "muglue"]) > 200);
    }

    #[test]
    fn scan_toks_against_the_oracle() {
        assert!(check_table(&["def", "xdef"]) > 100);
    }
}
