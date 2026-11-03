//! The binary index: what the cnf files and the `ls-R` databases say, kept in rstex's cache directory so that a run reads neither.
//!
//! It holds the table of `texmf.cnf` and the table of the databases, flat, and how they came about: each environment variable that was asked for and
//! its answer, and the [`Stamp`](crate::sys::Stamp) of each path that was looked at, whether it was there or not.  A run with the same answers
//! would read the same files and get these same tables, so it takes them from here, by mapping the file: no parsing, and only the pages a lookup touches.  Any
//! other answer and the index is stale: it is made again, and replaces the old one by temporary and rename.  The file name says for which program and tree it is.
//!
//! A run that cannot use the file makes the same bytes in memory, so that a lookup has one way to go.
//!
//! The file is a [`CachedFile`]: the front, up to the databases, is checked when it is opened, and the databases 4 KiB at a time when first touched.  A failed sum
//! means the caller discards the file, makes the tables again, and asks again, so no answer ever comes from bytes that were not checked.
//!
//! ```text
//! in the front:
//! a word for whether the databases are usable, and one for where each part is, all u64
//! environment                          name and value of each variable asked for
//! stamps                               path and stamp of each path looked at
//! recorded, warnings                   what a run reports having read and having to say
//! top directories, the `ls-R` files, aliases, the search paths of the cnf files and the databases
//! the search paths of the formats a document asks for, each worked out from the variables once    `INDEXED_FORMATS`
//! the cnf table                        open addressing over (key, value) records
//! in the body:
//! buckets                              the first entry of each bucket; the next bucket says where it ends
//! entries                              hash, name offset, name length, directory number; in a bucket, in the order read
//! directories                          offsets and text
//! names                                text
//! buckets and lists by the lower case  where [`crate::Config::db_casefold`] asks for them: each list has the entries of a name in any case, by number, in the order read
//! ```
use crate::cachefile::{CachedFile, Corrupt, hash64, make_cached_file};
use crate::db::Db;
use crate::format;
use crate::sys::{self, Stamp};
use crate::{DbState, FormatState, Kpse};
use std::collections::HashMap;
use std::sync::Arc;

/// The header, in u64s.
const H_USABLE: usize = 0;
const H_ENVS: usize = 1;
const H_STAMPS: usize = 2;
const H_RECORDED: usize = 3;
const H_WARNINGS: usize = 4;
const H_DIR_LIST: usize = 5;
const H_ALIASES: usize = 6;
const H_PATHS: usize = 7;
const H_CNF: usize = 8;
const H_BUCKETS: usize = 9;
const H_BUCKET_COUNT: usize = 10;
const H_ENTRIES: usize = 11;
const H_ENTRY_COUNT: usize = 12;
const H_DIR_OFFS: usize = 13;
const H_DIR_COUNT: usize = 14;
const H_DIR_TEXT: usize = 15;
const H_NAMES: usize = 16;
const H_DB_FILES: usize = 17;
const H_FOLD_BUCKETS: usize = 18;
const H_FOLD_LISTS: usize = 19;
const H_FORMATS: usize = 20;
const HEADER_WORDS: usize = 21;
const ENTRY_SIZE: usize = 16;
/// Changes when the layout of this file does.
const INDEX_LAYOUT: &str = "index 4";
/// What nearly every document looks in: working out a search path is some 100000 instructions, a tenth of a one-page document for the handful it needs.
const INDEXED_FORMATS: [i32; 8] = [
    format::KPSE_TEX_FORMAT,
    format::KPSE_TFM_FORMAT,
    format::KPSE_VF_FORMAT,
    format::KPSE_FONTMAP_FORMAT,
    format::KPSE_TYPE1_FORMAT,
    format::KPSE_ENC_FORMAT,
    format::KPSE_PICT_FORMAT,
    format::KPSE_PROGRAM_TEXT_FORMAT,
];
/// No value for a variable that was not set, among the lengths of `environment`.
const NOT_SET: u32 = u32::MAX;

#[inline]
pub(crate) fn hash_name(s: &[u8]) -> u32 {
    (hash64(s) >> 32) as u32
}

fn u32_at(b: &[u8], i: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(i..i.checked_add(4)?)?.try_into().ok()?))
}

