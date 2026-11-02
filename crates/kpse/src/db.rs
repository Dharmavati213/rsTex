//! The `ls-R` database: db.c.
//!
//! kpathsea reads `ls-R` into a hash table with one allocation per entry and two strings per line.
//! This keeps one buffer holding the text of all databases, and a flat table of offsets into it, so
//! the cost is a read and a scan, and a path search that `ls-R` can answer touches no directory.
//! A later binary index (M5) only has to replace `Db`.
use crate::names::*;
use crate::{Kpse, format, sys};
use std::collections::HashMap;
use std::io::Read;

/// One file name in a database.
#[derive(Clone, Copy)]
struct Entry {
    off: u32,
    len: u32,
    dir: u32,
    hash: u32,
    /// The next entry of the same bucket, in insertion order; `u32::MAX` ends the chain.
    next: u32,
}

/// `kpse->db` and `kpse->db_dir_list`.
#[derive(Default)]
pub(crate) struct Db {
    text: Vec<u8>,
    dirs: Vec<Vec<u8>>,
    entries: Vec<Entry>,
    heads: Vec<u32>,
    /// The same names by their lower case, built when first asked for: [`crate::Config::db_casefold`].
    fold_heads: Vec<u32>,
    fold_next: Vec<u32>,
    /// The top directories of the usable databases.
    dir_list: Vec<Vec<u8>>,
    /// True once any database has entries: `kpse->db.buckets != NULL`.
    pub(crate) usable: bool,
}

/// Word-at-a-time multiplicative hash.
#[inline]
fn hash_name(s: &[u8]) -> u32 {
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
    (h >> 32) as u32
}

/// `ignore_dir_p`: a directory with an element beginning with `.`, more than just `./`, is hidden.
fn ignore_dir_p(dirname: &[u8]) -> bool {
    let mut p = 1;
    while p < dirname.len() {
        if dirname[p] == b'.' && is_dir_sep_ch(dirname[p - 1]) && at(dirname, p + 1) != 0 && !is_dir_sep_ch(at(dirname, p + 1)) {
            return true;
        }
        p += 1;
    }
    false
}

impl Db {
    /// Adds a name found in directory number `dir`.
    #[inline]
    fn push_entry(&mut self, off: usize, len: usize, dir: u32) {
        let hash = hash_name(&self.text[off..off + len]);
        self.entries.push(Entry { off: off as u32, len: len as u32, dir, hash, next: u32::MAX });
    }

    /// `db_build`, on the text of one `ls-R`, already read to `text[start..]`.
    fn build(&mut self, start: usize, top_dir: &[u8]) -> u32 {
        // read_line drops NUL bytes and takes \r and \r\n for \n, which is rare enough to clean first.
        let region = &self.text[start..];
        if region.contains(&0) || region.contains(&b'\r') {
            let clean = read_lines(region);
            self.text.truncate(start);
            for l in clean {
                self.text.extend_from_slice(&l);
                self.text.push(b'\n');
            }
        }
        if DOSISH {
            // MONOCASE_FILENAMES: everything in the database is compared in one case.
            self.text[start..].make_ascii_lowercase();
        }
        let end = self.text.len();
        // A line of ls-R is some 16 bytes long.
        self.entries.reserve((end - start) / 16);
        let mut file_count = 0;
        let mut cur_dir: Option<u32> = None;
        let mut p = start;
        while p < end {
            let nl = self.text[p..end].iter().position(|&c| c == b'\n').map_or(end, |i| p + i);
            let len = nl - p;
            let (a, b) = (self.text[p], if len > 1 { self.text[p + 1] } else { 0 });
            // A line like `/foo:` or `./foo:` begins a new directory.
            if len > 0 && self.text[nl - 1] == b':' && absolute_p(&self.text[p..nl], true) {
                let line = &self.text[p..nl];
                if ignore_dir_p(line) {
                    cur_dir = None;
                } else {
                    let mut d = Vec::with_capacity(top_dir.len() + len);
                    // Skip a leading `./`, which would only confuse `match`.
                    if a == b'.' {
                        d.extend_from_slice(top_dir);
                        d.extend_from_slice(&line[2.min(len - 1)..len - 1]);
                    } else {
                        d.extend_from_slice(&line[..len - 1]);
                    }
                    d.push(b'/');
                    self.dirs.push(d);
                    cur_dir = Some(self.dirs.len() as u32 - 1);
                }
            } else if let Some(dir) = cur_dir {
                // Ignore blank, `.` and `..` lines.
                if len > 0 && !(a == b'.' && (len == 1 || (b == b'.' && len == 2))) {
                    self.push_entry(p, len, dir);
                    file_count += 1;
                }
            }
            p = nl + 1;
        }
        file_count
    }

