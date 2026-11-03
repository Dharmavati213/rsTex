//! Looking a name up: pathsearch.c, elt-dirs.c, and the file-finding part of tex-file.c.
use crate::db::readable_file;
use crate::expand::cnf_p;
use crate::format::{self, DefaultPath, FormatDef};
use crate::names::*;
use crate::{FormatState, Kpse, sys};
use std::sync::Arc;

/// One element of a search path: what `kpathsea_path_element` returns, with `!!` taken off and
/// normalized, as `path_search` does before using it.
#[derive(Clone, Debug)]
pub(crate) struct Elt {
    pub(crate) text: Vec<u8>,
    /// False for a `!!` element: the database only, never the disk.
    pub(crate) disk: bool,
}

/// One directory of an element's list: `str_llist_elt_type`.
#[derive(Clone, Debug)]
pub(crate) struct DirElt {
    pub(crate) dir: Vec<u8>,
    moved: bool,
}

/// Splits a search path.  kpathsea does this on every lookup; here it is done once per path.
pub(crate) fn split_elts(path: &[u8]) -> Vec<Elt> {
    path_elements(path)
        .into_iter()
        .map(|e| {
            let (e, disk) = match e.strip_prefix(b"!!") {
                Some(rest) => (rest, false),
                None => (e, true),
            };
            let mut text = e.to_vec();
            normalize_path(&mut text);
            Elt { text, disk }
        })
        .collect()
}

/// `str_llist_float`: moves the directory where a file was found towards the top, after the others
/// moved so far.  The next directory to look at stays the same, as the C remembers it beforehand.
fn float(dirs: &mut [DirElt], mover: usize) {
    if dirs[mover].moved {
        return;
    }
    let unmoved = dirs.iter().position(|d| !d.moved).unwrap_or(mover);
    dirs[unmoved..=mover].rotate_right(1);
    dirs[unmoved].moved = true;
}

/// `dir_list_add`: every directory in a list ends in a separator.
fn dir_list_add(out: &mut Vec<DirElt>, dir: &[u8]) {
    let mut d = dir.to_vec();
    let last = at(dir, dir.len().wrapping_sub(1));
    if !(is_dir_sep_ch(last) || is_device_sep_g::<DOSISH>(last)) {
        d.push(b'/');
    }
    out.push(DirElt { dir: d, moved: false });
}

/// `expand_elt`.  `leaf` is the value of `texmf_nlink_for_leaf`.  `scans` counts directories opened.
fn expand_elt(out: &mut Vec<DirElt>, elt: &[u8], start: usize, leaf: bool, scans: &mut u64) {
    let mut dir = start;
    while at(elt, dir) != 0 {
        if is_dir_sep_ch(at(elt, dir)) && is_dir_sep_ch(at(elt, dir + 1)) {
            // Two or more consecutive /'s: find subdirectories.
            let mut post = dir + 1;
            while is_dir_sep_ch(at(elt, post)) {
                post += 1;
            }
            do_subdir(out, elt, dir + 1, &elt[post..], leaf, scans);
            return;
        }
        dir += 1;
    }
    // At the end of elt it is a normal file name.
    if sys::is_dir(elt) {
        dir_list_add(out, elt);
    }
}

