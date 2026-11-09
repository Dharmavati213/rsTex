//! §128-150 Parts 8-9: packed data, dynamic memory allocation; §180-190 Part 11: memory layout.
//!
//! The allocator is literal: `get_node` and `free_node` fix every node address, and those show in
//! the statistics of `\tracingstats` and of `\dump`.  The one change is the contract's: `is_char_node(p)`
//! is `p >= MEM_SPLIT`, a compile-time boundary, so lo memory may grow only up to `MEM_SPLIT` and hi
//! memory down only to it.  `hi_mem_min` and `lo_mem_max` keep their meaning as the two water marks,
//! but neither is compared with the other any more.
//!
//! `print_word`, `check_mem` and `search_mem` are `debug` code in WEB, not compiled in the oracle.  They
//! are here since WEB declares them; nothing in a release run calls them.  tex.ch declares
//! `free` and `was_free` with ten elements, to be allocated by whoever turns `debug` on, so `check_mem`
//! keeps its own in `HVars` and the two fields of `Tex` stay unused.
//!
//! DROPPED §703: a section of `forward` declarations, which tangle prints after search_mem.  All procedures are methods of `Tex`.
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
pub struct HVars {
    /// §183 `free`, indexed by `mem` position: only `check_mem` uses it.
    pub free: Vec<bool>,
    /// §183 `was_free`.
    pub was_free: Vec<bool>,
}

impl Tex {
    /// §132
    pub fn print_word(&mut self, w: Word) {
        self.print_int(w.int());
        self.print_char(b' ' as i32);
        self.print_scaled(w.sc());
        self.print_char(b' ' as i32);
        self.print_scaled(round(UNITY as f64 * w.gr()));
        self.print_ln();
        self.print_int(w.lh());
        self.print_char(b'=' as i32);
        self.print_int(w.b0());
        self.print_char(b':' as i32);
        self.print_int(w.b1());
        self.print_char(b';' as i32);
        self.print_int(w.rh());
        self.print_char(b' ' as i32);
        self.print_int(w.q0());
        self.print_char(b':' as i32);
        self.print_int(w.q1());
        self.print_char(b':' as i32);
        self.print_int(w.q2());
        self.print_char(b':' as i32);
        self.print_int(w.q3());
    }

    /// §138
    pub fn get_avail(&mut self) -> i32 {
        let mut p = self.avail;
        if p != NULL {
            self.avail = self.link(self.avail);
        } else if self.mem_end < MEM_MAX {
            self.mem_end += 1;
            p = self.mem_end;
        } else if self.v_webmem.fine != 0 {
            // `mem_end` is `mem_top`, which is `MEM_MAX` here, so every new cell comes from below `hi_mem_min`, and this is where `webmem.rs` sees one.
            p = self.web_cell();
        } else {
            self.hi_mem_min -= 1;
            p = self.hi_mem_min;
            if self.hi_mem_min < MEM_SPLIT {
                self.runaway();
                self.overflow(b"main memory size", self.main_memory_size());
            }
            if p & 0xFFFF == 0 {
                self.web_cells_grew();
            }
        }
        self.set_link(p, NULL);
        self.dyn_used += 1;
        p
    }

    /// §141
    /// One-word mem lists only (§764, §765, §1274); token lists use toks.rs's flush_list.
    pub fn mem_flush_list(&mut self, p: i32) {
        if p != NULL {
            let mut r = p;
            let mut q: i32;
            loop {
                q = r;
                r = self.link(r);
                self.dyn_used -= 1;
                if r == NULL {
                    break;
                }
            }
            self.set_link(q, self.avail);
            self.avail = p;
        }
    }

    /// §143.  Out of line for most callers; the three that make most of the calls have `get_node_leaf` in line.
    #[inline(never)]
    pub fn get_node(&mut self, s: i32) -> i32 {
        self.get_node_leaf(s)
    }

    /// §143 as far as its commonest end: §145 for the first node of the search, `rover`, and §146 from its top.  Five calls in six
    /// end there on books/hott/hott-a4, after merging the node freed just above `rover`, so this has no loop over the ring and calls
    /// nothing on that way, and needs no frame: `conditional`'s if nodes, `copy_node_list`'s medium ones and `pf_condition` have
    /// it in line.  `get_node_rest` goes on from where this stops, so the steps, their order and the words written are §143's.
    ///
    /// Two of §145's steps do nothing for this node and are left out.  A free node's size is above 0, so `q` starts above `p`,
    /// which is `rover`, and only grows: `q = rover` cannot hold, so `rover` stays, and §146's `rover := p` would store what is
    /// there.
    #[inline(always)]
    pub fn get_node_leaf(&mut self, s: i32) -> i32 {
        // §145 Try to allocate within node p and its physical successors, for p = rover
        let p = self.rover;
        debug_assert!(self.node_size(p) > 0, "§145 would merge forever after a free node of size 0");
        let mut q = p + self.node_size(p);
        while self.is_empty(q) {
            let t = self.rlink(q);
            self.set_llink(t, self.llink(q));
            self.set_rlink(self.llink(q), t);
            q += self.node_size(q);
        }
        let r = q - s;
        if r > p + 1 {
            // §146 Allocate from the top of node p and goto found
            self.set_node_size(p, r - p);
            return self.get_node_found(r, s);
        }
        self.get_node_rest(s, p, q)
    }

