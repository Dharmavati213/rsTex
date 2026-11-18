//! What `search::Known` has learned of the directories of trees on the disk, kept in rstex's cache directory for the next run.
//!
//! A personal tree has no database that counts, so every run that looks in it reads each of its directories: 200 of them in a `~/texmf` with a few packages and
//! fonts, at five system calls each and 8000 names to take in, which was a fifth of a run that restores a checkpoint.  A directory's list is good while its
//! stamp stands, in the next run as in this one, so the lists are kept, and a run then makes one `stat` for each directory it comes to and reads none of them.
//!
//! Only what can be trusted from one run to another is kept:
//!
//! - a directory named from the root, since another run has another current directory;
//! - one whose stamp is an hour old or more.  A stamp that young belongs to a directory someone is working in, which would be written here after every run;
//! - one on a file system known to change a directory's times when an entry comes or goes: [`sys::fs_keeps_dir_stamps`].  Within a run a list that goes
//!   stale is the same risk `ListedDir` always took.  Kept for months, a list on a file system that leaves the times alone would hide a new file for good, where C,
//!   which asks for every name, finds it.  For a directory elsewhere the file says only that it is elsewhere, so that the question is not asked in every run.
//!
//! The file is a [`CachedFile`], replaced whole by temporary and rename when a run has something to add, and never otherwise.  A record that does not check is
//! as good as none: the directory is read.
//!
//! ```text
//! front   the layout's name
//! body    count; for each record the hash of its path and where it is, by hash; the records
//! record  path length, entry count, text length, flags; stamp; a spare word; path; the end of each name; the kind of each; the names; the hashes, in order, unless a name is odd
//! ```
use crate::cachefile::{CachedFile, hash64, make_cached_file};
use crate::search::Listing;
use crate::sys::{self, Stamp};

const LAYOUT: &[u8] = b"kpse dirs 1";
/// The file's name in rstex's cache directory.
pub(crate) const FILE: &str = "kpse-dirs-1";
/// A record's fixed part: four u32, the stamp, a word to spare.
const FIXED: usize = 16 + 40 + 8;
/// The directory is on a file system whose stamps are not known to be kept: nothing else is recorded.
const ELSEWHERE: u32 = 2;
const ODD: u32 = 1;
/// Younger than this, in nanoseconds, and a stamp is not kept by.
pub(crate) const SETTLED_NS: u64 = 3600 * 1_000_000_000;
/// The file does not grow past this: what a run did not use goes first.
const LIMIT: usize = 16 << 20;

fn key() -> u64 {
    hash64(LAYOUT)
}

fn u32_at(b: &[u8], i: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(i..i.checked_add(4)?)?.try_into().ok()?))
}

fn u64_at(b: &[u8], i: usize) -> Option<u64> {
    Some(u64::from_le_bytes(b.get(i..i.checked_add(8)?)?.try_into().ok()?))
}

/// What the file says of one directory.
#[derive(Clone, Copy)]
pub(crate) struct Stored {
    at: usize,
    len: usize,
    pub(crate) stamp: Stamp,
    pub(crate) elsewhere: bool,
}

pub(crate) struct Store {
    file: CachedFile,
    count: usize,
}

impl Store {
    pub(crate) fn open(path: &std::path::Path) -> Option<Store> {
        let file = CachedFile::open(path, key())?;
        if file.front() != LAYOUT {
            return None;
        }
        let count = u64_at(file.body(0, 8).ok()?, 0)? as usize;
        // The table is there, as far as its length goes.
        file.body(8, count.checked_mul(16)?).ok()?;
        Some(Store { file, count })
    }

    fn slot(&self, i: usize) -> Option<(u64, usize)> {
        let b = self.file.body(8 + i * 16, 16).ok()?;
        Some((u64_at(b, 0)?, u64_at(b, 8)? as usize))
    }

    /// The record at `at`, and how long it is.
    fn record(&self, at: usize) -> Option<(Stored, &[u8])> {
        let b = self.file.body(at, FIXED).ok()?;
        let (path_len, n, text_len, flags) = (u32_at(b, 0)? as usize, u32_at(b, 4)? as usize, u32_at(b, 8)? as usize, u32_at(b, 12)?);
        let mut stamp = [0; 5];
        for (k, s) in stamp.iter_mut().enumerate() {
            *s = u64_at(b, 16 + k * 8)?;
        }
        let folds = if flags & (ODD | ELSEWHERE) != 0 { 0 } else { n };
        let len = FIXED.checked_add(path_len)?.checked_add(n.checked_mul(5)?)?.checked_add(text_len)?.checked_add(folds.checked_mul(8)?)?;
        let path = self.file.body(at.checked_add(FIXED)?, path_len).ok()?;
        Some((Stored { at, len, stamp, elsewhere: flags & ELSEWHERE != 0 }, path))
    }