fn u64_at(b: &[u8], i: usize) -> Option<u64> {
    Some(u64::from_le_bytes(b.get(i..i.checked_add(8)?)?.try_into().ok()?))
}

fn put_u32(out: &mut Vec<u8>, x: usize) {
    out.extend_from_slice(&(x as u32).to_le_bytes());
}

/// `count`, then each as a length and its bytes.
fn put_list<T: AsRef<[u8]>>(out: &mut Vec<u8>, list: &[T]) {
    put_u32(out, list.len());
    for x in list {
        put_u32(out, x.as_ref().len());
        out.extend_from_slice(x.as_ref());
    }
}

/// What `put_list` wrote at `at`.
fn get_list(b: &[u8], at: usize) -> Option<Vec<&[u8]>> {
    let n = u32_at(b, at)? as usize;
    let mut p = at + 4;
    let mut out = Vec::with_capacity(n);
    for _ in 0..n {
        let len = u32_at(b, p)? as usize;
        out.push(b.get(p + 4..p + 4 + len)?);
        p += 4 + len;
    }
    Some(out)
}

/// The tables and how they came about.
pub(crate) struct Facts<'a> {
    pub key: u64,
    pub envs: &'a [(Vec<u8>, Option<Vec<u8>>)],
    pub stamps: &'a [(Vec<u8>, Stamp)],
    pub recorded: &'a [Vec<u8>],
    pub warnings: &'a [Vec<u8>],
    pub dir_list: &'a [Vec<u8>],
    pub db_files: &'a [Vec<u8>],
    pub aliases: &'a [Vec<u8>],
    pub cnf_path: &'a [u8],
    pub db_path: &'a [u8],
    /// The program they are for, then a format's code and search path, for each of `INDEXED_FORMATS`.
    pub format_paths: &'a [Vec<u8>],
    pub cnf: &'a HashMap<Vec<u8>, Vec<u8>>,
    pub usable: bool,
}

/// The part of the file after the front, and where its parts are: (buckets, bucket count, entries, entry count, directory offsets, directory count, directory text, names, buckets
/// by the lower case, their lists).
pub(crate) struct Body {
    bytes: Vec<u8>,
    at: [usize; 10],
}

/// Everything from `buckets` on, from a [`Db`] that has read its files.  With `folded`, also what makes a lookup that ignores case cheap.
pub(crate) fn make_body(db: &Db, folded: bool) -> Body {
    let count = db.entries.len();
    let cap = (count * 2).next_power_of_two().max(16);
    let mask = cap as u32 - 1;
    // Each bucket's size, then its start; entries go in in the order read, so a bucket keeps that order.
    let mut starts = vec![0u32; cap + 1];
    for e in &db.entries {
        starts[(e.hash & mask) as usize + 1] += 1;
    }
    for i in 0..cap {
        starts[i + 1] += starts[i];
    }
    let mut next = starts.clone();
    let mut sorted = vec![[0u32; 4]; count];
    let mut names = Vec::with_capacity(db.text.len());
    // Each entry's bucket by the lower case, and its number among the sorted ones, in the order read.
    let mut fold = Vec::with_capacity(if folded { count } else { 0 });
    for e in &db.entries {
        let slot = &mut next[(e.hash & mask) as usize];
        sorted[*slot as usize] = [e.hash, names.len() as u32, e.len, e.dir];
        if folded {
            fold.push((hash_name(&db.text[e.off as usize..(e.off + e.len) as usize].to_ascii_lowercase()) & mask, *slot));
        }
        *slot += 1;
        names.extend_from_slice(&db.text[e.off as usize..(e.off + e.len) as usize]);
    }
    let mut bytes = Vec::with_capacity(4 * cap + ENTRY_SIZE * count + names.len() + 4 * db.dirs.len());
    let buckets = bytes.len();
    for x in &starts {
        bytes.extend_from_slice(&x.to_le_bytes());
    }
    let entries = bytes.len();
    for e in &sorted {
        for x in e {
            bytes.extend_from_slice(&x.to_le_bytes());
        }
    }
    let dir_offs = bytes.len();
    let mut off = 0;
    for d in &db.dirs {
        put_u32(&mut bytes, off);
        off += d.len();
    }
    put_u32(&mut bytes, off);
    let dir_text = bytes.len();
    for d in &db.dirs {
        bytes.extend_from_slice(d);
    }
    let names_at = bytes.len();
    bytes.extend_from_slice(&names);
    let (mut fold_buckets, mut fold_lists) = (0, 0);
    if folded {
        let mut starts = vec![0u32; cap + 1];
        for (b, _) in &fold {
            starts[*b as usize + 1] += 1;
        }
        for i in 0..cap {
            starts[i + 1] += starts[i];
        }
        let mut next = starts.clone();
        let mut lists = vec![0u32; count];
        for (b, slot) in &fold {
            lists[next[*b as usize] as usize] = *slot;
            next[*b as usize] += 1;
        }
        fold_buckets = bytes.len();
        for x in &starts {
            bytes.extend_from_slice(&x.to_le_bytes());
        }
        fold_lists = bytes.len();
        for x in &lists {
            bytes.extend_from_slice(&x.to_le_bytes());
        }
    }
    Body { bytes, at: [buckets, cap, entries, count, dir_offs, db.dirs.len(), dir_text, names_at, fold_buckets, fold_lists] }
}