    /// §143 from where `get_node_leaf` stops: node `p`, which is `rover`, is merged with its free successors up to `q`, and too
    /// small for §146.  The rest of §145 for it, §147, comes first, then §145 for the other nodes of the ring, and §144's growth
    /// and its restart, as in §143.
    #[inline(never)]
    fn get_node_rest(&mut self, s: i32, p: i32, q: i32) -> i32 {
        let mut p = p;
        let mut q = q;
        let mut r = q - s;
        let mut t: i32;
        'found: {
            if r == p && self.rlink(p) != p {
                // §147 Allocate entire node p and goto found
                self.rover = self.rlink(p);
                t = self.llink(p);
                self.set_llink(self.rover, t);
                self.set_rlink(t, self.rover);
                break 'found;
            }
            self.set_node_size(p, q - p);
            p = self.rlink(p);
            // Whether §145 is to try node p: not when the search is back at `rover` already, and always after a restart, since
            // §143's loop is a `repeat`.
            let mut more = p != self.rover;
            loop {
                if more {
                    loop {
                        // §145 Try to allocate within node p and its physical successors, and goto found if allocation was possible
                        q = p + self.node_size(p);
                        while self.is_empty(q) {
                            t = self.rlink(q);
                            if q == self.rover {
                                self.rover = t;
                            }
                            self.set_llink(t, self.llink(q));
                            self.set_rlink(self.llink(q), t);
                            q += self.node_size(q);
                        }
                        r = q - s;
                        if r > p + 1 {
                            // §146 Allocate from the top of node p and goto found
                            self.set_node_size(p, r - p);
                            self.rover = p;
                            break 'found;
                        }
                        if r == p && self.rlink(p) != p {
                            // §147 Allocate entire node p and goto found
                            self.rover = self.rlink(p);
                            t = self.llink(p);
                            self.set_llink(self.rover, t);
                            self.set_rlink(t, self.rover);
                            break 'found;
                        }
                        self.set_node_size(p, q - p);
                        p = self.rlink(p);
                        if p == self.rover {
                            break;
                        }
                    }
                }
                if s == 0o10000000000 {
                    return MAX_HALFWORD;
                }
                // hi_mem_min: where `webmem.rs` says the oracle's would be, as this `mem` has no token lists in it.
                let hi = if self.lo_mem_max >= self.v_webmem.lo_watch { self.web_hi() } else { MEM_SPLIT };
                if self.lo_mem_max + 2 < hi && self.lo_mem_max + 2 <= MEM_BOT + MAX_HALFWORD {
                    // §144 Grow more variable-size memory and goto restart
                    if hi - self.lo_mem_max >= 1998 {
                        t = self.lo_mem_max + 1000;
                    } else {
                        t = self.lo_mem_max + 1 + (hi - self.lo_mem_max) / 2;
                    }
                    p = self.llink(self.rover);
                    q = self.lo_mem_max;
                    self.set_rlink(p, q);
                    self.set_llink(self.rover, q);
                    if t > MEM_BOT + MAX_HALFWORD {
                        t = MEM_BOT + MAX_HALFWORD;
                    }
                    self.set_rlink(q, self.rover);
                    self.set_llink(q, p);
                    self.set_link(q, EMPTY_FLAG);
                    self.set_node_size(q, t - self.lo_mem_max);
                    self.lo_mem_max = t;
                    self.set_link(self.lo_mem_max, NULL);
                    self.set_info(self.lo_mem_max, NULL);
                    self.rover = q;
                    if self.v_webmem.fine != 0 {
                        self.web_lo_grew();
                    }
                    // restart:
                    p = self.rover;
                    more = true;
                    continue;
                }
                self.overflow(b"main memory size", self.main_memory_size());
            }
        }
        self.get_node_found(r, s)
    }

    /// §143's `found`.
    #[inline(always)]
    fn get_node_found(&mut self, r: i32, s: i32) -> i32 {
        self.set_link(r, NULL);
        self.var_used += s;
        // §1907 Initialize bigger nodes with SyncTeX information
        if s >= MEDIUM_NODE_SIZE {
            self.set_sync_tag(r + s, self.synctex_tag());
            self.set_sync_line(r + s, self.line);
        }
        r
    }

    /// §148
    pub fn free_node(&mut self, p: i32, s: i32) {
        self.set_node_size(p, s);
        self.set_link(p, EMPTY_FLAG);
        let q = self.llink(self.rover);
        self.set_llink(p, q);
        self.set_rlink(p, self.rover);
        self.set_llink(self.rover, p);
        self.set_rlink(q, p);
        self.var_used -= s;
    }

    /// §149
    pub fn sort_avail(&mut self) {
        let mut q: i32;
        let mut r: i32;
        let _ = self.get_node(0o10000000000); // merge adjacent free areas
        let mut p = self.rlink(self.rover);
        self.set_rlink(self.rover, MAX_HALFWORD);
        let old_rover = self.rover;
        while p != old_rover {
            // §150 Sort p into the list starting at rover and advance p to rlink(p)
            if p < self.rover {
                q = p;
                p = self.rlink(q);
                self.set_rlink(q, self.rover);
                self.rover = q;
            } else {
                q = self.rover;
                while self.rlink(q) < p {
                    q = self.rlink(q);
                }
                r = self.rlink(p);
                self.set_rlink(p, self.rlink(q));
                self.set_rlink(q, p);
                p = r;
            }
        }
        p = self.rover;
        while self.rlink(p) != MAX_HALFWORD {
            self.set_llink(self.rlink(p), p);
            p = self.rlink(p);
        }
        self.set_rlink(p, self.rover);
        self.set_llink(self.rover, p);
    }

    /// §185
    pub fn check_mem(&mut self, print_locs: bool) {
        let mut p: i32;
        let mut q: i32;
        let mut clobbered: bool;
        if self.host.hv.mem.free.is_empty() {
            self.host.hv.mem.free = vec![false; MEM_CAP];
            self.host.hv.mem.was_free = vec![false; MEM_CAP];
        }
        let mut free = core::mem::take(&mut self.host.hv.mem.free);
        let mut was_free = core::mem::take(&mut self.host.hv.mem.was_free);
        for p in MEM_MIN..=self.lo_mem_max {
            free[p as usize] = false;
        }
        for p in self.hi_mem_min..=self.mem_end {
            free[p as usize] = false;
        }
        // §186 Check single-word avail list
        p = self.avail;
        q = NULL;
        clobbered = false;
        while p != NULL {
            if p > self.mem_end || p < self.hi_mem_min {
                clobbered = true;
            } else if free[p as usize] {
                clobbered = true;
            }
            if clobbered {
                self.print_nl_s(b"AVAIL list clobbered at ");
                self.print_int(q);
                break; // goto done1
            }
            free[p as usize] = true;
            q = p;
            p = self.link(q);
        }
        // done1:
        // §187 Check variable-size avail list
        p = self.rover;
        q = NULL;
        clobbered = false;
        'done2: loop {
            if p >= self.lo_mem_max || p < MEM_MIN {
                clobbered = true;
            } else if self.rlink(p) >= self.lo_mem_max || self.rlink(p) < MEM_MIN {
                clobbered = true;
            } else if !self.is_empty(p)
                || self.node_size(p) < 2
                || p + self.node_size(p) > self.lo_mem_max
                || self.llink(self.rlink(p)) != p
            {
                clobbered = true;
            }
            if clobbered {
                self.print_nl_s(b"Double-AVAIL list clobbered at ");
                self.print_int(q);
                break 'done2;
            }
            let e = p + self.node_size(p) - 1;
            q = p;
            while q <= e {
                // mark all locations free
                if free[q as usize] {
                    self.print_nl_s(b"Doubly free location at ");
                    self.print_int(q);
                    break 'done2;
                }
                free[q as usize] = true;
                q += 1;
            }
            q = p;
            p = self.rlink(p);
            if p == self.rover {
                break;
            }
        }
        // done2:
        // §188 Check flags of unavailable nodes
        p = MEM_MIN;
        while p <= self.lo_mem_max {
            // node p should not be empty
            if self.is_empty(p) {
                self.print_nl_s(b"Bad flag at ");
                self.print_int(p);
            }
            while p <= self.lo_mem_max && !free[p as usize] {
                p += 1;
            }
            while p <= self.lo_mem_max && free[p as usize] {
                p += 1;
            }
        }
        if print_locs {
            // §189 Print newly busy locations
            self.print_nl_s(b"New busy locs:");
            for p in MEM_MIN..=self.lo_mem_max {
                if !free[p as usize] && (p > self.was_lo_max || was_free[p as usize]) {
                    self.print_char(b' ' as i32);
                    self.print_int(p);
                }
            }
            for p in self.hi_mem_min..=self.mem_end {
                if !free[p as usize] && (p < self.was_hi_min || p > self.was_mem_end || was_free[p as usize]) {
                    self.print_char(b' ' as i32);
                    self.print_int(p);
                }
            }
        }
        for p in MEM_MIN..=self.lo_mem_max {
            was_free[p as usize] = free[p as usize];
        }
        for p in self.hi_mem_min..=self.mem_end {
            was_free[p as usize] = free[p as usize];
        }
        self.was_mem_end = self.mem_end;
        self.was_lo_max = self.lo_mem_max;
        self.was_hi_min = self.hi_mem_min;
        self.host.hv.mem.free = free;
        self.host.hv.mem.was_free = was_free;
    }

    /// §190
    pub fn search_mem(&mut self, p: i32) {
        for q in MEM_MIN..=self.lo_mem_max {
            if self.link(q) == p {
                self.print_nl_s(b"LINK(");
                self.print_int(q);
                self.print_char(b')' as i32);
            }
            if self.info(q) == p {
                self.print_nl_s(b"INFO(");
                self.print_int(q);
                self.print_char(b')' as i32);
            }
        }
        for q in self.hi_mem_min..=self.mem_end {
            if self.link(q) == p {
                self.print_nl_s(b"LINK(");
                self.print_int(q);
                self.print_char(b')' as i32);
            }
            if self.info(q) == p {
                self.print_nl_s(b"INFO(");
                self.print_int(q);
                self.print_char(b')' as i32);
            }
        }
        // §273 Search eqtb for equivalents equal to p
        for q in ACTIVE_BASE..=(BOX_BASE + 255) {
            if self.equiv(q) == p {
                self.print_nl_s(b"EQUIV(");
                self.print_int(q);
                self.print_char(b')' as i32);
            }
        }
        // §307 Search save_stack for equivalents that point to p
        if self.save_ptr > 0 {
            for q in 0..=(self.save_ptr - 1) {
                if self.equiv_field(self.save_stack[q]) == p {
                    self.print_nl_s(b"SAVE(");
                    self.print_int(q);
                    self.print_char(b')' as i32);
                }
            }
        }
        // §1110 Search hyph_list for pointers to p
        for q in 0..=self.hyph_size {
            if self.hyph_list[q] == p {
                self.print_nl_s(b"HYPH(");
                self.print_int(q);
                self.print_char(b')' as i32);
            }
        }
    }
}

