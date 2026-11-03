//! A file in rstex's cache directory that can be trusted without reading all of it: `index.rs` and the engine's font map tables.
//!
//! ```text
//! magic, the sum of all that follows up to the body, total size, length of the front, key, where the body is    all u64
//! the front             checked when the file is opened: it is small, and all of it gets used
//! sums                  one u32 for each 4 KiB of the body
//! the body, on a page boundary
//! ```
//!
//! The body is checked 4 KiB at a time, when first touched, so a run pays for what it reads, and a mapped file costs only those pages.  A failed sum is
//! [`Corrupt`], and nothing that did not pass comes out.  The caller then discards the file and makes what it held again.  Sums, not cryptography: they are
//! against a damaged file, not against an attacker, who could write anything else in the same directory as well.
//!
//! A file is published by temporary and rename, [`publish_cache`](crate::publish_cache), and it is never changed in place, so a run that has it mapped keeps what it mapped.
use crate::sys::FileBytes;
use std::cell::Cell;

const MAGIC: u64 = u64::from_le_bytes(*b"RSTXCCH1");
const HEADER: usize = 6 * 8;
const CHUNK: usize = 4096;

/// A sum did not match.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Corrupt;

/// Word-at-a-time multiplicative hash.  A difference in one word stays a difference to the end, since each step is one-to-one.
#[inline]
pub fn hash64(s: &[u8]) -> u64 {
    const K: u64 = 0x517c_c1b7_2722_0a95;
    let mut h: u64 = s.len() as u64;
    let mut chunks = s.chunks_exact(8);
    for c in &mut chunks {
        let w = u64::from_le_bytes([c[0], c[1], c[2], c[3], c[4], c[5], c[6], c[7]]);
        h = (h.rotate_left(5) ^ w).wrapping_mul(K);
    }
    let mut w = 0u64;
    for (i, &b) in chunks.remainder().iter().enumerate() {
        w |= (b as u64) << (i * 8);
    }
    h = (h.rotate_left(5) ^ w).wrapping_mul(K);
    h ^ (h >> 29)
}

fn u32_at(b: &[u8], i: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(i..i.checked_add(4)?)?.try_into().ok()?))
}

fn u64_at(b: &[u8], i: usize) -> Option<u64> {
    Some(u64::from_le_bytes(b.get(i..i.checked_add(8)?)?.try_into().ok()?))
}

/// The bytes of a file holding `front` and `body`, for `key`.
pub fn make_cached_file(key: u64, front: &[u8], body: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(HEADER + front.len() + body.len() / 512 + CHUNK + body.len());
    out.resize(HEADER, 0);
    out.extend_from_slice(front);
    for c in body.chunks(CHUNK) {
        out.extend_from_slice(&(hash64(c) as u32).to_le_bytes());
    }
    let start = out.len().next_multiple_of(CHUNK);
    out.resize(start, 0);
    let total = start + body.len();
    for (i, x) in [MAGIC, 0, total as u64, front.len() as u64, key, start as u64].iter().enumerate() {
        out[i * 8..i * 8 + 8].copy_from_slice(&x.to_le_bytes());
    }
    let sum = hash64(&out[16..start]);
    out[8..16].copy_from_slice(&sum.to_le_bytes());
    out.extend_from_slice(body);
    out
}

/// See the module's documentation.
pub struct CachedFile {
    file: FileBytes,
    front_len: usize,
    body: usize,
    /// One bit for each 4 KiB of the body: its sum has been checked.
    checked: Box<[Cell<u64>]>,
}

impl CachedFile {
    /// The file at `path`, if it opens and is sound as far as it can be told at once.
    pub fn open(path: &std::path::Path, key: u64) -> Option<CachedFile> {
        CachedFile::new(FileBytes::open(path)?, key)
    }

    /// Bytes from [`make_cached_file`].
    pub fn from_vec(bytes: Vec<u8>, key: u64) -> Option<CachedFile> {
        CachedFile::new(FileBytes::from_vec(bytes), key)
    }

