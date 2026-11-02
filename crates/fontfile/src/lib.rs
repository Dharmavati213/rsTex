//! Type 1 and TrueType font programs, encoding files, map lines, subfonts and ToUnicode, for pdfTeX's
//! font output: `writet1.c`, `writettf.c`, `writeenc.c`, `mapfile.c`, `subfont.c`, `tounicode.c`, and
//! `make_subset_tag` from `utils.c`.  (DESIGN.md section 9.)
//!
//! A port, not a redesign: the bytes are pdfTeX's, including what is odd about them.  The crate holds
//! no state and never calls the engine.  Files arrive as `&[u8]`, what the C keeps in statics between
//! calls is a plain value the caller owns and passes in, and the output goes to a caller's `Vec<u8>`.
//!
//! # What replaces what
//!
//! | C | Here | Notes |
//! |---|---|---|
//! | `writet1` | [`write_t1`] | the caller opens the file; `t1_length1..3` come back in [`T1Output`] |
//! | `load_enc_file` (writet1.c), behind `get_fe_entry` | [`load_enc_file`] | the caller keeps the tree of encodings |
//! | `write_enc`, `epdf_write_enc` | [`write_enc_differences`], [`epdf_write_enc_differences`] | the body of `/Differences [..]` |
//! | `writettf` | [`write_ttf`] | `pdfsaveoffset = pdfoffset(); pdfflush();` is the caller's, before the call |
//! | `writeotf` | [`write_otf`] | |
//! | `make_subset_tag` (utils.c) | [`make_subset_tag`] | [`write_t1`] and [`write_ttf`] call it themselves |
//! | `fm_scan_line` | [`fm_read_line`] then [`fm_scan_line`] | a `\pdfmapline` goes straight to the second |
//! | `check_fm_entry` | [`check_fm_entry`], run by [`fm_scan_line`] | |
//! | `check_std_t1font` | [`check_std_t1font`] | |
//! | the top of `process_map_item` | [`process_map_item`] | |
//! | `read_sfd` | [`read_sfd`] | the caller keeps the tree of `.sfd` files |
//! | the test in `handle_subfont_fm` | [`subfont_parts`] | |
//! | `deftounicode` | [`GlyphUnicodeMap::deftounicode`] | |
//! | `write_tounicode`, `set_glyph_unicode` | [`write_tounicode`] | the stream body; objects are the caller's |
//! | `dumptounicode`, `undumptounicode` | [`GlyphUnicodeMap::parts`], [`GlyphUnicodeMap::from_parts`] | |
//! | `mark_reenc_glyphs`, `mark_chars`, `get_char_range`, `font_is_used` (writefont.c) | [`mark_reenc_glyphs`], [`mark_chars`], [`get_char_range`] | over [`CodeSet`], the layout of `pdf_char_used` |
//! | `fix_fontmetrics`, `write_fontmetrics` | [`fix_fontmetrics`], [`write_fontmetrics`] | |
//! | the `/Flags` choice in `write_fontdescriptor` | [`font_descriptor_flags`] | |
//!
//! `avl_do_entry` and its trees, `preset_fontmetrics` (it needs the TFM), the object numbers and the
//! rest of `writefont.c` stay in `crates/engine/src/pdffont.rs`, which owns their state.
//!
//! # How the engine calls this for a Type 1 font
//!
//! ```text
//! // per font, in create_fontdictionary:
//! get_char_range, mark_chars(fe's or fd's CodeSet), mark_reenc_glyphs(fd's GlyphSet)
//! // in write_fontfile, once per descriptor:
//! let o = write_t1(&T1Input { font, subsetted, all_glyphs, slant, extend, fontname, tx_codes, used_tags },
//!                  &mut fd.dims, &mut fd.glyphs, &mut carry, &mut fontbuf)?;
//! // print o.warnings with pdftex_warn, push o.subset_tag on used_tags, write /Length1..3 and fontbuf.
//! ```
//!
//! `Err` is `pdftex_fail` (or a failed `assert`): print [`FontError::warnings`], then
//! [`FontError::message`] its way, and stop, since the C does not come back.  Warnings have to be printed
//! by the caller between its `<name` and `>`.
//! Messages are truncated as `vsnprintf` into `print_buf` does.
//!
//! The printed progress (`<cmr10.pfb>`, `{8r.enc}`), `recorder_record_input`, kpathsea and the
//! `cur_file_name` in messages are all the caller's.
//!
//! # For whoever ports `writefont.c` on top of this
//!
//! Three things the replay in `examples/fontfile_gate.rs` had to get right against the oracle:
//!
//! - `dopdffont` runs over `head_tab[obj_type_font]`, last used font first.  So the map entry behind a
//!   shared Type 1 descriptor (its preset name, its flags) is that of the last font using it.
//! - `write_fontdictionary` tests `(fo->fd != NULL && ... && strcmp(tfm_name, "dummy-space")) == 0`, with
//!   the parenthesis there: the `dummy-space` font gets a `write_tounicode` call even with
//!   `\pdfgentounicode=0`, and so the warning "no GlyphToUnicode entry has been inserted yet!".
//! - `ttf_write_post` sets the WEB global `k`.
//!
//! # Deviations from the C
//!
//! Only where the C has undefined behaviour on a malformed font: an out-of-bounds index or a stack
//! overflow becomes an `Err`, or is ignored where nothing can observe it.  Each place says so in a
//! comment.  The C's own heap overflows, such as a rewritten `/FontMatrix` line longer than a font's
//! longest line so far, do not happen here: the oracle can crash there and this does not.  `atan` and the
//! `sscanf` conversions are pure Rust, so every platform gives the same bytes.
#![forbid(unsafe_code)]