/// The whole file.
pub(crate) fn make_file(f: &Facts<'_>, body: &Body) -> Vec<u8> {
    let mut out = vec![0u8; HEADER_WORDS * 8];
    let mut h = [0u64; HEADER_WORDS];
    h[H_USABLE] = f.usable as u64;
    h[H_ENVS] = out.len() as u64;
    put_u32(&mut out, f.envs.len());
    for (name, value) in f.envs {
        put_u32(&mut out, name.len());
        match value {
            Some(v) => put_u32(&mut out, v.len()),
            None => out.extend_from_slice(&NOT_SET.to_le_bytes()),
        }
        out.extend_from_slice(name);
        if let Some(v) = value {
            out.extend_from_slice(v);
        }
    }
    h[H_STAMPS] = out.len() as u64;
    put_u32(&mut out, f.stamps.len());
    for (path, st) in f.stamps {
        put_u32(&mut out, path.len());
        for x in st {
            out.extend_from_slice(&x.to_le_bytes());
        }
        out.extend_from_slice(path);
    }
    h[H_RECORDED] = out.len() as u64;
    put_list(&mut out, f.recorded);
    h[H_WARNINGS] = out.len() as u64;
    put_list(&mut out, f.warnings);
    h[H_DIR_LIST] = out.len() as u64;
    put_list(&mut out, f.dir_list);
    h[H_DB_FILES] = out.len() as u64;
    put_list(&mut out, f.db_files);
    h[H_ALIASES] = out.len() as u64;
    put_list(&mut out, f.aliases);
    h[H_PATHS] = out.len() as u64;
    put_list(&mut out, &[f.cnf_path, f.db_path]);
    h[H_FORMATS] = out.len() as u64;
    put_list(&mut out, f.format_paths);
    // The cnf table, in a fixed order, so that one state of the world gives one file.
    h[H_CNF] = out.len() as u64;
    let mut keys: Vec<&Vec<u8>> = f.cnf.keys().collect();
    keys.sort();
    let cap = (keys.len() * 2).next_power_of_two().max(16);
    let mut slots = vec![0u32; cap];
    for (i, k) in keys.iter().enumerate() {
        let mut s = hash_name(k) as usize & (cap - 1);
        while slots[s] != 0 {
            s = (s + 1) & (cap - 1);
        }
        slots[s] = i as u32 + 1;
    }
    put_u32(&mut out, cap);
    for s in &slots {
        out.extend_from_slice(&s.to_le_bytes());
    }
    let mut off = 0;
    for k in &keys {
        let v = &f.cnf[*k];
        put_u32(&mut out, off);
        put_u32(&mut out, k.len());
        put_u32(&mut out, off + k.len());
        put_u32(&mut out, v.len());
        off += k.len() + v.len();
    }
    for k in &keys {
        out.extend_from_slice(k);
        out.extend_from_slice(&f.cnf[*k]);
    }
    for (i, slot) in [H_BUCKETS, H_BUCKET_COUNT, H_ENTRIES, H_ENTRY_COUNT, H_DIR_OFFS, H_DIR_COUNT, H_DIR_TEXT, H_NAMES, H_FOLD_BUCKETS, H_FOLD_LISTS].iter().enumerate() {
        h[*slot] = body.at[i] as u64;
    }
    for (i, x) in h.iter().enumerate() {
        out[i * 8..i * 8 + 8].copy_from_slice(&x.to_le_bytes());
    }
    make_cached_file(f.key, &out, &body.bytes)
}

