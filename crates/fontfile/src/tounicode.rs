//! `tounicode.c`: `\pdfglyphtounicode` and the `/ToUnicode` CMap.
//!
//! The C's `glyph_unicode_tree` is `GlyphUnicodeMap` here, a plain value the caller owns.  A run with
//! `glyphtounicode.tex` defines about 4400 names, so `deftounicode` appends and the table is sorted
//! once, on first use; the last definition of a name wins, as in the C.

use crate::cstr::*;
use crate::{FontError, GlyphNames};

const UNI_UNDEF: i64 = -1;
const UNI_STRING: i64 = -2; // string allocated by deftounicode()
const UNI_EXTRA_STRING: i64 = -3; // string allocated by set_glyph_unicode()

const SMALL_BUF_SIZE: usize = 256;
const PRINTF_BUF_SIZE: usize = 1024;
const TOUNICODE_C: &str = "../../../texk/web2c/pdftexdir/tounicode.c";

#[inline]
fn is_xdigit(c: u8) -> bool {
    c.is_ascii_digit() || (b'A'..=b'F').contains(&c)
}

/// One `glyph_unicode_entry`: plain numbers, so a table can be kept in an image.
#[repr(C)]
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct GuEntry {
    pub name_off: u32,
    pub name_len: u32,
    /// `UNI_UNDEF`, `UNI_STRING`, or a code.
    pub code: i32,
    pub seq_off: u32,
    pub seq_len: u32,
    /// Definition order, which decides between two definitions of a name.
    pub order: u32,
}

/// `glyph_unicode_tree`.
#[derive(Clone, Default, Debug)]
pub struct GlyphUnicodeMap {
    entries: Vec<GuEntry>,
    pool: Vec<u8>,
    sealed: bool,
    any: bool,
}

impl GlyphUnicodeMap {
    pub fn new() -> GlyphUnicodeMap {
        GlyphUnicodeMap { entries: Vec::new(), pool: Vec::new(), sealed: true, any: false }
    }

    /// `glyph_unicode_tree != NULL`.
    pub fn exists(&self) -> bool {
        self.any
    }

    /// The entries, sorted by name, and the bytes they point into: what `dumptounicode` needs.
    pub fn parts(&mut self) -> (&[GuEntry], &[u8]) {
        self.seal();
        (&self.entries, &self.pool)
    }

    /// What `undumptounicode` needs.  `entries` must come from `parts`.
    pub fn from_parts(entries: Vec<GuEntry>, pool: Vec<u8>) -> GlyphUnicodeMap {
        let any = !entries.is_empty();
        GlyphUnicodeMap { entries, pool, sealed: true, any }
    }

    /// `deftounicode`.  Returns the warning, if any.  `glyph` and `unistr` are what `makecstring`
    /// gives: stop them at a NUL first if the string can hold one.
    pub fn deftounicode(&mut self, glyph: &[u8], unistr: &[u8]) -> Result<Option<Vec<u8>>, FontError> {
        if glyph.len() >= SMALL_BUF_SIZE {
            return Err(FontError::assertion(TOUNICODE_C, 72, "deftounicode", "strlen(p) < SMALL_BUF_SIZE"));
        }
        let mut p = unistr;
        while p.first() == Some(&b' ') {
            p = &p[1..]; // ignore leading spaces
        }
        let mut l = p.len();
        while l > 0 && p[l - 1] == b' ' {
            l -= 1; // ignore trailing spaces
        }
        let mut valid_unistr = 1; // 0: invalid; 1: unicode value; 2: string
        for &c in &p[..l] {
            if c == b' ' {
                valid_unistr = 2; // if a space occurs we treat this entry as a string
            } else if !is_xdigit(c) {
                valid_unistr = 0;
                break;
            }
        }
        if l == 0 || valid_unistr == 0 || glyph.is_empty() || glyph == b".notdef" {
            let mut m = b"ToUnicode: invalid parameter(s): `".to_vec();
            m.extend_from_slice(glyph);
            m.extend_from_slice(b"' => `");
            m.extend_from_slice(p);
            m.push(b'\'');
            return Ok(Some(crate::truncate_message(m)));
        }
        self.any = true;
        let name_off = self.pool.len() as u32;
        self.pool.extend_from_slice(glyph);
        let mut e = GuEntry {
            name_off,
            name_len: glyph.len() as u32,
            code: 0,
            seq_off: 0,
            seq_len: 0,
            order: self.entries.len() as u32,
        };
        let mut warning = None;
        if valid_unistr == 2 {
            // a string with space(s): copy p, which goes on past l, ignoring spaces
            e.seq_off = self.pool.len() as u32;
            for &c in p {
                if c != b' ' {
                    self.pool.push(c);
                }
            }
            e.seq_len = self.pool.len() as u32 - e.seq_off;
            e.code = UNI_STRING as i32;
        } else {
            // sscanf(p, "%lX", ...): only hex digits here; the value saturates at ULONG_MAX
            let mut v: u64 = 0;
            for &c in &p[..l] {
                let d = (c as char).to_digit(16).expect("checked above") as u64;
                v = v.checked_mul(16).and_then(|x| x.checked_add(d)).unwrap_or(u64::MAX);
            }
            if v > 0x10FFFF {
                let mut m = b"ToUnicode: value out of range [0,10FFFF]: ".to_vec();
                push_hex(&mut m, v, 0);
                warning = Some(crate::truncate_message(m));
                e.code = UNI_UNDEF as i32;
            } else {
                e.code = v as i32;
            }
        }
        self.sealed = false;
        self.entries.push(e);
        Ok(warning)
    }