mod cstr;
mod enc;
mod map;
mod md5;
mod subfont;
mod t1;
mod tounicode;
mod ttf;

pub use enc::{epdf_write_enc_differences, load_enc_file, write_enc_differences};
pub use map::{
    F_INCLUDED, F_OTF, F_PK, F_STDT1FONT, F_SUBFONT, F_SUBSETTED, F_TRUETYPE, F_TYPE1, FmEntry, FmMode, FmScan, MapItem,
    MapType, check_fm_entry, check_std_t1font, fm_read_line, fm_scan_line, process_map_item,
};
pub use md5::md5_hex;
pub use subfont::{Sfd, Subfont, read_sfd, subfont_parts};
pub use t1::{T1Carry, T1Input, T1Output, write_t1};
pub use tounicode::{GlyphUnicodeMap, GuEntry, write_tounicode};
pub use ttf::{TtfCmapCache, TtfInput, TtfOutput, write_otf, write_ttf};

pub const ASCENT_CODE: usize = 0;
pub const CAPHEIGHT_CODE: usize = 1;
pub const DESCENT_CODE: usize = 2;
pub const ITALIC_ANGLE_CODE: usize = 3;
pub const STEMV_CODE: usize = 4;
pub const XHEIGHT_CODE: usize = 5;
pub const FONTBBOX1_CODE: usize = 6;
pub const FONTBBOX2_CODE: usize = 7;
pub const FONTBBOX3_CODE: usize = 8;
pub const FONTBBOX4_CODE: usize = 9;
pub const FONTNAME_CODE: usize = 10;
pub const GEN_KEY_NUM: usize = XHEIGHT_CODE + 1;
pub const INT_KEYS_NUM: usize = FONTBBOX4_CODE + 1;
pub const FONT_KEYS_NUM: usize = FONTNAME_CODE + 1;

/// `font_key[i].pdfname`.
pub const PDF_NAMES: [&[u8]; FONT_KEYS_NUM] = [
    b"Ascent", b"CapHeight", b"Descent", b"ItalicAngle", b"StemV", b"XHeight", b"FontBBox", b"", b"", b"", b"FontName",
];

/// `font_key[i].t1name`.
pub(crate) const T1_NAMES: [&[u8]; FONT_KEYS_NUM] = [
    b"Ascender", b"CapHeight", b"Descender", b"ItalicAngle", b"StdVW", b"XHeight", b"FontBBox", b"", b"", b"", b"FontName",
];