    /// What is recorded of the directory `path`, which is as `search::stat_path` gives it.
    pub(crate) fn find(&self, path: &[u8]) -> Option<Stored> {
        let h = hash64(path);
        let (mut lo, mut hi) = (0, self.count);
        while lo < hi {
            let mid = (lo + hi) / 2;
            if self.slot(mid)?.0 < h { lo = mid + 1 } else { hi = mid }
        }
        while lo < self.count {
            let (hash, at) = self.slot(lo)?;
            if hash != h {
                break;
            }
            let (stored, p) = self.record(at)?;
            if p == path {
                return Some(stored);
            }
            lo += 1;
        }
        None
    }

    /// The list of a record, checked as far as a list can be: a damaged one must not answer for a directory.
    pub(crate) fn listing(&self, s: &Stored) -> Option<Listing> {
        let b = self.file.body(s.at, s.len).ok()?;
        let (path_len, n, text_len, flags) = (u32_at(b, 0)? as usize, u32_at(b, 4)? as usize, u32_at(b, 8)? as usize, u32_at(b, 12)?);
        if flags & ELSEWHERE != 0 {
            return None;
        }
        let odd = flags & ODD != 0;
        let mut at = FIXED + path_len;
        let mut ends = Vec::with_capacity(n);
        let mut last = 0;
        for k in 0..n {
            let e = u32_at(b, at + k * 4)?;
            if e < last || e as usize > text_len {
                return None;
            }
            last = e;
            ends.push(e);
        }
        if n > 0 && last as usize != text_len {
            return None;
        }
        at += n * 4;
        let kinds = b.get(at..at + n)?.to_vec();
        at += n;
        let text = b.get(at..at + text_len)?.to_vec();
        at += text_len;
        let mut folds = Vec::with_capacity(if odd { 0 } else { n });
        if !odd {
            for k in 0..n {
                folds.push(u64_at(b, at + k * 8)?);
            }
            if folds.windows(2).any(|w| w[0] > w[1]) {
                return None;
            }
        }
        Some(Listing { text, ends, kinds, folds, odd })
    }

    /// Every record: its path, and its bytes as they are.
    fn records(&self) -> Vec<(&[u8], &[u8])> {
        let mut out = Vec::with_capacity(self.count);
        for i in 0..self.count {
            let Some((_, at)) = self.slot(i) else { break };
            let Some((s, path)) = self.record(at) else { continue };
            if let Ok(bytes) = self.file.body(s.at, s.len) {
                out.push((path, bytes));
            }
        }
        out
    }
}

/// A record's bytes.  `list` is `None` for a directory that is elsewhere.
pub(crate) fn record(path: &[u8], stamp: &Stamp, list: Option<&Listing>) -> Vec<u8> {
    let empty = Listing::default();
    let (l, mut flags) = match list {
        Some(l) => (l, 0),
        None => (&empty, ELSEWHERE),
    };
    if l.odd {
        flags |= ODD;
    }
    let mut out = Vec::with_capacity(FIXED + path.len() + l.ends.len() * 13 + l.text.len());
    for x in [path.len() as u32, l.ends.len() as u32, l.text.len() as u32, flags] {
        out.extend_from_slice(&x.to_le_bytes());
    }
    // The last is room for what a later layout may want to know.
    for x in stamp.iter().chain([&0]) {
        out.extend_from_slice(&x.to_le_bytes());
    }
    out.extend_from_slice(path);
    for e in &l.ends {
        out.extend_from_slice(&e.to_le_bytes());
    }
    out.extend_from_slice(&l.kinds);
    out.extend_from_slice(&l.text);
    if !l.odd && list.is_some() {
        for f in &l.folds {
            out.extend_from_slice(&f.to_le_bytes());
        }
    }
    out
}

/// Writes the file anew: `fresh` as given, by path, and of `old` what `drop` does not name, as long as there is room.
pub(crate) fn publish(to: &std::path::Path, fresh: &[(Vec<u8>, Vec<u8>)], old: Option<&Store>, drop: &dyn Fn(&[u8]) -> bool) {
    let mut all: Vec<(u64, &[u8])> = Vec::new();
    let mut size = 0;
    for (path, bytes) in fresh {
        if size + bytes.len() > LIMIT {
            break;
        }
        size += bytes.len();
        all.push((hash64(path), bytes));
    }
    let kept = old.map_or(Vec::new(), |o| o.records());
    for (path, bytes) in &kept {
        if drop(path) || size + bytes.len() > LIMIT {
            continue;
        }
        size += bytes.len();
        all.push((hash64(path), bytes));
    }
    all.sort_by_key(|r| r.0);
    let mut body = Vec::with_capacity(8 + all.len() * 16 + size);
    body.extend_from_slice(&(all.len() as u64).to_le_bytes());
    let mut at = 8 + all.len() * 16;
    for (h, bytes) in &all {
        body.extend_from_slice(&h.to_le_bytes());
        body.extend_from_slice(&(at as u64).to_le_bytes());
        at += bytes.len();
    }
    for (_, bytes) in &all {
        body.extend_from_slice(bytes);
    }
    sys::publish_path(to, &make_cached_file(key(), LAYOUT, &body));
}

/// Whether a list of the directory at `path` may be kept from one run to another, by where it is: see the top of this file.
pub(crate) fn place_is_trusted(path: &[u8]) -> bool {
    sys::fs_keeps_dir_stamps(path)
}