/// The tables, in a file or in memory.
pub(crate) struct Index {
    file: CachedFile,
    pub(crate) usable: bool,
    h: [usize; HEADER_WORDS],
    cnf_cap: usize,
    cnf_slots: usize,
    cnf_recs: usize,
    cnf_text: usize,
}

impl Index {
    /// `file`, if its parts are where it says.
    pub(crate) fn new(file: CachedFile) -> Option<Index> {
        let b = file.front();
        let mut h = [0usize; HEADER_WORDS];
        for (i, x) in h.iter_mut().enumerate() {
            *x = u64_at(b, i * 8)? as usize;
        }
        let cnf = h[H_CNF];
        let cnf_cap = u32_at(b, cnf)? as usize;
        if !cnf_cap.is_power_of_two() {
            return None;
        }
        let cnf_slots = cnf + 4;
        let cnf_recs = cnf_slots + cnf_cap * 4;
        // As many records as the slots hold, and the text follows.
        let n = b.get(cnf_slots..cnf_recs)?.chunks_exact(4).filter(|s| *s != [0; 4]).count();
        let cnf_text = cnf_recs + n * 16;
        let usable = h[H_USABLE] != 0;
        Some(Index { usable, cnf_cap, cnf_slots, cnf_recs, cnf_text, h, file })
    }

    fn front(&self) -> &[u8] {
        self.file.front()
    }

    fn checked(&self, at: usize, len: usize) -> Result<&[u8], Corrupt> {
        self.file.body(at, len)
    }

    /// The search path of `format` under `program`, if it was worked out when this file was made: all that goes into one is in `environment` and the cnf table.
    pub(crate) fn format_path(&self, program: &[u8], format: i32) -> Option<&[u8]> {
        let list = self.list(H_FORMATS);
        let (&prog, rest) = list.split_first()?;
        if prog != program {
            return None;
        }
        rest.chunks_exact(2).find(|p| p[0] == format.to_le_bytes()).map(|p| p[1])
    }

    /// The value of a key of the cnf table.
    pub(crate) fn cnf(&self, key: &[u8]) -> Option<&[u8]> {
        let b = self.front();
        let mask = self.cnf_cap - 1;
        let mut s = hash_name(key) as usize & mask;
        loop {
            let e = u32_at(b, self.cnf_slots + s * 4)? as usize;
            if e == 0 {
                return None;
            }
            let r = self.cnf_recs + (e - 1) * 16;
            let (k, klen) = (u32_at(b, r)? as usize, u32_at(b, r + 4)? as usize);
            if b.get(self.cnf_text + k..self.cnf_text + k + klen)? == key {
                let (v, vlen) = (u32_at(b, r + 8)? as usize, u32_at(b, r + 12)? as usize);
                return b.get(self.cnf_text + v..self.cnf_text + v + vlen);
            }
            s = (s + 1) & mask;
        }
    }

    /// `hash_lookup`: the numbers of the directories holding `name`, in the order read.
    pub(crate) fn dirs_of(&self, name: &[u8], out: &mut Vec<u32>) -> Result<(), Corrupt> {
        let cap = self.h[H_BUCKET_COUNT];
        let hash = hash_name(name);
        let b = self.checked(self.h[H_BUCKETS] + (hash as usize & (cap - 1)) * 4, 8)?;
        let (first, end) = (u32_at(b, 0).ok_or(Corrupt)? as usize, u32_at(b, 4).ok_or(Corrupt)? as usize);
        if first > end || end > self.h[H_ENTRY_COUNT] {
            return Err(Corrupt);
        }
        let entries = self.checked(self.h[H_ENTRIES] + first * ENTRY_SIZE, (end - first) * ENTRY_SIZE)?;
        for e in entries.chunks_exact(ENTRY_SIZE) {
            if u32_at(e, 0) == Some(hash) {
                let (off, len, dir) = (u32_at(e, 4).ok_or(Corrupt)?, u32_at(e, 8).ok_or(Corrupt)?, u32_at(e, 12).ok_or(Corrupt)?);
                if self.checked(self.h[H_NAMES] + off as usize, len as usize)? == name {
                    out.push(dir);
                }
            }
        }
        Ok(())
    }