    /// Gives a tree with no `ls-R` the same treatment by walking it.  MiKTeX has none (DESIGN.md
    /// section 12); every directory beginning with `.` is skipped, as `ignore_dir_p` would.
    fn walk(&mut self, top: &[u8]) -> u32 {
        let mut top_dir = top.to_vec();
        if top_dir.last() != Some(&b'/') {
            top_dir.push(b'/');
        }
        let mut stack = vec![top_dir.clone()];
        let mut file_count = 0;
        while let Some(dir) = stack.pop() {
            let Ok(rd) = std::fs::read_dir(sys::path_of(&dir)) else { continue };
            let mut names: Vec<(Vec<u8>, bool)> = rd
                .filter_map(|e| e.ok())
                .map(|e| (sys::os_bytes(&e.file_name()), sys::entry_is_dir(&e)))
                .collect();
            // Directory order is not portable, and the order decides which of two files wins.
            names.sort();
            let dir_key = if DOSISH { dir.to_ascii_lowercase() } else { dir.clone() };
            self.dirs.push(dir_key);
            let id = self.dirs.len() as u32 - 1;
            let mut subdirs = Vec::new();
            for (name, is_dir) in names {
                if is_dir {
                    if name.first() != Some(&b'.') {
                        subdirs.push([&dir[..], &name[..], b"/"].concat());
                    }
                } else {
                    let off = self.text.len();
                    let key = if DOSISH { name.to_ascii_lowercase() } else { name };
                    self.text.extend_from_slice(&key);
                    self.text.push(b'\n');
                    self.push_entry(off, key.len(), id);
                    file_count += 1;
                }
            }
            stack.extend(subdirs.into_iter().rev());
        }
        if file_count > 0 {
            self.dir_list.push(top_dir);
            self.usable = true;
        }
        file_count
    }

    /// Links the buckets, in insertion order.  After this, lookups are possible.
    fn finish(&mut self) {
        let cap = (self.entries.len() * 2).next_power_of_two().max(16);
        self.heads = vec![u32::MAX; cap];
        let mask = cap as u32 - 1;
        for i in (0..self.entries.len()).rev() {
            let b = (self.entries[i].hash & mask) as usize;
            self.entries[i].next = self.heads[b];
            self.heads[b] = i as u32;
        }
    }

    /// `hash_lookup`: the directories holding `name`, in the order they were read.
    fn dirs_of<'a>(&'a self, name: &'a [u8]) -> impl Iterator<Item = &'a [u8]> + 'a {
        let hash = hash_name(name);
        let mut i = if self.heads.is_empty() { u32::MAX } else { self.heads[(hash as usize) & (self.heads.len() - 1)] };
        std::iter::from_fn(move || {
            while i != u32::MAX {
                let e = self.entries[i as usize];
                i = e.next;
                if e.hash == hash && &self.text[e.off as usize..(e.off + e.len) as usize] == name {
                    return Some(&self.dirs[e.dir as usize][..]);
                }
            }
            None
        })
    }

    /// Each directory and name of a file matching `name` when case is ignored, in the order read.
    fn folded_hits<'a>(&'a mut self, name: &[u8]) -> impl Iterator<Item = (&'a [u8], &'a [u8])> + 'a {
        if self.fold_heads.is_empty() {
            let cap = (self.entries.len() * 2).next_power_of_two().max(16);
            self.fold_heads = vec![u32::MAX; cap];
            self.fold_next = vec![u32::MAX; self.entries.len()];
            for i in (0..self.entries.len()).rev() {
                let e = self.entries[i];
                let lower = self.text[e.off as usize..(e.off + e.len) as usize].to_ascii_lowercase();
                let b = hash_name(&lower) as usize & (cap - 1);
                self.fold_next[i] = self.fold_heads[b];
                self.fold_heads[b] = i as u32;
            }
        }
        let lower = name.to_ascii_lowercase();
        let this = &*self;
        let mut i = this.fold_heads[hash_name(&lower) as usize & (this.fold_heads.len() - 1)];
        std::iter::from_fn(move || {
            while i != u32::MAX {
                let at = i as usize;
                let e = this.entries[at];
                i = this.fold_next[at];
                let n = &this.text[e.off as usize..(e.off + e.len) as usize];
                if n.eq_ignore_ascii_case(&lower) {
                    return Some((&this.dirs[e.dir as usize][..], n));
                }
            }
            None
        })
    }

    /// `elt_in_db` over all databases: whether to try the db for a lookup in this element.
    pub(crate) fn relevant(&self, path_elt: &[u8]) -> bool {
        self.dir_list.iter().any(|d| elt_in_db(d, path_elt))
    }
}

