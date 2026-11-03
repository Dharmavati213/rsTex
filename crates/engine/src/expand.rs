//! §388-413 Part 25: expanding the next token, before macro_call.
//!
//! Token lists are in `toks`, not `mem`: docs/PORTING.md, "Token lists".
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

/// The conditionals with a bit here scan no number, with or without what they then skip.  `\unless` gets to
/// `conditional` by `expand_saving` only.
const COND_NO_NUMBER: u32 = 1 << IF_CHAR_CODE
    | 1 << IF_CAT_CODE
    | 1 << IF_VMODE_CODE
    | 1 << IF_HMODE_CODE
    | 1 << IF_MMODE_CODE
    | 1 << IF_INNER_CODE
    | 1 << IFX_CODE
    | 1 << IF_TRUE_CODE
    | 1 << IF_FALSE_CODE
    | 1 << IF_DEF_CODE
    | 1 << IF_CS_CODE
    | 1 << IF_IN_CSNAME_CODE;

impl Tex {
    /// §388.  What follows the macro's case and the `match` is WEB's procedure, in `expand_saving`.
    /// The commands taken here scan no number, so they leave `cur_val`, `cur_val_level`, `radix` and
    /// `cur_order` alone, and the backups are not needed.
    pub fn expand(&mut self) {
        if self.cur_cmd >= CALL && self.cur_cmd < END_TEMPLATE {
            // §390.  macro_call does not come back to expand, so the count is only tested.
            if self.expand_depth_count + 1 >= self.expand_depth {
                self.overflow(b"expansion depth", self.expand_depth);
            }
            self.macro_call();
            return;
        }
        self.expand_depth_count += 1;
        if self.expand_depth_count >= self.expand_depth {
            self.overflow(b"expansion depth", self.expand_depth);
        }
        if self.tracing_commands() <= 1 {
            match self.cur_cmd {
                EXPAND_AFTER if self.cur_chr == 0 => {
                    self.expand_after();
                    self.expand_depth_count -= 1;
                    return;
                }
                IF_TEST if COND_NO_NUMBER >> self.cur_chr & 1 != 0 => {
                    self.conditional();
                    self.expand_depth_count -= 1;
                    return;
                }
                FI_OR_ELSE => {
                    self.expand_fi_or_else();
                    self.expand_depth_count -= 1;
                    return;
                }
                CS_NAME => {
                    self.expand_cs_name();
                    self.expand_depth_count -= 1;
                    return;
                }
                _ => {}
            }
        }
        self.expand_saving();
        self.expand_depth_count -= 1;
    }

    /// §392
    #[inline]
    fn expand_after(&mut self) {
        self.get_token();
        let t = self.cur_tok;
        self.get_token();
        if self.cur_cmd > MAX_COMMAND {
            self.expand();
        } else {
            self.back_input();
        }
        self.cur_tok = t;
        self.back_input();
    }

    /// §398.  The characters go on the tx stack: see Tex::tx_mark.
    #[inline]
    fn expand_cs_name(&mut self) {
        let m = self.tx_mark();
        let b = self.is_in_csname;
        self.is_in_csname = true;
        loop {
            self.expand_tx_chars();
            self.get_x_token();
            if self.cur_cs == 0 {
                self.tx_push(self.cur_tok);
            }
            if self.cur_cs != 0 {
                break;
            }
        }
        if self.cur_cmd != END_CS_NAME {
            // §399
            self.print_err_s(b"Missing ");
            self.print_esc_s(b"endcsname");
            self.print_s(b" inserted");
            self.help(&[
                b"The control sequence marked <to be read again> should",
                b"not appear between \\csname and \\endcsname.",
            ]);
            self.back_error();
        }
        self.is_in_csname = b;
        // §400
        let j = self.expand_tx_to_buffer(m);
        if j > self.first + 1 {
            self.no_new_control_sequence = false;
            self.cur_cs = self.id_lookup(self.first, j - self.first);
            self.no_new_control_sequence = true;
        } else if j == self.first {
            self.cur_cs = NULL_CS;
        } else {
            self.cur_cs = SINGLE_BASE + self.buffer.at(self.first);
        }
        self.tx_discard(m);
        if self.eq_type(self.cur_cs) == UNDEFINED_CS {
            self.eq_define(self.cur_cs, RELAX, 256);
        }
        self.cur_tok = self.cur_cs + CS_TOKEN_FLAG;
        self.back_input();
    }

