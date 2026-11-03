//! §513-536 Part 28: conditional processing.
//!
//! A procedure includes every module reachable from it, wherever defined: `tools/web.py tangle NAME`.
//! Macro texts and §1767's characters are not in `mem`: docs/PORTING.md, "Token lists".
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
    pub struct Vars {
        /// If nodes that are free again, linked by `link`, ready for §521.  They are `get_node`'s and count in
        /// `var_used` as free; `cond_give_back` returns them.  Nesting is shallow, so these are few.
        pub free_if_nodes: i32,
    }
}

/// Non-POD state private to this module, in `self.host`.  Rarely needed.
#[derive(Default)]
pub struct HVars {}

const LESS: i32 = b'<' as i32;
const EQUALS: i32 = b'=' as i32;
const GREATER: i32 = b'>' as i32;

impl Tex {
    /// §520
    pub fn pass_text(&mut self) {
        let save_scanner_status = self.scanner_status;
        self.scanner_status = SKIPPING;
        let mut l = 0;
        self.skip_line = self.line;
        loop {
            #[cfg(not(feature = "slow_capture"))]
            if self.state() == TOKEN_LIST && self.capture_skip(&mut l) {
                break;
            }
            self.get_next();
            if self.cur_cmd == FI_OR_ELSE {
                if l == 0 {
                    break;
                }
                if self.cur_chr == FI_CODE {
                    l -= 1;
                }
            } else if self.cur_cmd == IF_TEST {
                l += 1;
            }
        }
        self.scanner_status = save_scanner_status;
        if self.tracing_ifs() > 0 {
            self.show_cur_cmd_chr();
        }
    }

    /// §522 Pop the condition stack
    #[inline(always)]
    pub fn cond_pop(&mut self) {
        if self.if_stack[self.in_open] == self.cond_ptr {
            self.if_warning(); // conditionals possibly not properly nested with files
        }
        let p = self.cond_ptr;
        self.if_line = self.if_line_field(p);
        self.cur_if = self.subtype(p);
        self.if_limit = self.ty(p);
        self.cond_ptr = self.link(p);
        // free_node(p,if_node_size), kept for the next §521
        self.set_link(p, self.v_cond.free_if_nodes);
        self.v_cond.free_if_nodes = p;
        self.var_used -= IF_NODE_SIZE;
    }

    /// `get_node(if_node_size)`, from what §522 kept if it can.
    #[inline(always)]
    fn cond_get_node(&mut self) -> i32 {
        let p = self.v_cond.free_if_nodes;
        if p != NULL {
            self.v_cond.free_if_nodes = self.link(p);
            self.var_used += IF_NODE_SIZE;
            p
        } else {
            self.get_node(IF_NODE_SIZE)
        }
    }

    /// Returns what §522 kept to `free_node`, before §1483 dumps `mem` and says how much of it is used.
    pub fn cond_give_back(&mut self) {
        while self.v_cond.free_if_nodes != NULL {
            let p = self.v_cond.free_if_nodes;
            self.v_cond.free_if_nodes = self.link(p);
            self.var_used += IF_NODE_SIZE;
            self.free_node(p, IF_NODE_SIZE);
        }
    }

    /// §523
    pub fn change_if_limit(&mut self, l: i32, p: i32) {
        if p == self.cond_ptr {
            self.if_limit = l; // that's the easy case
        } else {
            let mut q = self.cond_ptr;
            loop {
                if q == NULL {
                    self.confusion(b"if");
                }
                if self.link(q) == p {
                    self.set_ty(q, l);
                    return;
                }
                q = self.link(q);
            }
        }
    }

