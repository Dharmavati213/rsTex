//! The capture loops of DESIGN.md section 8: copy and skip loops that read the token heap directly when the input is a token list.
//!
//! §379 does nothing to a token that these loops have to know about, except for what they do here: `loc` moves, and a brace moves
//! `align_state`.  So they run where the literal loops of §418, §425, §500, §503, §520 and §1683 call `get_next` or `get_token`, and every
//! token that needs more of `get_next`, or more of the caller, is left for the literal loop to read:
//!
//! - the end of a list reached with nothing else to do: §346 does what depends on the level, so they call `end_token_list` as §379 does;
//! - the token 1, section 15's `\BAD.`, and `cs_token_flag` alone, which §405 can back up: `cur_tok` is 1 when it is read;
//! - `out_param`, unless §381's level can be left out, below;
//! - `\endtemplate`, `\endv`, the frozen `\relax`, `\endwrite` and the `\noexpand` mark, in one range of control sequences (`is_frozen`);
//! - a control sequence with `eq_type` from `outer_call` up, when the sticky `any_user_outer` says one may exist: without it none is read;
//! - while `align_state` could be 0, §364: a loop runs only where that cannot happen before it ends.  So `&`, `\cr` and `\span` need no test;
//! - `\par` when the macro is not `\long`, and whatever else the caller says.
//!
//! A character token of command `match` or `end_match` is not looked for, as no list that gets read holds one: they are all in a macro's
//! parameter text, and §416 starts after that.
//!
//! When a loop stops, the tokens it passed are on the builder, or have been skipped, with `loc` and `align_state` as `get_next` would have
//! left them just before that token.  `cur_cmd`, `cur_chr`, `cur_cs` and `cur_tok` are not, as the literal loop reads the next token at once.  A
//! caller that goes on without reading another, in §420 and §425 here, ends with `capture_leave`.
//!
//! **§381 without its level.**  A loop reading a macro's body comes to `#1`.  If the push of `begin_token_list` would not raise `max_in_stack`,
//! which is printed, and so cannot overflow, the argument can be read where it is: when it ends, §346 would only have popped the level.  If a loop stops
//! inside it, the level is made then, with `loc` where it stopped, and nothing shows that it came late.
//!
//! **An argument handed on whole.**  In `\\def\\a#1{\\b{#1}}` a loop would read the argument of `\\a` all through to copy it for `\\b`, though a loop has read it once already, and though nothing can write to a
//! block once it is made.  So a block that a loop made here says on its header what that loop saw, `ARG_CLEAN` and `ARG_NOPAR`, and `{#n}` takes the same block with one more reference if that means the new loop would pass it all: no user `\\outer` then or now, as `any_user_outer`
//! never goes back; no token that stops a loop, nor `\\par` for a macro that is not `\\long`.  What the read would have done is the end of §381's level, which is as above, and the two braces, which come to nothing.  The reference is the one thing that can tell: the end of a macro's
//! level gives back with `arg_release`.  The token heap comes out with other addresses in it, which only a stale word in `mem` could show: docs/PORTING.md section 15.
//!
//! The feature `slow_capture` compiles the calls out, for differential testing: the literal code runs alone.  `RSTEX_NO_SHARE` leaves out only the handing on, by setting `any_user_outer`, with no test on the way here.
#![forbid(unsafe_code)]
#![deny(unreachable_patterns, unused_variables)]
#![allow(non_snake_case, unused_mut, unused_imports, dead_code)]

use crate::access::*;
use crate::consts::*;
use crate::host::*;
use crate::raw::*;
use crate::state::*;
use crate::toks::EXACT_CLASSES;

/// What `capture_undelimited` and `capture_delimited` give: the argument is complete and in `pstack[n]`, with `cur_tok` its last token.
/// Otherwise it is `unbalance`: 0 at the top level of the argument, and more inside a group whose left brace is on the builder.
pub const CAPTURE_DONE: i32 = -1;

/// How many pieces of list a group read with its parameters in place can be made of, before it goes on the builder.
const GROUP_PIECES: usize = 8;

/// Character tokens from here up to `cs_token_flag` are none of the commands a loop looks at: most tokens are in this range.
const PLAIN_CHARS: i32 = SUP_MARK * 256;

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

