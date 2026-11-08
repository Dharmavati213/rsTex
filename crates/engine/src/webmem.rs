//! Where WEB's `mem` would be full.  Token lists do not live in `mem` here (`toks.rs`), and the oracle's do, so without this a job that fills
//! `main_memory` on the oracle goes on here: to the end, or in a loop that nothing else stops.  What the oracle's memory holds can be worked out at any moment:
//!
//! ```text
//! WEB's dyn_used = dyn_used here + tokens in blocks in use, once for each reference to an argument handed on + heads + csname_depth + tb_ptr + tx_ptr
//! ```
//!
//! `heads` being the cells that hold reference counts, which a block has in its header, and `csname_depth` the heads of the lists of `\csname` and `\ifcsname` under way, less the cells of lists that only `native_font.rs` keeps alive.  That is the oracle's number at each `\dump` and shipout that shows it, which `tests/integration/run.py` compares.  The native path of diagrams is another matter: it takes less memory than the macros it stands for, so a job with diagrams has room here that it has not on the oracle.  §120 and §125 then say when to stop:
//!
//! - `get_avail`, when no cell is free: `hi_mem_min<=lo_mem_max`.  `hi_mem_min` is `M` less the most cells ever in use, `M` being `mem_max+1-mem_min`, so that is `dyn_used>=M-lo_mem_max`.
//! - `get_node`, when no free node fits: `lo_mem_max+2<hi_mem_min` says whether memory can grow, and §126 by how much.
//!
//! **What an ordinary run pays for this: nothing on the way, as the sum is not kept.**  The heap's hot paths do not count: a block every 650 instructions or so, at the 5 or so that a count costs, would be
//! near 1%, for what nearly no document needs.  Three limits that are already there, or are compares with a constant, watch how far things have gone: `tb_limit` and `tx_limit` in place of the capacities of the builder and the transient stack, `toks_watch` in place of the end of the
//! heap in `tok_bump`, and `lo_watch` in §144.  The cells of `mem` have none, as a character node takes one with no block and no node: `get_avail` tells of every 65536th that `mem` gives out for the first time, below `hi_mem_min`, which only a new peak of them does, and the bound keeps that many clear of `M`.
//! While `lo_mem_max`, the heap's top, those stacks and the cells of `mem` cannot add up to `M`, these are far away, and a run hears of this module only in [`Tex::web_schedule`], a few times, which is the logarithm of what it uses.
//! That sum is an upper bound, too high by the two cells that each block has, its slack and what is free, and when it comes near `M` a walk of the heap, [`Tex::web_census`], takes those off for good.
//!
//! **Near the end, every rise shows.**  A block comes from a free list and a cell from `avail` without a word to anyone, so from there on those are kept empty, by moving what is on them to `parked` at each [`Tex::web_sync`].  A block then comes through `tok_bump` and a cell through the end of `get_avail`, both out of line,
//! and those are rises seen as they happen, which come off `tb_limit` and `tx_limit`.  What is freed since the last sync can be taken again unseen, but that is no new peak, as it was in use at the sync.  `committed` is so an upper bound between syncs and exact at one, and a limit that comes from it comes too early and is looked at again.
//! `tb_limit` and `tx_limit` stay as near as they can be to the next peak, so that a token that makes one comes here, and it is the stop if it is past the end: `runaway` and `show_context` show what the oracle's show, as `capture.rs` knows how to stop anywhere in a list that goes on the builder.  `arg_share_off`
//! stops arguments being handed on, as that would rise unseen.
//!
//! **What can differ.**  docs/PORTING.md section 15, "Where main memory fills".
#![forbid(unsafe_code)]
#![deny(unreachable_patterns, unused_variables)]
#![allow(non_snake_case, unused_mut, unused_imports, dead_code)]

use crate::access::*;
use crate::consts::*;
use crate::host::*;
use crate::raw::*;
use crate::state::*;
use crate::toks::{ARG_CLEAN, CLASS_CAP, EXACT_CLASSES, NUM_CLASSES, TOKS_FIRST_FREE, TOK_MARK};

