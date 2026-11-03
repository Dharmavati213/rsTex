//! §274-289 Part 18: the hash table.
//!
//! Not literal: DESIGN section 4 and the notes on `hash.rs` in docs/PORTING.md.  `id_lookup` searches
//! `hash_tab` by open addressing with linear probing.  An entry is `(hash32 << 32) | p`, 0 when empty;
//! `p` is never 0.  There is no `next`.  A new control sequence takes the next `eqtb` location above
//! `EQTB_SIZE`.  Nothing is ever deleted, and the table doubles at half full, rehashing from the
//! stored `hash32` without touching a name.  `id_lookup` answers as WEB's does; only the location it
//! gives differs, and that shows nowhere but in statistics.
//!
//! `prim_lookup` keeps its chained table as written.
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

/// `hash_tab_size` at the first use, in slots.  A format holds the size it grew to.
const HASH_TAB_FIRST_SIZE: i32 = 1 << 14;

// Every location of `eqtb` could be a control sequence and the table would still be at most half
// full, so it never has to grow past its capacity and a search always ends at an empty slot.
const _: () = assert!(EQTB_CAP * 2 <= HASH_TAB_CAP);
const _: () = assert!(HASH_TAB_CAP.is_power_of_two() && HASH_TAB_FIRST_SIZE as usize <= HASH_TAB_CAP);

#[inline(always)]
fn hash_mix(x: u64) -> u64 {
    let m = x as u128 * 0x9E37_79B9_7F4A_7C15_u128;
    m as u64 ^ (m >> 64) as u64
}

/// §280 Compute the hash code |h|: a word at a time, with the last word overlapping the one before.
#[inline(always)]
pub fn hash_name(s: &[u8]) -> u32 {
    let mut h = s.len() as u64;
    let last = if let Some(w) = s.last_chunk::<8>() {
        let mut r = s;
        while let Some((x, rest)) = r.split_first_chunk::<8>() {
            if rest.is_empty() {
                break;
            }
            h = hash_mix(h ^ u64::from_le_bytes(*x));
            r = rest;
        }
        u64::from_le_bytes(*w)
    } else if let (Some(a), Some(b)) = (s.first_chunk::<4>(), s.last_chunk::<4>()) {
        u32::from_le_bytes(*a) as u64 | (u32::from_le_bytes(*b) as u64) << 32
    } else if let (Some(&a), Some(&b)) = (s.first(), s.last()) {
        // one to three bytes
        a as u64 | (s[s.len() / 2] as u64) << 8 | (b as u64) << 16
    } else {
        0
    };
    hash_mix(h ^ last) as u32
}

/// `a=b`, for what is nearly always a match: a word at a time, without the call of `==` on slices,
/// which would make `id_lookup` save registers on every search.
#[inline(always)]
fn hash_same_name(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    if let (Some(la), Some(lb)) = (a.last_chunk::<8>(), b.last_chunk::<8>()) {
        for (x, y) in a.chunks_exact(8).zip(b.chunks_exact(8)) {
            let (x, y): ([u8; 8], [u8; 8]) = (x.try_into().unwrap(), y.try_into().unwrap());
            if u64::from_le_bytes(x) != u64::from_le_bytes(y) {
                return false;
            }
        }
        u64::from_le_bytes(*la) == u64::from_le_bytes(*lb)
    } else if let (Some(fa), Some(fb), Some(la), Some(lb)) = (a.first_chunk::<4>(), b.first_chunk::<4>(), a.last_chunk::<4>(), b.last_chunk::<4>()) {
        u32::from_le_bytes(*fa) == u32::from_le_bytes(*fb) && u32::from_le_bytes(*la) == u32::from_le_bytes(*lb)
    } else {
        // at most three bytes
        a.iter().zip(b).all(|(x, y)| x == y)
    }
}

impl Tex {
    /// §278
    /// Open addressing in hash_tab; run-time control sequences are sequential above EQTB_SIZE.
    pub fn id_lookup(&mut self, j: i32, l: i32) -> i32 {
        if self.hash_tab_size == 0 {
            self.hash_tab_size = HASH_TAB_FIRST_SIZE;
        }
        #[cfg(feature = "commute-probe")]
        {
            let copy = self.buffer.range(j, j + l).to_vec();
            self.probe_lookup(&copy);
        }
        let name = self.buffer.range(j, j + l);
        let h = hash_name(name);
        let mask = self.hash_tab_size - 1;
        let mut i = h as i32 & mask;
        loop {
            let e = self.hash_tab[i];
            if e == 0 {
                break;
            }
            if (e >> 32) as u32 == h {
                let p = e as u32 as i32;
                let s = self.text(p);
                if hash_same_name(self.str_pool.range(self.str_start[s], self.str_start[s + 1]), name) {
                    return p;
                }
            }
            i = (i + 1) & mask;
        }
        if self.no_new_control_sequence {
            return UNDEFINED_CONTROL_SEQUENCE;
        }
        self.hash_new_cs(j, l, h)
    }