/// `do_subdir`.  Appends the subdirectories of `elt[..elt_length]`, which ends in a /.  `post` is
/// what follows the doubled /.  C stats every entry; the directory entry's type makes that needless.
fn do_subdir(out: &mut Vec<DirElt>, elt: &[u8], elt_length: usize, post: &[u8], leaf: bool, scans: &mut u64) {
    let mut name = elt[..elt_length].to_vec();
    *scans += 1;
    let Ok(rd) = std::fs::read_dir(sys::path_of(&name)) else { return };
    // Include the top level before subdirectories, if nothing to match.
    if post.is_empty() {
        dir_list_add(out, &name);
    } else {
        // If there is something to match, see if it exists here.
        name.extend_from_slice(post);
        expand_elt(out, &name, elt_length, leaf, scans);
        name.truncate(elt_length);
    }
    for e in rd.flatten() {
        let file = sys::os_bytes(&e.file_name());
        // A name beginning with `.` allows hidden directories.
        if file.first() == Some(&b'.') || !sys::entry_is_dir(&e) {
            continue;
        }
        name.extend_from_slice(&file);
        let links = if leaf { sys::dir_nlink(&name) } else { 0 };
        name.push(b'/');
        let potential_len = name.len();
        if !post.is_empty() {
            name.extend_from_slice(post);
            expand_elt(out, &name, potential_len, leaf, scans);
            name.truncate(potential_len);
        }
        // A directory with two links is a leaf: no . and .. in any subdirectory.
        if !leaf || links != 2 {
            do_subdir(out, &name, potential_len, post, leaf, scans);
        } else if post.is_empty() {
            dir_list_add(out, &name);
        }
        name.truncate(elt_length);
    }
}

/// `casefold_readable_file`: the name with its last component matched case-insensitively.
fn casefold_readable_file(name: &[u8], scans: &mut u64) -> Option<Vec<u8>> {
    let base = xbasename(name);
    let dir = xdirname(name);
    // A directory that does not exist is fine: the name might be subdir/somefile.
    *scans += 1;
    let rd = std::fs::read_dir(sys::path_of(&dir)).ok()?;
    for e in rd.flatten() {
        let entry = sys::os_bytes(&e.file_name());
        if entry.eq_ignore_ascii_case(base) {
            let mut ret = [&dir[..], b"/", &entry[..]].concat();
            if readable_file(&mut ret) {
                return Some(ret);
            }
        }
    }
    None
}

/// `dir_list_search_list`.  Absolute and explicitly relative names were handled by the caller.
fn dir_list_search_list(dirs: &mut [DirElt], names: &[Vec<u8>], all: bool, casefold: bool, scans: &mut u64) -> Vec<Vec<u8>> {
    let mut ret = Vec::new();
    let mut i = 0;
    while i < dirs.len() {
        for name in names {
            if absolute_p(name, true) {
                continue;
            }
            let potential = [&dirs[i].dir[..], &name[..]].concat();
            let found = if casefold {
                casefold_readable_file(&potential, scans)
            } else {
                let mut p = potential;
                if readable_file(&mut p) { Some(p) } else { None }
            };
            if let Some(f) = found {
                ret.push(f);
                float(dirs, i);
                if !all {
                    return ret;
                }
            }
        }
        i += 1;
    }
    ret
}

impl Kpse {
    /// `texmf_casefold_search`, which C looks up on each miss.
    pub(crate) fn casefold_on(&mut self) -> bool {
        if let Some(v) = self.casefold_memo {
            return v;
        }
        let v = self.var_value(b"texmf_casefold_search");
        let r = cnf_p(v.as_deref());
        if self.cnf_read && !self.doing_cnf_init {
            self.casefold_memo = Some(r);
        }
        r
    }

    /// `kpathsea_element_dirs`.  `None` means the element is empty.
    pub(crate) fn element_dirs(&mut self, elt: &[u8]) -> Option<usize> {
        if elt.is_empty() {
            return None;
        }
        let mut e = elt.to_vec();
        let i = normalize_path(&mut e);
        let key = fold_key(&e);
        if let Some(&id) = self.elt_cache.get(&key) {
            return Some(id);
        }
        let leaf = !DOSISH && {
            let v = self.var_value(b"texmf_nlink_for_leaf");
            cnf_p(v.as_deref())
        };
        let mut list = Vec::new();
        expand_elt(&mut list, &e, i, leaf, &mut self.dir_scans);
        let id = self.dir_lists.len();
        self.dir_lists.push(list);
        self.elt_cache.insert(key, id);
        Some(id)
    }