crate::pod_struct! {
    /// POD state private to this module.  Add fields here, not to state.rs.  In a checkpoint and a page record as all of it is, and a format keeps `heads` and `peak`.
    pub struct Vars {
        /// The cells that hold reference counts: `def_ref` from §473 on.
        pub heads: i32,
        /// Whether rises are being seen: see the header.
        pub fine: i32,
        /// The most cells seen in use.  `hi_mem_min` is `M` less this, once it is more than the format had.
        pub peak: i32,
        /// §1490's `hi_mem_min`: `main_memory` less the format's `peak`.  0 under INITEX, which has none yet.
        pub hi0: i32,
        /// While `fine`: an upper bound on the tokens in blocks in use.  Exact at a sync.
        pub committed: i32,
        /// While `fine`: what arguments handed on before add, which only comes down.
        pub shares: i32,
        /// Cells of the heap that held no token at the last census.  What the coarse bound can take off for good, as a new block adds its two cells and its slack too.
        pub credit: i32,
        /// What has been handed on since the last census, of arguments too long for an exact size class.
        pub big_shares: i32,
        /// The value of `lo_mem_max` at which §144 asks again.
        pub lo_watch: i32,
        /// The value of `toks_top` at which `tok_bump` asks again.
        pub toks_watch: i32,
        /// While `fine`: what `pf_held_cells` said at the last look.
        pub held: i32,
        /// While `fine`: the cells of `avail`.
        pub parked_avail: i32,
        /// While `fine`: the free blocks of each class.
        pub parked: A<i32, 256>,
    }
}

/// Non-POD state private to this module, in `self.host`.  Rarely needed.
#[derive(Default)]
pub struct HVars {}

/// What the coarse bound keeps clear of `M`.  It is also how near the end the next peak is what `tb_limit` looks for: `Tex::web_limits`.
const MARGIN: i64 = 1 << 16;

/// The most §144 adds to `lo_mem_max` at a time.  `lo_watch` is asked before a growth, so `lo_mem_max` can go past it by one less than this.
const LO_STEP: i64 = 1000;

/// Which of the four that the limits watch takes the `need` of [`Tex::web_schedule`].  The coarse room has `need` taken off once, so only that one's limit has it on top of its quarter.
#[derive(Clone, Copy)]
pub enum Taker {
    /// The builder, `tb_limit`: `need` more tokens.
    Builder,
    /// The transient stack, `tx_limit`.
    Transient,
    /// The heap, `toks_watch`: a new block, two cells more than its class holds.
    Heap,
    /// `mem`, `lo_watch`: §144 is about to grow `lo_mem_max`.  Also a caller that takes nothing.
    Mem,
}

impl Tex {
    /// `M`, §138's `mem_max+1-mem_min`.  The two extras both go on the top: there is no room below `mem_bot` here.
    #[inline]
    fn web_m(&self) -> i64 {
        self.main_memory as i64 + self.extra_mem_top as i64 + self.extra_mem_bot as i64
    }

    /// The most `dyn_used` can be: one more and `get_avail` has `hi_mem_min<=lo_mem_max`.
    #[inline]
    fn web_top(&self) -> i64 {
        self.web_m() - self.lo_mem_max as i64 - 1
    }

    /// `hi_mem_min`, as far as what has been seen says.
    fn web_hi_now(&self) -> i64 {
        let hi = self.web_m() - self.v_webmem.peak as i64;
        if self.v_webmem.hi0 > 0 { hi.min(self.v_webmem.hi0 as i64) } else { hi }
    }

    #[inline]
    fn web_note_peak(&mut self, v: i64) {
        if v > self.v_webmem.peak as i64 {
            self.v_webmem.peak = v as i32;
        }
    }

