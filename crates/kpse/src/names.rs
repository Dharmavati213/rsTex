//! String helpers with no state: c-pathch.h, absolute.c, xdirname.c, xbasename.c, find-suffix.c,
//! rm-suffix.c, extend-fname.c, kdefault.c, path-elt.c, line.c.
//!
//! What differs on a DOS-like system is a const parameter `DOS`, so the Windows rules of these pure
//! functions can be exercised on any machine.  The plain names use the host's rules.

/// `DOSISH` and `MONOCASE_FILENAMES`, which kpathsea defines together on Windows.
pub const DOSISH: bool = cfg!(windows);

/// `ENV_SEP`.
pub const ENV_SEP: u8 = if DOSISH { b';' } else { b':' };

/// The byte at `i`, or 0 past the end: the C string's terminator.
#[inline]
pub fn at(s: &[u8], i: usize) -> u8 {
    s.get(i).copied().unwrap_or(0)
}

/// `IS_ENV_SEP`.
#[inline]
pub fn is_env_sep(c: u8) -> bool {
    c == ENV_SEP
}

/// `IS_KPSE_SEP`: a cnf file may use `;` or `:` on Unix, but `:` is the drive separator on Windows.
#[inline]
pub fn is_kpse_sep(c: u8) -> bool {
    if DOSISH { c == b';' } else { c == b':' || c == b';' }
}

/// `IS_DIR_SEP`.
#[inline]
pub const fn is_dir_sep_g<const DOS: bool>(c: u8) -> bool {
    c == b'/' || (DOS && c == b'\\')
}

/// `IS_DIR_SEP` for the host.
#[inline]
pub const fn is_dir_sep(c: u8) -> bool {
    is_dir_sep_g::<DOSISH>(c)
}

/// `IS_DIR_SEP_CH`: only `/`, since Windows names are slashified before this is asked.
#[inline]
pub const fn is_dir_sep_ch(c: u8) -> bool {
    c == b'/'
}

/// `IS_DEVICE_SEP`.
#[inline]
pub const fn is_device_sep_g<const DOS: bool>(c: u8) -> bool {
    DOS && c == b':'
}

/// `NAME_BEGINS_WITH_DEVICE`.
#[inline]
pub fn name_begins_with_device_g<const DOS: bool>(n: &[u8]) -> bool {
    at(n, 0) != 0 && is_device_sep_g::<DOS>(at(n, 1))
}

/// `IS_UNC_NAME`.
#[inline]
pub fn is_unc_name_g<const DOS: bool>(n: &[u8]) -> bool {
    DOS && n.len() >= 3 && is_dir_sep_g::<DOS>(n[0]) && is_dir_sep_g::<DOS>(n[1]) && n[2].is_ascii_alphanumeric()
}

/// `FILECHARCASEEQ`.
#[inline]
pub fn file_char_eq(a: u8, b: u8) -> bool {
    if DOSISH { a.eq_ignore_ascii_case(&b) } else { a == b }
}

/// `FILESTRCASEEQ`.
#[inline]
pub fn file_str_eq(a: &[u8], b: &[u8]) -> bool {
    if DOSISH { a.eq_ignore_ascii_case(b) } else { a == b }
}

/// `TRANSFORM` over a whole key: what makes hash tables case-insensitive on Windows.
pub fn fold_key(k: &[u8]) -> Vec<u8> {
    if DOSISH { k.to_ascii_lowercase() } else { k.to_vec() }
}

/// `ISSPACE`.
#[inline]
pub fn is_space(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r')
}

/// `kpathsea_absolute_p`.
pub fn absolute_p_g<const DOS: bool>(f: &[u8], relative_ok: bool) -> bool {
    let (c0, c1, c2) = (at(f, 0), at(f, 1), at(f, 2));
    let absolute = is_dir_sep_g::<DOS>(c0)
        || (DOS && c0 != 0 && is_device_sep_g::<DOS>(c1))
        || (DOS && c0 == b'\\' && c1 == b'\\')
        || (DOS && c0 == b'/' && c1 == b'/');
    let explicit_relative = relative_ok && c0 == b'.' && (is_dir_sep_g::<DOS>(c1) || (c1 == b'.' && is_dir_sep_g::<DOS>(c2)));
    absolute || explicit_relative
}

