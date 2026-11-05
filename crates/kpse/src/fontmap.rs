//! `texfonts.map`: fontmap.c.  Font name aliases for TFM lookups.
use crate::Kpse;
use crate::format::KPSE_FONTMAP_FORMAT;
use crate::names::*;
use crate::sys;
use std::collections::HashMap;

/// fontmap.c's `token`: the next whitespace-delimited word, or the empty string.
fn token(s: &[u8]) -> &[u8] {
    let mut p = 0;
    while is_space(at(s, p)) {
        p += 1;
    }
    let start = p;
    while at(s, p) != 0 && !is_space(at(s, p)) {
        p += 1;
    }
    &s[start.min(s.len())..p.min(s.len())]
}

fn find_sub(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

impl Kpse {
    /// `map_file_parse`.
    fn map_file_parse(&mut self, map: &mut HashMap<Vec<u8>, Vec<Vec<u8>>>, path: &[u8], map_path: &[u8], depth: u32) {
        // C has no limit, and a file including itself never returns from there.
        if depth > 64 {
            return;
        }
        let Ok(text) = std::fs::read(sys::path_of(path)) else { return };
        self.recorded.push(path.to_vec());
        self.map_files.push(path.to_vec());
        for (lineno, mut line) in read_lines(&text).into_iter().enumerate() {
            // Ignore anything after a % or @c.
            let comment = line.iter().rposition(|&c| c == b'%').or_else(|| find_sub(&line, b"@c"));
            if let Some(c) = comment {
                line.truncate(c);
            }
            let mut l = 0;
            while is_space(at(&line, l)) {
                l += 1;
            }
            let rest = &line[l.min(line.len())..];
            let filename = token(rest);
            let alias = token(&rest[filename.len()..]);
            if filename == b"include" {
                match self.search_for_include(map_path, alias) {
                    Some(inc) => self.map_file_parse(map, &inc, map_path, depth + 1),
                    None => {
                        let n = (lineno + 1).to_string();
                        self.warn(&[b"kpathsea: ", path, b":", n.as_bytes(), b": Can't find fontname include file `", alias, b"'"].concat());
                    }
                }
            } else {
                // A blank line has an empty name and an empty alias, as in C, and they are inserted.
                map.entry(fold_key(alias)).or_default().push(filename.to_vec());
            }
        }
    }

    fn search_for_include(&mut self, map_path: &[u8], alias: &[u8]) -> Option<Vec<u8>> {
        self.path_search(map_path, alias, false)
    }

    /// `read_all_maps`.  Entries in earlier files override later ones.
    fn read_all_maps(&mut self) {
        let Some(st) = self.init_format(KPSE_FONTMAP_FORMAT) else { return };
        let files = self.all_path_search_str(&st.path, b"texfonts.map");
        let mut map = HashMap::new();
        for f in &files {
            self.map_file_parse(&mut map, f, &st.path, 0);
        }
        self.map = Some(map);
    }

    /// `kpathsea_fontmap_lookup`: `key`, or else `key` without its suffix.  Any original suffix is
    /// appended to the names found.
    pub(crate) fn fontmap_lookup(&mut self, key: &[u8]) -> Option<Vec<Vec<u8>>> {
        if self.map.is_none() {
            self.read_all_maps();
        }
        let map = self.map.as_ref()?;
        let suffix = find_suffix(key);
        let mut ret = map.get(&fold_key(key));
        if ret.is_none() && suffix.is_some() {
            ret = map.get(&fold_key(remove_suffix(key)));
        }
        let ret = ret?;
        Some(match suffix {
            Some(s) => ret.iter().map(|e| extend_filename(e, s)).collect(),
            None => ret.clone(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens() {
        assert_eq!(token(b"  foo bar"), b"foo");
        assert_eq!(token(b"foo"), b"foo");
        assert_eq!(token(b""), b"");
        assert_eq!(token(b"   "), b"");
    }
}