#[inline(always)]
const fn is_out_param(t: i32) -> bool {
    (t.wrapping_sub(OUT_PARAM_TOKEN) as u32) < 256
}

#[inline(always)]
const fn is_match(t: i32) -> bool {
    t >= MATCH_TOKEN && t <= END_MATCH_TOKEN
}

/// On an argument's block, above any count of references it can have.  `ARG_CLEAN`: made by a loop that passed all its tokens, with no user `\\outer` about, so balanced and with none of `cs_stops`'s
/// tokens but perhaps `\\par`.  `ARG_NOPAR`: not that either.
const ARG_CLEAN: i32 = 1 << 28;
const ARG_NOPAR: i32 = 1 << 29;

impl Tex {
    /// What a loop knows of a block it has just made for an argument, `par` being the token it stopped for.
    #[inline(always)]
    fn arg_mark_clean(&mut self, p: i32, par: i32) {
        if p != NULL {
            self.toks[p] |= if par != 0 { ARG_CLEAN | ARG_NOPAR } else { ARG_CLEAN };
        }
    }

    /// Whether a macro whose loop stops at `par` would pass the whole of this argument, and it would get there: the literal loop has the argument on the builder as it goes, and stops at the token that does not
    /// fit, so one too long for an exact size class asks for room, as `capture_undelimited` does for one it has read.  Class 64 or more has `0xC0`: that test comes with the others, and the room is by the class's capacity,
    /// which can only be more than the argument, and sends a job at the very end of the builder to the loop, which knows the length.
    #[inline(always)]
    fn arg_shareable(&self, p: i32, par: i32) -> bool {
        p == NULL || {
            let h = self.toks[p];
            let need = if par != 0 { ARG_CLEAN | ARG_NOPAR } else { ARG_CLEAN };
            h & (need | 0xC0) == need || (h & need == need && self.capture_room(crate::toks::CLASS_CAP[(h & 0xFF) as usize] + 2))
        }
    }

    /// Whether a loop that does not otherwise look at meanings stops at this token, which is `cs_token_flag` or more.  The names that
    /// come after the frozen ones, which is all with more than one letter, have one test less.  `OUTER` is `any_user_outer`.
    #[inline(always)]
    fn cs_stops<const OUTER: bool>(&self, t: i32, par: i32) -> bool {
        t == par
            || (t <= CS_TOKEN_FLAG + FROZEN_DONT_EXPAND && (t == CS_TOKEN_FLAG || is_frozen(t)))
            || (OUTER && self.eq_type(t - CS_TOKEN_FLAG) >= OUTER_CALL)
    }

    /// The second clause of §422's test.  A token that is never read stands for `\par` when it may pass.
    #[inline(always)]
    fn capture_par(&self) -> i32 {
        if self.long_state == LONG_CALL { 0 } else { self.par_token }
    }

    /// The argument that §381 would start for the token `t`, if the level can be left out.
    #[inline(always)]
    fn capture_param(&self, t: i32) -> Option<i32> {
        if is_out_param(t) && self.token_type() == MACRO && self.input_ptr <= self.max_in_stack {
            Some(self.param_stack[self.param_start() + (t & 255) - 1])
        } else {
            None
        }
    }