/// `kpathsea_absolute_p` for the host.
#[inline]
pub fn absolute_p(f: &[u8], relative_ok: bool) -> bool {
    absolute_p_g::<DOSISH>(f, relative_ok)
}

/// `kpathsea_normalize_path`: slashifies and lowers the drive letter on Windows, and removes
/// multiple leading slashes, which would otherwise look like a request to expand the whole disk.
pub fn normalize_path_g<const DOS: bool>(elt: &mut Vec<u8>) -> usize {
    if DOS {
        for c in elt.iter_mut() {
            if *c == b'\\' {
                *c = b'/';
            }
        }
    }
    let ret = if name_begins_with_device_g::<DOS>(elt) {
        elt[0] = elt[0].to_ascii_lowercase();
        2
    } else if is_unc_name_g::<DOS>(elt) {
        let mut r = 2;
        while at(elt, r) != 0 && !is_dir_sep_ch(at(elt, r)) {
            r += 1;
        }
        r
    } else {
        0
    };
    let mut i = ret;
    while is_dir_sep_ch(at(elt, i)) {
        i += 1;
    }
    if i > ret + 1 {
        elt.drain(ret + 1..i);
    }
    ret
}

/// `kpathsea_normalize_path` for the host.
#[inline]
pub fn normalize_path(elt: &mut Vec<u8>) -> usize {
    normalize_path_g::<DOSISH>(elt)
}

/// `xbasename`.
pub fn xbasename_g<const DOS: bool>(name: &[u8]) -> &[u8] {
    let mut base = 0;
    if name_begins_with_device_g::<DOS>(name) {
        base = 2;
    } else if is_unc_name_g::<DOS>(name) {
        let mut limit = 2;
        while at(name, limit) != 0 && !is_dir_sep_g::<DOS>(at(name, limit)) {
            limit += 1;
        }
        let had = at(name, limit) != 0;
        limit += 1;
        if had && at(name, limit) != 0 && !is_dir_sep_g::<DOS>(at(name, limit)) {
            while at(name, limit) != 0 && !is_dir_sep_g::<DOS>(at(name, limit)) {
                limit += 1;
            }
        } else {
            limit = 0;
        }
        base = limit;
    }
    let mut p = base;
    while p < name.len() {
        if is_dir_sep_g::<DOS>(name[p]) {
            base = p + 1;
        }
        p += 1;
    }
    &name[base.min(name.len())..]
}

/// `xbasename` for the host.
#[inline]
pub fn xbasename(name: &[u8]) -> &[u8] {
    xbasename_g::<DOSISH>(name)
}

/// `xdirname`.
pub fn xdirname_g<const DOS: bool>(name: &[u8]) -> Vec<u8> {
    let mut limit = 0usize;
    if name_begins_with_device_g::<DOS>(name) {
        limit = 2;
    } else if is_unc_name_g::<DOS>(name) {
        limit = 2;
        while at(name, limit) != 0 && !is_dir_sep_g::<DOS>(at(name, limit)) {
            limit += 1;
        }
        let had = at(name, limit) != 0;
        limit += 1;
        if had && at(name, limit) != 0 && !is_dir_sep_g::<DOS>(at(name, limit)) {
            while at(name, limit) != 0 && !is_dir_sep_g::<DOS>(at(name, limit)) {
                limit += 1;
            }
            limit -= 1;
        } else {
            limit = 0;
        }
    }
    let mut loc = name.len();
    while loc > limit && !is_dir_sep_g::<DOS>(name[loc - 1]) {
        loc -= 1;
    }
    let mut ret = if loc == limit {
        if limit == 0 {
            b".".to_vec()
        } else if limit == 2 {
            vec![name[0], name[1], b'.']
        } else {
            name.to_vec()
        }
    } else {
        while loc > limit + 1 && is_dir_sep_g::<DOS>(name[loc - 1]) {
            loc -= 1;
        }
        name[..loc].to_vec()
    };
    if DOS {
        for c in ret.iter_mut() {
            if *c == b'\\' {
                *c = b'/';
            }
        }
    }
    ret
}