/// A machine whose `mem` is as INITEX leaves it, for the tests of this module and of nodes, show and copy.
#[cfg(test)]
pub(crate) mod memkit {
    use super::*;
    use crate::toks::testkit::machine;

    /// What INITEX's table entries do to `mem`, sections 182 and 246 of the WEB: print.rs owns those.
    /// Not the list heads: nothing here needs them.
    pub fn mem_init(t: &mut Tex) {
        for k in (MEM_BOT + 1)..=LO_MEM_STAT_MAX {
            t.mem[k].set_sc(0);
        }
        let mut k = MEM_BOT;
        while k <= LO_MEM_STAT_MAX {
            t.set_glue_ref_count(k, NULL + 1);
            t.set_stretch_order(k, NORMAL);
            t.set_shrink_order(k, NORMAL);
            k += GLUE_SPEC_SIZE;
        }
        t.set_stretch(FIL_GLUE, UNITY);
        t.set_stretch_order(FIL_GLUE, FIL);
        t.set_stretch(FILL_GLUE, UNITY);
        t.set_stretch_order(FILL_GLUE, FILL);
        t.set_stretch(SS_GLUE, UNITY);
        t.set_stretch_order(SS_GLUE, FIL);
        t.set_shrink(SS_GLUE, UNITY);
        t.set_shrink_order(SS_GLUE, FIL);
        t.set_stretch(FIL_NEG_GLUE, -UNITY);
        t.set_stretch_order(FIL_NEG_GLUE, FIL);
        t.rover = LO_MEM_STAT_MAX + 1;
        t.set_link(t.rover, EMPTY_FLAG);
        t.set_node_size(t.rover, 1000);
        t.set_llink(t.rover, t.rover);
        t.set_rlink(t.rover, t.rover);
        t.lo_mem_max = t.rover + 1000;
        t.set_link(t.lo_mem_max, NULL);
        t.set_info(t.lo_mem_max, NULL);
        t.avail = NULL;
        t.mem_end = MEM_TOP;
        t.hi_mem_min = HI_MEM_STAT_MIN;
        t.var_used = LO_MEM_STAT_MAX + 1 - MEM_BOT;
        t.dyn_used = HI_MEM_STAT_USAGE;
        for k in GLUE_BASE..LOCAL_BASE {
            t.set_equiv(k, ZERO_GLUE);
        }
    }