    /// `absolute_search`.
    fn absolute_search(&mut self, name: &[u8]) -> Option<Vec<u8>> {
        let mut found = name.to_vec();
        if readable_file(&mut found) {
            return Some(found);
        }
        // Only if needed and enabled.  Windows is case-insensitive already.
        if !DOSISH && self.casefold_on() {
            return casefold_readable_file(&found, &mut self.dir_scans);
        }
        None
    }

    /// The disk half of one element's search, shared by the two search functions of pathsearch.c.
    fn elt_disk_search(&mut self, elt: &Elt, names: &[Vec<u8>], all: bool, found: &mut Option<Vec<Vec<u8>>>) {
        let Some(id) = self.element_dirs(&elt.text) else { return };
        if self.dir_lists[id].is_empty() {
            return;
        }
        let mut r = dir_list_search_list(&mut self.dir_lists[id], names, all, false, &mut self.dir_scans);
        if r.is_empty() && !DOSISH && self.casefold_on() {
            // Nothing found: search again, case-insensitively.
            r = dir_list_search_list(&mut self.dir_lists[id], names, all, true, &mut self.dir_scans);
        }
        *found = Some(r);
    }

    /// `kpathsea_path_search_list_generic`.
    pub(crate) fn path_search_list_generic(&mut self, elts: &[Elt], names: &[Vec<u8>], must_exist: bool, all: bool) -> Vec<Vec<u8>> {
        let mut ret: Vec<Vec<u8>> = Vec::new();
        let mut all_absolute = true;
        let mut finished = false;
        // First catch any absolute or explicit relative names.
        for name in names {
            if absolute_p(name, true) {
                if let Some(f) = self.absolute_search(name) {
                    ret.push(f);
                    if !all {
                        finished = true;
                        break;
                    }
                }
            } else {
                all_absolute = false;
            }
        }
        if !finished && !all_absolute {
            for elt in elts {
                // Try ls-R, unless we are searching for texmf.cnf.
                let mut found = if self.followup_search { self.db_search_list(names, &elt.text, all) } else { None };
                // Search the file system if the path allows it, and either no db is relevant or
                // the name is not in it and must exist.
                if elt.disk && (found.is_none() || (must_exist && found.as_ref().is_some_and(Vec::is_empty))) {
                    self.elt_disk_search(elt, names, all, &mut found);
                }
                if let Some(mut f) = found {
                    if !f.is_empty() {
                        if all {
                            ret.append(&mut f);
                        } else {
                            ret.push(f.swap_remove(0));
                            break;
                        }
                    }
                }
            }
        }
        // Uniqify, since our paths can often end up finding the same file more than once.
        let mut uniq: Vec<Vec<u8>> = Vec::with_capacity(ret.len());
        for r in ret {
            if !uniq.iter().any(|u| file_str_eq(u, &r)) {
                uniq.push(r);
            }
        }
        self.followup_search = true;
        uniq
    }

    /// `path_search`, for one name.
    fn path_search_elts(&mut self, elts: &[Elt], name: &[u8], must_exist: bool, all: bool) -> Vec<Vec<u8>> {
        let names = [name.to_vec()];
        let mut ret = Vec::new();
        for elt in elts {
            let mut found = if self.followup_search { self.db_search(name, &elt.text, all) } else { None };
            if elt.disk && (found.is_none() || (must_exist && found.as_ref().is_some_and(Vec::is_empty))) {
                self.elt_disk_search(elt, &names, all, &mut found);
            }
            if let Some(mut f) = found {
                if !f.is_empty() {
                    if all {
                        ret.append(&mut f);
                    } else {
                        ret.push(f.swap_remove(0));
                        break;
                    }
                }
            }
        }
        ret
    }

    /// pathsearch.c's `search`.
    fn search(&mut self, path: &[u8], original_name: &[u8], must_exist: bool, all: bool) -> Vec<Vec<u8>> {
        // Make a leading ~ count as an absolute file name, and expand $FOO's.
        let name = self.expand(original_name);
        let ret = if absolute_p(&name, true) {
            self.absolute_search(&name).into_iter().collect()
        } else {
            let elts = split_elts(path);
            self.path_search_elts(&elts, &name, must_exist, all)
        };
        self.followup_search = true;
        ret
    }