    /// Sorts by name; the last definition of a name survives.
    pub fn seal(&mut self) {
        if self.sealed {
            return;
        }
        let pool = &self.pool;
        let name = |e: &GuEntry| &pool[e.name_off as usize..(e.name_off + e.name_len) as usize];
        self.entries.sort_unstable_by(|a, b| name(a).cmp(name(b)).then(a.order.cmp(&b.order)));
        let mut w = 0;
        for r in 0..self.entries.len() {
            let e = self.entries[r];
            if w > 0 && name(&self.entries[w - 1]) == name(&e) {
                self.entries[w - 1] = e;
            } else {
                self.entries[w] = e;
                w += 1;
            }
        }
        self.entries.truncate(w);
        for (i, e) in self.entries.iter_mut().enumerate() {
            e.order = i as u32;
        }
        self.sealed = true;
    }

    fn find(&self, name: &[u8]) -> Option<&GuEntry> {
        let pool = &self.pool;
        self.entries
            .binary_search_by(|e| pool[e.name_off as usize..(e.name_off + e.name_len) as usize].cmp(name))
            .ok()
            .map(|i| &self.entries[i])
    }

    fn seq(&self, e: &GuEntry) -> &[u8] {
        &self.pool[e.seq_off as usize..(e.seq_off + e.seq_len) as usize]
    }
}

/// `check_unicode_value`.  None is `UNI_UNDEF`.
fn check_unicode_value(s: &[u8], multiple_value: bool) -> Option<i64> {
    let l = s.len();
    if l == 0 {
        return None;
    }
    if multiple_value && l % 4 != 0 {
        return None;
    }
    if !multiple_value && !(4..=6).contains(&l) {
        return None;
    }
    let mut code: i64 = 0;
    let mut group: i64 = 0;
    for (i, &c) in s.iter().enumerate() {
        if !is_xdigit(c) {
            return None;
        }
        let d = (c as char).to_digit(16).expect("checked") as i64;
        if multiple_value {
            group = group * 16 + d;
            if i % 4 == 3 {
                code = group;
                group = 0;
                if !((0x0000..=0xD7FF).contains(&code) || (0xE000..=0xFFFF).contains(&code)) {
                    return None;
                }
            }
        } else {
            code = code * 16 + d;
            if i == l - 1 && !((0x0000..=0xD7FF).contains(&code) || (0xE000..=0x10FFFF).contains(&code)) {
                return None;
            }
        }
    }
    Some(code)
}

/// `utf16be_str`.
fn push_utf16be(out: &mut Vec<u8>, code: i64) {
    if code <= 0xFFFF {
        push_hex(out, code as u64, 4);
    } else {
        let v = code - 0x10000;
        push_hex(out, (v / 0x400 + 0xD800) as u64, 4);
        push_hex(out, (v % 0x400 + 0xDC00) as u64, 4);
    }
}

