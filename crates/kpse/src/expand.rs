//! Variable, tilde and brace expansion: variable.c, tilde.c, expand.c.
use crate::Kpse;
use crate::names::*;
use crate::sys;

/// `IS_VAR_CHAR`.
#[inline]
fn is_var_char(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_'
}

impl Kpse {
    /// `kpathsea_var_value`: `VAR.prog`, `VAR_prog`, `VAR` from the environment, then the cnf files
    /// (`VAR.prog`, then `VAR`); the value found has its variables and tilde expanded.
    pub fn var_value(&mut self, var: &[u8]) -> Option<Vec<u8>> {
        let prog = self.program_name.clone();
        let mut v = self.getenv(&[var, b".", &prog].concat()).filter(|s| !s.is_empty());
        if v.is_none() {
            v = self.getenv(&[var, b"_", &prog].concat()).filter(|s| !s.is_empty());
        }
        if v.is_none() {
            v = self.getenv(var).filter(|s| !s.is_empty());
        }
        if v.is_none() {
            v = self.cnf_get(var);
        }
        v.map(|value| self.expand(&value))
    }

    /// `expanding`.
    fn set_expanding(&mut self, var: &[u8], xp: bool) {
        for e in self.expansions.iter_mut() {
            if e.0 == var {
                e.1 = xp;
                return;
            }
        }
        self.expansions.push((var.to_vec(), xp));
    }

    /// `expanding_p`.
    fn expanding_p(&self, var: &[u8]) -> bool {
        self.expansions.iter().find(|e| e.0 == var).is_some_and(|e| e.1)
    }

    /// variable.c's `expand`: appends the value of `var` and says whether it was set.
    fn expand_var(&mut self, out: &mut Vec<u8>, var: &[u8]) -> bool {
        if self.expanding_p(var) {
            self.warn(&[b"kpathsea: variable `", var, b"' references itself (eventually)"].concat());
            return false;
        }
        let prog = self.program_name.clone();
        let mut value = self.getenv(&[var, b"_", &prog].concat()).filter(|s| !s.is_empty());
        if value.is_none() {
            value = self.getenv(var).filter(|s| !s.is_empty());
        }
        if value.is_none() {
            value = self.cnf_get(var);
        }
        match value {
            Some(v) => {
                self.set_expanding(var, true);
                let tmp = self.expand(&v);
                self.set_expanding(var, false);
                out.extend_from_slice(&tmp);
                true
            }
            None => false,
        }
    }

    /// `kpathsea_var_expand`.
    pub fn var_expand(&mut self, src: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(src.len());
        let mut s = 0;
        while s < src.len() {
            if src[s] == b'$' {
                s += 1;
                let c = at(src, s);
                if is_var_char(c) {
                    // $V: collect name constituents, then expand.
                    let mut var_end = s + 1;
                    while is_var_char(at(src, var_end)) {
                        var_end += 1;
                    }
                    if !self.expand_var(&mut out, &src[s..var_end]) {
                        // The literal construct, so that file names holding dollars can be read.
                        out.extend_from_slice(&src[s - 1..var_end]);
                    }
                    s = var_end - 1;
                } else if c == b'{' {
                    // ${: scan ahead for the matching delimiter, then expand.
                    s += 1;
                    let mut var_end = s;
                    while at(src, var_end) != 0 && at(src, var_end) != b'}' {
                        var_end += 1;
                    }
                    if at(src, var_end) == 0 {
                        self.warn(&[b"kpathsea: ", src, b": No matching } for ${"].concat());
                        s = var_end - 1;
                    } else {
                        self.expand_var(&mut out, &src[s..var_end]);
                        s = var_end;
                    }
                } else {
                    // $<something-else>: warn, but keep the characters.  At the end of the string C
                    // copies the terminator, so the result stops there.  Its `%c` prints that terminator too, a zero byte.
                    let mut msg = [b"kpathsea: ", src, b": Unrecognized variable construct `$"].concat();
                    msg.push(c);
                    msg.push(b'\'');
                    self.warn(&msg);
                    out.push(b'$');
                    if c == 0 {
                        break;
                    }
                    out.push(c);
                }
            } else {
                out.push(src[s]);
            }
            s += 1;
        }
        out
    }

