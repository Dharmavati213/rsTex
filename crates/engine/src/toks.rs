//! §311-318 Part 20: token lists; and the token heap of DESIGN.md section 5.
//!
//! Token lists do not live in `mem`.  They live in the contiguous `i32` heap `self.toks`, as
//! blocks:
//!
//! ```text
//! [hdr][t0][t1]...[tn][0]      hdr = (rc << 8) | size_class
//! ```
//!
//! - A *list pointer* (`TokPtr`, plain `i32`) is the index of the header.  `0` is the null list.
//!   `toks[0] = toks[1] = 0`, so both `loc = 0` and `loc = 0 + 1` read as end of list.
//! - `rc` follows TeX's `token_ref_count` convention: references minus one, so a fresh list has
//!   `rc = 0`.  Every list has a header, including argument lists and backed-up tokens, so
//!   `flush_list` is O(1) by size class, with no walk.
//! - The token value 0 never occurs in a list, so 0 terminates it.  `cmd` 0 character tokens cannot
//!   occur, §311; `cur_tok=0` from the end of a `\read` line is never stored, §509.  One path in WEB
//!   does store it: `\pdfprimitive\fi` or `\pdfprimitive\input`, where they make §405 run with
//!   `cur_cs=0`.  It backs up `cs_token_flag+0`, whose reread reads `eqtb[0]`, so `Tex::alloc` sets
//!   `equiv(0)=1`.  That token is then 1, which displays and acts as WEB's 0 does, `\BAD.` and cmd 0;
//!   docs/PORTING.md section 15, "Known divergences".  `tb_push` checks this under `checked`.
//! - The size class gives the capacity: exact classes up to 64 tokens, geometric above.  A block
//!   occupies `capacity + 2` cells.  Per-class free lists; otherwise bump.  Only when the bump space has
//!   run out are free blocks split and joined: [`Tex::tok_reclaim`].  No block in use ever moves.
//! - Lists are built on the LIFO builder stack `self.tb`, then committed.  Nesting is LIFO because
//!   nested expansion finishes before the outer builder resumes.
//! - `\csname`, `\ifcsname` and `scan_keyword` collect their tokens on a second stack `self.tx`, so
//!   that the list on top of `tb` is always the one `runaway` must display.  See [`Tex::tx_mark`].
//!
//! docs/PORTING.md section "Token lists" translates every WEB idiom to this API.
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
        /// `tight_heap`'s end of the bump space, set at the first block after a format is loaded.  In the image, so that a run from a checkpoint or a record reclaims where a cold one does.
        #[cfg(feature = "tight_heap")]
        pub tight_limit: i32,
    }
}

/// Non-POD state private to this module, in `self.host`.  Rarely needed.
#[derive(Default)]
pub struct HVars {}

/// First cell of the heap proper; below it are the null list, the fixed lists and `ANY_USER_OUTER`.
pub const TOKS_FIRST_FREE: i32 = OMIT_TEMPLATE + 4;
/// A cell above the fixed lists, nonzero once a control sequence other than the frozen ones has been given a meaning for which §379 must look further, `outer_call` to
/// `dont_expand`.  It never goes back to 0.  Here so that a format and a checkpoint carry it, as they do `toks` from `NULL_LIST` up, and so that no level reads it
/// as a token: nothing but a list's own `loc` comes above `NULL_LIST`, and no list holds this cell.
const ANY_USER_OUTER: i32 = OMIT_TEMPLATE + 3;
const _: () = assert!(ANY_USER_OUTER > NULL_LIST && ANY_USER_OUTER < TOKS_FIRST_FREE);
// §14 "we will want null_list>255": §532 tells frozen_endv, whose cmd is below active_char, from a character only by cur_chr>255.
const _: () = assert!(NULL_LIST > 255);
// And `get_next_leaf` reads `toks[loc]` without looking at the state: a file level's `loc` is at most `buf_size`, below
// BUF_CAP, and must read 0, the end of a list, which sends it to the slow path.  So the heap starts above that.
const _: () = assert!(NULL_LIST as usize > BUF_CAP);
/// Number of exact size classes: capacities 0..=EXACT_CLASSES-1.
pub const EXACT_CLASSES: usize = 65;
/// On an argument's block, above any count of references it can have: `capture.rs`.
pub const ARG_CLEAN: i32 = 1 << 28;
/// Number of size classes; `hdr & 0xFF` indexes `CLASS_CAP`.
pub const NUM_CLASSES: usize = 256;
/// Cells above every block, for what [`Tex::tb_show`] copies when the heap is full: `runaway` shows at most `error_line-10` characters, and that
/// is under 255 tokens unless a token prints nothing.
const TOKS_SHOW_ROOM: usize = 256;
/// Where the bump space ends.
const TOKS_LIMIT: usize = TOKS_CAP - TOKS_SHOW_ROOM;
/// With the feature `tight_heap`, how many cells a run that loaded a format has above what the format holds.  Ordinary documents then get to
/// [`Tex::tok_reclaim`], for differential testing.  One that has more than this in use stops, as it would on a heap of that size.
#[cfg(feature = "tight_heap")]
const TIGHT_CELLS: i32 = 400_000;
/// In a free block's header during [`Tex::tok_coalesce`].  A header in use is not negative.
pub(crate) const TOK_MARK: i32 = i32::MIN;

/// Capacity in tokens of each size class.  Above the exact classes, each is 9/8 of the previous,
/// so a list wastes at most about 1/8 of its block.  The tail saturates at the heap's size.
pub const CLASS_CAP: [i32; NUM_CLASSES] = {
    let mut t = [0i32; NUM_CLASSES];
    let mut c = 0;
    while c < EXACT_CLASSES {
        t[c] = c as i32;
        c += 1;
    }
    while c < NUM_CLASSES {
        let prev = t[c - 1] as i64;
        let next = prev + prev / 8 + 1;
        t[c] = if next > TOKS_CAP as i64 { TOKS_CAP as i32 } else { next as i32 };
        c += 1;
    }
    t
};

/// The size class that holds `n` tokens.
#[inline(always)]
pub fn class_for(n: i32) -> usize {
    if (n as usize) < EXACT_CLASSES {
        n as usize
    } else {
        class_for_large(n)
    }
}

#[cold]
fn class_for_large(n: i32) -> usize {
    let mut lo = EXACT_CLASSES;
    let mut hi = NUM_CLASSES - 1;
    while lo < hi {
        let mid = (lo + hi) / 2;
        if CLASS_CAP[mid] >= n {
            hi = mid;
        } else {
            lo = mid + 1;
        }
    }
    lo
}

impl Tex {
    /// Sets up the heap: the null list, the reserved fixed lists, and empty free lists.
    /// Part of `@<Initialize table entries@>`, in `initialize`; call before anything touches tokens.
    pub fn toks_init(&mut self) {
        // toks[0]=toks[1]=0 and all below NULL_LIST: the image starts as zeroes and these cells are never written.
        // null_list: permanently empty, §180; its rc may grow (\let of \endtemplate), it is never freed.
        self.toks[NULL_LIST] = 0;
        self.toks[NULL_LIST + 1] = 0;
        // omit_template: the constant list `\endtemplate`, §966.
        self.toks[OMIT_TEMPLATE] = 1;
        self.toks[OMIT_TEMPLATE + 1] = END_TEMPLATE_TOKEN;
        self.toks[OMIT_TEMPLATE + 2] = 0;
        self.toks_top = TOKS_FIRST_FREE;
        self.tb_ptr = 0;
        self.tx_ptr = 0;
        self.tb_def_mark = 0;
        self.tb_match_mark = 0;
        self.tb_align_mark = 0;
        for c in 0..NUM_CLASSES {
            self.tok_free[c as i32] = 0;
        }
    }

    // -----------------------------------------------------------------------
    // Allocation.
    // -----------------------------------------------------------------------

    /// A block of the given class with `rc = 0`.  Its contents, including the terminator, are the
    /// caller's to write.
    #[inline(always)]
    fn tok_alloc_class(&mut self, c: usize) -> i32 {
        let p = self.tok_free[c as i32];
        if p != 0 {
            self.tok_free[c as i32] = self.toks[p + 1];
            self.toks[p] = c as i32;
            p
        } else {
            self.tok_bump(c, CLASS_CAP[c], false)
        }
    }

    /// A new block, as far as `toks_watch`, which is never past the end of the bump space: `webmem.rs`.  `n` is its tokens, and `moved` says they are already counted somewhere.
    #[inline]
    fn tok_bump(&mut self, c: usize, n: i32, moved: bool) -> i32 {
        let p = self.toks_top;
        let cells = CLASS_CAP[c] + 2;
        if cells as i64 + p as i64 > self.v_webmem.toks_watch as i64 {
            return self.web_bump(c, n, moved);
        }
        self.toks_top = p + cells;
        self.toks[p] = c as i32;
        p
    }

    /// A new block as far as the true end of the bump space.
    pub(crate) fn tok_bump_real(&mut self, c: usize) -> i32 {
        let p = self.toks_top;
        let cells = CLASS_CAP[c] + 2;
        if cells as i64 + p as i64 > self.toks_limit() {
            return self.tok_reclaim(c);
        }
        self.toks_top = p + cells;
        self.toks[p] = c as i32;
        p
    }