/// `elt_in_db`: is `db_dir` a prefix of `path_elt`?
fn elt_in_db(db_dir: &[u8], path_elt: &[u8]) -> bool {
    if db_dir.is_empty() || path_elt.is_empty() {
        return false;
    }
    if path_elt.len() < db_dir.len() {
        return false;
    }
    db_dir.iter().zip(path_elt).all(|(&a, &b)| file_char_eq(a, b))
}

/// `match`: could `filename` be in `path_elt`?  `//` matches any number of directories.
pub(crate) fn match_elt(filename: &[u8], path_elt: &[u8]) -> bool {
    match_from(filename, 0, path_elt, 0)
}

fn match_from(filename: &[u8], f0: usize, path_elt: &[u8], p0: usize) -> bool {
    let (mut f, mut p) = (f0, p0);
    let mut matched = false;
    while at(filename, f) != 0 && at(path_elt, p) != 0 {
        if file_char_eq(at(filename, f), at(path_elt, p)) {
            // A normal character match.
        } else if is_dir_sep_ch(at(path_elt, p)) && f0 < f && p > 0 && is_dir_sep_ch(path_elt[p - 1]) {
            // At a //: get past the second and any later slash.
            while is_dir_sep_ch(at(path_elt, p)) {
                p += 1;
            }
            if at(path_elt, p) == 0 {
                // A trailing // matches anything.
                matched = true;
                break;
            }
            // An intermediate //: try matching the rest at each possible character.
            while !matched && at(filename, f) != 0 {
                if is_dir_sep_ch(filename[f - 1]) && file_char_eq(at(filename, f), at(path_elt, p)) {
                    matched = match_from(filename, f, path_elt, p);
                }
                f += 1;
            }
            // Prevents f++ at the end of filename.
            break;
        } else {
            break;
        }
        f += 1;
        p += 1;
    }
    // At the end of path_elt we must be at the last component of filename.
    if !matched && at(path_elt, p) == 0 {
        if is_dir_sep_ch(at(filename, f)) {
            f += 1;
        }
        if f0 == f || is_dir_sep_ch(filename[f - 1]) {
            while at(filename, f) != 0 && !is_dir_sep_ch(at(filename, f)) {
                f += 1;
            }
            matched = at(filename, f) == 0;
        }
    }
    matched
}

/// `alias_build`.
fn alias_parse(table: &mut HashMap<Vec<u8>, Vec<Vec<u8>>>, text: &[u8]) {
    for line in read_lines(text) {
        if line.is_empty() || line[0] == b'%' || line[0] == b'#' {
            continue;
        }
        // Each line should have two fields: realname aliasname.
        let mut r = 0;
        while is_space(at(&line, r)) {
            r += 1;
        }
        let mut a = r;
        while at(&line, a) != 0 && !is_space(at(&line, a)) {
            a += 1;
        }
        let real = &line[r..a];
        let mut s = (a + 1).min(line.len());
        while is_space(at(&line, s)) {
            s += 1;
        }
        // C keeps everything to the end of the line, including anything after a second field.
        let alias = &line[s..];
        if !real.is_empty() && !alias.is_empty() {
            table.entry(fold_key(alias)).or_default().push(real.to_vec());
        }
    }
}