/// `FD_FLAGS_NOT_SET_IN_MAPLINE`.
pub const FD_FLAGS_NOT_SET_IN_MAPLINE: i32 = -1;
/// `FD_FLAGS_DEFAULT_EMBED`: a symbol font.
pub const FD_FLAGS_DEFAULT_EMBED: i32 = 4;
/// `FD_FLAGS_DEFAULT_NON_EMBED`: a nonsymbolic serif font.
pub const FD_FLAGS_DEFAULT_NON_EMBED: i32 = 0x22;

/// `PRINTF_BUF_SIZE`: `pdftex_warn` and `pdftex_fail` format into a buffer of this size.
const PRINTF_BUF_SIZE: usize = 1024;

pub(crate) fn truncate_message(mut m: Vec<u8>) -> Vec<u8> {
    m.truncate(PRINTF_BUF_SIZE - 1);
    m
}

/// `pdftex_fail`, or a failed `assert`: the C does not come back from either.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FontError {
    /// The formatted message, without `!pdfTeX error: pdftex (file x): `.  For an assertion, what glibc
    /// prints after the program name and a colon.
    pub message: Vec<u8>,
    /// A failed `assert`, which aborts, rather than `pdftex_fail`, which exits.
    pub assertion: bool,
    /// The `pdftex_warn` messages before it, which the C has printed by then.
    pub warnings: Vec<Vec<u8>>,
}

impl FontError {
    pub(crate) fn new(msg: &[u8]) -> FontError {
        let n = msg.len().min(PRINTF_BUF_SIZE - 1);
        FontError { message: msg[..n].to_vec(), assertion: false, warnings: Vec::new() }
    }

    pub(crate) fn overflow(file: &str, line: u32) -> FontError {
        let m = format!("buffer overflow at file {}, line {}", file, line);
        FontError { message: m.into_bytes(), assertion: false, warnings: Vec::new() }
    }

    pub(crate) fn assertion(file: &str, line: u32, func: &str, expr: &str) -> FontError {
        let m = format!("{}:{}: {}: Assertion `{}' failed.", file, line, func, expr);
        FontError { message: m.into_bytes(), assertion: true, warnings: Vec::new() }
    }
}

impl core::fmt::Display for FontError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&String::from_utf8_lossy(&self.message))
    }
}

/// `intparm`.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct IntParm {
    pub val: i32,
    pub set: bool,
}

/// `fd->font_dim`.
pub type FontDims = [IntParm; FONT_KEYS_NUM];

/// All unset, as `new_fd_entry` leaves them.
pub fn new_font_dims() -> FontDims {
    [IntParm::default(); FONT_KEYS_NUM]
}

/// A set of character codes: the layout of `pdf_char_used[f]`, bit `c mod 8` of byte `c div 8`.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct CodeSet(pub [u8; 32]);

impl CodeSet {
    pub fn new() -> CodeSet {
        CodeSet([0; 32])
    }
    #[inline]
    pub fn contains(&self, c: usize) -> bool {
        self.0[c / 8] & (1 << (c % 8)) != 0
    }
    #[inline]
    pub fn insert(&mut self, c: usize) {
        self.0[c / 8] |= 1 << (c % 8);
    }
    pub fn is_empty(&self) -> bool {
        self.0 == [0; 32]
    }
}

/// `fd->gl_tree`: a set of glyph names in `strcmp` order, kept in one buffer.
#[derive(Clone, Debug, Default)]
pub struct GlyphSet {
    pool: Vec<u8>,
    items: Vec<(u32, u32)>,
}