    /// The bump space has run out.  WEB has one memory, whose free words serve any list, and a class here keeps its free blocks.  So a job that
    /// uses lists of one length after another would come to the end of the heap with the oracle's memory nowhere near full, and stop where
    /// the oracle goes on (`tests/integration/toks/heap_classes.tex`).  So a block comes out of a free block of a larger class, and if there is
    /// none, what is free is first put back together.  This is the only time blocks change size, and none in use moves.
    #[cold]
    #[inline(never)]
    fn tok_reclaim(&mut self, c: usize) -> i32 {
        let p = self.tok_try_reclaim(c);
        if p == 0 {
            self.tok_overflow();
        }
        p
    }

    /// A block for class `c` by those means, or 0.
    #[cold]
    #[inline(never)]
    fn tok_try_reclaim(&mut self, c: usize) -> i32 {
        self.web_before_reclaim();
        let p = self.tok_reclaim_block(c);
        self.web_after_reclaim(p);
        p
    }

    fn tok_reclaim_block(&mut self, c: usize) -> i32 {
        if let Some(p) = self.tok_split(c) {
            return p;
        }
        self.tok_coalesce();
        // The end of the heap may be free again, and so may a block of just this size.
        let p = self.toks_top;
        let cells = CLASS_CAP[c] + 2;
        if cells as i64 + p as i64 <= self.toks_limit() {
            self.toks_top = p + cells;
            self.toks[p] = c as i32;
            return p;
        }
        let p = self.tok_free[c as i32];
        if p != 0 {
            self.tok_free[c as i32] = self.toks[p + 1];
            self.toks[p] = c as i32;
            return p;
        }
        self.tok_split(c).unwrap_or(0)
    }

    /// A block for class `c` out of a free block of a larger class, the rest of which stays free.  It can be of that larger class if one cell
    /// would be left, which is no block.
    fn tok_split(&mut self, c: usize) -> Option<i32> {
        let cells = CLASS_CAP[c] + 2;
        for d in c + 1..NUM_CLASSES {
            let p = self.tok_free[d as i32];
            if p != 0 {
                self.tok_free[d as i32] = self.toks[p + 1];
                let rest = CLASS_CAP[d] + 2 - cells;
                if rest == 1 {
                    self.toks[p] = d as i32;
                } else {
                    self.toks[p] = c as i32;
                    self.tok_free_run(p + cells, rest);
                }
                return Some(p);
            }
        }
        None
    }

    /// `n` free cells at `p`, not 1, become free blocks, each as large as it can be.
    fn tok_free_run(&mut self, mut p: i32, mut n: i32) {
        while n > 0 {
            let mut d = class_for(n - 2);
            if CLASS_CAP[d] + 2 > n {
                d -= 1;
            }
            if n - (CLASS_CAP[d] + 2) == 1 {
                d -= 1;
            }
            self.toks[p] = d as i32;
            self.toks[p + 1] = self.tok_free[d as i32];
            self.tok_free[d as i32] = p;
            p += CLASS_CAP[d] + 2;
            n -= CLASS_CAP[d] + 2;
        }
    }

    /// Joins free blocks that lie next to each other, and gives those at the end of the heap back to the bump space.  Every cell from
    /// `TOKS_FIRST_FREE` to `toks_top` is in a block, whose header holds its class, in use or free.
    fn tok_coalesce(&mut self) {
        for c in 0..NUM_CLASSES {
            let mut p = self.tok_free[c as i32];
            while p != 0 {
                let next = self.toks[p + 1];
                self.toks[p] |= TOK_MARK;
                p = next;
            }
            self.tok_free[c as i32] = 0;
        }
        let top = self.toks_top;
        let mut p = TOKS_FIRST_FREE;
        let mut run = 0;
        while p < top {
            let h = self.toks[p];
            if h >= 0 && run != 0 {
                self.tok_free_run(run, p - run);
                run = 0;
            } else if h < 0 && run == 0 {
                run = p;
            }
            p += CLASS_CAP[(h & 0xFF) as usize] + 2;
        }
        #[cfg(feature = "checked")]
        assert!(p == top, "a block runs over the top of the heap: {} {}", p, top);
        if run != 0 {
            // As it is above the top of a heap that never came down.  A record holds the heap as far as it is live, so what was left here would be in a cold run and not in one resumed, and
            // a block made here later, which writes as far as its terminator, would keep it: DESIGN section 7, page records.
            self.toks.as_mut_slice()[run as usize..top as usize].fill(0);
            self.toks_top = run;
        }
    }

    #[cold]
    #[inline(never)]
    pub(crate) fn tok_overflow(&mut self) -> ! {
        self.runaway(); // if memory is exhausted, display possible runaway text
        self.overflow(b"main memory size", self.main_memory_size())
    }

    /// §138's `mem_max+1-mem_min`, which is what a format was built with and the two extras of this run: `webmem.rs` stops a job there.
    #[cold]
    pub fn main_memory_size(&self) -> i32 {
        self.main_memory + self.extra_mem_top + self.extra_mem_bot
    }

    /// A block for `n` tokens with `rc = 0`.  The caller writes the tokens and the terminator.
    #[inline(always)]
    pub fn tok_alloc(&mut self, n: i32) -> i32 {
        if (n as usize) < EXACT_CLASSES {
            self.tok_alloc_class(n as usize)
        } else {
            self.tok_alloc_large(n, false)
        }
    }

    /// `tok_alloc` for tokens that are on the builder, or in a list this one replaces: WEB's cells for them are in use already, so there is no rise.
    #[inline(always)]
    fn tok_alloc_moved(&mut self, n: i32) -> i32 {
        if (n as usize) < EXACT_CLASSES {
            let c = n as usize;
            let p = self.tok_free[c as i32];
            if p != 0 {
                self.tok_free[c as i32] = self.toks[p + 1];
                self.toks[p] = c as i32;
                p
            } else {
                self.tok_bump(c, n, true)
            }
        } else {
            self.tok_alloc_large(n, true)
        }
    }

    /// A block for a list too long for an exact size class: the only blocks whose capacity is not their length, which `webmem.rs` has to be told.
    #[cold]
    #[inline(never)]
    fn tok_alloc_large(&mut self, n: i32, moved: bool) -> i32 {
        let c = class_for_large(n);
        let p = self.tok_free[c as i32];
        if p == 0 {
            return self.tok_bump(c, n, moved);
        }
        let was = self.web_free_len_if_fine(p, c);
        self.tok_free[c as i32] = self.toks[p + 1];
        self.toks[p] = c as i32;
        if !self.web_large_reused(was, n, moved) {
            self.tok_overflow();
        }
        p
    }

    #[cfg(not(feature = "tight_heap"))]
    #[inline(always)]
    pub(crate) fn toks_limit(&self) -> i64 {
        TOKS_LIMIT as i64
    }

    #[cfg(feature = "tight_heap")]
    pub(crate) fn toks_limit(&mut self) -> i64 {
        if self.ini_version {
            return TOKS_LIMIT as i64;
        }
        if self.v_toks.tight_limit == 0 {
            self.v_toks.tight_limit = self.toks_top + TIGHT_CELLS;
        }
        self.v_toks.tight_limit as i64
    }

    /// `tok_alloc`, or 0 if the heap has no room.
    #[inline(always)]
    pub fn tok_try_alloc(&mut self, n: i32) -> i32 {
        if (n as usize) >= EXACT_CLASSES {
            return self.tok_try_alloc_large(n);
        }
        let c = n as usize;
        let p = self.tok_free[c as i32];
        if p != 0 {
            self.tok_free[c as i32] = self.toks[p + 1];
            self.toks[p] = c as i32;
            p
        } else {
            self.tok_try_bump(c, n)
        }
    }

    #[cold]
    #[inline(never)]
    fn tok_try_alloc_large(&mut self, n: i32) -> i32 {
        let c = class_for_large(n);
        let p = self.tok_free[c as i32];
        if p == 0 {
            return self.tok_try_bump(c, n);
        }
        let was = self.web_free_len_if_fine(p, c);
        self.tok_free[c as i32] = self.toks[p + 1];
        self.toks[p] = c as i32;
        if !self.web_large_reused(was, n, false) {
            self.toks[p + 1] = self.tok_free[c as i32];
            self.tok_free[c as i32] = p;
            return 0;
        }
        p
    }

    /// `tok_bump`, or 0.  Out of line and cold, as what each capture loop would otherwise carry in the middle of a macro call: a free list serves nearly every argument.
    #[cold]
    #[inline(never)]
    fn tok_try_bump(&mut self, c: usize, n: i32) -> i32 {
        let p = self.toks_top;
        let cells = CLASS_CAP[c] + 2;
        if cells as i64 + p as i64 > self.v_webmem.toks_watch as i64 {
            return self.web_try_bump(c, n);
        }
        self.toks_top = p + cells;
        self.toks[p] = c as i32;
        p
    }

