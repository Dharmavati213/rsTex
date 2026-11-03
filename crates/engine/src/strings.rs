//! §38-53 Part 4: string handling.
//!
//! There is no pool file: §51's `loadpoolstrings` becomes the table `WELL_KNOWN_STRINGS` in access.rs.
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
///
/// The index of `search_string`, whose literal scan goes through every string each time, and most strings are
/// the names of control sequences.  It holds what `v_web2c.str_index` does not: the strings this run made, or with no format all from 256.  `search_string` makes it up to
/// what it is asked, so nothing is paid before the first file name, and a process starts with none, whether from a format or from a checkpoint.
#[derive(Default)]
pub struct HVars {
    /// The first string the index would hold.
    index_base: i32,
    /// Strings `index_base` up to this one are in the index.
    index_upto: i32,
    /// The newest such string for each value of `strings_bucket`, or 0.
    index_head: Vec<i32>,
    /// `index_prev[s-index_base]` is the next older string in the bucket of string `s`, or 0.
    index_prev: Vec<i32>,
}

const STRINGS_BUCKETS: usize = 1 << 13;

impl Tex {
    /// §43
    pub fn make_string(&mut self) -> i32 {
        if self.str_ptr == self.max_strings {
            self.overflow(b"number of strings", self.max_strings - self.init_str_ptr);
        }
        self.strings_index_note_new();
        self.str_ptr += 1;
        self.str_start[self.str_ptr] = self.pool_ptr;
        self.str_ptr - 1
    }

    /// Called before string `str_ptr` is made.  If the index holds a string of that number, it was flushed, and
    /// the text of the new one is another.  A flushed string alone does no harm, since `search_string` looks below
    /// the string it is given.
    #[inline(always)]
    pub fn strings_index_note_new(&mut self) {
        if self.str_ptr < self.host.hv.strings.index_upto {
            self.strings_index_cut();
        }
    }

    /// Takes the strings from `str_ptr` up out of the index.  A bucket has the newest first, so they are what
    /// its head leads to first.
    #[cold]
    #[inline(never)]
    fn strings_index_cut(&mut self) {
        let n = self.str_ptr;
        let hv = &mut self.host.hv.strings;
        let base = hv.index_base;
        for h in hv.index_head.iter_mut() {
            while *h >= n {
                *h = if *h >= base { hv.index_prev[(*h - base) as usize] } else { 0 };
            }
        }
        hv.index_prev.truncate((n - base).max(0) as usize);
        hv.index_upto = n.max(base);
    }

    #[inline(always)]
    fn strings_bucket(&self, s: i32) -> usize {
        crate::hash::hash_name(self.str_pool.range(self.str_start[s], self.str_start[s + 1])) as usize & (STRINGS_BUCKETS - 1)
    }

    /// The first string to look at for one equal to string `s`, or 0.  `s` is not in the index, and it holds all strings from `lowest` below `s` when this returns.
    /// `strings_index_next` gives the next one.
    pub fn strings_index_first(&mut self, lowest: i32, s: i32) -> i32 {
        if self.host.hv.strings.index_head.is_empty() || self.host.hv.strings.index_base != lowest {
            let hv = &mut self.host.hv.strings;
            hv.index_head.clear();
            hv.index_head.resize(STRINGS_BUCKETS, 0);
            hv.index_prev.clear();
            hv.index_base = lowest;
            hv.index_upto = lowest;
        }
        while self.host.hv.strings.index_upto < s {
            let k = self.host.hv.strings.index_upto;
            let b = self.strings_bucket(k);
            let hv = &mut self.host.hv.strings;
            hv.index_prev.push(hv.index_head[b]);
            hv.index_head[b] = k;
            hv.index_upto = k + 1;
        }
        self.host.hv.strings.index_head[self.strings_bucket(s)]
    }

    #[inline]
    pub fn strings_index_next(&self, s: i32) -> i32 {
        let hv = &self.host.hv.strings;
        hv.index_prev[(s - hv.index_base) as usize]
    }

    /// §45
    pub fn str_eq_buf(&mut self, s: i32, k: i32) -> bool {
        let mut k = k;
        let mut j = self.str_start[s];
        while j < self.str_start[s + 1] {
            if self.str_pool.at(j) != self.buffer.at(k) {
                return false;
            }
            j += 1;
            k += 1;
        }
        true
    }

    /// §46
    pub fn str_eq_str(&mut self, s: i32, t: i32) -> bool {
        if self.length(s) != self.length(t) {
            return false;
        }
        let mut j = self.str_start[s];
        let mut k = self.str_start[t];
        while j < self.str_start[s + 1] {
            if self.str_pool.at(j) != self.str_pool.at(k) {
                return false;
            }
            j += 1;
            k += 1;
        }
        true
    }

    /// §47
    /// Makes strings 0-255, then the well-known strings S_*; no pool file.
    pub fn get_strings_started(&mut self) -> bool {
        self.pool_ptr = 0;
        self.str_ptr = 0;
        self.str_start[0] = 0;
        // §48 Make the first 256 strings
        for k in 0..=255 {
            // §49 Character |k| cannot be printed
            if k < b' ' as i32 || k > b'~' as i32 {
                self.append_char(b'^' as i32);
                self.append_char(b'^' as i32);
                if k < 0o100 {
                    self.append_char(k + 0o100);
                } else if k < 0o200 {
                    self.append_char(k - 0o100);
                } else {
                    self.strings_app_lc_hex(k / 16);
                    self.strings_app_lc_hex(k % 16);
                }
            } else {
                self.append_char(k);
            }
            let _ = self.make_string();
        }
        // §51 Read the other strings from the TEX.POOL file and return true, or give an error
        // message and return false.  `loadpoolstrings`, of web2c's generated pool file, in its
        // strings' order; the test is its `i>=spare_size`, on the bytes taken so far.
        let spare_size = self.pool_size - self.string_vacancies;
        let mut i = 0;
        let mut g = 0;
        for &s in WELL_KNOWN_STRINGS {
            i += s.len() as i32;
            if i >= spare_size {
                g = 0;
                break;
            }
            for &c in s {
                self.append_char(c as i32);
            }
            g = self.make_string();
        }
        if g == 0 {
            self.wake_up_terminal();
            self.wterm_ln_s(b"! You have to increase POOLSIZE.");
            return false;
        }
        true
    }

    /// §48 `app_lc_hex`.  WEB's `l` is a local of get_strings_started that this does without.
    fn strings_app_lc_hex(&mut self, l: i32) {
        if l < 10 {
            self.append_char(l + b'0' as i32);
        } else {
            self.append_char(l - 10 + b'a' as i32);
        }
    }
}