    /// Directory number `id`, ending in a separator.
    pub(crate) fn dir(&self, id: u32) -> Result<&[u8], Corrupt> {
        let o = self.checked(self.h[H_DIR_OFFS] + id as usize * 4, 8)?;
        let (a, z) = (u32_at(o, 0).ok_or(Corrupt)? as usize, u32_at(o, 4).ok_or(Corrupt)? as usize);
        if a > z {
            return Err(Corrupt);
        }
        self.checked(self.h[H_DIR_TEXT] + a, z - a)
    }

    /// What the databases hold of `name` in any case: (directory number, the name as it is there), in the order read.  `name` is in lower case.  Nothing if the file was
    /// made without `folded`, and then `make_body` has left the place at 0, which is the buckets.
    pub(crate) fn folded_entries(&self, name: &[u8], out: &mut Vec<(u32, Vec<u8>)>) -> Result<(), Corrupt> {
        if self.h[H_FOLD_LISTS] == 0 {
            return Ok(());
        }
        let cap = self.h[H_BUCKET_COUNT];
        let b = self.checked(self.h[H_FOLD_BUCKETS] + (hash_name(name) as usize & (cap - 1)) * 4, 8)?;
        let (first, end) = (u32_at(b, 0).ok_or(Corrupt)? as usize, u32_at(b, 4).ok_or(Corrupt)? as usize);
        if first > end || end > self.h[H_ENTRY_COUNT] {
            return Err(Corrupt);
        }
        for slot in self.checked(self.h[H_FOLD_LISTS] + first * 4, (end - first) * 4)?.chunks_exact(4) {
            let slot = u32_at(slot, 0).ok_or(Corrupt)? as usize;
            let e = self.checked(self.h[H_ENTRIES] + slot * ENTRY_SIZE, ENTRY_SIZE)?;
            let (off, len, dir) = (u32_at(e, 4).ok_or(Corrupt)?, u32_at(e, 8).ok_or(Corrupt)?, u32_at(e, 12).ok_or(Corrupt)?);
            let actual = self.checked(self.h[H_NAMES] + off as usize, len as usize)?;
            if actual.eq_ignore_ascii_case(name) {
                out.push((dir, actual.to_vec()));
            }
        }
        Ok(())
    }

    /// `elt_in_db` over all databases: whether to try the db for a lookup in this element.
    pub(crate) fn relevant(&self, path_elt: &[u8]) -> bool {
        get_list(self.front(), self.h[H_DIR_LIST]).is_some_and(|l| l.iter().any(|d| crate::db::elt_in_db(d, path_elt)))
    }

    /// See [`Kpse::in_database_tree`].
    pub(crate) fn in_database_tree(&self, path: &[u8]) -> bool {
        get_list(self.front(), self.h[H_DIR_LIST]).is_some_and(|l| l.iter().any(|d| path.starts_with(d)))
    }

    /// See [`Kpse::database_files`].
    pub(crate) fn database_files(&self) -> Vec<Vec<u8>> {
        self.list(H_DB_FILES).iter().map(|f| f.to_vec()).collect()
    }

    /// The other stored lists, which a hit gives to `Kpse`.
    fn list(&self, which: usize) -> Vec<&[u8]> {
        get_list(self.front(), self.h[which]).unwrap_or_default()
    }