    /// A new block as far as the true end of the bump space, or 0.
    pub(crate) fn tok_try_bump_real(&mut self, c: usize) -> i32 {
        let p = self.toks_top;
        let cells = CLASS_CAP[c] + 2;
        if cells as i64 + p as i64 > self.toks_limit() {
            return self.tok_try_reclaim(c);
        }
        self.toks_top = p + cells;
        self.toks[p] = c as i32;
        p
    }

    /// Returns a block to its free list, regardless of `rc`.
    #[inline(always)]
    fn tok_release(&mut self, p: i32) {
        #[cfg(feature = "checked")]
        assert!(p >= NULL_LIST && p < self.toks_top, "tok_release of a bad block {}", p);
        let c = self.toks[p] & 0xFF;
        self.toks[p + 1] = self.tok_free[c];
        self.tok_free[c] = p;
    }

    /// §141, for token lists: frees a whole list in O(1).  The list may be null.  Lists in `mem`
    /// (hyphenation lists, LR stacks, PDF object lists) use `mem_flush_list`.
    #[inline(always)]
    pub fn flush_list(&mut self, p: i32) {
        if p != NULL {
            self.tok_release(p);
        }
    }

    /// A macro's argument at the end of its level: `capture.rs` may have handed the block on to another macro.  The null list is allowed.
    #[inline(always)]
    pub fn arg_release(&mut self, p: i32) {
        if p != NULL {
            let h = self.toks[p];
            if h & 0x0FFF_FF00 == 0 {
                self.tok_release(p);
            } else {
                self.toks[p] = h - 256;
            }
        }
    }

    /// §218 `token_ref_count(p)`: references minus one.
    #[inline(always)]
    pub fn token_ref_count(&self, p: i32) -> i32 {
        self.toks[p] >> 8
    }

    /// §218 `token_ref_count(p):=r`.
    #[inline(always)]
    pub fn set_token_ref_count(&mut self, p: i32, r: i32) {
        self.toks[p] = (r << 8) | (self.toks[p] & 0xFF);
    }

    /// §221 `add_token_ref(p)`.  `p` must not be null, as in WEB.
    #[inline(always)]
    pub fn add_token_ref(&mut self, p: i32) {
        #[cfg(feature = "checked")]
        assert!(p != NULL, "add_token_ref of the null list");
        self.toks[p] = self.toks[p].wrapping_add(256);
    }

    /// §218: `p` points to the reference count of a token list that is losing one reference.
    /// `p` must not be null, as in WEB.
    #[inline(always)]
    pub fn delete_token_ref(&mut self, p: i32) {
        #[cfg(feature = "checked")]
        assert!(p != NULL, "delete_token_ref of the null list");
        if self.toks[p] < 256 {
            self.v_webmem.heads -= 1;
            self.tok_release(p);
        } else {
            self.toks[p] -= 256;
        }
    }

    /// `flush_list(def_ref)`: the whole of a list with a reference count, which WEB's walk frees with the rest.  The list may have been made for this and never shared, and it is never null.
    #[inline]
    pub fn flush_ref(&mut self, p: i32) {
        self.v_webmem.heads -= 1;
        self.tok_release(p);
    }

    /// `free_avail(def_ref)`: a list goes on without its reference count, where the block keeps its header.
    #[inline]
    pub fn drop_ref(&mut self) {
        self.v_webmem.heads -= 1;
    }

    // -----------------------------------------------------------------------
    // The builder stack.
    // -----------------------------------------------------------------------

    /// Starts a new list on the builder: `let m = self.tb_mark();`
    #[inline(always)]
    pub fn tb_mark(&self) -> i32 {
        self.tb_ptr
    }

    /// `store_new_token(t)`, `fast_store_new_token(t)`.
    #[inline(always)]
    pub fn tb_push(&mut self, t: i32) {
        #[cfg(feature = "checked")]
        assert!(t != 0, "the token 0 would terminate the list");
        let i = self.tb_ptr;
        if i >= self.tb_limit {
            return self.tb_push_past_limit(t);
        }
        self.tb[i] = t;
        self.tb_ptr = i + 1;
    }

    /// `tb_push` has come to `tb_limit`.  It does the rest itself, so that nothing in a loop around `tb_push` has to last over a call.
    #[cold]
    #[inline(never)]
    fn tb_push_past_limit(&mut self, t: i32) {
        if !self.web_room(1) {
            self.tb_overflow();
        }
        let i = self.tb_ptr;
        self.tb[i] = t;
        self.tb_ptr = i + 1;
    }

    #[cold]
    #[inline(never)]
    pub fn tb_overflow(&mut self) -> ! {
        self.runaway(); // where the oracle's tokens fill main memory, `webmem.rs` has brought `tb_limit` to this token
        self.overflow(b"main memory size", self.main_memory_size())
    }

    /// The number of tokens pushed since mark `m`.
    #[inline(always)]
    pub fn tb_len(&self, m: i32) -> i32 {
        self.tb_ptr - m
    }

    /// Allocates a block and copies `tb[a..b]` into it, terminated, with `rc = 0`.
    fn tb_copy_out(&mut self, a: i32, b: i32) -> i32 {
        let n = b - a;
        let p = self.tok_alloc_moved(n);
        self.toks.copy_cells_from(p + 1, &self.tb, a, n);
        self.toks[p + 1 + n] = 0;
        p
    }

    /// Appends `toks[a..b]`, in place of `tb_push` on each: what a capture loop has read.
    #[inline(always)]
    pub fn tb_push_range(&mut self, a: i32, b: i32) {
        let n = b - a;
        if n > 0 {
            let o = self.tb_ptr;
            if o + n > self.tb_limit && !self.web_room(n) {
                // As many as there are cells for, one at a time as WEB's loop has it.
                let room = self.tb_limit - o;
                self.tb.copy_cells_from(o, &self.toks, a, room);
                self.tb_ptr = o + room;
                self.tb_overflow();
            }
            self.tb.copy_cells_from(o, &self.toks, a, n);
            self.tb_ptr = o + n;
        }
    }

    /// `tb_push_range` by a caller that has seen to the room.
    #[inline(always)]
    pub fn tb_put_range(&mut self, a: i32, b: i32) {
        let n = b - a;
        if n > 0 {
            let o = self.tb_ptr;
            self.tb.copy_cells_from(o, &self.toks, a, n);
            self.tb_ptr = o + n;
        }
    }

    /// A block holding `toks[a..b]`, as `tb_commit` of those tokens would make it without the builder; none gives null, and so does a full heap,
    /// which only a loop in capture.rs with tokens not on the builder asks about: it then goes back to the literal loop, which has them on it.
    #[inline(always)]
    pub fn tok_from_range(&mut self, a: i32, b: i32) -> i32 {
        let n = b - a;
        if n <= 0 {
            return NULL;
        }
        let p = self.tok_try_alloc(n);
        if p != 0 {
            self.toks.copy_cells_within(a, p + 1, n);
            self.toks[p + 1 + n] = 0;
        }
        p
    }

    /// Whether a control sequence but the frozen ones may have `eq_type` from `outer_call` to `dont_expand`.  See [`ANY_USER_OUTER`].
    #[inline(always)]
    pub fn any_user_outer(&self) -> bool {
        self.toks[ANY_USER_OUTER] != 0
    }

    /// `RSTEX_NO_SHARE`: the loops then look at each control sequence's meaning, and hand no argument on, which is what they do once a document has an `\\outer` macro.
    pub fn arg_share_off(&mut self) {
        self.toks[ANY_USER_OUTER] = 1;
    }

    /// §297, §301: control sequence definitions come through here.
    #[inline(always)]
    pub fn note_outer_meaning(&mut self, t: i32) {
        if t >= OUTER_CALL && t <= DONT_EXPAND {
            self.toks[ANY_USER_OUTER] = 1;
        }
    }

    /// The list since mark `m` becomes a block with `rc = 0`, and is popped.  An empty list gives
    /// the null list 0, like `link(temp_head)=null`.  Use this for arguments, backed-up lists and
    /// templates; use [`Tex::tb_commit_ref`] where WEB has a reference-counted `def_ref`.
    #[inline]
    pub fn tb_commit(&mut self, m: i32) -> i32 {
        let top = self.tb_ptr;
        let p = if top == m { NULL } else { self.tb_copy_out(m, top) };
        self.tb_ptr = m;
        p
    }

    /// Like [`Tex::tb_commit`], but an empty list still gets a block, like WEB's `def_ref`, which
    /// always has a reference count node.  `scan_toks` and `read_toks` use this, so `def_ref` is
    /// never null.
    #[inline]
    pub fn tb_commit_ref(&mut self, m: i32) -> i32 {
        let top = self.tb_ptr;
        let p = self.tb_copy_out(m, top);
        self.tb_ptr = m;
        p
    }

    /// §426: the list since `m` is a single group `{...}`; commit it without its enclosing braces
    /// and pop it.  Truncate and advance; no mid-list mutation.
    #[inline]
    pub fn tb_commit_strip(&mut self, m: i32) -> i32 {
        let top = self.tb_ptr;
        let p = if top - m <= 2 { NULL } else { self.tb_copy_out(m + 1, top - 1) };
        self.tb_ptr = m;
        p
    }