    /// A control sequence by its name, `undefined_control_sequence` if the table has no such name: `id_lookup` that can create nothing and needs no `buffer`.
    pub fn find_cs(&self, name: &[u8]) -> i32 {
        match name.len() {
            0 => return NULL_CS,
            1 => return SINGLE_BASE + name[0] as i32,
            _ => {}
        }
        if self.hash_tab_size == 0 {
            return UNDEFINED_CONTROL_SEQUENCE;
        }
        let h = hash_name(name);
        let mask = self.hash_tab_size - 1;
        let mut i = h as i32 & mask;
        loop {
            let e = self.hash_tab[i];
            if e == 0 {
                return UNDEFINED_CONTROL_SEQUENCE;
            }
            if (e >> 32) as u32 == h {
                let p = e as u32 as i32;
                let s = self.text(p);
                if hash_same_name(self.str_pool.range(self.str_start[s], self.str_start[s + 1]), name) {
                    return p;
                }
            }
            i = (i + 1) & mask;
        }
    }

    /// §279 Insert a new control sequence, and return where it went.
    ///
    /// `id_lookup` runs while a string is in progress, since `scan_file_name` calls `get_x_token`
    /// between `more_name` calls.  So the move of the current string is as written.
    #[cold]
    #[inline(never)]
    fn hash_new_cs(&mut self, j: i32, l: i32, h: u32) -> i32 {
        let p = EQTB_SIZE + 1 + self.hash_high;
        if p > self.eqtb_top {
            self.overflow(b"hash size", HASH_SIZE + self.hash_extra);
        }
        self.hash_high += 1;
        // §240's value: region 2 stops at EQTB_SIZE, so nothing else has set this entry.
        self.eqtb[p] = Word::hb(NULL, UNDEFINED_CS, LEVEL_ZERO);
        self.hash_tab_insert(h, p);
        self.str_room(l);
        let d = self.cur_length();
        while self.pool_ptr > self.str_start[self.str_ptr] {
            self.pool_ptr -= 1;
            let c = self.str_pool.at(self.pool_ptr);
            self.str_pool.put(self.pool_ptr + l, c);
        } // move current string up to make room for another
        for k in j..j + l {
            let c = self.buffer.at(k);
            self.append_char(c);
        }
        let s = self.make_string();
        self.set_text(p, s);
        self.pool_ptr += d;
        self.cs_count += 1;
        p
    }

    /// Enters `p` under `h`, doubling the table first if that would make it more than half full.
    fn hash_tab_insert(&mut self, h: u32, p: i32) {
        if 2 * (self.hash_tab_used + 1) > self.hash_tab_size {
            self.hash_tab_grow();
        }
        let mask = self.hash_tab_size - 1;
        let mut i = h as i32 & mask;
        while self.hash_tab[i] != 0 {
            i = (i + 1) & mask;
        }
        self.hash_tab[i] = (h as u64) << 32 | p as u64;
        self.hash_tab_used += 1;
    }

    /// Doubles the table.  It takes a few milliseconds at most and happens a few times per format
    /// build, so a temporary copy of the entries is fine.
    #[cold]
    #[inline(never)]
    pub(crate) fn hash_tab_grow(&mut self) {
        let old = self.hash_tab_size;
        let entries: Vec<u64> = self.hash_tab.range(0, old).iter().copied().filter(|&e| e != 0).collect();
        self.hash_tab.range_mut(0, old).fill(0);
        self.hash_tab_size = old * 2;
        let mask = self.hash_tab_size - 1;
        for e in entries {
            let mut i = (e >> 32) as i32 & mask;
            while self.hash_tab[i] != 0 {
                i = (i + 1) & mask;
            }
            self.hash_tab[i] = e;
        }
    }

