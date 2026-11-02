//! What PDF inclusion needs of xpdf's fonts: `GfxFont::getFontType`, the encoding part of `Gfx8BitFont::Gfx8BitFont`,
//! `FoFiIdentifier`, `FoFiType1` and `FoFiType1C`.
//!
//! `writeEncodings` in pdftoepdf.cc asks a `Gfx8BitFont` for the glyph name of each code and writes them out as the
//! replacement font's `/Encoding`.  The rest of those classes is not reached.

use crate::enc_tables::*;
use crate::pdf::{Dict, Doc, Object};

/// `GfxFontType`, with the order `GfxFont::makeFont` compares on.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
enum FontType {
    Unknown,
    Type1,
    Type1C,
    Type1COT,
    Type3,
    TrueType,
    TrueTypeOT,
    CidType0,
    CidType0C,
    CidType0COT,
    CidType2,
    CidType2OT,
}

/// `FoFiIdentifierType`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum FoFiId {
    Type1Pfa,
    Type1Pfb,
    Cff8Bit,
    CffCid,
    TrueType,
    TrueTypeCollection,
    OpenTypeCff8Bit,
    OpenTypeCffCid,
    Unknown,
}

/// `Reader` over the bytes of the font file.
struct Rd<'a>(&'a [u8]);

impl Rd<'_> {
    fn byte(&self, pos: i64) -> i32 {
        if pos < 0 || pos as usize >= self.0.len() { -1 } else { self.0[pos as usize] as i32 }
    }

    fn u16be(&self, pos: i64) -> Option<i32> {
        if pos < 0 || pos > i32::MAX as i64 - 1 || pos as usize + 2 > self.0.len() {
            return None;
        }
        let p = pos as usize;
        Some(((self.0[p] as i32) << 8) | self.0[p + 1] as i32)
    }

    fn u32le(&self, pos: i64) -> Option<u32> {
        if pos < 0 || pos as usize + 4 > self.0.len() {
            return None;
        }
        let p = pos as usize;
        Some(u32::from_le_bytes([self.0[p], self.0[p + 1], self.0[p + 2], self.0[p + 3]]))
    }

    fn uvar_be(&self, pos: i64, size: i32) -> Option<u32> {
        if pos < 0 || !(1..=4).contains(&size) || pos as usize + size as usize > self.0.len() {
            return None;
        }
        let mut v = 0u32;
        for i in 0..size as usize {
            v = (v << 8) | self.0[pos as usize + i] as u32;
        }
        Some(v)
    }

    fn cmp(&self, pos: usize, s: &[u8]) -> bool {
        self.0.len() >= pos + s.len() && &self.0[pos..pos + s.len()] == s
    }
}

/// `identify`.
fn identify(d: &[u8]) -> FoFiId {
    let r = Rd(d);
    if r.cmp(0, b"%!PS-AdobeFont-1") || r.cmp(0, b"%!FontType1") {
        return FoFiId::Type1Pfa;
    }
    if r.byte(0) == 0x80 && r.byte(1) == 0x01 {
        if let Some(n) = r.u32le(2) {
            if (n >= 16 && r.cmp(6, b"%!PS-AdobeFont-1")) || (n >= 11 && r.cmp(6, b"%!FontType1")) {
                return FoFiId::Type1Pfb;
            }
        }
    }
    if (r.byte(0) == 0 && r.byte(1) == 1 && r.byte(2) == 0 && r.byte(3) == 0) || r.cmp(0, b"true") {
        return FoFiId::TrueType;
    }
    if r.cmp(0, b"ttcf") {
        return FoFiId::TrueTypeCollection;
    }
    if r.cmp(0, b"OTTO") {
        return identify_open_type(&r);
    }
    if r.byte(0) == 1 && r.byte(1) == 0 {
        return identify_cff(&r, 0);
    }
    if r.byte(1) == 1 && r.byte(2) == 0 {
        return identify_cff(&r, 1);
    }
    FoFiId::Unknown
}

fn identify_open_type(r: &Rd<'_>) -> FoFiId {
    let Some(n) = r.u16be(4) else {
        return FoFiId::Unknown;
    };
    for i in 0..n as usize {
        if r.cmp(12 + i * 16, b"CFF ") {
            if let Some(offset) = r.uvar_be(12 + i as i64 * 16 + 8, 4) {
                if offset < i32::MAX as u32 {
                    return match identify_cff(r, offset as i64) {
                        FoFiId::Cff8Bit => FoFiId::OpenTypeCff8Bit,
                        FoFiId::CffCid => FoFiId::OpenTypeCffCid,
                        t => t,
                    };
                }
            }
            return FoFiId::Unknown;
        }
    }
    FoFiId::Unknown
}