/// `is_last_byte_valid`: the last two hex digits of `utf16be_str(code)` are its low byte.
fn is_last_byte_valid(src_code1: i32, src_code2: i32, code: i64) -> bool {
    let low = if code <= 0xFFFF { code } else { (code - 0x10000) % 0x400 + 0xDC00 };
    (low & 0xFF) < 255 - (src_code2 - src_code1) as i64
}

/// `gtab[i]`: a code, or a string in `seqs`.
#[derive(Copy, Clone)]
struct G {
    code: i64,
    seq_start: u32,
    seq_end: u32,
}

const G_UNDEF: G = G { code: UNI_UNDEF, seq_start: 0, seq_end: 0 };

/// `set_glyph_unicode`.
fn set_glyph_unicode(s: &[u8], tfmname: &[u8], map: &GlyphUnicodeMap, seqs: &mut Vec<u8>) -> Result<G, FontError> {
    let mut g = G_UNDEF;
    // strip everything after the first dot
    let s = match s.iter().position(|&c| c == b'.') {
        Some(p) => &s[..p],
        None => s,
    };
    if s.is_empty() {
        return Ok(g);
    }
    // check for case of multiple components separated by '_'
    if s.contains(&b'_') {
        if s.len() >= SMALL_BUF_SIZE {
            return Err(FontError::assertion(TOUNICODE_C, 222, "set_glyph_unicode", "strlen(s) < sizeof(buf)"));
        }
        // buf2 holds what the components give, here at the end of seqs.
        let start = seqs.len() as u32;
        for comp in s.split(|&c| c == b'_') {
            let t = set_glyph_unicode(comp, tfmname, map, seqs)?;
            match t.code {
                UNI_UNDEF => {} // not found, do nothing
                UNI_STRING | UNI_EXTRA_STRING => {
                    // already at the end of seqs: a component has no '_'
                    if seqs.len() - start as usize >= SMALL_BUF_SIZE {
                        return Err(FontError::assertion(
                            TOUNICODE_C,
                            239,
                            "set_glyph_unicode",
                            "strlen(buf2) + strlen(tmp.unicode_seq) < sizeof(buf2)",
                        ));
                    }
                }
                code => push_utf16be(seqs, code),
            }
        }
        g.code = UNI_EXTRA_STRING;
        g.seq_start = start;
        g.seq_end = seqs.len() as u32;
        return Ok(g);
    }
    // Glyph name search strategy: first look up the glyph name in the tfm's namespace, failing that
    // look it up in the main database.
    // snprintf(buf2, SMALL_BUF_SIZE, "tfm:%s/%s", tfmname, s)
    let mut key = [0u8; SMALL_BUF_SIZE];
    let mut k = 0;
    for part in [&b"tfm:"[..], tfmname, b"/", s] {
        let n = part.len().min(SMALL_BUF_SIZE - 1 - k);
        key[k..k + n].copy_from_slice(&part[..n]);
        k += n;
    }
    let found = match map.find(&key[..k]) {
        Some(e) => Some(e),
        None => {
            let t = if s.len() > SMALL_BUF_SIZE - 1 { &s[..SMALL_BUF_SIZE - 1] } else { s };
            map.find(t)
        }
    };
    if let Some(e) = found {
        g.code = e.code as i64;
        if g.code == UNI_STRING {
            let start = seqs.len() as u32;
            seqs.extend_from_slice(map.seq(e));
            g.seq_start = start;
            g.seq_end = seqs.len() as u32;
        }
        return Ok(g);
    }
    // check for case of "uniXXXX" (multiple 4-hex-digit values allowed)
    if s.starts_with(b"uni") {
        let p2 = &s[3..];
        if let Some(code) = check_unicode_value(p2, true) {
            if p2.len() == 4 {
                g.code = code; // single value
            } else {
                // multiple value
                g.code = UNI_EXTRA_STRING;
                g.seq_start = seqs.len() as u32;
                seqs.extend_from_slice(p2);
                g.seq_end = seqs.len() as u32;
            }
        }
        return Ok(g); // since the last case cannot happen
    }
    // check for case of "uXXXX" (single value up to 6 hex digits)
    if s.starts_with(b"u") {
        if let Some(code) = check_unicode_value(&s[1..], false) {
            g.code = code;
        }
    }
    Ok(g)
}

