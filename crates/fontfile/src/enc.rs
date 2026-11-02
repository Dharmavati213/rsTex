//! `load_enc_file` of writet1.c, and the `/Differences` arrays of writeenc.c.

use crate::cstr::*;
use crate::{CodeSet, FontError, GlyphNames};

type R<T> = Result<T, FontError>;

const ENC_BUF_SIZE: usize = 0x1000;
const WRITET1_C: &str = "../../../texk/web2c/pdftexdir/writet1.c";

/// `append_char_to_buf` into a fixed buffer of the C's size; `line` is the C source line of the use,
/// which the overflow message quotes.  Returns the new write position.
pub(crate) fn append_char_to_buf(
    c: &mut i32,
    p: usize,
    buf: &mut [u8],
    file: &str,
    line: u32,
) -> R<usize> {
    if *c == 9 {
        *c = 32;
    }
    if *c == 13 || *c == -1 {
        *c = 10;
    }
    if *c != 32 || (p > 0 && buf[p - 1] != 32) {
        if p + 1 > buf.len() {
            return Err(FontError::overflow(file, line));
        }
        buf[p] = *c as u8;
        return Ok(p + 1);
    }
    Ok(p)
}

/// `append_eol`.  Returns the new write position.
pub(crate) fn append_eol(mut p: usize, buf: &mut [u8], file: &str, line: u32) -> R<usize> {
    if p + 2 > buf.len() {
        return Err(FontError::overflow(file, line));
    }
    if p > 1 && buf[p - 1] != 10 {
        buf[p] = 10;
        p += 1;
    }
    if p > 2 && buf[p - 2] == 32 {
        buf[p - 2] = 10;
        p -= 1;
    }
    buf[p] = 0;
    Ok(p)
}

struct Enc<'a> {
    data: &'a [u8],
    pos: usize,
    eof: bool,
    line: Vec<u8>,
}

impl Enc<'_> {
    #[inline]
    fn getchar(&mut self) -> i32 {
        match self.data.get(self.pos) {
            Some(&c) => {
                self.pos += 1;
                c as i32
            }
            None => {
                self.eof = true;
                -1
            }
        }
    }

    fn getline(&mut self) -> R<()> {
        loop {
            if self.eof {
                return Err(FontError::new(b"unexpected end of file"));
            }
            let mut p = 0;
            loop {
                let mut c = self.getchar();
                p = append_char_to_buf(&mut c, p, &mut self.line, WRITET1_C, 223)?;
                if c == 10 {
                    break;
                }
            }
            p = append_eol(p, &mut self.line, WRITET1_C, 225)?;
            if p < 2 || self.line[0] == b'%' {
                continue;
            }
            return Ok(());
        }
    }

    /// `remove_eol(r, enc_line)`, and the line.
    fn quoted(&self) -> &[u8] {
        let l = cstr(&self.line, 0);
        if l.last() == Some(&10) { &l[..l.len() - 1] } else { l }
    }
}

/// `load_enc_file`: the contents of a `.enc` file to its glyph names.
///
/// The C prints `{name` and `}` around the reading and fails if the file cannot be opened; those stay
/// with the caller.
pub fn load_enc_file(data: &[u8]) -> R<GlyphNames> {
    let mut e = Enc { data, pos: 0, eof: false, line: vec![0; ENC_BUF_SIZE] };
    let mut names = GlyphNames::new();
    e.getline()?;
    let mut r = match if e.line[0] == b'/' { strchr(&e.line, 0, b'[') } else { None } {
        Some(r) => r,
        None => {
            let mut m = b"invalid encoding vector (a name or `[' missing): `".to_vec();
            m.extend_from_slice(e.quoted());
            m.push(b'\'');
            return Err(FontError::new(&m));
        }
    };
    let mut names_count = 0;
    r += 1; // skip '['
    if at(&e.line, r) == b' ' {
        r += 1;
    }
    loop {
        while at(&e.line, r) == b'/' {
            r += 1;
            let start = r;
            loop {
                let c = at(&e.line, r);
                if c == b' ' || c == 10 || c == b']' || c == b'/' || r >= e.line.len() {
                    break;
                }
                r += 1;
            }
            // xstrdup of a C string: an embedded NUL ends the name.
            let name = cstr(&e.line[..r], start);
            if at(&e.line, r) == b' ' {
                r += 1;
            }
            if names_count > 255 {
                return Err(FontError::new(b"encoding vector contains more than 256 names"));
            }
            if name != b".notdef" {
                names.set(names_count, name);
            }
            names_count += 1;
        }
        let c = at(&e.line, r);
        if c != 10 && c != b'%' {
            if str_prefix(&e.line, r, b"] def") {
                return Ok(names);
            }
            let mut m = b"invalid encoding vector: a name or `] def' expected: `".to_vec();
            m.extend_from_slice(e.quoted());
            m.push(b'\'');
            return Err(FontError::new(&m));
        }
        e.getline()?;
        r = 0;
    }
}

