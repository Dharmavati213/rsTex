//! `subfont.c`: `.sfd` files and the `tfm@sfd@` map-line form.
//!
//! The C keeps a tree of the files read; the caller holds the `Sfd` values here.

use crate::cstr::*;
use crate::enc::{append_char_to_buf, append_eol};
use crate::FontError;

type R<T> = Result<T, FontError>;

const SFD_BUF_SIZE: usize = 256;
const SUBFONT_C: &str = "../../../texk/web2c/pdftexdir/subfont.c";

/// `subfont_entry`.
#[derive(Clone)]
pub struct Subfont {
    pub infix: Vec<u8>,
    /// The mapping for this subfont; -1 is unassigned.
    pub charcodes: Box<[i64; 256]>,
}

/// `sfd_entry`, with `subfont` in the C's list order: the last one in the file comes first.
#[derive(Clone, Default)]
pub struct Sfd {
    pub subfonts: Vec<Subfont>,
}

struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
    eof: bool,
    line: [u8; SFD_BUF_SIZE],
}

impl Reader<'_> {
    fn getline(&mut self, expect_eof: bool) -> R<()> {
        loop {
            if self.eof {
                if expect_eof {
                    return Ok(());
                }
                return Err(FontError::new(b"unexpected end of file"));
            }
            let mut p = 0;
            loop {
                let mut c = match self.data.get(self.pos) {
                    Some(&b) => {
                        self.pos += 1;
                        b as i32
                    }
                    None => {
                        self.eof = true;
                        -1
                    }
                };
                p = append_char_to_buf(&mut c, p, &mut self.line, SUBFONT_C, 95)?;
                if c == 10 {
                    break;
                }
            }
            p = append_eol(p, &mut self.line, SUBFONT_C, 97)?;
            if p < 2 || self.line[0] == b'#' {
                continue;
            }
            return Ok(());
        }
    }
}

fn invalid(what: &[u8], rest: &[u8]) -> FontError {
    let mut m = what.to_vec();
    m.extend_from_slice(rest);
    FontError::new(&m)
}

/// `read_sfd`: the contents of a `.sfd` file.  The C's "cannot open SFD file for reading" warning,
/// after which `handle_subfont_fm` says no, stays with the caller.
pub fn read_sfd(data: &[u8]) -> R<Sfd> {
    let mut rd = Reader { data, pos: 0, eof: false, line: [0; SFD_BUF_SIZE] };
    let mut sfd = Sfd::default();
    let mut list: Vec<Subfont> = Vec::new();
    while !rd.eof {
        rd.getline(true)?;
        if rd.line[0] == 10 {
            break; // empty line indicating eof
        }
        let mut codes = Box::new([-1i64; 256]);
        let mut sc = Scan::new(&rd.line, 0);
        let word = sc.word(usize::MAX);
        sc.skip_ws();
        let infix = match word {
            Some((a, b)) => rd.line[a..b].to_vec(),
            None => Vec::new(),
        };
        let mut p = sc.i; // skip to the next word
        let mut k: i64 = 0;
        loop {
            let c = at(&rd.line, p);
            if c == b'\\' {
                // continue on next line
                rd.getline(false)?;
                p = 0;
                continue;
            } else if c == 0 {
                break; // end of subfont
            }
            let mut sc = Scan::new(&rd.line, p);
            let mut i = match sc.int(0, usize::MAX) {
                Some(i) => i,
                None => return Err(invalid(b"invalid token:\n", cstr(&rd.line, p))),
            };
            sc.skip_ws();
            p = sc.i;
            let c = at(&rd.line, p);
            if c == b':' {
                // offset
                k = i;
                p += 1;
            } else if c == b'_' {
                // range
                let mut sc = Scan::new(&rd.line, p + 1);
                let j = match sc.int(0, usize::MAX) {
                    Some(j) => j,
                    None => return Err(invalid(b"invalid token:\n", cstr(&rd.line, p))),
                };
                sc.skip_ws();
                let after = sc.i;
                if i > j || k + (j - i) > 255 || k < 0 {
                    return Err(invalid(b"invalid range:\n", cstr(&rd.line, p)));
                }
                while i <= j {
                    codes[k as usize] = i;
                    k += 1;
                    i += 1;
                }
                p = after;
            } else {
                // codepoint.  The C has no check here and writes past charcodes[255].
                if !(0..256).contains(&k) {
                    return Err(invalid(b"invalid range:\n", cstr(&rd.line, p)));
                }
                codes[k as usize] = i;
                k += 1;
            }
        }
        list.push(Subfont { infix, charcodes: codes });
    }
    list.reverse();
    sfd.subfonts = list;
    Ok(sfd)
}