    pub fn mem_machine() -> &'static mut Tex {
        let t = machine();
        mem_init(t);
        t
    }

    /// The free blocks in ring order, from `rover`.
    pub fn ring(t: &Tex) -> Vec<(i32, i32)> {
        let mut v = Vec::new();
        let mut p = t.rover;
        loop {
            v.push((p, t.node_size(p)));
            assert_eq!(t.link(p), EMPTY_FLAG, "a block in the ring has the flag");
            assert_eq!(t.llink(t.rlink(p)), p, "the ring is doubly linked");
            p = t.rlink(p);
            if p == t.rover {
                return v;
            }
        }
    }

    /// §1489: what `var_used` should be, from the ring alone.  The ring must be sorted.
    pub fn recount_var_used(t: &Tex) -> i32 {
        let (mut p, mut x) = (MEM_BOT, 0);
        for (q, s) in ring(t) {
            x += q - p;
            p = q + s;
        }
        x + t.lo_mem_max - p
    }
}

#[cfg(test)]
mod tests {
    use super::memkit::*;
    use super::*;

    fn table(data: &'static [u8]) -> Vec<Vec<&'static str>> {
        core::str::from_utf8(data).unwrap().lines().map(|l| l.split('\t').collect()).collect()
    }

    #[test]
    fn get_node_takes_from_the_top_of_the_block() {
        let t = mem_machine();
        let rover = LO_MEM_STAT_MAX + 1;
        let a = t.get_node(4);
        assert_eq!(a, rover + 1000 - 4);
        assert_eq!(t.link(a), NULL, "§143: this node is now nonempty");
        let b = t.get_node(9);
        assert_eq!(b, a - 9);
        assert_eq!(ring(t), vec![(rover, 987)]);
        assert_eq!(t.var_used, 20 + 13);
        t.free_node(a, 4);
        assert_eq!(t.var_used, 20 + 9);
        assert_eq!(ring(t), vec![(rover, 987), (a, 4)]);
        Tex::dealloc(t);
    }

    #[test]
    fn free_node_goes_just_before_rover() {
        let t = mem_machine();
        let rover = t.rover;
        let (a, b, c) = (t.get_node(4), t.get_node(4), t.get_node(4));
        t.free_node(a, 4);
        t.free_node(c, 4);
        assert_eq!(ring(t), vec![(rover, 988), (a, 4), (c, 4)]);
        t.free_node(b, 4);
        assert_eq!(ring(t), vec![(rover, 988), (a, 4), (c, 4), (b, 4)]);
        // §145 merges only what is physically after the block it looks at, and only when asked.
        let d = t.get_node(6);
        assert_eq!(d, rover + 988 + 12 - 6);
        assert_eq!(t.rover, rover, "the search stopped at the first block, and a is merged into it");
        assert_eq!(ring(t), vec![(rover, 994)]);
        Tex::dealloc(t);
    }

    #[test]
    fn an_exact_fit_takes_the_whole_block_and_one_word_is_never_left() {
        let t = mem_machine();
        let rover = t.rover;
        let _ = t.get_node(980); // the top of the first block stays busy, so what is below it cannot merge upward
        assert_eq!(ring(t), vec![(rover, 20)]);
        let x = t.get_node(5);
        let _ = t.get_node(13);
        assert_eq!(x, rover + 15);
        assert_eq!(ring(t), vec![(rover, 2)]);
        t.free_node(x, 5);
        assert_eq!(ring(t), vec![(rover, 2), (x, 5)]);
        // r=p and rlink(p)<>p: §147 takes the whole block, and it leaves the ring.
        let c = t.get_node(5);
        assert_eq!(c, x);
        assert_eq!(t.rover, rover);
        assert_eq!(ring(t), vec![(rover, 2)]);
        // A block of 5 does not give 4: r=p+1 fails r>p+1, since one word cannot be a free block.  So the search
        // goes round the ring, finds nothing, and grows.
        t.free_node(c, 5);
        let lo = t.lo_mem_max;
        let e = t.get_node(4);
        assert_eq!(t.lo_mem_max, lo + 1000);
        assert_eq!(e, lo + 1000 - 4);
        assert_eq!(ring(t), vec![(lo, 996), (rover, 2), (x, 5)]);
        // And it gives 3, from its top.
        let f = t.get_node(3);
        assert_eq!(f, lo + 996 - 3, "rover is the grown block now");
        let g = t.get_node(990);
        assert_eq!(g, lo + 3);
        let h = t.get_node(3);
        assert_eq!(h, lo, "what was left of the grown block, whole");
        let i = t.get_node(3);
        assert_eq!(i, x + 2, "the block of 5 does give 3, from its top");
        assert_eq!(ring(t), vec![(x, 2), (rover, 2)]);
        Tex::dealloc(t);
    }

    #[test]
    fn the_last_block_is_not_given_away_whole() {
        let t = mem_machine();
        let rover = t.rover;
        // r=p but rlink(p)=p, so §147 does not apply: the ring must never be empty, and it grows.
        let r = t.get_node(1000);
        assert_eq!(t.lo_mem_max, rover + 2000, "§144: 1000 more words");
        assert_eq!(r, rover + 1000);
        assert_eq!(t.var_used, 20 + 1000);
        Tex::dealloc(t);
    }

    #[test]
    fn asking_for_two_to_the_thirty_merges_and_returns_max_halfword() {
        let t = mem_machine();
        let rover = t.rover;
        let (a, b) = (t.get_node(4), t.get_node(4));
        t.free_node(b, 4);
        t.free_node(a, 4);
        assert_eq!(t.get_node(0o10000000000), MAX_HALFWORD);
        assert_eq!(ring(t), vec![(rover, 1000)]);
        assert_eq!(t.var_used, 20);
        Tex::dealloc(t);
    }

    /// What the oracle's format file holds after the same assignments: see tests/oracle/gen_mem_tables.py.
    /// Its `mem_bot` is 0, as ours is, so an address here is the same.
    #[test]
    fn addresses_match_the_oracle() {
        let rows = table(include_bytes!("../testdata/mem_oracle.tsv"));
        assert!(rows.len() >= 30);
        for row in rows {
            let t = mem_machine();
            let mut regs = [NULL; 256];
            for op in row[1].split(';') {
                replay(t, &mut regs, op);
            }
            let tracked = t.var_used;
            t.sort_avail();
            let free: Vec<String> = ring(t).iter().map(|&(a, s)| format!("{}:{}", a, s)).collect();
            assert_eq!(free.join(" "), row[5], "free blocks of {}", row[0]);
            assert_eq!(t.lo_mem_max, row[2].parse::<i32>().unwrap(), "lo_mem_max of {}", row[0]);
            assert_eq!(t.rover, row[3].parse::<i32>().unwrap(), "rover of {}", row[0]);
            assert_eq!(recount_var_used(t), row[4].parse::<i32>().unwrap(), "var_used of {}", row[0]);
            assert_eq!(tracked, recount_var_used(t), "tracked var_used of {}", row[0]);
            Tex::dealloc(t);
        }
    }

    fn replay(t: &mut Tex, regs: &mut [i32; 256], op: &str) {
        let (head, rest) = op.split_once(' ').unwrap_or((op, ""));
        let a: usize = head[1..].parse().unwrap();
        match head.as_bytes()[0] {
            b'B' => {
                let mut i = 0;
                let l = replay_items(t, rest.as_bytes(), &mut i);
                let b = t.new_null_box();
                t.set_list_ptr(b, l);
                t.flush_node_list(regs[a]);
                regs[a] = b;
            }
            b'C' => {
                let c = t.copy_node_list(regs[rest.parse::<usize>().unwrap()]);
                t.flush_node_list(regs[a]);
                regs[a] = c;
            }
            b'V' => {
                t.flush_node_list(regs[a]);
                regs[a] = NULL;
            }
            _ => panic!("bad operation {}", op),
        }
    }

    fn replay_items(t: &mut Tex, s: &[u8], i: &mut usize) -> i32 {
        let (mut head, mut tail) = (NULL, NULL);
        while *i < s.len() && s[*i] != b')' {
            let n = match s[*i] {
                b'k' => t.new_kern(UNITY),
                b'p' => t.new_penalty(0),
                b'g' => {
                    let q = t.new_spec(ZERO_GLUE);
                    t.set_width(q, UNITY);
                    let n = t.new_glue(q);
                    t.set_glue_ref_count(q, NULL);
                    n
                }
                b'r' => t.new_rule(),
                b'(' => {
                    *i += 1;
                    let l = replay_items(t, s, i);
                    let b = t.new_null_box();
                    t.set_list_ptr(b, l);
                    b
                }
                c => panic!("bad item {}", c as char),
            };
            if head == NULL {
                head = n;
            } else {
                t.set_link(tail, n);
            }
            tail = n;
            *i += 1;
        }
        head
    }

    #[test]
    fn get_avail_reuses_then_grows_down() {
        let t = mem_machine();
        let d = t.dyn_used;
        let a = t.get_avail();
        assert_eq!(a, HI_MEM_STAT_MIN - 1);
        assert_eq!(t.hi_mem_min, a);
        assert!(t.is_char_node(a));
        assert_eq!(t.link(a), NULL);
        let b = t.get_avail();
        assert_eq!(b, a - 1);
        assert_eq!(t.dyn_used, d + 2);
        t.free_avail(a);
        assert_eq!(t.avail, a);
        assert_eq!(t.dyn_used, d + 1);
        t.set_info(a, 78);
        t.free_avail(b);
        assert_eq!(t.link(b), a, "b is in front of a on avail");
        assert_eq!(t.get_avail(), b, "last freed, first reused");
        assert_eq!(t.link(b), NULL, "§138: an oft-desired initialization");
        assert_eq!(t.avail, a);
        let c = t.get_avail();
        assert_eq!(c, a);
        assert_eq!(t.info(c), 78, "and only the link");
        assert_eq!(t.avail, NULL);
        Tex::dealloc(t);
    }

    #[test]
    fn fast_get_avail_is_get_avail() {
        let t = mem_machine();
        let a = t.fast_get_avail();
        assert_eq!(a, HI_MEM_STAT_MIN - 1);
        t.free_avail(a);
        t.set_link(a, NULL);
        assert_eq!(t.fast_get_avail(), a);
        assert_eq!(t.avail, NULL);
        assert_eq!(t.dyn_used, HI_MEM_STAT_USAGE + 1);
        Tex::dealloc(t);
    }

    #[test]
    fn mem_flush_list_makes_the_whole_list_available_keeping_its_order() {
        let t = mem_machine();
        let (a, b, c) = (t.get_avail(), t.get_avail(), t.get_avail());
        t.set_link(a, b);
        t.set_link(b, c);
        let d = t.dyn_used;
        t.mem_flush_list(a);
        assert_eq!(t.dyn_used, d - 3);
        assert_eq!(t.avail, a);
        assert_eq!((t.link(a), t.link(b), t.link(c)), (b, c, NULL));
        t.mem_flush_list(NULL);
        assert_eq!(t.dyn_used, d - 3);
        let e = t.get_avail();
        t.set_link(e, NULL);
        t.mem_flush_list(e);
        assert_eq!(t.avail, e);
        assert_eq!(t.link(e), b, "the list goes in front of avail");
        Tex::dealloc(t);
    }

    /// The contract's change: hi memory stops at MEM_SPLIT, not at lo_mem_max.
    #[test]
    fn hi_memory_goes_down_to_mem_split_and_it_is_a_char_node() {
        let t = mem_machine();
        t.hi_mem_min = MEM_SPLIT + 1;
        assert_eq!(t.get_avail(), MEM_SPLIT);
        assert!(t.is_char_node(MEM_SPLIT));
        assert!(!t.is_char_node(MEM_SPLIT - 1));
        assert_eq!(t.lo_mem_max, LO_MEM_STAT_MAX + 1001, "lo memory is not involved");
        Tex::dealloc(t);
    }

    /// §144 near the boundary: half of what is left, as WEB has it near hi_mem_min, and below MEM_SPLIT.
    #[test]
    fn lo_memory_grows_up_to_mem_split_by_halves_at_the_end() {
        let t = mem_machine();
        let rover = t.rover;
        let _ = t.get_node(990);
        assert_eq!(ring(t), vec![(rover, 10)]);
        // WEB's first branch still applies at 1998 words from the boundary.
        let lo = MEM_SPLIT - 1998;
        t.set_link(lo, NULL);
        t.lo_mem_max = lo;
        let r = t.get_node(20);
        assert_eq!(t.lo_mem_max, lo + 1000);
        assert_eq!(r, lo + 1000 - 20);
        // The block used for the growth starts at the old lo_mem_max.
        assert_eq!(ring(t), vec![(lo, 980), (rover, 10)]);

        // And the second branch one word closer.
        let t2 = mem_machine();
        let rover2 = t2.rover;
        let _ = t2.get_node(990);
        let lo = MEM_SPLIT - 1997;
        t2.set_link(lo, NULL);
        t2.lo_mem_max = lo;
        let r = t2.get_node(20);
        assert_eq!(t2.lo_mem_max, lo + 1 + 1997 / 2);
        assert_eq!(r, t2.lo_mem_max - 20);
        assert_eq!(ring(t2), vec![(lo, 1 + 1997 / 2 - 20), (rover2, 10)]);

        // At the very end: lo_mem_max+2<MEM_SPLIT is the last time it grows, and it stays below.
        let t3 = mem_machine();
        let rover3 = t3.rover;
        let _ = t3.get_node(998);
        assert_eq!(ring(t3), vec![(rover3, 2)]);
        let lo = MEM_SPLIT - 3;
        t3.set_link(lo, NULL);
        t3.lo_mem_max = lo;
        let r = t3.get_node(2);
        assert_eq!(t3.lo_mem_max, MEM_SPLIT - 1);
        assert_eq!(r, MEM_SPLIT - 3, "a whole new block: it is 2 words, and the ring has another");
        assert_eq!(ring(t3), vec![(rover3, 2)]);
        assert!(!t3.is_char_node(t3.lo_mem_max));

        Tex::dealloc(t);
        Tex::dealloc(t2);
        Tex::dealloc(t3);
    }

    #[test]
    fn sort_avail_sorts_and_keeps_the_blocks() {
        let t = mem_machine();
        let rover = t.rover;
        let v: Vec<i32> = (0..6).map(|_| t.get_node(4)).collect();
        let _ = t.get_node(4); // holds the blocks apart from the big one
        // Free every second one, in an order that is not address order.
        t.free_node(v[3], 4);
        t.free_node(v[1], 4);
        t.free_node(v[5], 4);
        t.sort_avail();
        assert_eq!(t.rover, rover, "the lowest address");
        let mut want = vec![(rover, 972), (v[5], 4), (v[3], 4), (v[1], 4)];
        want.sort();
        assert_eq!(ring(t), want);
        assert_eq!(recount_var_used(t), t.var_used);
        Tex::dealloc(t);
    }

    /// §143 as one procedure, as it was before `get_node_leaf` and `get_node_rest` split it: what the test below holds them to.
    /// `n` counts the ways out: §146 and §147 at the first node, which is the leaf's, and further on; §144; and a merge of `rover`.
    fn get_node_literal(t: &mut Tex, s: i32, n: &mut [u32; 6]) -> i32 {
        let mut p: i32;
        let mut q: i32;
        let mut r: i32;
        let mut t_: i32;
        let mut first = 0;
        'found: {
            loop {
                // restart:
                p = t.rover;
                loop {
                    // §145
                    q = p + t.node_size(p);
                    while t.is_empty(q) {
                        t_ = t.rlink(q);
                        if q == t.rover {
                            t.rover = t_;
                            n[5] += 1;
                        }
                        t.set_llink(t_, t.llink(q));
                        t.set_rlink(t.llink(q), t_);
                        q += t.node_size(q);
                    }
                    r = q - s;
                    if r > p + 1 {
                        // §146
                        t.set_node_size(p, r - p);
                        t.rover = p;
                        n[first] += 1;
                        break 'found;
                    }
                    if r == p && t.rlink(p) != p {
                        // §147
                        t.rover = t.rlink(p);
                        t_ = t.llink(p);
                        t.set_llink(t.rover, t_);
                        t.set_rlink(t_, t.rover);
                        n[2 + first] += 1;
                        break 'found;
                    }
                    first = 1;
                    t.set_node_size(p, q - p);
                    p = t.rlink(p);
                    if p == t.rover {
                        break;
                    }
                }
                if s == 0o10000000000 {
                    return MAX_HALFWORD;
                }
                let hi = if t.lo_mem_max >= t.v_webmem.lo_watch { t.web_hi() } else { MEM_SPLIT };
                if t.lo_mem_max + 2 < hi && t.lo_mem_max + 2 <= MEM_BOT + MAX_HALFWORD {
                    // §144
                    if hi - t.lo_mem_max >= 1998 {
                        t_ = t.lo_mem_max + 1000;
                    } else {
                        t_ = t.lo_mem_max + 1 + (hi - t.lo_mem_max) / 2;
                    }
                    p = t.llink(t.rover);
                    q = t.lo_mem_max;
                    t.set_rlink(p, q);
                    t.set_llink(t.rover, q);
                    if t_ > MEM_BOT + MAX_HALFWORD {
                        t_ = MEM_BOT + MAX_HALFWORD;
                    }
                    t.set_rlink(q, t.rover);
                    t.set_llink(q, p);
                    t.set_link(q, EMPTY_FLAG);
                    t.set_node_size(q, t_ - t.lo_mem_max);
                    t.lo_mem_max = t_;
                    t.set_link(t.lo_mem_max, NULL);
                    t.set_info(t.lo_mem_max, NULL);
                    t.rover = q;
                    if t.v_webmem.fine != 0 {
                        t.web_lo_grew();
                    }
                    n[4] += 1;
                    continue;
                }
                t.overflow(b"main memory size", t.main_memory_size());
            }
        }
        // found:
        t.set_link(r, NULL);
        t.var_used += s;
        // §1907
        if s >= MEDIUM_NODE_SIZE {
            t.set_sync_tag(r + s, t.synctex_tag());
            t.set_sync_line(r + s, t.line);
        }
        r
    }