impl Kpse {
    /// `kpathsea_init_db`.
    pub(crate) fn init_db(&mut self) {
        let Some(st) = self.init_format(format::KPSE_DB_FORMAT) else { return };
        // ls-r first: where the two are one file, the canonical name is the one kept below.
        let names = [b"ls-r".to_vec(), b"ls-R".to_vec()];
        let db_files = self.path_search_list_generic(&st.elts, &names, true, true);

        // On a case-insensitive file system ls-R and ls-r are one file; do not read it twice.
        let mut unique: Vec<&Vec<u8>> = Vec::new();
        for (i, path1) in db_files.iter().enumerate() {
            let same = db_files.get(i + 1).is_some_and(|path2| path1.eq_ignore_ascii_case(path2) && sys::same_file(path1, path2));
            if !same {
                unique.push(path1);
            }
        }

        let mut db = std::mem::take(&mut self.db);
        for file in &unique {
            let Ok(mut f) = std::fs::File::open(sys::path_of(file)) else { continue };
            let start = db.text.len();
            if f.read_to_end(&mut db.text).is_err() {
                db.text.truncate(start);
                continue;
            }
            // Keep the /: strlen (db_filename) - strlen ("ls-R").
            let top_dir = file[..file.len() - 4].to_vec();
            let count = db.build(start, &top_dir);
            if count == 0 {
                self.warn(&[b"kpathsea: ", &file[..], b": No usable entries in ls-R"].concat());
                self.warn(b"kpathsea: See the manual for how to generate ls-R");
            } else {
                db.dir_list.push(top_dir);
                db.usable = true;
            }
        }
        // A tree with no ls-R, when the tree finder says so.
        for root in self.tree.walk_roots.clone() {
            db.walk(&root);
        }
        db.finish();
        self.db = db;
        self.db_state = crate::DbState::Ready;

        // Add the content of any alias databases.
        let aliases = self.all_path_search_str(&st.path.clone(), b"aliases");
        let mut table = HashMap::new();
        let mut any = false;
        for file in &aliases {
            if let Ok(text) = std::fs::read(sys::path_of(file)) {
                alias_parse(&mut table, &text);
                any = true;
            }
        }
        self.alias = if any { Some(table) } else { None };
    }

    /// Loads the databases if a search needs them for the first time.
    pub(crate) fn ensure_db(&mut self) {
        if self.db_state == crate::DbState::NotLoaded {
            self.db_state = crate::DbState::Loading;
            self.init_db();
            self.db_state = crate::DbState::Ready;
        }
    }

    /// The names to try for `name`, with the name itself first.
    fn names_and_aliases(&self, name: &[u8]) -> Vec<Vec<u8>> {
        let mut out = vec![fold_key(name)];
        if let Some(t) = &self.alias {
            if let Some(list) = t.get(&fold_key(name)) {
                out.extend(list.iter().cloned());
            }
        }
        out
    }

    /// The loop body shared by `kpathsea_db_search` and `kpathsea_db_search_list`: the hits of
    /// one name (with no directory part) in `path`.
    fn db_hits(&self, name: &[u8], path: &[u8], all: bool, ret: &mut Vec<Vec<u8>>, fresh_per_alias: bool) -> bool {
        let aliases = self.names_and_aliases(name);
        let mut done = false;
        for ctry in &aliases {
            if done {
                break;
            }
            if fresh_per_alias {
                ret.clear();
            }
            for dir in self.db.dirs_of(ctry) {
                if done {
                    break;
                }
                let mut db_file = [dir, &ctry[..]].concat();
                // For each file found, see if it matches the path element: with .../cx/cmr10.300pk
                // and .../ricoh/cmr10.300pk and a path of .../cx, we do not want the ricoh file.
                if match_elt(&db_file, path) {
                    let mut found = None;
                    if readable_file(&mut db_file) {
                        found = Some(db_file);
                    } else {
                        // The hit is not on disk; try all its aliases.
                        for a in &aliases[1..] {
                            let mut atry = [dir, &a[..]].concat();
                            if readable_file(&mut atry) {
                                found = Some(atry);
                                break;
                            }
                        }
                    }
                    if let Some(f) = found {
                        ret.push(f);
                        if !all {
                            done = true;
                        }
                    }
                }
            }
        }
        done
    }

    /// `kpathsea_db_search`.  `None` means no database is relevant, so the caller may use the disk.
    pub(crate) fn db_search(&mut self, name: &[u8], orig_path_elt: &[u8], all: bool) -> Option<Vec<Vec<u8>>> {
        self.ensure_db();
        if self.db_state != crate::DbState::Ready || !self.db.usable {
            return None;
        }
        // dpi600/cmr10.pk: look for cmr10.pk and append /dpi600 to the path element.
        let (path, name): (Vec<u8>, &[u8]) = match name.iter().rposition(|&c| c == b'/') {
            Some(i) if i != 0 => ([orig_path_elt, b"/", &name[..i]].concat(), &name[i + 1..]),
            _ => (orig_path_elt.to_vec(), name),
        };
        if !self.db.relevant(&path) {
            return None;
        }
        let mut ret = Vec::new();
        self.db_hits(name, &path, all, &mut ret, true);
        Some(ret)
    }