fn identify_cff(r: &Rd<'_>, start: i64) -> FoFiId {
    if r.byte(start) != 1 || r.byte(start + 1) != 0 {
        return FoFiId::Unknown;
    }
    let hdr_size = r.byte(start + 2);
    if hdr_size < 0 {
        return FoFiId::Unknown;
    }
    let off0 = r.byte(start + 3);
    if !(1..=4).contains(&off0) {
        return FoFiId::Unknown;
    }
    let mut pos = start + hdr_size as i64;
    let Some(n) = r.u16be(pos) else {
        return FoFiId::Unknown;
    };
    if n == 0 {
        pos += 2;
    } else {
        let off1 = r.byte(pos + 2);
        if !(1..=4).contains(&off1) {
            return FoFiId::Unknown;
        }
        let Some(offset1) = r.uvar_be(pos + 3 + n as i64 * off1 as i64, off1) else {
            return FoFiId::Unknown;
        };
        if offset1 > i32::MAX as u32 {
            return FoFiId::Unknown;
        }
        pos += 3 + (n as i64 + 1) * off1 as i64 + offset1 as i64 - 1;
    }
    if pos < 0 {
        return FoFiId::Unknown;
    }
    let n = match r.u16be(pos) {
        Some(n) if n >= 1 => n,
        _ => return FoFiId::Unknown,
    };
    let off1 = r.byte(pos + 2);
    if !(1..=4).contains(&off1) {
        return FoFiId::Unknown;
    }
    let (Some(offset0), Some(offset1)) =
        (r.uvar_be(pos + 3, off1), r.uvar_be(pos + 3 + off1 as i64, off1))
    else {
        return FoFiId::Unknown;
    };
    if offset0 > i32::MAX as u32 || offset1 > i32::MAX as u32 || offset0 > offset1 {
        return FoFiId::Unknown;
    }
    let base = pos + 3 + (n as i64 + 1) * off1 as i64 - 1;
    let mut p = base + offset0 as i64;
    let end = base + offset1 as i64;
    while p >= 0 && p < end {
        let b0 = r.byte(p);
        if b0 == 0x1c {
            p += 3;
        } else if b0 == 0x1d {
            p += 5;
        } else if (0xf7..=0xfe).contains(&b0) {
            p += 2;
        } else if (0x20..=0xf6).contains(&b0) {
            p += 1;
        } else {
            break;
        }
    }
    if p + 1 < end && r.byte(p) == 12 && r.byte(p + 1) == 30 { FoFiId::CffCid } else { FoFiId::Cff8Bit }
}

/// `GfxFont::getFontType`: the type, and the embedded file's reference if it could be identified.
fn get_font_type(doc: &mut Doc, font_dict: &Dict) -> (FontType, Option<(i32, i32)>) {
    let subtype = doc.lookup(font_dict, b"Subtype");
    let mut expected = FontType::Unknown;
    let mut is_type0 = false;
    if subtype.is_name(b"Type1") || subtype.is_name(b"MMType1") {
        expected = FontType::Type1;
    } else if subtype.is_name(b"Type1C") {
        expected = FontType::Type1C;
    } else if subtype.is_name(b"Type3") {
        expected = FontType::Type3;
    } else if subtype.is_name(b"TrueType") {
        expected = FontType::TrueType;
    } else if subtype.is_name(b"Type0") {
        is_type0 = true;
    }
    let mut font_dict2: Option<std::sync::Arc<Dict>> = None;
    if let Object::Array(a) = doc.lookup(font_dict, b"DescendantFonts") {
        if !a.is_empty() {
            if let Object::Dict(d2) = doc.array_get(&a, 0) {
                let st = doc.lookup(&d2, b"Subtype");
                if st.is_name(b"CIDFontType0") {
                    if is_type0 {
                        expected = FontType::CidType0;
                    }
                } else if st.is_name(b"CIDFontType2") && is_type0 {
                    expected = FontType::CidType2;
                }
                font_dict2 = Some(d2);
            }
        }
    }
    let mut emb: Option<(i32, i32)> = None;
    let fd = match &font_dict2 {
        Some(d) => doc.lookup(d, b"FontDescriptor"),
        None => doc.lookup(font_dict, b"FontDescriptor"),
    };
    if let Object::Dict(fd) = fd {
        if let Object::Ref(n, g) = fd.get_nf(b"FontFile3") {
            emb = Some((n, g));
            if let Object::Stream(s) = doc.fetch(n, g, 0) {
                let st = doc.lookup(&s.dict, b"Subtype");
                if st.is_name(b"Type1") {
                    if expected != FontType::Type1 {
                        expected = if is_type0 { FontType::CidType0 } else { FontType::Type1 };
                    }
                } else if st.is_name(b"Type1C") {
                    if expected == FontType::Type1 {
                        expected = FontType::Type1C;
                    } else if expected != FontType::Type1C {
                        expected = if is_type0 { FontType::CidType0C } else { FontType::Type1C };
                    }
                } else if st.is_name(b"TrueType") {
                    if expected != FontType::TrueType {
                        expected = if is_type0 { FontType::CidType2 } else { FontType::TrueType };
                    }
                } else if st.is_name(b"CIDFontType0C") {
                    expected =
                        if expected == FontType::CidType0 || is_type0 { FontType::CidType0C } else { FontType::Type1C };
                } else if st.is_name(b"OpenType") {
                    if expected == FontType::TrueType {
                        expected = FontType::TrueTypeOT;
                    } else if expected == FontType::Type1 {
                        expected = FontType::Type1COT;
                    } else if expected == FontType::CidType0 {
                        expected = FontType::CidType0COT;
                    } else if expected == FontType::CidType2 {
                        expected = FontType::CidType2OT;
                    }
                }
            }
        }
        if emb.is_none() {
            if let Object::Ref(n, g) = fd.get_nf(b"FontFile2") {
                emb = Some((n, g));
                if is_type0 {
                    expected = FontType::CidType2;
                }
            }
        }
        if emb.is_none() {
            if let Object::Ref(n, g) = fd.get_nf(b"FontFile") {
                emb = Some((n, g));
            }
        }
    }
    let mut t = FontType::Unknown;
    if let Some((n, g)) = emb {
        if let Object::Stream(s) = doc.fetch(n, g, 0) {
            // identifyStream reads forward as far as it needs; the decoded file gives the same answers.
            let d = doc.decode_stream(&s, 0);
            t = match identify(&d.data) {
                FoFiId::Type1Pfa | FoFiId::Type1Pfb => FontType::Type1,
                FoFiId::Cff8Bit => {
                    if is_type0 {
                        FontType::CidType0C
                    } else {
                        FontType::Type1C
                    }
                }
                FoFiId::CffCid => FontType::CidType0C,
                FoFiId::TrueType | FoFiId::TrueTypeCollection => {
                    if is_type0 {
                        FontType::CidType2
                    } else {
                        FontType::TrueType
                    }
                }
                FoFiId::OpenTypeCff8Bit => {
                    if is_type0 {
                        FontType::CidType0COT
                    } else {
                        FontType::Type1COT
                    }
                }
                FoFiId::OpenTypeCffCid => FontType::CidType0COT,
                FoFiId::Unknown => FontType::Unknown,
            };
        }
    }
    if t == FontType::Unknown {
        emb = None;
        t = expected;
    }
    (t, emb)
}