    fn new(file: FileBytes, key: u64) -> Option<CachedFile> {
        let b = file.bytes();
        if u64_at(b, 0)? != MAGIC || u64_at(b, 16)? != b.len() as u64 || u64_at(b, 32)? != key {
            return None;
        }
        let (front_len, body) = (u64_at(b, 24)? as usize, u64_at(b, 40)? as usize);
        if body > b.len() || body < HEADER || hash64(&b[16..body]) != u64_at(b, 8)? {
            return None;
        }
        let chunks = (b.len() - body).div_ceil(CHUNK);
        if HEADER.checked_add(front_len)?.checked_add(chunks * 4)? > body {
            return None;
        }
        let checked = vec![Cell::new(0); chunks.div_ceil(64)].into_boxed_slice();
        Some(CachedFile { front_len, body, checked, file })
    }

    /// The part checked when the file was opened.
    #[inline]
    pub fn front(&self) -> &[u8] {
        &self.file.bytes()[HEADER..HEADER + self.front_len]
    }

    /// `at..at+len` of the body, with its sums checked.
    pub fn body(&self, at: usize, len: usize) -> Result<&[u8], Corrupt> {
        let b = self.file.bytes();
        let start = self.body.checked_add(at).ok_or(Corrupt)?;
        let end = start.checked_add(len).filter(|&e| e <= b.len()).ok_or(Corrupt)?;
        if len > 0 {
            for c in at / CHUNK..=(at + len - 1) / CHUNK {
                let (word, bit) = (&self.checked[c / 64], 1u64 << (c % 64));
                if word.get() & bit == 0 {
                    let from = self.body + c * CHUNK;
                    let chunk = &b[from..(from + CHUNK).min(b.len())];
                    if u32_at(b, HEADER + self.front_len + c * 4) != Some(hash64(chunk) as u32) {
                        return Err(Corrupt);
                    }
                    word.set(word.get() | bit);
                }
            }
        }
        Ok(&b[start..end])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(body_len: usize) -> (Vec<u8>, Vec<u8>) {
        let body: Vec<u8> = (0..body_len).map(|i| (i * 7 % 251) as u8).collect();
        (make_cached_file(9, b"front part", &body), body)
    }

    #[test]
    fn what_goes_in_comes_out() {
        for n in [0, 1, 100, 4095, 4096, 4097, 100_000, 3_000_000] {
            let (f, body) = sample(n);
            let c = CachedFile::from_vec(f, 9).unwrap();
            assert_eq!(c.front(), b"front part");
            assert_eq!(c.body(0, n).unwrap(), &body[..]);
            assert_eq!(c.body(n / 2, n - n / 2).unwrap(), &body[n / 2..]);
            assert_eq!(c.body(n, 1), Err(Corrupt));
        }
    }

    #[test]
    fn another_key_and_any_damage_are_refused() {
        let (f, _) = sample(10_000);
        assert!(CachedFile::from_vec(f.clone(), 8).is_none());
        assert!(CachedFile::from_vec(f[..f.len() - 1].to_vec(), 9).is_none(), "cut short");
        assert!(CachedFile::from_vec(Vec::new(), 9).is_none());
        // One bit anywhere: in the header or the front it fails at once, and in the body when that page is asked for.
        for at in (0..f.len()).step_by(97) {
            let mut g = f.clone();
            g[at] ^= 0x10;
            if let Some(c) = CachedFile::from_vec(g, 9) {
                assert!(at >= 4096, "{at}: the first page is the header, the front and the sums");
                assert_eq!(c.body(0, 10_000), Err(Corrupt), "{at}");
                let page = (at - 4096) / 4096;
                assert!(c.body(page * 4096, 1).is_err(), "{at}");
            }
        }
    }

    #[test]
    fn only_what_is_asked_for_is_checked() {
        let (mut f, _) = sample(20_000);
        let last = f.len() - 1;
        f[last] ^= 1;
        let c = CachedFile::from_vec(f, 9).unwrap();
        assert!(c.body(0, 4096).is_ok());
        assert!(c.body(4096 * 4, 1).is_err());
    }
}