    /// True if each variable has its answer and each path its stamp.
    fn still_holds(&self, k: &Kpse) -> bool {
        let b = self.front();
        let check = || -> Option<bool> {
            let mut p = self.h[H_ENVS];
            let n = u32_at(b, p)?;
            p += 4;
            for _ in 0..n {
                let (nlen, vlen) = (u32_at(b, p)? as usize, u32_at(b, p + 4)?);
                let name = b.get(p + 8..p + 8 + nlen)?;
                p += 8 + nlen;
                let now = k.getenv_now(name);
                if vlen == NOT_SET {
                    if now.is_some() {
                        return Some(false);
                    }
                } else {
                    let v = b.get(p..p + vlen as usize)?;
                    p += vlen as usize;
                    if now.as_deref() != Some(v) {
                        return Some(false);
                    }
                }
            }
            let mut p = self.h[H_STAMPS];
            let n = u32_at(b, p)?;
            p += 4;
            for _ in 0..n {
                let plen = u32_at(b, p)? as usize;
                let mut st: Stamp = [0; 5];
                for (i, x) in st.iter_mut().enumerate() {
                    *x = u64_at(b, p + 4 + i * 8)?;
                }
                let path = b.get(p + 44..p + 44 + plen)?;
                p += 44 + plen;
                if sys::file_stamp(path) != st {
                    return Some(false);
                }
            }
            Some(true)
        };
        check() == Some(true)
    }
}

impl Kpse {
    /// What the index's name and contents are for: this layout of the file, this program name, this tree.  What else the tables depend on is checked one by one.
    fn index_key(&self) -> u64 {
        hash64(&[INDEX_LAYOUT.as_bytes(), &[self.db_casefold as u8], &self.tree_key().to_le_bytes()[..], &self.program_name].concat())
    }

    /// A hash of which TeX tree this is: its place, and `TEXMFCNF`, so that two settings in use side by side have a file each.
    pub fn tree_key(&self) -> u64 {
        let mut k = Vec::new();
        let mut add = |s: &[u8]| {
            k.extend_from_slice(&(s.len() as u64).to_le_bytes());
            k.extend_from_slice(s);
        };
        for s in &self.tree.selfauto {
            add(s);
        }
        add(self.tree.cnf_default.as_deref().unwrap_or(b"-"));
        add(self.tree.builtin_cnf.as_deref().unwrap_or(b"-"));
        for (n, v) in &self.tree.vars {
            add(n);
            add(v);
        }
        for r in &self.tree.walk_roots {
            add(r);
        }
        add(self.getenv_now(b"TEXMFCNF").as_deref().unwrap_or(b"-"));
        hash64(&k)
    }

    fn index_file(&self, key: u64) -> Option<std::path::PathBuf> {
        if !self.process_env {
            return None;
        }
        Some(sys::cache_dir()?.join(format!("kpse-index-{key:016x}")))
    }

    /// What C does at the first `kpathsea_cnf_get`: reads the cnf files and then the databases.  From the index if it holds, and otherwise from the files.
    pub(crate) fn load_tables(&mut self) {
        let key = self.index_key();
        let file = self.index_file(key);
        if let Some(path) = &file {
            #[cfg(feature = "prof")]
            let _p = crate::prof::begin(&crate::prof::INDEX);
            if self.index_use(path, key) {
                return;
            }
        }
        self.tables_from_files(key, file.as_deref());
    }

    /// True if the file at `path` holds, and is now what the tables come from.
    fn index_use(&mut self, path: &std::path::Path, key: u64) -> bool {
        let Some(ix) = CachedFile::open(path, key).and_then(Index::new) else { return false };
        if !ix.still_holds(self) {
            return false;
        }
        self.recorded.extend(ix.list(H_RECORDED).iter().map(|r| r.to_vec()));
        for w in ix.list(H_WARNINGS) {
            self.warn(w);
        }
        let aliases = ix.list(H_ALIASES);
        if !aliases.is_empty() {
            let mut table: HashMap<Vec<u8>, Vec<Vec<u8>>> = HashMap::new();
            for pair in aliases.chunks_exact(2) {
                table.entry(pair[0].to_vec()).or_default().push(pair[1].to_vec());
            }
            self.alias = Some(table);
        }
        // Their paths stay under the program name of now, as if they had been worked out here.
        let paths = ix.list(H_PATHS);
        if let [cnf, db] = paths[..] {
            self.formats[format::KPSE_CNF_FORMAT as usize] = Some(Arc::new(FormatState { path: cnf.to_vec(), elts: crate::search::split_elts(cnf) }));
            self.formats[format::KPSE_DB_FORMAT as usize] = Some(Arc::new(FormatState { path: db.to_vec(), elts: crate::search::split_elts(db) }));
        }
        self.index = Some(ix);
        self.index_used = true;
        self.db_state = DbState::Ready;
        self.followup_search = true;
        true
    }