    /// `kpathsea_path_search`: the first `name` along a path given as text, which must already be
    /// brace expanded.
    pub fn path_search(&mut self, path: &[u8], name: &[u8], must_exist: bool) -> Option<Vec<u8>> {
        self.ensure_cnf();
        self.search(path, cut_at_nul(name), must_exist, false).into_iter().next()
    }

    /// `kpathsea_all_path_search`: every `name` along a path.
    pub fn all_path_search(&mut self, path: &[u8], name: &[u8]) -> Vec<Vec<u8>> {
        self.ensure_cnf();
        self.all_path_search_str(path, cut_at_nul(name))
    }

    /// `kpathsea_all_path_search` during initialization: no asking for the cnf files.
    pub(crate) fn all_path_search_str(&mut self, path: &[u8], name: &[u8]) -> Vec<Vec<u8>> {
        self.search(path, name, true, true)
    }

    /// `kpathsea_init_format`: the final search path of a format.  `None` for an unknown format.
    pub(crate) fn init_format(&mut self, format: i32) -> Option<Arc<FormatState>> {
        let def = format::format_def(format)?;
        if let Some(st) = &self.formats[format as usize] {
            return Some(st.clone());
        }
        if format == format::KPSE_PK_FORMAT || format == format::KPSE_ANY_GLYPH_FORMAT {
            self.init_maketex(format);
        }
        let mut path = self.init_path(format, def);
        if format == format::KPSE_DB_FORMAT {
            path = remove_dbonly(&path);
        }
        let elts = split_elts(&path);
        let st = Arc::new(FormatState { path, elts });
        self.formats[format as usize] = Some(st.clone());
        Some(st)
    }

    /// `init_path`.
    fn init_path(&mut self, format: i32, def: &FormatDef) -> Vec<u8> {
        let prog = self.program_name.clone();
        let upper = prog.to_ascii_uppercase();
        let program_env = [&upper[..], b"INPUTS"].concat();
        let default_path: Vec<u8> = match def.default {
            DefaultPath::Nonesuch => format::DEFAULT_NONESUCH.to_vec(),
            DefaultPath::Cnf => self.tree.cnf_default.clone().unwrap_or_else(|| format::DEFAULT_TEXMFCNF.to_vec()),
            DefaultPath::TrFonts => format::DEFAULT_TRFONTS.to_vec(),
            DefaultPath::CLua => format::DEFAULT_CLUAINPUTS.to_vec(),
            DefaultPath::Program => [b".", &[ENV_SEP][..], b"$TEXMF/", &prog[..], b"//"].concat(),
        };
        let envs: Vec<&[u8]> =
            if def.program_envs { vec![&program_env[..]] } else { def.envs.iter().map(|s| s.as_bytes()).collect() };

        // The first envvar set to a nonempty value; the first cnf entry that matches.
        let mut var: Option<Vec<u8>> = None;
        let mut cnf_path: Option<Vec<u8>> = None;
        for env_name in envs {
            if var.is_none() {
                var = self
                    .getenv(&[env_name, b".", &prog].concat())
                    .filter(|v| !v.is_empty())
                    .or_else(|| self.getenv(&[env_name, b"_", &prog].concat()).filter(|v| !v.is_empty()))
                    .or_else(|| self.getenv(env_name).filter(|v| !v.is_empty()));
            }
            // If we are initializing the cnf path, do not try to get values from the cnf files.
            if cnf_path.is_none() && format != format::KPSE_CNF_FORMAT {
                cnf_path = self.cnf_get(env_name);
            }
            if var.is_some() && cnf_path.is_some() {
                break;
            }
        }

        // For each level, an extra : is replaced with the path at the next lower level.
        let mut path = default_path;
        if let Some(c) = cnf_path {
            path = expand_default(Some(&c), &path);
        }
        if let Some(mut v) = var {
            if is_env_sep(b':') {
                for c in v.iter_mut() {
                    if *c == b';' {
                        *c = b':';
                    }
                }
            }
            path = expand_default(Some(&v), &path);
        }
        if let Some(over) = self.override_paths[format as usize].clone() {
            path = expand_default(Some(&over), &path);
        }
        self.brace_expand(&path)
    }