    /// `kpathsea_expand`: variables first, so `~${USER}` works.
    pub fn expand(&mut self, s: &[u8]) -> Vec<u8> {
        let var_expansion = self.var_expand(s);
        self.tilde_expand(var_expansion)
    }

    /// `kpathsea_tilde_expand`.
    pub(crate) fn tilde_expand(&mut self, name_in: Vec<u8>) -> Vec<u8> {
        let (prefix, name): (&[u8], &[u8]) =
            if name_in.starts_with(b"!!") { (b"!!", &name_in[2..]) } else { (b"", &name_in[..]) };
        if at(name, 0) != b'~' {
            return if DOSISH { slashify(name_in) } else { name_in };
        }
        let mut c;
        let mut home: Vec<u8>;
        if at(name, 1) != 0 && !is_dir_sep(at(name, 1)) {
            // `~user` or `~user/`: look the user up.
            c = 2;
            while !is_dir_sep(at(name, c)) && at(name, c) != 0 {
                c += 1;
            }
            home = sys::home_of_user(&name[1..c]).unwrap_or_else(|| b".".to_vec());
        } else {
            c = 1;
            home = self.getenv(sys::HOME_VAR).unwrap_or_else(|| b".".to_vec());
        }
        // Handle a leading //.
        if is_dir_sep(at(&home, 0)) && is_dir_sep(at(&home, 1)) {
            home.remove(0);
        }
        // If HOME ends in /, omit the / in ~/ or ~user/.
        if at(name, c) != 0 && home.last().is_some_and(|&l| is_dir_sep(l)) {
            c += 1;
        }
        let expansion = [prefix, &home[..], &name[c.min(name.len())..]].concat();
        if DOSISH { slashify(expansion) } else { expansion }
    }

    /// `kpathsea_brace_expand`: variables first, then each path element.
    pub fn brace_expand(&mut self, path: &[u8]) -> Vec<u8> {
        let xpath = self.var_expand(path);
        let mut ret = Vec::new();
        let elts: Vec<Vec<u8>> = path_elements(&xpath).into_iter().map(<[u8]>::to_vec).collect();
        for elt in &elts {
            let expansion = self.brace_expand_element(elt, 0);
            ret.extend_from_slice(&expansion);
            ret.push(ENV_SEP);
        }
        ret.pop();
        self.expand_kpse_dot(ret)
    }

    /// `kpathsea_brace_expand_element`.  C says this recursion must terminate since recursive variables
    /// are not allowed, but `FOO = $FOO/x` gives a longer string each time, and kpsewhich does not
    /// return from it.  Here the recursion stops at a depth no real value comes near.
    fn brace_expand_element(&mut self, elt: &[u8], depth: u32) -> Vec<u8> {
        const MAX_DEPTH: u32 = 64;
        let mut pos = 0;
        let expansions = self.brace_expand_list(elt, &mut pos);
        let mut ret = Vec::new();
        for e in &expansions {
            let mut x = self.expand(e);
            if x != *e {
                if depth < MAX_DEPTH {
                    // A variable's value may hold braces, like $TEXMF.
                    x = self.brace_expand_element(&x, depth + 1);
                } else {
                    self.warn(&[b"kpathsea: ", elt, b": expansion does not end"].concat());
                }
            }
            ret.extend_from_slice(&x);
            ret.push(ENV_SEP);
        }
        ret.pop();
        ret
    }

    /// expand.c's `brace_expand`.  `text` is an index into `s`, as the C's is a pointer.
    fn brace_expand_list(&mut self, s: &[u8], text: &mut usize) -> Vec<Vec<u8>> {
        let mut result: Vec<Vec<u8>> = Vec::new();
        let mut partial: Vec<Vec<u8>> = Vec::new();
        let mut p = *text;
        while at(s, p) != 0 && at(s, p) != b'}' {
            let c = at(s, p);
            if c == ENV_SEP || c == b',' {
                expand_append(&mut partial, &s[*text..p]);
                result.append(&mut partial);
                *text = p + 1;
            } else if c == b'{' {
                expand_append(&mut partial, &s[*text..p]);
                p += 1;
                let recurse = self.brace_expand_list(s, &mut p);
                concat_elements(&mut partial, recurse);
                if at(s, p) != b'}' {
                    self.warn(&[b"kpathsea: ", &s[*text..], b": Unmatched {"].concat());
                    p -= 1;
                }
                *text = p + 1;
            } else if c == b'$' && at(s, p + 1) == b'{' {
                // Skip ${VAR}.  C runs off the string when the brace is unmatched.
                p += 2;
                while at(s, p) != 0 && at(s, p) != b'}' {
                    p += 1;
                }
                if at(s, p) == 0 {
                    break;
                }
            }
            p += 1;
        }
        expand_append(&mut partial, &s[(*text).min(p)..p]);
        result.append(&mut partial);
        *text = p;
        result
    }