    /// The tokens of a free block, whose second cell is the link of its free list and whose terminator is where it was.
    pub(crate) fn web_free_len(&self, p: i32, c: usize) -> i64 {
        if c < EXACT_CLASSES {
            c as i64
        } else {
            let mut i = p + 2;
            while self.toks[i] != 0 {
                i += 1;
            }
            (i - (p + 1)) as i64
        }
    }

    /// A walk of the heap: tokens in blocks in use, what arguments handed on add to those, and the tokens that would fit in what is free.
    fn web_census(&mut self) -> (i64, i64, i64) {
        for c in 0..NUM_CLASSES {
            let mut p = self.tok_free[c as i32];
            while p != 0 {
                let next = self.toks[p + 1];
                self.toks[p] |= TOK_MARK;
                p = next;
            }
        }
        let top = self.toks_top;
        let mut p = TOKS_FIRST_FREE;
        let (mut used, mut shares, mut free) = (0i64, 0i64, 0i64);
        while p < top {
            let h = self.toks[p];
            let c = (h & 0xFF) as usize;
            if h >= 0 {
                let n = if c < EXACT_CLASSES { c as i64 } else { self.tok_len(p) as i64 };
                used += n;
                if h & ARG_CLEAN != 0 {
                    shares += ((h >> 8) & 0xFFFFF) as i64 * n;
                }
            } else {
                free += CLASS_CAP[c] as i64;
            }
            p += CLASS_CAP[c] + 2;
        }
        for c in 0..NUM_CLASSES {
            let mut p = self.tok_free[c as i32];
            while p != 0 {
                let next = self.toks[p + 1];
                self.toks[p] &= !TOK_MARK;
                p = next;
            }
        }
        (used, shares, free)
    }

    /// What arguments handed on add, for those still on the parameter stack.  Each reference is a list of its own on the oracle, and `{#n}` takes a reference from that stack, where `web_go_fine` has found them all.  None is made while `fine`, so this only comes down, and once at 0 it stays.
    fn web_shares(&mut self) -> i64 {
        if self.v_webmem.shares == 0 {
            return 0;
        }
        let mut seen: Vec<i32> = Vec::new();
        for k in 0..self.param_ptr {
            let p = self.param_stack[k];
            if p != NULL && self.toks[p] & ARG_CLEAN != 0 && (self.toks[p] >> 8) & 0xFFFFF != 0 {
                seen.push(p);
            }
        }
        seen.sort_unstable();
        seen.dedup();
        let mut s = 0i64;
        for p in seen {
            s += ((self.toks[p] >> 8) & 0xFFFFF) as i64 * self.tok_len(p) as i64;
        }
        self.v_webmem.shares = s as i32;
        s
    }

    /// WEB's `dyn_used`, which `\tracingstats` and `\dump` print.  A walk of the heap, or a sync when `fine`.
    pub fn web_dyn_used(&mut self) -> i32 {
        let v = if self.v_webmem.fine != 0 {
            self.web_sync()
        } else {
            let (used, shares, _) = self.web_census();
            self.dyn_used as i64 + used + shares + self.v_webmem.heads as i64 + self.csname_depth as i64 + self.tb_ptr as i64 + self.tx_ptr as i64 - self.pf_held_cells()
        };
        self.web_note_peak(v);
        v as i32
    }

    /// `hi_mem_min-lo_mem_max-1`, as far as what has been seen says: "still untouched" in §639.
    pub fn web_untouched(&mut self) -> i32 {
        (self.web_hi_now() - self.lo_mem_max as i64 - 1) as i32
    }

