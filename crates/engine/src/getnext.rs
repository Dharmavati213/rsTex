//! §354-387 Part 24: getting the next token.
//!
//! Token lists are in `toks`, not `mem`: docs/PORTING.md, "Token lists".  `get_next` is split as in DESIGN.md
//! section 8, into `get_next_leaf` and `get_next_slow`.
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

/// The first character token that `get_next_leaf` takes with no more ado, mac_param's.  Below it are the braces, math_shift,
/// tab_mark, out_param, and 0, the end of a list.
const CHAR_LEAF_FIRST: i32 = MAC_PARAM * 256;

/// Marks a branch unlikely, and so keeps LLVM from merging its test with the one before into two `setcc`s and an `or`.
#[cold]
#[inline]
fn cold() {}

impl Tex {
    /// §358
    pub fn check_outer_validity(&mut self) {
        if self.scanner_status != NORMAL {
            self.deletions_allowed = false;
            // §359
            if self.cur_cs != 0 {
                if self.state() == TOKEN_LIST || self.name() < 1 || self.name() > 17 {
                    let p = self.tok_one(CS_TOKEN_FLAG + self.cur_cs);
                    self.back_list(p);
                }
                self.cur_cmd = SPACER;
                self.cur_chr = b' ' as i32;
            }
            if self.scanner_status > SKIPPING {
                // §360
                self.runaway();
                if self.cur_cs == 0 {
                    self.print_err_s(b"File ended");
                } else {
                    self.cur_cs = 0;
                    self.print_err_s(b"Forbidden control sequence found");
                }
                // §361
                let p = match self.scanner_status {
                    DEFINING => {
                        self.print_s(b" while scanning definition");
                        self.tok_one(RIGHT_BRACE_TOKEN + b'}' as i32)
                    }
                    MATCHING => {
                        self.print_s(b" while scanning use");
                        self.long_state = OUTER_CALL;
                        self.tok_one(self.par_token)
                    }
                    ALIGNING => {
                        self.print_s(b" while scanning preamble");
                        self.align_state = -1000000;
                        self.tok_two(CS_TOKEN_FLAG + FROZEN_CR, RIGHT_BRACE_TOKEN + b'}' as i32)
                    }
                    _ /* ABSORBING */ => {
                        self.print_s(b" while scanning text");
                        self.tok_one(RIGHT_BRACE_TOKEN + b'}' as i32)
                    }
                };
                self.ins_list(p);
                self.print_s(b" of ");
                self.sprint_cs(self.warning_index);
                self.help(&[
                    b"I suspect you have forgotten a `}', causing me",
                    b"to read past where you wanted me to stop.",
                    b"I'll try to recover; but if the error is serious,",
                    b"you'd better type `E' or `X' now and fix your file.",
                ]);
                self.error();
            } else {
                self.print_err_s(b"Incomplete ");
                self.print_cmd_chr(IF_TEST, self.cur_if);
                self.print_s(b"; all text was ignored after line ");
                self.print_int(self.skip_line);
                self.help(&[
                    b"A forbidden control sequence occurred in skipped text.",
                    b"This kind of error happens when you say `\\if...' and forget",
                    b"the matching `\\fi'. I've inserted a `\\fi'; this might work.",
                ]);
                if self.cur_cs != 0 {
                    self.cur_cs = 0;
                } else {
                    self.set_help_line(2, b"The file ended while I was skipping conditional text.");
                }
                self.cur_tok = CS_TOKEN_FLAG + FROZEN_FI;
                self.ins_error();
            }
            self.deletions_allowed = true;
        }
    }

    /// §363, for the tokens of a list that need no more than this, DESIGN.md section 8: the character tokens outside the
    /// tests of §364 and §381, the braces, and the control sequences that are neither outer nor §364's.  On the thesis
    /// that is 89% of the fetches.  The result is the token, which is §387's `cur_tok`; or 0 with nothing done, and then
    /// `get_next_slow` does it all.  There is no test of `state`: in a file level `loc` is a buffer index, and those cells of
    /// `toks` are never written, so `t` is 0 there, as at the end of a list; see `NULL_LIST` in toks.rs.
    #[inline(always)]
    pub fn get_next_leaf(&mut self) -> i32 {
        let l = self.loc();
        let t = self.toks[l];
        if t >= CS_TOKEN_FLAG {
            let cs = t - CS_TOKEN_FLAG;
            let w = self.eqtb[cs];
            let cmd = w.b0();
            // One compare for the usual meanings: from mac_param to long_call there is nothing more to do.
            if ((cmd - (CAR_RET + 1)) as u32) >= (OUTER_CALL - (CAR_RET + 1)) as u32 {
                cold();
                if cmd >= TAB_MARK {
                    return 0;
                }
            }
            self.set_loc(l + 1);
            self.cur_cs = cs;
            self.cur_cmd = cmd;
            self.cur_chr = w.rh();
            return t;
        } else if t >= CHAR_LEAF_FIRST {
            self.set_loc(l + 1);
            self.cur_cs = 0;
            self.cur_cmd = t >> 8;
            self.cur_chr = t & 255;
            return t;
        } else if ((t - LEFT_BRACE_TOKEN) as u32) < (RIGHT_BRACE_LIMIT - LEFT_BRACE_TOKEN) as u32 {
            self.set_loc(l + 1);
            self.cur_cs = 0;
            let cmd = t >> 8;
            self.cur_cmd = cmd;
            self.cur_chr = t & 255;
            // left_brace is 1 and right_brace is 2, and which of them comes next is hard to predict.
            self.align_state += 3 - 2 * cmd;
            return t;
        }
        0
    }

    /// §363: `get_next_leaf`, and what it leaves to the whole of the procedure.
    pub fn get_next(&mut self) {
        if self.get_next_leaf() == 0 {
            self.get_next_slow();
        }
    }

