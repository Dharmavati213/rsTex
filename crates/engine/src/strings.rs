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
#[derive(Default)]
pub struct HVars {}

impl Tex {
    /// §43
    pub fn make_string(&mut self) -> i32 {
        if self.str_ptr == self.max_strings {
            self.overflow(b"number of strings", self.max_strings - self.init_str_ptr);
        }
        self.str_ptr += 1;
        self.str_start[self.str_ptr] = self.pool_ptr;
        self.str_ptr - 1
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
