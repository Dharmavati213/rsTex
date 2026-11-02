//! `mapfile.c`: the map-line parser, its checks and its `float` arithmetic.
//!
//! The trees (`tfm_tree`, `ps_tree`, `ff_tree`) and `avl_do_entry` are state, and stay with the
//! engine.  An `FmEntry` borrows its strings from the line, so scanning a line allocates nothing
//! unless it warns.

use crate::cstr::*;
use crate::enc::append_char_to_buf;
use crate::{FD_FLAGS_NOT_SET_IN_MAPLINE, FontError};

type R<T> = Result<T, FontError>;

const FM_BUF_SIZE: usize = 1024;
const MAPFILE_C: &str = "../../../texk/web2c/pdftexdir/mapfile.c";

pub const F_INCLUDED: u16 = 0x01;
pub const F_SUBSETTED: u16 = 0x02;
pub const F_STDT1FONT: u16 = 0x04;
pub const F_SUBFONT: u16 = 0x08;
pub const F_TYPE1: u16 = 0x10;
pub const F_TRUETYPE: u16 = 0x20;
pub const F_OTF: u16 = 0x40;
pub const F_PK: u16 = 0x80;

/// `fm_entry`, the fields scanned from the map line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FmEntry<'a> {
    pub tfm_name: &'a [u8],
    pub ps_name: Option<&'a [u8]>,
    pub fd_flags: i32,
    /// The C's `integer`, which is 64 bits.
    pub slant: i64,
    pub extend: i64,
    pub encname: Option<&'a [u8]>,
    pub ff_name: Option<&'a [u8]>,
    pub ty: u16,
    pub pid: i16,
    pub eid: i16,
}

impl FmEntry<'_> {
    pub fn is_included(&self) -> bool {
        self.ty & F_INCLUDED != 0
    }
    pub fn is_subsetted(&self) -> bool {
        self.ty & F_SUBSETTED != 0
    }
    pub fn is_std_t1font(&self) -> bool {
        self.ty & F_STDT1FONT != 0
    }
    pub fn is_type1(&self) -> bool {
        self.ty & F_TYPE1 != 0
    }
    pub fn is_truetype(&self) -> bool {
        self.ty & F_TRUETYPE != 0
    }
    pub fn is_opentype(&self) -> bool {
        self.ty & F_OTF != 0
    }
    pub fn is_pk(&self) -> bool {
        self.ty & F_PK != 0
    }
    pub fn is_reencoded(&self) -> bool {
        self.encname.is_some()
    }
    pub fn is_fontfile(&self) -> bool {
        self.ff_name.is_some()
    }
    pub fn is_t1fontfile(&self) -> bool {
        self.is_fontfile() && self.is_type1()
    }
    pub fn is_builtin(&self) -> bool {
        !self.is_fontfile()
    }
}

/// What `fm_scan_line` did with a line.
#[derive(Debug)]
pub struct FmScan<'a> {
    /// None for an empty line, a comment or a bad line (`goto bad_line`, then `delete_fm_entry`).
    pub entry: Option<FmEntry<'a>>,
    /// `pdftex_warn` messages, in order.
    pub warnings: Vec<Vec<u8>>,
}

/// `check_std_t1font`: the font number if `s` is one of the 14 standard font names, otherwise -1.
pub fn check_std_t1font(s: &[u8]) -> i32 {
    const NAMES: [&[u8]; 14] = [
        b"Courier",
        b"Courier-Bold",
        b"Courier-Oblique",
        b"Courier-BoldOblique",
        b"Helvetica",
        b"Helvetica-Bold",
        b"Helvetica-Oblique",
        b"Helvetica-BoldOblique",
        b"Symbol",
        b"Times-Roman",
        b"Times-Bold",
        b"Times-Italic",
        b"Times-BoldItalic",
        b"ZapfDingbats",
    ];
    const INDEX: [i32; 22] =
        [-1, -1, -1, -1, -1, -1, 8, 0, -1, 4, 10, 9, -1, -1, 5, 2, 12, 6, -1, 3, -1, 7];
    let n = s.len();
    if n > 21 {
        return -1;
    }
    let k = if n == 12 {
        match s[0] {
            b'C' => 1,
            b'T' => 11,
            b'Z' => 13,
            _ => return -1,
        }
    } else {
        INDEX[n]
    };
    if k > -1 && NAMES[k as usize] == s {
        return k;
    }
    -1
}