    /// Abandons the list since mark `m`.
    #[inline(always)]
    pub fn tb_discard(&mut self, m: i32) {
        self.tb_ptr = m;
    }

    /// `show_token_list` of the list since `m`, leaving the builder alone: what `runaway` displays through `tb_def_mark`, `tb_match_mark` and
    /// `tb_align_mark`.  The tokens go just above the heap for the moment and take no block, since a full heap is one reason to be here.
    pub fn tb_show(&mut self, m: i32, l: i32) {
        let top = self.toks_top;
        let n = (self.tb_ptr - m).min(TOKS_CAP as i32 - top - 1);
        self.toks.copy_cells_from(top, &self.tb, m, n);
        self.toks[top + n] = 0;
        self.toks_top = top + n + 1;
        self.show_token_list(top, NULL, l);
        self.toks_top = top;
        // What lies above the top is 0, as in a run restored from a record, which holds the heap as far as it is live.
        self.toks.as_mut_slice()[top as usize..(top + n + 1) as usize].fill(0);
    }

    /// Appends the tokens of list `p`, which may be null, to the builder: §492's copy loop.
    pub fn tb_push_list(&mut self, p: i32) {
        let n = self.tok_len(p);
        self.tb_push_range(p + 1, p + 1 + n);
    }

    // -----------------------------------------------------------------------
    // The transient stack.
    // -----------------------------------------------------------------------

    /// Starts a transient list: `\csname` §398, `\ifcsname` §1767 and `scan_keyword` §433.
    ///
    /// These three call `get_x_token` with tokens in flight, and those tokens must stay out of
    /// `runaway`, which shows `tb[mark..tb_ptr]`: in WEB they are on a separate list of their own, not
    /// on `def_ref`, `temp_head` or `hold_head`.  With `\edef\a{x\csname y` and the end of the file,
    /// "Runaway definition" shows `->x` and not `->xy`.  Hence a stack of their own.
    #[inline(always)]
    pub fn tx_mark(&self) -> i32 {
        self.tx_ptr
    }

    /// `store_new_token(t)` for a transient list.
    #[inline(always)]
    pub fn tx_push(&mut self, t: i32) {
        let i = self.tx_ptr;
        if i >= self.tx_limit {
            return self.tx_push_past_limit(t);
        }
        self.tx[i] = t;
        self.tx_ptr = i + 1;
    }

    /// `tx_push` has come to `tx_limit`.
    #[cold]
    #[inline(never)]
    fn tx_push_past_limit(&mut self, t: i32) {
        if !self.web_tx_room() {
            self.tx_overflow();
        }
        let i = self.tx_ptr;
        self.tx[i] = t;
        self.tx_ptr = i + 1;
    }

    #[cold]
    #[inline(never)]
    pub fn tx_overflow(&mut self) -> ! {
        self.runaway();
        self.overflow(b"main memory size", self.main_memory_size())
    }

    /// The number of tokens pushed since mark `m`.
    #[inline(always)]
    pub fn tx_len(&self, m: i32) -> i32 {
        self.tx_ptr - m
    }

    /// The transient list since `m` becomes a block with `rc = 0`, and is popped; empty gives null.
    pub fn tx_commit(&mut self, m: i32) -> i32 {
        let top = self.tx_ptr;
        let p = if top == m {
            NULL
        } else {
            let n = top - m;
            let q = self.tok_alloc_moved(n);
            let dst = q as usize + 1;
            let (a, b) = (m as usize, top as usize);
            self.toks.as_mut_slice()[dst..dst + (b - a)].copy_from_slice(&self.tx.as_slice()[a..b]);
            self.toks[q + 1 + n] = 0;
            q
        };
        self.tx_ptr = m;
        p
    }

    /// Abandons the transient list since mark `m`.
    #[inline(always)]
    pub fn tx_discard(&mut self, m: i32) {
        self.tx_ptr = m;
    }

    // -----------------------------------------------------------------------
    // Small list operations.
    // -----------------------------------------------------------------------

    /// A one-token list with `rc = 0`: `p:=get_avail; info(p):=t` for a token.
    #[inline(always)]
    pub fn tok_one(&mut self, t: i32) -> i32 {
        let p = self.tok_alloc_class(1);
        self.toks[p + 1] = t;
        self.toks[p + 2] = 0;
        p
    }

    /// A two-token list with `rc = 0`.
    #[inline]
    pub fn tok_two(&mut self, t0: i32, t1: i32) -> i32 {
        let p = self.tok_alloc_class(2);
        self.toks[p + 1] = t0;
        self.toks[p + 2] = t1;
        self.toks[p + 3] = 0;
        p
    }

    /// The number of tokens in list `p`, which may be null.
    pub fn tok_len(&self, p: i32) -> i32 {
        let mut i = p + 1;
        while self.toks[i] != 0 {
            i += 1;
        }
        i - (p + 1)
    }

    /// `link(p)=null` for a header `p`, or `p=null`: the list has no tokens.
    #[inline(always)]
    pub fn tok_is_empty(&self, p: i32) -> bool {
        self.toks[p + 1] == 0
    }

    /// A fresh copy of list `p` with `rc = 0`; null gives null.  §970's copy of a template.
    pub fn tok_copy(&mut self, p: i32) -> i32 {
        if p == NULL {
            return NULL;
        }
        let n = self.tok_len(p);
        let q = self.tok_alloc(n);
        self.toks.copy_within(p + 1, q + 1, n + 1);
        q
    }

    /// A new list `[t, tokens of p...]` that replaces `p`, freeing it, with `rc = 0`.
    /// §1396's protected_token, §1404's left brace.
    pub fn tok_prepend(&mut self, p: i32, t: i32) -> i32 {
        let n = self.tok_len(p);
        let q = self.tok_alloc_moved(n + 1);
        self.toks[q + 1] = t;
        self.toks.copy_within(p + 1, q + 2, n + 1);
        self.flush_list(p);
        q
    }

    /// A new list `[tokens of p..., t]` that replaces `p`, freeing it, with `rc = 0`.
    /// §1404's right brace.
    pub fn tok_append(&mut self, p: i32, t: i32) -> i32 {
        let n = self.tok_len(p);
        let q = self.tok_alloc_moved(n + 1);
        self.toks.copy_within(p + 1, q + 1, n);
        self.toks[q + 1 + n] = t;
        self.toks[q + 2 + n] = 0;
        self.flush_list(p);
        q
    }

    /// §1577 `concat_tokens`: `q` then `r`.  Both are consumed; the result has `rc = 0`.
    pub fn tok_concat(&mut self, q: i32, r: i32) -> i32 {
        if q == NULL {
            return r;
        }
        self.drop_ref(); // §1577's free_avail(r)
        let nq = self.tok_len(q);
        let nr = self.tok_len(r);
        let p = self.tok_alloc_moved(nq + nr);
        self.toks.copy_within(q + 1, p + 1, nq);
        self.toks.copy_within(r + 1, p + 1 + nq, nr + 1);
        self.flush_list(q);
        self.flush_list(r);
        p
    }

    /// §348, §393, §394: `p:=get_avail; info(p):=t; link(p):=loc; start:=p; loc:=p`.  The current
    /// input level is a backed-up list read up to `loc`; make `t` its next token.  The result is
    /// one fresh block holding `t` and the unread rest, which replaces the old block.
    pub fn back_prepend(&mut self, t: i32) {
        let old = self.cur_input.start_field;
        let l = self.cur_input.loc_field;
        let mut e = l;
        while self.toks[e] != 0 {
            e += 1;
        }
        let n = e - l;
        let q = self.tok_alloc_moved(n + 1);
        self.toks[q + 1] = t;
        self.toks.copy_within(l, q + 2, n + 1);
        self.flush_list(old);
        self.cur_input.start_field = q;
        self.cur_input.loc_field = q + 1;
    }

    /// `loc=null` of WEB's token-list state: the current level is a token list that is exhausted.
    /// Unlike WEB's test it must include the state, because `loc` in a file level is a buffer index
    /// and would read an unrelated cell of `toks`.
    #[inline(always)]
    pub fn at_list_end(&self) -> bool {
        self.cur_input.state_field == TOKEN_LIST && self.toks[self.cur_input.loc_field] == 0
    }

    /// `at_list_end` of a saved input level: `(state_field=token_list)and(loc_field=null)`.  §1309
    /// `do_endv`, the one site that tests these fields on `input_stack`: there `loc` is at the
    /// terminator, not 0, so a literal `loc_field == NULL` is false at the end of every v-template.
    #[inline]
    pub fn rec_at_list_end(&self, r: InStateRecord) -> bool {
        r.state_field == TOKEN_LIST && self.toks[r.loc_field] == 0
    }

    // -----------------------------------------------------------------------
    // Part 20.
    // -----------------------------------------------------------------------

