//! The capture loops of DESIGN.md section 8: copy and skip loops that read the token heap directly when the input is a token list.
//!
//! §379 does nothing to a token that these loops have to know about, except for what they do here: `loc` moves, and a brace moves
//! `align_state`.  So they run where the literal loops of §418, §425, §503, §520 and §1683 call `get_next` or `get_token`, and every
//! token that needs more of `get_next`, or more of the caller, is left for the literal loop to read:
//!
//! - the end of a list, since §346 does what depends on the level, and the token 1, §15's `\BAD.`;
//! - `out_param`, unless §381's level can be left out, below;
//! - a character token of command 13 to 15.  None can be read, but for `cs_token_flag`, 4095, whose `cur_tok` comes from `eqtb[0]`;
//! - `\endtemplate`, `\endv`, `\relax` as frozen, `\endwrite` and the `\noexpand` mark, in one range of control sequences (`is_frozen`);
//! - a control sequence with `eq_type` from `outer_call` up, when the sticky `any_user_outer` says one may exist: without it none is read;
//! - while `align_state` could be 0, §364: a loop runs only where that cannot happen before it ends.  So `&`, `\cr` and `\span` need no test;
//! - `\par` when the macro is not `\long`, and whatever else the caller says.
//!
//! When a loop stops, the tokens it passed are on the builder, or have been skipped, with `loc` and `align_state` as `get_next` would have
//! left them just before that token.  `cur_cmd`, `cur_chr`, `cur_cs` and `cur_tok` are not, as the literal loop reads the next token at once.  A
//! caller that goes on without reading another, in §425 and §420 here, ends with `capture_leave`.
//!
//! **§381 without its level.**  A loop reading a macro's body comes to `#1`.  If the push of `begin_token_list` would not raise `max_in_stack`,
//! which is printed, and so cannot overflow, the argument can be read where it is: when it ends, §346 would only have popped the level.  If a loop stops
//! inside it, the level is made then, with `loc` where it stopped, and nothing shows that it came late.
//!
//! The feature `slow_capture` compiles the calls out, for differential testing: the literal code runs alone.
#![forbid(unsafe_code)]
#![deny(unreachable_patterns, unused_variables)]
#![allow(non_snake_case, unused_mut, unused_imports, dead_code)]

use crate::access::*;
use crate::consts::*;
use crate::host::*;
use crate::raw::*;
use crate::state::*;

/// Commands of a character token that a loop leaves to `get_next`, as bits: a token below `cs_token_flag+1` shifted down by 8 is one of 0 to 15.
const CHAR_BAIL: u32 = 1 << OUT_PARAM | 1 << 13 | 1 << 14 | 1 << 15;

/// What `capture_undelimited` and `capture_delimited` give: the argument is complete and in `pstack[n]`, with `cur_tok` its last token.
/// Otherwise it is `unbalance`: 0 at the top level of the argument, and more inside a group whose left brace is on the builder.
pub const CAPTURE_DONE: i32 = -1;

/// `\endtemplate`, `\endv`, the frozen `\relax`, `\endwrite` and `\noexpand`'s mark.
#[inline(always)]
const fn is_frozen(t: i32) -> bool {
    (t.wrapping_sub(CS_TOKEN_FLAG + FROZEN_END_TEMPLATE) as u32) <= (FROZEN_DONT_EXPAND - FROZEN_END_TEMPLATE) as u32
}

#[inline(always)]
const fn is_left_brace(t: i32) -> bool {
    t >= LEFT_BRACE_TOKEN && t < LEFT_BRACE_LIMIT
}

#[inline(always)]
const fn is_right_brace(t: i32) -> bool {
    t >= RIGHT_BRACE_TOKEN && t < RIGHT_BRACE_LIMIT
}

impl Tex {
    /// Whether this control sequence token can be passed, in a loop that does not otherwise look at its meaning.
    #[inline(always)]
    fn cs_passes(&self, t: i32, par: i32, outer: bool) -> bool {
        !(t == par || is_frozen(t) || (outer && self.eq_type(t - CS_TOKEN_FLAG) >= OUTER_CALL))
    }

    /// The second clause of §422's test.  A token that is never read stands for `\par` when it may pass.
    #[inline(always)]
    fn capture_par(&self) -> i32 {
        if self.long_state == LONG_CALL { 0 } else { self.par_token }
    }

