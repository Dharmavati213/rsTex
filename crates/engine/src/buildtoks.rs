//! §490-512 Part 27: building token lists.
//!
//! A procedure includes every module reachable from it, wherever defined: `tools/web.py tangle NAME`.
//! Token lists are built on the `tb` stack and not in `mem`: docs/PORTING.md, "Token lists".
//!
//! Also here: the C side of the convert primitives, from utils.c and texmfmp.c, and `mod regex`,
//! which is glibc's regcomp and regexec in the C locale.
#![forbid(unsafe_code)]
#![deny(unreachable_patterns, unused_variables)]
#![allow(non_snake_case, unused_mut, unused_imports, dead_code)]

use crate::access::*;
use crate::consts::*;
use crate::host::*;
use crate::raw::*;
use crate::state::*;

crate::pod_struct! {
    /// `regmatch_t`.
    pub struct MatchSpan {
        pub so: i32,
        pub eo: i32,
    }
}

crate::pod_struct! {
    /// POD state private to this module.  Add fields here, not to state.rs.
    pub struct Vars {
        /// utils.c `sub_match_count`.  Its initial 10 is never seen: `match_string_set` guards it.
        pub sub_match_count: i32,
        /// utils.c `pmatch`.  Only the entries regexec really fills; the rest would read -1.
        pub pmatch: PodVec<MatchSpan>,
        /// utils.c `match_string`, with `match_string != NULL` as `match_string_set`.
        pub match_string: PodVec<u8>,
        pub match_string_set: bool,
        pub last_match_succeeded: bool,
    }
}

/// Non-POD state private to this module, in `self.host`.  Rarely needed.
#[derive(Default)]
pub struct HVars {}

/// utils.c `DEFAULT_SUB_MATCH_COUNT`.
const DEFAULT_SUB_MATCH_COUNT: i32 = 10;
const HEX_UPPER: &[u8; 16] = b"0123456789ABCDEF";

impl Tex {
    /// §490
    /// Appends to the current token builder instead of returning a tail; see PORTING.md.
    pub fn str_toks(&mut self, b: i32) {
        self.str_room(1);
        let mut k = b;
        while k < self.pool_ptr {
            let t = self.str_pool.at(k);
            let t = if t == b' ' as i32 { SPACE_TOKEN } else { OTHER_TOKEN + t };
            self.tb_push(t);
            k += 1;
        }
        self.pool_ptr = b;
    }

    /// §491
    /// Appends to the current token builder instead of returning a tail; see PORTING.md.
    pub fn the_toks(&mut self) {
        // §1688 Handle \unexpanded or \detokenize and return
        if odd(self.cur_chr) {
            let c = self.cur_chr;
            if c == 1 {
                self.scan_general_text(); // the tokens stay on the caller's builder
                return;
            }
            let m = self.tb_mark();
            self.scan_general_text();
            let p = self.tb_commit(m);
            let old_setting = self.selector;
            self.selector = NEW_STRING;
            let b = self.pool_ptr;
            self.token_show(p);
            self.flush_list(p);
            self.selector = old_setting;
            self.str_toks(b);
            return;
        }
        self.get_x_token();
        self.scan_something_internal(TOK_VAL, false);
        if self.cur_val_level >= IDENT_VAL {
            // §492 Copy the token list
            if self.cur_val_level == IDENT_VAL {
                self.tb_push(CS_TOKEN_FLAG + self.cur_val);
            } else if self.cur_val != NULL {
                self.tb_push_list(self.cur_val);
            }
        } else {
            let old_setting = self.selector;
            self.selector = NEW_STRING;
            let b = self.pool_ptr;
            match self.cur_val_level {
                INT_VAL => self.print_int(self.cur_val),
                DIMEN_VAL => {
                    self.print_scaled(self.cur_val);
                    self.print_s(b"pt");
                }
                GLUE_VAL => {
                    self.print_spec(self.cur_val, b"pt");
                    self.delete_glue_ref(self.cur_val);
                }
                MU_VAL => {
                    self.print_spec(self.cur_val, b"mu");
                    self.delete_glue_ref(self.cur_val);
                }
                _ => {} // there are no other cases
            }
            self.selector = old_setting;
            self.str_toks(b);
        }
    }

    /// §493
    pub fn ins_the_toks(&mut self) {
        let m = self.tb_mark();
        self.the_toks();
        let p = self.tb_commit(m);
        self.ins_list(p);
    }

    /// `link(garbage):=str_toks(b); ins_list(link(temp_head))`, with `flush_str(..)` between them.
    #[inline]
    fn buildtoks_str_toks_list(&mut self, b: i32) -> i32 {
        let m = self.tb_mark();
        self.str_toks(b);
        self.tb_commit(m)
    }

    /// `save_cur_string`, §496: returns `u`.
    #[inline]
    fn buildtoks_save_cur_string(&mut self) -> i32 {
        if self.str_start[self.str_ptr] < self.pool_ptr {
            self.make_string()
        } else {
            0
        }
    }

    /// `restore_cur_string`, §496.
    #[inline]
    fn buildtoks_restore_cur_string(&mut self, u: i32) {
        if u != 0 {
            self.str_ptr -= 1;
        }
    }

    /// §497: what `pdf_escape_string_code` and its five siblings share up to and including
    /// `scanner_status:=save_scanner_status`, and what `scan_pdf_ext_toks` leaves in a string.
    fn buildtoks_scan_ext_string(&mut self, save_scanner_status: i32, save_warning_index: i32, save_def_ref: i32) -> i32 {
        self.scan_pdf_ext_toks();
        let s = self.tokens_to_string(self.def_ref);
        self.delete_token_ref(self.def_ref);
        self.def_ref = save_def_ref;
        self.warning_index = save_warning_index;
        self.scanner_status = save_scanner_status;
        s
    }

    /// §497: the end of those same cases.
    fn buildtoks_finish_ext_string(&mut self, s: i32, b: i32, u: i32) {
        let p = self.buildtoks_str_toks_list(b);
        self.flush_str(s);
        self.ins_list(p);
        self.buildtoks_restore_cur_string(u);
    }

    /// §496
    pub fn conv_toks(&mut self) {
        let c = self.cur_chr;
        let mut p = NULL;
        let mut i = 0;
        let mut j = 0;
        // §497 Scan the argument for command c
        match c {
            NUMBER_CODE | ROMAN_NUMERAL_CODE => self.scan_int(),
            STRING_CODE | MEANING_CODE => {
                let save_scanner_status = self.scanner_status;
                self.scanner_status = NORMAL;
                self.get_token();
                self.scanner_status = save_scanner_status;
            }
            FONT_NAME_CODE => self.scan_font_ident(),
            ETEX_REVISION_CODE => {}
            EXPANDED_CODE => {
                let save_scanner_status = self.scanner_status;
                let save_warning_index = self.warning_index;
                let save_def_ref = self.def_ref;
                let u = self.buildtoks_save_cur_string();
                self.scan_pdf_ext_toks();
                self.warning_index = save_warning_index;
                self.scanner_status = save_scanner_status;
                self.ins_list(self.def_ref); // the block carries its own header; end_token_list frees it
                self.def_ref = save_def_ref;
                self.buildtoks_restore_cur_string(u);
                return;
            }
            PDFTEX_REVISION_CODE => {}
            PDFTEX_BANNER_CODE => {}
            PDF_FONT_NAME_CODE | PDF_FONT_OBJNUM_CODE | PDF_FONT_SIZE_CODE => {
                self.scan_font_ident();
                if self.cur_val == NULL_FONT {
                    self.pdf_error(b"font", b"invalid font identifier");
                }
                if c != PDF_FONT_SIZE_CODE {
                    self.pdf_check_vf_cur_val();
                    if !self.font_used[self.cur_val] {
                        self.pdf_init_font_cur_val();
                    }
                }
            }
            PDF_PAGE_REF_CODE => {
                self.scan_int();
                if self.cur_val <= 0 {
                    self.pdf_error(b"pageref", b"invalid page number");
                }
            }
            LEFT_MARGIN_KERN_CODE | RIGHT_MARGIN_KERN_CODE => {
                self.scan_register_num();
                p = self.fetch_box();
                if p == NULL || self.ty(p) != HLIST_NODE {
                    self.pdf_error(b"marginkern", b"a non-empty hbox expected");
                }
            }
            PDF_XFORM_NAME_CODE => {
                self.scan_int();
                self.pdf_check_obj(OBJ_TYPE_XFORM, self.cur_val);
            }
            PDF_ESCAPE_STRING_CODE | PDF_ESCAPE_NAME_CODE | PDF_ESCAPE_HEX_CODE | PDF_UNESCAPE_HEX_CODE
            | PDF_FILE_MOD_DATE_CODE | PDF_FILE_SIZE_CODE => {
                let save_scanner_status = self.scanner_status;
                let save_warning_index = self.warning_index;
                let save_def_ref = self.def_ref;
                let u = self.buildtoks_save_cur_string();
                let s = self.buildtoks_scan_ext_string(save_scanner_status, save_warning_index, save_def_ref);
                let b = self.pool_ptr;
                match c {
                    PDF_ESCAPE_STRING_CODE => self.escapestring(self.str_start[s]),
                    PDF_ESCAPE_NAME_CODE => self.escapename(self.str_start[s]),
                    PDF_ESCAPE_HEX_CODE => self.escapehex(self.str_start[s]),
                    PDF_UNESCAPE_HEX_CODE => self.unescapehex(self.str_start[s]),
                    PDF_FILE_MOD_DATE_CODE => self.getfilemoddate(s),
                    _ => self.getfilesize(s),
                }
                self.buildtoks_finish_ext_string(s, b, u);
                return;
            }
            PDF_CREATION_DATE_CODE => {
                let b = self.pool_ptr;
                self.getcreationdate();
                let p = self.buildtoks_str_toks_list(b);
                self.ins_list(p);
                return;
            }
            PDF_MDFIVE_SUM_CODE => {
                let save_scanner_status = self.scanner_status;
                let save_warning_index = self.warning_index;
                let save_def_ref = self.def_ref;
                let u = self.buildtoks_save_cur_string();
                let booltemp = self.scan_keyword(b"file");
                let s = self.buildtoks_scan_ext_string(save_scanner_status, save_warning_index, save_def_ref);
                let b = self.pool_ptr;
                self.getmd5sum(s, booltemp);
                self.buildtoks_finish_ext_string(s, b, u);
                return;
            }
            PDF_FILE_DUMP_CODE => {
                let save_scanner_status = self.scanner_status;
                let save_warning_index = self.warning_index;
                let save_def_ref = self.def_ref;
                let u = self.buildtoks_save_cur_string();
                // scan offset
                self.cur_val = 0;
                if self.scan_keyword(b"offset") {
                    self.scan_int();
                    if self.cur_val < 0 {
                        self.print_err_s(b"Bad file offset");
                        self.help(&[b"A file offset must be between 0 and 2^{31}-1,", b"I changed this one to zero."]);
                        self.int_error(self.cur_val);
                        self.cur_val = 0;
                    }
                }
                i = self.cur_val;
                // scan length
                self.cur_val = 0;
                if self.scan_keyword(b"length") {
                    self.scan_int();
                    if self.cur_val < 0 {
                        self.print_err_s(b"Bad dump length");
                        self.help(&[b"A dump length must be between 0 and 2^{31}-1,", b"I changed this one to zero."]);
                        self.int_error(self.cur_val);
                        self.cur_val = 0;
                    }
                }
                j = self.cur_val;
                // scan file name
                let s = self.buildtoks_scan_ext_string(save_scanner_status, save_warning_index, save_def_ref);
                let b = self.pool_ptr;
                self.getfiledump(s, i, j);
                self.buildtoks_finish_ext_string(s, b, u);
                return;
            }
            PDF_MATCH_CODE => {
                let save_scanner_status = self.scanner_status;
                let save_warning_index = self.warning_index;
                let save_def_ref = self.def_ref;
                let u = self.buildtoks_save_cur_string();
                // scan for icase
                let booltemp = self.scan_keyword(b"icase");
                // scan for subcount
                i = -1; // default for subcount
                if self.scan_keyword(b"subcount") {
                    self.scan_int();
                    i = self.cur_val;
                }
                self.scan_pdf_ext_toks();
                let s = self.tokens_to_string(self.def_ref);
                self.delete_token_ref(self.def_ref);
                self.scan_pdf_ext_toks();
                let t = self.tokens_to_string(self.def_ref);
                self.delete_token_ref(self.def_ref);
                self.def_ref = save_def_ref;
                self.warning_index = save_warning_index;
                self.scanner_status = save_scanner_status;
                let b = self.pool_ptr;
                self.matchstrings(s, t, i, booltemp);
                let p = self.buildtoks_str_toks_list(b);
                self.flush_str(t);
                self.flush_str(s);
                self.ins_list(p);
                self.buildtoks_restore_cur_string(u);
                return;
            }
            PDF_LAST_MATCH_CODE => {
                self.scan_int();
                if self.cur_val < 0 {
                    self.print_err_s(b"Bad match number");
                    self.help(&[b"Since I expected zero or a positive number,", b"I changed this one to zero."]);
                    self.int_error(self.cur_val);
                    self.cur_val = 0;
                }
                let b = self.pool_ptr;
                self.getmatch(self.cur_val);
                let p = self.buildtoks_str_toks_list(b);
                self.ins_list(p);
                return;
            }
            PDF_STRCMP_CODE => {
                let save_scanner_status = self.scanner_status;
                let save_warning_index = self.warning_index;
                let save_def_ref = self.def_ref;
                let u = self.buildtoks_save_cur_string();
                self.compare_strings();
                self.def_ref = save_def_ref;
                self.warning_index = save_warning_index;
                self.scanner_status = save_scanner_status;
                self.buildtoks_restore_cur_string(u);
            }
            PDF_COLORSTACK_INIT_CODE => {
                let booltemp = self.scan_keyword(b"page");
                if self.scan_keyword(b"direct") {
                    self.cur_val = DIRECT_ALWAYS;
                } else if self.scan_keyword(b"page") {
                    self.cur_val = DIRECT_PAGE;
                } else {
                    self.cur_val = SET_ORIGIN;
                }
                let save_scanner_status = self.scanner_status;
                let save_warning_index = self.warning_index;
                let save_def_ref = self.def_ref;
                let u = self.buildtoks_save_cur_string();
                // scan_general_text in this text leaves cur_val=temp_head: PORTING section 15
                let s = self.buildtoks_scan_ext_string(save_scanner_status, save_warning_index, save_def_ref);
                self.cur_val = self.newcolorstack(s, self.cur_val, booltemp);
                self.flush_str(s);
                self.cur_val_level = INT_VAL;
                if self.cur_val < 0 {
                    self.print_err_s(b"Too many color stacks");
                    self.help(&[
                        b"The number of color stacks is limited to 32768.",
                        b"I'll use the default color stack 0 here.",
                    ]);
                    self.error();
                    self.cur_val = 0;
                    self.buildtoks_restore_cur_string(u);
                }
            }
            JOB_NAME_CODE => {
                if self.job_name == 0 {
                    self.open_log_file();
                }
            }
            UNIFORM_DEVIATE_CODE => self.scan_int(),
            NORMAL_DEVIATE_CODE => {}
            PDF_INSERT_HT_CODE => self.scan_register_num(),
            PDF_XIMAGE_BBOX_CODE => {
                self.scan_int();
                self.pdf_check_obj(OBJ_TYPE_XIMAGE, self.cur_val);
                i = self.obj_ximage_data(self.cur_val);
                self.scan_int();
                j = self.cur_val;
                if j < 1 || j > 4 {
                    self.pdf_error(b"pdfximagebbox", b"invalid parameter");
                }
            }
            _ => {} // there are no other cases
        }
        let old_setting = self.selector;
        self.selector = NEW_STRING;
        let b = self.pool_ptr;
        // §498 Print the result of command c
        match c {
            NUMBER_CODE => self.print_int(self.cur_val),
            ROMAN_NUMERAL_CODE => self.print_roman_int(self.cur_val),
            STRING_CODE => {
                if self.cur_cs != 0 {
                    self.sprint_cs(self.cur_cs);
                } else {
                    self.print_char(self.cur_chr);
                }
            }
            MEANING_CODE => self.print_meaning(),
            FONT_NAME_CODE => {
                self.print(self.font_name[self.cur_val]);
                if self.font_size[self.cur_val] != self.font_dsize[self.cur_val] {
                    self.print_s(b" at ");
                    self.print_scaled(self.font_size[self.cur_val]);
                    self.print_s(b"pt");
                }
            }
            ETEX_REVISION_CODE => self.print_s(ETEX_REVISION),
            PDFTEX_REVISION_CODE => self.print_s(PDFTEX_REVISION),
            PDFTEX_BANNER_CODE => self.print(self.pdftex_banner),
            PDF_FONT_NAME_CODE | PDF_FONT_OBJNUM_CODE => {
                self.set_ff(self.cur_val);
                if c == PDF_FONT_NAME_CODE {
                    self.print_int(self.obj_info(self.pdf_font_num[self.ff]));
                } else {
                    self.print_int(self.pdf_font_num[self.ff]);
                }
            }
            PDF_FONT_SIZE_CODE => {
                self.print_scaled(self.font_size[self.cur_val]);
                self.print_s(b"pt");
            }
            PDF_PAGE_REF_CODE => {
                let o = self.get_obj(OBJ_TYPE_PAGE, self.cur_val, false);
                self.print_int(o);
            }
            LEFT_MARGIN_KERN_CODE => {
                p = self.list_ptr(p);
                while p != NULL
                    && (self.cp_skipable(p)
                        || (!self.is_char_node(p) && self.ty(p) == GLUE_NODE && self.subtype(p) == LEFT_SKIP_CODE + 1))
                {
                    p = self.link(p);
                }
                if p != NULL && !self.is_char_node(p) && self.ty(p) == MARGIN_KERN_NODE && self.subtype(p) == LEFT_SIDE {
                    self.print_scaled(self.width(p));
                } else {
                    self.print(b'0' as i32);
                }
                self.print_s(b"pt");
            }
            RIGHT_MARGIN_KERN_CODE => {
                let q = self.list_ptr(p);
                p = self.prev_rightmost(q, NULL);
                while p != NULL
                    && (self.cp_skipable(p)
                        || (!self.is_char_node(p) && self.ty(p) == GLUE_NODE && self.subtype(p) == RIGHT_SKIP_CODE + 1))
                {
                    p = self.prev_rightmost(q, p);
                }
                if p != NULL && !self.is_char_node(p) && self.ty(p) == MARGIN_KERN_NODE && self.subtype(p) == RIGHT_SIDE {
                    self.print_scaled(self.width(p));
                } else {
                    self.print(b'0' as i32);
                }
                self.print_s(b"pt");
            }
            PDF_XFORM_NAME_CODE => self.print_int(self.obj_info(self.cur_val)),
            PDF_STRCMP_CODE => self.print_int(self.cur_val),
            PDF_COLORSTACK_INIT_CODE => self.print_int(self.cur_val),
            UNIFORM_DEVIATE_CODE => {
                self.host.world.mark_taint(Taint::Hard);
                let v = self.unif_rand(self.cur_val);
                self.print_int(v);
            }
            NORMAL_DEVIATE_CODE => {
                self.host.world.mark_taint(Taint::Hard);
                let v = self.norm_rand();
                self.print_int(v);
            }
            PDF_INSERT_HT_CODE => {
                i = self.cur_val;
                p = PAGE_INS_HEAD;
                while i >= self.subtype(self.link(p)) {
                    p = self.link(p);
                }
                if self.subtype(p) == i {
                    self.print_scaled(self.height(p));
                } else {
                    self.print(b'0' as i32);
                }
                self.print_s(b"pt");
            }
            PDF_XIMAGE_BBOX_CODE => {
                if self.is_pdf_image(i) {
                    match j {
                        1 => {
                            let v = self.epdf_orig_x(i);
                            self.print_scaled(v);
                        }
                        2 => {
                            let v = self.epdf_orig_y(i);
                            self.print_scaled(v);
                        }
                        3 => {
                            let v = self.epdf_orig_x(i) + self.image_width(i);
                            self.print_scaled(v);
                        }
                        4 => {
                            let v = self.epdf_orig_y(i) + self.image_height(i);
                            self.print_scaled(v);
                        }
                        _ => {}
                    }
                } else {
                    self.print_scaled(0);
                }
                self.print_s(b"pt");
            }
            JOB_NAME_CODE => self.print(self.job_name),
            _ => {} // there are no other cases
        }
        self.selector = old_setting;
        let p = self.buildtoks_str_toks_list(b);
        self.ins_list(p);
    }