    /// §314
    /// p and q are toks indices of tokens, not headers: pass h+1 for a list h; q=0 for none.
    pub fn show_token_list(&mut self, p: i32, q: i32, l: i32) {
        let mut p = p;
        let mut match_chr = b'#' as i32;
        let mut n = b'0' as i32;
        self.tally = 0;
        while self.toks_show_more(p) && self.tally < l {
            if p == q {
                // §342
                self.set_trick_count();
            }
            // §315: the bound is toks_top, since token lists are not in mem.
            if p < 0 || p >= self.toks_top {
                self.print_esc_s(b"CLOBBERED.");
                return;
            }
            let t = self.toks[p];
            if t >= CS_TOKEN_FLAG {
                self.print_cs(t - CS_TOKEN_FLAG);
            } else {
                let m = t / 256;
                let c = t % 256;
                if t < 0 {
                    self.print_esc_s(b"BAD.");
                } else {
                    // §316
                    match m {
                        LEFT_BRACE | RIGHT_BRACE | MATH_SHIFT | TAB_MARK | SUP_MARK | SUB_MARK | SPACER | LETTER
                        | OTHER_CHAR => self.print(c),
                        MAC_PARAM => {
                            self.print(c);
                            self.print(c);
                        }
                        OUT_PARAM => {
                            self.print(match_chr);
                            if c <= 9 {
                                self.print_char(c + b'0' as i32);
                            } else {
                                self.print_char(b'!' as i32);
                                return;
                            }
                        }
                        MATCH => {
                            match_chr = c;
                            self.print(c);
                            n += 1;
                            self.print_char(n);
                            if n > b'9' as i32 {
                                return;
                            }
                        }
                        END_MATCH => {
                            if c == 0 {
                                self.print_s(b"->");
                            }
                        }
                        _ => self.print_esc_s(b"BAD."),
                    }
                }
            }
            p += 1;
        }
        if self.toks_show_more(p) {
            self.print_esc_s(b"ETC.");
        }
    }

    /// §314's `p<>null`.  WEB is robust against a wild `p`, and `toks` is unchecked, so a position
    /// outside the heap counts as not null without being read: §315 then reports it.
    fn toks_show_more(&self, p: i32) -> bool {
        p < 0 || p >= self.toks_top || self.toks[p] != 0
    }

    /// §317
    /// p is a list header in toks, or 0.
    pub fn token_show(&mut self, p: i32) {
        if p != NULL {
            self.show_token_list(p + 1, NULL, 10000000);
        }
    }

    /// §318
    /// cur_chr of a macro is a header in toks.
    pub fn print_meaning(&mut self) {
        self.print_cmd_chr(self.cur_cmd, self.cur_chr);
        if self.cur_cmd >= CALL {
            self.print_char(b':' as i32);
            self.print_ln();
            self.token_show(self.cur_chr);
        } else if self.cur_cmd == TOP_BOT_MARK && self.cur_chr < MARKS_CODE {
            self.print_char(b':' as i32);
            self.print_ln();
            self.token_show(self.cur_mark[self.cur_chr]);
        }
    }
}

/// A minimal machine for the unit tests of this module, `input.rs`, `getnext.rs`, `expand.rs` and
/// `macrocall.rs`, which run with most of the engine still stubs.
#[cfg(test)]
pub(crate) mod testkit {
    use super::*;