    /// `read_all_cnf` and `kathsea_init_db`, noting what they look at; makes the index of it, and publishes that if it may be trusted later.
    fn tables_from_files(&mut self, key: u64, file: Option<&std::path::Path>) {
        sys::observe_start();
        *self.env_log.borrow_mut() = Some(Vec::new());
        let (warned, recorded) = (self.warnings_len(), self.recorded.len());
        // A path already worked out would not be noted.
        self.formats[format::KPSE_CNF_FORMAT as usize] = None;
        self.formats[format::KPSE_DB_FORMAT as usize] = None;
        self.index = None;
        self.index_used = false;
        self.db_state = DbState::Loading;
        self.doing_cnf_init = true;
        self.read_all_cnf();
        self.doing_cnf_init = false;
        let db = self.init_db();
        let body = make_body(&db, self.db_casefold);
        // The databases are searched for the alias files, so they have to work now.
        let first = make_file(
            &Facts {
                key,
                envs: &[],
                stamps: &[],
                recorded: &[],
                warnings: &[],
                dir_list: &db.dir_list,
                db_files: &db.files,
                aliases: &[],
                cnf_path: b"",
                db_path: b"",
                format_paths: &[],
                cnf: &self.cnf,
                usable: db.usable,
            },
            &body,
        );
        self.index = CachedFile::from_vec(first, key).and_then(Index::new);
        self.db_state = DbState::Ready;
        let aliases = self.init_aliases();
        let (cnf_path, db_path) = (self.format_path(format::KPSE_CNF_FORMAT).unwrap_or_default(), self.format_path(format::KPSE_DB_FORMAT).unwrap_or_default());
        let mut format_paths = vec![self.program_name.clone()];
        for f in INDEXED_FORMATS {
            format_paths.push(f.to_le_bytes().to_vec());
            format_paths.push(self.format_path(f).unwrap_or_default());
        }
        let envs = self.env_log.borrow_mut().take().unwrap_or_default();
        let stamps = sys::observe_take();
        let warnings = self.warnings_since(warned);
        let bytes = make_file(
            &Facts {
                key,
                envs: &envs,
                stamps: &stamps,
                recorded: &self.recorded[recorded..],
                warnings: &warnings,
                dir_list: &db.dir_list,
                db_files: &db.files,
                aliases: &aliases,
                cnf_path: &cnf_path,
                db_path: &db_path,
                format_paths: &format_paths,
                cnf: &self.cnf,
                usable: db.usable,
            },
            &body,
        );
        if let Some(path) = file {
            if self.index_may_last(&stamps) {
                sys::publish_path(path, &bytes);
                if let Some(d) = path.parent() {
                    sys::cache_prune(d, "kpse-index-");
                }
            }
        }
        self.index = CachedFile::from_vec(bytes, key).and_then(Index::new);
        self.cnf = HashMap::new();
    }

    /// False if what the tables came from could change without a later run seeing it: a stamp too young to be sure of, or the order of a list of directories,
    /// which a search changes and a run would not find again.
    fn index_may_last(&self, stamps: &[(Vec<u8>, Stamp)]) -> bool {
        let now = sys::now_ns();
        // A stamp this young might not show a change made just after it, in the same tick of the file system's clock.
        let young = stamps.iter().any(|(_, s)| s[2].max(s[3]) + self.index_racy_ns > now);
        !young && self.dir_lists.iter().all(|l| l.len() <= 1)
    }

    /// A sum failed: the file goes, and the tables are made again, by a new instance, so that everything it looks at is looked at and noted.  What this run has
    /// said, recorded and worked out stays as it is.
    pub(crate) fn index_rebuild(&mut self) {
        if let Some(path) = self.index_file(self.index_key()) {
            let _ = std::fs::remove_file(path);
        }
        let mut fresh = Kpse::new(crate::Config {
            program_name: self.program_name.clone(),
            invocation_name: self.invocation_name().to_vec(),
            engine: None,
            process_env: self.process_env,
            vars: self.child_env(),
            tree: Some(self.tree.clone()),
            db_casefold: self.db_casefold,
        });
        fresh.index_racy_ns = self.index_racy_ns;
        fresh.ensure_cnf();
        self.index = fresh.index.take();
        self.alias = fresh.alias.take();
        self.index_used = false;
    }
}