    /// The sum at a sync: what was freed since the last goes from `committed` and to `parked`.
    fn web_sync(&mut self) -> i64 {
        let mut freed = 0i64;
        for c in 0..NUM_CLASSES {
            let first = self.tok_free[c as i32];
            if first != 0 {
                let mut p = first;
                loop {
                    freed += self.web_free_len(p, c);
                    let next = self.toks[p + 1];
                    if next == 0 {
                        break;
                    }
                    p = next;
                }
                self.toks[p + 1] = self.v_webmem.parked[c as i32];
                self.v_webmem.parked[c as i32] = first;
                self.tok_free[c as i32] = 0;
            }
        }
        self.v_webmem.committed -= freed as i32;
        let first = self.avail;
        if first != NULL {
            let mut p = first;
            while self.link(p) != NULL {
                p = self.link(p);
            }
            self.set_link(p, self.v_webmem.parked_avail);
            self.v_webmem.parked_avail = first;
            self.avail = NULL;
        }
        self.dyn_used as i64 + self.v_webmem.committed as i64 + self.web_shares() + self.v_webmem.heads as i64 + self.csname_depth as i64 + self.tb_ptr as i64 + self.tx_ptr as i64 - self.v_webmem.held as i64
    }

    /// `tb_limit` and `tx_limit`, with the sum at `v` and the two stacks at `tb` and `tx`.  Each may have half of what there is, as both may rise.  Far from the end that is half way there, as a sync costs a walk of 256 lists.  Near it, it is half way to the next peak, so that each token past it comes to [`Tex::web_room`].
    fn web_limits(&mut self, v: i64, tb: i32, tx: i32) {
        let top = self.web_top();
        let room = top - v;
        let b = if room > MARGIN { room } else { (self.v_webmem.peak as i64).min(top) - v }.max(0) / 2;
        self.tb_limit = (tb as i64 + b).min(TB_CAP as i64) as i32;
        self.tx_limit = (tx as i64 + b).min(TX_CAP as i64) as i32;
    }

    /// What is parked goes back to where WEB's `\dump` and `tok_reclaim` look for it.
    fn web_unpark(&mut self) {
        for c in 0..NUM_CLASSES {
            let first = self.v_webmem.parked[c as i32];
            if first != 0 {
                let mut p = first;
                while self.toks[p + 1] != 0 {
                    p = self.toks[p + 1];
                }
                self.toks[p + 1] = self.tok_free[c as i32];
                self.tok_free[c as i32] = first;
                self.v_webmem.parked[c as i32] = 0;
            }
        }
    }

    /// `\dump`: a format has neither the mode nor anything parked.
    pub fn web_leave_fine(&mut self) {
        if self.v_webmem.fine != 0 {
            self.web_sync();
            self.web_unpark();
            self.avail = self.v_webmem.parked_avail;
            self.v_webmem.parked_avail = NULL;
            self.v_webmem.fine = 0;
        }
    }

    /// What a format's load sets: the first use of each limit then comes to `web_schedule`.
    pub fn web_loaded(&mut self, heads: i32, peak: i32) {
        self.v_webmem.heads = heads;
        self.v_webmem.peak = peak;
        self.v_webmem.hi0 = self.main_memory - peak;
    }

    /// `tok_reclaim` needs the free blocks where it looks for them, and then the lists are others.
    pub fn web_before_reclaim(&mut self) {
        if self.v_webmem.fine != 0 {
            self.web_unpark();
        }
    }

    /// `p` is the block it came to, whose tokens the caller will add.
    pub fn web_after_reclaim(&mut self, p: i32) {
        if self.v_webmem.fine != 0 {
            let (used, _, _) = self.web_census();
            let c = if p != 0 { (self.toks[p] & 0xFF) as usize } else { 0 };
            self.v_webmem.committed = (used - if c < EXACT_CLASSES { c as i64 } else { self.tok_len(p) as i64 }) as i32;
            for c in 0..NUM_CLASSES {
                self.v_webmem.parked[c as i32] = self.tok_free[c as i32];
                self.tok_free[c as i32] = 0;
            }
        }
    }