    /// The state the token machine reads, set as `initialize` and INITEX would, with plain's category
    /// codes.  The bottom input level is an exhausted `\read` line: `get_next` then gives
    /// `cur_cmd=cur_chr=cur_cs=0`, without printing anything, so `cur_tok=0` means the input is over.
    pub fn machine() -> &'static mut Tex {
        let t = Tex::alloc();
        t.toks_init();
        t.stack_size = 5000;
        t.param_size = 5000;
        t.buf_size = 100000;
        t.pool_size = 100000;
        t.max_in_open = 15;
        t.expand_depth = 10000;
        t.align_state = 1000000;
        t.no_new_control_sequence = true;
        t.OK_to_interrupt = true;
        t.deletions_allowed = true;
        t.first = 1;
        t.last = 1;
        t.par_loc = cs(b'P');
        t.par_token = CS_TOKEN_FLAG + t.par_loc;
        for k in 0..256 {
            t.xord[k] = k as u8;
            t.xchr[k] = k as u8;
            t.set_cat_code(k, OTHER_CHAR);
        }
        for k in (b'a'..=b'z').chain(b'A'..=b'Z') {
            t.set_cat_code(k as i32, LETTER);
        }
        for (c, v) in [
            (b'\\', ESCAPE), (b'%', COMMENT), (127, INVALID_CHAR), (0, IGNORE), (13, CAR_RET), (b' ', SPACER),
            (b'{', LEFT_BRACE), (b'}', RIGHT_BRACE), (b'$', MATH_SHIFT), (b'&', TAB_MARK), (b'#', MAC_PARAM),
            (b'^', SUP_MARK), (b'_', SUB_MARK), (b'~', ACTIVE_CHAR),
        ] {
            t.set_cat_code(c as i32, v);
        }
        t.set_end_line_char(-1);
        t.set_state(NEW_LINE);
        t.set_start(1);
        t.set_loc(1);
        t.set_limit(0);
        t.set_name(1);
        t.set_index(0);
        t
    }

    /// The control sequence named by one character.
    pub fn cs(c: u8) -> i32 {
        SINGLE_BASE + c as i32
    }

    /// The token of that control sequence.
    pub fn cs_tok(c: u8) -> i32 {
        CS_TOKEN_FLAG + cs(c)
    }

    /// `\def` by hand: `params`, the end of match, and `body`.  Returns the list's header.
    pub fn def_macro(t: &mut Tex, c: i32, cmd: i32, params: &[i32], body: &[i32]) -> i32 {
        let m = t.tb_mark();
        for &x in params {
            t.tb_push(x);
        }
        t.tb_push(END_MATCH_TOKEN);
        for &x in body {
            t.tb_push(x);
        }
        let p = t.tb_commit_ref(m);
        t.set_eq_type(c, cmd);
        t.set_equiv(c, p);
        p
    }

    /// A list of these tokens becomes the current input, as `ins_list`.
    pub fn feed(t: &mut Tex, toks: &[i32]) {
        let m = t.tb_mark();
        for &x in toks {
            t.tb_push(x);
        }
        let p = t.tb_commit(m);
        t.ins_list(p);
    }

    /// Makes `s` the current line of the bottom level, in state `state`.
    pub fn set_line(t: &mut Tex, s: &[u8], state: i32) {
        let start = t.first;
        for (i, &c) in s.iter().enumerate() {
            t.buffer.put(start + i as i32, c as i32);
        }
        t.set_start(start);
        t.set_loc(start);
        t.set_limit(start + s.len() as i32 - 1);
        t.set_state(state);
        t.first = start + s.len() as i32;
    }

    /// `get_token` until the input is over, or up to and excluding the token `stop`.
    pub fn drain(t: &mut Tex, stop: i32) -> Vec<i32> {
        let mut v = Vec::new();
        loop {
            t.get_token();
            if t.cur_tok == 0 || t.cur_tok == stop {
                return v;
            }
            v.push(t.cur_tok);
        }
    }

    /// The tokens of a list.
    pub fn list(t: &Tex, p: i32) -> Vec<i32> {
        let mut v = Vec::new();
        let mut i = p + 1;
        while t.toks[i] != 0 {
            v.push(t.toks[i]);
            i += 1;
        }
        v
    }

    fn show_char(out: &mut Vec<u8>, c: i32) {
        if c < 32 || c == 127 {
            out.extend_from_slice(b"^^");
            out.push(if c < 64 { c + 64 } else { c - 64 } as u8);
        } else {
            out.push(c as u8);
        }
    }

    /// What `show_token_list` prints for these tokens, without using the print routines.
    /// A control sequence above `HASH_BASE` needs its name in `str_pool`.
    pub fn show(t: &Tex, toks: &[i32]) -> Vec<u8> {
        let mut out = Vec::new();
        for &x in toks {
            if x >= CS_TOKEN_FLAG {
                let p = x - CS_TOKEN_FLAG;
                if p == t.par_loc {
                    out.extend_from_slice(b"\\par ");
                } else if p < SINGLE_BASE {
                    show_char(&mut out, p - ACTIVE_BASE);
                } else if p < NULL_CS {
                    out.push(b'\\');
                    show_char(&mut out, p - SINGLE_BASE);
                    if t.cat_code(p - SINGLE_BASE) == LETTER {
                        out.push(b' ');
                    }
                } else {
                    out.push(b'\\');
                    let s = t.text(p);
                    for k in t.str_start[s]..t.str_start[s + 1] {
                        show_char(&mut out, t.str_pool.at(k));
                    }
                    out.push(b' ');
                }
            } else {
                let (m, c) = (x / 256, x % 256);
                show_char(&mut out, c);
                if m == MAC_PARAM {
                    show_char(&mut out, c);
                }
            }
        }
        out
    }

    /// For the tests that print: what printing needs.
    /// Output is left in `host.term_buf`; see [`printed`].
    pub fn print_setup(t: &mut Tex) {
        t.selector = TERM_ONLY;
        t.term_offset = 0;
        t.file_offset = 0;
        t.max_print_line = 79;
        t.error_line = 79;
        t.half_error_line = 50;
        t.max_strings = 100000;
        t.eqtb_top = EQTB_CAP as i32 - 1;
        t.interaction = SCROLL_MODE;
        t.set_escape_char(b'\\' as i32);
        t.set_new_line_char(-1);
        t.set_error_context_lines(1000000);
        for k in 0..256 {
            t.xprn[k] = (k >= 32 && k <= 126) as u8;
        }
        assert!(t.get_strings_started());
    }

    /// Gives a multi-letter control sequence a meaning.  Needs hash.rs.
    pub fn define(t: &mut Tex, name: &[u8], cmd: i32, chr: i32) -> i32 {
        for (i, &c) in name.iter().enumerate() {
            t.buffer.put(t.first + i as i32, c as i32);
        }
        t.no_new_control_sequence = false;
        let p = t.id_lookup(t.first, name.len() as i32);
        t.no_new_control_sequence = true;
        t.set_eq_type(p, cmd);
        t.set_equiv(p, chr);
        p
    }

    /// Takes what was printed since the last call, and starts a new line.
    pub fn printed(t: &mut Tex) -> String {
        t.term_offset = 0;
        let b = core::mem::take(&mut t.host.term_buf);
        String::from_utf8_lossy(&b).into_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn new_tex() -> &'static mut Tex {
        let t = Tex::alloc();
        t.toks_init();
        t
    }

    fn list(t: &Tex, p: i32) -> Vec<i32> {
        let mut v = Vec::new();
        let mut i = p + 1;
        while t.toks[i] != 0 {
            v.push(t.toks[i]);
            i += 1;
        }
        v
    }

    fn commit(t: &mut Tex, toks: &[i32]) -> i32 {
        let m = t.tb_mark();
        for &x in toks {
            t.tb_push(x);
        }
        t.tb_commit(m)
    }

    #[test]
    fn size_of_tex_is_under_2_gib() {
        // tests/foundation.rs prints the numbers.
        assert!(core::mem::offset_of!(Tex, host) < 1 << 31);
    }

    #[test]
    fn class_table_is_sane() {
        for c in 0..EXACT_CLASSES {
            assert_eq!(CLASS_CAP[c], c as i32);
        }
        for c in EXACT_CLASSES..NUM_CLASSES {
            assert!(CLASS_CAP[c] > CLASS_CAP[c - 1] || CLASS_CAP[c] == TOKS_CAP as i32);
        }
        assert_eq!(*CLASS_CAP.last().unwrap(), TOKS_CAP as i32, "the top class must reach the heap size");
        for n in [0, 1, 2, 63, 64, 65, 72, 73, 100, 1000, 100_000, 10_000_000] {
            let c = class_for(n);
            assert!(CLASS_CAP[c] >= n, "class {} too small for {}", c, n);
            if c > 0 {
                assert!(CLASS_CAP[c - 1] < n, "class {} not minimal for {}", c, n);
            }
        }
    }

    #[test]
    fn reserved_lists() {
        let t = new_tex();
        assert_eq!(t.toks[0], 0);
        assert_eq!(t.toks[1], 0);
        assert!(t.tok_is_empty(NULL));
        assert!(t.tok_is_empty(NULL_LIST));
        assert_eq!(list(t, OMIT_TEMPLATE), vec![END_TEMPLATE_TOKEN]);
        assert_eq!(t.token_ref_count(NULL_LIST), 0);
        // \let\x=\endtemplate increments the rc of null_list.
        t.add_token_ref(NULL_LIST);
        assert_eq!(t.token_ref_count(NULL_LIST), 1);
        assert!(t.toks_top >= TOKS_FIRST_FREE);
        Tex::dealloc(t);
    }

    /// `glue_ref` and above come through `eq_define` as well, for registers, and must not set it.
    #[test]
    fn any_user_outer_is_sticky_and_precise() {
        let t = new_tex();
        for cmd in [CALL, LONG_CALL, RELAX, UNDEFINED_CS, GLUE_REF, SHAPE_REF, BOX_REF, DATA] {
            t.note_outer_meaning(cmd);
            assert!(!t.any_user_outer(), "cmd {}", cmd);
        }
        for cmd in [OUTER_CALL, LONG_OUTER_CALL, END_TEMPLATE, DONT_EXPAND] {
            let t = new_tex();
            t.note_outer_meaning(cmd);
            assert!(t.any_user_outer(), "cmd {}", cmd);
            t.note_outer_meaning(CALL);
            assert!(t.any_user_outer(), "it stays");
            Tex::dealloc(t);
        }
        Tex::dealloc(t);
    }

    #[test]
    fn header_encoding() {
        let t = new_tex();
        let p = commit(t, &[300, 301, 302]);
        assert_eq!(t.toks[p] & 0xFF, 3, "exact class");
        assert_eq!(t.token_ref_count(p), 0);
        t.add_token_ref(p);
        t.add_token_ref(p);
        assert_eq!(t.token_ref_count(p), 2);
        assert_eq!(t.toks[p] & 0xFF, 3, "rc does not disturb the class");
        t.set_token_ref_count(p, 5);
        assert_eq!(t.token_ref_count(p), 5);
        assert_eq!(t.toks[p] & 0xFF, 3);
        Tex::dealloc(t);
    }

    #[test]
    fn refcount_frees_at_null() {
        let t = new_tex();
        let p = commit(t, &[300, 301]);
        t.add_token_ref(p); // two references
        t.delete_token_ref(p); // one left
        assert_eq!(list(t, p), vec![300, 301], "still alive");
        t.delete_token_ref(p); // freed
        // the same class reuses the block
        let q = commit(t, &[400, 401]);
        assert_eq!(q, p);
        assert_eq!(list(t, q), vec![400, 401]);
        assert_eq!(t.token_ref_count(q), 0, "a reused block starts at rc 0");
        Tex::dealloc(t);
    }

    #[test]
    fn flush_list_ignores_rc_and_null() {
        let t = new_tex();
        t.flush_list(NULL);
        let p = commit(t, &[300]);
        t.add_token_ref(p);
        t.flush_list(p);
        let q = commit(t, &[500]);
        assert_eq!(q, p);
        Tex::dealloc(t);
    }

    #[test]
    fn commit_variants() {
        let t = new_tex();
        // An empty list commits to null, like link(temp_head)=null.
        let m = t.tb_mark();
        assert_eq!(t.tb_commit(m), NULL);
        assert_eq!(t.tb_ptr, m);
        // But tb_commit_ref always allocates, like def_ref.
        let m = t.tb_mark();
        let d = t.tb_commit_ref(m);
        assert_ne!(d, NULL);
        assert!(t.tok_is_empty(d));
        t.add_token_ref(d);
        t.delete_token_ref(d);
        t.delete_token_ref(d);
        // Reading null as a list is end of list.
        assert!(t.tok_is_empty(NULL));
        assert_eq!(t.tok_len(NULL), 0);
        Tex::dealloc(t);
    }

    #[test]
    fn builder_nests_lifo() {
        let t = new_tex();
        let outer = t.tb_mark();
        t.tb_push(1000);
        t.tb_push(1001);
        // Nested expansion builds and commits while the outer list is in flight.
        let inner = t.tb_mark();
        assert_eq!(inner, outer + 2);
        t.tb_push(2000);
        let p = t.tb_commit(inner);
        assert_eq!(list(t, p), vec![2000]);
        // A nested discard, as in scan_keyword's success path.
        let inner = t.tb_mark();
        t.tb_push(3000);
        t.tb_discard(inner);
        assert_eq!(t.tb_len(outer), 2);
        t.tb_push(1002);
        let q = t.tb_commit(outer);
        assert_eq!(list(t, q), vec![1000, 1001, 1002]);
        assert_eq!(t.tb_ptr, 0);
        Tex::dealloc(t);
    }

    #[test]
    fn tb_push_list_is_the_copy_loop() {
        let t = new_tex();
        let a = commit(t, &[10, 11, 12]);
        let m = t.tb_mark();
        t.tb_push(9);
        t.tb_push_list(a);
        t.tb_push_list(NULL);
        t.tb_push(13);
        let b = t.tb_commit(m);
        assert_eq!(list(t, b), vec![9, 10, 11, 12, 13]);
        Tex::dealloc(t);
    }

    /// A machine that can print, with its heap's bump space used up but for `room` cells.
    fn nearly_full(room: i32) -> &'static mut Tex {
        let t = testkit::machine();
        testkit::print_setup(t);
        // One block fills what there is, and it stays in use.
        let want = TOKS_LIMIT as i32 - t.toks_top - room;
        let c = (0..NUM_CLASSES).rev().find(|&c| CLASS_CAP[c] + 2 <= want).unwrap();
        let p = t.tok_alloc_class(c);
        t.toks[p + 1] = 0;
        // And the rest in the largest blocks that fit, in use too.
        while TOKS_LIMIT as i32 - t.toks_top - room >= 2 {
            let n = TOKS_LIMIT as i32 - t.toks_top - room;
            let c = (0..NUM_CLASSES).rev().find(|&c| CLASS_CAP[c] + 2 <= n).unwrap();
            let p = t.tok_alloc_class(c);
            t.toks[p + 1] = 0;
        }
        t
    }

    #[test]
    fn runaway_display_takes_no_block() {
        let t = testkit::machine();
        testkit::print_setup(t);
        let m = t.tb_mark();
        t.tb_push(LETTER_TOKEN + 97);
        t.tb_push(LETTER_TOKEN + 98);
        let (top, free) = (t.toks_top, t.tok_free);
        t.tb_show(m, 100);
        assert_eq!(testkit::printed(t), "ab");
        assert_eq!(t.tb_len(m), 2, "the builder is untouched");
        assert_eq!((t.toks_top, t.tok_free.as_slice()), (top, free.as_slice()), "nor is the heap");
        t.tb_discard(m);
        let m = t.tb_mark();
        t.tb_show(m, 100);
        assert_eq!(testkit::printed(t), "");
        Tex::dealloc(t);
    }

    /// The display has room when no block can be had, and it says `\ETC.` as it would.
    #[test]
    fn runaway_display_on_a_full_heap() {
        let t = nearly_full(0);
        assert_eq!(t.toks_top as usize, TOKS_LIMIT);
        let m = t.tb_mark();
        for _ in 0..1000 {
            t.tb_push(LETTER_TOKEN + 97);
        }
        t.tb_show(m, 69);
        let s = testkit::printed(t);
        assert_eq!(s, format!("{}\\ETC.", "a".repeat(69)));
        assert_eq!(t.toks_top as usize, TOKS_LIMIT);
        t.tb_discard(m);
        Tex::dealloc(t);
    }

    /// A full heap in the middle of an argument: the message once, with its text.  Showing the text took a block, which took the message again, for ever.
    #[test]
    fn a_full_heap_shows_the_runaway_once() {
        let t = nearly_full(0);
        t.scanner_status = MATCHING;
        let m = t.tb_mark();
        t.tb_match_mark = m;
        for _ in 0..100 {
            t.tb_push(LETTER_TOKEN + 97);
        }
        let s = crate::save::testkit::stops(t, |t| {
            t.tb_commit(m);
        });
        let want = format!("Runaway argument?\n{}\\ETC.\n! TeX capacity exceeded, sorry [main memory size=0]", "a".repeat(69));
        assert!(s.contains(&want), "{}", s);
        assert_eq!(s.matches("Runaway").count(), 1, "{}", s);
    }

    /// Lists of one length after another: what `tests/integration/toks/heap_classes.tex` does through TeX.
    #[test]
    fn a_full_heap_of_free_blocks_serves_any_class() {
        let t = nearly_full(0);
        let top = t.toks_top;
        let mut p = TOKS_FIRST_FREE;
        let mut blocks = Vec::new();
        while p < top {
            blocks.push(p);
            p += CLASS_CAP[(t.toks[p] & 0xFF) as usize] + 2;
        }
        assert_eq!(p, top, "every cell is in a block");
        let big = *blocks.iter().max_by_key(|&&b| CLASS_CAP[(t.toks[b] & 0xFF) as usize]).unwrap();
        let big_class = (t.toks[big] & 0xFF) as usize;
        t.tok_release(big);
        // A small class has no free block, so it comes out of that one.
        let a = t.tok_alloc_class(5);
        assert!(a >= big && a < big + CLASS_CAP[big_class] + 2);
        assert_eq!(t.toks[a] & 0xFF, 5);
        assert_eq!(t.toks_top, top, "no bump");
        Tex::dealloc(t);
    }

    #[test]
    fn coalescing_joins_what_is_free_and_leaves_what_is_used() {
        let t = new_tex();
        let base = t.toks_top;
        // a b c d e, each of 10 tokens.
        let lists: Vec<i32> = (0..5).map(|k| commit(t, &[300 + k; 10])).collect();
        let cells = CLASS_CAP[10] + 2;
        assert_eq!(lists[1], base + cells);
        // Free a and b, which touch, and d and e, which reach the top.
        t.flush_list(lists[0]);
        t.flush_list(lists[1]);
        t.flush_list(lists[3]);
        t.flush_list(lists[4]);
        t.tok_coalesce();
        assert_eq!(list(t, lists[2]), vec![302; 10], "in use, and where it was");
        assert_eq!(t.toks_top, lists[2] + cells, "what was at the end is bump space again");
        assert_eq!(t.tok_free[10], 0, "what is left is not in small pieces");
        // Twice the cells, as one block of the class for 2*10+2 tokens.
        let joined = class_for(2 * cells - 2);
        assert_eq!(CLASS_CAP[joined] + 2, 2 * cells, "an exact class");
        assert_eq!(t.tok_free[joined as i32], base);
        assert_eq!(t.toks[base + 1], 0, "the only one");
        let q = commit(t, &[400; 22]);
        assert_eq!(q, base, "and it serves");
        assert_eq!(list(t, q), vec![400; 22]);
        Tex::dealloc(t);
    }

    /// Splitting leaves no block of one cell, whatever the sizes.
    #[test]
    fn a_free_run_of_any_length_is_blocks() {
        for n in (2..400).chain([1000, 12345, 1 << 20, (1 << 24) + 3]) {
            let t = new_tex();
            let base = t.toks_top;
            t.toks_top = base + n;
            t.tok_free_run(base, n);
            let mut p = base;
            let mut free = 0;
            while p < base + n {
                let c = (t.toks[p] & 0xFF) as usize;
                assert!(CLASS_CAP[c] >= 0);
                p += CLASS_CAP[c] + 2;
                free += 1;
            }
            assert_eq!(p, base + n, "{} cells", n);
            let listed: i32 = (0..NUM_CLASSES)
                .map(|c| {
                    let (mut q, mut k) = (t.tok_free[c as i32], 0);
                    while q != 0 {
                        q = t.toks[q + 1];
                        k += 1;
                    }
                    k
                })
                .sum();
            assert_eq!(listed, free, "{} cells", n);
            Tex::dealloc(t);
        }
    }

    /// One cell left over: the block stays of its class.
    #[test]
    fn a_split_with_one_cell_over_keeps_the_class() {
        let t = new_tex();
        let p = commit(t, &[300; 6]);
        t.flush_list(p);
        let q = t.tok_split(5).unwrap();
        assert_eq!(q, p);
        assert_eq!(t.toks[q] & 0xFF, 6);
        assert_eq!(t.tok_free[6], 0);
        Tex::dealloc(t);
    }

    #[test]
    fn brace_strip() {
        let t = new_tex();
        let m = t.tb_mark();
        for &x in &[LEFT_BRACE_TOKEN + 123, 50, 51, RIGHT_BRACE_TOKEN + 125] {
            t.tb_push(x);
        }
        let p = t.tb_commit_strip(m);
        assert_eq!(list(t, p), vec![50, 51]);
        assert_eq!(t.tb_ptr, m);
        // {} strips to the null list.
        let m = t.tb_mark();
        t.tb_push(LEFT_BRACE_TOKEN + 123);
        t.tb_push(RIGHT_BRACE_TOKEN + 125);
        assert_eq!(t.tb_commit_strip(m), NULL);
        Tex::dealloc(t);
    }

    #[test]
    fn transient_stack_is_separate_from_the_builder() {
        let t = new_tex();
        let b = t.tb_mark();
        t.tb_push(1000);
        let m = t.tx_mark();
        t.tx_push(2000);
        t.tx_push(2001);
        assert_eq!(t.tx_len(m), 2);
        assert_eq!(t.tb_len(b), 1, "runaway's view of the list being built is untouched");
        // A nested transient list, committed as scan_keyword does.
        let m2 = t.tx_mark();
        t.tx_push(2002);
        let q = t.tx_commit(m2);
        assert_eq!(list(t, q), vec![2002]);
        assert_eq!(t.tx_commit(t.tx_mark()), NULL, "empty gives the null list");
        assert_eq!(t.tx_len(m), 2);
        t.tx_discard(m);
        assert_eq!(t.tx_ptr, 0);
        t.tb_discard(b);
        Tex::dealloc(t);
    }

    #[test]
    fn small_lists() {
        let t = new_tex();
        let a = t.tok_one(600);
        assert_eq!(list(t, a), vec![600]);
        let b = t.tok_two(601, 602);
        assert_eq!(list(t, b), vec![601, 602]);
        assert_eq!(t.tok_len(b), 2);
        let c = t.tok_copy(b);
        assert_ne!(c, b);
        assert_eq!(list(t, c), vec![601, 602]);
        assert_eq!(t.tok_copy(NULL), NULL);
        let d = t.tok_prepend(c, 599);
        assert_eq!(list(t, d), vec![599, 601, 602]);
        let e = t.tok_append(d, 603);
        assert_eq!(list(t, e), vec![599, 601, 602, 603]);
        let f = t.tok_concat(e, a);
        assert_eq!(list(t, f), vec![599, 601, 602, 603, 600]);
        let g = t.tok_concat(NULL, b);
        assert_eq!(g, b);
        Tex::dealloc(t);
    }

    #[test]
    fn at_list_end_and_back_prepend() {
        let t = new_tex();
        t.cur_input.state_field = TOKEN_LIST;
        let p = commit(t, &[700, 701, 702]);
        t.cur_input.start_field = p;
        t.cur_input.loc_field = p + 2; // 701 is next
        assert!(!t.at_list_end());
        // \noexpand: the unread rest gets a marker in front, in one fresh block.
        t.back_prepend(900);
        let s = t.cur_input.start_field;
        assert_eq!(t.cur_input.loc_field, s + 1);
        assert_eq!(list(t, s), vec![900, 701, 702]);
        t.cur_input.loc_field = s + 4; // the terminator
        assert!(t.at_list_end());
        // `loc=null`, as §380 assigns, reads as end of list too.
        t.cur_input.loc_field = NULL;
        assert!(t.at_list_end());
        t.cur_input.loc_field = NULL + 1;
        assert!(t.at_list_end());
        // In a file level `loc` is a buffer index: never the end of a token list.
        t.cur_input.state_field = MID_LINE;
        t.cur_input.loc_field = 1;
        assert!(!t.at_list_end());
        Tex::dealloc(t);
    }

    /// §1309 do_endv: a literal `loc_field=null` is false at the end of every v-template, which
    /// would raise "interwoven alignment preambles" at the first `&` or `\\cr` of every `\\halign`.
    #[test]
    fn rec_at_list_end_tests_a_saved_level() {
        let t = new_tex();
        let mut r = t.cur_input;
        r.state_field = TOKEN_LIST;
        r.index_field = V_TEMPLATE;
        r.start_field = OMIT_TEMPLATE;
        r.loc_field = OMIT_TEMPLATE + 1;
        assert!(!t.rec_at_list_end(r), "the end template token is still unread");
        r.loc_field = OMIT_TEMPLATE + 2;
        assert_ne!(r.loc_field, NULL);
        assert!(t.rec_at_list_end(r), "at the terminator");
        r.loc_field = NULL;
        assert!(t.rec_at_list_end(r), "§380 assigns loc:=null");
        r.state_field = MID_LINE;
        r.loc_field = 1;
        assert!(!t.rec_at_list_end(r), "in a file level loc is a buffer index");
        Tex::dealloc(t);
    }

    #[test]
    fn large_lists_use_geometric_classes() {
        let t = new_tex();
        let n = 5000;
        let m = t.tb_mark();
        for i in 0..n {
            t.tb_push(1000 + i);
        }
        let p = t.tb_commit(m);
        assert_eq!(t.tok_len(p), n);
        let c = (t.toks[p] & 0xFF) as usize;
        assert!(c >= EXACT_CLASSES);
        assert!(CLASS_CAP[c] >= n);
        t.flush_list(p);
        // A slightly shorter list in the same class reuses the block.
        let m = t.tb_mark();
        for i in 0..n - 1 {
            t.tb_push(2000 + i);
        }
        let q = t.tb_commit(m);
        assert_eq!(q, p);
        assert_eq!(t.tok_len(q), n - 1, "the terminator moved with the shorter list");
        Tex::dealloc(t);
    }

    /// Small deterministic generator, so that a failure reproduces.
    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }
        fn below(&mut self, n: u64) -> u64 {
            self.next() % n
        }
    }

    /// Random non-LIFO allocation against a model; proves blocks never overlap and survive.
    #[test]
    fn stress_against_a_model() {
        let t = new_tex();
        let mut rng = Rng(0x9E3779B97F4A7C15);
        let mut live: Vec<(i32, Vec<i32>, i32)> = Vec::new(); // (ptr, tokens, extra refs)
        for step in 0..200_000u64 {
            let r = rng.below(100);
            if r < 45 || live.is_empty() {
                // Commit a list of mixed sizes.
                let len = match rng.below(20) {
                    0 => 65 + rng.below(1000),
                    1..=4 => 0,
                    _ => rng.below(64),
                } as usize;
                let toks: Vec<i32> = (0..len).map(|i| 1 + ((step as i32) << 8 | (i as i32 & 255))).collect();
                let m = t.tb_mark();
                for &x in &toks {
                    t.tb_push(x);
                }
                let p = t.tb_commit_ref(m);
                live.push((p, toks, 0));
            } else if r < 70 {
                // Free one chosen at random: frees are non-LIFO.
                let i = rng.below(live.len() as u64) as usize;
                let (p, toks, refs) = live.swap_remove(i);
                assert_eq!(list(t, p), toks, "a list changed while alive");
                assert_eq!(t.token_ref_count(p), refs);
                for _ in 0..=refs {
                    t.delete_token_ref(p);
                }
            } else if r < 85 {
                let i = rng.below(live.len() as u64) as usize;
                t.add_token_ref(live[i].0);
                live[i].2 += 1;
            } else {
                let i = rng.below(live.len() as u64) as usize;
                assert_eq!(list(t, live[i].0), live[i].1);
            }
        }
        for (p, toks, _) in &live {
            assert_eq!(&list(t, *p), toks);
        }
        assert_eq!(t.tb_ptr, 0);
        Tex::dealloc(t);
    }

    /// Every cell is in one block, a free list holds free blocks once each, and what is in use is not on one, and reads as it should.
    fn heap_is_whole(t: &Tex, live: &[(i32, Vec<i32>, i32)]) {
        let mut blocks = std::collections::BTreeMap::new();
        let mut p = TOKS_FIRST_FREE;
        while p < t.toks_top {
            let c = t.toks[p] & 0xFF;
            blocks.insert(p, c);
            p += CLASS_CAP[c as usize] + 2;
        }
        assert_eq!(p, t.toks_top, "a block runs over the top");
        let mut free = std::collections::BTreeSet::new();
        for c in 0..NUM_CLASSES {
            let mut q = t.tok_free[c as i32];
            while q != 0 {
                assert_eq!(blocks.get(&q), Some(&(c as i32)), "a free list holds what is no block of its class");
                assert!(free.insert(q), "twice on the free lists");
                q = t.toks[q + 1];
            }
        }
        for (p, toks, refs) in live {
            assert!(blocks.contains_key(p), "in use and no block");
            assert!(!free.contains(p), "in use and free");
            assert_eq!(&list(t, *p), toks, "a list changed while alive");
            assert_eq!(t.token_ref_count(*p), *refs);
        }
    }

    /// `stress_against_a_model` where the bump space has come to its end, so that a block comes from a free list or by splitting and joining, and may not come at all.
    #[test]
    fn reclaim_against_a_model() {
        for (room, seed) in [(3000, 0x2545F4914F6CDD1Du64), (20000, 0x9E3779B97F4A7C15), (150000, 0xD1B54A32D192ED03)] {
            let t = nearly_full(room);
            let (mut rng, mut none, mut top, mut went_back) = (Rng(seed), 0, t.toks_top, false);
            let mut live: Vec<(i32, Vec<i32>, i32)> = Vec::new();
            for step in 0..200_000u64 {
                let r = rng.below(100);
                if r < 50 || live.is_empty() {
                    let len = match rng.below(20) {
                        0 => 65 + rng.below((room / 4) as u64),
                        1..=3 => 0,
                        4..=8 => 1 + rng.below(3),
                        _ => rng.below(64),
                    } as i32;
                    let p = t.tok_try_alloc(len);
                    if p == 0 {
                        none += 1;
                        // A full heap has what it had.
                        heap_is_whole(t, &live);
                        for _ in 0..live.len() / 2 {
                            let i = rng.below(live.len() as u64) as usize;
                            let (q, _, refs) = live.swap_remove(i);
                            for _ in 0..=refs {
                                t.delete_token_ref(q);
                            }
                        }
                        continue;
                    }
                    assert_eq!(t.token_ref_count(p), 0, "a block given out has no references");
                    let toks: Vec<i32> = (0..len).map(|i| 1 + ((step as i32) << 8 | (i & 255))).collect();
                    for (k, &x) in toks.iter().enumerate() {
                        t.toks[p + 1 + k as i32] = x;
                    }
                    t.toks[p + 1 + len] = 0;
                    live.push((p, toks, 0));
                } else if r < 90 {
                    let i = rng.below(live.len() as u64) as usize;
                    let (p, toks, refs) = live.swap_remove(i);
                    assert_eq!(list(t, p), toks, "a list changed while alive");
                    if rng.below(4) == 0 {
                        t.flush_list(p); // regardless of its references, as WEB's flush_list
                    } else {
                        for _ in 0..=refs {
                            t.delete_token_ref(p);
                        }
                    }
                } else {
                    let i = rng.below(live.len() as u64) as usize;
                    t.add_token_ref(live[i].0);
                    live[i].2 += 1;
                }
                went_back |= t.toks_top < top;
                top = t.toks_top;
                if step % 4096 == 0 {
                    heap_is_whole(t, &live);
                }
            }
            heap_is_whole(t, &live);
            assert!(none > 0, "it never came to a full heap, with {} cells", room);
            assert!(went_back, "the end of the heap never went back to the bump space, with {} cells", room);
            Tex::dealloc(t);
        }
    }
}