/// `readEmbFontFile`.
fn read_emb_font_file(doc: &mut Doc, emb: (i32, i32)) -> Option<Vec<u8>> {
    match doc.fetch(emb.0, emb.1, 0) {
        Object::Stream(s) => Some(doc.decode_stream(&s, 0).data),
        _ => None,
    }
}

/// `base14FontMap`, the alternate names that lead to Symbol or ZapfDingbats.  Only a non-embedded one of those two has
/// an encoding other than the standard one, so the other base-14 names need only be recognised as base-14.
const BASE14_NAMES: &[(&str, &str)] = &[
    ("Arial", "Helvetica"),
    ("Arial,Bold", "Helvetica-Bold"),
    ("Arial,BoldItalic", "Helvetica-BoldOblique"),
    ("Arial,Italic", "Helvetica-Oblique"),
    ("Arial-Bold", "Helvetica-Bold"),
    ("Arial-BoldItalic", "Helvetica-BoldOblique"),
    ("Arial-BoldItalicMT", "Helvetica-BoldOblique"),
    ("Arial-BoldMT", "Helvetica-Bold"),
    ("Arial-Italic", "Helvetica-Oblique"),
    ("Arial-ItalicMT", "Helvetica-Oblique"),
    ("ArialMT", "Helvetica"),
    ("Courier", "Courier"),
    ("Courier,Bold", "Courier-Bold"),
    ("Courier,BoldItalic", "Courier-BoldOblique"),
    ("Courier,Italic", "Courier-Oblique"),
    ("Courier-Bold", "Courier-Bold"),
    ("Courier-BoldOblique", "Courier-BoldOblique"),
    ("Courier-Oblique", "Courier-Oblique"),
    ("CourierNew", "Courier"),
    ("CourierNew,Bold", "Courier-Bold"),
    ("CourierNew,BoldItalic", "Courier-BoldOblique"),
    ("CourierNew,Italic", "Courier-Oblique"),
    ("CourierNew-Bold", "Courier-Bold"),
    ("CourierNew-BoldItalic", "Courier-BoldOblique"),
    ("CourierNew-Italic", "Courier-Oblique"),
    ("CourierNewPS-BoldItalicMT", "Courier-BoldOblique"),
    ("CourierNewPS-BoldMT", "Courier-Bold"),
    ("CourierNewPS-ItalicMT", "Courier-Oblique"),
    ("CourierNewPSMT", "Courier"),
    ("Helvetica", "Helvetica"),
    ("Helvetica,Bold", "Helvetica-Bold"),
    ("Helvetica,BoldItalic", "Helvetica-BoldOblique"),
    ("Helvetica,Italic", "Helvetica-Oblique"),
    ("Helvetica-Bold", "Helvetica-Bold"),
    ("Helvetica-BoldItalic", "Helvetica-BoldOblique"),
    ("Helvetica-BoldOblique", "Helvetica-BoldOblique"),
    ("Helvetica-Italic", "Helvetica-Oblique"),
    ("Helvetica-Oblique", "Helvetica-Oblique"),
    ("Symbol", "Symbol"),
    ("Symbol,Bold", "Symbol"),
    ("Symbol,BoldItalic", "Symbol"),
    ("Symbol,Italic", "Symbol"),
    ("Times-Bold", "Times-Bold"),
    ("Times-BoldItalic", "Times-BoldItalic"),
    ("Times-Italic", "Times-Italic"),
    ("Times-Roman", "Times-Roman"),
    ("TimesNewRoman", "Times-Roman"),
    ("TimesNewRoman,Bold", "Times-Bold"),
    ("TimesNewRoman,BoldItalic", "Times-BoldItalic"),
    ("TimesNewRoman,Italic", "Times-Italic"),
    ("TimesNewRoman-Bold", "Times-Bold"),
    ("TimesNewRoman-BoldItalic", "Times-BoldItalic"),
    ("TimesNewRoman-Italic", "Times-Italic"),
    ("TimesNewRomanPS", "Times-Roman"),
    ("TimesNewRomanPS-Bold", "Times-Bold"),
    ("TimesNewRomanPS-BoldItalic", "Times-BoldItalic"),
    ("TimesNewRomanPS-BoldItalicMT", "Times-BoldItalic"),
    ("TimesNewRomanPS-BoldMT", "Times-Bold"),
    ("TimesNewRomanPS-Italic", "Times-Italic"),
    ("TimesNewRomanPS-ItalicMT", "Times-Italic"),
    ("TimesNewRomanPSMT", "Times-Roman"),
    ("TimesNewRomanPSMT,Bold", "Times-Bold"),
    ("TimesNewRomanPSMT,BoldItalic", "Times-BoldItalic"),
    ("TimesNewRomanPSMT,Italic", "Times-Italic"),
    ("ZapfDingbats", "ZapfDingbats"),
];

