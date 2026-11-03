//! The `ls-R` database: db.c.
//!
//! kpathsea reads `ls-R` into a hash table with one allocation per entry and two strings per line.
//! This reads it once, into one buffer holding the text of all databases and a flat table of offsets into it, to
//! make the flat tables of `index.rs`, and a run maps those.  A path search that `ls-R` can answer touches no directory.
use crate::cachefile::Corrupt;
use crate::index::hash_name;
use crate::names::*;
use crate::{Kpse, format, sys};
use std::collections::HashMap;
use std::io::Read;

/// One file name in a database.
#[derive(Clone, Copy)]
pub(crate) struct Entry {
    pub(crate) off: u32,
    pub(crate) len: u32,
    pub(crate) dir: u32,
    pub(crate) hash: u32,
}

/// `kpse->db` and `kpse->db_dir_list`, while they are being read.
#[derive(Default)]
pub(crate) struct Db {
    pub(crate) text: Vec<u8>,
    pub(crate) dirs: Vec<Vec<u8>>,
    pub(crate) entries: Vec<Entry>,
    /// The top directories of the usable databases.
    pub(crate) dir_list: Vec<Vec<u8>>,
    /// The `ls-R` files themselves.
    pub(crate) files: Vec<Vec<u8>>,
    /// True once any database has entries: `kpse->db.buckets != NULL`.
    pub(crate) usable: bool,
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
        self.entries.push(Entry { off: off as u32, len: len as u32, dir, hash });
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
            let Ok(rd) = sys::read_dir(&dir) else { continue };
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
}