    /// From the coarse mode to the fine.
    #[cold]
    fn web_go_fine(&mut self) {
        self.arg_share_off();
        let (used, shares, _) = self.web_census();
        self.v_webmem.fine = 1;
        self.v_webmem.committed = used as i32;
        self.v_webmem.shares = shares as i32;
        for c in 0..NUM_CLASSES {
            self.v_webmem.parked[c as i32] = self.tok_free[c as i32];
            self.tok_free[c as i32] = 0;
        }
        self.v_webmem.toks_watch = 0;
        self.v_webmem.lo_watch = 0;
        self.v_webmem.held = self.pf_held_cells() as i32;
        let v = self.web_sync();
        self.web_note_peak(v);
        self.web_limits(v, self.tb_ptr, self.tx_ptr);
    }

    /// The coarse bound, with `need` more cells than there are: what is left of `M`.  A small argument handed on has a place on the parameter stack, and the rest are in `big_shares`.
    fn web_coarse_room(&self, need: i64) -> i64 {
        let cells = (self.mem_end as i64 + 1 - self.hi_mem_min as i64).max(self.dyn_used as i64);
        let shares = self.v_webmem.big_shares as i64 + self.param_ptr as i64 * EXACT_CLASSES as i64;
        self.web_m() - MARGIN
            - (self.lo_mem_max as i64 + (self.toks_top - TOKS_FIRST_FREE) as i64 - self.v_webmem.credit as i64)
            - (self.tb_ptr as i64 + self.tx_ptr as i64 + self.csname_depth as i64 + cells + shares + self.v_webmem.heads as i64 + need)
    }

    /// Where the limits go while the bound is clear of `M`: each has a quarter of what is left, and the one of `taker` has `need` as well, which is what the caller is about to take, so that no more than the bound has room for can rise unseen.
    /// If the bound comes to `M`, a census says how much of it was air, and if that is not enough, rises have to be seen from here on.
    #[cold]
    #[inline(never)]
    pub fn web_schedule(&mut self, need: i64, taker: Taker) {
        if self.v_webmem.fine != 0 {
            return;
        }
        if self.main_memory <= 0 {
            // As under the unit tests' machine: then the capacities of the arrays are all there is.
            self.tb_limit = TB_CAP as i32;
            self.tx_limit = TX_CAP as i32;
            self.v_webmem.toks_watch = self.toks_limit() as i32;
            self.v_webmem.lo_watch = i32::MAX;
            return;
        }
        let mut r = self.web_coarse_room(need);
        if r <= 0 {
            let (used, shares, free) = self.web_census();
            self.v_webmem.big_shares = shares.min(i32::MAX as i64) as i32;
            self.v_webmem.credit = ((self.toks_top - TOKS_FIRST_FREE) as i64 - used - free) as i32;
            r = self.web_coarse_room(need);
            if r <= 0 {
                self.web_go_fine();
                return;
            }
        }
        let s = r / 4;
        let (b, x, h, l) = match taker {
            Taker::Builder => (need, 0, 0, 0),
            Taker::Transient => (0, need, 0, 0),
            Taker::Heap => (0, 0, need, 0),
            Taker::Mem => (0, 0, 0, need),
        };
        self.tb_limit = (self.tb_ptr as i64 + b + s).min(TB_CAP as i64) as i32;
        self.tx_limit = (self.tx_ptr as i64 + x + s).min(TX_CAP as i64) as i32;
        self.v_webmem.toks_watch = (self.toks_top as i64 + h + s).min(self.toks_limit()) as i32;
        self.v_webmem.lo_watch = (self.lo_mem_max as i64 + l + s - (LO_STEP - 1)).min(i32::MAX as i64) as i32;
    }

    /// `def_ref:=get_avail`, or the like: one cell more, with the list to come.  The limits do not hear of it, as this is as near a hot path as any here: they leave half of what there is, so that it takes some of these in a row with no token between them to get past one.
    #[inline(always)]
    pub fn web_head(&mut self) {
        self.v_webmem.heads += 1;
    }

    /// A list too long for an exact size class has been handed on: a further list of its length on the oracle.
    pub fn web_big_share(&mut self, cap: i32) {
        self.v_webmem.big_shares = self.v_webmem.big_shares.saturating_add(cap);
        self.tb_limit -= cap;
        self.tx_limit -= cap;
    }