    /// §425's loop.  Reads from `i` at `unbalance` `u`, which stays above 0, up to the first token that it may not pass or that is the
    /// group's right brace.  Gives the position of that token, and `unbalance`.
    #[inline(always)]
    fn scan_group<const OUTER: bool>(&self, mut i: i32, mut u: i32, par: i32) -> (i32, i32) {
        loop {
            let t = self.toks[i];
            if t >= CS_TOKEN_FLAG {
                if self.cs_stops::<OUTER>(t, par) {
                    break;
                }
            } else if t >= PLAIN_CHARS {
            } else if t < RIGHT_BRACE_LIMIT {
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
            } else if is_out_param(t) {
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
        let bail = if macro_def { 1 << OUT_PARAM | 1 << MAC_PARAM | 1 << 15 } else { 1u32 << OUT_PARAM | 1 << 15 };
        loop {
            let t = self.toks[i];
            if t > CS_TOKEN_FLAG {
                let c = self.eq_type(t - CS_TOKEN_FLAG);
                if c >= (if XPAND { MAX_COMMAND + 1 } else { OUTER_CALL }) || (macro_def && c == MAC_PARAM) {
                    break;
                }
            } else if t >= PLAIN_CHARS && t < CS_TOKEN_FLAG {
            } else if t < RIGHT_BRACE_LIMIT {
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
            } else if (bail >> (t >> 8)) & 1 != 0 {
                break;
            }
            i += 1;
        }
        (i, u)
    }

    /// Whether the builder has room for `n` more tokens.  The literal loops have a list on the builder, one token at a time, and they stop at the
    /// token that does not fit, with `loc` just past it and the argument it is in on the input stack, which `runaway` and `show_context` then show.  So what
    /// these loops read counts as on the builder, even what goes straight to the heap, and they stop in the same place.  Only an argument too long for
    /// an exact size class asks this, as a test on every argument costs what a macro call can least afford: a shorter one that goes straight to the
    /// heap needs the builder for nothing, and the stop comes at the next token that does go there.  docs/PORTING.md section 15, "Where main memory fills".
    #[inline(always)]
    fn capture_room(&self, n: i32) -> bool {
        self.tb_ptr as usize + n as usize <= TB_CAP
    }

    /// `tb_push_range(i, j)`, of what a loop has read in the list it is in.
    #[inline(always)]
    fn capture_push(&mut self, i: i32, j: i32) {
        if j > i {
            if !self.capture_room(j - i) {
                self.capture_overflow(i);
            }
            self.tb_put_range(i, j);
        }
    }

    /// The same of `toks[p+1..j]`, what a loop has read of the argument `p` of the `out_param` at `i`, without its level: §381 makes it now.
    #[inline(always)]
    fn capture_push_param(&mut self, i: i32, p: i32, j: i32) {
        if j > p + 1 {
            if !self.capture_room(j - (p + 1)) {
                self.set_loc(i + 1);
                self.begin_token_list(p, PARAMETER);
                self.capture_overflow(p + 1);
            }
            self.tb_put_range(p + 1, j);
        }
    }

    /// `tb_push(t)`, where the literal loop would have `loc` at `l` by then.
    #[inline(always)]
    fn capture_push_one(&mut self, t: i32, l: i32) {
        if self.tb_ptr as usize >= TB_CAP {
            self.set_loc(l);
        }
        self.tb_push(t);
    }

    /// There is no room on the builder for the tokens from `i` in the current list: as many as there is room for go on, and the next is the one that stops the literal loop.
    #[cold]
    #[inline(never)]
    fn capture_overflow(&mut self, i: i32) -> ! {
        let room = TB_CAP as i32 - self.tb_ptr;
        let x = i + room;
        // §423 puts a false start on the builder when it has read the token that ended it, so a false start's token that does not fit has `loc` past that one.  They are at the top level of an
        // argument, where the left braces on the builder have all been closed.
        let at_top = self.scanner_status == MATCHING && {
            let (m, mut u) = (self.tb_match_mark, 0);
            for k in m..self.tb_ptr {
                let t = self.tb[k];
                u += is_left_brace(t) as i32 - is_right_brace(t) as i32;
            }
            u == 0
        };
        let l = if !at_top {
            x
        } else if self.any_user_outer() {
            self.capture_read_up_to::<true>(i, x, self.v_macrocall.delimiter)
        } else {
            self.capture_read_up_to::<false>(i, x, self.v_macrocall.delimiter)
        };
        self.tb_put_range(i, i + room);
        self.set_loc(l + 1);
        self.tb_overflow()
    }

    /// `scan_top`'s way from `i` to the token at `x`, which it has passed: how far the literal loop has read when that token goes on the builder.
    fn capture_read_up_to<const OUTER: bool>(&self, i: i32, x: i32, r: i32) -> i32 {
        let d = self.toks[r];
        let par = self.capture_par();
        let mut q = i;
        while q <= x {
            let t = self.toks[q];
            if t == d {
                let f = self.capture_false_start::<OUTER>(r, q);
                if q + f > x {
                    return q + f;
                }
                q += f;
            } else if is_left_brace(t) {
                let (e, _) = self.scan_group::<OUTER>(q + 1, 1, par);
                if !is_right_brace(self.toks[e]) {
                    break;
                }
                q = e + 1;
            } else {
                q += 1;
            }
        }
        x
    }

    /// What is common to the loops inside a group that end on the builder: `scan` as `scan_group` does, and what happens around it.
    /// `align_state` is `a+unbalance` all through, and `a` is not negative, so it is not 0 while `unbalance` is above 0.  `self.state()` is `token_list`, as
    /// the callers test before they call, since a list is what a call is worth making for.
    #[inline(always)]
    fn capture_in_group<F: Fn(&Tex, i32, i32) -> (i32, i32)>(&mut self, unbalance: &mut i32, scan: F) {
        let mut u = *unbalance;
        let mut a = self.align_state - u;
        if a < 0 {
            return;
        }
        let mut i = self.loc();
        loop {
            let (j, v) = scan(self, i, u);
            self.capture_push(i, j);
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
            self.capture_push_param(i, p, j);
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
        if self.any_user_outer() {
            self.capture_in_group(unbalance, |t, i, u| t.scan_group::<true>(i, u, par));
        } else {
            self.capture_in_group(unbalance, |t, i, u| t.scan_group::<false>(i, u, par));
        }
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

    /// §419 and §425 for a parameter without a delimiter.  The builder is empty.  `self.state()` is `token_list`.
    #[inline]
    pub fn capture_undelimited(&mut self, n: i32) -> i32 {
        if self.any_user_outer() { self.capture_undelimited_as::<true>(n) } else { self.capture_undelimited_as::<false>(n) }
    }

    #[inline(always)]
    fn capture_undelimited_as<const OUTER: bool>(&mut self, n: i32) -> i32 {
        let mut i = self.loc();
        let mut t = self.toks[i];
        loop {
            if t == SPACE_TOKEN {
                i += 1;
            } else if t == 0 {
                // §379's end of the list.  A u_template's end changes align_state.
                self.set_loc(i);
                self.end_token_list();
                if self.state() != TOKEN_LIST {
                    return 0;
                }
                i = self.loc();
            } else {
                break;
            }
            t = self.toks[i];
        }
        let a = self.align_state;
        let par = self.capture_par();
        if t >= RIGHT_BRACE_LIMIT {
            let passes = if t >= CS_TOKEN_FLAG { !self.cs_stops::<OUTER>(t, par) } else { !is_out_param(t) };
            if passes && a != 0 {
                let p = self.tok_try_alloc(1);
                if p != 0 {
                    self.toks[p + 1] = t;
                    self.toks[p + 2] = 0;
                    if !OUTER {
                        self.arg_mark_clean(p, par);
                    }
                    self.pstack[n] = p;
                    self.set_loc(i + 1);
                    self.cur_tok = t;
                    return CAPTURE_DONE;
                }
            }
        } else if is_left_brace(t) && a >= 0 {
            // `{#n}`: an argument handed on whole.  Its block is immutable and known to pass, so the new argument can be the same block with one more reference.
            let t1 = self.toks[i + 1];
            if !OUTER && is_out_param(t1) && is_right_brace(self.toks[i + 2]) {
                if let Some(p) = self.capture_param(t1) {
                    if self.arg_shareable(p, par) {
                        if p != NULL {
                            self.toks[p] += 256;
                        }
                        self.pstack[n] = p;
                        let e = self.toks[i + 2];
                        self.set_loc(i + 3);
                        self.cur_tok = e;
                        return CAPTURE_DONE;
                    }
                }
            }
            let (j, u) = self.scan_group::<OUTER>(i + 1, 1, par);
            let e = self.toks[j];
            if !is_right_brace(e) {
                return self.capture_group_in_place::<OUTER>(n, i, j, u, par, a);
            }
            // The test is class_for's, which tok_from_range makes on the same number.
            if (j - (i + 1)) as usize >= EXACT_CLASSES && !self.capture_room(j - i + 1) {
                self.set_loc(i);
                return 0;
            }
            let p = self.tok_from_range(i + 1, j);
            if p != NULL || j == i + 1 {
                if !OUTER {
                    self.arg_mark_clean(p, par);
                }
                self.pstack[n] = p;
                self.set_loc(j + 1);
                self.cur_tok = e;
                return CAPTURE_DONE;
            }
        }
        self.set_loc(i);
        0
    }

    /// §425, for a group with a left brace at `i` that a scan has read as far as `j`, at `unbalance` `u`.  Its pieces are all in lists that stay, as
    /// long as they are the macro's body and its arguments, so the block can be made from them without the builder.  If it turns out
    /// otherwise, they go on the builder and the caller goes on from `j`.
    #[inline(never)]
    fn capture_group_in_place<const OUTER: bool>(&mut self, n: i32, i: i32, j: i32, u: i32, par: i32, a: i32) -> i32 {
        let mut pieces = [(0, 0); GROUP_PIECES];
        let mut k = 0;
        let mut len = 0;
        if j > i + 1 {
            pieces[0] = (i + 1, j);
            len = j - (i + 1);
            k = 1;
        }
        let (mut j, mut u) = (j, u);
        loop {
            if is_right_brace(self.toks[j]) {
                let p = if len == 0 {
                    NULL_LIST
                } else if len as usize >= EXACT_CLASSES && !self.capture_room(len + 2) {
                    0
                } else {
                    self.tok_try_alloc(len)
                };
                if p == 0 {
                    self.set_loc(i);
                    return 0;
                }
                self.pstack[n] = if len == 0 {
                    NULL
                } else {
                    let mut o = p + 1;
                    for &(b, e) in &pieces[..k] {
                        self.toks.copy_cells_within(b, o, e - b);
                        o += e - b;
                    }
                    self.toks[o] = 0;
                    if !OUTER {
                        self.arg_mark_clean(p, par);
                    }
                    p
                };
                self.cur_tok = self.toks[j];
                self.set_loc(j + 1);
                return CAPTURE_DONE;
            }
            if k > GROUP_PIECES - 2 {
                break;
            }
            let Some(p) = self.capture_param(self.toks[j]) else { break };
            let (e, v) = self.scan_group::<OUTER>(p + 1, u, par);
            if self.toks[e] != 0 {
                break;
            }
            if e > p + 1 {
                pieces[k] = (p + 1, e);
                len += e - (p + 1);
                k += 1;
            }
            let (next, w) = self.scan_group::<OUTER>(j + 1, v, par);
            if next > j + 1 {
                pieces[k] = (j + 1, next);
                len += next - (j + 1);
                k += 1;
            }
            j = next;
            u = w;
        }
        if !self.capture_room(len + 1) {
            self.set_loc(i);
            return 0;
        }
        self.tb_put_range(i, i + 1);
        for &(b, e) in &pieces[..k] {
            self.tb_put_range(b, e);
        }
        self.set_loc(j);
        self.align_state = a + u;
        u
    }

    /// The tokens from `i` that are a false start of the delimiter at `r`, of which the first has matched, or 0 if they are not: what §423
    /// would end up putting in the argument one by one, with its `r:=s`.  It takes only a delimiter whose first token does not come
    /// again in the part that matched, for which §423 finds no shorter match.  A left brace can only be a delimiter's last token, so none is passed.
    #[inline]
    fn capture_false_start<const OUTER: bool>(&self, r: i32, i: i32) -> i32 {
        let d = self.toks[r];
        let mut k = 1;
        loop {
            let e = self.toks[r + k];
            if is_match(e) {
                return 0;
            }
            let t = self.toks[i + k];
            if t != e {
                // The match can go on in the next list or in an argument.  And §423 comes after the read of this token: if that read is §358's error, the runaway it shows has no false start yet.
                if t == 0 || is_out_param(t) || (t >= CS_TOKEN_FLAG && self.cs_stops::<OUTER>(t, 0)) {
                    return 0;
                }
                break;
            }
            if e == d {
                return 0;
            }
            k += 1;
        }
        for x in 0..k {
            let e = self.toks[r + x];
            if e >= CS_TOKEN_FLAG && self.cs_stops::<OUTER>(e, 0) {
                return 0;
            }
        }
        k
    }

    /// §418 and §425 for a parameter delimited by `toks[r]` and possibly more: up to that token, outside any group.  `m` as in §418.
    #[inline(always)]
    fn scan_top<const OUTER: bool>(&self, mut i: i32, r: i32, par: i32, m: &mut i32) -> (i32, i32) {
        let d = self.toks[r];
        let mut k = *m;
        let u = loop {
            let t = self.toks[i];
            if t == d {
                let f = self.capture_false_start::<OUTER>(r, i);
                if f == 0 {
                    break 0;
                }
                k += f;
                i += f;
                continue;
            }
            if t >= CS_TOKEN_FLAG {
                if self.cs_stops::<OUTER>(t, par) {
                    break 0;
                }
            } else if t >= PLAIN_CHARS {
            } else if t < RIGHT_BRACE_LIMIT {
                if !is_left_brace(t) {
                    break 0;
                }
                let (j, u) = self.scan_group::<OUTER>(i + 1, 1, par);
                if !is_right_brace(self.toks[j]) {
                    i = j;
                    break u;
                }
                k += 1;
                i = j + 1;
                continue;
            } else if is_out_param(t) {
                break 0;
            }
            k += 1;
            i += 1;
        };
        *m = k;
        (i, u)
    }

    /// The number of tokens of the delimiter at `r` that follow at `i`, if all of it does in this list and all of it can be read by a loop; or 0.
    #[inline]
    fn capture_whole_delimiter<const OUTER: bool>(&self, r: i32, i: i32) -> i32 {
        let mut k = 0;
        loop {
            let e = self.toks[r + k];
            if is_match(e) {
                return k;
            }
            if self.toks[i + k] != e || (e >= CS_TOKEN_FLAG && self.cs_stops::<OUTER>(e, 0)) {
                return 0;
            }
            k += 1;
        }
    }

    /// §418 at the start of a delimited parameter's match, `r=s`.  `self.state()` is `token_list`.  Done means that the delimiter has been read too, and `r` is
    /// past it.  `align_state` is above 0, so that a token at the top level of the argument is never under §364 and a group's braces cannot make it 0.
    #[inline]
    pub fn capture_delimited(&mut self, r: &mut i32, n: i32, mark: i32, m: &mut i32) -> i32 {
        if self.any_user_outer() {
            let (c, s, k) = self.capture_delimited_outer(*r, n, mark, *m);
            *r = s;
            *m = k;
            c
        } else {
            self.capture_delimited_as::<false>(r, n, mark, m)
        }
    }

    /// Out of line: inlined as well as the form with no `\outer` about, as in LaTeX, it took the registers of all of `macro_call`, in lines that have nothing to do with `\outer`.
    /// `r` and `m` go by value, so that they need not be in memory there either.
    #[inline(never)]
    fn capture_delimited_outer(&mut self, mut r: i32, n: i32, mark: i32, mut m: i32) -> (i32, i32, i32) {
        let c = self.capture_delimited_as::<true>(&mut r, n, mark, &mut m);
        (c, r, m)
    }

    #[inline(always)]
    fn capture_delimited_as<const OUTER: bool>(&mut self, r: &mut i32, n: i32, mark: i32, m: &mut i32) -> i32 {
        let mut a = self.align_state;
        if a <= 0 {
            return 0;
        }
        let par = self.capture_par();
        let mut i = self.loc();
        let mut u;
        loop {
            let (j, v) = self.scan_top::<OUTER>(i, *r, par, m);
            u = v;
            let t = self.toks[j];
            if u == 0 && t == self.toks[*r] {
                let k = self.capture_whole_delimiter::<OUTER>(*r, j);
                if k > 0 {
                    // §426, and §420 with its correction of `align_state` for `#{`
                    self.pstack[n] = if self.tb_ptr != mark {
                        if !self.capture_room(j - i) {
                            self.capture_overflow(i);
                        }
                        self.set_loc(j + k);
                        self.tb_put_range(i, j);
                        if *m == 1 && self.tb[self.tb_ptr - 1] < RIGHT_BRACE_LIMIT { self.tb_commit_strip(mark) } else { self.tb_commit(mark) }
                    } else {
                        let (a, b) = if *m == 1 && self.toks[j - 1] < RIGHT_BRACE_LIMIT { (i + 1, j - 1) } else { (i, j) };
                        // The test is class_for's, which tok_from_range makes on the same number.
                        if (b - a) as usize >= EXACT_CLASSES && !self.capture_room(j - i) {
                            self.capture_overflow(i);
                        }
                        self.set_loc(j + k);
                        let p = self.tok_from_range(a, b);
                        if p == NULL && b > a {
                            // No room in the heap.  The literal loop has them on the builder by now.
                            self.tb_put_range(i, j);
                            self.tb_commit(mark);
                        }
                        if !OUTER {
                            // A false start goes in the argument with no look at §422's test, so a `\\par` in the delimiter may be in it.
                            let holds_par = k > 1 && (0..k).any(|x| self.toks[*r + x] == par);
                            self.arg_mark_clean(p, if holds_par { 0 } else { par });
                        }
                        p
                    };
                    self.cur_tok = self.toks[j + k - 1];
                    *r += k;
                    return CAPTURE_DONE;
                }
            }
            self.capture_push(i, j);
            i = j;
            if u != 0 {
                break;
            }
            if t == 0 {
                // §379's end of the list.  A u_template's end changes align_state.
                self.set_loc(i);
                self.end_token_list();
                a = self.align_state;
                if self.state() != TOKEN_LIST || a <= 0 {
                    return 0;
                }
                i = self.loc();
                continue;
            }
            let Some(p) = self.capture_param(t) else { break };
            let (j, v) = self.scan_top::<OUTER>(p + 1, *r, par, m);
            self.capture_push_param(i, p, j);
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
            // A control sequence armed for a checkpoint reads as what it is, as `checkpoint_trap` would have had it, though a mention in an argument does not go there.
            self.cur_cmd = self.eq_type(cs) & !crate::native::ARMED_BITS;
            self.cur_chr = self.equiv(cs);
        } else {
            self.cur_cs = 0;
            self.cur_cmd = t >> 8;
            self.cur_chr = t & 255;
        }
        self.no_new_control_sequence = true;
    }

    /// §500's loop, with `t` as there: what comes before the left brace of a macro definition, up to anything but an ordinary token or a
    /// parameter that is in order.  §502 does the rest, its errors among them.  True if it has read that left brace, for §500's `goto done1`.
    pub fn capture_parameter_text(&mut self, t: &mut i32) -> bool {
        if self.align_state == 0 {
            return false;
        }
        let mut k = *t;
        let mut i = self.loc();
        loop {
            let x = self.toks[i];
            if x > CS_TOKEN_FLAG {
                let c = self.eq_type(x - CS_TOKEN_FLAG);
                if c == MAC_PARAM || c >= OUTER_CALL {
                    break;
                }
                self.capture_push_one(x, i + 1);
                i += 1;
            } else if is_left_brace(x) {
                self.set_loc(i + 1);
                self.align_state += 1;
                *t = k;
                self.cur_tok = x;
                self.capture_leave();
                return true;
            } else if x < RIGHT_BRACE_LIMIT || x == CS_TOKEN_FLAG || is_out_param(x) {
                break;
            } else if x >> 8 == MAC_PARAM {
                if self.toks[i + 1] != k + 1 || k == ZERO_TOKEN + 9 {
                    break;
                }
                self.capture_push_one(MATCH_TOKEN + (x & 255), i + 2);
                k += 1;
                i += 2;
            } else {
                self.capture_push_one(x, i + 1);
                i += 1;
            }
        }
        self.set_loc(i);
        *t = k;
        false
    }

    /// §520's loop.  A character token needs no table, and a brace only moves `align_state`.  True if it has read the `\fi` or `\else` that ends the text,
    /// as `get_next` would have, for §520's `goto done`.  `cs_token_flag` alone has the meaning of `eqtb[0]`, which is nothing to this loop.
    pub fn capture_skip(&mut self, l: &mut i32) -> bool {
        let mut a = self.align_state;
        if a == 0 {
            return false;
        }
        let mut done = false;
        let mut n = *l;
        let mut i = self.loc();
        // The argument being read without its level, and where to go on in the macro's body after it, or 0.
        let (mut p, mut back) = (0, 0);
        loop {
            let t = self.toks[i];
            if t >= CS_TOKEN_FLAG {
                let cs = t - CS_TOKEN_FLAG;
                let c = self.eq_type(cs);
                if c >= IF_TEST {
                    if c == IF_TEST {
                        n += 1;
                    } else if c == FI_OR_ELSE {
                        if n == 0 {
                            self.cur_cs = cs;
                            self.cur_cmd = c;
                            self.cur_chr = self.equiv(cs);
                            done = true;
                            i += 1;
                            break;
                        }
                        if self.equiv(cs) == FI_CODE {
                            n -= 1;
                        }
                    } else if c >= OUTER_CALL {
                        break;
                    }
                }
            } else if t >= PLAIN_CHARS {
            } else if t < RIGHT_BRACE_LIMIT {
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
                        return false;
                    }
                    i = self.loc();
                    continue;
                }
            } else if is_out_param(t) {
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
        done
    }
}