    /// `kpathsea_expand_kpse_dot`: `$KPSE_DOT` goes before any relative path component.
    fn expand_kpse_dot(&mut self, path: Vec<u8>) -> Vec<u8> {
        let Some(dot) = self.getenv(b"KPSE_DOT") else { return path };
        let mut ret = Vec::new();
        for elt in path_elements(&path) {
            if absolute_p(elt, false) || elt.starts_with(b"!!") {
                ret.extend_from_slice(elt);
                ret.push(ENV_SEP);
            } else if elt == b"." {
                ret.extend_from_slice(&dot);
                ret.push(ENV_SEP);
            } else if at(elt, 0) == b'.' && is_dir_sep(at(elt, 1)) {
                ret.extend_from_slice(&dot);
                ret.extend_from_slice(&elt[1..]);
                ret.push(ENV_SEP);
            } else if !elt.is_empty() {
                ret.extend_from_slice(&dot);
                ret.push(b'/');
                ret.extend_from_slice(elt);
                ret.push(ENV_SEP);
            }
        }
        ret.pop();
        ret
    }

    /// `kpathsea_path_expand`: every construct expanded, and only directories that exist.
    pub fn path_expand(&mut self, path: &[u8]) -> Vec<u8> {
        let ypath = if DOSISH { slashify(path.to_vec()) } else { path.to_vec() };
        let xpath = self.brace_expand(&ypath);
        let mut ret: Vec<u8> = Vec::new();
        let elts: Vec<Vec<u8>> = path_elements(&xpath).into_iter().map(<[u8]>::to_vec).collect();
        for elt in &elts {
            let elt = elt.strip_prefix(b"!!").unwrap_or(elt);
            let Some(id) = self.element_dirs(elt) else { continue };
            for d in &self.dir_lists[id] {
                let thedir = &d.dir;
                let dirlen = thedir.len();
                // Keep the trailing slash of a root directory.
                if dirlen == 1 || (dirlen == 3 && name_begins_with_device_g::<DOSISH>(thedir) && is_dir_sep(thedir[2])) {
                    ret.extend_from_slice(thedir);
                } else {
                    ret.extend_from_slice(&thedir[..dirlen - 1]);
                }
                ret.push(ENV_SEP);
            }
        }
        ret.pop();
        ret
    }
}

/// On Windows expansions use `/` throughout.
fn slashify(mut s: Vec<u8>) -> Vec<u8> {
    for c in s.iter_mut() {
        if *c == b'\\' {
            *c = b'/';
        }
    }
    s
}

/// `expand_append`.
fn expand_append(partial: &mut Vec<Vec<u8>>, piece: &[u8]) {
    concat_elements(partial, vec![piece.to_vec()]);
}

/// `str_list_concat_elements`: each of `more` after each of `target`, the first index varying fastest.
fn concat_elements(target: &mut Vec<Vec<u8>>, more: Vec<Vec<u8>>) {
    if more.is_empty() {
        return;
    }
    if target.is_empty() {
        *target = more;
        return;
    }
    let mut out = Vec::with_capacity(target.len() * more.len());
    for m in &more {
        for t in target.iter() {
            out.push([&t[..], &m[..]].concat());
        }
    }
    *target = out;
}

/// `kpse_cnf_p`: true if the string is loosely true.
pub fn cnf_p(s: Option<&[u8]>) -> bool {
    matches!(s, Some(v) if !v.is_empty() && v[0] != b'f' && v[0] != b'0')
}