    /// §363 when the leaf has done nothing.  Two `goto restart`s of §379 are most of it, 14 M of the 15 M on the thesis: the end
    /// of a list, and §381.  Neither needs a frame, and after each the leaf is tried again.  `cur_cmd` and `cur_chr` are not
    /// set for §381, as nothing can see them before they are set again.  All the rest is `get_next_literal`.  No loop here:
    /// LLVM would keep a dozen fields in registers across it and save them all.
    #[inline(never)]
    pub fn get_next_slow(&mut self) {
        if self.state() == TOKEN_LIST {
            let l = self.loc();
            let t = self.toks[l];
            if t == 0 {
                self.cur_cs = 0;
                self.end_token_list_unchecked();
                if self.interrupt != 0 {
                    return self.getnext_interrupted();
                }
                if self.get_next_leaf() != 0 {
                    return;
                }
            } else if (t >> 8) == OUT_PARAM {
                self.set_loc(l + 1);
                // §381
                let p = self.param_stack[self.param_start() + (t & 255) - 1];
                self.begin_parameter_list(p);
                if self.get_next_leaf() != 0 {
                    return;
                }
            }
        }
        self.get_next_literal();
    }

    /// The rest of `end_token_list`, when `get_next_slow` finds `interrupt`, and §379's `goto restart`.
    #[cold]
    #[inline(never)]
    fn getnext_interrupted(&mut self) {
        self.check_interrupt();
        self.get_next_literal();
    }