/// The first half of `fm_scan_line` for a map file: `fm_getchar` to `fm_line`.  `pos` is the file
/// position, `buf` holds at least 1024 bytes; the result is the line, with its tabs and runs of
/// blanks already squeezed, which a `\pdfmapline` is not.  None at end of file: the C would then
/// call `fm_scan_line` once more on nothing, which does nothing.
pub fn fm_read_line<'b>(data: &[u8], pos: &mut usize, buf: &'b mut [u8]) -> R<Option<&'b [u8]>> {
    if *pos >= data.len() {
        return Ok(None);
    }
    let mut p = 0;
    let lim = buf.len().min(FM_BUF_SIZE);
    loop {
        let mut c = match data.get(*pos) {
            Some(&b) => {
                *pos += 1;
                b as i32
            }
            None => -1,
        };
        p = append_char_to_buf(&mut c, p, &mut buf[..lim], MAPFILE_C, 440)?;
        if c == 10 {
            break;
        }
    }
    p -= 1; // *(--p) = '\0'
    Ok(Some(&buf[..p]))
}

/// `read_field`: returns the span and leaves `r` after one blank.
fn read_field(line: &[u8], r: &mut usize, src_line: u32) -> R<(usize, usize)> {
    let start = *r;
    loop {
        let c = at(line, *r);
        if c == b' ' || c == b'<' || c == b'"' || c == 0 {
            break;
        }
        if *r - start + 1 > FM_BUF_SIZE {
            return Err(FontError::overflow(MAPFILE_C, src_line));
        }
        *r += 1;
    }
    let end = *r;
    if at(line, *r) == b' ' {
        *r += 1;
    }
    Ok((start, end))
}

/// `d *= 1000.0; (integer)(d > 0 ? d + 0.5 : d - 0.5)`, with `d` a `float`.
fn fm_float_param(d: f32) -> i64 {
    let d = (d as f64 * 1000.0) as f32;
    dtol(if d > 0.0 { d as f64 + 0.5 } else { d as f64 - 0.5 })
}