    /// §499
    /// Returns def_ref (rc 0, never null), not the tail; see PORTING.md.
    pub fn scan_toks(&mut self, macro_def: bool, xpand: bool) -> i32 {
        self.scanner_status = if macro_def { DEFINING } else { ABSORBING };
        self.warning_index = self.cur_cs;
        let m = self.tb_mark();
        let saved_mark = self.tb_def_mark;
        self.tb_def_mark = m;
        let mut hash_brace = 0;
        let mut t = ZERO_TOKEN;
        'found: {
            if macro_def {
                // §500 Scan and build the parameter part of the macro definition
                'done: {
                    loop {
                        // continue:
                        #[cfg(not(feature = "slow_capture"))]
                        if self.capture_parameter_text(&mut t) {
                            break; // done1
                        }
                        self.get_token(); // set cur_cmd, cur_chr, cur_tok
                        if self.cur_tok < RIGHT_BRACE_LIMIT {
                            break; // done1
                        }
                        if self.cur_cmd == MAC_PARAM {
                            // §502 If the next character is a parameter number, make cur_tok a
                            // match token; but if it is a left brace, store `left_brace, end_match',
                            // set hash_brace, and goto done
                            let s = MATCH_TOKEN + self.cur_chr;
                            self.get_token();
                            if self.cur_tok < LEFT_BRACE_LIMIT {
                                hash_brace = self.cur_tok;
                                self.tb_push(self.cur_tok);
                                self.tb_push(END_MATCH_TOKEN);
                                break 'done;
                            }
                            if t == ZERO_TOKEN + 9 {
                                self.print_err_s(b"You already have nine parameters");
                                self.help(&[
                                    b"I'm going to ignore the # sign you just used,",
                                    b"as well as the token that followed it.",
                                ]);
                                self.error();
                                continue;
                            }
                            t += 1;
                            if self.cur_tok != t {
                                self.print_err_s(b"Parameters must be numbered consecutively");
                                self.help(&[
                                    b"I've inserted the digit you should have used after the #.",
                                    b"Type `1' to delete what you did use.",
                                ]);
                                self.back_error();
                            }
                            self.cur_tok = s;
                        }
                        self.tb_push(self.cur_tok);
                    }
                    // done1:
                    self.tb_push(END_MATCH_TOKEN);
                    if self.cur_cmd == RIGHT_BRACE {
                        // §501 Express shock at the missing left brace; goto found
                        self.print_err_s(b"Missing { inserted");
                        self.align_state += 1;
                        self.help(&[
                            b"Where was the left brace? You said something like `\\def\\a}',",
                            b"which I'm going to interpret as `\\def\\a{}'.",
                        ]);
                        self.error();
                        break 'found;
                    }
                }
                // done:
            } else {
                self.scan_left_brace(); // remove the compulsory left brace
            }
            // §503 Scan and build the body of the token list; goto found when finished
            let mut unbalance = 1;
            loop {
                #[cfg(not(feature = "slow_capture"))]
                {
                    self.capture_body(&mut unbalance, macro_def, xpand);
                    if unbalance == 1 && self.capture_right_brace().is_some() {
                        break 'found;
                    }
                }
                if xpand {
                    // §504 Expand the next part of the input
                    loop {
                        self.get_next();
                        if self.cur_cmd >= CALL && self.toks[self.cur_chr + 1] == PROTECTED_TOKEN {
                            self.cur_cmd = RELAX;
                            self.cur_chr = NO_EXPAND_FLAG;
                        }
                        if self.cur_cmd <= MAX_COMMAND {
                            break;
                        }
                        if self.cur_cmd != THE {
                            self.expand();
                        } else {
                            self.the_toks(); // the current builder is the list being built
                        }
                    }
                    self.x_token();
                } else {
                    self.get_token();
                }
                if self.cur_tok < RIGHT_BRACE_LIMIT {
                    if self.cur_cmd < RIGHT_BRACE {
                        unbalance += 1;
                    } else {
                        unbalance -= 1;
                        if unbalance == 0 {
                            break 'found;
                        }
                    }
                } else if self.cur_cmd == MAC_PARAM && macro_def {
                    // §505 Look for parameter number or ##
                    let s = self.cur_tok;
                    if xpand {
                        self.get_x_token();
                    } else {
                        self.get_token();
                    }
                    if self.cur_cmd != MAC_PARAM {
                        if self.cur_tok <= ZERO_TOKEN || self.cur_tok > t {
                            self.print_err_s(b"Illegal parameter number in definition of ");
                            self.sprint_cs(self.warning_index);
                            self.help(&[
                                b"You meant to type ## instead of #, right?",
                                b"Or maybe a } was forgotten somewhere earlier, and things",
                                b"are all screwed up? I'm going to assume that you meant ##.",
                            ]);
                            self.back_error();
                            self.cur_tok = s;
                        } else {
                            self.cur_tok = OUT_PARAM_TOKEN - b'0' as i32 + self.cur_chr;
                        }
                    }
                }
                self.tb_push(self.cur_tok);
            }
        }
        // found:
        self.scanner_status = NORMAL;
        if hash_brace != 0 {
            self.tb_push(hash_brace);
        }
        self.def_ref = self.tb_commit_ref(m);
        self.tb_def_mark = saved_mark;
        self.def_ref
    }

    /// §508
    /// Commits with tb_commit_ref; cur_val is the header.
    pub fn read_toks(&mut self, n: i32, r: i32, j: i32) {
        let mut n = n;
        self.scanner_status = DEFINING;
        self.warning_index = r;
        let mk = self.tb_mark();
        let saved_mark = self.tb_def_mark;
        self.tb_def_mark = mk;
        self.tb_push(END_MATCH_TOKEN);
        let m = if n < 0 || n > 15 { 16 } else { n };
        let s = self.align_state;
        self.align_state = 1000000; // disable tab marks, etc.
        loop {
            // §509 Input and store tokens from the next line of the file
            self.begin_file_reading();
            self.set_name(m + 1);
            if self.read_open[m] == CLOSED {
                // §510 Input for \read from the terminal
                if self.interaction > NONSTOP_MODE {
                    if n < 0 {
                        self.prompt_input_s(b"");
                    } else {
                        self.wake_up_terminal();
                        self.print_ln();
                        self.sprint_cs(r);
                        self.prompt_input_s(b"=");
                        n = -1;
                    }
                } else {
                    self.set_limit(0);
                    self.fatal_error(b"*** (cannot \\read from terminal in nonstop modes)");
                }
            } else if self.read_open[m] == JUST_OPEN {
                // §511 Input the first line of read_file[m]
                if self.input_ln(read_file(m), false) {
                    self.read_open[m] = NORMAL;
                } else {
                    self.a_close(read_file(m));
                    self.read_open[m] = CLOSED;
                }
            } else {
                // §512 Input the next line of read_file[m]
                if !self.input_ln(read_file(m), true) {
                    self.a_close(read_file(m));
                    self.read_open[m] = CLOSED;
                    if self.align_state != 1000000 {
                        self.runaway();
                        self.print_err_s(b"File ended within ");
                        self.print_esc_s(b"read");
                        self.help(&[b"This \\read has unbalanced braces."]);
                        self.align_state = 1000000;
                        self.set_limit(0);
                        self.error();
                    }
                }
            }
            self.set_limit(self.last);
            if self.end_line_char_inactive() {
                self.set_limit(self.limit() - 1);
            } else {
                let c = self.end_line_char();
                let l = self.limit();
                self.buffer.put(l, c);
            }
            self.first = self.limit() + 1;
            self.set_loc(self.start());
            self.set_state(NEW_LINE);
            'done: {
                // §1761 Handle \readline and goto done
                if j == 1 {
                    while self.loc() <= self.limit() {
                        // current line not yet finished
                        self.cur_chr = self.buffer.at(self.loc());
                        self.set_loc(self.loc() + 1);
                        if self.cur_chr == b' ' as i32 {
                            self.cur_tok = SPACE_TOKEN;
                        } else {
                            self.cur_tok = self.cur_chr + OTHER_TOKEN;
                        }
                        self.tb_push(self.cur_tok);
                    }
                    break 'done;
                }
                loop {
                    self.get_token();
                    if self.cur_tok == 0 {
                        break 'done; // cur_cmd=cur_chr=0 will occur at the end of the line
                    }
                    if self.align_state < 1000000 {
                        // unmatched `}' aborts the line
                        loop {
                            self.get_token();
                            if self.cur_tok == 0 {
                                break;
                            }
                        }
                        self.align_state = 1000000;
                        break 'done;
                    }
                    self.tb_push(self.cur_tok);
                }
            }
            self.end_file_reading();
            if self.align_state == 1000000 {
                break;
            }
        }
        self.def_ref = self.tb_commit_ref(mk);
        self.tb_def_mark = saved_mark;
        self.cur_val = self.def_ref;
        self.scanner_status = NORMAL;
        self.align_state = s;
    }

    // ---- External functions: the C side of web2c and pdfTeX, called by WEB. ----

    /// External: utils.c, \pdfescapehex
    pub fn escapehex(&mut self, in_: i32) {
        let out = self.pool_ptr;
        let mut i = in_;
        while i < out {
            if self.pool_ptr + 2 >= self.pool_size {
                self.pool_ptr = self.pool_size;
                return; // error by str_toks that calls str_room(1)
            }
            let ch = self.str_pool.at(i);
            i += 1;
            self.buildtoks_put_hex(ch);
        }
    }

    /// `%.2X`
    #[inline]
    fn buildtoks_put_hex(&mut self, ch: i32) {
        self.str_pool.put(self.pool_ptr, HEX_UPPER[(ch >> 4) as usize] as i32);
        self.str_pool.put(self.pool_ptr + 1, HEX_UPPER[(ch & 15) as usize] as i32);
        self.pool_ptr += 2;
    }

    #[inline]
    fn buildtoks_put(&mut self, ch: i32) {
        self.str_pool.put(self.pool_ptr, ch);
        self.pool_ptr += 1;
    }

    /// External: utils.c, \pdfunescapehex
    pub fn unescapehex(&mut self, in_: i32) {
        let out = self.pool_ptr;
        let mut i = in_;
        let mut a = 0;
        let mut first = true;
        while i < out {
            if self.pool_ptr + 1 >= self.pool_size {
                self.pool_ptr = self.pool_size;
                return; // error by str_toks that calls str_room(1)
            }
            let mut ch = self.str_pool.at(i);
            i += 1;
            if ch >= b'0' as i32 && ch <= b'9' as i32 {
                ch -= b'0' as i32;
            } else if ch >= b'A' as i32 && ch <= b'F' as i32 {
                ch -= b'A' as i32 - 10;
            } else if ch >= b'a' as i32 && ch <= b'f' as i32 {
                ch -= b'a' as i32 - 10;
            } else {
                continue; // ignore wrong character
            }
            if first {
                a = ch << 4;
                first = false;
                continue;
            }
            self.buildtoks_put(a + ch);
            first = true;
        }
        if !first {
            // last hex digit is omitted
            self.buildtoks_put(a);
        }
    }

    /// External: utils.c, \pdfescapename
    pub fn escapename(&mut self, in_: i32) {
        let out = self.pool_ptr;
        let mut i = in_;
        while i < out {
            if self.pool_ptr + 3 >= self.pool_size {
                self.pool_ptr = self.pool_size;
                return; // error by str_toks that calls str_room(1)
            }
            let ch = self.str_pool.at(i);
            i += 1;
            if (ch >= 1 && ch <= 32) || ch >= 127 {
                self.buildtoks_put(b'#' as i32);
                self.buildtoks_put_hex(ch);
                continue;
            }
            match ch {
                0 => {} // ignore
                35 | 37 | 40 | 41 | 47 | 60 | 62 | 91 | 93 | 123 | 125 => {
                    self.buildtoks_put(b'#' as i32);
                    self.buildtoks_put_hex(ch);
                }
                _ => self.buildtoks_put(ch),
            }
        }
    }

    /// External: utils.c, \pdfescapestring
    pub fn escapestring(&mut self, in_: i32) {
        let out = self.pool_ptr;
        let mut i = in_;
        while i < out {
            if self.pool_ptr + 4 >= self.pool_size {
                self.pool_ptr = self.pool_size;
                return; // error by str_toks that calls str_room(1)
            }
            let ch = self.str_pool.at(i);
            i += 1;
            if ch < b'!' as i32 || ch > b'~' as i32 {
                // convert control characters into oct
                self.buildtoks_put(b'\\' as i32);
                self.buildtoks_put(b'0' as i32 + (ch >> 6));
                self.buildtoks_put(b'0' as i32 + ((ch >> 3) & 7));
                self.buildtoks_put(b'0' as i32 + (ch & 7));
                continue;
            }
            if ch == b'(' as i32 || ch == b')' as i32 || ch == b'\\' as i32 {
                self.buildtoks_put(b'\\' as i32); // escape parenthesis and backslash
            }
            self.buildtoks_put(ch);
        }
    }

    /// texmfmp.c `makecstring`: the bytes of string `s`, as C sees them: up to the first NUL.
    fn buildtoks_cstring(&self, s: i32) -> Vec<u8> {
        let (a, b) = (self.str_start[s], self.str_start[s + 1]);
        let bytes = &self.str_pool.as_slice()[a as usize..b as usize];
        match bytes.iter().position(|&c| c == 0) {
            Some(n) => bytes[..n].to_vec(),
            None => bytes.to_vec(),
        }
    }

    /// texmfmp.c `find_input_file`.  `access(R_OK)` and `dir_p` of the -output-directory try are
    /// one read here, which succeeds in just those cases.
    fn buildtoks_find_input_file(&mut self, s: i32) -> Option<Vec<u8>> {
        // makecfilename: quotes removed
        let mut filename = self.buildtoks_cstring(s);
        filename.retain(|&c| c != b'"');
        if let Some(dir) = self.host.world.output_directory.clone() {
            if !crate::platform::is_absolute(&filename) {
                let mut pathname = dir;
                pathname.push(b'/');
                pathname.extend_from_slice(&filename);
                if self.host.world.read(&pathname).is_some() {
                    return Some(pathname);
                }
            }
        }
        if !rstex_kpse::in_name_ok(&filename) {
            return None; // no permission
        }
        self.host.world.lookup(&filename, KPSE_TEX_FORMAT, true)
    }

    /// texmfmp.c `makepdftime`.
    fn buildtoks_makepdftime(&self, t: i64, utc: bool) -> Vec<u8> {
        let lt = crate::platform::broken_time(t, utc);
        let gmt = crate::platform::broken_time(t, true);
        // %S can be in range 00..61, and PDF expects 00..59
        let second = if lt.second >= 60 { 59 } else { lt.second };
        let mut time_str = format!(
            "D:{}{:02}{:02}{:02}{:02}{:02}",
            lt.year, lt.month, lt.day, lt.hour, lt.minute, second
        );
        // this calculation method was found in exim's tod.c
        let mut off = 60 * (lt.hour - gmt.hour) + lt.minute - gmt.minute;
        if lt.year != gmt.year {
            off += if lt.year > gmt.year { 1440 } else { -1440 };
        } else if (lt.month, lt.day) != (gmt.month, gmt.day) {
            off += if (lt.month, lt.day) > (gmt.month, gmt.day) { 1440 } else { -1440 };
        }
        if off == 0 {
            time_str.push('Z');
        } else {
            let off_hours = off / 60;
            let off_mins = (off - off_hours * 60).abs();
            time_str.push_str(&format!("{:+03}'{:02}'", off_hours, off_mins));
        }
        time_str.into_bytes()
    }

    /// Puts `s` on the pool, or sets `pool_ptr:=pool_size` if there is no room, as the C does.
    fn buildtoks_put_or_fill(&mut self, s: &[u8]) {
        if self.pool_ptr as i64 + s.len() as i64 >= self.pool_size as i64 {
            self.pool_ptr = self.pool_size; // error by str_toks that calls str_room(1)
        } else {
            for &c in s {
                self.buildtoks_put(c as i32);
            }
        }
    }

    /// External: texmfmp.c, \pdffilesize
    pub fn getfilesize(&mut self, s: i32) {
        self.host.world.mark_taint(Taint::Soft);
        let Some(file_name) = self.buildtoks_find_input_file(s) else {
            return; // empty string
        };
        if let Some(st) = self.host.world.stat(&file_name) {
            let buf = st.size.to_string();
            self.buildtoks_put_or_fill(buf.as_bytes());
        }
    }

    /// External: texmfmp.c, \pdffilemoddate
    pub fn getfilemoddate(&mut self, s: i32) {
        self.host.world.mark_taint(Taint::Soft);
        let Some(file_name) = self.buildtoks_find_input_file(s) else {
            return; // empty string
        };
        if let Some(st) = self.host.world.stat(&file_name) {
            // FORCE_SOURCE_DATE_set && SOURCE_DATE_EPOCH_set
            let use_utc = self.host.world.env(b"FORCE_SOURCE_DATE").as_deref() == Some(b"1")
                && self.host.world.env(b"SOURCE_DATE_EPOCH").is_some();
            let time_str = self.buildtoks_makepdftime(st.mtime, use_utc);
            self.buildtoks_put_or_fill(&time_str);
        }
    }

    /// External: texmfmp.c, \pdffiledump
    pub fn getfiledump(&mut self, s: i32, offset: i32, length: i32) {
        self.host.world.mark_taint(Taint::Soft);
        if length == 0 {
            return; // empty result string
        }
        if self.pool_ptr as i64 + 2 * length as i64 + 1 >= self.pool_size as i64 {
            self.pool_ptr = self.pool_size; // no place for the result; error by str_toks
            return;
        }
        let Some(file_name) = self.buildtoks_find_input_file(s) else {
            return; // empty string
        };
        let Some(data) = self.host.world.read(&file_name) else {
            return;
        };
        let from = (offset as usize).min(data.len());
        let to = (from + length as usize).min(data.len());
        for k in from..to {
            self.buildtoks_put_hex(data[k] as i32);
        }
    }

    /// External: texmfmp.c, \pdfmdfivesum
    pub fn getmd5sum(&mut self, s: i32, file: bool) {
        let digest;
        if file {
            self.host.world.mark_taint(Taint::Soft);
            let Some(file_name) = self.buildtoks_find_input_file(s) else {
                return; // empty string
            };
            let Some(data) = self.host.world.read(&file_name) else {
                return;
            };
            digest = rstex_pdfio::md5::md5(&data);
        } else {
            // s contains the data
            let (a, b) = (self.str_start[s], self.str_start[s + 1]);
            digest = rstex_pdfio::md5::md5(&self.str_pool.as_slice()[a as usize..b as usize]);
        }
        if self.pool_ptr + 32 >= self.pool_size {
            return; // error by str_toks that calls str_room(1)
        }
        for d in digest {
            self.buildtoks_put_hex(d as i32);
        }
    }

    /// External: utils.c, \pdfmatch: POSIX ERE, leftmost-longest
    pub fn matchstrings(&mut self, s: i32, t: i32, subcount: i32, icase: bool) -> i32 {
        if self.pool_ptr + 10 >= self.pool_size {
            self.pool_ptr = self.pool_size;
            return 0;
        }
        let pattern = self.buildtoks_cstring(s);
        match regex::Regex::compile(&pattern, icase) {
            Err(message) => {
                // utils.c pdftex_warn("%s%s", "\\pdfmatch: ", str)
                self.print_ln();
                self.print_ln();
                self.print_s(b"pdfTeX warning: ");
                let name = rstex_kpse::invocation_name();
                self.print_s(&name);
                self.print_s(b": \\pdfmatch: ");
                self.print_s(message);
                self.print_ln();
                self.buildtoks_put(b'-' as i32);
                self.buildtoks_put(b'1' as i32);
            }
            Ok(preg) => {
                let text = self.buildtoks_cstring(t);
                let count = if subcount < 0 { DEFAULT_SUB_MATCH_COUNT } else { subcount };
                self.v_buildtoks.sub_match_count = count;
                let ret = preg.exec(&text, count as usize);
                let mut pm = self.v_buildtoks.pmatch;
                pm.len = 0;
                if let Some(regs) = &ret {
                    for &(so, eo) in regs {
                        self.arena_push(&mut pm, MatchSpan { so, eo });
                    }
                }
                self.v_buildtoks.pmatch = pm;
                let mut ms = self.v_buildtoks.match_string;
                ms.len = 0;
                for &c in &text {
                    self.arena_push(&mut ms, c);
                }
                self.v_buildtoks.match_string = ms;
                self.v_buildtoks.match_string_set = true;
                self.v_buildtoks.last_match_succeeded = ret.is_some();
                self.buildtoks_put(if ret.is_some() { b'1' } else { b'0' } as i32);
            }
        }
        0
    }

    /// External: utils.c, \pdflastmatch
    pub fn getmatch(&mut self, i: i32) {
        let mut found = false;
        let mut span = MatchSpan { so: -1, eo: -1 };
        let pm = self.v_buildtoks.pmatch;
        if i >= 0
            && i < self.v_buildtoks.sub_match_count
            && self.v_buildtoks.match_string_set
            && self.v_buildtoks.last_match_succeeded
            && i < pm.len
        {
            span = pm.get(&mut self.arena, i);
            found = span.so >= 0 && span.eo >= span.so;
        }
        let len = if found { span.eo - span.so } else { 0 };
        let size = if found { 20 + len } else { 4 }; // 20: place for integer number and '->'
        if self.pool_ptr + size >= self.pool_size {
            self.pool_ptr = self.pool_size;
            return;
        }
        if found {
            let number = span.so.to_string();
            for &c in number.as_bytes() {
                self.buildtoks_put(c as i32);
            }
            self.buildtoks_put(b'-' as i32);
            self.buildtoks_put(b'>' as i32);
            let ms = self.v_buildtoks.match_string;
            for k in 0..len {
                let c = ms.get(&mut self.arena, span.so + k);
                self.buildtoks_put(c as i32);
            }
            return;
        }
        for &c in b"-1->" {
            self.buildtoks_put(c as i32);
        }
    }

    /// External: utils.c.  The stacks are `pdfship.rs`'s, with the other functions of that file.
    pub fn newcolorstack(&mut self, s: i32, literal_mode: i32, pagestart: bool) -> i32 {
        self.pdfship_new_colorstack(s, literal_mode, pagestart)
    }

    /// External: texmfmp.c, \pdfcreationdate
    pub fn getcreationdate(&mut self) {
        self.host.world.mark_taint(Taint::Hard);
        let (time_str, n) = self.pdfship_start_time_str();
        self.buildtoks_put_or_fill(&time_str[..n]);
    }
}