    /// `target_fontmaps`.
    fn target_fontmaps(&mut self, target: &mut Vec<Vec<u8>>, name: &[u8]) {
        if let Some(mapped) = self.fontmap_lookup(name) {
            target.extend(mapped);
        }
    }

    /// `target_asis_name`.
    fn target_asis_name(&mut self, target: &mut Vec<Vec<u8>>, def: &FormatDef, name: &[u8], use_fontmaps: bool, has_potential_suffix: bool) {
        if has_potential_suffix || !def.suffix_search_only {
            target.push(name.to_vec());
            if use_fontmaps {
                self.target_fontmaps(target, name);
            }
        }
    }

    /// `target_suffixed_names`.
    fn target_suffixed_names(&mut self, target: &mut Vec<Vec<u8>>, def: &FormatDef, name: &[u8], use_fontmaps: bool, has_potential_suffix: bool) {
        if has_potential_suffix || def.suffix.is_empty() {
            return;
        }
        for ext in def.suffix {
            let with_suffix = [name, ext.as_bytes()].concat();
            target.push(with_suffix.clone());
            if use_fontmaps {
                self.target_fontmaps(target, &with_suffix);
            }
        }
    }

    /// `kpathsea_find_file`.
    pub fn find_file(&mut self, name: &[u8], format: i32, must_exist: bool) -> Option<Vec<u8>> {
        self.find_file_generic(name, format, must_exist, false).into_iter().next()
    }

    /// `kpathsea_find_file_generic`.  With `all` every match is returned, otherwise at most one.
    /// Where C would then run `mktex*`, this returns nothing: only a bitmap font is ever made, by [`Kpse::find_glyph`].
    pub fn find_file_generic(&mut self, const_name: &[u8], format: i32, must_exist: bool, all: bool) -> Vec<Vec<u8>> {
        let Some(def) = format::format_def(format) else {
            self.warn(format!("kpse_init_format: Unknown format {format}").as_bytes());
            return Vec::new();
        };
        let use_fontmaps = matches!(
            format,
            format::KPSE_TFM_FORMAT | format::KPSE_GF_FORMAT | format::KPSE_PK_FORMAT | format::KPSE_OFM_FORMAT
        );
        let Some(st) = self.init_format(format) else { return Vec::new() };

        // Do variable and tilde expansion.
        let name = self.expand(cut_at_nul(const_name));

        let dir_sep = b'/';
        let has_any_suffix = match name.iter().rposition(|&c| c == b'.') {
            Some(d) => !name[d..].contains(&dir_sep),
            None => false,
        };

        // Does the name already end in a possible suffix?
        let ends_with = |ext: &&str| {
            let e = ext.as_bytes();
            name.len() >= e.len() && file_str_eq(e, &name[name.len() - e.len()..])
        };
        let has_potential_suffix = def.suffix.iter().any(ends_with) || def.alt_suffix.iter().any(ends_with);

        // The list of target names, in an order that depends on try_std_extension_first.
        let mut target: Vec<Vec<u8>> = Vec::new();
        let try_std = self.var_value(b"try_std_extension_first");
        if has_any_suffix && !cnf_p(try_std.as_deref()) {
            self.target_asis_name(&mut target, def, &name, use_fontmaps, has_potential_suffix);
            self.target_suffixed_names(&mut target, def, &name, use_fontmaps, has_potential_suffix);
        } else {
            self.target_suffixed_names(&mut target, def, &name, use_fontmaps, has_potential_suffix);
            self.target_asis_name(&mut target, def, &name, use_fontmaps, has_potential_suffix);
        }

        // Search, trying to minimize disk-pounding.
        let mut ret = self.path_search_list_generic(&st.elts, &target, false, all);

        // Do we need to pound the disk?
        if ret.is_empty() && must_exist {
            target.clear();
            // A subset of the previous names; no alternate names from the fontmaps.
            if !has_potential_suffix && def.suffix_search_only {
                for ext in def.suffix {
                    target.push([&name[..], ext.as_bytes()].concat());
                }
            }
            if has_potential_suffix || !def.suffix_search_only {
                target.push(name.clone());
            }
            ret = self.path_search_list_generic(&st.elts, &target, true, all);
        }
        ret
    }
}