    /// `get_node`, and the leaf that three callers have in line, against `get_node_literal` on the same random requests and frees:
    /// the same address, `rover`, `var_used` and `lo_mem_max` after every call, and every word of lo memory now and then and at the
    /// end.  The requests are mostly of the sizes of nodes, with exact fits, merges across `rover`, `sort_avail` and its 2^30, and
    /// some that need §144 more than once in one call.
    #[test]
    fn get_node_is_the_literal_procedure() {
        let (a, b) = (mem_machine(), mem_machine());
        let mut seed: u64 = 0x9E37_79B9_7F4A_7C15;
        let mut next = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        let same_lo_mem = |a: &Tex, b: &Tex, i: usize| {
            assert_eq!(a.lo_mem_max, b.lo_mem_max);
            for k in MEM_BOT..=a.lo_mem_max {
                assert_eq!(a.mem[k].raw(), b.mem[k].raw(), "mem[{}] after step {}", k, i);
            }
        };
        let mut live: Vec<(i32, i32)> = Vec::new();
        let mut live_words = 0;
        let mut n = [0; 6];
        let mut grew_twice = 0;
        for i in 0..60000 {
            a.line = i as i32;
            b.line = i as i32;
            let x = next() % 100;
            if x == 0 {
                assert_eq!(a.get_node(0o10000000000), MAX_HALFWORD);
                assert_eq!(get_node_literal(b, 0o10000000000, &mut n), MAX_HALFWORD);
            } else if x == 1 {
                a.sort_avail();
                b.sort_avail();
            } else if x < 52 && live_words < 40000 || live.is_empty() {
                let y = next() % 100;
                let s = if y < 70 {
                    2 + (y % 12) as i32
                } else if y < 95 {
                    14 + (next() % 107) as i32
                } else if y < 99 {
                    121 + (next() % 879) as i32
                } else {
                    1000 + (next() % 1600) as i32
                };
                let grown = n[4];
                let p = if i % 2 == 0 { a.get_node_leaf(s) } else { a.get_node(s) };
                assert_eq!(p, get_node_literal(b, s, &mut n), "get_node({}) at step {}", s, i);
                grew_twice += (n[4] - grown >= 2) as u32;
                live.push((p, s));
                live_words += s;
            } else {
                let (p, s) = live.swap_remove((next() % live.len() as u64) as usize);
                a.free_node(p, s);
                b.free_node(p, s);
                live_words -= s;
            }
            assert_eq!((a.rover, a.var_used, a.lo_mem_max), (b.rover, b.var_used, b.lo_mem_max), "after step {}", i);
            if i % 509 == 0 {
                same_lo_mem(a, b, i);
            }
        }
        same_lo_mem(a, b, 60000);
        assert!(n.iter().all(|&k| k >= 20) && grew_twice > 0, "every way out is taken: {:?}, and {} calls grew twice", n, grew_twice);
        Tex::dealloc(a);
        Tex::dealloc(b);
    }