    /// `kpathsea_db_search_list`.
    pub(crate) fn db_search_list(&mut self, names: &[Vec<u8>], path_elt: &[u8], all: bool) -> Option<Vec<Vec<u8>>> {
        self.ensure_db();
        if self.db_state != crate::DbState::Ready || !self.db.usable {
            return None;
        }
        if !self.db.relevant(path_elt) {
            return None;
        }
        let mut ret = Vec::new();
        for full in names {
            // Absolute names were caught by the caller.
            if absolute_p(full, true) {
                continue;
            }
            let done = match full.iter().rposition(|&c| c == b'/') {
                Some(i) if i != 0 => {
                    let path = [path_elt, b"/", &full[..i]].concat();
                    self.db_hits(&full[i + 1..], &path, all, &mut ret, false)
                }
                _ => self.db_hits(full, path_elt, all, &mut ret, false),
            };
            if done {
                break;
            }
        }
        // Nothing with the exact case.  Same order as above, and only the last component is folded, as
        // casefold_readable_file does on the disk.
        if ret.is_empty() && self.db_casefold && self.casefold_on() {
            let mut db = std::mem::take(&mut self.db);
            'names: for full in names {
                if absolute_p(full, true) {
                    continue;
                }
                let (path, name) = match full.iter().rposition(|&c| c == b'/') {
                    Some(i) if i != 0 => ([path_elt, b"/", &full[..i]].concat(), &full[i + 1..]),
                    _ => (path_elt.to_vec(), &full[..]),
                };
                for (dir, actual) in db.folded_hits(name) {
                    let mut db_file = [dir, actual].concat();
                    if match_elt(&db_file, &path) && readable_file(&mut db_file) {
                        ret.push(db_file);
                        if !all {
                            break 'names;
                        }
                    }
                }
            }
            self.db = db;
        }
        Some(ret)
    }
}

/// `kpathsea_readable_file`: normalizes `name` and, if a component is too long, truncates it.  True if
/// the file is readable under the name now in `name`.
pub(crate) fn readable_file(name: &mut Vec<u8>) -> bool {
    normalize_path(name);
    match sys::readable(name) {
        sys::Readable::Yes => true,
        sys::Readable::No => false,
        sys::Readable::TooLong => {
            truncate_long_components(name);
            sys::readable(name) == sys::Readable::Yes
        }
    }
}

/// readable.c's `NAME_MAX`.
const NAME_MAX: usize = 255;