impl GlyphSet {
    pub fn new() -> GlyphSet {
        GlyphSet { pool: Vec::new(), items: Vec::new() }
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// The `i`-th name, in order.
    #[inline]
    pub fn get(&self, i: usize) -> &[u8] {
        let (o, l) = self.items[i];
        &self.pool[o as usize..(o + l) as usize]
    }

    /// `avl_find`, as a position.
    pub fn position(&self, name: &[u8]) -> Option<usize> {
        self.search(name).ok()
    }

    pub fn contains(&self, name: &[u8]) -> bool {
        self.search(name).is_ok()
    }

    fn search(&self, name: &[u8]) -> Result<usize, usize> {
        let pool = &self.pool;
        self.items.binary_search_by(|&(o, l)| pool[o as usize..(o + l) as usize].cmp(name))
    }

    /// `avl_probe`.  True if the name was not there yet.
    pub fn insert(&mut self, name: &[u8]) -> bool {
        match self.search(name) {
            Ok(_) => false,
            Err(at) => {
                let o = self.pool.len() as u32;
                self.pool.extend_from_slice(name);
                self.items.insert(at, (o, name.len() as u32));
                true
            }
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = &[u8]> + '_ {
        (0..self.items.len()).map(move |i| self.get(i))
    }
}

/// A `char **glyph_names` of 256 entries, kept in one buffer.  An entry equal to the pointer `notdef`
/// is a separate state: [`GlyphNames::is_notdef`].  [`GlyphNames::name`] reads as `.notdef` there.
#[derive(Clone, Debug)]
pub struct GlyphNames {
    pool: Vec<u8>,
    spans: [(u32, u32); 256],
}

const NOTDEF_SPAN: (u32, u32) = (u32::MAX, 0);

impl Default for GlyphNames {
    fn default() -> GlyphNames {
        GlyphNames::new()
    }
}

impl GlyphNames {
    pub fn new() -> GlyphNames {
        GlyphNames { pool: Vec::new(), spans: [NOTDEF_SPAN; 256] }
    }

    #[inline]
    pub fn is_notdef(&self, code: usize) -> bool {
        self.spans[code].0 == u32::MAX
    }

    /// The name at `code`; `.notdef` for the pointer.
    #[inline]
    pub fn name(&self, code: usize) -> &[u8] {
        let (o, l) = self.spans[code];
        if o == u32::MAX { b".notdef" } else { &self.pool[o as usize..(o + l) as usize] }
    }

    /// `glyph_names[code] = xstrdup(name)`.
    pub fn set(&mut self, code: usize, name: &[u8]) {
        let o = self.pool.len() as u32;
        self.pool.extend_from_slice(name);
        self.spans[code] = (o, name.len() as u32);
    }

    /// `copy_glyph_names(glyph_names, a, b)`, in which `a == b` clears the entry.
    pub(crate) fn copy(&mut self, a: usize, b: usize) {
        self.spans[b] = NOTDEF_SPAN;
        if !self.is_notdef(a) {
            self.spans[b] = self.spans[a];
        }
    }
}

/// `make_subset_tag`.  `glyphs` is `fd->gl_tree`, `fontname` is `fd->fontname`, and `used` is `st_tree`:
/// the tags given out so far this run.  The caller adds the result to `used`.
pub fn make_subset_tag(glyphs: &GlyphSet, fontname: &[u8], used: &[[u8; 6]]) -> [u8; 6] {
    let mut j: i32 = 0;
    loop {
        let mut pms = md5::Md5::new();
        for glyph in glyphs.iter() {
            pms.update(glyph);
            pms.update(b" ");
        }
        pms.update(fontname);
        // md5_append(&pms, &j, sizeof(int)), to resolve collision.  The oracle is little-endian.
        pms.update(&j.to_le_bytes());
        let digest = pms.finish();
        let mut a = [0i32; 6];
        for d in digest.iter().take(13) {
            a[0] += *d as i32;
        }
        for i in 1..6 {
            a[i] = a[i - 1] - digest[i - 1] as i32 + digest[(i + 12) % 16] as i32;
        }
        let mut tag = [0u8; 6];
        for i in 0..6 {
            tag[i] = (a[i] % 26) as u8 + b'A';
        }
        j += 1;
        if !used.contains(&tag) {
            return tag;
        }
        // The C asserts j < 100; a run does not have that many collisions on one name.
    }
}

/// `get_char_range`: the first and last marked codes in `bc..=ec`, or `(1, 0)` if there are none.
/// `font_is_used` is `first <= last`.
pub fn get_char_range(marked: &CodeSet, bc: i32, ec: i32) -> (i32, i32) {
    let mut first = bc;
    while first <= ec && !marked.contains(first as usize) {
        first += 1;
    }
    let mut last = ec;
    while last >= bc && !marked.contains(last as usize) {
        last -= 1;
    }
    // (first_char > last_char) || !pdfcharmarked(f, first_char)
    if first > last || !marked.contains(first as usize) {
        return (1, 0);
    }
    (first, last)
}

/// `mark_chars`: adds the marked codes in `first..=last` to `tx`.
pub fn mark_chars(tx: &mut CodeSet, marked: &CodeSet, first: i32, last: i32) {
    let mut i = first;
    while i <= last {
        if marked.contains(i as usize) {
            tx.insert(i as usize);
        }
        i += 1;
    }
}

/// `mark_reenc_glyphs`: adds the names of the marked codes in `first..=last`.  The caller tests
/// `is_subsetted(fm)` as the C does.
pub fn mark_reenc_glyphs(names: &GlyphNames, marked: &CodeSet, first: i32, last: i32, gl: &mut GlyphSet) {
    let mut i = first;
    while i <= last {
        let c = i as usize;
        if marked.contains(c) && !names.is_notdef(c) {
            gl.insert(names.name(c));
        }
        i += 1;
    }
}

/// `fix_fontmetrics`.  Returns the warning, if the font has no bounding box.
pub fn fix_fontmetrics(p: &mut FontDims, ff_name: &[u8]) -> Option<Vec<u8>> {
    if !p[FONTBBOX1_CODE].set || !p[FONTBBOX2_CODE].set || !p[FONTBBOX3_CODE].set || !p[FONTBBOX4_CODE].set {
        let mut m = b"font `".to_vec();
        m.extend_from_slice(ff_name);
        m.extend_from_slice(b"' doesn't have a BoundingBox");
        return Some(truncate_message(m));
    }
    if !p[ASCENT_CODE].set {
        p[ASCENT_CODE].val = p[FONTBBOX4_CODE].val;
        p[ASCENT_CODE].set = true;
    }
    if !p[DESCENT_CODE].set {
        p[DESCENT_CODE].val = p[FONTBBOX2_CODE].val;
        p[DESCENT_CODE].set = true;
    }
    if !p[CAPHEIGHT_CODE].set {
        p[CAPHEIGHT_CODE].val = p[FONTBBOX4_CODE].val;
        p[CAPHEIGHT_CODE].set = true;
    }
    None
}

/// `write_fontmetrics`, after `fix_fontmetrics`: `/FontBBox` and the other keys, into `out`.
pub fn write_fontmetrics(d: &FontDims, out: &mut Vec<u8>) {
    if d[FONTBBOX1_CODE].set && d[FONTBBOX2_CODE].set && d[FONTBBOX3_CODE].set && d[FONTBBOX4_CODE].set {
        out.extend_from_slice(b"/FontBBox [");
        for (n, k) in [FONTBBOX1_CODE, FONTBBOX2_CODE, FONTBBOX3_CODE, FONTBBOX4_CODE].iter().enumerate() {
            if n > 0 {
                out.push(b' ');
            }
            cstr::push_i64(out, d[*k].val as i64);
        }
        out.extend_from_slice(b"]\n");
    }
    for i in 0..GEN_KEY_NUM {
        if d[i].set {
            out.push(b'/');
            out.extend_from_slice(PDF_NAMES[i]);
            out.push(b' ');
            cstr::push_i64(out, d[i].val as i64);
            out.push(b'\n');
        }
    }
}

/// The `/Flags` choice of `write_fontdescriptor`, with the warning it may give.
pub fn font_descriptor_flags(
    fm_fd_flags: i32,
    ff_found: bool,
    ps_name: Option<&[u8]>,
    tfm_name: &[u8],
    is_std_t1font: bool,
) -> (i32, Option<Vec<u8>>) {
    const STD_FLAGS: [i32; 14] = [
        1 + 2 + (1 << 5),                        // Courier
        1 + 2 + (1 << 5) + (1 << 18),            // Courier-Bold
        1 + 2 + (1 << 5) + (1 << 6),             // Courier-Oblique
        1 + 2 + (1 << 5) + (1 << 6) + (1 << 18), // Courier-BoldOblique
        1 << 5,                                  // Helvetica
        (1 << 5) + (1 << 18),                    // Helvetica-Bold
        (1 << 5) + (1 << 6),                     // Helvetica-Oblique
        (1 << 5) + (1 << 6) + (1 << 18),         // Helvetica-BoldOblique
        4,                                       // Symbol
        2 + (1 << 5),                            // Times-Roman
        2 + (1 << 5) + (1 << 18),                // Times-Bold
        2 + (1 << 5) + (1 << 6),                 // Times-Italic
        2 + (1 << 5) + (1 << 6) + (1 << 18),     // Times-BoldItalic
        4,                                       // ZapfDingbats
    ];
    if fm_fd_flags != FD_FLAGS_NOT_SET_IN_MAPLINE {
        return (fm_fd_flags, None);
    }
    if ff_found {
        return (FD_FLAGS_DEFAULT_EMBED, None);
    }
    let flags = if is_std_t1font {
        STD_FLAGS[check_std_t1font(ps_name.unwrap_or(b"")) as usize]
    } else {
        FD_FLAGS_DEFAULT_NON_EMBED
    };
    let mut m = b"No flags specified for non-embedded font `".to_vec();
    m.extend_from_slice(ps_name.unwrap_or(b"No name given"));
    m.extend_from_slice(b"' (");
    m.extend_from_slice(tfm_name);
    m.extend_from_slice(b") (I'm using ");
    cstr::push_i64(&mut m, flags as i64);
    m.extend_from_slice(b"): fix your map entry.");
    (flags, Some(truncate_message(m)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn glyph_set_is_sorted_and_unique() {
        let mut g = GlyphSet::new();
        assert!(g.insert(b"b"));
        assert!(g.insert(b"a"));
        assert!(g.insert(b"B"));
        assert!(!g.insert(b"a"));
        assert!(g.insert(b".notdef"));
        let v: Vec<&[u8]> = g.iter().collect();
        assert_eq!(v, vec![&b".notdef"[..], b"B", b"a", b"b"]);
        assert_eq!(g.position(b"a"), Some(2));
        assert!(!g.contains(b"c"));
    }

    #[test]
    fn subset_tag_resolves_collisions() {
        let mut g = GlyphSet::new();
        g.insert(b"A");
        let t0 = make_subset_tag(&g, b"Foo", &[]);
        let t1 = make_subset_tag(&g, b"Foo", &[t0]);
        assert_ne!(t0, t1);
        assert_eq!(t0, make_subset_tag(&g, b"Foo", &[]));
        assert!(t0.iter().all(|c| c.is_ascii_uppercase()));
    }

    #[test]
    fn char_range() {
        let mut m = CodeSet::new();
        assert_eq!(get_char_range(&m, 0, 255), (1, 0));
        m.insert(65);
        m.insert(70);
        assert_eq!(get_char_range(&m, 0, 255), (65, 70));
        assert_eq!(get_char_range(&m, 66, 69), (1, 0));
        let mut tx = CodeSet::new();
        mark_chars(&mut tx, &m, 65, 70);
        assert!(tx.contains(65) && tx.contains(70) && !tx.contains(66));
    }

    #[test]
    fn metrics_print_like_the_c() {
        let mut d = new_font_dims();
        for (k, v) in [(FONTBBOX1_CODE, -1), (FONTBBOX2_CODE, -2), (FONTBBOX3_CODE, 3), (FONTBBOX4_CODE, 4)] {
            d[k] = IntParm { val: v, set: true };
        }
        assert_eq!(fix_fontmetrics(&mut d, b"x"), None);
        assert_eq!(d[ASCENT_CODE].val, 4);
        assert_eq!(d[DESCENT_CODE].val, -2);
        let mut out = Vec::new();
        write_fontmetrics(&d, &mut out);
        assert_eq!(out, b"/FontBBox [-1 -2 3 4]\n/Ascent 4\n/CapHeight 4\n/Descent -2\n".to_vec());
        let mut e = new_font_dims();
        assert!(fix_fontmetrics(&mut e, b"x").is_some());
    }

    #[test]
    fn flags() {
        assert_eq!(font_descriptor_flags(34, false, None, b"x", false).0, 34);
        assert_eq!(font_descriptor_flags(-1, true, None, b"x", false).0, 4);
        let (f, w) = font_descriptor_flags(-1, false, Some(b"Times-Roman"), b"x", true);
        assert_eq!(f, 34);
        assert_eq!(
            w.unwrap(),
            b"No flags specified for non-embedded font `Times-Roman' (x) (I'm using 34): fix your map entry.".to_vec()
        );
    }
}