/// `xdirname` for the host.
#[inline]
pub fn xdirname(name: &[u8]) -> Vec<u8> {
    xdirname_g::<DOSISH>(name)
}

/// `find_suffix`: what follows the last dot, if no directory separator does.
pub fn find_suffix(name: &[u8]) -> Option<&[u8]> {
    let dot = name.iter().rposition(|&c| c == b'.')?;
    let rest = &name[dot + 1..];
    if rest.iter().any(|&c| is_dir_sep(c)) { None } else { Some(rest) }
}

/// `remove_suffix`.
pub fn remove_suffix(s: &[u8]) -> &[u8] {
    match find_suffix(s) {
        Some(suffix) => &s[..s.len() - suffix.len() - 1],
        None => s,
    }
}

/// `extend_filename`.
pub fn extend_filename(name: &[u8], default_suffix: &[u8]) -> Vec<u8> {
    let mut out = name.to_vec();
    if find_suffix(name).is_none() {
        out.push(b'.');
        out.extend_from_slice(default_suffix);
    }
    out
}

/// `kpathsea_expand_default`: a leading, trailing or doubled separator stands for `fallback`.
pub fn expand_default(path: Option<&[u8]>, fallback: &[u8]) -> Vec<u8> {
    let path = match path {
        Some(p) if !p.is_empty() => p,
        _ => return fallback.to_vec(),
    };
    if is_env_sep(path[0]) {
        if path.len() == 1 {
            return fallback.to_vec();
        }
        return [fallback, path].concat();
    }
    if path[path.len() - 1] == ENV_SEP {
        return [path, fallback].concat();
    }
    for loc in 0..path.len() - 1 {
        if is_env_sep(path[loc]) && is_env_sep(path[loc + 1]) {
            return [&path[..=loc], fallback, &path[loc + 1..]].concat();
        }
    }
    path.to_vec()
}

/// `kpathsea_path_element`, gathered: splits at a separator outside braces.  An empty path has one
/// empty element, and a trailing separator gives a last empty one.
pub fn path_elements(path: &[u8]) -> Vec<&[u8]> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut level = 0i32;
    let mut p = 0;
    while p < path.len() {
        let c = path[p];
        if level == 0 && is_kpse_sep(c) {
            out.push(&path[start..p]);
            start = p + 1;
        } else if c == b'{' {
            level += 1;
        } else if c == b'}' {
            level -= 1;
        }
        p += 1;
    }
    out.push(&path[start..]);
    out
}

/// A name as bytes, to the UTF-16 a wide Windows call takes: UTF-8, and a surrogate on its own as the three bytes that [`wide_to_bytes`] gives it, which makes WTF-8, so
/// whatever name the system hands out comes back as it was.  Any other byte is U+FFFD: `MultiByteToWideChar (CP_UTF8, 0, ...)` does the same for TeX Live under
/// its default `command_line_encoding = utf-8`, where `file_system_codepage` is UTF-8 too.  A plain function, so that it can be tested anywhere.
pub fn bytes_to_wide(b: &[u8]) -> Vec<u16> {
    let cont = |i: usize, lo: u8, hi: u8| b.get(i).copied().filter(|c| (lo..=hi).contains(c)).map(|c| (c & 0x3f) as u32);
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        let (cp, n) = match c {
            0..=0x7f => (Some(c as u32), 1),
            0xc2..=0xdf => (cont(i + 1, 0x80, 0xbf).map(|x| (c as u32 & 0x1f) << 6 | x), 2),
            0xe0..=0xef => {
                let lo = if c == 0xe0 { 0xa0 } else { 0x80 };
                (cont(i + 1, lo, 0xbf).zip(cont(i + 2, 0x80, 0xbf)).map(|(x, y)| (c as u32 & 0x0f) << 12 | x << 6 | y), 3)
            }
            0xf0..=0xf4 => {
                let (lo, hi) = (if c == 0xf0 { 0x90 } else { 0x80 }, if c == 0xf4 { 0x8f } else { 0xbf });
                let rest = cont(i + 2, 0x80, 0xbf).zip(cont(i + 3, 0x80, 0xbf));
                (cont(i + 1, lo, hi).zip(rest).map(|(x, (y, z))| (c as u32 & 0x07) << 18 | x << 12 | y << 6 | z), 4)
            }
            _ => (None, 1),
        };
        match cp {
            Some(cp) if cp >= 0x10000 => {
                out.push(0xd800 + ((cp - 0x10000) >> 10) as u16);
                out.push(0xdc00 + ((cp - 0x10000) & 0x3ff) as u16);
                i += n;
            }
            Some(cp) => {
                out.push(cp as u16);
                i += n;
            }
            None => {
                out.push(0xfffd);
                i += 1;
            }
        }
    }
    out
}