    /// §363, literally.
    #[inline(never)]
    fn get_next_literal(&mut self) {
        'restart: loop {
            self.cur_cs = 0;
            if self.state() != TOKEN_LIST {
                // §365
                if self.getnext_file() {
                    continue 'restart;
                }
            } else {
                // §379
                let l = self.loc();
                let t = self.toks[l];
                if t != 0 {
                    self.set_loc(l + 1);
                    if t >= CS_TOKEN_FLAG {
                        let cs = t - CS_TOKEN_FLAG;
                        self.cur_cs = cs;
                        self.cur_cmd = self.eq_type(cs);
                        self.cur_chr = self.equiv(cs);
                        if self.cur_cmd >= OUTER_CALL {
                            self.getnext_outer_or_dont_expand();
                        }
                    } else {
                        // A token is never negative here, so these are `div` and `mod` by 256.
                        self.cur_cmd = t >> 8;
                        self.cur_chr = t & 255;
                        match self.cur_cmd {
                            LEFT_BRACE => self.align_state += 1,
                            RIGHT_BRACE => self.align_state -= 1,
                            OUT_PARAM => {
                                // §381
                                let p = self.param_stack[self.param_start() + self.cur_chr - 1];
                                self.begin_token_list(p, PARAMETER);
                                continue 'restart;
                            }
                            _ => {}
                        }
                    }
                } else {
                    self.end_token_list();
                    continue 'restart;
                }
            }
            // §364
            if self.cur_cmd <= CAR_RET && self.cur_cmd >= TAB_MARK && self.align_state == 0 {
                self.getnext_insert_v_template();
                continue 'restart;
            }
            return;
        }
    }

    /// §379's branch for `cur_cmd>=outer_call`, with §380.
    #[inline(never)]
    fn getnext_outer_or_dont_expand(&mut self) {
        if self.cur_cmd == DONT_EXPAND {
            // §380.  loc:=null reads as end of list, since toks[0]=0.
            self.cur_cs = self.toks[self.loc()] - CS_TOKEN_FLAG;
            self.set_loc(NULL);
            self.cur_cmd = self.eq_type(self.cur_cs);
            self.cur_chr = self.equiv(self.cur_cs);
            if self.cur_cmd > MAX_COMMAND {
                self.cur_cmd = RELAX;
                self.cur_chr = NO_EXPAND_FLAG;
            }
        } else {
            if self.cur_cs == END_WRITE && self.mode() == 0 {
                self.fatal_error(b"Unbalanced write command");
            }
            self.check_outer_validity();
        }
    }

    /// §965, which WEB uses as `goto restart` from §364: the caller restarts.
    #[inline(never)]
    fn getnext_insert_v_template(&mut self) {
        if self.scanner_status == ALIGNING || self.cur_align == NULL {
            self.fatal_error(b"(interwoven alignment preambles are not allowed)");
        }
        self.cur_cmd = self.extra_info(self.cur_align);
        self.set_extra_info(self.cur_align, self.cur_chr);
        if self.cur_cmd == OMIT {
            self.begin_token_list(OMIT_TEMPLATE, V_TEMPLATE);
        } else {
            let p = self.v_part(self.cur_align);
            self.begin_token_list(p, V_TEMPLATE);
        }
        self.align_state = 1000000;
    }

    /// §365 Input from external file.  Returns true for `goto restart`.  False is reaching the end of
    /// the module with a token; §382's `return` also gives false, since §364's test on `cur_cmd=0`
    /// then does nothing and `exit` follows.
    #[inline(never)]
    fn getnext_file(&mut self) -> bool {
        'switch: loop {
            if self.loc() <= self.limit() {
                self.cur_chr = self.buffer.at(self.loc());
                self.set_loc(self.loc() + 1);
                'reswitch: loop {
                    self.cur_cmd = self.cat_code(self.cur_chr);
                    // §366
                    match (self.state(), self.cur_cmd) {
                        // §367
                        (_, IGNORE) | (SKIP_BLANKS, SPACER) | (NEW_LINE, SPACER) => continue 'switch,
                        // §376
                        (_, ESCAPE) => {
                            'found: {
                                if self.loc() > self.limit() {
                                    self.cur_cs = NULL_CS;
                                } else {
                                    'start_cs: loop {
                                        let mut k = self.loc();
                                        self.cur_chr = self.buffer.at(k);
                                        let mut cat = self.cat_code(self.cur_chr);
                                        k += 1;
                                        if cat == LETTER {
                                            self.set_state(SKIP_BLANKS);
                                        } else if cat == SPACER {
                                            self.set_state(SKIP_BLANKS);
                                        } else {
                                            self.set_state(MID_LINE);
                                        }
                                        if cat == LETTER && k <= self.limit() {
                                            // §378
                                            loop {
                                                self.cur_chr = self.buffer.at(k);
                                                cat = self.cat_code(self.cur_chr);
                                                k += 1;
                                                if cat != LETTER || k > self.limit() {
                                                    break;
                                                }
                                            }
                                            // §377
                                            if self.getnext_reduce_expanded(k, cat) {
                                                continue 'start_cs;
                                            }
                                            if cat != LETTER {
                                                k -= 1;
                                            }
                                            if k > self.loc() + 1 {
                                                self.cur_cs = self.id_lookup(self.loc(), k - self.loc());
                                                self.set_loc(k);
                                                break 'found;
                                            }
                                        } else if self.getnext_reduce_expanded(k, cat) {
                                            continue 'start_cs;
                                        }
                                        self.cur_cs = SINGLE_BASE + self.buffer.at(self.loc());
                                        self.set_loc(self.loc() + 1);
                                        break;
                                    }
                                }
                            }
                            // found:
                            self.cur_cmd = self.eq_type(self.cur_cs);
                            self.cur_chr = self.equiv(self.cur_cs);
                            if self.cur_cmd >= OUTER_CALL {
                                self.check_outer_validity();
                            }
                        }
                        // §375
                        (_, ACTIVE_CHAR) => {
                            self.cur_cs = self.cur_chr + ACTIVE_BASE;
                            self.cur_cmd = self.eq_type(self.cur_cs);
                            self.cur_chr = self.equiv(self.cur_cs);
                            self.set_state(MID_LINE);
                            if self.cur_cmd >= OUTER_CALL {
                                self.check_outer_validity();
                            }
                        }
                        // §374
                        (_, SUP_MARK) => {
                            if self.cur_chr == self.buffer.at(self.loc()) && self.loc() < self.limit() {
                                let c = self.buffer.at(self.loc() + 1);
                                if c < 0o200 {
                                    self.set_loc(self.loc() + 2);
                                    if self.is_hex(c) && self.loc() <= self.limit() {
                                        let cc = self.buffer.at(self.loc());
                                        if self.is_hex(cc) {
                                            self.set_loc(self.loc() + 1);
                                            self.hex_to_cur_chr(c, cc);
                                            continue 'reswitch;
                                        }
                                    }
                                    if c < 0o100 {
                                        self.cur_chr = c + 0o100;
                                    } else {
                                        self.cur_chr = c - 0o100;
                                    }
                                    continue 'reswitch;
                                }
                            }
                            self.set_state(MID_LINE);
                        }
                        // §368
                        (_, INVALID_CHAR) => {
                            self.print_err_s(b"Text line contains an invalid character");
                            self.help(&[
                                b"A funny symbol that I can't read has just been input.",
                                b"Continue, and I'll forget that it ever happened.",
                            ]);
                            self.deletions_allowed = false;
                            self.error();
                            self.deletions_allowed = true;
                            return true;
                        }
                        // §369; §371
                        (MID_LINE, SPACER) => {
                            self.set_state(SKIP_BLANKS);
                            self.cur_chr = b' ' as i32;
                        }
                        // §370
                        (MID_LINE, CAR_RET) => {
                            self.set_loc(self.limit() + 1);
                            self.cur_cmd = SPACER;
                            self.cur_chr = b' ' as i32;
                        }
                        // §372
                        (SKIP_BLANKS, CAR_RET) | (_, COMMENT) => {
                            self.set_loc(self.limit() + 1);
                            continue 'switch;
                        }
                        // §373
                        (NEW_LINE, CAR_RET) => {
                            self.set_loc(self.limit() + 1);
                            self.cur_cs = self.par_loc;
                            self.cur_cmd = self.eq_type(self.cur_cs);
                            self.cur_chr = self.equiv(self.cur_cs);
                            if self.cur_cmd >= OUTER_CALL {
                                self.check_outer_validity();
                            }
                        }
                        (MID_LINE, LEFT_BRACE) => self.align_state += 1,
                        (SKIP_BLANKS, LEFT_BRACE) | (NEW_LINE, LEFT_BRACE) => {
                            self.set_state(MID_LINE);
                            self.align_state += 1;
                        }
                        (MID_LINE, RIGHT_BRACE) => self.align_state -= 1,
                        (SKIP_BLANKS, RIGHT_BRACE) | (NEW_LINE, RIGHT_BRACE) => {
                            self.set_state(MID_LINE);
                            self.align_state -= 1;
                        }
                        (SKIP_BLANKS | NEW_LINE, MATH_SHIFT | TAB_MARK | MAC_PARAM | SUB_MARK | LETTER | OTHER_CHAR) => {
                            self.set_state(MID_LINE);
                        }
                        _ => {}
                    }
                    return false;
                }
            } else {
                self.set_state(NEW_LINE);
                // §382
                if self.name() > 17 {
                    // §384
                    self.line += 1;
                    self.first = self.start();
                    if !self.force_eof {
                        if self.name() <= 19 {
                            if self.pseudo_input() {
                                self.firm_up_the_line();
                            } else if self.every_eof() != NULL && !self.eof_seen[self.index()] {
                                self.set_limit(self.first - 1);
                                let k = self.index();
                                self.eof_seen[k] = true;
                                self.begin_token_list(self.every_eof(), EVERY_EOF_TEXT);
                                return true;
                            } else {
                                self.force_eof = true;
                            }
                        } else if self.input_ln(self.cur_file(), true) {
                            self.firm_up_the_line();
                        } else if self.every_eof() != NULL && !self.eof_seen[self.index()] {
                            self.set_limit(self.first - 1);
                            let k = self.index();
                            self.eof_seen[k] = true;
                            self.begin_token_list(self.every_eof(), EVERY_EOF_TEXT);
                            return true;
                        } else {
                            self.force_eof = true;
                        }
                    }
                    if self.force_eof {
                        if self.tracing_nesting() > 0 {
                            if self.grp_stack[self.in_open] != self.cur_boundary
                                || self.if_stack[self.in_open] != self.cond_ptr
                            {
                                self.file_warning();
                            }
                        }
                        if self.name() >= 19 {
                            self.print_char(b')' as i32);
                            self.open_parens -= 1;
                            self.update_terminal();
                        }
                        self.force_eof = false;
                        self.end_file_reading();
                        self.check_outer_validity();
                        return true;
                    }
                    if self.end_line_char_inactive() {
                        self.set_limit(self.limit() - 1);
                    } else {
                        self.buffer.put(self.limit(), self.end_line_char());
                    }
                    self.first = self.limit() + 1;
                    self.set_loc(self.start());
                } else {
                    if !self.terminal_input() {
                        self.cur_cmd = 0;
                        self.cur_chr = 0;
                        return false;
                    }
                    if self.input_ptr > 0 {
                        self.end_file_reading();
                        return true;
                    }
                    if self.selector < LOG_ONLY {
                        self.open_log_file();
                    }
                    if self.interaction > NONSTOP_MODE {
                        if self.end_line_char_inactive() {
                            self.set_limit(self.limit() + 1);
                        }
                        if self.limit() == self.start() {
                            self.print_nl_s(b"(Please type a command or say `\\end')");
                        }
                        self.print_ln();
                        self.first = self.start();
                        self.prompt_input_s(b"*");
                        self.set_limit(self.last);
                        if self.end_line_char_inactive() {
                            self.set_limit(self.limit() - 1);
                        } else {
                            self.buffer.put(self.limit(), self.end_line_char());
                        }
                        self.first = self.limit() + 1;
                        self.set_loc(self.start());
                    } else {
                        self.fatal_error(b"*** (job aborted, no legal \\end found)");
                    }
                }
                self.check_interrupt();
                // goto switch
            }
        }
    }

    /// §374 `hex_to_cur_chr`.
    fn hex_to_cur_chr(&mut self, c: i32, cc: i32) {
        if c <= b'9' as i32 {
            self.cur_chr = c - b'0' as i32;
        } else {
            self.cur_chr = c - b'a' as i32 + 10;
        }
        if cc <= b'9' as i32 {
            self.cur_chr = 16 * self.cur_chr + cc - b'0' as i32;
        } else {
            self.cur_chr = 16 * self.cur_chr + cc - b'a' as i32 + 10;
        }
    }

    /// §377 If an expanded code is present, reduce it.  Returns true for `goto start_cs`.
    fn getnext_reduce_expanded(&mut self, k: i32, cat: i32) -> bool {
        let mut k = k;
        if self.buffer.at(k) == self.cur_chr && cat == SUP_MARK && k < self.limit() {
            let c = self.buffer.at(k + 1);
            if c < 0o200 {
                let mut d = 2;
                let mut cc = 0;
                if self.is_hex(c) && k + 2 <= self.limit() {
                    cc = self.buffer.at(k + 2);
                    if self.is_hex(cc) {
                        d += 1;
                    }
                }
                if d > 2 {
                    self.hex_to_cur_chr(c, cc);
                    self.buffer.put(k - 1, self.cur_chr);
                } else if c < 0o100 {
                    self.buffer.put(k - 1, c + 0o100);
                } else {
                    self.buffer.put(k - 1, c - 0o100);
                }
                self.set_limit(self.limit() - d);
                self.first -= d;
                while k <= self.limit() {
                    let b = self.buffer.at(k + d);
                    self.buffer.put(k, b);
                    k += 1;
                }
                return true;
            }
        }
        false
    }

    /// §385
    pub fn firm_up_the_line(&mut self) {
        self.set_limit(self.last);
        if self.pausing() > 0 {
            if self.interaction > NONSTOP_MODE {
                self.wake_up_terminal();
                self.print_ln();
                if self.start() < self.limit() {
                    for k in self.start()..self.limit() {
                        self.print(self.buffer.at(k));
                    }
                }
                self.first = self.limit();
                self.prompt_input_s(b"=>");
                if self.last > self.first {
                    for k in self.first..self.last {
                        let b = self.buffer.at(k);
                        self.buffer.put(k + self.start() - self.first, b);
                    }
                    self.set_limit(self.start() + self.last - self.first);
                }
            }
        }
    }

    /// §387.  Only §365 looks a name up, so the leaf can leave `no_new_control_sequence` alone.  It is true here, for it is
    /// false only during `get_token_slow`, and from there the one way back to get_token is §88, which calls `get_token_slow`.
    pub fn get_token(&mut self) {
        #[cfg(feature = "checked")]
        assert!(self.no_new_control_sequence);
        let t = self.get_next_leaf();
        if t != 0 {
            self.cur_tok = t;
        } else {
            self.get_token_slow();
        }
    }

    /// §387, literally.
    #[inline(never)]
    pub fn get_token_slow(&mut self) {
        self.no_new_control_sequence = false;
        self.get_next_slow();
        self.no_new_control_sequence = true;
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

    /// Lines of a table from tests/oracle/gen_token_machine_tables.py.
    fn table(data: &'static [u8]) -> Vec<Vec<Vec<u8>>> {
        data.split(|&c| c == b'\n')
            .filter(|l| !l.is_empty())
            .map(|l| l.split(|&c| c == b'\t').map(|f| f.iter().map(|&c| if c == 0x0B { b'\n' } else { c }).collect()).collect())
            .collect()
    }

    #[test]
    fn characters_from_a_token_list() {
        let t = machine();
        feed(t, &[LETTER_TOKEN + 97, OTHER_TOKEN + 49, SPACE_TOKEN, MATH_SHIFT_TOKEN + 36]);
        let mut got = Vec::new();
        for _ in 0..4 {
            t.get_next();
            got.push((t.cur_cmd, t.cur_chr, t.cur_cs));
        }
        assert_eq!(got, vec![(LETTER, 97, 0), (OTHER_CHAR, 49, 0), (SPACER, 32, 0), (MATH_SHIFT, 36, 0)]);
        Tex::dealloc(t);
    }

    #[test]
    fn braces_move_align_state_in_both_directions() {
        let t = machine();
        feed(t, &[LEFT_BRACE_TOKEN + 123, LEFT_BRACE_TOKEN + 123, RIGHT_BRACE_TOKEN + 125]);
        t.get_next();
        assert_eq!(t.align_state, 1000001);
        t.get_next();
        assert_eq!(t.align_state, 1000002);
        t.get_next();
        assert_eq!(t.align_state, 1000001);
        assert_eq!((t.cur_cmd, t.cur_chr), (RIGHT_BRACE, 125));
        Tex::dealloc(t);
    }

    #[test]
    fn control_sequence_reads_its_meaning() {
        let t = machine();
        t.set_eq_type(cs(b'x'), CHAR_GIVEN);
        t.set_equiv(cs(b'x'), 77);
        feed(t, &[cs_tok(b'x')]);
        t.get_next();
        assert_eq!((t.cur_cmd, t.cur_chr, t.cur_cs), (CHAR_GIVEN, 77, cs(b'x')));
        Tex::dealloc(t);
    }

    #[test]
    fn get_token_packs_cur_tok() {
        let t = machine();
        feed(t, &[LETTER_TOKEN + 97, cs_tok(b'x')]);
        t.get_token();
        assert_eq!(t.cur_tok, LETTER_TOKEN + 97);
        assert!(t.no_new_control_sequence, "true again after get_token");
        t.get_token();
        assert_eq!(t.cur_tok, cs_tok(b'x'));
        Tex::dealloc(t);
    }

    /// All that §363 can set or change.
    fn registers(t: &Tex) -> (i32, i32, i32, i32, i32, i32, i32) {
        (t.cur_cmd, t.cur_chr, t.cur_cs, t.loc(), t.input_ptr, t.align_state, t.param_ptr)
    }

    /// A machine in which each of `cmds` is the meaning of its own one-letter control sequence.
    fn meanings(cmds: &[i32]) -> &'static mut Tex {
        let t = machine();
        for (k, &c) in cmds.iter().enumerate() {
            t.set_eq_type(cs(b'a' + k as u8), c);
            t.set_equiv(cs(b'a' + k as u8), 300 + k as i32);
        }
        t
    }

    /// Everything the leaf does and everything it leaves, against §363 itself, in one list.
    #[test]
    fn get_next_agrees_with_the_literal_procedure() {
        let cmds = [
            RELAX, LEFT_BRACE, RIGHT_BRACE, MATH_SHIFT, TAB_MARK, CAR_RET, MAC_PARAM, OTHER_CHAR, MAX_COMMAND, UNDEFINED_CS, EXPAND_AFTER,
            CALL, LONG_CALL, OUTER_CALL, LONG_OUTER_CALL, END_TEMPLATE,
        ];
        let mut toks: Vec<i32> = (0..cmds.len()).map(|k| cs_tok(b'a' + k as u8)).collect();
        for cmd in [LEFT_BRACE, RIGHT_BRACE, MATH_SHIFT, TAB_MARK, MAC_PARAM, SUP_MARK, SUB_MARK, SPACER, LETTER, OTHER_CHAR, ACTIVE_CHAR] {
            toks.push(cmd * 256 + 65);
        }
        toks.push(OUT_PARAM_TOKEN + 1); // an argument of one token
        toks.push(OUT_PARAM_TOKEN + 2); // an empty one
        toks.push(LETTER_TOKEN + 65);
        let set_up = |t: &mut Tex| {
            let a1 = t.tok_one(LETTER_TOKEN + 66);
            t.param_stack[0] = a1;
            t.param_stack[1] = NULL;
            t.param_ptr = 2;
            let m = t.tb_mark();
            for &x in &toks {
                t.tb_push(x);
            }
            let p = t.tb_commit(m);
            t.begin_token_list(p, PARAMETER);
            t.set_limit(0);
        };
        let (a, b) = (meanings(&cmds), meanings(&cmds));
        set_up(a);
        set_up(b);
        for k in 0.. {
            a.get_next();
            b.get_next_literal();
            assert_eq!(registers(a), registers(b), "token {k}");
            if a.input_ptr == 0 {
                break;
            }
        }
        Tex::dealloc(a);
        Tex::dealloc(b);
    }

    /// What is not for the leaf, it leaves as it was.
    #[test]
    fn the_leaf_does_nothing_with_what_it_leaves() {
        let cmds = [TAB_MARK, CAR_RET, OUTER_CALL, LONG_OUTER_CALL, END_TEMPLATE, DONT_EXPAND];
        let t = meanings(&cmds);
        let mut toks: Vec<i32> = (0..cmds.len()).map(|k| cs_tok(b'a' + k as u8)).collect();
        toks.extend([MATH_SHIFT_TOKEN + 36, TAB_TOKEN + 38, OUT_PARAM_TOKEN + 1]);
        for x in toks.into_iter().chain([0]) {
            if x != 0 {
                feed(t, &[x, LETTER_TOKEN + 97]);
            } else {
                feed(t, &[]);
            }
            t.cur_cmd = 99;
            t.cur_chr = 98;
            t.cur_cs = 97;
            let before = registers(t);
            assert_eq!(t.get_next_leaf(), 0, "token {x}");
            assert_eq!(registers(t), before, "token {x}");
            t.end_token_list();
        }
        Tex::dealloc(t);
    }

    /// The leaf has no test of `state`, so `loc` in a file level must read as the end of a list, for any line.
    #[test]
    fn the_leaf_leaves_a_file_level() {
        let t = machine();
        set_line(t, b"a", MID_LINE);
        let before = registers(t);
        assert_eq!(t.get_next_leaf(), 0);
        assert_eq!(registers(t), before);
        t.set_loc(BUF_CAP as i32);
        assert_eq!(t.get_next_leaf(), 0, "the last cell a buffer index can name");
        t.set_loc(t.start());
        t.get_next();
        assert_eq!((t.cur_cmd, t.cur_chr), (LETTER, 97));
        assert_eq!(t.state(), MID_LINE);
        Tex::dealloc(t);
    }

    /// `get_next_slow` meets `interrupt` at the end of a list.  With `OK_to_interrupt` false the pause waits.
    #[test]
    fn the_end_of_a_list_checks_for_an_interrupt() {
        let t = machine();
        t.OK_to_interrupt = false;
        t.interrupt = 1;
        feed(t, &[LETTER_TOKEN + 98]);
        feed(t, &[]);
        t.get_next();
        assert_eq!(t.cur_chr, 98);
        assert_eq!(t.interrupt, 1);
        Tex::dealloc(t);
    }

    /// `get_token`'s leaf leaves `no_new_control_sequence`.  §88 can come here with it false, and leaves it true.
    #[test]
    fn get_token_slow_sets_no_new_control_sequence_in_any_case() {
        let t = machine();
        feed(t, &[LETTER_TOKEN + 97, LETTER_TOKEN + 98]);
        t.no_new_control_sequence = false;
        t.get_token_slow();
        assert_eq!(t.cur_tok, LETTER_TOKEN + 97);
        assert!(t.no_new_control_sequence);
        t.get_token();
        assert_eq!(t.cur_tok, LETTER_TOKEN + 98);
        assert!(t.no_new_control_sequence);
        Tex::dealloc(t);
    }

    /// From the end of a list down to a file level: a new name goes in the table for get_token, not for get_next.
    #[test]
    fn a_name_after_the_end_of_a_list() {
        let t = machine();
        print_setup(t);
        set_line(t, b"\\abc \\abc ", MID_LINE);
        feed(t, &[]);
        t.get_next();
        assert_eq!(t.cur_cs, UNDEFINED_CONTROL_SEQUENCE);
        assert_eq!(t.input_ptr, 0);
        feed(t, &[]);
        t.get_token();
        assert!(t.cur_cs >= HASH_BASE && t.cur_cs != UNDEFINED_CONTROL_SEQUENCE);
        assert_eq!(t.cur_tok, CS_TOKEN_FLAG + t.cur_cs);
        assert!(t.no_new_control_sequence);
        Tex::dealloc(t);
    }

    #[test]
    fn end_of_list_pops_the_level_and_restarts() {
        let t = machine();
        feed(t, &[LETTER_TOKEN + 98]);
        let inner = t.tok_one(LETTER_TOKEN + 97);
        t.ins_list(inner);
        t.get_next();
        assert_eq!(t.cur_chr, 97);
        assert_eq!(t.input_ptr, 2);
        t.get_next();
        assert_eq!(t.cur_chr, 98, "the inner list ended, so this comes from the outer one");
        assert_eq!(t.input_ptr, 1);
        // Both lists were one token long, so the blocks are on the free list of class 1.
        let reused = t.tok_one(1000);
        assert_eq!(reused, inner, "the exhausted list's block was freed");
        Tex::dealloc(t);
    }

    #[test]
    fn end_of_an_empty_list() {
        let t = machine();
        feed(t, &[LETTER_TOKEN + 97]);
        t.ins_list(NULL);
        t.get_next();
        assert_eq!(t.cur_chr, 97, "ins_list(null) is one empty level");
        Tex::dealloc(t);
    }

    #[test]
    fn out_param_inserts_the_argument() {
        let t = machine();
        let a1 = t.tok_two(LETTER_TOKEN + 97, LETTER_TOKEN + 98);
        t.param_stack[0] = a1;
        t.param_stack[1] = NULL; // an empty argument
        t.param_ptr = 2;
        let h = def_macro(t, cs(b'm'), CALL, &[], &[OUT_PARAM_TOKEN + 1, OUT_PARAM_TOKEN + 2, LETTER_TOKEN + 99]);
        t.begin_token_list(h, MACRO);
        t.set_param_start(0);
        t.set_loc(h + 2);
        let got: Vec<i32> = (0..3)
            .map(|_| {
                t.get_next();
                t.cur_chr
            })
            .collect();
        assert_eq!(got, vec![97, 98, 99]);
        assert_eq!(t.token_type(), MACRO, "the empty argument's level is gone");
        t.get_next();
        assert_eq!(t.cur_cmd, 0, "the macro level ended, and flushed its parameters");
        assert_eq!(t.param_ptr, 0);
        assert_eq!(t.input_ptr, 0);
        Tex::dealloc(t);
    }

    #[test]
    fn dont_expand_marker_suppresses_expansion() {
        let t = machine();
        t.set_eq_type(FROZEN_DONT_EXPAND, DONT_EXPAND);
        let h = def_macro(t, cs(b'x'), CALL, &[], &[]);
        feed(t, &[LETTER_TOKEN + 98]);
        feed(t, &[CS_TOKEN_FLAG + FROZEN_DONT_EXPAND, cs_tok(b'x')]);
        t.get_next();
        assert_eq!((t.cur_cmd, t.cur_chr, t.cur_cs), (RELAX, NO_EXPAND_FLAG, cs(b'x')));
        assert_eq!(t.loc(), NULL, "§380's loc:=null");
        assert!(t.at_list_end());
        t.get_next();
        assert_eq!(t.cur_chr, 98, "the list ended");
        // With an unexpandable meaning the token keeps it.
        t.set_eq_type(cs(b'y'), CHAR_GIVEN);
        t.set_equiv(cs(b'y'), 5);
        feed(t, &[CS_TOKEN_FLAG + FROZEN_DONT_EXPAND, cs_tok(b'y')]);
        t.get_next();
        assert_eq!((t.cur_cmd, t.cur_chr, t.cur_cs), (CHAR_GIVEN, 5, cs(b'y')));
        t.delete_token_ref(h);
        Tex::dealloc(t);
    }

    #[test]
    fn outer_macro_passes_when_the_status_is_normal() {
        let t = machine();
        t.set_eq_type(cs(b'o'), OUTER_CALL);
        t.set_equiv(cs(b'o'), 5);
        feed(t, &[cs_tok(b'o')]);
        t.get_next();
        assert_eq!((t.cur_cmd, t.cur_chr, t.cur_cs), (OUTER_CALL, 5, cs(b'o')));
        // \endwrite in horizontal or vertical mode is only an outer macro.
        t.set_mode(VMODE);
        t.set_eq_type(END_WRITE, OUTER_CALL);
        feed(t, &[END_WRITE_TOKEN]);
        t.get_next();
        assert_eq!(t.cur_cs, END_WRITE);
        Tex::dealloc(t);
    }

    /// A template with `v_part` holding `tokens`, as `cur_align`; `extra_info` starts as `info`.
    fn template(t: &mut Tex, tokens: &[i32], info: i32) -> i32 {
        let m = t.tb_mark();
        for &x in tokens {
            t.tb_push(x);
        }
        let v = t.tb_commit(m);
        let p = 1000;
        t.set_v_part(p, v);
        t.set_extra_info(p, info);
        t.cur_align = p;
        p
    }

    #[test]
    fn tab_at_align_state_zero_fires_the_v_template() {
        let t = machine();
        let p = template(t, &[LETTER_TOKEN + 118], 42);
        t.align_state = 0;
        feed(t, &[LETTER_TOKEN + 122]);
        feed(t, &[TAB_TOKEN + 38]);
        t.get_next();
        assert_eq!((t.cur_cmd, t.cur_chr), (LETTER, 118), "the v template comes first");
        assert_eq!(t.align_state, 1000000);
        assert_eq!(t.extra_info(p), 38, "the column's end is remembered");
        assert_eq!(t.token_type(), V_TEMPLATE);
        // At the end of a v template its level is popped like any other.
        t.get_next();
        assert_eq!((t.cur_cmd, t.cur_chr), (LETTER, 122));
        Tex::dealloc(t);
    }

    #[test]
    fn omit_uses_the_omit_template() {
        let t = machine();
        let p = template(t, &[LETTER_TOKEN + 118], OMIT);
        t.set_eq_type(FROZEN_END_TEMPLATE, END_TEMPLATE);
        t.set_equiv(FROZEN_END_TEMPLATE, NULL_LIST);
        t.align_state = 0;
        feed(t, &[TAB_TOKEN + 38]);
        t.get_next();
        assert_eq!(t.cur_cs, FROZEN_END_TEMPLATE);
        assert_eq!(t.cur_cmd, END_TEMPLATE);
        assert_eq!(t.extra_info(p), 38);
        Tex::dealloc(t);
    }

    #[test]
    fn tab_elsewhere_is_only_a_token() {
        let t = machine();
        t.align_state = 1;
        feed(t, &[TAB_TOKEN + 38]);
        t.get_next();
        assert_eq!((t.cur_cmd, t.cur_chr), (TAB_MARK, 38));
        Tex::dealloc(t);
    }

    #[test]
    fn cr_with_noexpand_also_fires_the_v_template() {
        let t = machine();
        t.set_eq_type(FROZEN_DONT_EXPAND, DONT_EXPAND);
        t.set_eq_type(cs(b'c'), CAR_RET);
        t.set_equiv(cs(b'c'), CR_CODE);
        let p = template(t, &[LETTER_TOKEN + 118], 42);
        t.align_state = 0;
        feed(t, &[CS_TOKEN_FLAG + FROZEN_DONT_EXPAND, cs_tok(b'c')]);
        t.get_next();
        assert_eq!((t.cur_cmd, t.cur_chr), (LETTER, 118), "§380 falls through to §364");
        assert_eq!(t.extra_info(p), CR_CODE);
        Tex::dealloc(t);
    }

    #[test]
    fn a_read_line_ends_with_all_zero() {
        let t = machine();
        t.cur_cs = 99;
        t.cur_chr = 99;
        t.cur_cmd = 99;
        t.get_token();
        assert_eq!((t.cur_cmd, t.cur_chr, t.cur_cs, t.cur_tok), (0, 0, 0, 0));
        assert_eq!(t.state(), NEW_LINE);
        Tex::dealloc(t);
    }

    #[test]
    fn states_of_a_line() {
        let t = machine();
        set_line(t, b"  a  b{", NEW_LINE);
        t.get_next();
        assert_eq!((t.cur_cmd, t.cur_chr), (LETTER, 97), "spaces are skipped in new_line state");
        assert_eq!(t.state(), MID_LINE);
        t.get_next();
        assert_eq!((t.cur_cmd, t.cur_chr), (SPACER, 32));
        assert_eq!(t.state(), SKIP_BLANKS);
        t.get_next();
        assert_eq!(t.cur_chr, 98, "the second space is skipped");
        assert_eq!(t.state(), MID_LINE);
        t.get_next();
        assert_eq!(t.cur_cmd, LEFT_BRACE);
        assert_eq!(t.align_state, 1000001);
        Tex::dealloc(t);
    }

    #[test]
    fn expanded_character_shortens_the_line() {
        let t = machine();
        t.first = 10;
        set_line(t, b"\\^^41.", NEW_LINE);
        let (limit, first) = (t.limit(), t.first);
        t.get_next();
        assert_eq!(t.cur_cs, cs(b'A'), "§377 reduces ^^41 inside a control sequence name");
        assert_eq!(t.limit(), limit - 3);
        assert_eq!(t.first, first - 3);
        assert_eq!(t.buffer.at(t.loc()), b'.' as i32);
        Tex::dealloc(t);
    }

    /// The text of a tokenize case in a file, as the oracle read it: `\g ` first and `\E` on the next
    /// line, standing for `\grab ` and `\endtest`.
    fn tokenize_case(t: &mut Tex, text: &[u8]) -> Vec<u8> {
        let mut data = b"\\g ".to_vec();
        data.extend_from_slice(text);
        data.extend_from_slice(b"\n\\E\n");
        t.host.files[input_file(1) as usize] = Slot::In(InFile { path: Vec::new(), data, pos: 0 });
        t.first = 1;
        t.set_start(1);
        t.set_loc(1);
        t.set_limit(0);
        t.set_index(1);
        t.in_open = 1;
        t.set_name(20);
        t.set_state(NEW_LINE);
        t.set_end_line_char(13);
        t.get_token();
        assert_eq!(t.cur_tok, cs_tok(b'g'));
        let toks = drain(t, cs_tok(b'E'));
        show(t, &toks)
    }

    #[test]
    fn tokenize_against_the_oracle() {
        let rows = table(include_bytes!("../testdata/tokenize_oracle.tsv"));
        assert!(rows.len() > 40);
        let mut n = 0;
        for r in &rows {
            if r[1] == b"1" {
                continue;
            }
            let t = machine();
            let got = tokenize_case(t, &r[0]);
            assert_eq!(
                String::from_utf8_lossy(&got),
                String::from_utf8_lossy(&r[2]),
                "source {:?}",
                String::from_utf8_lossy(&r[0])
            );
            Tex::dealloc(t);
            n += 1;
        }
        assert!(n > 40);
    }

    /// Needs `id_lookup`, in hash.rs.
    #[test]
    fn tokenize_multiletter_against_the_oracle() {
        let rows = table(include_bytes!("../testdata/tokenize_oracle.tsv"));
        for r in &rows {
            if r[1] == b"0" {
                continue;
            }
            let t = machine();
            print_setup(t);
            let got = tokenize_case(t, &r[0]);
            assert_eq!(
                String::from_utf8_lossy(&got),
                String::from_utf8_lossy(&r[2]),
                "source {:?}",
                String::from_utf8_lossy(&r[0])
            );
            Tex::dealloc(t);
        }
    }

    // -------------------------------------------------------------------------------------------
    // The error paths print.  Each expectation is what the oracle's log shows for the same line.
    // -------------------------------------------------------------------------------------------

    fn error_machine() -> &'static mut Tex {
        let t = machine();
        print_setup(t);
        t.line = 7;
        t.in_open = 1;
        t.set_index(1);
        t.set_name(20);
        t.set_end_line_char(13);
        t
    }

    #[test]
    fn invalid_character() {
        let t = error_machine();
        set_line(t, b"a^^?b\r", NEW_LINE);
        t.get_token();
        assert_eq!(t.cur_tok, LETTER_TOKEN + 97);
        printed(t);
        t.get_token();
        assert_eq!(t.cur_tok, LETTER_TOKEN + 98, "§368 goes to restart");
        assert!(t.deletions_allowed);
        let s = printed(t);
        let want = "! Text line contains an invalid character.\nl.7 a^^?\n        b\n";
        assert!(s.starts_with(want), "got {:?}", s);
        Tex::dealloc(t);
    }

    #[test]
    fn incomplete_conditional() {
        let t = error_machine();
        t.set_eq_type(cs(b'O'), OUTER_CALL);
        set_line(t, b"\\iffalse \\O\r", NEW_LINE);
        t.set_loc(t.start() + 9);
        t.scanner_status = SKIPPING;
        t.cur_if = IF_FALSE_CODE;
        t.skip_line = 7;
        let s = t.lit_string(b"fi");
        t.set_text(FROZEN_FI, s);
        t.get_token();
        // \O was replaced by a space, as a token of the skipped text.
        assert_eq!((t.cur_cmd, t.cur_chr, t.cur_cs), (SPACER, 32, 0));
        let s = printed(t);
        let want = "! Incomplete \\iffalse; all text was ignored after line 7.\n<inserted text> \n                \\fi \n<to be read again> \n                   \\O \nl.7 \\iffalse \\O\n               \n";
        assert!(s.starts_with(want), "got {:?}", s);
        assert!(t.deletions_allowed);
        assert_eq!(t.align_state, 1000000);
        Tex::dealloc(t);
    }

    #[test]
    fn file_ended_while_defining() {
        let t = machine();
        print_setup(t);
        // The oracle's \def\a{xy at the end of eof.tex: line 7 of a file, under <*> eof.tex.
        t.set_end_line_char(13);
        t.first = 1;
        t.set_name(0);
        set_line(t, b"eof.tex", MID_LINE);
        t.set_loc(t.limit() + 1);
        t.first = t.limit() + 1;
        t.begin_file_reading();
        t.set_name(20);
        t.set_limit(t.start() - 1);
        t.set_loc(t.start());
        t.set_state(NEW_LINE);
        let m = t.tb_mark();
        t.tb_def_mark = m;
        t.tb_push(END_MATCH_TOKEN);
        t.tb_push(LETTER_TOKEN + 120);
        t.tb_push(LETTER_TOKEN + 121);
        t.tb_push(SPACE_TOKEN);
        t.scanner_status = DEFINING;
        t.warning_index = cs(b'a');
        t.get_token();
        assert_eq!(t.cur_tok, RIGHT_BRACE_TOKEN + 125, "the inserted right brace");
        let s = printed(t);
        let want = ")\nRunaway definition?\n->xy \n! File ended while scanning definition of \\a.\n<inserted text> \n                }\n<*> eof.tex\n           \n";
        assert!(s.starts_with(want), "got {:?}", s);
        t.tb_discard(m);
        t.scanner_status = NORMAL;
        Tex::dealloc(t);
    }

    /// The end of a file with `\everyeof`: §384's fake empty line, then the file really ends.
    #[test]
    fn everyeof_then_the_end_of_the_file() {
        let t = machine();
        print_setup(t);
        t.set_end_line_char(13);
        let e = t.tok_one(LETTER_TOKEN + 120);
        t.set_every_eof(e);
        t.open_parens = 1;
        t.begin_file_reading();
        t.set_name(20);
        t.set_limit(t.start() - 1);
        t.set_loc(t.start());
        t.set_state(NEW_LINE);
        t.get_token();
        assert_eq!(t.cur_tok, LETTER_TOKEN + 120);
        assert_eq!(t.token_type(), EVERY_EOF_TEXT);
        assert!(t.eof_seen[1]);
        assert_eq!(t.input_ptr, 2);
        t.get_token();
        assert_eq!(t.cur_tok, 0, "the file ended, and the level below has no more");
        assert_eq!((t.input_ptr, t.in_open, t.open_parens), (0, 0, 0));
        assert!(!t.force_eof);
        assert_eq!(t.token_ref_count(e), 0, "the list stays for the next file");
        Tex::dealloc(t);
    }

    /// Not a gate: a rough figure for the token-list branch, to compare with the 1.36 ns/token of the
    /// replay in docs/research/architecture-proposal.md.  `cargo test --release -p rstex-engine --lib --features timing --
    /// --exact getnext::tests::token_list_speed`.  This file may not use std::time, so `Instant` is not here: the
    /// test only runs the loop, and whoever runs it times the process.
    #[cfg(feature = "timing")]
    #[test]
    fn token_list_speed() {
        let t = machine();
        for k in 0..256 {
            t.set_eq_type(SINGLE_BASE + k, LETTER);
        }
        let m = t.tb_mark();
        for k in 0..1_000_000 {
            t.tb_push(if k % 4 == 0 { CS_TOKEN_FLAG + SINGLE_BASE + (k % 256) } else { LETTER_TOKEN + (k % 26) + 97 });
        }
        let p = t.tb_commit(m);
        let mut sum = 0i64;
        for _ in 0..1000 {
            t.begin_token_list(p, PARAMETER);
            loop {
                t.get_next();
                if t.cur_cmd == 0 {
                    break;
                }
                sum += t.cur_chr as i64;
            }
        }
        assert!(sum != 0);
        Tex::dealloc(t);
    }
}