type Enc = Vec<Option<Vec<u8>>>;

fn table_enc(t: &[Option<&'static str>; 256]) -> Enc {
    t.iter().map(|o| o.map(|s| s.as_bytes().to_vec())).collect()
}

/// The result of `GfxFont::makeFont`, as `writeEncodings` uses it.
pub(crate) struct FontEnc {
    /// `isCIDFont()`: `writeEncodings` stops with an error.
    pub is_cid: bool,
    /// `getCharName(0..255)`.
    pub names: Enc,
}

/// `GfxFont::makeFont` and the encoding part of `Gfx8BitFont::Gfx8BitFont`.
pub(crate) fn make_font_enc(doc: &mut Doc, font_dict: &Dict) -> FontEnc {
    let name: Option<Vec<u8>> = match doc.lookup(font_dict, b"BaseFont") {
        Object::Name(n) => Some(n.as_ref().clone()),
        Object::Str(s) => Some(s.as_ref().clone()),
        _ => None,
    };
    let (ty, emb) = get_font_type(doc, font_dict);
    if ty >= FontType::CidType0 {
        return FontEnc { is_cid: true, names: Vec::new() };
    }
    // base 14: spaces do not count in the name.
    let base14: Option<&str> = name.as_ref().and_then(|n| {
        let n2: Vec<u8> = n.iter().copied().filter(|&c| c != b' ').collect();
        BASE14_NAMES.iter().find(|(alt, _)| alt.as_bytes() == n2.as_slice()).map(|(_, b)| *b)
    });
    let builtin_symbol = base14 == Some("Symbol");
    let builtin_zapf = base14 == Some("ZapfDingbats");

    let enc_obj = doc.lookup(font_dict, b"Encoding");
    let mut base_enc: Option<Enc> = None;
    match &enc_obj {
        Object::Dict(d) => {
            let b = doc.lookup(d, b"BaseEncoding");
            if b.is_name(b"MacRomanEncoding") {
                base_enc = Some(table_enc(&MAC_ROMAN));
            } else if b.is_name(b"MacExpertEncoding") {
                base_enc = Some(table_enc(&MAC_EXPERT));
            } else if b.is_name(b"WinAnsiEncoding") {
                base_enc = Some(table_enc(&WIN_ANSI));
            }
        }
        o => {
            if o.is_name(b"MacRomanEncoding") {
                base_enc = Some(table_enc(&MAC_ROMAN));
            } else if o.is_name(b"MacExpertEncoding") {
                base_enc = Some(table_enc(&MAC_EXPERT));
            } else if o.is_name(b"WinAnsiEncoding") {
                base_enc = Some(table_enc(&WIN_ANSI));
            }
        }
    }
    // A non-embedded Symbol or ZapfDingbats font should never use one of the non-symbol encodings.
    if (builtin_symbol || builtin_zapf) && emb.is_none() && !matches!(enc_obj, Object::Dict(_)) {
        base_enc = None;
    }
    let mut base_from_file = false;
    let mut emb_id = emb;
    if ty == FontType::Type1 {
        if let Some(e) = emb_id {
            match read_emb_font_file(doc, e) {
                Some(buf) => {
                    if base_enc.is_none() {
                        base_enc = type1_encoding(&buf);
                        base_from_file = true;
                    }
                }
                None => emb_id = None,
            }
        }
    } else if ty == FontType::Type1C {
        if let Some(e) = emb_id {
            match read_emb_font_file(doc, e) {
                Some(buf) => {
                    if let Some(enc) = type1c_encoding(&buf) {
                        if base_enc.is_none() {
                            base_enc = enc;
                            base_from_file = true;
                        }
                    }
                }
                None => emb_id = None,
            }
        }
    }
    let mut enc: Enc = match base_enc {
        Some(e) => e,
        None => {
            if base14.is_some() && emb_id.is_none() {
                if builtin_symbol {
                    table_enc(&SYMBOL)
                } else if builtin_zapf {
                    table_enc(&ZAPF_DINGBATS)
                } else {
                    table_enc(&STANDARD)
                }
            } else if ty == FontType::TrueType {
                table_enc(&WIN_ANSI)
            } else {
                table_enc(&STANDARD)
            }
        }
    };
    // some Type 1C font files have empty encodings: fill in any gaps from StandardEncoding
    if ty == FontType::Type1C && emb_id.is_some() && base_from_file {
        for i in 0..256 {
            if enc[i].is_none() {
                enc[i] = STANDARD[i].map(|s| s.as_bytes().to_vec());
            }
        }
    }
    // merge differences into encoding
    if let Object::Dict(d) = &enc_obj {
        if let Object::Array(diffs) = doc.lookup(d, b"Differences") {
            let mut code = 0i32;
            for i in 0..diffs.len() {
                match doc.array_get(&diffs, i) {
                    Object::Int(c) => code = c,
                    Object::Name(n) => {
                        if (0..256).contains(&code) {
                            enc[code as usize] = Some(n.as_ref().clone());
                        }
                        code = code.wrapping_add(1);
                    }
                    _ => {}
                }
            }
        }
    }
    FontEnc { is_cid: false, names: enc }
}

/// `FoFiType1::undoPFB`.
fn undo_pfb(file: &[u8]) -> Vec<u8> {
    if file.first() != Some(&0x80) {
        return file.to_vec();
    }
    let mut out = Vec::with_capacity(file.len());
    let mut pos = 0usize;
    while file.get(pos) == Some(&0x80) {
        let Some(&ty) = file.get(pos + 1) else {
            break;
        };
        if !(1..=2).contains(&ty) || pos + 6 > file.len() {
            break;
        }
        let n = u32::from_le_bytes([file[pos + 2], file[pos + 3], file[pos + 4], file[pos + 5]]) as usize;
        pos += 6;
        if n > file.len() - pos {
            break;
        }
        out.extend_from_slice(&file[pos..pos + n]);
        pos += n;
    }
    out
}

fn next_line(f: &[u8], mut line: usize) -> Option<usize> {
    while line < f.len() && f[line] != 0x0a && f[line] != 0x0d {
        line += 1;
    }
    if line < f.len() && f[line] == 0x0d {
        line += 1;
    }
    if line < f.len() && f[line] == 0x0a {
        line += 1;
    }
    if line >= f.len() { None } else { Some(line) }
}

fn starts_with_at(f: &[u8], p: usize, s: &[u8]) -> bool {
    f.len() >= p + s.len() && &f[p..p + s.len()] == s
}

/// A C string out of `strncpy(buf, line, n); buf[n] = 0`.
fn c_str(b: &[u8]) -> &[u8] {
    &b[..b.iter().position(|&c| c == 0).unwrap_or(b.len())]
}

/// `FoFiType1::parse`, the encoding only.
fn type1_encoding(file: &[u8]) -> Option<Enc> {
    let f = undo_pfb(file);
    let mut name = false;
    let mut got_matrix = false;
    let mut encoding: Option<Enc> = None;
    let mut line = Some(0usize);
    let mut i = 1;
    while i <= 100 && line.is_some() && (!name || encoding.is_none() || !got_matrix) {
        let l = line.unwrap();
        if !name && starts_with_at(&f, l, b"/FontName") {
            let n = 255.min(f.len() - l);
            let buf = c_str(&f[l..l + n]);
            if buf.len() > 9 {
                if let Some(p) = buf[9..].iter().position(|&c| c == b'/') {
                    let rest = &buf[9 + p + 1..];
                    if rest.iter().any(|&c| !matches!(c, b' ' | b'\t' | b'\n' | b'\r')) {
                        name = true;
                    }
                }
            }
            line = next_line(&f, l);
        } else if encoding.is_none() && starts_with_at(&f, l, b"/Encoding StandardEncoding def") {
            encoding = Some(table_enc(&FOFI_STANDARD));
        } else if encoding.is_none() && starts_with_at(&f, l, b"/Encoding 256 array") {
            let mut enc: Enc = vec![None; 256];
            let mut j = 0;
            let mut cur = next_line(&f, l);
            while j < 300 {
                let Some(ln) = cur else {
                    break;
                };
                let Some(ln1) = next_line(&f, ln) else {
                    cur = None;
                    break;
                };
                let n = (ln1 - ln).min(255);
                let buf = c_str(&f[ln..ln + n]);
                let mut p = 0usize;
                while p < buf.len() && (buf[p] == b' ' || buf[p] == b'\t') {
                    p += 1;
                }
                let ends_with_dup = (ln >= 4 && &f[ln - 4..ln] == b"dup\x0a")
                    || (ln >= 5 && &f[ln - 5..ln - 1] == b"dup\x0d");
                let starts_with_dup = buf[p..].starts_with(b"dup");
                if ends_with_dup || starts_with_dup {
                    if starts_with_dup {
                        p += 3;
                    }
                    loop {
                        while p < buf.len() && (buf[p] == b' ' || buf[p] == b'\t') {
                            p += 1;
                        }
                        let mut code: u32 = 0;
                        let base: u32;
                        let at = |q: usize| -> u8 { buf.get(q).copied().unwrap_or(0) };
                        if at(p) == b'8' && at(p + 1) == b'#' {
                            base = 8;
                            p += 2;
                        } else if at(p).is_ascii_digit() {
                            base = 10;
                        } else {
                            break;
                        }
                        while at(p) >= b'0' && (at(p) as u32) < b'0' as u32 + base {
                            code = code.wrapping_mul(base).wrapping_add((at(p) - b'0') as u32);
                            p += 1;
                        }
                        while at(p) == b' ' || at(p) == b'\t' {
                            p += 1;
                        }
                        if at(p) != b'/' {
                            break;
                        }
                        p += 1;
                        let mut p2 = p;
                        while at(p2) != 0 && at(p2) != b' ' && at(p2) != b'\t' {
                            p2 += 1;
                        }
                        if code < 256 {
                            enc[code as usize] = Some(buf[p.min(buf.len())..p2.min(buf.len())].to_vec());
                        }
                        p = p2;
                        while at(p) == b' ' || at(p) == b'\t' {
                            p += 1;
                        }
                        if !buf[p.min(buf.len())..].starts_with(b"put") {
                            break;
                        }
                        p += 3;
                        while at(p) == b' ' || at(p) == b'\t' {
                            p += 1;
                        }
                        if !buf[p.min(buf.len())..].starts_with(b"dup") {
                            break;
                        }
                        p += 3;
                    }
                } else {
                    // strtok(buf, " \t") && (p = strtok(NULL, " \t\n\r")) && !strcmp(p, "def")
                    let first_end = {
                        let s = buf.iter().position(|&c| c != b' ' && c != b'\t');
                        s.map(|s| s + buf[s..].iter().position(|&c| c == b' ' || c == b'\t').unwrap_or(buf.len() - s))
                    };
                    if let Some(e) = first_end {
                        if e < buf.len() {
                            let rest = &buf[e + 1..];
                            let is_d = |c: u8| matches!(c, b' ' | b'\t' | b'\n' | b'\r');
                            if let Some(s) = rest.iter().position(|&c| !is_d(c)) {
                                let t = &rest[s..];
                                let t = &t[..t.iter().position(|&c| is_d(c)).unwrap_or(t.len())];
                                if t == b"def" {
                                    cur = Some(ln);
                                    break;
                                }
                            }
                        }
                    }
                }
                j += 1;
                cur = Some(ln1);
            }
            encoding = Some(enc);
            line = cur;
        } else if !got_matrix && starts_with_at(&f, l, b"/FontMatrix") {
            got_matrix = true;
        } else {
            line = next_line(&f, l);
        }
        i += 1;
    }
    encoding
}

//------------------------------------------------------------------------
// FoFiType1C
//------------------------------------------------------------------------

struct Cff<'a> {
    file: &'a [u8],
    ok: bool,
}