/// What a wide Windows call gave, as the bytes TeX sees: see [`bytes_to_wide`].
pub fn wide_to_bytes(w: &[u16]) -> Vec<u8> {
    let mut out = Vec::with_capacity(w.len());
    for r in char::decode_utf16(w.iter().copied()) {
        match r {
            Ok(c) => out.extend_from_slice(c.encode_utf8(&mut [0; 4]).as_bytes()),
            Err(e) => {
                let u = e.unpaired_surrogate();
                out.extend_from_slice(&[0xe0 | (u >> 12) as u8, 0x80 | ((u >> 6) & 0x3f) as u8, 0x80 | (u & 0x3f) as u8]);
            }
        }
    }
    out
}

/// What a console takes of `bytes`, which `std` refuses, whole, if they are not UTF-8: a character cut in two by the end of the buffer stays in `tail` for the next time, and
/// a byte that fits nothing is U+FFFD.  A TeX run prints whatever bytes a document gives it.  Only the Windows console needs this.
pub fn console_text(tail: &mut Vec<u8>, bytes: &[u8]) -> String {
    let data = [&tail[..], bytes].concat();
    tail.clear();
    let mut out = String::with_capacity(data.len());
    let mut rest = &data[..];
    loop {
        match std::str::from_utf8(rest) {
            Ok(s) => {
                out.push_str(s);
                return out;
            }
            Err(e) => {
                let (good, bad) = rest.split_at(e.valid_up_to());
                out.push_str(std::str::from_utf8(good).unwrap_or(""));
                let Some(n) = e.error_len() else {
                    tail.extend_from_slice(bad);
                    return out;
                };
                out.push('\u{fffd}');
                rest = &bad[n..];
            }
        }
    }
}