/// `elt_in_db`: is `db_dir` a prefix of `path_elt`?
pub(crate) fn elt_in_db(db_dir: &[u8], path_elt: &[u8]) -> bool {
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
pub(crate) fn alias_parse(table: &mut HashMap<Vec<u8>, Vec<Vec<u8>>>, text: &[u8]) {
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
    /// The first part of `kpathsea_init_db`: the `ls-R` files, read.
    pub(crate) fn init_db(&mut self) -> Db {
        #[cfg(feature = "prof")]
        let _p = crate::prof::begin(&crate::prof::DB);
        let mut db = Db::default();
        let Some(st) = self.init_format(format::KPSE_DB_FORMAT) else { return db };
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

        for file in &unique {
            sys::note(file);
            let Ok(mut f) = std::fs::File::open(sys::path_of(file)) else { continue };
            let start = db.text.len();
            #[cfg(feature = "prof")]
            let read_time = crate::prof::begin(&crate::prof::DB_READ);
            if f.read_to_end(&mut db.text).is_err() {
                db.text.truncate(start);
                continue;
            }
            #[cfg(feature = "prof")]
            drop(read_time);
            // Keep the /: strlen (db_filename) - strlen ("ls-R").
            let top_dir = file[..file.len() - 4].to_vec();
            #[cfg(feature = "prof")]
            let build_time = crate::prof::begin(&crate::prof::DB_BUILD);
            let count = db.build(start, &top_dir);
            #[cfg(feature = "prof")]
            drop(build_time);
            if count == 0 {
                self.warn(&[b"kpathsea: ", &file[..], b": No usable entries in ls-R"].concat());
                self.warn(b"kpathsea: See the manual for how to generate ls-R");
            } else {
                db.dir_list.push(top_dir);
                db.files.push(file.to_vec());
                db.usable = true;
            }
        }
        // A tree with no ls-R, when the tree finder says so.
        for root in self.tree.walk_roots.clone() {
            db.walk(&root);
        }
        db
    }

    /// The rest of `kpathsea_init_db`: the content of any alias databases, found with the databases already at work.  Also as a list for the index: each key, then one of its names.
    pub(crate) fn init_aliases(&mut self) -> Vec<Vec<u8>> {
        let path = self.init_format(format::KPSE_DB_FORMAT).map(|st| st.path.clone()).unwrap_or_default();
        let aliases = self.all_path_search_str(&path, b"aliases");
        let mut table = HashMap::new();
        let mut any = false;
        for file in &aliases {
            if let Ok(text) = sys::read_file(file) {
                alias_parse(&mut table, &text);
                any = true;
            }
        }
        let mut pairs = Vec::new();
        let mut keys: Vec<&Vec<u8>> = table.keys().collect();
        keys.sort();
        for k in keys {
            for v in &table[k] {
                pairs.push(k.clone());
                pairs.push(v.clone());
            }
        }
        self.alias = if any { Some(table) } else { None };
        pairs
    }

    /// True if `path` is below the top of a database that has been read: a file that a checkpoint may take by its stat.
    pub fn in_database_tree(&self, path: &[u8]) -> bool {
        self.index.as_ref().is_some_and(|ix| ix.in_database_tree(path))
    }

    /// The `ls-R` files, for a checkpoint's stamp of them.
    pub fn database_files(&mut self) -> Vec<Vec<u8>> {
        self.ensure_db();
        self.index.as_ref().map_or_else(Vec::new, |ix| ix.database_files())
    }

    /// Loads the databases if a search needs them for the first time: they come with the cnf files.
    pub(crate) fn ensure_db(&mut self) {
        if self.db_state == crate::DbState::NotLoaded {
            self.ensure_cnf();
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
    fn db_hits(&self, name: &[u8], path: &[u8], all: bool, ret: &mut Vec<Vec<u8>>, fresh_per_alias: bool) -> Result<bool, Corrupt> {
        let Some(ix) = &self.index else { return Ok(false) };
        let aliases = self.names_and_aliases(name);
        let mut done = false;
        let mut dirs = Vec::new();
        for ctry in &aliases {
            if done {
                break;
            }
            if fresh_per_alias {
                ret.clear();
            }
            dirs.clear();
            ix.dirs_of(ctry, &mut dirs)?;
            // What `db_insert` added goes last in its bucket, after what was read.
            let added: Vec<&[u8]> = self.db_added.iter().filter(|(_, n)| n == ctry).map(|(d, _)| &d[..]).collect();
            for i in 0..dirs.len() + added.len() {
                if done {
                    break;
                }
                let dir = if i < dirs.len() { ix.dir(dirs[i])? } else { added[i - dirs.len()] };
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
        Ok(done)
    }

    /// `kpathsea_db_search`.  `None` means no database is relevant, so the caller may use the disk.
    pub(crate) fn db_search(&mut self, name: &[u8], orig_path_elt: &[u8], all: bool) -> Option<Vec<Vec<u8>>> {
        self.ensure_db();
        if self.db_state != crate::DbState::Ready || !self.db_usable() {
            return None;
        }
        // dpi600/cmr10.pk: look for cmr10.pk and append /dpi600 to the path element.
        let (path, name): (Vec<u8>, &[u8]) = match name.iter().rposition(|&c| c == b'/') {
            Some(i) if i != 0 => ([orig_path_elt, b"/", &name[..i]].concat(), &name[i + 1..]),
            _ => (orig_path_elt.to_vec(), name),
        };
        if !self.db_relevant(&path) {
            return None;
        }
        let mut ret = Vec::new();
        if self.db_hits(name, &path, all, &mut ret, true).is_err() {
            self.index_rebuild();
            ret.clear();
            let _ = self.db_hits(name, &path, all, &mut ret, true);
        }
        Some(ret)
    }

    /// `kpathsea_db_search_list`.
    pub(crate) fn db_search_list(&mut self, names: &[Vec<u8>], path_elt: &[u8], all: bool) -> Option<Vec<Vec<u8>>> {
        self.ensure_db();
        if self.db_state != crate::DbState::Ready || !self.db_usable() {
            return None;
        }
        if !self.db_relevant(path_elt) {
            return None;
        }
        let mut ret = Vec::new();
        if self.db_list_hits(names, path_elt, all, &mut ret).is_err() {
            self.index_rebuild();
            ret.clear();
            let _ = self.db_list_hits(names, path_elt, all, &mut ret);
        }
        // Nothing with the exact case.  Same order as above, and only the last component is folded, as
        // casefold_readable_file does on the disk.
        if ret.is_empty() && self.db_casefold && self.casefold_on() {
            if self.folded.is_none() {
                self.folded = Some(self.folded_table());
            }
            let (Some(table), Some(ix)) = (&self.folded, &self.index) else { return Some(ret) };
            'names: for full in names {
                if absolute_p(full, true) {
                    continue;
                }
                let (path, name) = match full.iter().rposition(|&c| c == b'/') {
                    Some(i) if i != 0 => ([path_elt, b"/", &full[..i]].concat(), &full[i + 1..]),
                    _ => (path_elt.to_vec(), &full[..]),
                };
                let read = table.get(&name.to_ascii_lowercase()).into_iter().flatten().filter_map(|(d, actual)| Some((ix.dir(*d).ok()?, &actual[..])));
                let added = self.db_added.iter().filter(|(_, n)| n.eq_ignore_ascii_case(name)).map(|(d, n)| (&d[..], &n[..]));
                for (dir, actual) in read.chain(added) {
                    let mut db_file = [dir, actual].concat();
                    if match_elt(&db_file, &path) && readable_file(&mut db_file) {
                        ret.push(db_file);
                        if !all {
                            break 'names;
                        }
                    }
                }
            }
        }
        Some(ret)
    }

    /// The exact part of `kpathsea_db_search_list`.
    fn db_list_hits(&self, names: &[Vec<u8>], path_elt: &[u8], all: bool, ret: &mut Vec<Vec<u8>>) -> Result<(), Corrupt> {
        for full in names {
            // Absolute names were caught by the caller.
            if absolute_p(full, true) {
                continue;
            }
            let done = match full.iter().rposition(|&c| c == b'/') {
                Some(i) if i != 0 => {
                    let path = [path_elt, b"/", &full[..i]].concat();
                    self.db_hits(&full[i + 1..], &path, all, ret, false)?
                }
                _ => self.db_hits(full, path_elt, all, ret, false)?,
            };
            if done {
                break;
            }
        }
        Ok(())
    }

    /// The names of the databases by their lower case, in the order read, made when first asked for: [`crate::Config::db_casefold`].
    fn folded_table(&mut self) -> HashMap<Vec<u8>, Vec<(u32, Vec<u8>)>> {
        let mut entries = self.index.as_ref().map(|ix| ix.all_entries_owned());
        if matches!(entries, Some(Err(_))) {
            self.index_rebuild();
            entries = self.index.as_ref().map(|ix| ix.all_entries_owned());
        }
        let mut table: HashMap<Vec<u8>, Vec<(u32, Vec<u8>)>> = HashMap::new();
        for (d, n) in entries.and_then(Result::ok).unwrap_or_default() {
            table.entry(n.to_ascii_lowercase()).or_default().push((d, n));
        }
        table
    }

    pub(crate) fn db_usable(&self) -> bool {
        self.index.as_ref().is_some_and(|ix| ix.usable)
    }

    /// `kpathsea_db_insert`: a file made after the databases were read.  The tables are mapped and stay as they are; this run looks at the file after them, and a
    /// later run finds it through `ls-R`, whose new stamp makes the tables again, if the script that made the file put it there.
    pub(crate) fn db_insert(&mut self, fname: &[u8]) {
        let base = xbasename(fname);
        let dir = fold_key(&fname[..fname.len() - base.len()]);
        self.db_added.push((dir, fold_key(base)));
    }

    fn db_relevant(&self, path_elt: &[u8]) -> bool {
        self.index.as_ref().is_some_and(|ix| ix.relevant(path_elt))
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

    /// `db`'s flat tables, and a way to ask them.
    fn tables(db: &Db) -> crate::index::Index {
        let cnf = HashMap::new();
        let f = crate::index::Facts {
            key: 1,
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
            cnf: &cnf,
            usable: db.usable,
        };
        let bytes = crate::index::make_file(&f, &crate::index::make_body(db));
        crate::index::Index::new(crate::cachefile::CachedFile::from_vec(bytes, 1).unwrap()).unwrap()
    }

    fn dirs_in(ix: &crate::index::Index, name: &[u8]) -> Vec<Vec<u8>> {
        let mut ids = Vec::new();
        ix.dirs_of(name, &mut ids).unwrap();
        ids.iter().map(|&d| ix.dir(d).unwrap().to_vec()).collect()
    }

    #[test]
    fn db_build_reads_a_small_tree() {
        let mut db = Db::default();
        let text = b"% ls-R -- comment\nignored-before-any-directory\n./:\nls-R\ntop.tex\n.\n..\n\n./a:\none.tex\ndup.tex\n\n./a/.git:\nhidden.tex\n\n./b:\ndup.tex\ndir-looking:\n\n/abs/dir:\nabs.tex\n";
        db.text.extend_from_slice(text);
        let n = db.build(0, b"/top/");
        let ix = tables(&db);
        assert_eq!(n, 7);
        let dirs = |name: &[u8]| dirs_in(&ix, name);
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
        let ix = tables(&db);
        assert_eq!(dirs_in(&ix, b"abc.tex").len(), 1);
        assert_eq!(dirs_in(&ix, b"last.tex").len(), 1);
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