    /// §529, §1769: the relation between the operands; prints the error and assumes `=` if it is
    /// not one of `<`, `=`, `>`.
    #[inline]
    fn cond_scan_relation(&mut self, this_if: i32) -> i32 {
        // §432 Get the next non-blank non-call token
        loop {
            self.get_x_token();
            if self.cur_cmd != SPACER {
                break;
            }
        }
        if self.cur_tok >= OTHER_TOKEN + LESS && self.cur_tok <= OTHER_TOKEN + GREATER {
            self.cur_tok - OTHER_TOKEN
        } else {
            self.print_err_s(b"Missing = inserted for ");
            self.print_cmd_chr(IF_TEST, this_if);
            self.help(&[b"I was expecting to see `<', `=', or `>'. Didn't."]);
            self.back_error();
            EQUALS
        }
    }

    /// §524.  The conditionals that are rare in every document are in `cond_other`, so that this does not save
    /// six registers for all of them.
    pub fn conditional(&mut self) {
        if self.tracing_ifs() > 0 && self.tracing_commands() <= 1 {
            self.show_cur_cmd_chr();
        }
        // §521 Push the condition stack
        let p = self.cond_get_node();
        self.set_link(p, self.cond_ptr);
        self.set_ty(p, self.if_limit);
        self.set_subtype(p, self.cur_if);
        self.set_if_line_field(p, self.if_line);
        self.cond_ptr = p;
        self.cur_if = self.cur_chr;
        self.if_limit = IF_CODE;
        self.if_line = self.line;

        let save_cond_ptr = self.cond_ptr;
        let is_unless = self.cur_chr >= UNLESS_CODE;
        let this_if = self.cur_chr & (UNLESS_CODE - 1); // cur_chr mod unless_code: it is not negative
        let mut b;
        // §527 Either process \ifcase or set b to the value of a boolean condition
        match this_if {
            IF_CHAR_CODE | IF_CAT_CODE => {
                // §532 Test if two characters match
                self.get_x_token_or_active_char();
                let (m, n);
                if self.cur_cmd > ACTIVE_CHAR || self.cur_chr > 255 {
                    // not a character
                    m = RELAX;
                    n = 256;
                } else {
                    m = self.cur_cmd;
                    n = self.cur_chr;
                }
                self.get_x_token_or_active_char();
                if self.cur_cmd > ACTIVE_CHAR || self.cur_chr > 255 {
                    self.cur_cmd = RELAX;
                    self.cur_chr = 256;
                }
                if this_if == IF_CHAR_CODE {
                    b = n == self.cur_chr;
                } else {
                    b = m == self.cur_cmd;
                }
            }
            IF_INT_CODE => {
                // §529 Test relation between integers or dimensions; the next step is to read the relation
                self.scan_int_keeping_next();
                let n = self.cur_val;
                let r = self.cond_scan_relation(this_if);
                self.scan_int();
                b = cond_compare(n, r, self.cur_val);
            }
            IFX_CODE => {
                // §533 Test if two tokens match
                let save_scanner_status = self.scanner_status;
                self.scanner_status = NORMAL;
                self.get_next();
                let n = self.cur_cs;
                let p = self.cur_cmd;
                let q = self.cur_chr;
                self.get_next();
                if self.cur_cmd != p {
                    b = false;
                } else if self.cur_cmd < CALL {
                    b = self.cur_chr == q;
                } else {
                    // §534 Test if two macro texts match; the reference counts are not compared
                    let (mut p, mut q) = (self.cur_chr + 1, self.equiv(n) + 1);
                    if p == q {
                        b = true;
                    } else {
                        while self.toks[p] != 0 && self.toks[q] != 0 && self.toks[p] == self.toks[q] {
                            p += 1;
                            q += 1;
                        }
                        b = self.toks[p] == 0 && self.toks[q] == 0;
                    }
                }
                self.scanner_status = save_scanner_status;
            }
            IF_TRUE_CODE => b = true,
            IF_FALSE_CODE => b = false,
            IF_CASE_CODE => {
                self.cond_case(save_cond_ptr);
                return;
            }
            _ => b = self.cond_other(this_if),
        }
        if is_unless {
            b = !b;
        }
        if self.tracing_commands() > 1 {
            // §528 Display the value of b
            self.begin_diagnostic();
            if b {
                self.print_s(b"{true}");
            } else {
                self.print_s(b"{false}");
            }
            self.end_diagnostic(false);
        }
        if b {
            self.change_if_limit(ELSE_CODE, save_cond_ptr);
            return; // wait for \else or \fi
        }
        // §526 Skip to \else or \fi, then goto common_ending
        loop {
            self.pass_text();
            if self.cond_ptr == save_cond_ptr {
                if self.cur_chr != OR_CODE {
                    break;
                }
                self.cond_extra_or();
            } else if self.cur_chr == FI_CODE {
                self.cond_pop(); // §522
            }
        }
        self.cond_common_ending();
    }