/// `fm_scan_line` up to its end: scans `line` and runs `check_fm_entry(fm, true)`.
///
/// `line` is a whole `\pdfmapline` string, or the result of `fm_read_line`.  The subfont test and
/// `avl_do_entry` come next and are the caller's: `crate::subfont_parts` says whether this is a subfont.
pub fn fm_scan_line(line: &[u8]) -> R<FmScan<'_>> {
    let mut warnings: Vec<Vec<u8>> = Vec::new();
    let mut r = 0;
    let c0 = at(line, 0);
    if c0 == 0 || c0 == 10 || c0 == b'*' || c0 == b'#' || c0 == b';' || c0 == b'%' {
        return Ok(FmScan { entry: None, warnings });
    }
    let mut fm = FmEntry {
        tfm_name: &[],
        ps_name: None,
        fd_flags: FD_FLAGS_NOT_SET_IN_MAPLINE,
        slant: 0,
        extend: 0,
        encname: None,
        ff_name: None,
        ty: 0,
        pid: -1,
        eid: -1,
    };
    let mut tfm_null = true;
    let mut u = 0u8;
    let mut v = 0u8;
    'done: {
        let (a, b) = read_field(line, &mut r, 455)?;
        if b > a {
            fm.tfm_name = &line[a..b];
            tfm_null = false;
        }
        if at(line, r) == 0 {
            break 'done;
        }
        tfm_null = false; // used in messages, so don't let be null
        if !at(line, r).is_ascii_digit() {
            // 2nd field ps_name may not start with a digit
            let (a, b) = read_field(line, &mut r, 460)?;
            if b > a {
                fm.ps_name = Some(&line[a..b]);
            }
            if at(line, r) == 0 {
                break 'done;
            }
        }
        if at(line, r).is_ascii_digit() {
            // font descriptor /Flags given?
            let mut s = r;
            while at(line, s).is_ascii_digit() {
                s += 1;
            }
            let c = at(line, s);
            if c == b' ' || c == b'"' || c == b'<' || c == 0 {
                // not e. g. 8r.enc
                let mut sc = Scan::new(line, r);
                fm.fd_flags = sc.int(10, usize::MAX).unwrap_or(0) as i32;
                while at(line, r).is_ascii_digit() {
                    r += 1;
                }
            }
        }
        loop {
            // loop through "specials", encoding, font file
            if at(line, r) == b' ' {
                r += 1;
            }
            match at(line, r) {
                0 => break 'done,
                b'"' => {
                    // opening quote
                    r += 1;
                    u = 0;
                    v = 0;
                    loop {
                        if at(line, r) == b' ' {
                            r += 1;
                        }
                        let mut sc = Scan::new(line, r);
                        if let Some(d) = sc.float() {
                            sc.skip_ws();
                            let mut s = sc.i; // jump behind number, eat also blanks, if any
                            if s > 0 && (at(line, s - 1) == b'E' || at(line, s - 1) == b'e') {
                                // e. g. 0.5ExtendFont: %f = 0.5E, says the C.  This glibc fails that
                                // conversion instead, so only something like `infE` gets here.
                                s -= 1;
                            }
                            if str_prefix(line, s, b"SlantFont") {
                                fm.slant = fm_float_param(d);
                                r = s + 9;
                            } else if str_prefix(line, s, b"ExtendFont") {
                                fm.extend = fm_float_param(d);
                                if fm.extend == 1000 {
                                    fm.extend = 0;
                                }
                                r = s + 10;
                            } else {
                                // unknown name
                                r = s;
                                loop {
                                    let c = at(line, r);
                                    if c == b' ' || c == b'"' || c == 0 {
                                        break;
                                    }
                                    r += 1;
                                }
                                let mut m = b"invalid entry for `".to_vec();
                                m.extend_from_slice(fm.tfm_name);
                                m.extend_from_slice(b"': unknown name `");
                                m.extend_from_slice(&line[s.min(line.len())..r.min(line.len())]);
                                m.extend_from_slice(b"' ignored");
                                warnings.push(crate::truncate_message(m));
                            }
                        } else {
                            loop {
                                let c = at(line, r);
                                if c == b' ' || c == b'"' || c == 0 {
                                    break;
                                }
                                r += 1;
                            }
                        }
                        if at(line, r) != b' ' {
                            break;
                        }
                    }
                    if at(line, r) == b'"' {
                        r += 1; // closing quote
                    } else {
                        let mut m = b"invalid entry for `".to_vec();
                        m.extend_from_slice(fm.tfm_name);
                        m.extend_from_slice(b"': closing quote missing");
                        warnings.push(crate::truncate_message(m));
                        return Ok(FmScan { entry: None, warnings });
                    }
                    continue;
                }
                b'P' => {
                    // handle cases for subfonts like 'PidEid=3,1'
                    let mut sc = Scan::new(line, r);
                    if sc.lit(b"PidEid=") {
                        if let Some(a) = sc.int(0, usize::MAX) {
                            if sc.lit(b", ") {
                                if let Some(b) = sc.int(0, usize::MAX) {
                                    sc.skip_ws();
                                    fm.pid = a as i32 as i16;
                                    fm.eid = b as i32 as i16;
                                    r = sc.i;
                                    continue;
                                }
                            }
                        }
                    }
                }
                _ => {}
            }
            // encoding or font file specification
            let mut a = 0u8;
            let mut b = 0u8;
            if at(line, r) == b'<' {
                a = b'<';
                r += 1;
                if at(line, r) == b'<' || at(line, r) == b'[' {
                    b = at(line, r);
                    r += 1;
                }
            }
            let (fa, fb) = read_field(line, &mut r, 531)?;
            let field = &line[fa..fb];
            // encoding, formats: '8r.enc' or '<8r.enc' or '<[8r.enc'
            if field.len() > 4 && ascii_ieq(&field[field.len() - 4..], b".enc") {
                fm.encname = Some(field);
                u = 0; // u, v used if intervening blank: "<< foo"
                v = 0;
            } else if !field.is_empty() {
                // font file, formats:
                //  subsetting:    '<cmr10.pfa'
                //  no subsetting: '<<cmr10.pfa'
                //  no embedding:  'cmr10.pfa'
                if a == b'<' || u == b'<' {
                    fm.ty |= F_INCLUDED;
                    if (a == b'<' && b == 0) || (a == 0 && v == 0) {
                        fm.ty |= F_SUBSETTED;
                    }
                    // otherwise b == '<' (or '[') => no subsetting
                }
                fm.ff_name = Some(field);
                if at(line, r) == 0 {
                    break 'done;
                }
                u = 0;
                v = 0;
            } else {
                u = a;
                v = b;
            }
        }
    }
    // done:
    if let Some(ps) = fm.ps_name {
        if check_std_t1font(ps) >= 0 {
            fm.ty |= F_STDT1FONT;
        }
    }
    match fm.ff_name {
        Some(ff) if ff.len() > 3 => {
            let ext = &ff[ff.len() - 4..];
            if ascii_ieq(ext, b".ttf") || ascii_ieq(ext, b".ttc") {
                fm.ty |= F_TRUETYPE;
            } else if ascii_ieq(ext, b".otf") {
                fm.ty |= F_OTF;
            } else {
                fm.ty |= F_TYPE1;
            }
        }
        _ => {
            if fm.ps_name.is_none() {
                fm.ty |= F_PK; // without ps_name and without fontfile it can only be a bitmap PK font
            } else {
                fm.ty |= F_TYPE1; // assume a builtin font is Type1
            }
        }
    }
    if tfm_null {
        return Err(FontError::assertion(MAPFILE_C, 286, "check_fm_entry", "fm->tfm_name"));
    }
    if check_fm_entry(&mut fm, &mut warnings) != 0 {
        return Ok(FmScan { entry: None, warnings });
    }
    Ok(FmScan { entry: Some(fm), warnings })
}