    /// The next argument of the current macro, if §381 can leave out its level.
    #[inline(always)]
    fn capture_param(&self, t: i32) -> Option<i32> {
        if (t >> 8) == OUT_PARAM && self.token_type() == MACRO && self.input_ptr <= self.max_in_stack {
            Some(self.param_stack[self.param_start() + (t & 255) - 1])
        } else {
            None
        }
    }

    /// §425's loop.  Reads from `i` at `unbalance` `u`, which stays above 0, up to the first token that it may not pass or that is the
    /// group's right brace.  Gives the position of that token, and `unbalance`.
    #[inline(always)]
    fn scan_group(&self, mut i: i32, mut u: i32, par: i32, outer: bool) -> (i32, i32) {
        loop {
            let t = self.toks[i];
            if t < RIGHT_BRACE_LIMIT {
                if t >= LEFT_BRACE_LIMIT {
                    if u == 1 {
                        break;
                    }
                    u -= 1;
                } else if t >= LEFT_BRACE_TOKEN {
                    u += 1;
                } else {
                    break;
                }
            } else if t > CS_TOKEN_FLAG {
                if !self.cs_passes(t, par, outer) {
                    break;
                }
            } else if (CHAR_BAIL >> (t >> 8)) & 1 != 0 {
                break;
            }
            i += 1;
        }
        (i, u)
    }

    /// §503's loop, in `\def` or with expansion, which has to look at each control sequence: at `mac_param` of §505 if `macro_def`, and
    /// at anything that expands if `XPAND`.  As `scan_group` otherwise; the frozen control sequences that matter have `eq_type` above those.
    #[inline(always)]
    fn scan_body<const XPAND: bool>(&self, mut i: i32, mut u: i32, macro_def: bool) -> (i32, i32) {
        let bail = if macro_def { CHAR_BAIL | 1 << MAC_PARAM } else { CHAR_BAIL };
        loop {
            let t = self.toks[i];
            if t < RIGHT_BRACE_LIMIT {
                if t >= LEFT_BRACE_LIMIT {
                    if u == 1 {
                        break;
                    }
                    u -= 1;
                } else if t >= LEFT_BRACE_TOKEN {
                    u += 1;
                } else {
                    break;
                }
            } else if t > CS_TOKEN_FLAG {
                let c = self.eq_type(t - CS_TOKEN_FLAG);
                if c >= (if XPAND { MAX_COMMAND + 1 } else { OUTER_CALL }) || (macro_def && c == MAC_PARAM) {
                    break;
                }
            } else if (bail >> (t >> 8)) & 1 != 0 {
                break;
            }
            i += 1;
        }
        (i, u)
    }

    /// What is common to the loops inside a group that end on the builder: `scan` as `scan_group` does, and what happens around it.
    /// `align_state` is `a+unbalance` all through, and `a` is not negative, so it is not 0 while `unbalance` is above 0.
    #[inline(always)]
    fn capture_in_group<F: Fn(&Tex, i32, i32) -> (i32, i32)>(&mut self, unbalance: &mut i32, scan: F) {
        if self.state() != TOKEN_LIST {
            return;
        }
        let mut u = *unbalance;
        let mut a = self.align_state - u;
        if a < 0 {
            return;
        }
        let mut i = self.loc();
        loop {
            let (j, v) = scan(self, i, u);
            self.tb_push_range(i, j);
            u = v;
            i = j;
            let t = self.toks[i];
            if t == 0 {
                // §379's end of the list.  A u_template's end changes align_state.
                self.set_loc(i);
                self.align_state = a + u;
                self.end_token_list();
                *unbalance = u;
                a = self.align_state - u;
                if self.state() != TOKEN_LIST || a < 0 {
                    return;
                }
                i = self.loc();
                continue;
            }
            let Some(p) = self.capture_param(t) else { break };
            let (j, v) = scan(self, p + 1, u);
            self.tb_push_range(p + 1, j);
            u = v;
            i += 1;
            if self.toks[j] != 0 {
                self.set_loc(i);
                self.begin_token_list(p, PARAMETER);
                i = j;
                break;
            }
        }
        self.set_loc(i);
        self.align_state = a + u;
        *unbalance = u;
    }

    /// §425's loop inside `macro_call`, and §1683's and the part of §503 where nothing depends on a control sequence's meaning, with `par` 0.
    #[inline]
    pub fn capture_group(&mut self, unbalance: &mut i32, par: i32) {
        let outer = self.any_user_outer();
        self.capture_in_group(unbalance, |t, i, u| t.scan_group(i, u, par, outer));
    }