    /// §526's complaint.
    #[cold]
    #[inline(never)]
    fn cond_extra_or(&mut self) {
        self.print_err_s(b"Extra ");
        self.print_esc_s(b"or");
        self.help(&[b"I'm ignoring this; it doesn't match any \\if."]);
        self.error();
    }

    /// §524 common_ending:
    #[inline(always)]
    fn cond_common_ending(&mut self) {
        if self.cur_chr == FI_CODE {
            self.cond_pop(); // §522
        } else {
            self.if_limit = FI_CODE; // wait for \fi
        }
    }

    /// §535 Select the appropriate case and return or goto common_ending
    #[inline(never)]
    fn cond_case(&mut self, save_cond_ptr: i32) {
        self.scan_int();
        let mut n = self.cur_val; // n is the number of cases to pass
        if self.tracing_commands() > 1 {
            self.begin_diagnostic();
            self.print_s(b"{case ");
            self.print_int(n);
            self.print_char(b'}' as i32);
            self.end_diagnostic(false);
        }
        while n != 0 {
            self.pass_text();
            if self.cond_ptr == save_cond_ptr {
                if self.cur_chr == OR_CODE {
                    n -= 1;
                } else {
                    self.cond_common_ending();
                    return;
                }
            } else if self.cur_chr == FI_CODE {
                self.cond_pop(); // §522
            }
        }
        self.change_if_limit(OR_CODE, save_cond_ptr); // wait for \or, \else, or \fi
    }