/// C strings end at the first NUL.
fn cut_at_nul(s: &[u8]) -> &[u8] {
    match s.iter().position(|&c| c == 0) {
        Some(n) => &s[..n],
        None => s,
    }
}

/// `remove_dbonly`: the `!!` of each path element.
fn remove_dbonly(path: &[u8]) -> Vec<u8> {
    let mut ret = Vec::with_capacity(path.len());
    let mut new_elt = true;
    let mut p = 0;
    while p < path.len() {
        if new_elt && path[p] == b'!' && at(path, p + 1) == b'!' {
            p += 2;
        } else {
            new_elt = path[p] == ENV_SEP;
            ret.push(path[p]);
            p += 1;
        }
    }
    ret
}

#[cfg(test)]
mod tests {
    use super::*;

    fn list(names: &[&[u8]]) -> Vec<DirElt> {
        names.iter().map(|n| DirElt { dir: n.to_vec(), moved: false }).collect()
    }
    fn order(d: &[DirElt]) -> Vec<Vec<u8>> {
        d.iter().map(|e| e.dir.clone()).collect()
    }

    #[test]
    fn float_as_str_llist_c() {
        let mut d = list(&[b"a", b"b", b"c", b"d"]);
        float(&mut d, 2);
        assert_eq!(order(&d), vec![b"c".to_vec(), b"a".to_vec(), b"b".to_vec(), b"d".to_vec()]);
        // The next one goes after the first moved, not before it.
        float(&mut d, 3);
        assert_eq!(order(&d), vec![b"c".to_vec(), b"d".to_vec(), b"a".to_vec(), b"b".to_vec()]);
        // Moving a moved element is a no-op.
        float(&mut d, 0);
        assert_eq!(order(&d), vec![b"c".to_vec(), b"d".to_vec(), b"a".to_vec(), b"b".to_vec()]);
        // The first unmoved element only gets marked.
        float(&mut d, 2);
        assert_eq!(order(&d), vec![b"c".to_vec(), b"d".to_vec(), b"a".to_vec(), b"b".to_vec()]);
        assert!(d[2].moved);
    }

    #[test]
    fn dbonly_marks_are_removed() {
        let s = |t: &str| t.replace(':', std::str::from_utf8(&[ENV_SEP]).unwrap()).into_bytes();
        assert_eq!(remove_dbonly(&s("!!a:b:!!c")), s("a:b:c"));
        assert_eq!(remove_dbonly(&s("a!!b")), s("a!!b"));
        assert_eq!(remove_dbonly(b""), b"");
    }

    #[test]
    fn elements_keep_the_disk_mark_and_normalize() {
        let sep = std::str::from_utf8(&[ENV_SEP]).unwrap();
        let e = split_elts(format!(".{sep}!!/a//{sep}//b/c{sep}").as_bytes());
        assert_eq!(e.len(), 4);
        assert!(e[0].disk && !e[1].disk && e[2].disk && e[3].disk);
        assert_eq!(e[1].text, b"/a//");
        assert_eq!(e[2].text, b"/b/c");
        assert_eq!(e[3].text, b"");
    }
}