    /// One of the two stacks has come to its limit and wants `n` more tokens: whether they fit, with the sum and its peak seen to.  If not, `limit` tokens are as far as they fit.
    fn web_stack_room(&mut self, n: i32, on_tx: bool) -> Result<(), i64> {
        if self.v_webmem.fine == 0 {
            self.web_schedule(n as i64, if on_tx { Taker::Transient } else { Taker::Builder });
            if self.v_webmem.fine == 0 {
                return Ok(());
            }
        }
        let v = self.web_sync();
        let room = self.web_top() - v;
        if n as i64 > room {
            self.web_note_peak(v);
            return Err(room.max(0));
        }
        let w = v + n as i64;
        self.web_note_peak(w);
        if on_tx {
            self.web_limits(w, self.tb_ptr, self.tx_ptr + n);
        } else {
            self.web_limits(w, self.tb_ptr + n, self.tx_ptr);
        }
        Ok(())
    }

    /// The builder has come to `tb_limit` and wants `n` more tokens: whether they fit.  If not, `tb_limit` is as far as they fit, which is where the literal loop stops.
    #[cold]
    #[inline(never)]
    pub fn web_room(&mut self, n: i32) -> bool {
        if self.tb_ptr as i64 + n as i64 > TB_CAP as i64 {
            self.tb_limit = TB_CAP as i32;
            return false;
        }
        match self.web_stack_room(n, false) {
            Ok(()) => true,
            Err(room) => {
                self.tb_limit = (self.tb_ptr as i64 + room) as i32;
                false
            }
        }
    }

    /// The same for one more token on the transient stack.
    #[cold]
    #[inline(never)]
    pub fn web_tx_room(&mut self) -> bool {
        if self.tx_ptr as usize >= TX_CAP {
            self.tx_limit = TX_CAP as i32;
            return false;
        }
        self.web_stack_room(1, true).is_ok()
    }

    /// A block out of what is parked, or else a new one.
    fn web_take(&mut self, c: usize) -> i32 {
        let p = self.v_webmem.parked[c as i32];
        if p != 0 {
            self.v_webmem.parked[c as i32] = self.toks[p + 1];
            self.toks[p] = c as i32;
            p
        } else {
            self.tok_bump_real(c)
        }
    }

    /// `n` tokens have gone into a block.  `moved`: they were on the builder, or in the list this replaces, so the sum does not rise.  Otherwise a rise: false if it goes past the end.
    fn web_rose(&mut self, n: i32, moved: bool) -> bool {
        self.v_webmem.committed += n;
        self.tb_limit -= n;
        self.tx_limit -= n;
        if !moved && (self.tb_limit <= self.tb_ptr || self.tx_limit <= self.tx_ptr) {
            let v = self.web_sync();
            if v > self.web_top() {
                return false;
            }
            self.web_note_peak(v);
            self.web_limits(v, self.tb_ptr, self.tx_ptr);
        }
        true
    }

    /// `tok_bump` has come to `toks_watch`.  `n` is the tokens, which a caller with a large class has.
    #[cold]
    #[inline(never)]
    pub fn web_bump(&mut self, c: usize, n: i32, moved: bool) -> i32 {
        if self.v_webmem.fine == 0 {
            self.web_schedule((CLASS_CAP[c] + 2) as i64, Taker::Heap);
        }
        if self.v_webmem.fine == 0 {
            return self.tok_bump_real(c);
        }
        let p = self.web_take(c);
        if !self.web_rose(n, moved) {
            self.tok_overflow();
        }
        p
    }