    /// §527's other cases, and b.
    #[inline(never)]
    fn cond_other(&mut self, this_if: i32) -> bool {
        let mut b = false;
        match this_if {
            IF_DIM_CODE => {
                // §529 Test relation between integers or dimensions
                self.scan_normal_dimen();
                let n = self.cur_val;
                let r = self.cond_scan_relation(this_if);
                self.scan_normal_dimen();
                b = cond_compare(n, r, self.cur_val);
            }
            IF_ODD_CODE => {
                // §530 Test if an integer is odd
                self.scan_int();
                b = odd(self.cur_val);
            }
            IF_VMODE_CODE => b = abs(self.mode()) == VMODE,
            IF_HMODE_CODE => b = abs(self.mode()) == HMODE,
            IF_MMODE_CODE => b = abs(self.mode()) == MMODE,
            IF_INNER_CODE => b = self.mode() < 0,
            IF_VOID_CODE | IF_HBOX_CODE | IF_VBOX_CODE => {
                // §531 Test box register status
                self.scan_register_num();
                let p = self.fetch_box();
                if this_if == IF_VOID_CODE {
                    b = p == NULL;
                } else if p == NULL {
                    b = false;
                } else if this_if == IF_HBOX_CODE {
                    b = self.ty(p) == HLIST_NODE;
                } else {
                    b = self.ty(p) == VLIST_NODE;
                }
            }
            IF_EOF_CODE => {
                self.scan_four_bit_int_or_18();
                if self.cur_val == 18 {
                    b = self.shellenabledp == 0;
                } else {
                    b = self.read_open[self.cur_val] == CLOSED;
                }
            }
            // §1766
            IF_DEF_CODE => {
                let save_scanner_status = self.scanner_status;
                self.scanner_status = NORMAL;
                self.get_next();
                b = self.cur_cmd != UNDEFINED_CS;
                self.scanner_status = save_scanner_status;
            }
            // §1767.  The characters go on the tx stack: see Tex::tx_mark.
            IF_CS_CODE => {
                let n = self.tx_mark();
                let e = self.is_in_csname;
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
                    // §399 Complain about missing \endcsname
                    self.print_err_s(b"Missing ");
                    self.print_esc_s(b"endcsname");
                    self.print_s(b" inserted");
                    self.help(&[
                        b"The control sequence marked <to be read again> should",
                        b"not appear between \\csname and \\endcsname.",
                    ]);
                    self.back_error();
                }
                // §1768 Look up the characters of list n in the hash table, and set cur_cs
                let m = self.expand_tx_to_buffer(n);
                if m > self.first + 1 {
                    self.cur_cs = self.id_lookup(self.first, m - self.first); // no_new_control_sequence is true
                } else if m == self.first {
                    self.cur_cs = NULL_CS; // the list is empty
                } else {
                    self.cur_cs = SINGLE_BASE + self.buffer.at(self.first); // the list has length one
                }
                self.tx_discard(n);
                b = self.eq_type(self.cur_cs) != UNDEFINED_CS;
                self.is_in_csname = e;
            }
            // §1769
            IF_IN_CSNAME_CODE => b = self.is_in_csname,
            IF_PDFABS_DIM_CODE | IF_PDFABS_NUM_CODE => {
                if this_if == IF_PDFABS_NUM_CODE {
                    self.scan_int();
                } else {
                    self.scan_normal_dimen();
                }
                let mut n = self.cur_val;
                if n < 0 {
                    n = -n;
                }
                let r = self.cond_scan_relation(this_if);
                if this_if == IF_PDFABS_NUM_CODE {
                    self.scan_int();
                } else {
                    self.scan_normal_dimen();
                }
                if self.cur_val < 0 {
                    self.cur_val = -self.cur_val;
                }
                b = cond_compare(n, r, self.cur_val);
            }
            IF_FONT_CHAR_CODE => {
                self.scan_font_ident();
                let n = self.cur_val;
                self.scan_char_num();
                if self.font_bc[n] <= self.cur_val && self.font_ec[n] >= self.cur_val {
                    b = self.char_exists(self.char_info(n, self.cur_val));
                } else {
                    b = false;
                }
            }
            IF_PDFPRIMITIVE_CODE => {
                let save_scanner_status = self.scanner_status;
                self.scanner_status = NORMAL;
                self.get_next();
                self.scanner_status = save_scanner_status;
                let m = if self.cur_cs < HASH_BASE {
                    self.prim_lookup(self.cur_cs - SINGLE_BASE)
                } else {
                    self.prim_lookup(self.text(self.cur_cs))
                };
                b = self.cur_cmd != UNDEFINED_CS
                    && m != UNDEFINED_PRIMITIVE
                    && self.cur_cmd == self.prim_eq_type(m)
                    && self.cur_chr == self.prim_equiv(m);
            }
            _ => {} // there are no other cases
        }
        b
    }
}

/// §529's end: `r` is `<`, `=` or `>`.
#[inline(always)]
fn cond_compare(n: i32, r: i32, v: i32) -> bool {
    match r {
        LESS => n < v,
        EQUALS => n == v,
        GREATER => n > v,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use crate::scan::testkit::*;

    /// The body of an `\edef`: conditionals, and the convert commands, with the real expand and get_next.  tests/oracle/gen_scan_tables.py has the oracle's answers.
    #[test]
    fn conditionals_in_an_edef_against_the_oracle() {
        assert!(check_table(&["edef"]) > 300);
    }
}