/// The `ENAMETOOLONG` branch of `kpathsea_readable_file`.
fn truncate_long_components(name: &mut Vec<u8>) {
    let (mut c_len, mut s, mut t) = (0usize, 0usize, 0usize);
    while s < name.len() {
        if c_len <= NAME_MAX {
            t = s;
        }
        let c = name[s];
        if is_dir_sep(c) || is_device_sep_g::<DOSISH>(c) {
            if c_len > NAME_MAX {
                name.drain(t..s);
                s = t;
            }
            c_len = 0;
        } else {
            c_len += 1;
        }
        s += 1;
    }
    if c_len > NAME_MAX {
        name.truncate(t);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn match_as_db_c() {
        // The examples in the comments of db.c's match.
        assert!(match_elt(b"ptmr.vf", b""));
        assert!(!match_elt(b"foo/ptmr.vf", b""));
        assert!(match_elt(b"vf/ptmr.vf", b"vf/"));
        assert!(match_elt(b"vf/ptmr.vf", b"vf"));
        assert!(!match_elt(b"vfoo.ext", b"vf"));
        // Trailing and intermediate //.
        assert!(match_elt(b"/a/b/c/d.tex", b"/a//"));
        assert!(match_elt(b"/a/b/c/d.tex", b"/a/b//"));
        assert!(match_elt(b"/a/b/c/d.tex", b"/a///"));
        assert!(match_elt(b"/a/b/c/d.tex", b"/a//c"));
        assert!(match_elt(b"/a/b/c/d.tex", b"/a//c/"));
        assert!(match_elt(b"/a/b/c/d.tex", b"/a//b//"));
        assert!(!match_elt(b"/a/b/c/d.tex", b"/a//x"));
        assert!(!match_elt(b"/a/b/c/d.tex", b"/a/b"));
        assert!(!match_elt(b"/a/bb/d.tex", b"/a/b//"));
        assert!(!match_elt(b"/ab/d.tex", b"/a"));
        // A part of a component is not a component.
        assert!(!match_elt(b"/a/xc/d.tex", b"/a//c"));
    }

    #[test]
    fn elt_in_db_as_db_c() {
        assert!(elt_in_db(b"/a/b/", b"/a/b/c//"));
        assert!(elt_in_db(b"/a/b/", b"/a/b/"));
        assert!(!elt_in_db(b"/a/b/", b"/a/b"));
        assert!(!elt_in_db(b"/a/b/", b"."));
        assert!(!elt_in_db(b"", b"/a"));
        assert!(!elt_in_db(b"/a/", b""));
    }

    #[test]
    fn hidden_directories() {
        assert!(!ignore_dir_p(b"./:"));
        assert!(!ignore_dir_p(b"./a/b:"));
        assert!(ignore_dir_p(b"./a/.git:"));
        assert!(ignore_dir_p(b"./.hidden/b:"));
        assert!(ignore_dir_p(b"/a/.:"));
        assert!(!ignore_dir_p(b"/a/b.c/d:"));
    }

    #[test]
    fn db_build_reads_a_small_tree() {
        let mut db = Db::default();
        let text = b"% ls-R -- comment\nignored-before-any-directory\n./:\nls-R\ntop.tex\n.\n..\n\n./a:\none.tex\ndup.tex\n\n./a/.git:\nhidden.tex\n\n./b:\ndup.tex\ndir-looking:\n\n/abs/dir:\nabs.tex\n";
        db.text.extend_from_slice(text);
        let n = db.build(0, b"/top/");
        db.finish();
        assert_eq!(n, 7);
        let dirs = |name: &[u8]| db.dirs_of(name).map(<[u8]>::to_vec).collect::<Vec<_>>();
        // `./:` is `top_dir` and what follows the `./`, which in C is the / that replaced the colon.
        assert_eq!(dirs(b"top.tex"), vec![b"/top//".to_vec()]);
        assert_eq!(dirs(b"one.tex"), vec![b"/top/a/".to_vec()]);
        // In the order read.
        assert_eq!(dirs(b"dup.tex"), vec![b"/top/a/".to_vec(), b"/top/b/".to_vec()]);
        assert_eq!(dirs(b"abs.tex"), vec![b"/abs/dir/".to_vec()]);
        assert_eq!(dirs(b"dir-looking:"), vec![b"/top/b/".to_vec()]);
        assert!(dirs(b"hidden.tex").is_empty());
        assert!(dirs(b"ignored-before-any-directory").is_empty());
        assert!(dirs(b".").is_empty());
        assert!(dirs(b"..").is_empty());
        assert!(dirs(b"").is_empty());
    }

    #[test]
    fn db_build_handles_crlf_and_nul() {
        let mut db = Db::default();
        db.text.extend_from_slice(b"./x:\r\nab\0c.tex\rlast.tex");
        assert_eq!(db.build(0, b"/t/"), 2);
        db.finish();
        assert_eq!(db.dirs_of(b"abc.tex").count(), 1);
        assert_eq!(db.dirs_of(b"last.tex").count(), 1);
    }

    #[test]
    fn long_components_are_truncated_as_readable_c() {
        let long = vec![b'x'; 300];
        let mut n = [b"/a/", &long[..], b"/b"].concat();
        truncate_long_components(&mut n);
        assert_eq!(n, [b"/a/", &vec![b'x'; 255][..], b"/b"].concat());
        let mut n = [b"/a/", &long[..]].concat();
        truncate_long_components(&mut n);
        assert_eq!(n, [b"/a/", &vec![b'x'; 255][..]].concat());
        let mut n = b"/a/short".to_vec();
        truncate_long_components(&mut n);
        assert_eq!(n, b"/a/short");
    }

    #[test]
    fn aliases() {
        let mut t = HashMap::new();
        alias_parse(&mut t, b"% comment\n# comment\n\nreal.mf   alias.mf\nnoalias\n  sp.x\tsp.y\n");
        assert_eq!(t.get(&b"alias.mf"[..]).unwrap(), &vec![b"real.mf".to_vec()]);
        assert_eq!(t.get(&b"sp.y"[..]).unwrap(), &vec![b"sp.x".to_vec()]);
        assert_eq!(t.len(), 2);
    }
}