/// The CMap name `<tfmname>-<encname>` of `write_tounicode`: `buf` in the C.  `encname` None is a
/// builtin encoding.
fn cmap_name(tfmname: &[u8], encname: Option<&[u8]>, warnings: &mut Vec<Vec<u8>>) -> Result<Vec<u8>, FontError> {
    let mut buf = tfmname.to_vec();
    buf.push(b'-');
    match encname {
        Some(e) => {
            if tfmname.len() + e.len() + 1 >= SMALL_BUF_SIZE {
                return Err(FontError::assertion(
                    TOUNICODE_C,
                    357,
                    "write_tounicode",
                    "strlen(tfmname) + strlen(encname) + 1 < SMALL_BUF_SIZE",
                ));
            }
            buf.extend_from_slice(e);
            let stripped = match buf.iter().rposition(|&c| c == b'.') {
                Some(p) if &buf[p..] == b".enc" => Some(p),
                _ => None,
            };
            match stripped {
                Some(p) => buf.truncate(p), // strip ".enc" from encoding name
                None => {
                    // some silly encoding file name not ending in enc; use as-is
                    let mut m = b"Dubious encoding file name: `".to_vec();
                    m.extend_from_slice(e);
                    m.push(b'\'');
                    warnings.push(crate::truncate_message(m));
                }
            }
        }
        None => {
            if tfmname.len() + 7 + 1 >= SMALL_BUF_SIZE {
                return Err(FontError::assertion(
                    TOUNICODE_C,
                    364,
                    "write_tounicode",
                    "strlen(tfmname) + strlen(builtin_suffix) + 1 < SMALL_BUF_SIZE",
                ));
            }
            buf.extend_from_slice(b"builtin");
        }
    }
    Ok(buf)
}