    /// §503's loop, before `get_token` or §504.
    #[inline]
    pub fn capture_body(&mut self, unbalance: &mut i32, macro_def: bool, xpand: bool) {
        if xpand {
            self.capture_in_group(unbalance, |t, i, u| t.scan_body::<true>(i, u, macro_def));
        } else if macro_def {
            self.capture_in_group(unbalance, |t, i, u| t.scan_body::<false>(i, u, true));
        } else {
            self.capture_group(unbalance, 0);
        }
    }

    /// If the next token of a list is a right brace, reads it as §379 does, and gives it: what ends §425's loop when `unbalance` is 1.
    #[inline(always)]
    pub fn capture_right_brace(&mut self) -> Option<i32> {
        if self.state() == TOKEN_LIST {
            let l = self.loc();
            let t = self.toks[l];
            if is_right_brace(t) {
                self.set_loc(l + 1);
                self.align_state -= 1;
                self.cur_tok = t;
                self.capture_leave();
                return Some(t);
            }
        }
        None
    }

    /// The ends of lists at the start of a parameter, as §379 meets them.
    #[inline(always)]
    pub fn capture_end_lists(&mut self) {
        while self.at_list_end() {
            self.end_token_list();
        }
    }

    /// §419 and §425 for a parameter without a delimiter.  The builder is empty.  `self.state()` is `token_list`.
    #[inline]
    pub fn capture_undelimited(&mut self, n: i32) -> i32 {
        let a = self.align_state;
        let mut i = self.loc();
        let mut t = self.toks[i];
        while t == SPACE_TOKEN {
            i += 1;
            t = self.toks[i];
        }
        let par = self.capture_par();
        let outer = self.any_user_outer();
        if t >= RIGHT_BRACE_LIMIT {
            let passes = if t > CS_TOKEN_FLAG { self.cs_passes(t, par, outer) } else { (CHAR_BAIL >> (t >> 8)) & 1 == 0 };
            if passes && a != 0 {
                self.pstack[n] = self.tok_one(t);
                self.set_loc(i + 1);
                self.cur_tok = t;
                return CAPTURE_DONE;
            }
        } else if is_left_brace(t) && a >= 0 {
            let (j, u) = self.scan_group(i + 1, 1, par, outer);
            let e = self.toks[j];
            if is_right_brace(e) {
                self.pstack[n] = self.tok_from_range(i + 1, j);
                self.set_loc(j + 1);
                self.cur_tok = e;
                return CAPTURE_DONE;
            }
            self.tb_push_range(i, j);
            self.set_loc(j);
            self.align_state = a + u;
            return u;
        }
        self.set_loc(i);
        0
    }

    /// §418 and §425 for a parameter delimited by `toks[r]` and possibly more: up to that token, outside any group.
    #[inline(always)]
    fn scan_top(&self, mut i: i32, d: i32, par: i32, outer: bool, m: &mut i32) -> (i32, i32) {
        let mut k = *m;
        let u = loop {
            let t = self.toks[i];
            if t == d {
                break 0;
            }
            if t < RIGHT_BRACE_LIMIT {
                if !is_left_brace(t) {
                    break 0;
                }
                let (j, u) = self.scan_group(i + 1, 1, par, outer);
                if !is_right_brace(self.toks[j]) {
                    i = j;
                    break u;
                }
                k += 1;
                i = j + 1;
                continue;
            }
            if t > CS_TOKEN_FLAG {
                if !self.cs_passes(t, par, outer) {
                    break 0;
                }
            } else if (CHAR_BAIL >> (t >> 8)) & 1 != 0 {
                break 0;
            }
            k += 1;
            i += 1;
        };
        *m = k;
        (i, u)
    }