/// line.c's `read_line`, gathered: lines end in `\n`, `\r` or `\r\n`, and NUL bytes are dropped.
pub fn read_lines(buf: &[u8]) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    let mut line = Vec::new();
    let mut i = 0;
    let n = buf.len();
    while i < n {
        let c = buf[i];
        i += 1;
        match c {
            b'\n' => out.push(std::mem::take(&mut line)),
            b'\r' => {
                if i < n && buf[i] == b'\n' {
                    i += 1;
                }
                out.push(std::mem::take(&mut line));
            }
            0 => {}
            _ => line.push(c),
        }
    }
    if !line.is_empty() {
        out.push(line);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expand_default_as_kdefault_c() {
        let d = b"default";
        let sep = |s: &str| s.replace(':', std::str::from_utf8(&[ENV_SEP]).unwrap()).into_bytes();
        assert_eq!(expand_default(None, d), b"default");
        assert_eq!(expand_default(Some(b""), d), b"default");
        assert_eq!(expand_default(Some(b"none"), d), b"none");
        assert_eq!(expand_default(Some(&sep(":")), d), b"default");
        assert_eq!(expand_default(Some(&sep(":first")), d), sep("default:first"));
        assert_eq!(expand_default(Some(&sep("last:")), d), sep("last:default"));
        assert_eq!(expand_default(Some(&sep("middle::elddim")), d), sep("middle:default:elddim"));
        // Only the first doubled separator is replaced.
        assert_eq!(expand_default(Some(&sep("a::b::c")), d), sep("a:default:b::c"));
    }

    #[test]
    fn path_elements_as_path_elt_c() {
        let sep = std::str::from_utf8(&[ENV_SEP]).unwrap().to_string();
        let split = |s: String| path_elements(s.as_bytes()).iter().map(|e| e.to_vec()).collect::<Vec<_>>();
        assert_eq!(split(String::new()), vec![b"".to_vec()]);
        assert_eq!(split("a".into()), vec![b"a".to_vec()]);
        assert_eq!(split(sep.clone()), vec![b"".to_vec(), b"".to_vec()]);
        assert_eq!(split(format!("{sep}{sep}")), vec![b"".to_vec(), b"".to_vec(), b"".to_vec()]);
        assert_eq!(split(format!("a{sep}")), vec![b"a".to_vec(), b"".to_vec()]);
        assert_eq!(split(format!("{sep}b")), vec![b"".to_vec(), b"b".to_vec()]);
        assert_eq!(split(format!("a{sep}b")), vec![b"a".to_vec(), b"b".to_vec()]);
        // A separator in braces does not split.
        assert_eq!(split(format!("{{a{sep}b}}{sep}c")), vec![format!("{{a{sep}b}}").into_bytes(), b"c".to_vec()]);
    }

    #[test]
    fn absolute_as_absolute_c() {
        assert!(absolute_p_g::<false>(b"/a", false));
        assert!(!absolute_p_g::<false>(b"a/b", true));
        assert!(!absolute_p_g::<false>(b"./a", false));
        assert!(absolute_p_g::<false>(b"./a", true));
        assert!(absolute_p_g::<false>(b"../a", true));
        assert!(!absolute_p_g::<false>(b".a", true));
        assert!(!absolute_p_g::<false>(b"", true));
        assert!(!absolute_p_g::<false>(b"\\a", true));
        assert!(!absolute_p_g::<false>(b"c:/a", true));
        // Windows.
        assert!(absolute_p_g::<true>(b"c:/a", false));
        assert!(absolute_p_g::<true>(b"c:a", false));
        assert!(absolute_p_g::<true>(b"\\a", false));
        assert!(absolute_p_g::<true>(b"\\\\server\\share", false));
        assert!(absolute_p_g::<true>(b".\\a", true));
        assert!(absolute_p_g::<true>(b"..\\a", true));
        assert!(!absolute_p_g::<true>(b"a\\b", true));
    }

    #[test]
    fn normalize_as_elt_dirs_c() {
        let run = |s: &[u8]| {
            let mut v = s.to_vec();
            let r = normalize_path_g::<false>(&mut v);
            (v, r)
        };
        assert_eq!(run(b"//a/b"), (b"/a/b".to_vec(), 0));
        assert_eq!(run(b"////a//b//"), (b"/a//b//".to_vec(), 0));
        assert_eq!(run(b"/a"), (b"/a".to_vec(), 0));
        assert_eq!(run(b"a//b"), (b"a//b".to_vec(), 0));
        let win = |s: &[u8]| {
            let mut v = s.to_vec();
            let r = normalize_path_g::<true>(&mut v);
            (v, r)
        };
        assert_eq!(win(b"C:\\TeX\\\\x"), (b"c:/TeX//x".to_vec(), 2));
        assert_eq!(win(b"C://dir"), (b"c:/dir".to_vec(), 2));
        assert_eq!(win(b"\\\\server\\share//x"), (b"//server/share//x".to_vec(), 8));
        assert_eq!(win(b"////x"), (b"/x".to_vec(), 0));
    }

    #[test]
    fn dirname_and_basename_as_xdirtest_c() {
        for (input, dir, base) in [
            (&b"/w/kpathsea"[..], &b"/w"[..], &b"kpathsea"[..]),
            (b"/w//kpathsea", b"/w", b"kpathsea"),
            (b"///a", b"/", b"a"),
            (b"/a", b"/", b"a"),
            (b"a", b".", b"a"),
            (b"a/", b"a", b""),
            (b"", b".", b""),
            (b"./x", b".", b"x"),
        ] {
            assert_eq!(xdirname_g::<false>(input), dir, "{:?}", String::from_utf8_lossy(input));
            assert_eq!(xbasename_g::<false>(input), base, "{:?}", String::from_utf8_lossy(input));
        }
        for (input, dir, base) in [
            (&b"c:/w/x"[..], &b"c:/w"[..], &b"x"[..]),
            (b"c:x", b"c:.", b"x"),
            (b"c:\\w\\x", b"c:/w", b"x"),
            (b"//server/share/file", b"//server/share", b"file"),
            (b"\\\\server\\share\\", b"//server/share", b""),
            (b"//server/share", b"//server/share", b""),
        ] {
            assert_eq!(xdirname_g::<true>(input), dir, "{:?}", String::from_utf8_lossy(input));
            assert_eq!(xbasename_g::<true>(input), base, "{:?}", String::from_utf8_lossy(input));
        }
    }

    #[test]
    fn suffixes() {
        assert_eq!(find_suffix(b"foo"), None);
        assert_eq!(find_suffix(b"/foo.bar/baz"), None);
        assert_eq!(find_suffix(b"a.tex"), Some(&b"tex"[..]));
        assert_eq!(find_suffix(b"a."), Some(&b""[..]));
        assert_eq!(remove_suffix(b"a.b.c"), b"a.b");
        assert_eq!(remove_suffix(b"a"), b"a");
        assert_eq!(extend_filename(b"a", b"tfm"), b"a.tfm");
        assert_eq!(extend_filename(b"a.vf", b"tfm"), b"a.vf");
    }

    #[test]
    fn a_name_comes_back_from_utf16_as_it_was() {
        for s in ["", "plain.tex", "caf\u{e9}/\u{4e2d}\u{6587}.tex", "\u{1d11e}\u{10ffff}x", "C:/Users/A B/d.tex", "\u{7f}\u{80}\u{7ff}\u{800}\u{ffff}\u{10000}"] {
            let wide: Vec<u16> = s.encode_utf16().collect();
            assert_eq!(bytes_to_wide(s.as_bytes()), wide, "{s:?}");
            assert_eq!(wide_to_bytes(&wide), s.as_bytes(), "{s:?}");
        }
        // A name Windows allows and UTF-8 has no form for: an unpaired surrogate.
        for w in [&[0xd800u16][..], &[0x41, 0xdc00, 0x42], &[0xdc00, 0xd800], &[0xd83d]] {
            assert_eq!(bytes_to_wide(&wide_to_bytes(w)), w, "{w:x?}");
        }
    }

    #[test]
    fn bytes_that_are_no_utf8_become_replacement_characters() {
        let r = 0xfffd;
        assert_eq!(bytes_to_wide(b"a\xe9b"), [0x61, r, 0x62], "Latin-1");
        assert_eq!(bytes_to_wide(b"\xc0\x80"), [r, r], "too long a form");
        assert_eq!(bytes_to_wide(b"\xe0\x80\x80"), [r, r, r], "too long a form");
        assert_eq!(bytes_to_wide(b"\xf0\x80\x80\x80"), [r, r, r, r], "too long a form");
        assert_eq!(bytes_to_wide(b"\xf4\x90\x80\x80"), [r, r, r, r], "above U+10FFFF");
        assert_eq!(bytes_to_wide(b"\xe2\x82"), [r, r], "cut short");
        assert_eq!(bytes_to_wide(b"\xff"), [r]);
    }

    #[test]
    fn a_console_gets_valid_text_and_no_character_in_two_pieces() {
        let mut tail = Vec::new();
        assert_eq!(console_text(&mut tail, b"plain\n"), "plain\n");
        assert_eq!(console_text(&mut tail, "caf\u{e9}".as_bytes()), "caf\u{e9}");
        assert!(tail.is_empty());
        let e = "\u{20ac}".as_bytes();
        assert_eq!(console_text(&mut tail, &e[..1]), "");
        assert_eq!(console_text(&mut tail, &e[1..2]), "");
        assert_eq!(tail, &e[..2]);
        assert_eq!(console_text(&mut tail, &[&e[2..], b"x"].concat()), "\u{20ac}x");
        assert!(tail.is_empty());
        assert_eq!(console_text(&mut tail, b"a\xe9b\xffc"), "a\u{fffd}b\u{fffd}c");
        assert!(tail.is_empty());
    }

    #[test]
    fn lines_as_line_c() {
        let l = read_lines(b"a\nb\r\nc\rd\0e\n\nf");
        assert_eq!(l, vec![b"a".to_vec(), b"b".to_vec(), b"c".to_vec(), b"de".to_vec(), b"".to_vec(), b"f".to_vec()]);
        assert!(read_lines(b"").is_empty());
        assert_eq!(read_lines(b"\n"), vec![b"".to_vec()]);
    }
}