fn warn_entry(warnings: &mut Vec<Vec<u8>>, parts: &[&[u8]]) {
    let mut m = Vec::new();
    for p in parts {
        m.extend_from_slice(p);
    }
    warnings.push(crate::truncate_message(m));
}

/// `%g` of `v / 1000.0`, for the two "too big" warnings.
fn g_thousandths(v: i64) -> Vec<u8> {
    let mut b = Vec::new();
    push_g(&mut b, v as f64 / 1000.0);
    b
}

/// `check_fm_entry` with its warn flag on: zero if ok, otherwise bits of what is wrong.
pub fn check_fm_entry(fm: &mut FmEntry<'_>, w: &mut Vec<Vec<u8>>) -> i32 {
    let mut a = 0;
    if fm.is_fontfile() && !fm.is_included() {
        warn_entry(w, &[
            b"ambiguous entry for `",
            fm.tfm_name,
            b"': font file present but not included, will be treated as font file not present",
        ]);
        fm.ff_name = None;
        // do not set variable a as this entry will still be accepted
    }
    // if no tfm name, nothing to do here; a bare tfm is ok
    if fm.tfm_name.is_empty() {
        warn_entry(w, &[b"invalid map entry: tfm missing"]);
        a += 1;
    }
    // TrueType fonts cannot be reencoded without subsetting
    if fm.is_truetype() && fm.is_reencoded() && !fm.is_subsetted() {
        warn_entry(w, &[
            b"invalid entry for `",
            fm.tfm_name,
            b"': only subsetted TrueType fonts can be reencoded",
        ]);
        a += 2;
    }
    // SlantFont and ExtendFont can be used only with Type1 fonts
    if (fm.slant != 0 || fm.extend != 0)
        && (fm.tfm_name.is_empty() || !(fm.is_t1fontfile() && fm.is_included()))
    {
        warn_entry(w, &[
            b"invalid entry for `",
            fm.tfm_name,
            b"': SlantFont/ExtendFont can be used only with embedded Type1 fonts",
        ]);
        a += 4;
    }
    // the value of SlantFont and ExtendFont must be reasonable.  abs takes an int.
    if (fm.slant as i32).wrapping_abs() > 1000 {
        warn_entry(w, &[
            b"invalid entry for `",
            fm.tfm_name,
            b"': SlantFont value too big: ",
            &g_thousandths(fm.slant),
        ]);
        a += 8;
    }
    if (fm.extend as i32).wrapping_abs() > 2000 {
        warn_entry(w, &[
            b"invalid entry for `",
            fm.tfm_name,
            b"': ExtendFont value too big: ",
            &g_thousandths(fm.extend),
        ]);
        a += 16;
    }
    // subfonts must be used with subsetted non-reencoded TrueType fonts
    if fm.pid != -1 && !(fm.is_truetype() && fm.is_subsetted() && !fm.is_reencoded()) {
        warn_entry(w, &[
            b"invalid entry for `",
            fm.tfm_name,
            b"': PidEid can be used only with subsetted non-reencoded TrueType fonts",
        ]);
        a += 32;
    }
    // font file for bitmap PK fonts is determinated by kpse and depends on current font size;
    // writet3.c ignores font file for bitmap PK fonts
    if let (Some(ff), true) = (fm.ff_name, fm.is_pk()) {
        warn_entry(w, &[
            b"invalid entry for `",
            fm.tfm_name,
            b"': FontFile cannot be specified for bitmap PK font: ",
            ff,
        ]);
        a += 64;
    }
    // ps name cannot be stored into PDF file for PDF Type3 fonts
    if let (Some(ps), true) = (fm.ps_name, fm.is_pk()) {
        warn_entry(w, &[
            b"invalid entry for `",
            fm.tfm_name,
            b"': PsName cannot be specified for bitmap PK font: ",
            ps,
        ]);
        a += 128;
    }
    a
}