/// glibc's `regcomp` and `regexec` for `REG_EXTENDED`, with or without `REG_ICASE`, in the C locale,
/// which is the only one pdfTeX runs in: it never calls `setlocale`.
///
/// `\pdfmatch` shows every submatch, and glibc's submatches are not POSIX's: they come from walking
/// its automaton along a path that prefers lower node numbers, with quirks at empty iterations, at
/// anchors in a repeated group, and throughout back-references.  So this follows the C function by
/// function, under the C's names: the parser builds the same tree, numbered the same way, and
/// `exec` is `re_search_internal`.  What is left out changes nothing a caller can see: the DFA's
/// state table, since a state is made afresh instead of found again; the fastmap, which only avoids
/// start positions that would fail; and all multibyte code.
///
/// The pinned sources have an older copy of this code in `pdftexdir/regex`, which pdfTeX does not
/// use: the oracle links glibc 2.44.  Where they differ this follows 2.44, and the differences are
/// marked; `crates/engine/testdata/regex_oracle.tsv` has the oracle's answers.
pub(crate) mod regex {
    use std::rc::Rc;

    // re_token_type_t
    const CHARACTER: u8 = 1;
    const END_OF_RE: u8 = 2;
    const SIMPLE_BRACKET: u8 = 3;
    const OP_BACK_REF: u8 = 4;
    const OP_PERIOD: u8 = 5;
    const EPSILON_BIT: u8 = 8;
    const OP_OPEN_SUBEXP: u8 = EPSILON_BIT;
    const OP_CLOSE_SUBEXP: u8 = EPSILON_BIT | 1;
    const OP_ALT: u8 = EPSILON_BIT | 2;
    const OP_DUP_ASTERISK: u8 = EPSILON_BIT | 3;
    const ANCHOR: u8 = EPSILON_BIT | 4;
    const CONCAT: u8 = 16;
    const SUBEXP: u8 = 17;
    const OP_DUP_PLUS: u8 = 18;
    const OP_DUP_QUESTION: u8 = 19;
    const OP_OPEN_BRACKET: u8 = 20;
    const OP_CLOSE_BRACKET: u8 = 21;
    const OP_CHARSET_RANGE: u8 = 22;
    const OP_OPEN_DUP_NUM: u8 = 23;
    const OP_CLOSE_DUP_NUM: u8 = 24;
    const OP_NON_MATCH_LIST: u8 = 25;
    const OP_OPEN_COLL_ELEM: u8 = 26;
    const OP_OPEN_EQUIV_CLASS: u8 = 27;
    const OP_OPEN_CHAR_CLASS: u8 = 28;
    const OP_WORD: u8 = 29;
    const OP_NOTWORD: u8 = 30;
    const OP_SPACE: u8 = 31;
    const OP_NOTSPACE: u8 = 32;
    const BACK_SLASH: u8 = 33;

    // re_context_type
    const PREV_WORD_CONSTRAINT: u32 = 0x0001;
    const PREV_NOTWORD_CONSTRAINT: u32 = 0x0002;
    const NEXT_WORD_CONSTRAINT: u32 = 0x0004;
    const NEXT_NOTWORD_CONSTRAINT: u32 = 0x0008;
    const PREV_NEWLINE_CONSTRAINT: u32 = 0x0010;
    const NEXT_NEWLINE_CONSTRAINT: u32 = 0x0020;
    const PREV_BEGBUF_CONSTRAINT: u32 = 0x0040;
    const NEXT_ENDBUF_CONSTRAINT: u32 = 0x0080;
    const WORD_DELIM_CONSTRAINT: u32 = 0x0100;
    const NOT_WORD_DELIM_CONSTRAINT: u32 = 0x0200;
    const INSIDE_WORD: u32 = PREV_WORD_CONSTRAINT | NEXT_WORD_CONSTRAINT;
    const WORD_FIRST: u32 = PREV_NOTWORD_CONSTRAINT | NEXT_WORD_CONSTRAINT;
    const WORD_LAST: u32 = PREV_WORD_CONSTRAINT | NEXT_NOTWORD_CONSTRAINT;
    const INSIDE_NOTWORD: u32 = PREV_NOTWORD_CONSTRAINT | NEXT_NOTWORD_CONSTRAINT;
    const LINE_FIRST: u32 = PREV_NEWLINE_CONSTRAINT;
    const LINE_LAST: u32 = NEXT_NEWLINE_CONSTRAINT;
    const BUF_FIRST: u32 = PREV_BEGBUF_CONSTRAINT;
    const BUF_LAST: u32 = NEXT_ENDBUF_CONSTRAINT;
    const WORD_DELIM: u32 = WORD_DELIM_CONSTRAINT;
    const NOT_WORD_DELIM: u32 = NOT_WORD_DELIM_CONSTRAINT;

    const CONTEXT_WORD: u32 = 1;
    const CONTEXT_NEWLINE: u32 = 2;
    const CONTEXT_BEGBUF: u32 = 4;
    const CONTEXT_ENDBUF: u32 = 8;

    const RE_DUP_MAX: i32 = 0x7fff;
    const BRACKET_NAME_BUF_SIZE: usize = 32;
    const NONE: i32 = -1;

    // regerror's messages
    const REG_BADPAT: &[u8] = b"Invalid regular expression";
    const REG_ECOLLATE: &[u8] = b"Invalid collation character";
    const REG_ECTYPE: &[u8] = b"Invalid character class name";
    const REG_EESCAPE: &[u8] = b"Trailing backslash";
    const REG_ESUBREG: &[u8] = b"Invalid back reference";
    const REG_EBRACK: &[u8] = b"Unmatched [, [^, [:, [., or [=";
    const REG_EPAREN: &[u8] = b"Unmatched ( or \\(";
    const REG_EBRACE: &[u8] = b"Unmatched \\{";
    const REG_BADBR: &[u8] = b"Invalid content of \\{\\}";
    const REG_ERANGE: &[u8] = b"Invalid range end";
    const REG_BADRPT: &[u8] = b"Invalid preceding regular expression";
    const REG_ESIZE: &[u8] = b"Regular expression too big";

    type Err = &'static [u8];
    type Bitset = [u64; 4];
    /// re_node_set: sorted, no duplicates.
    type NodeSet = Vec<i32>;
    type StateRef = Option<Rc<State>>;

    #[inline]
    fn bit_set(b: &mut Bitset, c: u8) {
        b[(c >> 6) as usize] |= 1u64 << (c & 63);
    }
    #[inline]
    fn bit_contain(b: &Bitset, c: u8) -> bool {
        b[(c >> 6) as usize] & (1u64 << (c & 63)) != 0
    }

    #[inline]
    fn set_contains(s: &[i32], e: i32) -> bool {
        s.binary_search(&e).is_ok()
    }
    #[inline]
    fn set_insert(s: &mut NodeSet, e: i32) {
        if let Err(i) = s.binary_search(&e) {
            s.insert(i, e);
        }
    }
    #[inline]
    fn set_remove(s: &mut NodeSet, e: i32) {
        if let Ok(i) = s.binary_search(&e) {
            s.remove(i);
        }
    }
    fn set_merge(d: &mut NodeSet, s: &[i32]) {
        if s.is_empty() {
            return;
        }
        let mut out = Vec::with_capacity(d.len() + s.len());
        let (mut i, mut j) = (0, 0);
        while i < d.len() && j < s.len() {
            if d[i] < s[j] {
                out.push(d[i]);
                i += 1;
            } else if d[i] > s[j] {
                out.push(s[j]);
                j += 1;
            } else {
                out.push(d[i]);
                i += 1;
                j += 1;
            }
        }
        out.extend_from_slice(&d[i..]);
        out.extend_from_slice(&s[j..]);
        *d = out;
    }
    /// re_node_set_add_intersect: `d := d union (s1 intersect s2)`.
    fn set_add_intersect(d: &mut NodeSet, s1: &[i32], s2: &[i32]) {
        let mut common = Vec::new();
        let (mut i, mut j) = (0, 0);
        while i < s1.len() && j < s2.len() {
            if s1[i] < s2[j] {
                i += 1;
            } else if s1[i] > s2[j] {
                j += 1;
            } else {
                common.push(s1[i]);
                i += 1;
                j += 1;
            }
        }
        set_merge(d, &common);
    }

    #[inline]
    fn is_epsilon(ty: u8) -> bool {
        ty & EPSILON_BIT != 0
    }
    #[inline]
    fn is_alnum(c: u8) -> bool {
        c.is_ascii_alphanumeric()
    }
    #[inline]
    fn is_word_char(c: u8) -> bool {
        is_alnum(c) || c == b'_'
    }
    #[inline]
    fn not_satisfy_prev(constraint: u32, context: u32) -> bool {
        (constraint & PREV_WORD_CONSTRAINT != 0 && context & CONTEXT_WORD == 0)
            || (constraint & PREV_NOTWORD_CONSTRAINT != 0 && context & CONTEXT_WORD != 0)
            || (constraint & PREV_NEWLINE_CONSTRAINT != 0 && context & CONTEXT_NEWLINE == 0)
            || (constraint & PREV_BEGBUF_CONSTRAINT != 0 && context & CONTEXT_BEGBUF == 0)
    }
    #[inline]
    fn not_satisfy_next(constraint: u32, context: u32) -> bool {
        (constraint & NEXT_WORD_CONSTRAINT != 0 && context & CONTEXT_WORD == 0)
            || (constraint & NEXT_NOTWORD_CONSTRAINT != 0 && context & CONTEXT_WORD != 0)
            || (constraint & NEXT_NEWLINE_CONSTRAINT != 0 && context & CONTEXT_NEWLINE == 0)
            || (constraint & NEXT_ENDBUF_CONSTRAINT != 0 && context & CONTEXT_ENDBUF == 0)
    }

    /// re_token_t.  C has `opr` as a union, and `fetch_number` reads its `c` whatever the token is,
    /// so `c` here holds the low byte of `idx` or `ctx` where the union would.
    #[derive(Clone, Copy, Default)]
    struct Tok {
        ty: u8,
        c: u8,
        idx: i32,
        ctx: u32,
        sbc: i32,
        word_char: bool,
        duplicated: bool,
        opt_subexp: bool,
        constraint: u32,
    }

    /// bin_tree_t
    #[derive(Clone, Copy)]
    struct Bin {
        parent: i32,
        left: i32,
        right: i32,
        first: i32,
        next: i32,
        token: Tok,
        node_idx: i32,
    }

    /// bracket_elem_t
    enum Elem {
        SbChar(u8),
        CollSym(Vec<u8>),
        EquivClass(Vec<u8>),
        CharClass(Vec<u8>),
    }

    /// re_dfastate_t, without the tables.
    struct State {
        nodes: NodeSet,
        non_eps_nodes: NodeSet,
        /// `entrance_nodes`, when not `nodes`.
        entrance: Option<NodeSet>,
        context: u32,
        halt: bool,
        has_backref: bool,
        has_constraint: bool,
    }

    impl State {
        fn entrance_nodes(&self) -> &NodeSet {
            self.entrance.as_ref().unwrap_or(&self.nodes)
        }

        /// What the pointers' being equal means in C, where the table holds one state for these.
        fn same_as(&self, other: &State) -> bool {
            self.context == other.context && self.entrance_nodes() == other.entrance_nodes()
        }
    }

    /// re_dfa_t and regex_t.
    pub struct Regex {
        nodes: Vec<Tok>,
        nexts: Vec<i32>,
        org_indices: Vec<i32>,
        edests: Vec<NodeSet>,
        eclosures: Vec<NodeSet>,
        /// `eclosures[n].nelem` of C: 0 not calculated, -1 in progress, 1 done.
        ecl_state: Vec<i8>,
        inveclosures: Vec<NodeSet>,
        sets: Vec<Bitset>,
        word_char: Bitset,
        word_ops_used: bool,
        nbackref: i32,
        has_plural_match: bool,
        has_mb_node: bool,
        used_bkref_map: u64,
        completed_bkref_map: u32,
        subexp_map: Option<Vec<i32>>,
        init_node: i32,
        init_nodes: NodeSet,
        re_nsub: i32,
        icase: bool,
    }