    /// What `get_x_token` and the store would do with the letters and other characters that come next in
    /// the current token list, in `\csname` and `\ifcsname`.  For these get_next only sets `cur_cmd`,
    /// `cur_chr` and `cur_cs`, which the `get_x_token` after this sets again.
    #[inline(always)]
    pub fn expand_tx_chars(&mut self) {
        if self.state() == TOKEN_LIST {
            let mut l = self.loc();
            let mut i = self.tx_ptr;
            loop {
                let t = self.toks[l];
                if ((t >> 8) - LETTER) as u32 > (OTHER_CHAR - LETTER) as u32 {
                    break;
                }
                if i as usize >= TX_CAP {
                    self.tx_overflow();
                }
                self.tx[i] = t;
                i += 1;
                l += 1;
            }
            self.tx_ptr = i;
            self.set_loc(l);
        }
    }

    /// §400, §1768: the characters on the tx stack from `m` go in `buffer` from `first` on, and this says
    /// where they end.  A token here is `cmd*256+chr`, not negative, so its `mod 256` is its low byte.  §400's
    /// test on each `j` comes to the one on the last, and with no characters there is none.
    #[inline]
    pub fn expand_tx_to_buffer(&mut self, m: i32) -> i32 {
        let j = self.first + self.tx_ptr - m;
        if j > self.max_buf_stack && self.tx_ptr > m {
            self.max_buf_stack = if j < self.buf_size { j } else { self.buf_size };
            if self.max_buf_stack == self.buf_size {
                self.overflow(b"buffer size", self.buf_size);
            }
        }
        for (b, &t) in self.buffer.range_mut(self.first, j).iter_mut().zip(self.tx.range(m, self.tx_ptr)) {
            *b = t as u8;
        }
        j
    }

    /// §536
    #[inline]
    fn expand_fi_or_else(&mut self) {
        if self.tracing_ifs() > 0 && self.tracing_commands() <= 1 {
            self.show_cur_cmd_chr();
        }
        if self.cur_chr > self.if_limit {
            if self.if_limit == IF_CODE {
                self.insert_relax();
            } else {
                self.print_err_s(b"Extra ");
                self.print_cmd_chr(FI_OR_ELSE, self.cur_chr);
                self.help(&[b"I'm ignoring this; it doesn't match any \\if."]);
                self.error();
            }
        } else {
            while self.cur_chr != FI_CODE {
                self.pass_text();
            }
            self.cond_pop(); // §522
        }
    }