/// The body of `/Differences [...]` from `write_enc`, for the codes used by TeX.
pub fn write_enc_differences(names: &GlyphNames, tx_codes: &CodeSet, out: &mut Vec<u8>) {
    let mut i_old: i32 = -2;
    for i in 0..256 {
        if !tx_codes.contains(i) {
            continue;
        }
        if i as i32 == i_old + 1 {
            // no gap
            out.push(b'/');
            out.extend_from_slice(names.name(i));
        } else {
            if i_old != -2 {
                out.push(b' ');
            }
            push_i64(out, i as i64);
            out.push(b'/');
            out.extend_from_slice(names.name(i));
        }
        i_old = i as i32;
    }
}

/// The body of `/Differences [...]` from `epdf_write_enc`: every code with a name.
pub fn epdf_write_enc_differences(names: &GlyphNames, out: &mut Vec<u8>) {
    let mut i_old: i32 = -2;
    for i in 0..256 {
        if names.is_notdef(i) {
            continue;
        }
        if i as i32 == i_old + 1 {
            out.push(b'/');
            out.extend_from_slice(names.name(i));
        } else {
            if i_old != -2 {
                out.push(b' ');
            }
            push_i64(out, i as i64);
            out.push(b'/');
            out.extend_from_slice(names.name(i));
        }
        i_old = i as i32;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_comments_blanks_and_notdef() {
        let src = b"% header\n/MyEnc [ /a /.notdef\n% more\n  /b/c  % trailing\n] def\n";
        let n = load_enc_file(src).unwrap();
        assert_eq!(n.name(0), b"a");
        assert!(n.is_notdef(1));
        assert_eq!(n.name(1), b".notdef");
        assert_eq!(n.name(2), b"b");
        assert_eq!(n.name(3), b"c");
        assert!(n.is_notdef(4));
    }

    #[test]
    fn errors_quote_the_line() {
        let e = load_enc_file(b"junk\n").err().unwrap();
        assert_eq!(e.message, b"invalid encoding vector (a name or `[' missing): `junk'".to_vec());
        let e = load_enc_file(b"/E [ /a\nfoo\n").err().unwrap();
        assert_eq!(e.message, b"invalid encoding vector: a name or `] def' expected: `foo'".to_vec());
        let e = load_enc_file(b"").err();
        // An empty file: feof is not set until the first read, which then makes an empty line.
        assert_eq!(e.unwrap().message, b"unexpected end of file".to_vec());
    }

    #[test]
    fn differences_like_write_enc() {
        let mut n = GlyphNames::new();
        n.set(65, b"A");
        n.set(66, b"B");
        n.set(70, b"F");
        let mut tx = CodeSet::new();
        tx.insert(65);
        tx.insert(66);
        tx.insert(70);
        let mut out = Vec::new();
        write_enc_differences(&n, &tx, &mut out);
        assert_eq!(out, b"65/A/B 70/F".to_vec());
        out.clear();
        epdf_write_enc_differences(&n, &mut out);
        assert_eq!(out, b"65/A/B 70/F".to_vec());
    }
}