    /// §418 at the start of a delimited parameter's match, `r=s`.  `self.state()` is `token_list`.  Done means that the delimiter, which is one token, has been read
    /// too.  `align_state` is above 0, so that a token at the top level of the argument is never under §364 and a group's braces cannot make it 0.
    #[inline]
    pub fn capture_delimited(&mut self, r: i32, n: i32, mark: i32, m: &mut i32) -> i32 {
        let a = self.align_state;
        if a <= 0 {
            return 0;
        }
        let d = self.toks[r];
        let par = self.capture_par();
        let outer = self.any_user_outer();
        let mut i = self.loc();
        let mut u;
        loop {
            let (j, v) = self.scan_top(i, d, par, outer, m);
            u = v;
            if u == 0 && self.toks[j] == d && self.tb_ptr == mark {
                let e = self.toks[r + 1];
                if e >= MATCH_TOKEN && e <= END_MATCH_TOKEN && (d <= CS_TOKEN_FLAG || self.cs_passes(d, 0, outer)) {
                    // §426, and §420 with its correction of `align_state` for `#{`
                    self.pstack[n] = if *m == 1 && self.toks[j - 1] < RIGHT_BRACE_LIMIT {
                        self.tok_from_range(i + 1, j - 1)
                    } else {
                        self.tok_from_range(i, j)
                    };
                    self.set_loc(j + 1);
                    self.cur_tok = d;
                    return CAPTURE_DONE;
                }
            }
            self.tb_push_range(i, j);
            i = j;
            if u != 0 {
                break;
            }
            let Some(p) = self.capture_param(self.toks[i]) else { break };
            let (j, v) = self.scan_top(p + 1, d, par, outer, m);
            self.tb_push_range(p + 1, j);
            u = v;
            i += 1;
            if self.toks[j] != 0 {
                self.set_loc(i);
                self.begin_token_list(p, PARAMETER);
                i = j;
                break;
            }
            if u != 0 {
                break;
            }
        }
        self.set_loc(i);
        self.align_state = a + u;
        u
    }

    /// Where a loop has read the last token of a macro's parameters, as `get_token` would have left things.
    #[inline(always)]
    pub fn capture_leave(&mut self) {
        let t = self.cur_tok;
        if t > CS_TOKEN_FLAG {
            let cs = t - CS_TOKEN_FLAG;
            self.cur_cs = cs;
            self.cur_cmd = self.eq_type(cs);
            self.cur_chr = self.equiv(cs);
        } else {
            self.cur_cs = 0;
            self.cur_cmd = t >> 8;
            self.cur_chr = t & 255;
        }
        self.no_new_control_sequence = true;
    }

    /// §520's loop.  A character token needs no table, and a brace only moves `align_state`; this stops short of the `\fi` or `\else` that
    /// ends the text, so that the literal loop reads it.
    pub fn capture_skip(&mut self, l: &mut i32) {
        if self.state() != TOKEN_LIST {
            return;
        }
        let mut a = self.align_state;
        if a == 0 {
            return;
        }
        let mut n = *l;
        let mut i = self.loc();
        // The argument being read without its level, and where to go on in the macro's body after it, or 0.
        let (mut p, mut back) = (0, 0);
        loop {
            let t = self.toks[i];
            if t < RIGHT_BRACE_LIMIT {
                if t >= LEFT_BRACE_TOKEN {
                    a += if t < LEFT_BRACE_LIMIT { 1 } else { -1 };
                    if a == 0 {
                        i += 1;
                        break;
                    }
                } else if t != 0 {
                    break;
                } else if back != 0 {
                    i = back;
                    back = 0;
                    continue;
                } else {
                    // §379's end of the list
                    self.set_loc(i);
                    self.align_state = a;
                    self.end_token_list();
                    *l = n;
                    a = self.align_state;
                    if self.state() != TOKEN_LIST || a == 0 {
                        return;
                    }
                    i = self.loc();
                    continue;
                }
            } else if t > CS_TOKEN_FLAG {
                let cs = t - CS_TOKEN_FLAG;
                let c = self.eq_type(cs);
                if c >= IF_TEST {
                    if c == IF_TEST {
                        n += 1;
                    } else if c == FI_OR_ELSE {
                        if n == 0 {
                            break;
                        }
                        if self.equiv(cs) == FI_CODE {
                            n -= 1;
                        }
                    } else if c >= OUTER_CALL {
                        break;
                    }
                }
            } else if (CHAR_BAIL >> (t >> 8)) & 1 != 0 {
                if back != 0 {
                    break;
                }
                let Some(q) = self.capture_param(t) else { break };
                p = q;
                back = i + 1;
                i = p + 1;
                continue;
            }
            i += 1;
        }
        if back != 0 {
            self.set_loc(back);
            self.begin_token_list(p, PARAMETER);
        }
        self.set_loc(i);
        self.align_state = a;
        *l = n;
    }
}