#[derive(Clone, Copy, Default)]
struct CffIndex {
    pos: i32,
    len: i32,
    off_size: i32,
    start_pos: i32,
    end_pos: i32,
}

impl Cff<'_> {
    fn u8_at(&mut self, pos: i32) -> i32 {
        if pos < 0 || pos as usize >= self.file.len() {
            self.ok = false;
            return 0;
        }
        self.file[pos as usize] as i32
    }

    fn u16be(&mut self, pos: i32) -> i32 {
        if pos < 0 || pos > i32::MAX - 1 || pos as usize + 1 >= self.file.len() {
            self.ok = false;
            return 0;
        }
        ((self.file[pos as usize] as i32) << 8) | self.file[pos as usize + 1] as i32
    }

    fn uvar_be(&mut self, pos: i32, size: i32) -> u32 {
        if pos < 0 || !(1..=4).contains(&size) || pos as i64 + size as i64 > self.file.len() as i64 {
            self.ok = false;
            return 0;
        }
        let mut x = 0u32;
        for i in 0..size {
            x = (x << 8) | self.file[(pos + i) as usize] as u32;
        }
        x
    }

    fn get_index(&mut self, pos: i32) -> CffIndex {
        let mut idx = CffIndex { pos, len: self.u16be(pos), ..CffIndex::default() };
        if idx.len == 0 {
            idx.start_pos = pos + 2;
            idx.end_pos = pos + 2;
        } else {
            idx.off_size = self.u8_at(pos + 2);
            if !(1..=4).contains(&idx.off_size) {
                self.ok = false;
                return idx;
            }
            idx.start_pos = pos + 3 + (idx.len + 1) * idx.off_size - 1;
            if idx.start_pos < 0 || idx.start_pos as usize >= self.file.len() {
                self.ok = false;
            }
            idx.end_pos = idx.start_pos.wrapping_add(self.uvar_be(pos + 3 + idx.len * idx.off_size, idx.off_size) as i32);
            if idx.end_pos < idx.start_pos || idx.end_pos as usize > self.file.len() {
                self.ok = false;
            }
        }
        idx
    }

    fn get_index_val(&mut self, idx: &CffIndex, i: i32) -> (i32, i32) {
        if i < 0 || i >= idx.len {
            self.ok = false;
            return (0, 0);
        }
        let pos0 = idx.start_pos.wrapping_add(self.uvar_be(idx.pos + 3 + i * idx.off_size, idx.off_size) as i32);
        let pos1 = idx.start_pos.wrapping_add(self.uvar_be(idx.pos + 3 + (i + 1) * idx.off_size, idx.off_size) as i32);
        if pos0 < idx.start_pos || pos0 > idx.end_pos || pos1 <= idx.start_pos || pos1 > idx.end_pos || pos1 < pos0 {
            self.ok = false;
        }
        (pos0, pos1.wrapping_sub(pos0))
    }

    /// `getOp`: returns the new position, and the operator if it was one.  Operands go on `ops`, at most 49 as in
    /// the C, integers as themselves and reals truncated by `toInt`.
    fn get_op(&mut self, mut pos: i32, ops: &mut Vec<i32>) -> (i32, Option<i32>) {
        let b0 = self.u8_at(pos);
        pos += 1;
        let mut val = None;
        let mut oper = None;
        if b0 == 28 {
            let mut x = self.u8_at(pos);
            x = (x << 8) | self.u8_at(pos + 1);
            pos += 2;
            if x & 0x8000 != 0 {
                x |= !0xffff;
            }
            val = Some(x);
        } else if b0 == 29 {
            let mut x: u32 = 0;
            for k in 0..4 {
                x = (x << 8) | self.u8_at(pos + k) as u32;
            }
            pos += 4;
            val = Some(x as i32);
        } else if b0 == 30 {
            const NYB: &[u8; 16] = b"0123456789.ee -\0";
            let mut buf: Vec<u8> = Vec::new();
            'real: while buf.len() < 64 {
                let b1 = self.u8_at(pos);
                pos += 1;
                let (n0, n1) = ((b1 >> 4) as usize, (b1 & 15) as usize);
                if n0 == 0xf {
                    break;
                }
                buf.push(NYB[n0]);
                if buf.len() == 64 {
                    break;
                }
                if n0 == 0xc {
                    buf.push(b'-');
                }
                if buf.len() == 64 {
                    break;
                }
                if n1 == 0xf {
                    break 'real;
                }
                buf.push(NYB[n1]);
                if buf.len() == 64 {
                    break;
                }
                if n1 == 0xc {
                    buf.push(b'-');
                }
            }
            val = Some(crate::pdf::atof(&buf) as i32);
        } else if (32..=246).contains(&b0) {
            val = Some(b0 - 139);
        } else if (247..=250).contains(&b0) {
            val = Some(((b0 - 247) << 8) + self.u8_at(pos) + 108);
            pos += 1;
        } else if (251..=254).contains(&b0) {
            val = Some(-((b0 - 251) << 8) - self.u8_at(pos) - 108);
            pos += 1;
        } else if b0 == 12 {
            oper = Some(0x0c00 + self.u8_at(pos));
            pos += 1;
        } else {
            oper = Some(b0);
        }
        if let Some(v) = val {
            if ops.len() < 49 {
                ops.push(v);
            }
        }
        (pos, oper)
    }

    /// `getString`.
    fn get_string(&mut self, sid: i32, string_idx: &CffIndex) -> Vec<u8> {
        if sid < 0 {
            Vec::new()
        } else if sid < 391 {
            FOFI_STD_STRINGS[sid as usize].unwrap_or("").as_bytes().to_vec()
        } else {
            let (p, l) = self.get_index_val(string_idx, sid - 391);
            if self.ok {
                let n = l.min(255) as usize;
                let s = &self.file[p as usize..p as usize + n];
                c_str(s).to_vec()
            } else {
                Vec::new()
            }
        }
    }
}