    /// §281
    pub fn prim_lookup(&mut self, s: i32) -> i32 {
        let mut p;
        let mut l = 0; // WEB leaves it unset for a single character; no multi-letter name has length 0
        'found: {
            if s <= BIGGEST_CHAR {
                if s < 0 {
                    p = UNDEFINED_PRIMITIVE;
                    break 'found;
                } else {
                    p = (s % PRIM_PRIME) + PRIM_BASE; // we start searching here
                }
            } else {
                let j = self.str_start[s];
                if s == self.str_ptr {
                    l = self.cur_length();
                } else {
                    l = self.length(s);
                }
                // §283 Compute the primitive code |h|
                let mut h = self.str_pool.at(j);
                for k in j + 1..j + l {
                    h = h + h + self.str_pool.at(k);
                    while h >= PRIM_PRIME {
                        h -= PRIM_PRIME;
                    }
                }
                p = h + PRIM_BASE; // we start searching here; note that 0<=h<prim_prime
            }
            loop {
                if self.prim_text(p) > 1 + BIGGEST_CHAR {
                    // p points a multi-letter primitive
                    if self.length(self.prim_text(p) - 1) == l && self.str_eq_str(self.prim_text(p) - 1, s) {
                        break 'found;
                    }
                } else if self.prim_text(p) == 1 + s {
                    break 'found; // p points a single-letter primitive
                }
                if self.prim_next(p) == 0 {
                    if self.no_new_control_sequence {
                        p = UNDEFINED_PRIMITIVE;
                    } else {
                        // §282 Insert a new primitive after |p|, then make |p| point to it
                        if self.prim_text(p) > 0 {
                            loop {
                                if self.prim_used == PRIM_BASE {
                                    self.overflow(b"primitive size", PRIM_SIZE);
                                }
                                self.prim_used -= 1;
                                if self.prim_text(self.prim_used) == 0 {
                                    break; // search for an empty location in prim
                                }
                            }
                            self.set_prim_next(p, self.prim_used);
                            p = self.prim_used;
                        }
                        self.set_prim_text(p, s + 1);
                    }
                    break 'found;
                }
                p = self.prim_next(p);
            }
        }
        p
    }

    /// §284
    pub fn print_cs(&mut self, p: i32) {
        if p < HASH_BASE {
            // single character
            if p >= SINGLE_BASE {
                if p == NULL_CS {
                    self.print_esc_s(b"csname");
                    self.print_esc_s(b"endcsname");
                    self.print_char(b' ' as i32);
                } else {
                    self.print_esc(p - SINGLE_BASE);
                    if self.cat_code(p - SINGLE_BASE) == LETTER {
                        self.print_char(b' ' as i32);
                    }
                }
            } else if p < ACTIVE_BASE {
                self.print_esc_s(b"IMPOSSIBLE.");
            } else {
                self.print(p - ACTIVE_BASE);
            }
        } else if (p >= UNDEFINED_CONTROL_SEQUENCE && p <= EQTB_SIZE) || p > self.eqtb_top {
            self.print_esc_s(b"IMPOSSIBLE.");
        } else if self.text(p) >= self.str_ptr {
            self.print_esc_s(b"NONEXISTENT.");
        } else {
            if p >= PRIM_EQTB_BASE && p < FROZEN_NULL_FONT {
                self.print_esc(self.prim_text(p - PRIM_EQTB_BASE) - 1);
            } else {
                self.print_esc(self.text(p));
            }
            self.print_char(b' ' as i32);
        }
    }

    /// §285
    pub fn sprint_cs(&mut self, p: i32) {
        if p < HASH_BASE {
            if p < SINGLE_BASE {
                self.print(p - ACTIVE_BASE);
            } else if p < NULL_CS {
                self.print_esc(p - SINGLE_BASE);
            } else {
                self.print_esc_s(b"csname");
                self.print_esc_s(b"endcsname");
            }
        } else if p >= PRIM_EQTB_BASE && p < FROZEN_NULL_FONT {
            self.print_esc(self.prim_text(p - PRIM_EQTB_BASE) - 1);
        } else {
            self.print_esc(self.text(p));
        }
    }

    /// §286
    /// s is the name.  A one-byte name is a single character: cur_val=s[0]+SINGLE_BASE.
    ///
    /// WEB's `flush_string; text(cur_val):=s` throws away the copy `id_lookup` made, in favour of the
    /// one in the pool file.  There is no pool file here, so that copy is the name.
    pub fn primitive(&mut self, s: &[u8], c: i32, o: i32) {
        let prim_val;
        if let [ch] = *s {
            self.cur_val = ch as i32 + SINGLE_BASE;
            prim_val = self.prim_lookup(ch as i32);
        } else {
            let l = s.len() as i32;
            // we will move s into the (possibly non-empty) buffer
            if self.first + l > self.buf_size + 1 {
                self.overflow(b"buffer size", self.buf_size);
            }
            for (k, &ch) in s.iter().enumerate() {
                self.buffer.put(self.first + k as i32, ch as i32);
            }
            self.cur_val = self.id_lookup(self.first, l); // no_new_control_sequence is false
            prim_val = self.prim_lookup(self.text(self.cur_val));
        }
        self.set_eq_level(self.cur_val, LEVEL_ONE);
        self.set_eq_type(self.cur_val, c);
        self.set_equiv(self.cur_val, o);
        self.set_prim_eq_level(prim_val, LEVEL_ONE);
        self.set_prim_eq_type(prim_val, c);
        self.set_prim_equiv(prim_val, o);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::save::testkit::*;
    use crate::toks::testkit::*;
    use std::collections::HashMap;

    /// Enters a name without `make_string`: the string is made by hand, and the entry goes through the same
    /// `hash_tab_insert`.
    fn enter_by_hand(t: &mut Tex, name: &[u8]) -> i32 {
        if t.hash_tab_size == 0 {
            t.hash_tab_size = HASH_TAB_FIRST_SIZE;
        }
        let p = EQTB_SIZE + 1 + t.hash_high;
        t.hash_high += 1;
        for &c in name {
            t.str_pool.put(t.pool_ptr, c as i32);
            t.pool_ptr += 1;
        }
        t.set_text(p, t.str_ptr);
        t.str_ptr += 1;
        t.str_start[t.str_ptr] = t.pool_ptr;
        t.hash_tab_insert(hash_name(name), p);
        p
    }

    fn lookup(t: &mut Tex, name: &[u8]) -> i32 {
        for (i, &c) in name.iter().enumerate() {
            t.buffer.put(t.first + i as i32, c as i32);
        }
        t.id_lookup(t.first, name.len() as i32)
    }

    /// The name's string, as stored.
    fn name_of(t: &Tex, p: i32) -> Vec<u8> {
        let s = t.text(p);
        t.str_pool.range(t.str_start[s], t.str_start[s + 1]).to_vec()
    }

    fn occupied(t: &Tex) -> i32 {
        t.hash_tab.range(0, t.hash_tab_size).iter().filter(|&&e| e != 0).count() as i32
    }

    #[test]
    fn the_hash_sees_every_byte_and_the_length() {
        for l in 0..64usize {
            let base: Vec<u8> = (0..l).map(|i| b'a' + (i % 26) as u8).collect();
            let h = hash_name(&base);
            for i in 0..l {
                let mut other = base.clone();
                other[i] ^= 1;
                assert_ne!(hash_name(&other), h, "byte {} of {}", i, l);
            }
            let mut longer = base.clone();
            longer.push(0);
            assert_ne!(hash_name(&longer), h, "length {}", l);
        }
    }

    #[test]
    fn names_compare_by_every_byte() {
        for l in 0..64usize {
            let a: Vec<u8> = (0..l).map(|i| (i * 7) as u8).collect();
            assert!(hash_same_name(&a, &a.clone()));
            for i in 0..l {
                let mut b = a.clone();
                b[i] ^= 0x80;
                assert!(!hash_same_name(&a, &b), "byte {} of {}", i, l);
            }
            let mut c = a.clone();
            c.push(0);
            assert!(!hash_same_name(&a, &c), "length {}", l);
            assert!(!hash_same_name(&c, &a), "length {}", l);
        }
    }

    /// Names as packages make them.  A poor hash shows in the probes, whatever the load.
    fn realistic_name(k: u32) -> Vec<u8> {
        match k % 4 {
            0 => format!("c@section{}", k).into_bytes(),
            1 => format!("__kernel_tl_{}:Nn", k).into_bytes(),
            2 => format!("r@sec:{}@cref", k).into_bytes(),
            _ => format!("pgfmath@{}", k).into_bytes(),
        }
    }

    #[test]
    fn the_hash_spreads_names() {
        let n = 100_000u32;
        let mut seen = HashMap::new();
        let mut collisions = 0;
        for k in 0..n {
            let name = realistic_name(k);
            if seen.insert(hash_name(&name), k).is_some() {
                collisions += 1;
            }
        }
        // 100000 random 32-bit values have about 1.2 equal pairs.
        assert!(collisions < 10, "{} names share a 32-bit hash", collisions);
        let t = machine();
        let mut probes = 0u64;
        t.hash_tab_size = 1 << 18;
        for k in 0..n {
            let name = realistic_name(k);
            let h = hash_name(&name);
            let mask = t.hash_tab_size - 1;
            let mut i = h as i32 & mask;
            probes += 1;
            while t.hash_tab[i] != 0 {
                i = (i + 1) & mask;
                probes += 1;
            }
            t.hash_tab[i] = (h as u64) << 32 | 1;
        }
        // Linear probing at load 0.38 averages about 1.3 probes per insertion, over the whole filling.
        assert!((probes as f64 / n as f64) < 1.4, "{} probes per name", probes as f64 / n as f64);
        Tex::dealloc(t);
    }

    #[test]
    fn no_new_control_sequence_answers_undefined_and_changes_nothing() {
        let t = machine();
        assert!(t.no_new_control_sequence);
        assert_eq!(lookup(t, b"nothinghere"), UNDEFINED_CONTROL_SEQUENCE, "on the empty table");
        assert_eq!(t.hash_tab_size, HASH_TAB_FIRST_SIZE);
        let p = enter_by_hand(t, b"here");
        let before = (t.hash_high, t.cs_count, t.str_ptr, t.pool_ptr, t.hash_tab_used, t.hash_tab_size);
        assert_eq!(lookup(t, b"nothinghere"), UNDEFINED_CONTROL_SEQUENCE);
        assert_eq!(lookup(t, b"her"), UNDEFINED_CONTROL_SEQUENCE);
        assert_eq!(lookup(t, b"heree"), UNDEFINED_CONTROL_SEQUENCE);
        assert_eq!(lookup(t, b"here"), p);
        assert_eq!(before, (t.hash_high, t.cs_count, t.str_ptr, t.pool_ptr, t.hash_tab_used, t.hash_tab_size));
        Tex::dealloc(t);
    }

    #[test]
    fn names_of_every_short_length_are_found() {
        let t = machine();
        let names: Vec<Vec<u8>> = (0..40).map(|l| (0..l).map(|i| b'A' + i as u8).collect()).collect();
        let ps: Vec<i32> = names.iter().map(|n| enter_by_hand(t, n)).collect();
        for (n, &p) in names.iter().zip(&ps) {
            assert_eq!(lookup(t, n), p, "{:?}", String::from_utf8_lossy(n));
        }
        Tex::dealloc(t);
    }

    /// Bytes of all values, since `\csname` makes names of anything.
    #[test]
    fn names_of_any_bytes_are_found() {
        let t = machine();
        let names: Vec<Vec<u8>> = (0..=255u8).map(|b| vec![b, 255 - b, b, 0, 255]).collect();
        let ps: Vec<i32> = names.iter().map(|n| enter_by_hand(t, n)).collect();
        for (n, &p) in names.iter().zip(&ps) {
            assert_eq!(lookup(t, n), p);
        }
        Tex::dealloc(t);
    }

    /// Two names with one 32-bit hash: only the comparison of the names tells them apart.
    #[test]
    fn a_full_collision_is_told_apart_by_the_name() {
        let mut seen: HashMap<u32, Vec<u8>> = HashMap::new();
        let mut pair = None;
        for k in 0u32.. {
            let name = format!("collide{}", k).into_bytes();
            if let Some(other) = seen.insert(hash_name(&name), name.clone()) {
                pair = Some((other, name));
                break;
            }
        }
        let (a, b) = pair.unwrap();
        assert_ne!(a, b);
        assert_eq!(hash_name(&a), hash_name(&b));
        let t = machine();
        let pa = enter_by_hand(t, &a);
        assert_eq!(lookup(t, &b), UNDEFINED_CONTROL_SEQUENCE, "the hash is equal, the name is not");
        let pb = enter_by_hand(t, &b);
        assert_ne!(pa, pb);
        assert_eq!(lookup(t, &a), pa);
        assert_eq!(lookup(t, &b), pb);
        Tex::dealloc(t);
    }

    /// Names that start at the last slot run on through the end of the table to its start.
    #[test]
    fn collisions_probe_and_wrap_around() {
        let t = machine();
        t.hash_tab_size = 8;
        let mut names = Vec::new();
        for k in 0u32.. {
            let name = format!("n{}", k).into_bytes();
            if hash_name(&name) & 7 == 7 {
                names.push(name);
                if names.len() == 4 {
                    break;
                }
            }
        }
        let p0 = enter_by_hand(t, &names[0]);
        let p1 = enter_by_hand(t, &names[1]);
        let p2 = enter_by_hand(t, &names[2]);
        assert_eq!(t.hash_tab_size, 8, "three of eight is under half full");
        assert_eq!(t.hash_tab[7] as u32 as i32, p0);
        assert_eq!(t.hash_tab[0] as u32 as i32, p1, "wrapped");
        assert_eq!(t.hash_tab[1] as u32 as i32, p2);
        assert_eq!(lookup(t, &names[0]), p0);
        assert_eq!(lookup(t, &names[1]), p1);
        assert_eq!(lookup(t, &names[2]), p2);
        assert_eq!(lookup(t, &names[3]), UNDEFINED_CONTROL_SEQUENCE, "the search ends at the empty slot");
        Tex::dealloc(t);
    }

    #[test]
    fn growth_by_rehash_keeps_every_entry_at_most_half_full() {
        let t = machine();
        let n = 40000;
        let mut sizes = vec![];
        let mut ps = Vec::with_capacity(n);
        for k in 0..n as u32 {
            ps.push(enter_by_hand(t, &realistic_name(k)));
            if sizes.last() != Some(&t.hash_tab_size) {
                sizes.push(t.hash_tab_size);
            }
            assert!(2 * t.hash_tab_used <= t.hash_tab_size, "over half full at {}", k);
        }
        assert_eq!(sizes, vec![1 << 14, 1 << 15, 1 << 16, 1 << 17]);
        assert_eq!(t.hash_tab_used, n as i32);
        assert_eq!(occupied(t), n as i32, "no entry lost or doubled");
        for k in 0..n as u32 {
            assert_eq!(lookup(t, &realistic_name(k)), ps[k as usize]);
            assert_eq!(name_of(t, ps[k as usize]), realistic_name(k));
        }
        Tex::dealloc(t);
    }

    /// An entry keeps its hash through a rehash, so growing touches no name.
    #[test]
    fn growth_reads_no_name() {
        let t = machine();
        t.hash_tab_size = 16;
        let mut hs = Vec::new();
        for p in 1..=8 {
            let h = 0x1234_5678u32.wrapping_mul(p as u32);
            hs.push((h, p));
            t.hash_tab_insert(h, p);
        }
        assert_eq!(t.hash_tab_size, 16);
        t.hash_tab_grow();
        assert_eq!(t.hash_tab_size, 32);
        assert_eq!(occupied(t), 8);
        for (h, p) in hs {
            let mut i = h as i32 & 31;
            while t.hash_tab[i] != ((h as u64) << 32 | p as u64) {
                assert_ne!(t.hash_tab[i], 0, "the probe from entry {}'s home reaches it", p);
                i = (i + 1) & 31;
            }
        }
        Tex::dealloc(t);
    }

    #[test]
    fn a_single_character_primitive_needs_no_string() {
        let t = table_machine();
        t.no_new_control_sequence = false;
        t.prim_used = PRIM_SIZE;
        t.primitive(b"/", ITAL_CORR, 7);
        assert_eq!(t.cur_val, SINGLE_BASE + b'/' as i32);
        assert_eq!((t.eq_level(t.cur_val), t.eq_type(t.cur_val), t.equiv(t.cur_val)), (LEVEL_ONE, ITAL_CORR, 7));
        t.no_new_control_sequence = true;
        let v = t.prim_lookup(b'/' as i32);
        assert_eq!(v, b'/' as i32 % PRIM_PRIME + PRIM_BASE);
        assert_eq!((t.prim_eq_level(v), t.prim_eq_type(v), t.prim_equiv(v)), (LEVEL_ONE, ITAL_CORR, 7));
        assert_eq!(t.prim_text(v), b'/' as i32 + 1);
        Tex::dealloc(t);
    }

    #[test]
    fn prim_lookup_of_what_is_not_there() {
        let t = machine();
        t.prim_used = PRIM_SIZE;
        assert!(t.no_new_control_sequence);
        assert_eq!(t.prim_lookup(b'x' as i32), UNDEFINED_PRIMITIVE);
        assert_eq!(t.prim_lookup(-1), UNDEFINED_PRIMITIVE, "s<0");
        t.no_new_control_sequence = false;
        assert_eq!(t.prim_lookup(-1), UNDEFINED_PRIMITIVE, "s<0, even when entering");
        Tex::dealloc(t);
    }

    /// Not a gate: a rough figure, to compare with the 101 ns per lookup of the oracle in DESIGN section
    /// 1, for 57k names.  20 M hits among 57k names in `buffer`, in a shuffled order.  This file may not
    /// use std::time; whoever runs it times the process:
    /// `cargo test --release -p rstex-engine --lib --features timing -- --exact hash::tests::lookup_speed`.
    #[cfg(feature = "timing")]
    #[test]
    fn lookup_speed() {
        let t = machine();
        let n = 57000u32;
        let mut at = Vec::new();
        for k in 0..n {
            let name = realistic_name(k);
            enter_by_hand(t, &name);
            at.push((t.first, name.len() as i32));
            for &c in &name {
                t.buffer.put(t.first, c as i32);
                t.first += 1;
            }
        }
        let mut sum = 0i64;
        let mut i = 0usize;
        for _ in 0..20_000_000 {
            i = (i + 12347) % n as usize;
            let (j, l) = at[i];
            sum += t.id_lookup(j, l) as i64;
        }
        assert!(sum != 0);
        Tex::dealloc(t);
    }

    // ---------------------------------------------------------------------------------------------
    // These enter a name the real way, which ends in make_string.
    // ---------------------------------------------------------------------------------------------

    fn entering_machine() -> &'static mut Tex {
        let t = table_machine();
        print_setup(t);
        t.prim_used = PRIM_SIZE;
        t.hash_extra = 600000;
        t.no_new_control_sequence = false;
        t
    }

    #[test]
    fn a_new_control_sequence_is_undefined_and_next_above_eqtb_size() {
        let t = entering_machine();
        let (strs, pool) = (t.str_ptr, t.pool_ptr);
        let p = lookup(t, b"brandnew");
        assert_eq!(p, EQTB_SIZE + 1);
        assert_eq!(t.hash_high, 1);
        assert_eq!(t.cs_count, 1);
        assert_eq!(t.hash_tab_used, 1);
        assert_eq!((t.eq_type(p), t.equiv(p), t.eq_level(p)), (UNDEFINED_CS, NULL, LEVEL_ZERO));
        assert_eq!(name_of(t, p), b"brandnew");
        assert_eq!((t.str_ptr, t.pool_ptr), (strs + 1, pool + 8));
        assert_eq!(lookup(t, b"brandnew"), p, "found, not entered again");
        assert_eq!((t.str_ptr, t.hash_high, t.cs_count), (strs + 1, 1, 1));
        assert_eq!(lookup(t, b"another"), EQTB_SIZE + 2);
        t.no_new_control_sequence = true;
        assert_eq!(lookup(t, b"third"), UNDEFINED_CONTROL_SEQUENCE);
        assert_eq!(t.hash_high, 2);
        Tex::dealloc(t);
    }

    /// `\input t8\csname brandnewname\endcsname` opens t8 and creates \brandnewname, on the oracle:
    /// `scan_file_name` has `t8` in progress when `\csname` calls `id_lookup`.
    #[test]
    fn a_string_in_progress_survives_a_new_control_sequence() {
        let t = entering_machine();
        t.append_char(b't' as i32);
        t.append_char(b'8' as i32);
        let s = t.str_ptr;
        let p = lookup(t, b"brandnewname");
        assert_eq!(t.str_ptr, s + 1, "the name is the older string");
        assert_eq!(t.text(p), s);
        assert_eq!(name_of(t, p), b"brandnewname");
        assert_eq!(t.cur_length(), 2, "the file name is still in progress");
        assert_eq!(t.str_pool.at(t.str_start[t.str_ptr]), b't' as i32);
        assert_eq!(t.str_pool.at(t.str_start[t.str_ptr] + 1), b'8' as i32);
        t.append_char(b'x' as i32);
        let f = t.make_string();
        assert_eq!(name_of_string(t, f), b"t8x");
        assert_eq!(name_of(t, p), b"brandnewname", "untouched");
        Tex::dealloc(t);
    }

    fn name_of_string(t: &Tex, s: i32) -> Vec<u8> {
        t.str_pool.range(t.str_start[s], t.str_start[s + 1]).to_vec()
    }

    #[test]
    fn growth_while_entering() {
        let t = entering_machine();
        t.pool_size = 10_000_000;
        t.max_strings = 1_000_000;
        let n = 70000u32;
        let ps: Vec<i32> = (0..n).map(|k| lookup(t, &realistic_name(k))).collect();
        assert_eq!(t.hash_high, n as i32);
        assert_eq!(t.cs_count, n as i32);
        assert_eq!(t.hash_tab_size, 1 << 18);
        t.no_new_control_sequence = true;
        for k in 0..n {
            assert_eq!(lookup(t, &realistic_name(k)), ps[k as usize]);
        }
        assert_eq!(ps[0], EQTB_SIZE + 1);
        assert_eq!(*ps.last().unwrap(), EQTB_SIZE + n as i32);
        Tex::dealloc(t);
    }

    #[test]
    fn the_hash_is_full_when_eqtb_is() {
        let t = entering_machine();
        t.eqtb_top = EQTB_SIZE + 2;
        t.hash_extra = 2;
        assert_eq!(lookup(t, b"first"), EQTB_SIZE + 1);
        assert_eq!(lookup(t, b"second"), EQTB_SIZE + 2);
        assert_eq!(lookup(t, b"first"), EQTB_SIZE + 1, "a name that exists needs no room");
        assert_overflow(t, |t| { lookup(t, b"third"); }, "hash size", 2);
    }

    #[test]
    fn prim_lookup_chains_collisions_as_written() {
        let t = entering_machine();
        // 2*97+98 and 2*98+96 have one primitive code.
        let a = t.lit_string(b"ab");
        let b = t.lit_string(b"b`");
        let c = t.lit_string(b"zz");
        let pa = t.prim_lookup(a);
        assert_eq!(pa, 292 + PRIM_BASE);
        let pb = t.prim_lookup(b);
        assert_eq!(pb, PRIM_SIZE - 1, "the first empty location below prim_used");
        assert_eq!(t.prim_next(pa), pb);
        assert_eq!(t.prim_used, PRIM_SIZE - 1);
        assert_eq!(t.prim_text(pb), b + 1);
        assert_eq!(t.prim_lookup(a), pa);
        assert_eq!(t.prim_lookup(b), pb);
        t.no_new_control_sequence = true;
        assert_eq!(t.prim_lookup(c), UNDEFINED_PRIMITIVE);
        assert_eq!(t.prim_lookup(a), pa);
        // A single character and a multi-letter name with its code: 2*1+63 is the home of "A".
        t.no_new_control_sequence = false;
        let m = t.lit_string(&[1, 63]);
        let single = t.prim_lookup(b'A' as i32);
        let multi = t.prim_lookup(m);
        assert_eq!(single, b'A' as i32 % PRIM_PRIME + PRIM_BASE);
        assert_eq!(t.prim_next(single), multi);
        assert_eq!(t.prim_lookup(b'A' as i32), single);
        assert_eq!(t.prim_lookup(m), multi);
        Tex::dealloc(t);
    }

    #[test]
    fn the_primitive_table_can_fill_up() {
        let t = entering_machine();
        let (a, b) = (t.lit_string(b"ab"), t.lit_string(b"b`"));
        t.prim_lookup(a);
        t.prim_used = PRIM_BASE;
        assert_overflow(t, |t| { t.prim_lookup(b); }, "primitive size", PRIM_SIZE);
    }

    /// Every primitive there is: tests/primitives.json is the inventory the oracle was checked against.
    fn primitive_names() -> Vec<Vec<u8>> {
        let json = include_str!("../../../tests/primitives.json");
        let names: Vec<Vec<u8>> = json
            .lines()
            .filter_map(|l| l.strip_prefix("\"name\": \"").and_then(|l| l.strip_suffix("\",")))
            .map(|n| n.as_bytes().to_vec())
            .collect();
        assert_eq!(names.len(), 568);
        names
    }

    /// The table's part of `every_primitive_can_be_entered_and_found`, which needs only the table.
    #[test]
    fn every_primitive_name_is_found_in_the_table() {
        let t = machine();
        let names = primitive_names();
        for name in &names {
            assert_eq!(lookup(t, name), UNDEFINED_CONTROL_SEQUENCE);
        }
        let ps: Vec<i32> = names.iter().map(|n| enter_by_hand(t, n)).collect();
        for (name, &p) in names.iter().zip(&ps) {
            assert_eq!(lookup(t, name), p, "{:?}", String::from_utf8_lossy(name));
        }
        assert_eq!(t.hash_tab_used, 568);
        Tex::dealloc(t);
    }

    #[test]
    fn every_primitive_can_be_entered_and_found() {
        let t = entering_machine();
        let names = primitive_names();
        let mut seen = Vec::new();
        for (k, name) in names.iter().enumerate() {
            let c = (k % 100) as i32 + 1;
            t.primitive(name, c, k as i32 + 1000);
            seen.push(t.cur_val);
        }
        assert_eq!(t.hash_high as usize, names.iter().filter(|n| n.len() > 1).count());
        assert_eq!(t.cs_count, t.hash_high);
        t.no_new_control_sequence = true;
        for (k, name) in names.iter().enumerate() {
            let (c, o) = ((k % 100) as i32 + 1, k as i32 + 1000);
            let p = seen[k];
            let s;
            if let [ch] = name[..] {
                assert_eq!(p, ch as i32 + SINGLE_BASE);
                s = ch as i32;
            } else {
                assert_eq!(lookup(t, name), p, "{:?}", String::from_utf8_lossy(name));
                assert_eq!(&name_of(t, p), name);
                s = t.text(p);
            }
            assert_eq!((t.eq_level(p), t.eq_type(p), t.equiv(p)), (LEVEL_ONE, c, o));
            let v = t.prim_lookup(s);
            assert_ne!(v, UNDEFINED_PRIMITIVE, "{:?}", String::from_utf8_lossy(name));
            assert_eq!((t.prim_eq_level(v), t.prim_eq_type(v), t.prim_equiv(v)), (LEVEL_ONE, c, o));
        }
        Tex::dealloc(t);
    }

    #[test]
    fn print_cs_against_the_oracle() {
        for r in table(include_str!("../testdata/hash_print_cs_oracle.tsv")) {
            let t = entering_machine();
            match r[0].as_str() {
                "active" => t.print_cs(ACTIVE_BASE + b'~' as i32),
                "letter" => t.print_cs(SINGLE_BASE + b'a' as i32),
                "nonletter" => t.print_cs(SINGLE_BASE + b'.' as i32),
                "unprintable" => t.print_cs(SINGLE_BASE + 13),
                "nullcs" => t.print_cs(NULL_CS),
                "multi" => {
                    let p = lookup(t, b"foo");
                    t.print_cs(p);
                }
                "prim" => {
                    t.set_text(FROZEN_PRIMITIVE, S_PDFPRIMITIVE);
                    t.print_cs(FROZEN_PRIMITIVE);
                    t.primitive(b"relax", RELAX, 256);
                    let s = t.text(t.cur_val);
                    let v = t.prim_lookup(s);
                    t.print_cs(PRIM_EQTB_BASE + v);
                }
                k => panic!("{}", k),
            }
            assert_eq!(printed(t), r[1], "{}", r[0]);
            Tex::dealloc(t);
        }
    }

    #[test]
    fn sprint_cs_never_adds_a_space() {
        let t = entering_machine();
        let p = lookup(t, b"foo");
        t.primitive(b"relax", RELAX, 256);
        let s = t.text(t.cur_val);
        let v = t.prim_lookup(s);
        for (q, want) in [
            (p, "\\foo"),
            (SINGLE_BASE + b'a' as i32, "\\a"),
            (ACTIVE_BASE + b'~' as i32, "~"),
            (NULL_CS, "\\csname\\endcsname"),
            (PRIM_EQTB_BASE + v, "\\relax"),
        ] {
            t.sprint_cs(q);
            assert_eq!(printed(t), want);
        }
        Tex::dealloc(t);
    }

    /// `print_cs` "might be invoked with invalid data, so it is extra robust".
    #[test]
    fn print_cs_of_what_is_no_control_sequence() {
        let t = entering_machine();
        for (q, want) in [
            (ACTIVE_BASE - 1, "\\IMPOSSIBLE."),
            (-5, "\\IMPOSSIBLE."),
            (UNDEFINED_CONTROL_SEQUENCE, "\\IMPOSSIBLE."),
            (EQTB_SIZE, "\\IMPOSSIBLE."),
            (t.eqtb_top + 1, "\\IMPOSSIBLE."),
        ] {
            t.print_cs(q);
            assert_eq!(printed(t), want, "{}", q);
        }
        t.set_text(FROZEN_NULL_FONT, t.str_ptr);
        t.print_cs(FROZEN_NULL_FONT);
        assert_eq!(printed(t), "\\NONEXISTENT.");
        t.set_text(FROZEN_PROTECTION, S_INACCESSIBLE);
        t.print_cs(FROZEN_PROTECTION);
        assert_eq!(printed(t), "\\inaccessible ");
        Tex::dealloc(t);
    }
}