    /// `check_mem` prints only when something is wrong, so here it must not print at all.
    #[test]
    fn check_mem_is_silent_on_a_good_memory() {
        let t = mem_machine();
        let a = t.get_avail();
        let b = t.get_avail();
        t.free_avail(a);
        let n = t.get_node(4);
        let m = t.get_node(9);
        t.set_ty(n, KERN_NODE);
        t.set_ty(m, HLIST_NODE);
        t.free_node(n, 4);
        t.check_mem(false);
        assert_eq!((t.was_lo_max, t.was_hi_min, t.was_mem_end), (t.lo_mem_max, t.hi_mem_min, t.mem_end));
        let _ = b;
        Tex::dealloc(t);
    }

    #[test]
    fn check_mem_reports_what_is_clobbered() {
        use crate::toks::testkit::*;
        let t = mem_machine();
        print_setup(t);
        let n = t.get_node(4);
        t.check_mem(false);
        assert_eq!(printed(t), "");
        // A node freed twice: it is in the ring twice, and §187 finds the second time.
        t.free_node(n, 4);
        t.free_node(n, 4);
        t.check_mem(false);
        let s = printed(t);
        assert!(s.contains("Double-AVAIL list clobbered at "), "{:?}", s);
        Tex::dealloc(t);

        let t = mem_machine();
        print_setup(t);
        t.avail = LO_MEM_STAT_MAX + 5;
        t.check_mem(false);
        assert_eq!(printed(t), "AVAIL list clobbered at 0");
        Tex::dealloc(t);

        let t = mem_machine();
        print_setup(t);
        let n = t.get_node(4);
        t.set_link(n, EMPTY_FLAG); // looks free, and is not in the ring
        t.check_mem(false);
        assert_eq!(printed(t), format!("Bad flag at {}", n));
        Tex::dealloc(t);

        let t = mem_machine();
        print_setup(t);
        let n = t.get_node(4);
        t.free_node(n, 4);
        t.free_node(n + 2, 2); // inside the block just freed
        t.check_mem(false);
        assert_eq!(printed(t), format!("Doubly free location at {}", n + 2));
        Tex::dealloc(t);
    }