    /// The parser's and tree passes' work area, with re_string_t for the pattern.
    struct Compiler<'a> {
        raw: &'a [u8],
        mbs: Vec<u8>,
        cur: usize,
        tree: Vec<Bin>,
        dfa: Regex,
    }

    impl<'a> Compiler<'a> {
        fn eoi(&self) -> bool {
            self.cur >= self.mbs.len()
        }

        fn create_token_tree(&mut self, left: i32, right: i32, token: Tok) -> i32 {
            let mut t = token;
            t.duplicated = false;
            t.opt_subexp = false;
            let n = self.tree.len() as i32;
            self.tree.push(Bin { parent: NONE, left, right, first: NONE, next: NONE, token: t, node_idx: NONE });
            if left != NONE {
                self.tree[left as usize].parent = n;
            }
            if right != NONE {
                self.tree[right as usize].parent = n;
            }
            n
        }

        fn create_tree(&mut self, left: i32, right: i32, ty: u8) -> i32 {
            self.create_token_tree(left, right, Tok { ty, ..Tok::default() })
        }

        /// RE_SYNTAX_POSIX_EXTENDED is built into what this answers.
        fn peek_token(&self) -> (Tok, usize) {
            let mut t = Tok::default();
            if self.eoi() {
                t.ty = END_OF_RE;
                return (t, 0);
            }
            let c = self.mbs[self.cur];
            t.c = c;
            if c == b'\\' {
                if self.cur + 1 >= self.mbs.len() {
                    t.ty = BACK_SLASH;
                    return (t, 1);
                }
                // re_string_peek_byte_case: an escaped character is not folded
                let c2 = self.raw[self.cur + 1];
                t.c = c2;
                t.ty = CHARACTER;
                t.word_char = is_word_char(c2);
                match c2 {
                    b'1'..=b'9' => {
                        t.ty = OP_BACK_REF;
                        t.idx = (c2 - b'1') as i32;
                        t.c = t.idx as u8;
                    }
                    b'<' => Self::anchor(&mut t, WORD_FIRST),
                    b'>' => Self::anchor(&mut t, WORD_LAST),
                    b'b' => Self::anchor(&mut t, WORD_DELIM),
                    b'B' => Self::anchor(&mut t, NOT_WORD_DELIM),
                    b'w' => t.ty = OP_WORD,
                    b'W' => t.ty = OP_NOTWORD,
                    b's' => t.ty = OP_SPACE,
                    b'S' => t.ty = OP_NOTSPACE,
                    b'`' => Self::anchor(&mut t, BUF_FIRST),
                    b'\'' => Self::anchor(&mut t, BUF_LAST),
                    _ => {}
                }
                return (t, 2);
            }
            t.ty = CHARACTER;
            t.word_char = is_word_char(c);
            match c {
                b'|' => t.ty = OP_ALT,
                b'*' => t.ty = OP_DUP_ASTERISK,
                b'+' => t.ty = OP_DUP_PLUS,
                b'?' => t.ty = OP_DUP_QUESTION,
                b'{' => t.ty = OP_OPEN_DUP_NUM,
                b'}' => t.ty = OP_CLOSE_DUP_NUM,
                b'(' => t.ty = OP_OPEN_SUBEXP,
                b')' => t.ty = OP_CLOSE_SUBEXP,
                b'[' => t.ty = OP_OPEN_BRACKET,
                b'.' => t.ty = OP_PERIOD,
                b'^' => Self::anchor(&mut t, LINE_FIRST),
                b'$' => Self::anchor(&mut t, LINE_LAST),
                _ => {}
            }
            (t, 1)
        }

        fn anchor(t: &mut Tok, ctx: u32) {
            t.ty = ANCHOR;
            t.ctx = ctx;
            t.c = ctx as u8;
        }

        fn fetch_token(&mut self) -> Tok {
            let (t, n) = self.peek_token();
            self.cur += n;
            t
        }

        fn peek_token_bracket(&self) -> (Tok, usize) {
            let mut t = Tok::default();
            if self.eoi() {
                t.ty = END_OF_RE;
                return (t, 0);
            }
            let c = self.mbs[self.cur];
            t.c = c;
            if c == b'[' {
                let c2 = if self.cur + 1 < self.mbs.len() { self.mbs[self.cur + 1] } else { 0 };
                t.c = c2;
                match c2 {
                    b'.' => t.ty = OP_OPEN_COLL_ELEM,
                    b'=' => t.ty = OP_OPEN_EQUIV_CLASS,
                    b':' => t.ty = OP_OPEN_CHAR_CLASS,
                    _ => {
                        t.ty = CHARACTER;
                        t.c = c;
                        return (t, 1);
                    }
                }
                return (t, 2);
            }
            t.ty = match c {
                b'-' => OP_CHARSET_RANGE,
                b']' => OP_CLOSE_BRACKET,
                b'^' => OP_NON_MATCH_LIST,
                _ => CHARACTER,
            };
            (t, 1)
        }

        fn parse(&mut self) -> Result<i32, Err> {
            let mut token = self.fetch_token();
            let tree = self.parse_reg_exp(&mut token, 0)?;
            let eor = self.create_tree(NONE, NONE, END_OF_RE);
            Ok(if tree != NONE { self.create_tree(tree, eor, CONCAT) } else { eor })
        }

        fn parse_reg_exp(&mut self, token: &mut Tok, nest: i32) -> Result<i32, Err> {
            // glibc 2.44: a branch cannot refer to a group of an earlier branch.
            let initial_bkref_map = self.dfa.completed_bkref_map;
            let mut tree = self.parse_branch(token, nest)?;
            while token.ty == OP_ALT {
                *token = self.fetch_token();
                let branch;
                if token.ty != OP_ALT && token.ty != END_OF_RE && (nest == 0 || token.ty != OP_CLOSE_SUBEXP) {
                    let accumulated_bkref_map = self.dfa.completed_bkref_map;
                    self.dfa.completed_bkref_map = initial_bkref_map;
                    branch = self.parse_branch(token, nest)?;
                    self.dfa.completed_bkref_map |= accumulated_bkref_map;
                } else {
                    branch = NONE;
                }
                tree = self.create_tree(tree, branch, OP_ALT);
            }
            Ok(tree)
        }

        fn parse_branch(&mut self, token: &mut Tok, nest: i32) -> Result<i32, Err> {
            let mut tree = self.parse_expression(token, nest)?;
            while token.ty != OP_ALT && token.ty != END_OF_RE && (nest == 0 || token.ty != OP_CLOSE_SUBEXP) {
                let exp = self.parse_expression(token, nest)?;
                if tree != NONE && exp != NONE {
                    tree = self.create_tree(tree, exp, CONCAT);
                } else if tree == NONE {
                    tree = exp;
                }
                // otherwise exp == NULL, we don't need to create a new tree
            }
            Ok(tree)
        }

        fn parse_expression(&mut self, token: &mut Tok, nest: i32) -> Result<i32, Err> {
            let mut tree;
            match token.ty {
                CHARACTER => tree = self.create_token_tree(NONE, NONE, *token),
                OP_OPEN_SUBEXP => tree = self.parse_sub_exp(token, nest + 1)?,
                OP_OPEN_BRACKET => tree = self.parse_bracket_exp(token)?,
                OP_BACK_REF => {
                    if self.dfa.completed_bkref_map & (1 << token.idx) == 0 {
                        return Err(REG_ESUBREG);
                    }
                    self.dfa.used_bkref_map |= 1 << token.idx;
                    tree = self.create_token_tree(NONE, NONE, *token);
                    self.dfa.nbackref += 1;
                    self.dfa.has_mb_node = true;
                }
                // RE_CONTEXT_INVALID_OPS
                OP_OPEN_DUP_NUM | OP_DUP_ASTERISK | OP_DUP_PLUS | OP_DUP_QUESTION => return Err(REG_BADRPT),
                // RE_UNMATCHED_RIGHT_PAREN_ORD
                OP_CLOSE_SUBEXP | OP_CLOSE_DUP_NUM => {
                    token.ty = CHARACTER;
                    tree = self.create_token_tree(NONE, NONE, *token);
                }
                ANCHOR => {
                    if token.ctx & (WORD_DELIM | NOT_WORD_DELIM | WORD_FIRST | WORD_LAST) != 0 && !self.dfa.word_ops_used {
                        self.init_word_char();
                    }
                    if token.ctx == WORD_DELIM || token.ctx == NOT_WORD_DELIM {
                        let tree_first;
                        if token.ctx == WORD_DELIM {
                            token.ctx = WORD_FIRST;
                            tree_first = self.create_token_tree(NONE, NONE, *token);
                            token.ctx = WORD_LAST;
                        } else {
                            token.ctx = INSIDE_WORD;
                            tree_first = self.create_token_tree(NONE, NONE, *token);
                            token.ctx = INSIDE_NOTWORD;
                        }
                        let tree_last = self.create_token_tree(NONE, NONE, *token);
                        tree = self.create_tree(tree_first, tree_last, OP_ALT);
                    } else {
                        tree = self.create_token_tree(NONE, NONE, *token);
                    }
                    // We must return here, since ANCHORs can't be followed by repetition operators.
                    *token = self.fetch_token();
                    return Ok(tree);
                }
                OP_PERIOD => tree = self.create_token_tree(NONE, NONE, *token),
                OP_WORD | OP_NOTWORD => tree = self.build_charclass_op(b"alnum", b"_", token.ty == OP_NOTWORD)?,
                OP_SPACE | OP_NOTSPACE => tree = self.build_charclass_op(b"space", b"", token.ty == OP_NOTSPACE)?,
                OP_ALT | END_OF_RE => return Ok(NONE),
                BACK_SLASH => return Err(REG_EESCAPE),
                _ => return Ok(NONE),
            }
            *token = self.fetch_token();
            while token.ty == OP_DUP_ASTERISK
                || token.ty == OP_DUP_PLUS
                || token.ty == OP_DUP_QUESTION
                || token.ty == OP_OPEN_DUP_NUM
            {
                tree = self.parse_dup_op(tree, token)?;
            }
            Ok(tree)
        }

        fn parse_sub_exp(&mut self, token: &mut Tok, nest: i32) -> Result<i32, Err> {
            let cur_nsub = self.dfa.re_nsub;
            self.dfa.re_nsub += 1;
            *token = self.fetch_token();
            let tree;
            if token.ty == OP_CLOSE_SUBEXP {
                tree = NONE; // the subexpression may be a null string
            } else {
                tree = self.parse_reg_exp(token, nest)?;
                if token.ty != OP_CLOSE_SUBEXP {
                    return Err(REG_EPAREN);
                }
            }
            if cur_nsub <= (b'9' - b'1') as i32 {
                self.dfa.completed_bkref_map |= 1 << cur_nsub;
            }
            let t = self.create_tree(tree, NONE, SUBEXP);
            self.tree[t as usize].token.idx = cur_nsub;
            Ok(t)
        }

        fn fetch_number(&mut self, token: &mut Tok) -> i32 {
            let mut num = -1;
            loop {
                *token = self.fetch_token();
                let c = token.c;
                if token.ty == END_OF_RE {
                    return -2;
                }
                if token.ty == OP_CLOSE_DUP_NUM || c == b',' {
                    break;
                }
                num = if token.ty != CHARACTER || c < b'0' || b'9' < c || num == -2 {
                    -2
                } else if num == -1 {
                    (c - b'0') as i32
                } else {
                    (RE_DUP_MAX + 1).min(num * 10 + (c - b'0') as i32) // glibc 2.44
                };
            }
            num
        }

        fn parse_dup_op(&mut self, elem: i32, token: &mut Tok) -> Result<i32, Err> {
            let mut elem = elem;
            let (start, end);
            if token.ty == OP_OPEN_DUP_NUM {
                let mut s = self.fetch_number(token);
                let mut e = 0;
                if s == -1 {
                    if token.ty == CHARACTER && token.c == b',' {
                        s = 0; // we treat "{,m}" as "{0,m}"
                    } else {
                        return Err(REG_BADBR); // <re>{} is invalid
                    }
                }
                if s != -2 {
                    // we treat "{n}" as "{n,n}"
                    e = if token.ty == OP_CLOSE_DUP_NUM {
                        s
                    } else if token.ty == CHARACTER && token.c == b',' {
                        self.fetch_number(token)
                    } else {
                        -2
                    };
                }
                if s == -2 || e == -2 {
                    // Invalid sequence; RE_INVALID_INTERVAL_ORD is not set
                    return Err(if token.ty == END_OF_RE { REG_EBRACE } else { REG_BADBR });
                }
                if (e != -1 && s > e) || token.ty != OP_CLOSE_DUP_NUM {
                    return Err(REG_BADBR); // first number greater than second
                }
                if RE_DUP_MAX < (if e == -1 { s } else { e }) {
                    return Err(REG_ESIZE); // glibc 2.44
                }
                start = s;
                end = e;
            } else {
                start = if token.ty == OP_DUP_PLUS { 1 } else { 0 };
                end = if token.ty == OP_DUP_QUESTION { 1 } else { -1 };
            }
            *token = self.fetch_token();
            if elem == NONE {
                return Ok(NONE);
            }
            if start == 0 && end == 0 {
                return Ok(NONE);
            }
            let old_tree;
            let mut tree;
            // Extract "<re>{n,m}" to "<re><re>...<re><re>{0,<m-n>}".
            if start > 0 {
                tree = elem;
                let mut i = 2;
                while i <= start {
                    elem = self.duplicate_tree(elem);
                    tree = self.create_tree(tree, elem, CONCAT);
                    i += 1;
                }
                if start == end {
                    return Ok(tree);
                }
                // Duplicate ELEM before it is marked optional.
                elem = self.duplicate_tree(elem);
                old_tree = tree;
            } else {
                old_tree = NONE;
            }
            if self.tree[elem as usize].token.ty == SUBEXP {
                let idx = self.tree[elem as usize].token.idx;
                self.postorder(elem, &mut |c: &mut Compiler, n: i32| {
                    let t = &mut c.tree[n as usize].token;
                    if t.ty == SUBEXP && t.idx == idx {
                        t.opt_subexp = true;
                    }
                });
            }
            tree = self.create_tree(elem, NONE, if end == -1 { OP_DUP_ASTERISK } else { OP_ALT });
            // This loop is actually executed only when end != -1,
            // to rewrite <re>{0,n} as (<re>(<re>...<re>?)?)?...  We have
            // already created the start+1-th copy.
            let mut i = start + 2;
            while i <= end {
                elem = self.duplicate_tree(elem);
                tree = self.create_tree(tree, elem, CONCAT);
                tree = self.create_tree(tree, NONE, OP_ALT);
                i += 1;
            }
            if old_tree != NONE {
                tree = self.create_tree(old_tree, tree, CONCAT);
            }
            Ok(tree)
        }

        fn parse_bracket_exp(&mut self, _open: &mut Tok) -> Result<i32, Err> {
            let mut sbcset: Bitset = [0; 4];
            let mut non_match = false;
            let (mut token, mut token_len) = self.peek_token_bracket();
            if token.ty == END_OF_RE {
                return Err(REG_BADPAT);
            }
            if token.ty == OP_NON_MATCH_LIST {
                non_match = true;
                self.cur += token_len;
                (token, token_len) = self.peek_token_bracket();
                if token.ty == END_OF_RE {
                    return Err(REG_BADPAT);
                }
            }
            // We treat the first ']' as a normal character.
            if token.ty == OP_CLOSE_BRACKET {
                token.ty = CHARACTER;
            }
            let mut first_round = true;
            loop {
                let start_elem = self.parse_bracket_element(&token, token_len, first_round)?;
                first_round = false;
                // Get information about the next token.  We need it in any case.
                (token, token_len) = self.peek_token_bracket();
                let mut range_token = None;
                // Do not check for ranges if we know they are not allowed.
                if !matches!(start_elem, Elem::CharClass(_) | Elem::EquivClass(_)) {
                    if token.ty == END_OF_RE {
                        return Err(REG_EBRACK);
                    }
                    if token.ty == OP_CHARSET_RANGE {
                        self.cur += token_len; // skip '-'
                        let (token2, token_len2) = self.peek_token_bracket();
                        if token2.ty == END_OF_RE {
                            return Err(REG_EBRACK);
                        }
                        if token2.ty == OP_CLOSE_BRACKET {
                            // We treat the last '-' as a normal character.
                            self.cur -= token_len;
                            token.ty = CHARACTER;
                        } else {
                            range_token = Some((token2, token_len2));
                        }
                    }
                }
                if let Some((token2, token_len2)) = range_token {
                    let end_elem = self.parse_bracket_element(&token2, token_len2, true)?;
                    (token, token_len) = self.peek_token_bracket();
                    Self::build_range_exp(&mut sbcset, &start_elem, &end_elem)?;
                } else {
                    match &start_elem {
                        Elem::SbChar(c) => bit_set(&mut sbcset, *c),
                        Elem::EquivClass(name) | Elem::CollSym(name) => {
                            // build_equiv_class and build_collating_symbol, for nrules=0
                            if name.len() != 1 {
                                return Err(REG_ECOLLATE);
                            }
                            bit_set(&mut sbcset, name[0]);
                        }
                        Elem::CharClass(name) => Self::build_charclass(&mut sbcset, name, self.dfa.icase)?,
                    }
                }
                if token.ty == END_OF_RE {
                    return Err(REG_EBRACK);
                }
                if token.ty == OP_CLOSE_BRACKET {
                    break;
                }
            }
            self.cur += token_len; // skip a token
            if non_match {
                for w in sbcset.iter_mut() {
                    *w = !*w;
                }
            }
            let sbc = self.dfa.sets.len() as i32;
            self.dfa.sets.push(sbcset);
            Ok(self.create_token_tree(NONE, NONE, Tok { ty: SIMPLE_BRACKET, sbc, ..Tok::default() }))
        }

        fn parse_bracket_element(&mut self, token: &Tok, token_len: usize, accept_hyphen: bool) -> Result<Elem, Err> {
            self.cur += token_len; // skip a token
            if token.ty == OP_OPEN_COLL_ELEM || token.ty == OP_OPEN_CHAR_CLASS || token.ty == OP_OPEN_EQUIV_CLASS {
                return self.parse_bracket_symbol(token);
            }
            if token.ty == OP_CHARSET_RANGE && !accept_hyphen {
                // A '-' must only appear as anything but a range indicator before the closing bracket.
                let (token2, _) = self.peek_token_bracket();
                if token2.ty != OP_CLOSE_BRACKET {
                    return Err(REG_ERANGE);
                }
            }
            Ok(Elem::SbChar(token.c))
        }

        fn parse_bracket_symbol(&mut self, token: &Tok) -> Result<Elem, Err> {
            let delim = token.c;
            let mut name = Vec::new();
            if self.eoi() {
                return Err(REG_EBRACK);
            }
            loop {
                if name.len() >= BRACKET_NAME_BUF_SIZE {
                    return Err(REG_EBRACK);
                }
                // re_string_fetch_byte_case for a class name, re_string_fetch_byte otherwise
                let ch = if token.ty == OP_OPEN_CHAR_CLASS { self.raw[self.cur] } else { self.mbs[self.cur] };
                self.cur += 1;
                if self.eoi() {
                    return Err(REG_EBRACK);
                }
                if ch == delim && self.mbs[self.cur] == b']' {
                    break;
                }
                name.push(ch);
            }
            self.cur += 1;
            Ok(match token.ty {
                OP_OPEN_COLL_ELEM => Elem::CollSym(name),
                OP_OPEN_EQUIV_CLASS => Elem::EquivClass(name),
                _ => Elem::CharClass(name),
            })
        }

        /// lookup_collation_sequence_value with nrules=0: `collseqmb` is the identity.
        fn collseq(e: &Elem) -> Option<u32> {
            match e {
                Elem::SbChar(c) => Some(*c as u32),
                Elem::CollSym(name) if name.len() == 1 => Some(name[0] as u32),
                _ => None,
            }
        }

        fn build_range_exp(sbcset: &mut Bitset, start_elem: &Elem, end_elem: &Elem) -> Result<(), Err> {
            // Equivalence Classes and Character Classes can't be a range start/end.
            if matches!(start_elem, Elem::EquivClass(_) | Elem::CharClass(_))
                || matches!(end_elem, Elem::EquivClass(_) | Elem::CharClass(_))
            {
                return Err(REG_ERANGE);
            }
            let (Some(start), Some(end)) = (Self::collseq(start_elem), Self::collseq(end_elem)) else {
                return Err(REG_ECOLLATE);
            };
            if start > end {
                return Err(REG_ERANGE); // RE_NO_EMPTY_RANGES
            }
            for ch in start..=end {
                bit_set(sbcset, ch as u8);
            }
            Ok(())
        }

        fn build_charclass(sbcset: &mut Bitset, name: &[u8], icase: bool) -> Result<(), Err> {
            // In case of REG_ICASE "upper" and "lower" match the both of upper and lower cases.
            let name: &[u8] = if icase && (name == b"upper" || name == b"lower") { b"alpha" } else { name };
            let f: fn(u8) -> bool = match name {
                b"alnum" => |c| c.is_ascii_alphanumeric(),
                b"cntrl" => |c| c.is_ascii_control(),
                b"lower" => |c| c.is_ascii_lowercase(),
                b"space" => |c| c == b' ' || (9..=13).contains(&c),
                b"alpha" => |c| c.is_ascii_alphabetic(),
                b"digit" => |c| c.is_ascii_digit(),
                b"print" => |c| (32..=126).contains(&c),
                b"upper" => |c| c.is_ascii_uppercase(),
                b"blank" => |c| c == b' ' || c == 9,
                b"graph" => |c| (33..=126).contains(&c),
                b"punct" => |c| c.is_ascii_punctuation(),
                b"xdigit" => |c| c.is_ascii_hexdigit(),
                _ => return Err(REG_ECTYPE),
            };
            for i in 0..=255u8 {
                if f(i) {
                    bit_set(sbcset, i);
                }
            }
            Ok(())
        }

        fn build_charclass_op(&mut self, class_name: &[u8], extra: &[u8], non_match: bool) -> Result<i32, Err> {
            let mut sbcset: Bitset = [0; 4];
            // We don't care the syntax in this case.
            Self::build_charclass(&mut sbcset, class_name, false)?;
            for &c in extra {
                bit_set(&mut sbcset, c); // \w match '_' also
            }
            if non_match {
                for w in sbcset.iter_mut() {
                    *w = !*w;
                }
            }
            let sbc = self.dfa.sets.len() as i32;
            self.dfa.sets.push(sbcset);
            Ok(self.create_token_tree(NONE, NONE, Tok { ty: SIMPLE_BRACKET, sbc, ..Tok::default() }))
        }

        fn init_word_char(&mut self) {
            self.dfa.word_ops_used = true;
            for ch in 0..=255u8 {
                if is_word_char(ch) {
                    bit_set(&mut self.dfa.word_char, ch);
                }
            }
        }

        /// Our parse trees are very unbalanced, so these two use parent pointers, as the C does.
        fn postorder(&mut self, root: i32, f: &mut dyn FnMut(&mut Self, i32)) {
            let mut node = root;
            loop {
                // Descend down the tree, preferably to the left (or to the right if that's the only child).
                loop {
                    let b = self.tree[node as usize];
                    if b.left != NONE {
                        node = b.left;
                    } else if b.right != NONE {
                        node = b.right;
                    } else {
                        break;
                    }
                }
                loop {
                    f(self, node);
                    let parent = self.tree[node as usize].parent;
                    if parent == NONE {
                        return;
                    }
                    let prev = node;
                    node = parent;
                    // Go up while we have a node that is reached from the right.
                    let r = self.tree[node as usize].right;
                    if !(r == prev || r == NONE) {
                        break;
                    }
                }
                node = self.tree[node as usize].right;
            }
        }

        fn preorder(&mut self, root: i32, f: &mut dyn FnMut(&mut Self, i32)) {
            let mut node = root;
            loop {
                f(self, node);
                // Go to the left node, or up and to the right.
                if self.tree[node as usize].left != NONE {
                    node = self.tree[node as usize].left;
                } else {
                    let mut prev = NONE;
                    loop {
                        let r = self.tree[node as usize].right;
                        if !(r == prev || r == NONE) {
                            break;
                        }
                        prev = node;
                        node = self.tree[node as usize].parent;
                        if node == NONE {
                            return;
                        }
                    }
                    node = self.tree[node as usize].right;
                }
            }
        }

        fn duplicate_tree(&mut self, root: i32) -> i32 {
            #[derive(Clone, Copy)]
            enum Slot {
                Root,
                Left(i32),
                Right(i32),
            }
            let mut dup_root = NONE;
            let mut p_new = Slot::Root;
            let mut dup_node = self.tree[root as usize].parent;
            let mut node = root;
            loop {
                // Create a new tree and link it back to the current parent.
                let tok = self.tree[node as usize].token;
                let nn = self.create_token_tree(NONE, NONE, tok);
                match p_new {
                    Slot::Root => dup_root = nn,
                    Slot::Left(x) => self.tree[x as usize].left = nn,
                    Slot::Right(x) => self.tree[x as usize].right = nn,
                }
                self.tree[nn as usize].parent = dup_node;
                self.tree[nn as usize].token.duplicated = true;
                dup_node = nn;
                // Go to the left node, or up and to the right.
                if self.tree[node as usize].left != NONE {
                    node = self.tree[node as usize].left;
                    p_new = Slot::Left(dup_node);
                } else {
                    let mut prev = NONE;
                    loop {
                        let r = self.tree[node as usize].right;
                        if !(r == prev || r == NONE) {
                            break;
                        }
                        prev = node;
                        node = self.tree[node as usize].parent;
                        dup_node = self.tree[dup_node as usize].parent;
                        if node == NONE {
                            return dup_root;
                        }
                    }
                    node = self.tree[node as usize].right;
                    p_new = Slot::Right(dup_node);
                }
            }
        }

        /// Optimization pass: if a SUBEXP is entirely contained, strip it and tell
        /// re_search_internal to map the inner one's opr.idx to this one's.
        fn optimize_subexps(&mut self, node: i32) {
            let n = self.tree[node as usize];
            if n.token.ty == OP_BACK_REF {
                if let Some(map) = &self.dfa.subexp_map {
                    let idx = map[n.token.idx as usize];
                    self.tree[node as usize].token.idx = idx;
                    self.dfa.used_bkref_map |= 1 << idx;
                }
            } else if n.token.ty == SUBEXP && n.left != NONE && self.tree[n.left as usize].token.ty == SUBEXP {
                let inner = self.tree[n.left as usize];
                let other_idx = inner.token.idx;
                self.tree[node as usize].left = inner.left;
                if inner.left != NONE {
                    self.tree[inner.left as usize].parent = node;
                }
                let map = self.dfa.subexp_map.as_mut().unwrap();
                map[other_idx as usize] = map[n.token.idx as usize];
                if other_idx < 64 {
                    self.dfa.used_bkref_map &= !(1u64 << other_idx);
                }
            }
        }

        /// Lowering pass: Turn each SUBEXP node into the appropriate concatenation
        /// of OP_OPEN_SUBEXP, the body of the SUBEXP (if any) and OP_CLOSE_SUBEXP.
        fn lower_subexps(&mut self, node: i32) {
            let n = self.tree[node as usize];
            if n.left != NONE && self.tree[n.left as usize].token.ty == SUBEXP {
                let l = self.lower_subexp(n.left);
                self.tree[node as usize].left = l;
                self.tree[l as usize].parent = node;
            }
            if n.right != NONE && self.tree[n.right as usize].token.ty == SUBEXP {
                let r = self.lower_subexp(n.right);
                self.tree[node as usize].right = r;
                self.tree[r as usize].parent = node;
            }
        }

        fn lower_subexp(&mut self, node: i32) -> i32 {
            let n = self.tree[node as usize];
            let op = self.create_tree(NONE, NONE, OP_OPEN_SUBEXP);
            let cls = self.create_tree(NONE, NONE, OP_CLOSE_SUBEXP);
            let tree1 = if n.left != NONE { self.create_tree(n.left, cls, CONCAT) } else { cls };
            let tree = self.create_tree(op, tree1, CONCAT);
            self.tree[op as usize].token.idx = n.token.idx;
            self.tree[cls as usize].token.idx = n.token.idx;
            self.tree[op as usize].token.opt_subexp = n.token.opt_subexp;
            self.tree[cls as usize].token.opt_subexp = n.token.opt_subexp;
            tree
        }

        fn re_dfa_add_node(&mut self, token: Tok) -> i32 {
            let d = &mut self.dfa;
            let n = d.nodes.len() as i32;
            let mut t = token;
            t.constraint = 0;
            d.nodes.push(t);
            d.nexts.push(-1);
            d.org_indices.push(-1);
            d.edests.push(Vec::new());
            d.eclosures.push(Vec::new());
            d.ecl_state.push(0);
            n
        }

        /// Pass 1 in building the NFA: compute FIRST and create unlinked automaton nodes.
        fn calc_first(&mut self, node: i32) {
            let n = self.tree[node as usize];
            if n.token.ty == CONCAT {
                let l = self.tree[n.left as usize];
                self.tree[node as usize].first = l.first;
                self.tree[node as usize].node_idx = l.node_idx;
            } else {
                self.tree[node as usize].first = node;
                let idx = self.re_dfa_add_node(n.token);
                self.tree[node as usize].node_idx = idx;
                if n.token.ty == ANCHOR {
                    self.dfa.nodes[idx as usize].constraint = n.token.ctx;
                }
            }
        }

        /// Pass 2: compute NEXT on the tree.  Preorder visit.
        fn calc_next(&mut self, node: i32) {
            let n = self.tree[node as usize];
            match n.token.ty {
                OP_DUP_ASTERISK => self.tree[n.left as usize].next = node,
                CONCAT => {
                    self.tree[n.left as usize].next = self.tree[n.right as usize].first;
                    self.tree[n.right as usize].next = n.next;
                }
                _ => {
                    if n.left != NONE {
                        self.tree[n.left as usize].next = n.next;
                    }
                    if n.right != NONE {
                        self.tree[n.right as usize].next = n.next;
                    }
                }
            }
        }

        /// Pass 3: link all DFA nodes to their NEXT node (any order will do).
        fn link_nfa_nodes(&mut self, node: i32) {
            let n = self.tree[node as usize];
            let idx = n.node_idx as usize;
            match n.token.ty {
                CONCAT | END_OF_RE => {}
                OP_DUP_ASTERISK | OP_ALT => {
                    self.dfa.has_plural_match = true;
                    let idx_of = |c: &Self, t: i32| c.tree[t as usize].node_idx;
                    let left = if n.left != NONE {
                        let f = self.tree[n.left as usize].first;
                        idx_of(self, f)
                    } else {
                        idx_of(self, n.next)
                    };
                    let right = if n.right != NONE {
                        let f = self.tree[n.right as usize].first;
                        idx_of(self, f)
                    } else {
                        idx_of(self, n.next)
                    };
                    // re_node_set_init_2: in order of node number, and only one if they are the same
                    self.dfa.edests[idx] = if left == right {
                        vec![left]
                    } else if left < right {
                        vec![left, right]
                    } else {
                        vec![right, left]
                    };
                }
                ANCHOR | OP_OPEN_SUBEXP | OP_CLOSE_SUBEXP => {
                    self.dfa.edests[idx] = vec![self.tree[n.next as usize].node_idx];
                }
                OP_BACK_REF => {
                    let d = self.tree[n.next as usize].node_idx;
                    self.dfa.nexts[idx] = d;
                    self.dfa.edests[idx] = vec![d];
                }
                _ => self.dfa.nexts[idx] = self.tree[n.next as usize].node_idx,
            }
        }

        fn duplicate_node(&mut self, org_idx: i32, constraint: u32) -> i32 {
            let tok = self.dfa.nodes[org_idx as usize];
            let dup_idx = self.re_dfa_add_node(tok);
            let d = &mut self.dfa;
            d.nodes[dup_idx as usize].constraint = constraint | tok.constraint;
            d.nodes[dup_idx as usize].duplicated = true;
            d.org_indices[dup_idx as usize] = org_idx;
            dup_idx
        }

        fn search_duplicated_node(&self, org_node: i32, constraint: u32) -> i32 {
            let d = &self.dfa;
            let mut idx = d.nodes.len() as i32 - 1;
            while d.nodes[idx as usize].duplicated && idx > 0 {
                if org_node == d.org_indices[idx as usize] && constraint == d.nodes[idx as usize].constraint {
                    return idx;
                }
                idx -= 1;
            }
            -1
        }

        /// Duplicate the epsilon closure of the node ROOT_NODE.
        /// Note that duplicated nodes have constraint INIT_CONSTRAINT in addition to their own.
        fn duplicate_node_closure(&mut self, top_org_node: i32, top_clone_node: i32, root_node: i32, init_constraint: u32) {
            let mut constraint = init_constraint;
            let (mut org_node, mut clone_node) = (top_org_node, top_clone_node);
            loop {
                let org_dest;
                let clone_dest;
                let (o, c) = (org_node as usize, clone_node as usize);
                if self.dfa.nodes[o].ty == OP_BACK_REF {
                    // If the back reference epsilon-transit, its destination must also have the
                    // constraint.  Then duplicate the epsilon closure of the destination of the
                    // back reference, and store it in edests of the back reference.
                    org_dest = self.dfa.nexts[o];
                    self.dfa.edests[c].clear();
                    clone_dest = self.duplicate_node(org_dest, constraint);
                    self.dfa.nexts[c] = self.dfa.nexts[o];
                    set_insert(&mut self.dfa.edests[c], clone_dest);
                } else if self.dfa.edests[o].is_empty() {
                    // In case of the node can't epsilon-transit, don't duplicate the destination
                    // and store the original destination as the destination of the node.
                    self.dfa.nexts[c] = self.dfa.nexts[o];
                    break;
                } else if self.dfa.edests[o].len() == 1 {
                    // In case of the node can epsilon-transit, and it has only one destination.
                    org_dest = self.dfa.edests[o][0];
                    self.dfa.edests[c].clear();
                    // If the node is root_node itself, it means the epsilon closure has a loop.
                    // Then tie it to the destination of the root_node.
                    if org_node == root_node && clone_node != org_node {
                        set_insert(&mut self.dfa.edests[c], org_dest);
                        break;
                    }
                    // In case the node has another constraint, append it.
                    constraint |= self.dfa.nodes[o].constraint;
                    clone_dest = self.duplicate_node(org_dest, constraint);
                    set_insert(&mut self.dfa.edests[c], clone_dest);
                } else {
                    // In case of the node can epsilon-transit, and it has two destinations.
                    // In the bin_tree_t and DFA, that's '|' and '*'.
                    let first_dest = self.dfa.edests[o][0];
                    self.dfa.edests[c].clear();
                    // Search for a duplicated node which satisfies the constraint.
                    let found = self.search_duplicated_node(first_dest, constraint);
                    if found == -1 {
                        // There is no such duplicated node, create a new one.
                        let d = self.duplicate_node(first_dest, constraint);
                        set_insert(&mut self.dfa.edests[c], d);
                        self.duplicate_node_closure(first_dest, d, root_node, constraint);
                    } else {
                        // There is a duplicated node which satisfies the constraint.
                        // Use it to avoid infinite loop.
                        set_insert(&mut self.dfa.edests[c], found);
                    }
                    org_dest = self.dfa.edests[o][1];
                    clone_dest = self.duplicate_node(org_dest, constraint);
                    set_insert(&mut self.dfa.edests[c], clone_dest);
                }
                org_node = org_dest;
                clone_node = clone_dest;
            }
        }

        /// Calculate epsilon closure of NODE.
        fn calc_eclosure_iter(&mut self, node: i32, root: bool) -> NodeSet {
            let n = node as usize;
            let mut eclosure = Vec::new();
            let mut incomplete = false;
            // This indicates that we are calculating this node now.
            self.dfa.ecl_state[n] = -1;
            // If the current node has constraints, duplicate all nodes
            // since they must inherit the constraints.
            if self.dfa.nodes[n].constraint != 0
                && !self.dfa.edests[n].is_empty()
                && !self.dfa.nodes[self.dfa.edests[n][0] as usize].duplicated
            {
                let c = self.dfa.nodes[n].constraint;
                self.duplicate_node_closure(node, node, node, c);
            }
            // Expand each epsilon destination nodes.
            if is_epsilon(self.dfa.nodes[n].ty) {
                let mut i = 0;
                while i < self.dfa.edests[n].len() {
                    let edest = self.dfa.edests[n][i];
                    i += 1;
                    // If calculating the epsilon closure of `edest' is in progress,
                    // return intermediate result.
                    let st = self.dfa.ecl_state[edest as usize];
                    if st == -1 {
                        incomplete = true;
                        continue;
                    }
                    let elem = if st == 0 {
                        self.calc_eclosure_iter(edest, false)
                    } else {
                        self.dfa.eclosures[edest as usize].clone()
                    };
                    set_merge(&mut eclosure, &elem);
                    // If the epsilon closure of `edest' is incomplete,
                    // the epsilon closure of this node is also incomplete.
                    if self.dfa.ecl_state[edest as usize] == 0 {
                        incomplete = true;
                    }
                }
            }
            // Epsilon closures include itself.
            set_insert(&mut eclosure, node);
            if incomplete && !root {
                self.dfa.ecl_state[n] = 0;
            } else {
                self.dfa.ecl_state[n] = 1;
                self.dfa.eclosures[n] = eclosure.clone();
            }
            eclosure
        }

        fn calc_eclosure(&mut self) {
            let mut incomplete = false;
            let mut node_idx = 0;
            loop {
                if node_idx == self.dfa.nodes.len() {
                    if !incomplete {
                        break;
                    }
                    incomplete = false;
                    node_idx = 0;
                }
                // If we have already calculated, skip it.
                if self.dfa.ecl_state[node_idx] != 0 {
                    node_idx += 1;
                    continue;
                }
                // Calculate epsilon closure of `node_idx'.
                let _ = self.calc_eclosure_iter(node_idx as i32, true);
                if self.dfa.ecl_state[node_idx] == 0 {
                    incomplete = true;
                }
                node_idx += 1;
            }
        }

        fn calc_inveclosure(&mut self) {
            let d = &mut self.dfa;
            d.inveclosures = vec![Vec::new(); d.nodes.len()];
            for src in 0..d.nodes.len() {
                for &e in &d.eclosures[src] {
                    d.inveclosures[e as usize].push(src as i32);
                }
            }
        }

        fn analyze(&mut self, root: i32) {
            if self.dfa.re_nsub > 0 {
                self.dfa.subexp_map = Some((0..self.dfa.re_nsub).collect());
                self.preorder(root, &mut |c: &mut Compiler, n: i32| c.optimize_subexps(n));
                let map = self.dfa.subexp_map.as_ref().unwrap();
                if map.iter().enumerate().all(|(i, &m)| m == i as i32) {
                    self.dfa.subexp_map = None;
                }
            }
            self.postorder(root, &mut |c: &mut Compiler, n: i32| c.lower_subexps(n));
            self.postorder(root, &mut |c: &mut Compiler, n: i32| c.calc_first(n));
            self.preorder(root, &mut |c: &mut Compiler, n: i32| c.calc_next(n));
            self.preorder(root, &mut |c: &mut Compiler, n: i32| c.link_nfa_nodes(n));
            self.calc_eclosure();
            // We only need this during the prune_impossible_nodes pass in regexec.c;
            // skip it if p_i_n will not run, as calc_inveclosure can be quadratic.
            if (self.dfa.re_nsub > 0 && self.dfa.has_plural_match) || self.dfa.nbackref != 0 {
                self.calc_inveclosure();
            }
        }

        /// create_initial_state, up to the states themselves: those are made at each start.
        fn create_initial_state(&mut self, root: i32) {
            let d = &mut self.dfa;
            // Initial states have the epsilon closure of the node which is
            // the first node of the regular expression.
            let first = self.tree[root as usize].first;
            let first = self.tree[first as usize].node_idx;
            d.init_node = first;
            let mut init_nodes = d.eclosures[first as usize].clone();
            // The back-references which are in initial states can epsilon transit,
            // since in this case all of the subexpressions can be null.
            // Then we add epsilon closures of the nodes which are the next nodes of the back-references.
            if d.nbackref > 0 {
                let mut i = 0;
                while i < init_nodes.len() {
                    let node_idx = init_nodes[i] as usize;
                    if d.nodes[node_idx].ty == OP_BACK_REF
                        && init_nodes.iter().any(|&n| {
                            let c = &d.nodes[n as usize];
                            c.ty == OP_CLOSE_SUBEXP && c.idx == d.nodes[node_idx].idx
                        })
                    {
                        let dest_idx = d.edests[node_idx][0];
                        if !set_contains(&init_nodes, dest_idx) {
                            set_merge(&mut init_nodes, &d.eclosures[dest_idx as usize]);
                            i = 0;
                        }
                    }
                    i += 1;
                }
            }
            d.init_nodes = init_nodes;
        }
    }

    impl Regex {
        /// `regcomp(preg, pattern, REG_EXTENDED | (icase ? REG_ICASE : 0))`; an error is its
        /// `regerror` message.
        pub fn compile(pattern: &[u8], icase: bool) -> Result<Regex, Err> {
            let mbs = if icase { pattern.to_ascii_uppercase() } else { pattern.to_vec() };
            let mut c = Compiler {
                raw: pattern,
                mbs,
                cur: 0,
                tree: Vec::new(),
                dfa: Regex {
                    nodes: Vec::new(),
                    nexts: Vec::new(),
                    org_indices: Vec::new(),
                    edests: Vec::new(),
                    eclosures: Vec::new(),
                    ecl_state: Vec::new(),
                    inveclosures: Vec::new(),
                    sets: Vec::new(),
                    word_char: [0; 4],
                    word_ops_used: false,
                    nbackref: 0,
                    has_plural_match: false,
                    has_mb_node: false,
                    used_bkref_map: 0,
                    completed_bkref_map: 0,
                    subexp_map: None,
                    init_node: 0,
                    init_nodes: Vec::new(),
                    re_nsub: 0,
                    icase,
                },
            };
            let root = c.parse()?;
            c.analyze(root);
            c.create_initial_state(root);
            Ok(c.dfa)
        }

        /// re_acquire_state: a state independent of the context.
        fn acquire(&self, nodes: &[i32]) -> StateRef {
            if nodes.is_empty() {
                return None;
            }
            let mut st = State {
                nodes: nodes.to_vec(),
                non_eps_nodes: Vec::new(),
                entrance: None,
                context: 0,
                halt: false,
                has_backref: false,
                has_constraint: false,
            };
            for &n in nodes {
                let node = &self.nodes[n as usize];
                if node.ty == CHARACTER && node.constraint == 0 {
                    continue;
                }
                if node.ty == END_OF_RE {
                    st.halt = true;
                } else if node.ty == OP_BACK_REF {
                    st.has_backref = true;
                } else if node.ty == ANCHOR || node.constraint != 0 {
                    st.has_constraint = true;
                }
            }
            self.register_state(st)
        }

        /// re_acquire_state_context: a state depending on the context.
        fn acquire_context(&self, nodes: &[i32], context: u32) -> StateRef {
            if nodes.is_empty() {
                return None;
            }
            let mut st = State {
                nodes: nodes.to_vec(),
                non_eps_nodes: Vec::new(),
                entrance: None,
                context,
                halt: false,
                has_backref: false,
                has_constraint: false,
            };
            let mut nctx_nodes = 0;
            for (i, &n) in nodes.iter().enumerate() {
                let node = &self.nodes[n as usize];
                let constraint = node.constraint;
                if node.ty == CHARACTER && constraint == 0 {
                    continue;
                }
                if node.ty == END_OF_RE {
                    st.halt = true;
                } else if node.ty == OP_BACK_REF {
                    st.has_backref = true;
                }
                if constraint != 0 {
                    if st.entrance.is_none() {
                        st.entrance = Some(nodes.to_vec());
                        nctx_nodes = 0;
                        st.has_constraint = true;
                    }
                    if not_satisfy_prev(constraint, context) {
                        st.nodes.remove(i - nctx_nodes);
                        nctx_nodes += 1;
                    }
                }
            }
            self.register_state(st)
        }

        fn register_state(&self, mut st: State) -> StateRef {
            st.non_eps_nodes = st.nodes.iter().copied().filter(|&e| !is_epsilon(self.nodes[e as usize].ty)).collect();
            Some(Rc::new(st))
        }

        /// What group_nodes_into_DFAstates decides for one node and one character.
        #[inline]
        fn transition_accepts(&self, n: i32, ch: u8) -> bool {
            let node = &self.nodes[n as usize];
            let accepts = match node.ty {
                CHARACTER => node.c == ch,
                SIMPLE_BRACKET => bit_contain(&self.sets[node.sbc as usize], ch),
                OP_PERIOD => ch != 0, // RE_DOT_NEWLINE is set, and RE_DOT_NOT_NULL
                _ => false,
            };
            if !accepts {
                return false;
            }
            let constraint = node.constraint;
            if constraint != 0 {
                if constraint & NEXT_NEWLINE_CONSTRAINT != 0 && ch != b'\n' {
                    return false;
                }
                if constraint & NEXT_ENDBUF_CONSTRAINT != 0 {
                    return false;
                }
                if constraint & NEXT_WORD_CONSTRAINT != 0
                    && ((node.ty == CHARACTER && !node.word_char) || !bit_contain(&self.word_char, ch))
                {
                    return false;
                }
                if constraint & NEXT_NOTWORD_CONSTRAINT != 0
                    && ((node.ty == CHARACTER && node.word_char) || bit_contain(&self.word_char, ch))
                {
                    return false;
                }
            }
            true
        }

        /// `regexec(preg, string, nmatch, pmatch, 0)`: `string` holds no NUL.  `None` is REG_NOMATCH;
        /// the result has the entries re_search_internal fills itself, at most `re_nsub+1`.
        pub fn exec(&self, string: &[u8], nmatch: usize) -> Option<Vec<(i32, i32)>> {
            let nsub = self.re_nsub as usize;
            let extra_nmatch = if nmatch > nsub { nmatch - (nsub + 1) } else { 0 };
            let nmatch = nmatch - extra_nmatch;
            let upper;
            let buf: &[u8] = if self.icase {
                upper = string.to_ascii_uppercase();
                &upper
            } else {
                string
            };
            // We must check the longest matching, if nmatch > 0.
            let fl_longest_match = nmatch != 0 || self.nbackref != 0;
            let log = nmatch > 1 || self.has_mb_node;
            let mut mctx = Mctx {
                dfa: self,
                input: buf,
                tip_context: CONTEXT_NEWLINE | CONTEXT_BEGBUF,
                cur_idx: 0,
                state_log: if log { vec![None; buf.len() + 2] } else { Vec::new() },
                log,
                state_log_top: 0,
                match_last: -1,
                last_node: 0,
                bkref_ents: Vec::new(),
                sub_tops: Vec::new(),
                max_mb_elem_len: 0,
            };
            // If initial states with non-begbuf contexts have no elements, the regex must be anchored.
            let empty_from = |context: u32| self.acquire_context(&self.init_nodes, context).is_none_or(|s| s.nodes.is_empty());
            let right_lim = if empty_from(0) && empty_from(CONTEXT_WORD) { 0 } else { buf.len() };
            let mut match_first = 0;
            loop {
                if match_first > right_lim {
                    return None;
                }
                // re_string_reconstruct
                mctx.input = &buf[match_first..];
                mctx.tip_context = if match_first == 0 {
                    CONTEXT_NEWLINE | CONTEXT_BEGBUF
                } else if bit_contain(&self.word_char, buf[match_first - 1]) {
                    CONTEXT_WORD
                } else {
                    0 // no REG_NEWLINE, so newline_anchor is 0
                };
                mctx.cur_idx = 0;
                mctx.state_log_top = 0;
                mctx.bkref_ents.clear();
                mctx.max_mb_elem_len = 0;
                let match_last = mctx.check_matching(fl_longest_match, &mut match_first);
                if match_last != -1 {
                    mctx.match_last = match_last;
                    if nmatch > 1 || self.nbackref != 0 {
                        let pstate = mctx.state_log[match_last as usize].clone().unwrap();
                        mctx.last_node = mctx.check_halt_state_context(&pstate, match_last);
                    }
                    if (nmatch > 1 && self.has_plural_match) || self.nbackref != 0 {
                        if mctx.prune_impossible_nodes() {
                            break;
                        }
                    } else {
                        break; // we found a match
                    }
                }
                // match_ctx_clean
                mctx.sub_tops.clear();
                mctx.bkref_ents.clear();
                match_first += 1;
            }
            // Set pmatch[] if we need.
            let mut pmatch = vec![(-1, -1); nmatch];
            if nmatch > 0 {
                // Set the points where matching start/end.
                pmatch[0] = (0, mctx.match_last);
                if nmatch > 1 && !mctx.set_regs(nmatch, &mut pmatch, self.has_plural_match && self.nbackref > 0) {
                    return None;
                }
                // At last, add the offset to each register, since we slid the buffers
                // so that we could assume that the matching starts from 0.
                for r in pmatch.iter_mut() {
                    if r.0 != -1 {
                        r.0 += match_first as i32;
                        r.1 += match_first as i32;
                    }
                }
                if let Some(map) = &self.subexp_map {
                    let mut reg_idx = 0;
                    while reg_idx + 1 < nmatch {
                        if map[reg_idx] != reg_idx as i32 {
                            pmatch[reg_idx + 1] = pmatch[map[reg_idx] as usize + 1];
                        }
                        reg_idx += 1;
                    }
                }
            }
            Some(pmatch)
        }
    }

    /// state_array_t
    #[derive(Default)]
    struct Path {
        array: Vec<StateRef>,
        alloc: i32,
        next_idx: i32,
    }

    /// re_sub_match_last_t
    struct SubLast {
        node: i32,
        str_idx: i32,
        path: Path,
    }

    /// re_sub_match_top_t
    struct SubTop {
        str_idx: i32,
        node: i32,
        path: Path,
        lasts: Vec<SubLast>,
    }

    /// struct re_backref_cache_entry
    #[derive(Clone, Copy)]
    struct BkrefEnt {
        node: i32,
        str_idx: i32,
        subexp_from: i32,
        subexp_to: i32,
        more: bool,
        eps_reachable_subexps_map: u64,
    }

    /// What every re_sift_context_t of one prune shares: the two arrays it points to.
    struct SiftArrays {
        sifted: Vec<StateRef>,
        limited: Option<Vec<StateRef>>,
    }

    /// The rest of a re_sift_context_t.
    #[derive(Clone)]
    struct Sift {
        last_node: i32,
        last_str_idx: i32,
        limits: NodeSet,
    }

    /// struct re_fail_stack_ent_t
    struct FailEnt {
        idx: i32,
        node: i32,
        regs: Vec<(i32, i32)>,
        prevregs: Vec<(i32, i32)>,
        eps_via_nodes: NodeSet,
    }

    /// re_match_context_t, with the re_string_t of the subject.  `input` starts at match_first.
    struct Mctx<'a> {
        dfa: &'a Regex,
        input: &'a [u8],
        tip_context: u32,
        cur_idx: i32,
        state_log: Vec<StateRef>,
        log: bool,
        state_log_top: i32,
        match_last: i32,
        last_node: i32,
        bkref_ents: Vec<BkrefEnt>,
        sub_tops: Vec<SubTop>,
        max_mb_elem_len: i32,
    }

    impl<'a> Mctx<'a> {
        #[inline]
        fn len(&self) -> i32 {
            self.input.len() as i32
        }

        /// re_string_byte_at; the C buffer ends in NUL.
        #[inline]
        fn byte_at(&self, idx: i32) -> u8 {
            if (idx as usize) < self.input.len() {
                self.input[idx as usize]
            } else {
                0
            }
        }

        /// re_string_context_at
        fn context_at(&self, idx: i32) -> u32 {
            if idx < 0 {
                return self.tip_context;
            }
            if idx == self.len() {
                return CONTEXT_NEWLINE | CONTEXT_ENDBUF;
            }
            if bit_contain(&self.dfa.word_char, self.byte_at(idx)) {
                return CONTEXT_WORD;
            }
            0 // no REG_NEWLINE, so newline_anchor is 0
        }

        /// The nodes of `state_log[idx]`, or none.
        fn log_nodes(&self, idx: i32) -> Vec<i32> {
            match self.state_log.get(idx as usize) {
                Some(Some(s)) => s.nodes.clone(),
                _ => Vec::new(),
            }
        }

        fn log_contains(&self, idx: i32, node: i32) -> bool {
            match self.state_log.get(idx as usize) {
                Some(Some(s)) => set_contains(&s.nodes, node),
                _ => false,
            }
        }

        /// P_MATCH_FIRST is moved on to the next place worth trying when there is no match.
        fn check_matching(&mut self, fl_longest_match: bool, p_match_first: &mut usize) -> i32 {
            let dfa = self.dfa;
            let mut matched = false;
            let mut match_last = -1;
            let cur_str_idx = self.cur_idx;
            let mut at_init_state = true;
            let mut move_match_first = true;
            let mut next_start_idx = cur_str_idx;
            let context = self.context_at(cur_str_idx - 1);
            // acquire_init_state_context: an initial state must not be NULL
            let mut cur_state = dfa.acquire_context(&dfa.init_nodes, context).unwrap();
            if self.log {
                self.state_log[cur_str_idx as usize] = Some(cur_state.clone());
                // Check OP_OPEN_SUBEXP in the initial state in case that we use them later.
                // E.g. Processing back references.
                if dfa.nbackref != 0 {
                    at_init_state = false;
                    self.check_subexp_matching_top(&cur_state.nodes, 0);
                    if cur_state.has_backref {
                        self.transit_state_bkref(&cur_state.nodes);
                    }
                }
            }
            // If the RE accepts NULL string.
            if cur_state.halt
                && (!cur_state.has_constraint || self.check_halt_state_context(&cur_state, cur_str_idx) != 0)
            {
                if !fl_longest_match {
                    return cur_str_idx;
                }
                match_last = cur_str_idx;
                matched = true;
            }
            while self.cur_idx < self.len() {
                let old_state = cur_state.clone();
                let next_char_idx = self.cur_idx + 1;
                let next = self.transit_state(&cur_state);
                let next = if self.log { self.merge_state_with_log(next) } else { next };
                cur_state = match next {
                    Some(s) => s,
                    None => {
                        // Reached the invalid state or an error.  Try to recover a valid state using
                        // the state log, if available and if we have not already found a valid
                        // (even if not the longest) match.
                        if !self.log || (matched && !fl_longest_match) {
                            break;
                        }
                        match self.find_recover_state() {
                            Some(s) => s,
                            None => break,
                        }
                    }
                };
                if at_init_state {
                    if old_state.same_as(&cur_state) {
                        next_start_idx = next_char_idx;
                    } else {
                        at_init_state = false;
                    }
                }
                if cur_state.halt {
                    // Reached a halt state.  Check the halt state can satisfy the current context.
                    if !cur_state.has_constraint || self.check_halt_state_context(&cur_state, self.cur_idx) != 0 {
                        // We found an appropriate halt state.
                        match_last = self.cur_idx;
                        matched = true;
                        // We found a match, do not modify match_first below.
                        move_match_first = false;
                        if !fl_longest_match {
                            break;
                        }
                    }
                }
            }
            if move_match_first {
                *p_match_first += next_start_idx as usize;
            }
            match_last
        }

        /// Check NODE match the current context.
        fn check_halt_node_context(&self, node: i32, context: u32) -> bool {
            let t = &self.dfa.nodes[node as usize];
            if t.ty != END_OF_RE {
                return false;
            }
            t.constraint == 0 || !not_satisfy_next(t.constraint, context)
        }

        /// Return 0 if not match; if the node STATE has is a halt node and matches the context,
        /// return the node.
        fn check_halt_state_context(&self, state: &State, idx: i32) -> i32 {
            let context = self.context_at(idx);
            for &n in &state.nodes {
                if self.check_halt_node_context(n, context) {
                    return n;
                }
            }
            0
        }

        /// transit_state and its transition table: re_string_fetch_byte, then what the table holds
        /// for that byte.
        fn transit_state(&mut self, state: &State) -> StateRef {
            let dfa = self.dfa;
            let ch = self.input[self.cur_idx as usize];
            self.cur_idx += 1;
            let mut follows: NodeSet = Vec::new();
            for &n in &state.nodes {
                if dfa.transition_accepts(n, ch) {
                    let next_node = dfa.nexts[n as usize];
                    if next_node != -1 {
                        set_merge(&mut follows, &dfa.eclosures[next_node as usize]);
                    }
                }
            }
            // build_trtable chooses by the character itself, and takes the newline's context for
            // '\n' even without REG_NEWLINE; re_string_context_at does not.
            let context = if bit_contain(&dfa.word_char, ch) {
                CONTEXT_WORD
            } else if ch == b'\n' {
                CONTEXT_NEWLINE
            } else {
                0
            };
            dfa.acquire_context(&follows, context)
        }

        /// Update the state_log if we need.
        fn merge_state_with_log(&mut self, next_state: StateRef) -> StateRef {
            let dfa = self.dfa;
            let cur_idx = self.cur_idx;
            let mut next_state = next_state;
            if cur_idx > self.state_log_top {
                self.state_log[cur_idx as usize] = next_state.clone();
                self.state_log_top = cur_idx;
            } else if self.state_log[cur_idx as usize].is_none() {
                self.state_log[cur_idx as usize] = next_state.clone();
            } else {
                // If (state_log[cur_idx] != 0), it implies that cur_idx is the destination of a
                // back reference.  Then the next state is the union set of these destinations and
                // the results of the transition table.
                let pstate = self.state_log[cur_idx as usize].clone().unwrap();
                let mut next_nodes = pstate.entrance_nodes().clone();
                if let Some(ns) = &next_state {
                    set_merge(&mut next_nodes, ns.entrance_nodes());
                }
                // Note: We already add the nodes of the initial state, then we don't need to add
                // them here.
                let context = self.context_at(self.cur_idx - 1);
                next_state = dfa.acquire_context(&next_nodes, context);
                self.state_log[cur_idx as usize] = next_state.clone();
            }
            if dfa.nbackref != 0 {
                if let Some(ns) = next_state.clone() {
                    // Check OP_OPEN_SUBEXP in the current state in case that we use them later.
                    // We must check them here, since the back references in the next state might
                    // use them.
                    self.check_subexp_matching_top(&ns.nodes, cur_idx);
                    // If the next state has back references.
                    if ns.has_backref {
                        self.transit_state_bkref(&ns.nodes);
                        next_state = self.state_log[cur_idx as usize].clone();
                    }
                }
            }
            next_state
        }

        /// Skip bytes in the input that correspond to part of a multi-byte match, then look in the
        /// log for a state from which to restart matching.
        fn find_recover_state(&mut self) -> StateRef {
            loop {
                let max = self.state_log_top;
                let mut cur_str_idx = self.cur_idx;
                loop {
                    cur_str_idx += 1;
                    if cur_str_idx > max {
                        return None;
                    }
                    self.cur_idx += 1;
                    if self.state_log[cur_str_idx as usize].is_some() {
                        break;
                    }
                }
                let cur_state = self.merge_state_with_log(None);
                if cur_state.is_some() {
                    return cur_state;
                }
            }
        }

        /// From the node set CUR_NODES, pick up the nodes whose types are OP_OPEN_SUBEXP and which
        /// have corresponding back references in the regular expression.  And register them.
        fn check_subexp_matching_top(&mut self, cur_nodes: &[i32], str_idx: i32) {
            let dfa = self.dfa;
            for &node in cur_nodes {
                let t = &dfa.nodes[node as usize];
                if t.ty == OP_OPEN_SUBEXP && t.idx < 64 && dfa.used_bkref_map & (1u64 << t.idx) != 0 {
                    // match_ctx_add_subtop
                    self.sub_tops.push(SubTop { str_idx, node, path: Path::default(), lasts: Vec::new() });
                }
            }
        }

        fn transit_state_bkref(&mut self, nodes: &[i32]) {
            let dfa = self.dfa;
            let cur_str_idx = self.cur_idx;
            for &node_idx in nodes {
                let node = &dfa.nodes[node_idx as usize];
                // Check whether `node' is a backreference or not.
                if node.ty != OP_BACK_REF {
                    continue;
                }
                if node.constraint != 0 && not_satisfy_next(node.constraint, self.context_at(cur_str_idx)) {
                    continue;
                }
                // `node' is a backreference.  Check the substring which the substring matched.
                let mut bkc_idx = self.bkref_ents.len();
                self.get_subexp(node_idx, cur_str_idx);
                // And add the epsilon closures (which is `new_dest_nodes') of
                // the backreference to appropriate state_log.
                while bkc_idx < self.bkref_ents.len() {
                    let ent = self.bkref_ents[bkc_idx];
                    bkc_idx += 1;
                    if ent.node != node_idx || ent.str_idx != cur_str_idx {
                        continue;
                    }
                    let subexp_len = ent.subexp_to - ent.subexp_from;
                    let new_dest_nodes = if subexp_len == 0 {
                        &dfa.eclosures[dfa.edests[node_idx as usize][0] as usize]
                    } else {
                        &dfa.eclosures[dfa.nexts[node_idx as usize] as usize]
                    };
                    let dest_str_idx = cur_str_idx + ent.subexp_to - ent.subexp_from;
                    let context = self.context_at(dest_str_idx - 1);
                    let dest_state = self.state_log[dest_str_idx as usize].clone();
                    let prev_nelem = match &self.state_log[cur_str_idx as usize] {
                        None => 0,
                        Some(s) => s.nodes.len(),
                    };
                    // Add `new_dest_node' to state_log.
                    match dest_state {
                        None => {
                            self.state_log[dest_str_idx as usize] = dfa.acquire_context(new_dest_nodes, context);
                        }
                        Some(d) => {
                            let mut dest_nodes = d.entrance_nodes().clone();
                            set_merge(&mut dest_nodes, new_dest_nodes);
                            self.state_log[dest_str_idx as usize] = dfa.acquire_context(&dest_nodes, context);
                        }
                    }
                    // We need to check recursively if the backreference can epsilon transit.
                    let now = match &self.state_log[cur_str_idx as usize] {
                        None => 0,
                        Some(s) => s.nodes.len(),
                    };
                    if subexp_len == 0 && now > prev_nelem {
                        self.check_subexp_matching_top(new_dest_nodes, cur_str_idx);
                        self.transit_state_bkref(new_dest_nodes);
                    }
                }
            }
        }

        /// Enumerate all the candidates which the backreference BKREF_NODE can match at
        /// BKREF_STR_IDX, and register them by match_ctx_add_entry().
        fn get_subexp(&mut self, bkref_node: i32, bkref_str_idx: i32) {
            let dfa = self.dfa;
            let buf = self.input;
            let len = self.len();
            // Return if we have already checked BKREF_NODE at BKREF_STR_IDX.
            let cache_idx = self.search_cur_bkref_entry(bkref_str_idx);
            if cache_idx != -1 {
                let mut e = cache_idx as usize;
                loop {
                    if self.bkref_ents[e].node == bkref_node {
                        return; // we already checked it
                    }
                    let more = self.bkref_ents[e].more;
                    e += 1;
                    if !more {
                        break;
                    }
                }
            }
            let subexp_num = dfa.nodes[bkref_node as usize].idx;
            // For each sub expression
            for st in 0..self.sub_tops.len() {
                let top_node = self.sub_tops[st].node;
                if dfa.nodes[top_node as usize].idx != subexp_num {
                    continue; // it isn't related
                }
                let mut sl_str = self.sub_tops[st].str_idx;
                let mut bkref_str_off = bkref_str_idx;
                // At first, check the last node of sub expressions we already evaluated.
                let mut sub_last_idx = 0;
                while sub_last_idx < self.sub_tops[st].lasts.len() {
                    let sl_str_diff = self.sub_tops[st].lasts[sub_last_idx].str_idx - sl_str;
                    // The matched string by the sub expression match with the substring at the
                    // back reference?
                    if sl_str_diff > 0 {
                        // Not enough chars for a successful match.
                        if bkref_str_off + sl_str_diff > len {
                            break;
                        }
                        let (a, b, n) = (bkref_str_off as usize, sl_str as usize, sl_str_diff as usize);
                        if buf[a..a + n] != buf[b..b + n] {
                            break; // we don't need to search this sub expression any more
                        }
                    }
                    bkref_str_off += sl_str_diff;
                    sl_str += sl_str_diff;
                    let _ = self.get_subexp_sub(st, sub_last_idx, bkref_node, bkref_str_idx);
                    sub_last_idx += 1;
                }
                if sub_last_idx < self.sub_tops[st].lasts.len() {
                    continue;
                }
                if sub_last_idx > 0 {
                    sl_str += 1;
                }
                // Then, search for the other last nodes of the sub expression.
                while sl_str <= bkref_str_idx {
                    let sl_str_off = sl_str - self.sub_tops[st].str_idx;
                    // The matched string by the sub expression match with the substring at the
                    // back reference?
                    if sl_str_off > 0 {
                        // If we are at the end of the input, we cannot match.
                        if bkref_str_off >= len {
                            break;
                        }
                        let same = buf[bkref_str_off as usize] == buf[sl_str as usize - 1];
                        bkref_str_off += 1;
                        if !same {
                            break; // we don't need to search this sub expression any more
                        }
                    }
                    let nodes = match &self.state_log[sl_str as usize] {
                        None => {
                            sl_str += 1;
                            continue;
                        }
                        Some(s) => s.clone(),
                    };
                    // Does this state have a ')' of the sub expression?
                    let cls_node = Self::find_subexp_node(dfa, &nodes.nodes, subexp_num, OP_CLOSE_SUBEXP);
                    if cls_node == -1 {
                        sl_str += 1;
                        continue; // no
                    }
                    // Can the OP_OPEN_SUBEXP node arrive the OP_CLOSE_SUBEXP node in the current context?
                    let top_str = self.sub_tops[st].str_idx;
                    let mut path = core::mem::take(&mut self.sub_tops[st].path);
                    let arrived = self.check_arrival(&mut path, top_node, top_str, cls_node, sl_str, OP_CLOSE_SUBEXP);
                    self.sub_tops[st].path = path;
                    if !arrived {
                        sl_str += 1;
                        continue;
                    }
                    self.sub_tops[st].lasts.push(SubLast { node: cls_node, str_idx: sl_str, path: Path::default() });
                    let last = self.sub_tops[st].lasts.len() - 1;
                    let _ = self.get_subexp_sub(st, last, bkref_node, bkref_str_idx);
                    sl_str += 1;
                }
            }
        }

        /// Check SUB_LAST can arrive to the back reference BKREF_NODE at BKREF_STR.
        /// If it can arrive, register the sub expression expressed with SUB_TOP and SUB_LAST.
        fn get_subexp_sub(&mut self, st: usize, sl: usize, bkref_node: i32, bkref_str: i32) -> bool {
            let (last_node, last_str) = {
                let l = &self.sub_tops[st].lasts[sl];
                (l.node, l.str_idx)
            };
            let top_str = self.sub_tops[st].str_idx;
            // Can the subexpression arrive the back reference?
            let mut path = core::mem::take(&mut self.sub_tops[st].lasts[sl].path);
            let arrived = self.check_arrival(&mut path, last_node, last_str, bkref_node, bkref_str, OP_OPEN_SUBEXP);
            self.sub_tops[st].lasts[sl].path = path;
            if !arrived {
                return false;
            }
            self.match_ctx_add_entry(bkref_node, bkref_str, top_str, last_str);
            let to_idx = bkref_str + last_str - top_str;
            // clean_state_log_if_needed
            if self.state_log_top < to_idx {
                for k in self.state_log_top + 1..=to_idx {
                    self.state_log[k as usize] = None;
                }
                self.state_log_top = to_idx;
            }
            true
        }

        fn find_subexp_node(dfa: &Regex, nodes: &[i32], subexp_idx: i32, ty: u8) -> i32 {
            for &cls_node in nodes {
                let node = &dfa.nodes[cls_node as usize];
                if node.ty == ty && node.idx == subexp_idx {
                    return cls_node;
                }
            }
            -1
        }

        /// Check whether the node TOP_NODE at TOP_STR can arrive to the node LAST_NODE at LAST_STR.
        /// We record the path onto PATH since it will be heavily reused.
        fn check_arrival(&mut self, path: &mut Path, top_node: i32, top_str: i32, last_node: i32, last_str: i32, ty: u8) -> bool {
            let dfa = self.dfa;
            let subexp_num = dfa.nodes[top_node as usize].idx;
            // Extend the buffer if we need.
            if path.alloc < last_str + self.max_mb_elem_len + 1 {
                path.alloc += last_str + self.max_mb_elem_len + 1;
                path.array.resize(path.alloc as usize, None);
            }
            let mut str_idx = if path.next_idx != 0 { path.next_idx } else { top_str };
            // Temporary modify MCTX.
            core::mem::swap(&mut self.state_log, &mut path.array);
            // Setup initial node set.
            let mut context = self.context_at(str_idx - 1);
            let mut next_nodes: NodeSet;
            let mut cur_state: StateRef = None;
            if str_idx == top_str {
                next_nodes = vec![top_node];
                self.check_arrival_expand_ecl(&mut next_nodes, subexp_num, ty);
            } else {
                cur_state = self.state_log[str_idx as usize].clone();
                next_nodes = match &cur_state {
                    Some(s) if s.has_backref => s.nodes.clone(),
                    _ => Vec::new(),
                };
            }
            if str_idx == top_str || cur_state.as_ref().is_some_and(|s| s.has_backref) {
                if !next_nodes.is_empty() {
                    self.expand_bkref_cache(&mut next_nodes, str_idx, subexp_num, ty);
                }
                cur_state = dfa.acquire_context(&next_nodes, context);
                self.state_log[str_idx as usize] = cur_state.clone();
            }
            let mut null_cnt = 0;
            while str_idx < last_str && null_cnt <= self.max_mb_elem_len {
                next_nodes.clear();
                if let Some(s) = &self.state_log[str_idx as usize + 1] {
                    set_merge(&mut next_nodes, &s.nodes);
                }
                if let Some(s) = &cur_state {
                    // check_arrival_add_next_nodes
                    for &cur_node in &s.non_eps_nodes {
                        if self.check_node_accept(cur_node, str_idx) {
                            set_insert(&mut next_nodes, dfa.nexts[cur_node as usize]);
                        }
                    }
                }
                str_idx += 1;
                if !next_nodes.is_empty() {
                    self.check_arrival_expand_ecl(&mut next_nodes, subexp_num, ty);
                    self.expand_bkref_cache(&mut next_nodes, str_idx, subexp_num, ty);
                }
                context = self.context_at(str_idx - 1);
                cur_state = dfa.acquire_context(&next_nodes, context);
                self.state_log[str_idx as usize] = cur_state.clone();
                null_cnt = if cur_state.is_none() { null_cnt + 1 } else { 0 };
            }
            let arrived = match &self.state_log[last_str as usize] {
                Some(s) => set_contains(&s.nodes, last_node),
                None => false,
            };
            path.next_idx = str_idx;
            // Fix MCTX.
            core::mem::swap(&mut self.state_log, &mut path.array);
            arrived
        }

        /// For all the nodes in CUR_NODES, add the epsilon closures of them to CUR_NODES, however
        /// exclude the nodes which are:
        ///  - inside the sub expression whose number is EX_SUBEXP, if FL_OPEN.
        ///  - out of the sub expression whose number is EX_SUBEXP, if !FL_OPEN.
        fn check_arrival_expand_ecl(&self, cur_nodes: &mut NodeSet, ex_subexp: i32, ty: u8) {
            let dfa = self.dfa;
            let mut new_nodes = Vec::new();
            // Create a new node set NEW_NODES with the nodes which are epsilon closures of the
            // node in CUR_NODES.
            for &cur_node in cur_nodes.iter() {
                let eclosure = &dfa.eclosures[cur_node as usize];
                if Self::find_subexp_node(dfa, eclosure, ex_subexp, ty) == -1 {
                    set_merge(&mut new_nodes, eclosure); // there are no problematic nodes, just merge them
                } else {
                    // There are problematic nodes, re-calculate incrementally.
                    Self::check_arrival_expand_ecl_sub(dfa, &mut new_nodes, cur_node, ex_subexp, ty);
                }
            }
            *cur_nodes = new_nodes;
        }

        /// Check incrementally the epsilon closure of TARGET, and if it isn't problematic append it
        /// to DST_NODES.
        fn check_arrival_expand_ecl_sub(dfa: &Regex, dst_nodes: &mut NodeSet, target: i32, ex_subexp: i32, ty: u8) {
            let mut cur_node = target;
            while !set_contains(dst_nodes, cur_node) {
                let t = &dfa.nodes[cur_node as usize];
                if t.ty == ty && t.idx == ex_subexp {
                    if ty == OP_CLOSE_SUBEXP {
                        set_insert(dst_nodes, cur_node);
                    }
                    break;
                }
                set_insert(dst_nodes, cur_node);
                let e = &dfa.edests[cur_node as usize];
                if e.is_empty() {
                    break;
                }
                if e.len() == 2 {
                    Self::check_arrival_expand_ecl_sub(dfa, dst_nodes, e[1], ex_subexp, ty);
                }
                cur_node = e[0];
            }
        }

        /// For all the back references in the current state, calculate the destination of the back
        /// references by the appropriate entry in MCTX->BKREF_ENTS.
        fn expand_bkref_cache(&mut self, cur_nodes: &mut NodeSet, cur_str: i32, subexp_num: i32, ty: u8) {
            let dfa = self.dfa;
            let cache_idx_start = self.search_cur_bkref_entry(cur_str);
            if cache_idx_start == -1 {
                return;
            }
            'restart: loop {
                let mut e = cache_idx_start as usize;
                loop {
                    let ent = self.bkref_ents[e];
                    e += 1;
                    // Is this entry ENT is appropriate?
                    if set_contains(cur_nodes, ent.node) {
                        let to_idx = cur_str + ent.subexp_to - ent.subexp_from;
                        // Calculate the destination of the back reference, and append it to
                        // MCTX->STATE_LOG.
                        if to_idx == cur_str {
                            // The backreference did epsilon transit, we must re-check all the node
                            // in the current state.
                            let next_node = dfa.edests[ent.node as usize][0];
                            if !set_contains(cur_nodes, next_node) {
                                let mut new_dests = vec![next_node];
                                self.check_arrival_expand_ecl(&mut new_dests, subexp_num, ty);
                                set_merge(cur_nodes, &new_dests);
                                continue 'restart;
                            }
                        } else {
                            let next_node = dfa.nexts[ent.node as usize];
                            let mut union_set;
                            match &self.state_log[to_idx as usize] {
                                Some(s) if set_contains(&s.nodes, next_node) => {
                                    if !ent.more {
                                        break;
                                    }
                                    continue;
                                }
                                Some(s) => {
                                    union_set = s.nodes.clone();
                                    set_insert(&mut union_set, next_node);
                                }
                                None => union_set = vec![next_node],
                            }
                            self.state_log[to_idx as usize] = dfa.acquire(&union_set);
                        }
                    }
                    if !ent.more {
                        break;
                    }
                }
                return;
            }
        }

        /// check_node_accept
        fn check_node_accept(&self, node: i32, idx: i32) -> bool {
            let dfa = self.dfa;
            let ch = self.byte_at(idx);
            let t = &dfa.nodes[node as usize];
            match t.ty {
                CHARACTER => {
                    if t.c != ch {
                        return false;
                    }
                }
                SIMPLE_BRACKET => {
                    if !bit_contain(&dfa.sets[t.sbc as usize], ch) {
                        return false;
                    }
                }
                OP_PERIOD => {
                    if ch == 0 {
                        return false; // RE_DOT_NOT_NULL; RE_DOT_NEWLINE is set
                    }
                }
                _ => return false,
            }
            // The node has constraints.  Check whether the current context satisfies them.
            t.constraint == 0 || !not_satisfy_next(t.constraint, self.context_at(idx))
        }

        /// match_ctx_add_entry
        fn match_ctx_add_entry(&mut self, node: i32, str_idx: i32, from: i32, to: i32) {
            if let Some(last) = self.bkref_ents.last_mut() {
                if last.str_idx == str_idx {
                    last.more = true;
                }
            }
            self.bkref_ents.push(BkrefEnt {
                node,
                str_idx,
                subexp_from: from,
                subexp_to: to,
                more: false,
                // A backreference does not epsilon-transition unless it is empty, so set to all
                // zeros if FROM != TO.
                eps_reachable_subexps_map: if from == to { !0 } else { 0 },
            });
            if self.max_mb_elem_len < to - from {
                self.max_mb_elem_len = to - from;
            }
        }

        /// Search for the first entry which has the same str_idx, or -1 if none is found.
        fn search_cur_bkref_entry(&self, str_idx: i32) -> i32 {
            let last = self.bkref_ents.len();
            let (mut left, mut right) = (0, last);
            while left < right {
                let mid = (left + right) / 2;
                if self.bkref_ents[mid].str_idx < str_idx {
                    left = mid + 1;
                } else {
                    right = mid;
                }
            }
            if left < last && self.bkref_ents[left].str_idx == str_idx {
                left as i32
            } else {
                -1
            }
        }

        fn prune_impossible_nodes(&mut self) -> bool {
            let dfa = self.dfa;
            let mut match_last = self.match_last;
            let mut halt_node = self.last_node;
            let mut arrays = SiftArrays { sifted: vec![None; match_last as usize + 1], limited: None };
            if dfa.nbackref != 0 {
                loop {
                    arrays.limited = Some(vec![None; match_last as usize + 1]);
                    let sctx = Sift { last_node: halt_node, last_str_idx: match_last, limits: Vec::new() };
                    self.sift_states_backward(&mut arrays, &sctx);
                    if arrays.sifted[0].is_some() || arrays.limited.as_ref().unwrap()[0].is_some() {
                        break;
                    }
                    loop {
                        match_last -= 1;
                        if match_last < 0 {
                            return false;
                        }
                        if self.state_log[match_last as usize].as_ref().is_some_and(|s| s.halt) {
                            break;
                        }
                    }
                    let st = self.state_log[match_last as usize].clone().unwrap();
                    halt_node = self.check_halt_state_context(&st, match_last);
                }
                // merge_state_array
                let limited = arrays.limited.take().unwrap();
                Self::merge_state_array(dfa, &mut arrays.sifted, &limited, match_last + 1);
            } else {
                let sctx = Sift { last_node: halt_node, last_str_idx: match_last, limits: Vec::new() };
                self.sift_states_backward(&mut arrays, &sctx);
                if arrays.sifted[0].is_none() {
                    return false; // glibc 2.44
                }
            }
            self.state_log = arrays.sifted;
            self.last_node = halt_node;
            self.match_last = match_last;
            true
        }

        fn merge_state_array(dfa: &Regex, dst: &mut [StateRef], src: &[StateRef], num: i32) {
            for st_idx in 0..num as usize {
                if dst[st_idx].is_none() {
                    dst[st_idx] = src[st_idx].clone();
                } else if let Some(s) = &src[st_idx] {
                    let mut merged_set = dst[st_idx].as_ref().unwrap().nodes.clone();
                    set_merge(&mut merged_set, &s.nodes);
                    dst[st_idx] = dfa.acquire(&merged_set);
                }
            }
        }

        /// This function checks the STATE_LOG from the SCTX->last_str_idx to 0 and sift the nodes in
        /// each states according to the rules at the head of this function in regexec.c.
        fn sift_states_backward(&mut self, arrays: &mut SiftArrays, sctx: &Sift) {
            let mut null_cnt = 0;
            let mut str_idx = sctx.last_str_idx;
            // Build sifted state_log[str_idx].  It has the nodes which can epsilon transit to the
            // last_node and the last_node itself.
            let mut cur_dest = vec![sctx.last_node];
            self.update_cur_sifted_state(arrays, sctx, str_idx, &mut cur_dest);
            // Then check each states in the state_log.
            while str_idx > 0 {
                // Update counters.
                null_cnt = if arrays.sifted[str_idx as usize].is_none() { null_cnt + 1 } else { 0 };
                if null_cnt > self.max_mb_elem_len {
                    for k in 0..str_idx as usize {
                        arrays.sifted[k] = None;
                    }
                    return;
                }
                cur_dest.clear();
                str_idx -= 1;
                if self.state_log[str_idx as usize].is_some() {
                    self.build_sifted_states(arrays, sctx, str_idx, &mut cur_dest);
                }
                // Add all the nodes which satisfy the following conditions:
                // - It can epsilon transit to a node in CUR_DEST.
                // - It is in CUR_SRC.
                // And update state_log.
                self.update_cur_sifted_state(arrays, sctx, str_idx, &mut cur_dest);
            }
        }

        fn build_sifted_states(&mut self, arrays: &SiftArrays, sctx: &Sift, str_idx: i32, cur_dest: &mut NodeSet) {
            let dfa = self.dfa;
            let cur_src = self.state_log[str_idx as usize].clone().unwrap();
            // Then build the next sifted state.
            for &prev_node in &cur_src.non_eps_nodes {
                // We don't check backreferences here.  See update_cur_sifted_state().
                let next = dfa.nexts[prev_node as usize];
                let accepted = self.check_node_accept(prev_node, str_idx)
                    && match &arrays.sifted[str_idx as usize + 1] {
                        Some(s) => set_contains(&s.nodes, next),
                        None => false,
                    };
                if !accepted {
                    continue;
                }
                if !sctx.limits.is_empty() && self.check_dst_limits(&sctx.limits, next, str_idx + 1, prev_node, str_idx) {
                    continue;
                }
                set_insert(cur_dest, prev_node);
            }
        }

        fn update_cur_sifted_state(&mut self, arrays: &mut SiftArrays, sctx: &Sift, str_idx: i32, dest_nodes: &mut NodeSet) {
            let dfa = self.dfa;
            let candidates = self.state_log[str_idx as usize].clone();
            if dest_nodes.is_empty() {
                arrays.sifted[str_idx as usize] = None;
            } else {
                if let Some(c) = &candidates {
                    // At first, add the nodes which can epsilon transit to a node in DEST_NODE.
                    Self::add_epsilon_src_nodes(dfa, dest_nodes, &c.nodes);
                    // Then, check the limitations in the current sift_context.
                    if !sctx.limits.is_empty() {
                        self.check_subexp_limits(dest_nodes, &c.nodes, &sctx.limits, str_idx);
                    }
                }
                arrays.sifted[str_idx as usize] = dfa.acquire(dest_nodes);
            }
            if let Some(c) = &candidates {
                if c.has_backref {
                    self.sift_states_bkref(arrays, sctx, str_idx, &c.nodes);
                }
            }
        }

        fn add_epsilon_src_nodes(dfa: &Regex, dest_nodes: &mut NodeSet, candidates: &[i32]) {
            let mut inveclosure = Vec::new();
            for &d in dest_nodes.iter() {
                set_merge(&mut inveclosure, &dfa.inveclosures[d as usize]);
            }
            set_add_intersect(dest_nodes, candidates, &inveclosure);
        }

        fn sub_epsilon_src_nodes(dfa: &Regex, node: i32, dest_nodes: &mut NodeSet, candidates: &[i32]) {
            let inv_eclosure = &dfa.inveclosures[node as usize];
            let mut except_nodes = Vec::new();
            for &cur_node in inv_eclosure {
                if cur_node == node {
                    continue;
                }
                if is_epsilon(dfa.nodes[cur_node as usize].ty) {
                    let e = &dfa.edests[cur_node as usize];
                    let edst1 = e[0];
                    let edst2 = if e.len() > 1 { e[1] } else { -1 };
                    if (!set_contains(inv_eclosure, edst1) && set_contains(dest_nodes, edst1))
                        || (edst2 > 0 && !set_contains(inv_eclosure, edst2) && set_contains(dest_nodes, edst2))
                    {
                        set_add_intersect(&mut except_nodes, candidates, &dfa.inveclosures[cur_node as usize]);
                    }
                }
            }
            for &cur_node in inv_eclosure {
                if !set_contains(&except_nodes, cur_node) {
                    set_remove(dest_nodes, cur_node);
                }
            }
        }

        fn check_dst_limits(&mut self, limits: &[i32], dst_node: i32, dst_idx: i32, src_node: i32, src_idx: i32) -> bool {
            let dfa = self.dfa;
            let dst_bkref_idx = self.search_cur_bkref_entry(dst_idx);
            let src_bkref_idx = self.search_cur_bkref_entry(src_idx);
            for &lim in limits {
                let ent = self.bkref_ents[lim as usize];
                let subexp_idx = dfa.nodes[ent.node as usize].idx;
                let dst_pos = self.check_dst_limits_calc_pos(lim, subexp_idx, dst_node, dst_idx, dst_bkref_idx);
                let src_pos = self.check_dst_limits_calc_pos(lim, subexp_idx, src_node, src_idx, src_bkref_idx);
                // In case of:
                // <src> <dst> ( <subexp> )
                // ( <subexp> ) <src> <dst>
                // ( <subexp1> <src> <subexp2> <dst> <subexp3> )
                if src_pos == dst_pos {
                    continue; // this is unrelated limitation
                }
                return true;
            }
            false
        }

        fn check_dst_limits_calc_pos_1(&mut self, boundaries: i32, subexp_idx: i32, from_node: i32, bkref_idx: i32) -> i32 {
            let dfa = self.dfa;
            // Else, we are on the boundary: examine the nodes on the epsilon closure.
            for &node in &dfa.eclosures[from_node as usize] {
                let t = &dfa.nodes[node as usize];
                match t.ty {
                    OP_BACK_REF => {
                        if bkref_idx != -1 {
                            let mut e = bkref_idx as usize;
                            loop {
                                'ent: {
                                    let ent = self.bkref_ents[e];
                                    if ent.node != node {
                                        break 'ent;
                                    }
                                    if subexp_idx < 64 && ent.eps_reachable_subexps_map & (1u64 << subexp_idx) == 0 {
                                        break 'ent;
                                    }
                                    // Recurse trying to reach the OP_OPEN_SUBEXP and OP_CLOSE_SUBEXP
                                    // cases below.  But, if the destination node is the same node as
                                    // the source node, don't recurse because it would cause an
                                    // infinite loop: a regex that exhibits this behavior is ()\1*\1*
                                    let dst = dfa.edests[node as usize][0];
                                    if dst == from_node {
                                        if boundaries & 1 != 0 {
                                            return -1;
                                        }
                                        return 0; // boundaries & 2
                                    }
                                    let cpos = self.check_dst_limits_calc_pos_1(boundaries, subexp_idx, dst, bkref_idx);
                                    if cpos == -1 {
                                        return -1;
                                    }
                                    if cpos == 0 && boundaries & 2 != 0 {
                                        return 0;
                                    }
                                    if subexp_idx < 64 {
                                        self.bkref_ents[e].eps_reachable_subexps_map &= !(1u64 << subexp_idx);
                                    }
                                }
                                let more = self.bkref_ents[e].more;
                                e += 1;
                                if !more {
                                    break;
                                }
                            }
                        }
                    }
                    OP_OPEN_SUBEXP => {
                        if boundaries & 1 != 0 && subexp_idx == t.idx {
                            return -1;
                        }
                    }
                    OP_CLOSE_SUBEXP => {
                        if boundaries & 2 != 0 && subexp_idx == t.idx {
                            return 0;
                        }
                    }
                    _ => {}
                }
            }
            if boundaries & 2 != 0 {
                1
            } else {
                0
            }
        }

        fn check_dst_limits_calc_pos(&mut self, limit: i32, subexp_idx: i32, from_node: i32, str_idx: i32, bkref_idx: i32) -> i32 {
            let lim = self.bkref_ents[limit as usize];
            // If we are outside the range of the subexpression, return -1 or 1.
            if str_idx < lim.subexp_from {
                return -1;
            }
            if lim.subexp_to < str_idx {
                return 1;
            }
            // If we are within the subexpression, return 0.
            let mut boundaries = (str_idx == lim.subexp_from) as i32;
            boundaries |= ((str_idx == lim.subexp_to) as i32) << 1;
            if boundaries == 0 {
                return 0;
            }
            // Else, examine epsilon closure.
            self.check_dst_limits_calc_pos_1(boundaries, subexp_idx, from_node, bkref_idx)
        }

        /// Check the limitations of sub expressions LIMITS, and remove the nodes which are against
        /// limitations from DEST_NODES.
        fn check_subexp_limits(&self, dest_nodes: &mut NodeSet, candidates: &[i32], limits: &[i32], str_idx: i32) {
            let dfa = self.dfa;
            for &lim in limits {
                let ent = self.bkref_ents[lim as usize];
                if str_idx <= ent.subexp_from || ent.str_idx < str_idx {
                    continue; // this is unrelated limitation
                }
                let subexp_idx = dfa.nodes[ent.node as usize].idx;
                if ent.subexp_to == str_idx {
                    let mut ops_node = -1;
                    let mut cls_node = -1;
                    for &node in dest_nodes.iter() {
                        let t = &dfa.nodes[node as usize];
                        if t.ty == OP_OPEN_SUBEXP && subexp_idx == t.idx {
                            ops_node = node;
                        } else if t.ty == OP_CLOSE_SUBEXP && subexp_idx == t.idx {
                            cls_node = node;
                        }
                    }
                    // Check the limitation of the open subexpression.
                    // Note that (ent->subexp_to = str_idx != ent->subexp_from).
                    if ops_node >= 0 {
                        Self::sub_epsilon_src_nodes(dfa, ops_node, dest_nodes, candidates);
                    }
                    // Check the limitation of the close subexpression.
                    if cls_node >= 0 {
                        let mut node_idx: i32 = 0;
                        while node_idx < dest_nodes.len() as i32 {
                            let node = dest_nodes[node_idx as usize];
                            if !set_contains(&dfa.inveclosures[node as usize], cls_node)
                                && !set_contains(&dfa.eclosures[node as usize], cls_node)
                            {
                                // It is against this limitation.
                                // Remove it form the current sifted state.
                                Self::sub_epsilon_src_nodes(dfa, node, dest_nodes, candidates);
                                node_idx -= 1;
                            }
                            node_idx += 1;
                        }
                    }
                } else {
                    let mut node_idx = 0;
                    while node_idx < dest_nodes.len() {
                        let node = dest_nodes[node_idx];
                        let t = &dfa.nodes[node as usize];
                        if (t.ty == OP_CLOSE_SUBEXP || t.ty == OP_OPEN_SUBEXP) && subexp_idx == t.idx {
                            // It is against this limitation.
                            // Remove it form the current sifted state.
                            Self::sub_epsilon_src_nodes(dfa, node, dest_nodes, candidates);
                        }
                        node_idx += 1;
                    }
                }
            }
        }

        fn sift_states_bkref(&mut self, arrays: &mut SiftArrays, sctx: &Sift, str_idx: i32, candidates: &[i32]) {
            let dfa = self.dfa;
            let first_idx = self.search_cur_bkref_entry(str_idx);
            if first_idx == -1 {
                return;
            }
            let mut local_sctx: Option<Sift> = None; // not initialized yet
            for &node in candidates {
                // Avoid infinite loop for the REs like "()\1+".
                if node == sctx.last_node && str_idx == sctx.last_str_idx {
                    continue;
                }
                if dfa.nodes[node as usize].ty != OP_BACK_REF {
                    continue;
                }
                let mut enabled_idx = first_idx;
                loop {
                    'entry: {
                        let entry = self.bkref_ents[enabled_idx as usize];
                        if entry.node != node {
                            break 'entry;
                        }
                        let subexp_len = entry.subexp_to - entry.subexp_from;
                        let to_idx = str_idx + subexp_len;
                        let dst_node = if subexp_len != 0 {
                            dfa.nexts[node as usize]
                        } else {
                            dfa.edests[node as usize][0]
                        };
                        if to_idx > sctx.last_str_idx {
                            break 'entry;
                        }
                        let contains = match &arrays.sifted[to_idx as usize] {
                            Some(s) => set_contains(&s.nodes, dst_node),
                            None => false,
                        };
                        if !contains || self.check_dst_limits(&sctx.limits, node, str_idx, dst_node, to_idx) {
                            break 'entry;
                        }
                        let mut local = local_sctx.take().unwrap_or_else(|| sctx.clone());
                        local.last_node = node;
                        local.last_str_idx = str_idx;
                        set_insert(&mut local.limits, enabled_idx);
                        let cur_state = arrays.sifted[str_idx as usize].clone();
                        self.sift_states_backward(arrays, &local);
                        if let Some(limited) = &mut arrays.limited {
                            Self::merge_state_array(dfa, limited, &arrays.sifted, str_idx + 1);
                        }
                        arrays.sifted[str_idx as usize] = cur_state;
                        set_remove(&mut local.limits, enabled_idx);
                        local_sctx = Some(local);
                    }
                    let more = self.bkref_ents[enabled_idx as usize].more;
                    enabled_idx += 1;
                    if !more {
                        break;
                    }
                }
            }
        }

        /// Compute the next node to which "NFA" transit from NODE.
        /// Return the destination node, and update EPS_VIA_NODES; -1 in case of errors.
        fn proceed_next_node(
            &self,
            nregs: usize,
            regs: &[(i32, i32)],
            prevregs: &[(i32, i32)],
            pidx: &mut i32,
            node: i32,
            eps_via_nodes: &mut NodeSet,
            fs: &mut Option<Vec<FailEnt>>,
        ) -> i32 {
            let dfa = self.dfa;
            let t = &dfa.nodes[node as usize];
            if is_epsilon(t.ty) {
                let cur_nodes = self.log_nodes(*pidx);
                set_insert(eps_via_nodes, node);
                // Pick a valid destination, or return -1 if none is found.
                let mut dest_node = -1;
                for &candidate in &dfa.edests[node as usize] {
                    if !set_contains(&cur_nodes, candidate) {
                        continue;
                    }
                    if dest_node == -1 {
                        dest_node = candidate;
                    } else {
                        // In order to avoid infinite loop like "(a*)*", return the second
                        // epsilon-transition if the first was already considered.
                        if set_contains(eps_via_nodes, dest_node) {
                            return candidate;
                        }
                        // Otherwise, push the second epsilon-transition on the fail stack.
                        if let Some(stack) = fs {
                            stack.push(FailEnt {
                                idx: *pidx,
                                node: candidate,
                                regs: regs.to_vec(),
                                prevregs: prevregs.to_vec(),
                                eps_via_nodes: eps_via_nodes.clone(),
                            });
                        }
                        // We know we are going to exit.
                        break;
                    }
                }
                dest_node
            } else {
                let mut naccepted = 0;
                if t.ty == OP_BACK_REF {
                    let subexp_idx = t.idx as usize + 1;
                    if subexp_idx < nregs {
                        naccepted = regs[subexp_idx].1 - regs[subexp_idx].0;
                    }
                    if fs.is_some() {
                        if subexp_idx >= nregs || regs[subexp_idx].0 == -1 || regs[subexp_idx].1 == -1 {
                            return -1;
                        }
                        if naccepted != 0 {
                            let (so, n, at) = (regs[subexp_idx].0 as usize, naccepted as usize, *pidx as usize);
                            if self.input.len() < at + n || self.input[so..so + n] != self.input[at..at + n] {
                                return -1;
                            }
                        }
                    }
                    if naccepted == 0 {
                        set_insert(eps_via_nodes, node);
                        let dest_node = dfa.edests[node as usize][0];
                        if self.log_contains(*pidx, dest_node) {
                            return dest_node;
                        }
                    }
                }
                if naccepted != 0 || self.check_node_accept(node, *pidx) {
                    let dest_node = dfa.nexts[node as usize];
                    *pidx = if naccepted == 0 { *pidx + 1 } else { *pidx + naccepted };
                    if fs.is_some() && (*pidx > self.match_last || !self.log_contains(*pidx, dest_node)) {
                        return -1;
                    }
                    eps_via_nodes.clear();
                    return dest_node;
                }
                -1
            }
        }

        fn update_regs(&self, pmatch: &mut [(i32, i32)], prev_idx_match: &mut [(i32, i32)], cur_node: i32, cur_idx: i32) {
            let t = &self.dfa.nodes[cur_node as usize];
            let nmatch = pmatch.len();
            if t.ty == OP_OPEN_SUBEXP {
                let reg_num = t.idx as usize + 1;
                // We are at the first node of this sub expression.
                if reg_num < nmatch {
                    pmatch[reg_num] = (cur_idx, -1);
                }
            } else if t.ty == OP_CLOSE_SUBEXP {
                // We are at the last node of this sub expression.
                let reg_num = t.idx as usize + 1;
                if reg_num < nmatch {
                    if pmatch[reg_num].0 < cur_idx {
                        pmatch[reg_num].1 = cur_idx;
                        // This is a non-empty match or we are not inside an optional subexpression.
                        // Accept this right away.
                        prev_idx_match.copy_from_slice(pmatch);
                    } else if t.opt_subexp && prev_idx_match[reg_num].0 != -1 {
                        // We transited through an empty match for an optional subexpression, like
                        // (a?)*, and this is not the subexp's first match.  Copy back the old content
                        // of the registers so that matches of an inner subexpression are undone as
                        // well, like in ((a?))*.
                        pmatch.copy_from_slice(prev_idx_match);
                    } else {
                        // We completed a subexpression, but it may be part of an optional one, so do
                        // not update PREV_IDX_MATCH.
                        pmatch[reg_num].1 = cur_idx;
                    }
                }
            }
        }

        /// Set the positions where the subexpressions are starts/ends to registers PMATCH.  False is
        /// REG_NOMATCH.  Note: We assume that pmatch[0] is already set, and
        /// pmatch[i].rm_so == pmatch[i].rm_eo == -1 for 0 < i < nmatch.
        fn set_regs(&self, nmatch: usize, pmatch: &mut [(i32, i32)], fl_backtrack: bool) -> bool {
            let dfa = self.dfa;
            let mut fs: Option<Vec<FailEnt>> = if fl_backtrack { Some(Vec::new()) } else { None };
            let mut cur_node = dfa.init_node;
            let mut eps_via_nodes: NodeSet = Vec::new();
            let mut prev_idx_match = pmatch.to_vec();
            let mut idx = pmatch[0].0;

            fn pop_fail_stack(
                fs: &mut Option<Vec<FailEnt>>,
                pidx: &mut i32,
                regs: &mut [(i32, i32)],
                prevregs: &mut [(i32, i32)],
                eps_via_nodes: &mut NodeSet,
            ) -> i32 {
                let Some(stack) = fs else {
                    return -1;
                };
                let Some(ent) = stack.pop() else {
                    return -1;
                };
                *pidx = ent.idx;
                regs.copy_from_slice(&ent.regs);
                prevregs.copy_from_slice(&ent.prevregs);
                *eps_via_nodes = ent.eps_via_nodes;
                ent.node
            }

            while idx <= pmatch[0].1 {
                self.update_regs(pmatch, &mut prev_idx_match, cur_node, idx);
                if (idx == pmatch[0].1 && cur_node == self.last_node)
                    || (fs.is_some() && set_contains(&eps_via_nodes, cur_node))
                {
                    cur_node = -1;
                    if fs.is_some() {
                        for reg_idx in 0..nmatch {
                            if pmatch[reg_idx].0 > -1 && pmatch[reg_idx].1 == -1 {
                                cur_node = pop_fail_stack(&mut fs, &mut idx, pmatch, &mut prev_idx_match, &mut eps_via_nodes);
                                break;
                            }
                        }
                    }
                    if cur_node < 0 {
                        return true;
                    }
                }
                // Proceed to next node.
                cur_node = self.proceed_next_node(nmatch, pmatch, &prev_idx_match, &mut idx, cur_node, &mut eps_via_nodes, &mut fs);
                if cur_node < 0 {
                    cur_node = pop_fail_stack(&mut fs, &mut idx, pmatch, &mut prev_idx_match, &mut eps_via_nodes);
                    if cur_node < 0 {
                        return false;
                    }
                }
            }
            true
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scan::testkit::decode as decode_str;
    use crate::toks::testkit::*;

    /// Runs one of the escape routines on these bytes in the pool, and takes what it appended.
    fn on_pool(t: &mut Tex, f: fn(&mut Tex, i32), input: &[u8]) -> Vec<u8> {
        let b0 = t.pool_ptr;
        for &c in input {
            t.str_pool.put(t.pool_ptr, c as i32);
            t.pool_ptr += 1;
        }
        let out = t.pool_ptr;
        f(t, b0);
        let r = t.str_pool.as_slice()[out as usize..t.pool_ptr as usize].to_vec();
        t.pool_ptr = b0;
        r
    }

    #[test]
    fn escapes_against_the_oracle() {
        let t = machine();
        let mut n = 0;
        for line in include_str!("../testdata/escape_oracle.tsv").lines() {
            let f: Vec<&str> = line.split('\t').collect();
            n += 1;
            match f[0] {
                "B" => {
                    let k = [f[1].parse::<u16>().unwrap() as u8];
                    assert_eq!(on_pool(t, Tex::escapestring, &k), decode_str(f[2]), "string of {}", k[0]);
                    assert_eq!(on_pool(t, Tex::escapename, &k), decode_str(f[3]), "name of {}", k[0]);
                    assert_eq!(on_pool(t, Tex::escapehex, &k), decode_str(f[4]), "hex of {}", k[0]);
                }
                "U" => {
                    let u = decode_str(f[1]);
                    assert_eq!(on_pool(t, Tex::unescapehex, &u), decode_str(f[2]), "unescape of {:?}", f[1]);
                }
                _ => {
                    let got = t.buildtoks_makepdftime(f[1].parse().unwrap(), true);
                    assert_eq!(got, f[2].as_bytes(), "the date of epoch {}", f[1]);
                }
            }
        }
        assert!(n > 270);
        Tex::dealloc(t);
    }

    /// utils.c leaves `pool_ptr=pool_size` for str_toks's str_room to report, one test before it
    /// would write past the end.
    #[test]
    fn escapes_stop_at_the_end_of_the_pool() {
        for (room, f) in [
            (4, Tex::escapestring as fn(&mut Tex, i32)),
            (3, Tex::escapename as fn(&mut Tex, i32)),
            (2, Tex::escapehex as fn(&mut Tex, i32)),
            (1, Tex::unescapehex as fn(&mut Tex, i32)),
        ] {
            let t = machine();
            t.pool_size = 5 + room;
            t.pool_ptr = 0;
            t.str_pool.put(0, b'4' as i32);
            t.str_pool.put(1, b'1' as i32);
            t.pool_ptr = 5;
            f(t, 0);
            assert_eq!(t.pool_ptr, t.pool_size, "room {}", room);
            Tex::dealloc(t);
        }
        let t = machine();
        t.pool_size = 100;
        t.pool_ptr = 96;
        t.getmatch(0);
        assert_eq!(t.pool_ptr, t.pool_size, "getmatch's size is 4 and the test is >=");
        t.pool_ptr = 89;
        t.getfiledump(0, 0, 5);
        assert_eq!(t.pool_ptr, t.pool_size, "room for 2*length+1 only");
        t.pool_ptr = 68;
        t.getmd5sum(0, false);
        assert_eq!(t.pool_ptr, 68, "no room for the 32 digits: the result is empty, and no overflow follows");
        t.pool_ptr = 90;
        assert_eq!(t.matchstrings(0, 0, -1, false), 0);
        assert_eq!(t.pool_ptr, t.pool_size, "matchstrings wants 10");
        Tex::dealloc(t);
    }

    /// What `\pdfmatch` and `\pdflastmatch` keep between them, by the real routines.
    #[test]
    fn matchstrings_and_getmatch_keep_their_state() {
        let t = machine();
        print_setup(t);
        let mk = |t: &mut Tex, b: &[u8]| {
            for &c in b {
                t.append_char(c as i32);
            }
            t.make_string()
        };
        let lm = |t: &mut Tex, i: i32| {
            let b = t.pool_ptr;
            t.getmatch(i);
            let r = String::from_utf8_lossy(&t.str_pool.as_slice()[b as usize..t.pool_ptr as usize]).into_owned();
            t.pool_ptr = b;
            r
        };
        let res = |t: &mut Tex, s: i32, x: i32, c: i32, i: bool| {
            let b = t.pool_ptr;
            t.matchstrings(s, x, c, i);
            let r = String::from_utf8_lossy(&t.str_pool.as_slice()[b as usize..t.pool_ptr as usize]).into_owned();
            t.pool_ptr = b;
            r
        };
        assert_eq!(lm(t, 0), "-1->", "before any match");
        let (p, x) = (mk(t, b"(a)(b)?"), mk(t, b"xa\0b"));
        assert_eq!(res(t, p, x, -1, false), "1");
        assert_eq!(lm(t, 0), "1->a");
        assert_eq!(lm(t, 1), "1->a");
        assert_eq!(lm(t, 2), "-1->", "the string stops at the NUL");
        assert_eq!(lm(t, 9), "-1->");
        assert_eq!(lm(t, 10), "-1->");
        let bad = mk(t, b"(");
        assert_eq!(res(t, bad, x, -1, false), "-1");
        assert!(printed(t).contains(": \\pdfmatch: Unmatched ( or \\("));
        assert_eq!(lm(t, 1), "1->a", "a bad pattern leaves the last match");
        let none = mk(t, b"z");
        assert_eq!(res(t, none, x, 5, false), "0");
        assert_eq!(lm(t, 0), "-1->");
        let a = mk(t, b"A");
        assert_eq!(res(t, a, x, 0, true), "1");
        assert_eq!(lm(t, 0), "-1->", "subcount 0 keeps no registers");
        assert_eq!(res(t, a, x, 1, true), "1");
        assert_eq!(lm(t, 0), "1->a");
        assert_eq!(lm(t, 1), "-1->");
        Tex::dealloc(t);
    }

    /// The stacks are pdfship.rs's: a stack made here is one that `\pdfcolorstack` finds, which is what
    /// pdfship.rs's `colorstack...` functions test.
    #[test]
    fn newcolorstack_makes_the_stacks_pdfship_uses() {
        let t = machine();
        print_setup(t);
        for &c in b"1 0 0 rg\0 x" {
            t.append_char(c as i32);
        }
        let s = t.make_string();
        assert_eq!(t.newcolorstack(s, DIRECT_PAGE, true), 1, "stack 0 is the default");
        assert_eq!(t.newcolorstack(S_EMPTY, SET_ORIGIN, false), 2);
        assert_eq!(t.colorstackused(), 3);
        assert_eq!(t.colorstackskippagestart(1), 0, "page start, and a colour");
        assert_eq!(t.colorstackskippagestart(2), 1, "no page start");
        let b = t.pool_ptr;
        assert_eq!(t.colorstackcurrent(1), DIRECT_PAGE);
        assert_eq!(t.str_pool.range(b, t.pool_ptr), b"1 0 0 rg", "a C string stops at the NUL");
        t.pool_ptr = b;
        assert_eq!(t.colorstackcurrent(2), SET_ORIGIN);
        assert_eq!(t.pool_ptr, b, "an empty text is NULL");
        t.v_pdfship.colstacks.len = 32768;
        assert_eq!(t.newcolorstack(S_EMPTY, SET_ORIGIN, false), -1);
        Tex::dealloc(t);
    }

    /// %XX in a field of the tables stands for a byte.
    fn decode(f: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut i = 0;
        while i < f.len() {
            if f[i] == b'%' {
                let h = core::str::from_utf8(&f[i + 1..i + 3]).unwrap();
                out.push(u8::from_str_radix(h, 16).unwrap());
                i += 3;
            } else {
                out.push(f[i]);
                i += 1;
            }
        }
        out
    }

    fn show(b: &[u8]) -> String {
        String::from_utf8_lossy(b).into_owned()
    }

    /// Every row of tests/oracle/gen_regex_tables.py: regcomp's message or regexec's status, and what
    /// `\pdflastmatch` makes of the registers.
    #[test]
    fn regex_against_the_oracle() {
        let table: &[u8] = include_bytes!("../testdata/regex_oracle.tsv");
        let (mut n, mut bad) = (0, 0);
        let mut report = String::new();
        for line in table.split(|&c| c == b'\n') {
            if line.is_empty() {
                continue;
            }
            n += 1;
            let f: Vec<&[u8]> = line.split(|&c| c == b'\t').collect();
            let (pat, text) = (decode(f[0]), decode(f[1]));
            let icase = f[2] == b"1";
            let count: usize = if f[3] == b"-" { 10 } else { show(f[3]).parse().unwrap() };
            let want_res = decode(f[4]);
            let mut want_lm: Vec<Vec<u8>> = f[5..].iter().map(|x| decode(x)).collect();
            want_lm.resize(12, b"-1->".to_vec());
            let (got_res, got_lm): (Vec<u8>, Vec<Vec<u8>>) = match regex::Regex::compile(&pat, icase) {
                Err(msg) => (msg.to_vec(), vec![b"-1->".to_vec(); 12]),
                Ok(re) => {
                    let r = re.exec(&text, count);
                    let lm = (0..12)
                        .map(|k| {
                            if let Some(regs) = &r {
                                if k < count && k < regs.len() && regs[k].0 >= 0 && regs[k].1 >= regs[k].0 {
                                    let (so, eo) = (regs[k].0 as usize, regs[k].1 as usize);
                                    let mut v = so.to_string().into_bytes();
                                    v.extend_from_slice(b"->");
                                    v.extend_from_slice(&text[so..eo]);
                                    return v;
                                }
                            }
                            b"-1->".to_vec()
                        })
                        .collect();
                    (if r.is_some() { b"1".to_vec() } else { b"0".to_vec() }, lm)
                }
            };
            // A bad pattern leaves what the last good one matched, which the table has not kept.
            let compare_lm = want_res == b"1" || want_res == b"0";
            if got_res != want_res || (compare_lm && got_lm != want_lm) {
                bad += 1;
                if bad <= 40 {
                    report.push_str(&format!(
                        "row {}: {:?} on {:?} icase={} count={}\n  oracle {:?} {:?}\n  port   {:?} {:?}\n",
                        n,
                        show(&pat),
                        show(&text),
                        icase,
                        count,
                        show(&want_res),
                        want_lm.iter().map(|x| show(x)).collect::<Vec<_>>(),
                        show(&got_res),
                        got_lm.iter().map(|x| show(x)).collect::<Vec<_>>()
                    ));
                }
            }
        }
        assert_eq!(bad, 0, "{} of {} rows differ\n{}", bad, n, report);
        assert!(n > 500);
    }
}