/// The stream of `write_tounicode`, appended to `out`; the warnings are returned.
///
/// The caller does what surrounds it: if `!map.exists()`, warn "no GlyphToUnicode entry has been
/// inserted yet!", set `fixedgentounicode` to 0 and write nothing; otherwise `pdfnewobjnum`,
/// `pdfbegindict(objnum, 0)`, `pdfbeginstream`, this, `pdfendstream`.
pub fn write_tounicode(
    glyph_names: &GlyphNames,
    tfmname: &[u8],
    encname: Option<&[u8]>,
    map: &mut GlyphUnicodeMap,
    out: &mut Vec<u8>,
) -> Result<Vec<Vec<u8>>, FontError> {
    let mut warnings = Vec::new();
    map.seal();
    let map = &*map;
    let buf = cmap_name(tfmname, encname, &mut warnings)?;
    let name = &buf[..];
    // One pdf_printf, so one vsnprintf into print_buf: a long name cuts the header short.
    let mut header: Vec<u8> = Vec::with_capacity(PRINTF_BUF_SIZE);
    header.extend_from_slice(b"%!PS-Adobe-3.0 Resource-CMap\n%%DocumentNeededResources: ProcSet (CIDInit)\n%%IncludeResource: ProcSet (CIDInit)\n%%BeginResource: CMap (TeX-");
    header.extend_from_slice(name);
    header.extend_from_slice(b"-0)\n%%Title: (TeX-");
    header.extend_from_slice(name);
    header.extend_from_slice(b"-0 TeX ");
    header.extend_from_slice(name);
    header.extend_from_slice(b" 0)\n%%Version: 1.000\n%%EndComments\n/CIDInit /ProcSet findresource begin\n12 dict begin\nbegincmap\n/CIDSystemInfo\n<< /Registry (TeX)\n/Ordering (");
    header.extend_from_slice(name);
    header.extend_from_slice(b")\n/Supplement 0\n>> def\n/CMapName /TeX-");
    header.extend_from_slice(name);
    header.extend_from_slice(b"-0 def\n/CMapType 2 def\n1 begincodespacerange\n<00> <FF>\nendcodespacerange\n");
    header.truncate(PRINTF_BUF_SIZE - 1);
    out.extend_from_slice(&header);

    // set gtab
    let mut seqs: Vec<u8> = Vec::new();
    let mut gtab = [G_UNDEF; 257];
    for (i, g) in gtab.iter_mut().enumerate().take(256) {
        if !glyph_names.is_notdef(i) {
            *g = set_glyph_unicode(glyph_names.name(i), tfmname, map, &mut seqs).map_err(|mut e| {
                e.warnings = warnings.clone();
                e
            })?;
        }
    }

    // set range_size
    let mut range_size = [0i16; 257];
    let mut i: i32 = 0;
    while i < 256 {
        let c = gtab[i as usize].code;
        if c == UNI_STRING || c == UNI_EXTRA_STRING {
            range_size[i as usize] = 1; // single entry
            i += 1;
        } else if c == UNI_UNDEF {
            range_size[i as usize] = 0; // no entry
            i += 1;
        } else {
            // gtab[i].code >= 0
            let j = i;
            while i < 256
                && gtab[i as usize + 1].code >= 0
                && gtab[i as usize].code + 1 == gtab[i as usize + 1].code
                && is_last_byte_valid(j, i, gtab[i as usize].code)
            {
                i += 1;
            }
            // at this point i is the last entry of the subrange
            i += 1; // move i to the next entry
            range_size[j as usize] = (i - j) as i16;
        }
    }

    // calculate bfrange_count and bfchar_count
    let mut bfrange_count = 0;
    let mut bfchar_count = 0;
    let mut i: usize = 0;
    while i < 256 {
        if range_size[i] == 1 {
            bfchar_count += 1;
            i += 1;
        } else if range_size[i] > 1 {
            bfrange_count += 1;
            i += range_size[i] as usize;
        } else {
            i += 1;
        }
    }

    // write out bfrange
    let mut i: usize = 0;
    loop {
        let subrange_count = if bfrange_count > 100 { 100 } else { bfrange_count };
        bfrange_count -= subrange_count;
        push_i64(out, subrange_count as i64);
        out.extend_from_slice(b" beginbfrange\n");
        for _ in 0..subrange_count {
            while range_size[i] <= 1 && i < 256 {
                i += 1;
            }
            out.push(b'<');
            push_hex(out, i as u64, 2);
            out.extend_from_slice(b"> <");
            push_hex(out, (i as i64 + range_size[i] as i64 - 1) as u64, 2);
            out.extend_from_slice(b"> <");
            push_utf16be(out, gtab[i].code);
            out.extend_from_slice(b">\n");
            i += range_size[i] as usize;
        }
        out.extend_from_slice(b"endbfrange\n");
        if bfrange_count <= 0 {
            break;
        }
    }

    // write out bfchar
    let mut i: usize = 0;
    loop {
        let subrange_count = if bfchar_count > 100 { 100 } else { bfchar_count };
        bfchar_count -= subrange_count;
        push_i64(out, subrange_count as i64);
        out.extend_from_slice(b" beginbfchar\n");
        for _ in 0..subrange_count {
            while i < 256 {
                if range_size[i] > 1 {
                    i += range_size[i] as usize;
                } else if range_size[i] == 0 {
                    i += 1;
                } else {
                    break; // range_size[i] == 1
                }
            }
            out.push(b'<');
            push_hex(out, i as u64, 2);
            out.extend_from_slice(b"> <");
            let g = gtab[i];
            if g.code == UNI_STRING || g.code == UNI_EXTRA_STRING {
                out.extend_from_slice(&seqs[g.seq_start as usize..g.seq_end as usize]);
            } else {
                push_utf16be(out, g.code);
            }
            out.extend_from_slice(b">\n");
            i += 1;
        }
        out.extend_from_slice(b"endbfchar\n");
        if bfchar_count <= 0 {
            break;
        }
    }
    out.extend_from_slice(b"endcmap\nCMapName currentdict /CMap defineresource pop\nend\nend\n%%EndResource\n%%EOF\n");
    Ok(warnings)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lookup(m: &mut GlyphUnicodeMap, glyph: &[u8]) -> (i64, Vec<u8>) {
        m.seal();
        let mut seqs = Vec::new();
        let g = set_glyph_unicode(glyph, b"cmr10", m, &mut seqs).unwrap();
        (g.code, seqs[g.seq_start as usize..g.seq_end as usize].to_vec())
    }

    #[test]
    fn deftounicode_forms() {
        let mut m = GlyphUnicodeMap::new();
        assert!(!m.exists());
        assert!(m.deftounicode(b"A", b"0041").unwrap().is_none());
        assert!(m.exists());
        assert!(m.deftounicode(b"ffi", b"0066 0066 0069").unwrap().is_none());
        assert!(m.deftounicode(b"", b"0041").unwrap().is_some());
        assert!(m.deftounicode(b".notdef", b"0041").unwrap().is_some());
        assert!(m.deftounicode(b"x", b"zz").unwrap().is_some());
        assert!(m.deftounicode(b"x", b"  ").unwrap().is_some());
        let w = m.deftounicode(b"big", b"110000").unwrap().unwrap();
        assert_eq!(w, b"ToUnicode: value out of range [0,10FFFF]: 110000".to_vec());
        assert_eq!(lookup(&mut m, b"A"), (0x41, vec![]));
        assert_eq!(lookup(&mut m, b"ffi"), (UNI_STRING, b"006600660069".to_vec()));
        assert_eq!(lookup(&mut m, b"big"), (UNI_UNDEF, vec![]));
        // later definitions win
        assert!(m.deftounicode(b"A", b"0061").unwrap().is_none());
        assert_eq!(lookup(&mut m, b"A"), (0x61, vec![]));
    }

    #[test]
    fn glyph_name_forms() {
        let mut m = GlyphUnicodeMap::new();
        m.deftounicode(b"A", b"0041").unwrap();
        m.deftounicode(b"B", b"0042").unwrap();
        m.deftounicode(b"ffi", b"0066 0066 0069").unwrap();
        m.deftounicode(b"tfm:cmr10/B", b"0062").unwrap();
        // the tfm's namespace goes first
        assert_eq!(lookup(&mut m, b"B").0, 0x62);
        // everything after the first dot goes
        assert_eq!(lookup(&mut m, b"A.sc").0, 0x41);
        assert_eq!(lookup(&mut m, b".notdefx").0, UNI_UNDEF);
        // uniXXXX, one or several
        assert_eq!(lookup(&mut m, b"uni0041").0, 0x41);
        assert_eq!(lookup(&mut m, b"uni00410042"), (UNI_EXTRA_STRING, b"00410042".to_vec()));
        assert_eq!(lookup(&mut m, b"uniD800").0, UNI_UNDEF);
        assert_eq!(lookup(&mut m, b"uni004").0, UNI_UNDEF);
        // uXXXX to uXXXXXX
        assert_eq!(lookup(&mut m, b"u1F600").0, 0x1F600);
        assert_eq!(lookup(&mut m, b"u41").0, UNI_UNDEF);
        // components
        assert_eq!(lookup(&mut m, b"A_ffi_u1F600_nope"), (UNI_EXTRA_STRING, b"0041006600660069D83DDE00".to_vec()));
    }

    #[test]
    fn dubious_encoding_names() {
        // kpathsea does not find such a file, so the oracle cannot show this; the text is the C's.
        let mut w = Vec::new();
        assert_eq!(cmap_name(b"t", Some(b"x.ENC"), &mut w).unwrap(), b"t-x.ENC".to_vec());
        assert_eq!(w, vec![b"Dubious encoding file name: `x.ENC'".to_vec()]);
        let mut w = Vec::new();
        assert_eq!(cmap_name(b"t", Some(b"y.enc"), &mut w).unwrap(), b"t-y".to_vec());
        assert_eq!(cmap_name(b"t", None, &mut w).unwrap(), b"t-builtin".to_vec());
        assert!(w.is_empty());
        let long = vec![b'a'; 300];
        assert!(cmap_name(&long, None, &mut w).is_err());
        assert!(cmap_name(b"t", Some(&long), &mut w).is_err());
    }

    #[test]
    fn utf16() {
        let mut b = Vec::new();
        push_utf16be(&mut b, 0x41);
        push_utf16be(&mut b, 0x10000);
        push_utf16be(&mut b, 0x10FFFF);
        assert_eq!(b, b"0041D800DC00DBFFDFFF".to_vec());
    }
}