    #[test]
    fn check_mem_prints_newly_busy_locations() {
        use crate::toks::testkit::*;
        let t = mem_machine();
        print_setup(t);
        t.check_mem(false);
        let n = t.get_node(2);
        let c = t.get_avail();
        t.check_mem(true);
        assert_eq!(printed(t), format!("New busy locs: {} {} {}", n, n + 1, c));
        t.check_mem(true);
        assert_eq!(printed(t), "New busy locs:");
        Tex::dealloc(t);
    }

    #[test]
    fn search_mem_finds_the_pointers() {
        use crate::toks::testkit::*;
        let t = mem_machine();
        print_setup(t);
        // Zeroed: only what is set here refers to this number, in the live ranges.
        let target = 999999;
        let (a, b) = (t.get_node(2), t.get_avail());
        t.set_link(a, target);
        t.set_info(b, target);
        t.set_equiv(BOX_BASE + 7, target);
        t.save_ptr = 2;
        t.save_stack[1].set_rh(target);
        t.hyph_size = 3;
        t.hyph_list[3] = target;
        t.search_mem(target);
        assert_eq!(
            printed(t),
            format!("LINK({})\nINFO({})\nEQUIV({})\nSAVE(1)\nHYPH(3)", a, b, BOX_BASE + 7)
        );
        Tex::dealloc(t);
    }

    #[test]
    fn print_word_prints_it_in_all_ways() {
        use crate::toks::testkit::*;
        let t = mem_machine();
        print_setup(t);
        t.print_word(Word::hb(65536, 3, 4));
        // §132.  int and sc are on rh; b0 and b1 are on lh: raw.rs has the layout.  gr is the whole word as f64, not checked here.
        let s = printed(t);
        let mut l = s.lines();
        assert!(l.next().unwrap().starts_with("65536 1.0 "), "{:?}", s);
        assert_eq!(l.next().unwrap(), format!("{}=3:4;65536 0:1:4:3", 4 + (3 << 16)));
        Tex::dealloc(t);
    }
}