/// `FoFiType1C::make` then `getEncoding`.  The outer `None` means `make` failed; the inner one, no encoding.
///
/// `readFD` and `readFDSelect` are left out: they belong to CID fonts, which `getFontType` has sent elsewhere.
fn type1c_encoding(file: &[u8]) -> Option<Option<Enc>> {
    // some tools embed Type 1C fonts with an extra whitespace char at the beginning
    let f = if !file.is_empty() && file[0] != 1 { &file[1..] } else { file };
    let mut c = Cff { file: f, ok: true };
    let hdr = c.u8_at(2);
    let name_idx = c.get_index(hdr);
    let top_idx = c.get_index(name_idx.end_pos);
    let string_idx = c.get_index(top_idx.end_pos);
    let _gsubr = c.get_index(string_idx.end_pos);
    if !c.ok {
        return None;
    }
    let _name = c.get_index_val(&name_idx, 0);
    if !c.ok {
        return None;
    }
    // readTopDict
    let (top_pos, top_len) = c.get_index_val(&top_idx, 0);
    let mut pos = top_pos;
    let mut ops: Vec<i32> = Vec::new();
    let (mut first_op, mut charset_offset, mut encoding_offset, mut charstrings_offset) = (-1, 0, 0, 0);
    let (mut private_size, mut private_offset) = (0, 0);
    while pos < top_pos.wrapping_add(top_len) {
        let (p, oper) = c.get_op(pos, &mut ops);
        pos = p;
        if !c.ok {
            break;
        }
        if let Some(op) = oper {
            if first_op < 0 {
                first_op = op;
            }
            let o0 = ops.first().copied().unwrap_or(0);
            match op {
                0x000f => charset_offset = o0,
                0x0010 => encoding_offset = o0,
                0x0011 => charstrings_offset = o0,
                0x0012 => {
                    private_size = o0;
                    private_offset = ops.get(1).copied().unwrap_or(0);
                }
                _ => {}
            }
            ops.clear();
        }
    }
    // readPrivateDict: only whether its operators can be read matters here.
    if first_op != 0x0c1e && private_offset != 0 && private_size != 0 {
        let mut pos = private_offset;
        let mut ops: Vec<i32> = Vec::new();
        while pos < private_offset.wrapping_add(private_size) {
            pos = c.get_op(pos, &mut ops).0;
            ops.clear();
            if !c.ok {
                break;
            }
        }
    }
    if !c.ok || charstrings_offset <= 0 {
        return None;
    }
    let cs_idx = c.get_index(charstrings_offset);
    if !c.ok {
        return None;
    }
    let mut n_glyphs = cs_idx.len;
    // readCharset
    let mut charset: Vec<u16>;
    if charset_offset == 0 {
        charset = CFF_ISO_ADOBE_CHARSET.to_vec();
        n_glyphs = n_glyphs.min(229);
    } else if charset_offset == 1 {
        charset = CFF_EXPERT_CHARSET.to_vec();
        n_glyphs = n_glyphs.min(166);
    } else if charset_offset == 2 {
        charset = CFF_EXPERT_SUBSET_CHARSET.to_vec();
        n_glyphs = n_glyphs.min(87);
    } else {
        charset = vec![0u16; n_glyphs.max(0) as usize];
        let mut pos = charset_offset;
        let format = c.u8_at(pos);
        pos += 1;
        if format == 0 {
            for i in 1..n_glyphs {
                charset[i as usize] = c.u16be(pos) as u16;
                pos += 2;
                if !c.ok {
                    break;
                }
            }
        } else if format == 1 || format == 2 {
            let mut i = 1;
            while i < n_glyphs {
                let mut sid = c.u16be(pos);
                pos += 2;
                let n_left = if format == 1 {
                    let v = c.u8_at(pos);
                    pos += 1;
                    v
                } else {
                    let v = c.u16be(pos);
                    pos += 2;
                    v
                };
                if !c.ok {
                    break;
                }
                let mut j = 0;
                while j <= n_left && i < n_glyphs {
                    charset[i as usize] = sid as u16;
                    sid += 1;
                    i += 1;
                    j += 1;
                }
            }
        }
        if !c.ok {
            return None;
        }
    }
    // for 8-bit fonts: build the encoding.  A CID font has none.
    if first_op == 0x0c14 || first_op == 0x0c1e {
        return Some(None);
    }
    let enc: Enc;
    if encoding_offset == 0 {
        enc = table_enc(&FOFI_STANDARD);
    } else if encoding_offset == 1 {
        enc = table_enc(&FOFI_EXPERT);
    } else {
        let mut e: Enc = vec![None; 256];
        let mut pos = encoding_offset;
        let format = c.u8_at(pos);
        pos += 1;
        if !c.ok {
            return None;
        }
        if format & 0x7f == 0 {
            let mut n_codes = 1 + c.u8_at(pos);
            pos += 1;
            if !c.ok {
                return None;
            }
            if n_codes > n_glyphs {
                n_codes = n_glyphs;
            }
            for i in 1..n_codes {
                let code = c.u8_at(pos);
                pos += 1;
                if !c.ok {
                    return None;
                }
                let sid = charset.get(i as usize).copied().unwrap_or(0) as i32;
                e[code as usize] = Some(c.get_string(sid, &string_idx));
            }
        } else if format & 0x7f == 1 {
            let n_ranges = c.u8_at(pos);
            pos += 1;
            if !c.ok {
                return None;
            }
            let mut n_codes = 1;
            for _ in 0..n_ranges {
                let mut code = c.u8_at(pos);
                let n_left = c.u8_at(pos + 1);
                pos += 2;
                if !c.ok {
                    return None;
                }
                let mut j = 0;
                while j <= n_left && n_codes < n_glyphs {
                    if code < 256 {
                        let sid = charset.get(n_codes as usize).copied().unwrap_or(0) as i32;
                        e[code as usize] = Some(c.get_string(sid, &string_idx));
                    }
                    n_codes += 1;
                    code += 1;
                    j += 1;
                }
            }
        }
        if format & 0x80 != 0 {
            let n_sups = c.u8_at(pos);
            pos += 1;
            if !c.ok {
                return None;
            }
            for _ in 0..n_sups {
                let code = c.u8_at(pos);
                pos += 1;
                if !c.ok {
                    return None;
                }
                let sid = c.u16be(pos);
                pos += 2;
                if !c.ok {
                    return None;
                }
                e[code as usize] = Some(c.get_string(sid, &string_idx));
            }
        }
        enc = e;
    }
    if !c.ok {
        return None;
    }
    charset.clear();
    Some(Some(enc))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn type1_encodings() {
        let f = b"%!PS-AdobeFont-1.0: X\n/FontName /X def\n/Encoding 256 array\n0 1 255 {1 index exch /.notdef put} for\ndup 65 /A put\ndup 8#102 /B put dup 67 /C put\nreadonly def\n";
        let e = type1_encoding(f).unwrap();
        assert_eq!(e[65].as_deref(), Some(&b"A"[..]));
        assert_eq!(e[66].as_deref(), Some(&b"B"[..]));
        assert_eq!(e[67].as_deref(), Some(&b"C"[..]));
        assert_eq!(e[68], None);
        let f = b"%!PS-AdobeFont-1.0: X\n/Encoding StandardEncoding def\n";
        let e = type1_encoding(f).unwrap();
        assert_eq!(e[65].as_deref(), Some(&b"A"[..]));
        assert!(type1_encoding(b"%!PS-AdobeFont-1.0: X\nnothing here\n").is_none());
    }

    #[test]
    fn pfb_segments_are_joined() {
        let mut f = vec![0x80, 1, 3, 0, 0, 0];
        f.extend_from_slice(b"abc");
        f.extend_from_slice(&[0x80, 2, 2, 0, 0, 0]);
        f.extend_from_slice(b"de");
        f.extend_from_slice(&[0x80, 3]);
        assert_eq!(undo_pfb(&f), b"abcde");
        assert_eq!(undo_pfb(b"plain"), b"plain");
    }

    #[test]
    fn identify_kinds() {
        assert_eq!(identify(b"%!PS-AdobeFont-1.0"), FoFiId::Type1Pfa);
        assert_eq!(identify(b"%!FontType1-1.0"), FoFiId::Type1Pfa);
        assert_eq!(identify(&[0, 1, 0, 0, 9]), FoFiId::TrueType);
        assert_eq!(identify(b"ttcf1234"), FoFiId::TrueTypeCollection);
        assert_eq!(identify(b"junkjunk"), FoFiId::Unknown);
    }
}