/// `updatemode`.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum FmMode {
    DupIgnore,
    Replace,
    Delete,
}

/// `maptype`.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum MapType {
    MapFile,
    MapLine,
}

/// What `process_map_item` works out from its string.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct MapItem<'a> {
    pub mode: FmMode,
    /// The default map file must be forgotten, not read first (`mitem->line = NULL`).
    pub flush_default: bool,
    /// The file name or the line; empty if there is nothing to process.
    pub item: &'a [u8],
}

/// The parsing part of `process_map_item`.  The caller then reads the default map file if it is still
/// pending and not flushed, and processes `item` with `mode` unless it is empty.
pub fn process_map_item(s: &[u8], ty: MapType) -> MapItem<'_> {
    let mut i = 0;
    if at(s, i) == b' ' {
        i += 1; // ignore leading blank
    }
    let mut flush_default = false;
    let mode = match at(s, i) {
        b'+' => {
            i += 1;
            FmMode::DupIgnore // insert entry, if it is not duplicate
        }
        b'=' => {
            i += 1;
            FmMode::Replace // try to replace earlier entry
        }
        b'-' => {
            i += 1;
            FmMode::Delete // try to delete entry
        }
        _ => {
            flush_default = true; // like +, but also flush default map file name
            FmMode::DupIgnore
        }
    };
    if at(s, i) == b' ' {
        i += 1; // ignore blank after [+-=]
    }
    let start = i.min(s.len());
    let mut p = start;
    match ty {
        MapType::MapFile => {
            // remove blank at end
            while at(s, p) != 0 && at(s, p) != b' ' {
                p += 1;
            }
        }
        MapType::MapLine => {
            // blank at end allowed
            while at(s, p) != 0 {
                p += 1;
            }
        }
    }
    MapItem { mode, flush_default, item: &s[start..p] }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scan(s: &[u8]) -> FmEntry<'_> {
        fm_scan_line(s).unwrap().entry.expect("a good line")
    }

    #[test]
    fn typical_lines() {
        let f = scan(b"cmr10 CMR10 <cmr10.pfb");
        assert_eq!(f.tfm_name, b"cmr10");
        assert_eq!(f.ps_name, Some(&b"CMR10"[..]));
        assert_eq!(f.ff_name, Some(&b"cmr10.pfb"[..]));
        assert!(f.is_included() && f.is_subsetted() && f.is_type1());
        assert_eq!(f.fd_flags, -1);

        let f = scan(b"ptmro8r NimbusRomNo9L-Regu \" .167 SlantFont TeXBase1Encoding ReEncodeFont \" <8r.enc <utmr8a.pfb");
        assert_eq!(f.slant, 167);
        assert_eq!(f.encname, Some(&b"8r.enc"[..]));
        assert_eq!(f.ff_name, Some(&b"utmr8a.pfb"[..]));

        let f = scan(b"x Y 4 \"0.5 ExtendFont\" <<a.pfb");
        assert_eq!(f.fd_flags, 4);
        assert_eq!(f.extend, 500);
        assert!(f.is_included() && !f.is_subsetted());

        // No blank: the C's comment expects %f to stop at "0.5E", but on the oracle it fails.
        assert_eq!(scan(b"x Y \"0.5ExtendFont\" <a.pfb").extend, 0);

        let f = scan(b"x Y \" -.167 SlantFont 1 ExtendFont \" << a.pfb");
        assert_eq!(f.slant, -167);
        assert_eq!(f.extend, 0, "1000 means no extension");
        assert!(f.is_included() && !f.is_subsetted());

        let f = scan(b"x Y < a.pfb");
        assert!(f.is_included() && f.is_subsetted());
    }

    #[test]
    fn digits_in_the_second_field() {
        let f = scan(b"x 8r.enc <a.pfb");
        assert_eq!(f.encname, Some(&b"8r.enc"[..]));
        assert_eq!(f.ps_name, None);
        assert_eq!(f.fd_flags, -1);
    }

    #[test]
    fn comments_and_blanks() {
        for s in [&b""[..], b"% x", b"* x", b"# x", b"; x", b"\n"] {
            let r = fm_scan_line(s).unwrap();
            assert!(r.entry.is_none() && r.warnings.is_empty());
        }
        assert!(fm_scan_line(b" ").is_err(), "the C fails an assert on a tfm name that is NULL");
    }

    #[test]
    fn warnings() {
        let r = fm_scan_line(b"x Y \"1 SlantFont <a.pfb").unwrap();
        assert!(r.entry.is_none());
        assert_eq!(r.warnings, vec![b"invalid entry for `x': closing quote missing".to_vec()]);

        let r = fm_scan_line(b"x Y a.pfb").unwrap();
        assert_eq!(r.entry.unwrap().ff_name, None);
        assert_eq!(r.warnings.len(), 1);

        let r = fm_scan_line(b"x \"1.5 SlantFont\" <a.pfb").unwrap();
        assert!(r.entry.is_none());
        assert_eq!(r.warnings, vec![b"invalid entry for `x': SlantFont value too big: 1.5".to_vec()]);

        let r = fm_scan_line(b"x Y \"3 FooBar\" <a.pfb").unwrap();
        assert!(r.entry.is_some());
        assert_eq!(r.warnings, vec![b"invalid entry for `x': unknown name `FooBar' ignored".to_vec()]);

        let r = fm_scan_line(b"x Y PidEid=3,1 <a.ttf").unwrap();
        let f = r.entry.expect("subsetted, not reencoded, TrueType");
        assert_eq!((f.pid, f.eid), (3, 1));

        let r = fm_scan_line(b"x Y PidEid=3, 1 <8r.enc <a.ttf").unwrap();
        assert!(r.entry.is_none(), "not with reencoding: {:?}", r.warnings);
        // In the second field it is the PostScript name, and then 1 is the flags.
        let f = scan(b"x PidEid=3, 1 <8r.enc <a.ttf");
        assert_eq!((f.ps_name, f.fd_flags, f.pid), (Some(&b"PidEid=3,"[..]), 1, -1));
    }

    #[test]
    fn file_kinds() {
        assert!(scan(b"x <a.ttf").is_truetype());
        assert!(scan(b"x <A.TTC").is_truetype());
        assert!(scan(b"x <<a.otf").is_opentype());
        assert!(scan(b"x").is_pk());
        assert!(scan(b"x Y").is_type1());
        assert!(scan(b"x Times-Roman").is_std_t1font());
    }

    #[test]
    fn float_arithmetic_is_single_precision() {
        // The float nearest to .167, times 1000 narrowed to float, plus .5: rounds to 167.
        assert_eq!(fm_float_param(0.167), 167);
        assert_eq!(fm_float_param(-0.167), -167);
        assert_eq!(fm_float_param(0.0005), 1);
        assert_eq!(fm_float_param(1.0005), 1000);
        // Here narrowing to float reaches 4.5 exactly; in double the value stays below it, and gives 4.
        assert_eq!(fm_float_param(0.0045), 5);
    }

    #[test]
    fn std_fonts() {
        assert_eq!(check_std_t1font(b"Courier"), 0);
        assert_eq!(check_std_t1font(b"Courier-Bold"), 1);
        assert_eq!(check_std_t1font(b"Times-Italic"), 11);
        assert_eq!(check_std_t1font(b"ZapfDingbats"), 13);
        assert_eq!(check_std_t1font(b"Helvetica-BoldOblique"), 7);
        assert_eq!(check_std_t1font(b"Foo"), -1);
    }

    #[test]
    fn map_items() {
        let i = process_map_item(b" + foo.map bar", MapType::MapFile);
        assert_eq!((i.mode, i.flush_default, i.item), (FmMode::DupIgnore, false, &b"foo.map"[..]));
        let i = process_map_item(b"foo.map", MapType::MapFile);
        assert_eq!((i.mode, i.flush_default, i.item), (FmMode::DupIgnore, true, &b"foo.map"[..]));
        let i = process_map_item(b"=x Y <a.pfb ", MapType::MapLine);
        assert_eq!((i.mode, i.item), (FmMode::Replace, &b"x Y <a.pfb "[..]));
        let i = process_map_item(b"-x", MapType::MapLine);
        assert_eq!((i.mode, i.item), (FmMode::Delete, &b"x"[..]));
    }

    #[test]
    fn reading_a_map_file_squeezes_blanks() {
        let data = b"a\t b  <c.pfb\r\nnext\nlast";
        let mut pos = 0;
        let mut buf = [0u8; 1024];
        assert_eq!(fm_read_line(data, &mut pos, &mut buf).unwrap().unwrap(), b"a b <c.pfb");
        assert_eq!(fm_read_line(data, &mut pos, &mut buf).unwrap().unwrap(), b"");
        assert_eq!(fm_read_line(data, &mut pos, &mut buf).unwrap().unwrap(), b"next");
        assert_eq!(fm_read_line(data, &mut pos, &mut buf).unwrap().unwrap(), b"last");
        assert!(fm_read_line(data, &mut pos, &mut buf).unwrap().is_none());
    }
}