    /// The same for a caller that goes back to the literal loop if there is no room: 0 then, with nothing taken.
    #[cold]
    #[inline(never)]
    pub fn web_try_bump(&mut self, c: usize, n: i32) -> i32 {
        if self.v_webmem.fine == 0 {
            self.web_schedule((CLASS_CAP[c] + 2) as i64, Taker::Heap);
        }
        if self.v_webmem.fine == 0 {
            return self.tok_try_bump_real(c);
        }
        let q = if self.v_webmem.parked[c as i32] != 0 {
            self.web_take(c)
        } else {
            let q = self.tok_try_bump_real(c);
            if q == 0 {
                return 0;
            }
            q
        };
        if !self.web_rose(n, false) {
            self.toks[q + 1] = self.tok_free[c as i32];
            self.tok_free[c as i32] = q;
            return 0;
        }
        q
    }

    /// What `web_large_reused` wants to know of a block of a large class, while it is still free.
    #[cold]
    pub fn web_free_len_if_fine(&self, p: i32, c: usize) -> i64 {
        if self.v_webmem.fine != 0 { self.web_free_len(p, c) } else { 0 }
    }

    /// A block of a large class that came off a free list: it had another length than this one, and `committed` has that one until the next sync.
    #[cold]
    pub fn web_large_reused(&mut self, was: i64, n: i32, moved: bool) -> bool {
        if self.v_webmem.fine == 0 {
            return true;
        }
        self.web_rose((n as i64 - was) as i32, moved)
    }

    /// §144: what `hi_mem_min` is for `get_node`, now that `lo_mem_max` has come to `lo_watch`.
    #[cold]
    #[inline(never)]
    pub fn web_hi(&mut self) -> i32 {
        if self.v_webmem.fine == 0 {
            self.web_schedule(LO_STEP, Taker::Mem);
        }
        if self.v_webmem.fine == 0 {
            return MEM_SPLIT;
        }
        self.v_webmem.held = self.pf_held_cells() as i32;
        let v = self.web_sync();
        self.web_note_peak(v);
        self.web_limits(v, self.tb_ptr, self.tx_ptr);
        self.web_hi_now().min(MEM_SPLIT as i64) as i32
    }

    /// §144 has moved `lo_mem_max` while `fine`.
    #[cold]
    pub fn web_lo_grew(&mut self) {
        let v = self.web_sync();
        self.web_limits(v, self.tb_ptr, self.tx_ptr);
    }

    /// `get_avail` with `avail` empty, while `fine`: a rise of one, which comes off the two limits as a block's does in [`Tex::web_rose`], and §120's test once one of them is reached.  Until then the sum is below the end, and below the next peak
    /// when that is near, as [`Tex::web_limits`] sets them, so a sync for each cell would say nothing more, and it would cost a walk of the free lists for each character of a job that has come near the end.  The cell comes from what is
    /// parked, or is one `mem` never gave out before, below `hi_mem_min`, as `mem_end` is at `MEM_MAX` from the start here.
    #[cold]
    #[inline(never)]
    pub fn web_cell(&mut self) -> i32 {
        self.tb_limit -= 1;
        self.tx_limit -= 1;
        if self.tb_limit <= self.tb_ptr || self.tx_limit <= self.tx_ptr {
            let v = self.web_sync() + 1;
            if v > self.web_top() {
                self.runaway(); // if memory is exhausted, display possible runaway text
                self.overflow(b"main memory size", self.main_memory_size());
            }
            self.web_note_peak(v);
            self.web_limits(v, self.tb_ptr, self.tx_ptr);
        }
        let p = self.v_webmem.parked_avail;
        if p != NULL {
            self.v_webmem.parked_avail = self.link(p);
            p
        } else {
            self.hi_mem_min -= 1;
            if self.hi_mem_min < MEM_SPLIT {
                self.runaway();
                self.overflow(b"main memory size", self.main_memory_size());
            }
            self.hi_mem_min
        }
    }

    /// `mem` has given out another 65536 cells that it never gave out before: its share of the coarse bound has moved.
    #[cold]
    pub fn web_cells_grew(&mut self) {
        self.web_schedule(0, Taker::Mem);
    }
}