/// The test at the top of `handle_subfont_fm`: for `tfm_name` of the form `prefix@sfd@`, the prefix and
/// the sfd name.  The caller reads `<sfd>.sfd`, warns if it cannot, and otherwise makes one map entry
/// per subfont: name `prefix` + `infix`, the same font file and type with `F_SUBFONT`, no `ps_name`,
/// and PidEid defaulting to 3,1.
pub fn subfont_parts(tfm_name: &[u8]) -> Option<(&[u8], &[u8])> {
    let q = tfm_name.iter().position(|&c| c == b'@')?;
    let r = q + 1 + tfm_name[q + 1..].iter().position(|&c| c == b'@')?;
    // prefix or sfd name is empty, or the second '@' is not the last char yet
    if q == 0 || r <= q + 1 || r != tfm_name.len() - 1 {
        return None;
    }
    Some((&tfm_name[..q], &tfm_name[q + 1..r]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parts() {
        assert_eq!(subfont_parts(b"foo@Unicode@"), Some((&b"foo"[..], &b"Unicode"[..])));
        assert_eq!(subfont_parts(b"foo"), None);
        assert_eq!(subfont_parts(b"foo@x"), None);
        assert_eq!(subfont_parts(b"@x@"), None);
        assert_eq!(subfont_parts(b"foo@@"), None);
        assert_eq!(subfont_parts(b"foo@x@y"), None);
    }

    #[test]
    fn sfd_forms() {
        let src = b"# comment\n01 0x41_0x43 0x50\n02 10: 0x61 \\\n 0x62_0x63 \n";
        let s = read_sfd(src).unwrap();
        assert_eq!(s.subfonts.len(), 2);
        assert_eq!(s.subfonts[0].infix, b"02", "the C prepends");
        assert_eq!(s.subfonts[1].infix, b"01");
        let a = &s.subfonts[1].charcodes;
        assert_eq!((a[0], a[1], a[2], a[3], a[4]), (0x41, 0x42, 0x43, 0x50, -1));
        let b = &s.subfonts[0].charcodes;
        assert_eq!((b[9], b[10], b[11], b[12], b[13]), (-1, 0x61, 0x62, 0x63, -1));
    }

    #[test]
    fn sfd_ends() {
        assert_eq!(read_sfd(b"").unwrap().subfonts.len(), 0);
        assert_eq!(read_sfd(b"# only a comment\n").unwrap().subfonts.len(), 0);
        // A comment that is the last line, with no newline, is taken for a subfont: the C's quirk.
        let s = read_sfd(b"01 0x41\n#end").unwrap();
        assert_eq!(s.subfonts.len(), 2);
        assert_eq!(s.subfonts[0].infix, b"#end");
        // With more words, the second is a bad token, and the whole file fails (the oracle says so too).
        assert!(read_sfd(b"01 0x41\n# the end").is_err());
    }

    #[test]
    fn sfd_errors() {
        assert!(read_sfd(b"01 x\n").is_err());
        assert!(read_sfd(b"01 5_3\n").is_err());
        assert!(read_sfd(b"01 0_300\n").is_err());
        assert!(read_sfd(b"01 300: 1\n").is_err());
    }
}