    /// §388: the procedure as WEB has it.
    #[inline(never)]
    fn expand_saving(&mut self) {
        let cv_backup = self.cur_val;
        let cvl_backup = self.cur_val_level;
        let radix_backup = self.radix;
        let co_backup = self.cur_order;
        // WEB's backup_backup is gone: scan_keyword keeps its tokens on the tx stack, not in
        // link(backup_head), and that stack nests by itself.
        'reswitch: loop {
            if self.cur_cmd < CALL {
                // §391
                if self.tracing_commands() > 1 {
                    self.show_cur_cmd_chr();
                }
                match self.cur_cmd {
                    TOP_BOT_MARK => {
                        // §412
                        let t = self.cur_chr % MARKS_CODE;
                        if self.cur_chr >= MARKS_CODE {
                            self.scan_register_num();
                        } else {
                            self.cur_val = 0;
                        }
                        if self.cur_val == 0 {
                            self.cur_ptr = self.cur_mark[t];
                        } else {
                            // §1824
                            self.find_sa_element(MARK_VAL, self.cur_val, false);
                            if self.cur_ptr != NULL {
                                if odd(t) {
                                    self.cur_ptr = self.link(self.cur_ptr + (t / 2) + 1);
                                } else {
                                    self.cur_ptr = self.info(self.cur_ptr + (t / 2) + 1);
                                }
                            }
                        }
                        if self.cur_ptr != NULL {
                            self.begin_token_list(self.cur_ptr, MARK_TEXT);
                        }
                    }
                    EXPAND_AFTER => {
                        if self.cur_chr == 0 {
                            self.expand_after(); // §392
                        } else {
                            // §1765
                            self.get_token();
                            if self.cur_cmd == IF_TEST && self.cur_chr != IF_CASE_CODE {
                                self.cur_chr += UNLESS_CODE;
                                continue 'reswitch;
                            }
                            self.print_err_s(b"You can't use `");
                            self.print_esc_s(b"unless");
                            self.print_s(b"' before `");
                            self.print_cmd_chr(self.cur_cmd, self.cur_chr);
                            self.print_char(b'\'' as i32);
                            self.help(&[b"Continue, and I'll forget that it ever happened."]);
                            self.back_error();
                        }
                    }
                    NO_EXPAND => {
                        if self.cur_chr == 0 {
                            // §393
                            let save_scanner_status = self.scanner_status;
                            self.scanner_status = NORMAL;
                            self.get_token();
                            self.scanner_status = save_scanner_status;
                            let t = self.cur_tok;
                            self.back_input();
                            if t >= CS_TOKEN_FLAG && t != END_WRITE_TOKEN {
                                self.back_prepend(CS_TOKEN_FLAG + FROZEN_DONT_EXPAND);
                            }
                        } else {
                            // §394
                            let save_scanner_status = self.scanner_status;
                            self.scanner_status = NORMAL;
                            self.get_token();
                            self.scanner_status = save_scanner_status;
                            if self.cur_cs < HASH_BASE {
                                self.cur_cs = self.prim_lookup(self.cur_cs - SINGLE_BASE);
                            } else {
                                self.cur_cs = self.prim_lookup(self.text(self.cur_cs));
                            }
                            if self.cur_cs != UNDEFINED_PRIMITIVE {
                                let t = self.prim_eq_type(self.cur_cs);
                                if t > MAX_COMMAND {
                                    self.cur_cmd = t;
                                    self.cur_chr = self.prim_equiv(self.cur_cs);
                                    self.cur_tok = self.cur_cmd * 256 + self.cur_chr;
                                    self.cur_cs = 0;
                                    continue 'reswitch;
                                } else {
                                    self.back_input();
                                    self.back_prepend(CS_TOKEN_FLAG + FROZEN_PRIMITIVE);
                                }
                            }
                        }
                    }
                    CS_NAME => self.expand_cs_name(),
                    CONVERT => self.conv_toks(),
                    THE => self.ins_the_toks(),
                    IF_TEST => self.conditional(),
                    FI_OR_ELSE => self.expand_fi_or_else(),
                    INPUT => {
                        // §404
                        if self.cur_chr == 1 {
                            self.force_eof = true;
                        } else if self.cur_chr == 2 {
                            // §1749
                            self.pseudo_start();
                        } else if self.name_in_progress {
                            self.insert_relax();
                        } else {
                            self.start_input();
                        }
                    }
                    _ => {
                        // §396
                        self.print_err_s(b"Undefined control sequence");
                        self.help(&[
                            b"The control sequence at the end of the top line",
                            b"of your error message was never \\def'ed. If you have",
                            b"misspelled it (e.g., `\\hobx'), type `I' and the correct",
                            b"spelling (e.g., `I\\hbox'). Otherwise just continue,",
                            b"and I'll forget about whatever was undefined.",
                        ]);
                        self.error();
                    }
                }
            } else if self.cur_cmd < END_TEMPLATE {
                self.macro_call();
            } else {
                // §401
                self.cur_tok = CS_TOKEN_FLAG + FROZEN_ENDV;
                self.back_input();
            }
            break;
        }
        self.cur_val = cv_backup;
        self.cur_val_level = cvl_backup;
        self.radix = radix_backup;
        self.cur_order = co_backup;
    }

    /// §405
    pub fn insert_relax(&mut self) {
        self.cur_tok = CS_TOKEN_FLAG + self.cur_cs;
        self.back_input();
        self.cur_tok = CS_TOKEN_FLAG + FROZEN_RELAX;
        self.back_input();
        self.set_token_type(INSERTED);
    }

    /// §406
    pub fn get_x_token(&mut self) {
        let t = self.get_next_leaf();
        if t != 0 && self.cur_cmd <= MAX_COMMAND {
            self.cur_tok = t;
        } else {
            self.get_x_token_rest(t);
        }
    }

    /// §406 after the first `get_next_leaf`, whose result is `t`: either it did nothing, or the token it gave is expandable.
    #[inline(never)]
    fn get_x_token_rest(&mut self, t: i32) {
        if t == 0 {
            self.get_next_slow();
        }
        while self.cur_cmd > MAX_COMMAND {
            if self.cur_cmd >= CALL {
                if self.cur_cmd < END_TEMPLATE {
                    self.macro_call();
                } else {
                    self.cur_cs = FROZEN_ENDV;
                    self.cur_cmd = ENDV;
                    break;
                }
            } else {
                self.expand();
            }
            self.get_next();
        }
        // done:
        if self.cur_cs == 0 {
            self.cur_tok = self.cur_cmd * 256 + self.cur_chr;
        } else {
            self.cur_tok = CS_TOKEN_FLAG + self.cur_cs;
        }
    }

    /// §407
    pub fn x_token(&mut self) {
        while self.cur_cmd > MAX_COMMAND {
            self.expand();
            self.get_next();
        }
        if self.cur_cs == 0 {
            self.cur_tok = self.cur_cmd * 256 + self.cur_chr;
        } else {
            self.cur_tok = CS_TOKEN_FLAG + self.cur_cs;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::toks::testkit::*;

    /// `get_x_token` until the input is over.
    fn expanded(t: &mut Tex) -> Vec<i32> {
        let mut v = Vec::new();
        loop {
            t.get_x_token();
            if t.cur_tok == 0 {
                return v;
            }
            v.push(t.cur_tok);
        }
    }

    fn lets(s: &[u8]) -> Vec<i32> {
        s.iter().map(|&c| LETTER_TOKEN + c as i32).collect()
    }

    #[test]
    fn macros_expand_recursively() {
        let t = machine();
        def_macro(t, cs(b'a'), CALL, &[], &[LETTER_TOKEN + 120, cs_tok(b'b'), LETTER_TOKEN + 122]);
        def_macro(t, cs(b'b'), CALL, &[], &[LETTER_TOKEN + 121]);
        feed(t, &[cs_tok(b'a')]);
        assert_eq!(expanded(t), lets(b"xyz"));
        assert_eq!(t.input_ptr, 0);
        Tex::dealloc(t);
    }

    #[test]
    fn expand_after() {
        let t = machine();
        t.set_eq_type(cs(b'E'), EXPAND_AFTER);
        def_macro(t, cs(b'a'), CALL, &[MATCH_TOKEN + 35], &[LETTER_TOKEN + 40, OUT_PARAM_TOKEN + 1]);
        def_macro(t, cs(b'b'), CALL, &[], &[LETTER_TOKEN + 120]);
        // \expandafter\a\b gives \a x.
        feed(t, &[cs_tok(b'E'), cs_tok(b'a'), cs_tok(b'b')]);
        assert_eq!(expanded(t), vec![LETTER_TOKEN + 40, LETTER_TOKEN + 120]);
        // With an unexpandable second token, both are simply reread in order.
        feed(t, &[cs_tok(b'E'), LETTER_TOKEN + 97, LETTER_TOKEN + 98]);
        assert_eq!(expanded(t), lets(b"ab"));
        assert_eq!(t.expand_depth_count, 0);
        Tex::dealloc(t);
    }

    #[test]
    fn noexpand() {
        let t = machine();
        t.set_eq_type(FROZEN_DONT_EXPAND, DONT_EXPAND);
        t.set_eq_type(cs(b'N'), NO_EXPAND);
        def_macro(t, cs(b'a'), CALL, &[], &[LETTER_TOKEN + 120]);
        // \noexpand\a: a \relax look-alike that remembers \a.
        feed(t, &[cs_tok(b'N'), cs_tok(b'a'), LETTER_TOKEN + 98]);
        t.get_x_token();
        assert_eq!((t.cur_cmd, t.cur_chr, t.cur_cs, t.cur_tok), (RELAX, NO_EXPAND_FLAG, cs(b'a'), cs_tok(b'a')));
        t.get_x_token();
        assert_eq!(t.cur_tok, LETTER_TOKEN + 98);
        // \noexpand of a character: just the character, no marker.
        feed(t, &[cs_tok(b'N'), LETTER_TOKEN + 97]);
        t.get_x_token();
        assert_eq!((t.cur_cmd, t.cur_chr), (LETTER, 97));
        // \noexpand\endwrite: no marker either.
        t.set_eq_type(END_WRITE, OUTER_CALL);
        t.set_mode(VMODE);
        feed(t, &[cs_tok(b'N'), END_WRITE_TOKEN]);
        t.get_next();
        t.expand();
        assert_eq!(t.toks[t.loc()], END_WRITE_TOKEN);
        // \noexpand clears scanner_status for an \outer token, then restores it.
        t.set_eq_type(cs(b'o'), OUTER_CALL);
        t.scanner_status = MATCHING;
        feed(t, &[cs_tok(b'N'), cs_tok(b'o')]);
        t.get_next();
        t.expand();
        assert_eq!(t.scanner_status, MATCHING);
        t.scanner_status = NORMAL;
        Tex::dealloc(t);
    }

    #[test]
    fn csname_finds_a_one_letter_name() {
        let t = machine();
        t.set_eq_type(cs(b'C'), CS_NAME);
        t.set_eq_type(cs(b'c'), END_CS_NAME);
        t.set_eq_type(cs(b'q'), CHAR_GIVEN);
        t.set_equiv(cs(b'q'), 66);
        def_macro(t, cs(b'n'), CALL, &[], &[LETTER_TOKEN + 113]);
        // \csname\n\endcsname: the name comes from expansion.
        feed(t, &[cs_tok(b'C'), cs_tok(b'n'), cs_tok(b'c'), LETTER_TOKEN + 122]);
        t.get_x_token();
        assert_eq!((t.cur_cmd, t.cur_chr, t.cur_cs), (CHAR_GIVEN, 66, cs(b'q')));
        assert!(!t.is_in_csname);
        assert_eq!(t.tx_ptr, 0);
        assert_eq!(t.tb_ptr, 0, "the builder is not touched");
        t.get_x_token();
        assert_eq!(t.cur_tok, LETTER_TOKEN + 122);
        // \csname\endcsname is the null control sequence.
        feed(t, &[cs_tok(b'C'), cs_tok(b'c')]);
        t.get_x_token();
        assert_eq!(t.cur_cs, NULL_CS);
        assert_eq!(t.first, 1, "first is not disturbed");
        Tex::dealloc(t);
    }

    #[test]
    fn csname_nests_and_leaves_a_list_in_flight_alone() {
        let t = machine();
        t.set_eq_type(cs(b'C'), CS_NAME);
        t.set_eq_type(cs(b'c'), END_CS_NAME);
        // \def\A{a}; \def\a{...} is chosen by the inner name.  \csname\csname A\endcsname\endcsname
        // reads the inner one first: \A is expanded by the outer's get_x_token.
        def_macro(t, cs(b'A'), CALL, &[], &[LETTER_TOKEN + 97]);
        t.set_eq_type(cs(b'a'), CHAR_GIVEN);
        t.set_equiv(cs(b'a'), 7);
        let m = t.tb_mark();
        t.tb_push(1000);
        t.tb_def_mark = m;
        feed(t, &[cs_tok(b'C'), cs_tok(b'C'), LETTER_TOKEN + 65, cs_tok(b'c'), cs_tok(b'c')]);
        t.get_x_token();
        assert_eq!((t.cur_cmd, t.cur_chr, t.cur_cs), (CHAR_GIVEN, 7, cs(b'a')));
        assert_eq!(t.tb_len(m), 1);
        assert_eq!(t.tb[m], 1000);
        t.tb_discard(m);
        Tex::dealloc(t);
    }

    #[test]
    fn end_template_becomes_endv() {
        let t = machine();
        t.set_eq_type(FROZEN_ENDV, ENDV);
        t.set_equiv(FROZEN_ENDV, NULL_LIST);
        t.set_eq_type(cs(b't'), END_TEMPLATE);
        t.set_equiv(cs(b't'), NULL_LIST);
        // get_x_token's own shortcut, §406.
        feed(t, &[cs_tok(b't')]);
        t.get_x_token();
        assert_eq!((t.cur_cmd, t.cur_cs, t.cur_tok), (ENDV, FROZEN_ENDV, CS_TOKEN_FLAG + FROZEN_ENDV));
        // expand's way, §401.
        feed(t, &[cs_tok(b't')]);
        t.get_next();
        t.expand();
        t.get_next();
        assert_eq!((t.cur_cmd, t.cur_cs), (ENDV, FROZEN_ENDV));
        Tex::dealloc(t);
    }

    #[test]
    fn insert_relax_backs_up_two_tokens() {
        let t = machine();
        t.set_eq_type(FROZEN_RELAX, RELAX);
        t.cur_cs = cs(b'x');
        t.insert_relax();
        assert_eq!(t.token_type(), INSERTED);
        assert_eq!(drain(t, 0), vec![CS_TOKEN_FLAG + FROZEN_RELAX, cs_tok(b'x')]);
        Tex::dealloc(t);
    }

    /// `\\edef\\x{aa\\ifnum -1<\\pdfprimitive\\fi 2 bb\\fi cc}`: §394 leaves `cur_cs=0`, so §405 backs
    /// up `cs_token_flag+0`, whose reread reads `eqtb[0]`.  WEB gets the token 0, which here would
    /// end the list.  Reading it must give cmd 0 and a token that can be stored.
    #[test]
    fn insert_relax_with_no_control_sequence_stores_no_terminator() {
        let t = machine();
        t.set_eq_type(FROZEN_RELAX, RELAX);
        t.cur_cs = 0;
        t.insert_relax();
        t.get_token();
        assert_eq!(t.cur_tok, CS_TOKEN_FLAG + FROZEN_RELAX);
        t.get_token();
        assert_eq!((t.cur_cmd, t.cur_cs), (RELAX, 0));
        assert_ne!(t.cur_tok, 0);
        assert_eq!(t.cur_tok, 1);
        let bad = t.cur_tok;
        let m = t.tb_mark();
        t.tb_push(LETTER_TOKEN + 97);
        t.tb_push(bad);
        t.tb_push(LETTER_TOKEN + 98);
        let p = t.tb_commit(m);
        assert_eq!(t.tok_len(p), 3, "the list goes on past that token");
        // And rereading it from the list gives the same.
        t.ins_list(p);
        assert_eq!(drain(t, bad), vec![LETTER_TOKEN + 97]);
        assert_eq!((t.cur_cmd, t.cur_cs, t.cur_tok), (RELAX, 0, bad));
        Tex::dealloc(t);
    }

    /// The oracle's runaway for that input shows `->aa\\relax \\BAD.2 bbcc}`.
    #[test]
    fn the_token_from_no_control_sequence_shows_as_the_oracle() {
        let t = machine();
        print_setup(t);
        let m = t.tb_mark();
        t.tb_push(LETTER_TOKEN + 97);
        t.tb_push(1);
        t.tb_push(OTHER_TOKEN + 50);
        let p = t.tb_commit(m);
        t.show_token_list(p + 1, NULL, 100);
        assert_eq!(printed(t), "a\\BAD.2");
        Tex::dealloc(t);
    }

    #[test]
    fn marks_go_in_as_mark_text() {
        let t = machine();
        t.set_eq_type(cs(b'T'), TOP_BOT_MARK);
        t.set_equiv(cs(b'T'), BOT_MARK_CODE);
        let p = t.tok_two(LETTER_TOKEN + 97, LETTER_TOKEN + 98);
        t.cur_mark[BOT_MARK_CODE] = p;
        feed(t, &[cs_tok(b'T')]);
        assert_eq!(expanded(t), lets(b"ab"));
        assert_eq!(t.token_ref_count(p), 0, "the reference taken for the level was given back");
        assert_eq!(list(t, p), lets(b"ab"), "and the mark is still alive");
        // A null mark expands to nothing.
        t.cur_mark[BOT_MARK_CODE] = NULL;
        feed(t, &[cs_tok(b'T'), LETTER_TOKEN + 99]);
        assert_eq!(expanded(t), lets(b"c"));
        Tex::dealloc(t);
    }

    #[test]
    fn endinput_only_sets_force_eof() {
        let t = machine();
        t.set_eq_type(cs(b'I'), INPUT);
        t.set_equiv(cs(b'I'), 1);
        feed(t, &[cs_tok(b'I')]);
        t.get_next();
        t.expand();
        assert!(t.force_eof);
        Tex::dealloc(t);
    }

    #[test]
    fn input_while_a_name_is_in_progress_inserts_relax() {
        let t = machine();
        t.set_eq_type(cs(b'I'), INPUT);
        t.set_eq_type(FROZEN_RELAX, RELAX);
        t.name_in_progress = true;
        feed(t, &[cs_tok(b'I')]);
        t.get_next();
        t.expand();
        assert_eq!(drain(t, 0), vec![CS_TOKEN_FLAG + FROZEN_RELAX, cs_tok(b'I')]);
        Tex::dealloc(t);
    }

    #[test]
    fn fi_before_the_condition_is_evaluated_inserts_relax() {
        let t = machine();
        t.set_eq_type(cs(b'F'), FI_OR_ELSE);
        t.set_equiv(cs(b'F'), FI_CODE);
        t.set_eq_type(FROZEN_RELAX, RELAX);
        t.if_limit = IF_CODE;
        feed(t, &[cs_tok(b'F')]);
        t.get_next();
        t.expand();
        assert_eq!(drain(t, 0), vec![CS_TOKEN_FLAG + FROZEN_RELAX, cs_tok(b'F')]);
        Tex::dealloc(t);
    }

    #[test]
    fn expand_restores_what_it_saves() {
        let t = machine();
        t.set_eq_type(cs(b'N'), NO_EXPAND);
        t.cur_val = 11;
        t.cur_val_level = 2;
        t.radix = 16;
        t.cur_order = 3;
        feed(t, &[cs_tok(b'N'), LETTER_TOKEN + 97]);
        t.get_next();
        t.expand();
        assert_eq!((t.cur_val, t.cur_val_level, t.radix, t.cur_order), (11, 2, 16, 3));
        assert_eq!(t.expand_depth_count, 0);
        Tex::dealloc(t);
    }

    /// The oracle's output for `\csname` with a missing `\endcsname`, and for the undefined control
    /// sequence: both go through `error`, with the context displayed.
    #[test]
    fn undefined_control_sequence_against_the_oracle() {
        let t = machine();
        print_setup(t);
        t.set_eq_type(cs(b'u'), UNDEFINED_CS);
        t.line = 7;
        t.in_open = 1;
        t.set_index(1);
        t.set_name(20);
        t.set_end_line_char(13);
        set_line(t, b"ab\\u cd\r", NEW_LINE);
        t.get_x_token();
        t.get_x_token();
        printed(t);
        t.get_x_token();
        assert_eq!(t.cur_tok, LETTER_TOKEN + 99, "c: \\u was reported and discarded");
        let s = printed(t);
        assert!(s.starts_with("! Undefined control sequence.\nl.7 ab\\u\n         cd\n"), "got {:?}", s);
        assert_eq!(t.history, ERROR_MESSAGE_ISSUED);
        Tex::dealloc(t);
    }

    #[test]
    fn missing_endcsname_against_the_oracle() {
        let t = machine();
        print_setup(t);
        t.set_eq_type(cs(b'r'), RELAX);
        t.line = 7;
        t.in_open = 1;
        t.set_index(1);
        t.set_name(20);
        t.set_end_line_char(13);
        define(t, b"csname", CS_NAME, 0);
        set_line(t, b"\\csname a\\r b\r", NEW_LINE);
        t.get_x_token();
        // The name was read: the one character a.  \r comes next, after \a.
        assert_eq!(t.cur_cs, cs(b'a'));
        let s = printed(t);
        let want = "! Missing \\endcsname inserted.\n<to be read again> \n                   \\r \nl.7 \\csname a\\r\n                b\n";
        assert!(s.starts_with(want), "got {:?}", s);
        t.get_x_token();
        assert_eq!(t.cur_cs, cs(b'r'));
        Tex::dealloc(t);
    }

    /// `\edef\a{x\csname y\O` with an outer `\O`: the runaway display shows what is in the definition,
    /// `->x`, and not `\csname`'s characters, as in the oracle's log.  This is why the tx stack exists.
    /// (The whole `get_x_token` would loop here, since `\O` is reread, until the real `error` stops at
    /// 100 errors; so this calls `check_outer_validity` with the state `\csname` has at that point.)
    #[test]
    fn runaway_does_not_show_csname_tokens() {
        let t = machine();
        print_setup(t);
        t.set_eq_type(cs(b'O'), OUTER_CALL);
        let m = t.tb_mark();
        t.tb_def_mark = m;
        t.tb_push(END_MATCH_TOKEN);
        t.tb_push(LETTER_TOKEN + 120);
        t.scanner_status = DEFINING;
        t.warning_index = cs(b'a');
        let x = t.tx_mark();
        t.tx_push(LETTER_TOKEN + 121);
        feed(t, &[cs_tok(b'O')]);
        t.get_next();
        assert_eq!((t.cur_cmd, t.cur_chr), (SPACER, 32), "replaced by a space: §359");
        let s = printed(t);
        assert!(s.starts_with("Runaway definition?\n->x\n! Forbidden control sequence found while scanning definition of \\a.\n<inserted text> \n                }\n<to be read again> \n                   \\O \n"), "got {:?}", s);
        t.tx_discard(x);
        t.tb_discard(m);
        t.scanner_status = NORMAL;
        Tex::dealloc(t);
    }

    #[test]
    fn unless_before_something_else_against_the_oracle() {
        let t = machine();
        print_setup(t);
        t.line = 6;
        t.in_open = 1;
        t.set_index(1);
        t.set_name(20);
        t.set_end_line_char(13);
        define(t, b"unless", EXPAND_AFTER, 1);
        set_line(t, b"\\unless a\r", NEW_LINE);
        t.get_x_token();
        assert_eq!(t.cur_tok, LETTER_TOKEN + 97, "backed up, and read again");
        let s = printed(t);
        let want = "! You can't use `\\unless' before `the letter a'.\n<to be read again> \n                   a\nl.6 \\unless a\n             \n";
        assert!(s.starts_with(want), "got {:?}", s);
        Tex::dealloc(t);
    }

    #[test]
    fn extra_fi_against_the_oracle() {
        let t = machine();
        print_setup(t);
        t.line = 6;
        t.in_open = 1;
        t.set_index(1);
        t.set_name(20);
        t.set_end_line_char(13);
        define(t, b"fi", FI_OR_ELSE, FI_CODE);
        set_line(t, b"\\fi x\r", NEW_LINE);
        t.get_x_token();
        assert_eq!(t.cur_tok, LETTER_TOKEN + 120);
        let s = printed(t);
        let want = "! Extra \\fi.\nl.6 \\fi\n        x\n";
        assert!(s.starts_with(want), "got {:?}", s);
        Tex::dealloc(t);
    }
}
