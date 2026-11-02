//! `writettf.c`: TrueType (whole or subsetted) and OpenType (whole, its CFF table).
//!
//! The file is a literal port with the same order of reads and writes.  The C reads by `getc` and
//! `fseek`; here a position over the bytes.  `fb_array` is `out`, from where it stood on entry, with
//! a write position, since the table directory is filled in last.
//!
//! The C does `pdfsaveoffset = pdfoffset(); pdfflush();` after reading the font, in `writettf`.  That
//! touches the engine's buffers, so the caller does it before `write_ttf`; nothing here depends on it.
//!
//! `ttf_write_post` assigns a variable `k` that is declared nowhere in writettf.c: it is the WEB global
//! `k`, from pdftexd.h.  So the C scribbles over that global when it writes `post` names.

use crate::cstr::*;
use crate::{CodeSet, FontDims, FontError, GlyphNames, GlyphSet, make_subset_tag};
use crate::{
    ASCENT_CODE, CAPHEIGHT_CODE, DESCENT_CODE, FONTBBOX1_CODE, FONTBBOX2_CODE, FONTBBOX3_CODE, FONTBBOX4_CODE,
    FONTNAME_CODE, ITALIC_ANGLE_CODE, XHEIGHT_CODE,
};

type R<T> = Result<T, FontError>;

const WRITETTF_C: &str = "../../../texk/web2c/pdftexdir/writettf.c";

const DEFAULT_NTABS: u16 = 14;
const NEW_CMAP_SIZE: usize = 2;
const NMACGLYPHS: usize = 258;
const TABDIR_OFF: i32 = 12;

const TTF_CHAR_SIZE: i32 = 1;
const TTF_BYTE_SIZE: i32 = 1;
const TTF_SHORT_SIZE: i32 = 2;
const TTF_USHORT_SIZE: i32 = 2;
const TTF_ULONG_SIZE: i32 = 4;
const TTF_FIXED_SIZE: i32 = 4;
const TTF_FWORD_SIZE: i32 = 2;
const TTF_UFWORD_SIZE: i32 = 2;
const TTF_F2DOT14_SIZE: i32 = 2;

const ARG_1_AND_2_ARE_WORDS: u16 = 1 << 0;
const WE_HAVE_A_SCALE: u16 = 1 << 3;
const MORE_COMPONENTS: u16 = 1 << 5;
const WE_HAVE_AN_X_AND_Y_SCALE: u16 = 1 << 6;
const WE_HAVE_A_TWO_BY_TWO: u16 = 1 << 7;
const WE_HAVE_INSTRUCTIONS: u16 = 1 << 8;

/// `mac_glyph_names`, macnames.c.
const MAC_GLYPH_NAMES: [&[u8]; NMACGLYPHS] = [
    b".notdef", b".null", b"CR", b"space", b"exclam", b"quotedbl", b"numbersign", b"dollar", b"percent",
    b"ampersand", b"quotesingle", b"parenleft", b"parenright", b"asterisk", b"plus", b"comma", b"hyphen",
    b"period", b"slash", b"zero", b"one", b"two", b"three", b"four", b"five", b"six", b"seven", b"eight",
    b"nine", b"colon", b"semicolon", b"less", b"equal", b"greater", b"question", b"at", b"A", b"B", b"C", b"D",
    b"E", b"F", b"G", b"H", b"I", b"J", b"K", b"L", b"M", b"N", b"O", b"P", b"Q", b"R", b"S", b"T", b"U", b"V",
    b"W", b"X", b"Y", b"Z", b"bracketleft", b"backslash", b"bracketright", b"asciicircum", b"underscore",
    b"grave", b"a", b"b", b"c", b"d", b"e", b"f", b"g", b"h", b"i", b"j", b"k", b"l", b"m", b"n", b"o", b"p",
    b"q", b"r", b"s", b"t", b"u", b"v", b"w", b"x", b"y", b"z", b"braceleft", b"bar", b"braceright",
    b"asciitilde", b"Adieresis", b"Aring", b"Ccedilla", b"Eacute", b"Ntilde", b"Odieresis", b"Udieresis",
    b"aacute", b"agrave", b"acircumflex", b"adieresis", b"atilde", b"aring", b"ccedilla", b"eacute", b"egrave",
    b"ecircumflex", b"edieresis", b"iacute", b"igrave", b"icircumflex", b"idieresis", b"ntilde", b"oacute",
    b"ograve", b"ocircumflex", b"odieresis", b"otilde", b"uacute", b"ugrave", b"ucircumflex", b"udieresis",
    b"dagger", b"degree", b"cent", b"sterling", b"section", b"bullet", b"paragraph", b"germandbls",
    b"registered", b"copyright", b"trademark", b"acute", b"dieresis", b"notequal", b"AE", b"Oslash",
    b"infinity", b"plusminus", b"lessequal", b"greaterequal", b"yen", b"mu", b"partialdiff", b"Sigma", b"Pi",
    b"pi", b"integral", b"ordfeminine", b"ordmasculine", b"Omega", b"ae", b"oslash", b"questiondown",
    b"exclamdown", b"logicalnot", b"radical", b"florin", b"approxequal", b"Delta", b"guillemotleft",
    b"guillemotright", b"ellipsis", b"nbspace", b"Agrave", b"Atilde", b"Otilde", b"OE", b"oe", b"endash",
    b"emdash", b"quotedblleft", b"quotedblright", b"quoteleft", b"quoteright", b"divide", b"lozenge",
    b"ydieresis", b"Ydieresis", b"fraction", b"currency", b"guilsinglleft", b"guilsinglright", b"fi", b"fl",
    b"daggerdbl", b"periodcentered", b"quotesinglbase", b"quotedblbase", b"perthousand", b"Acircumflex",
    b"Ecircumflex", b"Aacute", b"Edieresis", b"Egrave", b"Iacute", b"Icircumflex", b"Idieresis", b"Igrave",
    b"Oacute", b"Ocircumflex", b"applelogo", b"Ograve", b"Uacute", b"Ucircumflex", b"Ugrave", b"dotlessi",
    b"circumflex", b"tilde", b"macron", b"breve", b"dotaccent", b"ring", b"cedilla", b"hungarumlaut",
    b"ogonek", b"caron", b"Lslash", b"lslash", b"Scaron", b"scaron", b"Zcaron", b"zcaron", b"brokenbar",
    b"Eth", b"eth", b"Yacute", b"yacute", b"Thorn", b"thorn", b"minus", b"multiply", b"onesuperior",
    b"twosuperior", b"threesuperior", b"onehalf", b"onequarter", b"threequarters", b"franc", b"Gbreve",
    b"gbreve", b"Idot", b"Scedilla", b"scedilla", b"Cacute", b"cacute", b"Ccaron", b"ccaron", b"dmacron",
];

const AMBIGUOUS_NAMES: [&[u8]; 7] =
    [b"Delta", b"Omega", b"Pi", b"Sigma", b"dmacron", b"macron", b"periodcentered"];

const NEWTABNAMES: [&[u8; 4]; DEFAULT_NTABS as usize] = [
    b"OS/2", b"PCLT", b"cmap", b"cvt ", b"fpgm", b"glyf", b"head", b"hhea", b"hmtx", b"loca", b"maxp", b"name",
    b"post", b"prep",
];

/// What `glyph_entry.name` points at.
#[derive(Copy, Clone)]
enum GName {
    Notdef,
    Mac(u16),
    Buf(u32),
}

#[derive(Copy, Clone)]
struct GlyphEntry {
    offset: i32,
    newoffset: i32,
    adv_width: u16,
    lsb: i16,
    name: GName,
    newindex: i16,
    name_index: u16,
}

#[derive(Copy, Clone)]
struct TtfEnc {
    name: Option<usize>, // the code in glyph_names; None is notdef
    code: i64,
    newindex: i64,
}

#[derive(Copy, Clone, Default)]
struct NameRecord {
    platform_id: u16,
    encoding_id: u16,
    language_id: u16,
    name_id: u16,
    length: u16,
    offset: u16,
    new_offset: u16,
    new_length: u16,
}

#[derive(Copy, Clone)]
struct DirTab {
    tag: [u8; 4],
    checksum: u32,
    offset: u32,
    length: u32,
}

#[derive(Copy, Clone, Default)]
struct CmapEntry {
    platform_id: u16,
    encoding_id: u16,
    offset: u32,
    format: u16,
}

struct CachedCmap {
    ttf_name: Vec<u8>,
    pid: i32,
    eid: i32,
    table: Box<[i32]>,
}

/// `ttf_cmap_tree`: format 4 cmaps already read, so that one file used by many subfonts reads and warns
/// once.  A plain value: pass the same one to each `write_ttf` of a run.
#[derive(Default)]
pub struct TtfCmapCache {
    entries: Vec<CachedCmap>,
}

/// What `writettf` reads from its `fd_entry` and `fm_entry`.
pub struct TtfInput<'a> {
    /// The whole file.
    pub font: &'a [u8],
    /// `cur_file_name` after opening: only whether it ends in `.ttc` matters.
    pub file_name: &'a [u8],
    /// `fm->ff_name`: the cmap cache's key.
    pub ff_name: &'a [u8],
    /// `fm->tfm_name`, for messages.
    pub tfm_name: &'a [u8],
    /// `is_subsetted(fm)`.
    pub subsetted: bool,
    /// `fd->fontname` on entry.
    pub fontname: &'a [u8],
    /// `fd->fe->glyph_names`, for a reencoded font.
    pub fe_names: Option<&'a GlyphNames>,
    /// `fm->subfont->charcodes`, for a subfont (`is_subfont(fm)`).
    pub subfont: Option<&'a [i64; 256]>,
    /// `fm->pid`, `fm->eid`.
    pub pid: i32,
    pub eid: i32,
    /// `fd->write_ttf_glyph_names`.
    pub write_ttf_glyph_names: bool,
    /// `st_tree` of `make_subset_tag`.
    pub used_tags: &'a [[u8; 6]],
}

/// What `writettf` leaves behind.
pub struct TtfOutput {
    /// `ttf_length`, for `/Length1`.
    pub length: i32,
    /// `fd->fontname`.
    pub fontname: Vec<u8>,
    /// `fd->subset_tag`; the caller adds it to `used_tags`.
    pub subset_tag: Option<[u8; 6]>,
    /// `pdftex_warn` messages, in order.
    pub warnings: Vec<Vec<u8>>,
}

struct Ttf<'a> {
    font: &'a [u8],
    pos: usize,
    dims: &'a mut FontDims,
    out: &'a mut Vec<u8>,
    base: usize,
    fb: usize,
    warnings: Vec<Vec<u8>>,
    fontname: Vec<u8>,

    ntabs: u16,
    upem: u16,
    post_format: u32,
    loca_format: i16,
    last_glyf_offset: u32,
    glyphs_count: u16,
    new_glyphs_count: u16,
    nhmtxs: u16,
    new_ntabs: u16,

    glyph_tab: Vec<GlyphEntry>,
    glyph_tab_null: bool,
    glyph_index: Vec<i64>,
    name_tab: Vec<NameRecord>,
    name_buf: Vec<u8>,
    dir_tab: Vec<DirTab>,
    glyph_name_buf: Vec<u8>,
    glyph_name_offs: Vec<u32>,
    checksum: u32,
    tab_length: u32,
    tmp_ulong: u32,
    check_sum_adjustment_offset: u32,
    ttfenc_tab: [TtfEnc; 256],
}

fn hex4(v: i64) -> Vec<u8> {
    let mut b = Vec::new();
    push_hex(&mut b, v as u64, 4);
    b
}

fn dec(v: i64) -> Vec<u8> {
    let mut b = Vec::new();
    push_i64(&mut b, v);
    b
}

fn join(parts: &[&[u8]]) -> Vec<u8> {
    let mut m = Vec::new();
    for p in parts {
        m.extend_from_slice(p);
    }
    m
}

impl Ttf<'_> {
    fn warn(&mut self, m: Vec<u8>) {
        self.warnings.push(crate::truncate_message(m));
    }

    // ---- bytes out, fb_putchar and friends ----

    #[inline]
    fn fb_putchar(&mut self, b: u8) {
        let i = self.base + self.fb;
        if i < self.out.len() {
            self.out[i] = b;
        } else {
            if i > self.out.len() {
                self.out.resize(i, 0);
            }
            self.out.push(b);
        }
        self.fb += 1;
    }

    #[inline]
    fn ttf_offset(&self) -> i64 {
        self.fb as i64
    }

    #[inline]
    fn ttf_addchksm(&mut self, b: u8) -> u8 {
        self.tmp_ulong = (self.tmp_ulong << 8).wrapping_add(b as u32);
        self.tab_length = self.tab_length.wrapping_add(1);
        if self.tab_length % 4 == 0 {
            self.checksum = self.checksum.wrapping_add(self.tmp_ulong);
            self.tmp_ulong = 0;
        }
        b
    }

    fn ttf_getchksm(&mut self) -> u32 {
        while self.tab_length % 4 != 0 {
            let b = self.ttf_addchksm(0); // ttf_addchksm updates tab_length
            self.fb_putchar(b);
        }
        self.checksum
    }

    /// `ttf_putnum`: the low `s` bytes of `n`, big-endian.
    #[inline]
    fn ttf_putnum(&mut self, s: i32, n: i64) {
        let mut k = s;
        while k > 0 {
            k -= 1;
            let b = (n >> (8 * k)) as u8;
            let b = self.ttf_addchksm(b);
            self.fb_putchar(b);
        }
    }

    #[inline]
    fn put_char(&mut self, n: i64) {
        self.ttf_putnum(TTF_CHAR_SIZE, n);
    }
    #[inline]
    fn put_byte(&mut self, n: i64) {
        self.ttf_putnum(TTF_BYTE_SIZE, n);
    }
    #[inline]
    fn put_short(&mut self, n: i64) {
        self.ttf_putnum(TTF_SHORT_SIZE, n);
    }
    #[inline]
    fn put_ushort(&mut self, n: i64) {
        self.ttf_putnum(TTF_USHORT_SIZE, n);
    }
    #[inline]
    fn put_ulong(&mut self, n: i64) {
        self.ttf_putnum(TTF_ULONG_SIZE, n);
    }
    #[inline]
    fn put_fixed(&mut self, n: i64) {
        self.ttf_putnum(TTF_FIXED_SIZE, n);
    }
    #[inline]
    fn put_ufword(&mut self, n: i64) {
        self.ttf_putnum(TTF_UFWORD_SIZE, n);
    }

    // ---- bytes in ----

    #[inline]
    fn ttf_getnum(&mut self, s: i32) -> R<i64> {
        let mut i: i64 = 0;
        let mut k = s;
        while k > 0 {
            let c = match self.font.get(self.pos) {
                Some(&c) => c,
                None => return Err(FontError::new(b"unexpected EOF")),
            };
            self.pos += 1;
            i = (i << 8).wrapping_add(c as i64);
            k -= 1;
        }
        Ok(i)
    }

    #[inline]
    fn ttf_skip(&mut self, n: i32) -> R<()> {
        self.ttf_getnum(n)?;
        Ok(())
    }
    #[inline]
    fn get_byte(&mut self) -> R<u8> {
        Ok(self.ttf_getnum(TTF_BYTE_SIZE)? as u8)
    }
    #[inline]
    fn get_char(&mut self) -> R<i8> {
        Ok(self.ttf_getnum(TTF_CHAR_SIZE)? as i8)
    }
    #[inline]
    fn get_ushort(&mut self) -> R<u16> {
        Ok(self.ttf_getnum(TTF_USHORT_SIZE)? as u16)
    }
    #[inline]
    fn get_short(&mut self) -> R<i16> {
        Ok(self.ttf_getnum(TTF_SHORT_SIZE)? as i16)
    }
    #[inline]
    fn get_ulong(&mut self) -> R<u32> {
        Ok(self.ttf_getnum(TTF_ULONG_SIZE)? as u32)
    }
    #[inline]
    fn get_fixed(&mut self) -> R<u32> {
        Ok(self.ttf_getnum(TTF_FIXED_SIZE)? as u32)
    }
    #[inline]
    fn get_fword(&mut self) -> R<i16> {
        Ok(self.ttf_getnum(TTF_FWORD_SIZE)? as i16)
    }
    #[inline]
    fn get_ufword(&mut self) -> R<u16> {
        Ok(self.ttf_getnum(TTF_UFWORD_SIZE)? as u16)
    }

    #[inline]
    fn copy_byte(&mut self) -> R<()> {
        let b = self.get_byte()?;
        self.put_byte(b as i64);
        Ok(())
    }
    fn copy_char(&mut self) -> R<()> {
        let b = self.get_char()?;
        self.put_char(b as i64);
        Ok(())
    }
    fn copy_ushort(&mut self) -> R<u16> {
        let v = self.get_ushort()?;
        self.put_ushort(v as i64);
        Ok(v)
    }
    fn copy_short(&mut self) -> R<i16> {
        let v = self.get_short()?;
        self.put_short(v as i64);
        Ok(v)
    }

    fn ttf_ncopy(&mut self, mut n: i32) -> R<()> {
        while n > 0 {
            n -= 1;
            self.copy_byte()?;
        }
        Ok(())
    }

    fn ttf_funit(&self, n: i64) -> R<i64> {
        let upem = self.upem as i64;
        if upem == 0 {
            // The C divides by zero: SIGFPE.
            return Err(FontError::new(b"unitsPerEm is zero"));
        }
        Ok(if n < 0 {
            -((-n / upem) * 1000 + ((-n % upem) * 1000) / upem)
        } else {
            (n / upem) * 1000 + ((n % upem) * 1000) / upem
        })
    }

    /// `ttf_name_lookup`.  `s` is compared over four bytes, as `strncmp` does.
    fn ttf_name_lookup(&self, s: &[u8; 4], required: bool) -> R<Option<usize>> {
        for (i, t) in self.dir_tab.iter().enumerate() {
            if &t.tag == s {
                return Ok(Some(i));
            }
        }
        if required {
            return Err(FontError::new(&join(&[b"can't find table `", s, b"'"])));
        }
        Ok(None)
    }

    fn ttf_seek_tab(&mut self, name: &[u8; 4], offset: i32) -> R<usize> {
        let tab = self.ttf_name_lookup(name, true)?.expect("required");
        self.pos = (self.dir_tab[tab].offset as i64 + offset as i64) as u32 as usize;
        Ok(tab)
    }

    fn ttf_copy_encoding(&mut self, inp: &TtfInput<'_>, gl: &mut GlyphSet, tx: &mut CodeSet) -> R<[u8; 6]> {
        if let Some(glyph_names) = inp.fe_names {
            for e in self.ttfenc_tab.iter_mut() {
                e.name = None;
            }
            // a workaround for a bug of AcroReader 4.0
            if glyph_names.name(97) == b"a" {
                tx.insert(97);
            }
            // take over collected characters from TeX, reencode them
            for q in 0..256 {
                if tx.contains(q) {
                    self.ttfenc_tab[q].name = if glyph_names.is_notdef(q) { None } else { Some(q) };
                }
            }
            Ok(make_subset_tag(gl, &self.fontname, inp.used_tags))
        } else if let Some(charcodes) = inp.subfont {
            for e in self.ttfenc_tab.iter_mut() {
                e.code = -1;
            }
            // take over collected characters from TeX
            for q in 0..256 {
                if !tx.contains(q) {
                    continue;
                }
                let code = charcodes[q];
                self.ttfenc_tab[q].code = code;
                if code == -1 {
                    let m = join(&[
                        b"character ",
                        &dec(q as i64),
                        b" in subfont ",
                        inp.tfm_name,
                        b" is not mapped to any charcode",
                    ]);
                    self.warn(m);
                } else {
                    if code >= 0x10000 {
                        return Err(FontError::assertion(WRITETTF_C, 302, "ttf_copy_encoding", "e->code < 0x10000"));
                    }
                    let mut name = b"/c".to_vec();
                    push_hex(&mut name, code as i32 as u32 as u64, 4);
                    gl.insert(&name);
                }
            }
            Ok(make_subset_tag(gl, &self.fontname, inp.used_tags))
        } else {
            Err(FontError::assertion(WRITETTF_C, 310, "ttf_copy_encoding", "0"))
        }
    }

    fn ttf_read_name(&mut self) -> R<()> {
        let tab = self.ttf_seek_tab(b"name", TTF_USHORT_SIZE)?;
        let name_record_num = self.get_ushort()? as usize;
        self.name_tab = vec![NameRecord::default(); name_record_num];
        let name_buf_size = self.dir_tab[tab].length as i64
            - (3 * TTF_USHORT_SIZE as i64 + name_record_num as i64 * 6 * TTF_USHORT_SIZE as i64);
        self.ttf_skip(TTF_USHORT_SIZE)?;
        for i in 0..name_record_num {
            let r = NameRecord {
                platform_id: self.get_ushort()?,
                encoding_id: self.get_ushort()?,
                language_id: self.get_ushort()?,
                name_id: self.get_ushort()?,
                length: self.get_ushort()?,
                offset: self.get_ushort()?,
                new_offset: 0,
                new_length: 0,
            };
            self.name_tab[i] = r;
        }
        let mut k = 0;
        self.name_buf.clear();
        while k < name_buf_size {
            let c = self.get_char()?;
            self.name_buf.push(c as u8);
            k += 1;
        }
        // look for PS font name
        for i in 0..name_record_num {
            let r = self.name_tab[i];
            if r.platform_id == 1 && r.encoding_id == 0 && r.name_id == 6 {
                self.fontname = self.strip_spaces_and_delims(r.offset as usize, r.length as usize)?;
                self.dims[FONTNAME_CODE].set = true;
                break;
            }
        }
        if !self.dims[FONTNAME_CODE].set {
            for i in 0..name_record_num {
                let r = self.name_tab[i];
                if r.platform_id == 3 && (r.encoding_id == 0 || r.encoding_id == 1) && r.name_id == 6 {
                    if r.length as usize >= 256 {
                        return Err(FontError::assertion(
                            WRITETTF_C,
                            378,
                            "ttf_read_name",
                            "name_tab[i].length < sizeof(buf)",
                        ));
                    }
                    let mut buf = Vec::with_capacity(128);
                    let mut j = 0;
                    while j < r.length as usize {
                        buf.push(at(&self.name_buf, r.offset as usize + j + 1));
                        j += 2;
                    }
                    let l = strlen(&buf, 0);
                    buf.truncate(l);
                    self.fontname = strip_delims(&buf)?;
                    self.dims[FONTNAME_CODE].set = true;
                    break;
                }
            }
        }
        Ok(())
    }

    fn strip_spaces_and_delims(&self, offset: usize, length: usize) -> R<Vec<u8>> {
        let mut src = Vec::with_capacity(length);
        for i in 0..length {
            src.push(at(&self.name_buf, offset + i));
        }
        strip_delims(&src)
    }

    fn ttf_read_mapx(&mut self) -> R<()> {
        self.ttf_seek_tab(b"maxp", TTF_FIXED_SIZE)?;
        self.glyphs_count = self.get_ushort()?;
        let e = GlyphEntry {
            offset: 0,
            newoffset: 0,
            adv_width: 0,
            lsb: 0,
            name: GName::Notdef,
            newindex: -1,
            name_index: 0,
        };
        self.glyph_tab = vec![e; 1 + self.glyphs_count as usize];
        self.glyph_tab_null = false;
        // The C has room for glyphs_count + 1 and writes [1] regardless.
        self.glyph_index = vec![0; (self.glyphs_count as usize + 1).max(2)];
        self.glyph_index[0] = 0; // index of ".notdef" glyph
        self.glyph_index[1] = 1; // index of ".null" glyph
        Ok(())
    }

    fn ttf_read_head(&mut self) -> R<()> {
        self.ttf_seek_tab(b"head", 2 * TTF_FIXED_SIZE + 2 * TTF_ULONG_SIZE + TTF_USHORT_SIZE)?;
        self.upem = self.get_ushort()?;
        self.ttf_skip(16)?;
        for k in [FONTBBOX1_CODE, FONTBBOX2_CODE, FONTBBOX3_CODE, FONTBBOX4_CODE] {
            let v = self.get_fword()? as i64;
            self.dims[k].val = self.ttf_funit(v)? as i32;
        }
        for k in [FONTBBOX1_CODE, FONTBBOX2_CODE, FONTBBOX3_CODE, FONTBBOX4_CODE] {
            self.dims[k].set = true;
        }
        self.ttf_skip(2 * TTF_USHORT_SIZE + TTF_SHORT_SIZE)?;
        self.loca_format = self.get_short()?;
        Ok(())
    }

    fn ttf_read_hhea(&mut self) -> R<()> {
        self.ttf_seek_tab(b"hhea", TTF_FIXED_SIZE)?;
        let v = self.get_fword()? as i64;
        self.dims[ASCENT_CODE].val = self.ttf_funit(v)? as i32;
        let v = self.get_fword()? as i64;
        self.dims[DESCENT_CODE].val = self.ttf_funit(v)? as i32;
        self.dims[ASCENT_CODE].set = true;
        self.dims[DESCENT_CODE].set = true;
        self.ttf_skip(TTF_FWORD_SIZE + TTF_UFWORD_SIZE + 3 * TTF_FWORD_SIZE + 8 * TTF_SHORT_SIZE)?;
        self.nhmtxs = self.get_ushort()?;
        Ok(())
    }

    fn ttf_read_pclt(&mut self) -> R<()> {
        if self.ttf_name_lookup(b"PCLT", false)?.is_none() {
            return Ok(());
        }
        self.ttf_seek_tab(b"PCLT", TTF_FIXED_SIZE + TTF_ULONG_SIZE + TTF_USHORT_SIZE)?;
        let v = self.get_ushort()? as i64;
        self.dims[XHEIGHT_CODE].val = self.ttf_funit(v)? as i32;
        self.ttf_skip(2 * TTF_USHORT_SIZE)?;
        let v = self.get_ushort()? as i64;
        self.dims[CAPHEIGHT_CODE].val = self.ttf_funit(v)? as i32;
        self.dims[XHEIGHT_CODE].set = true;
        self.dims[CAPHEIGHT_CODE].set = true;
        Ok(())
    }

    fn ttf_read_hmtx(&mut self) -> R<()> {
        self.ttf_seek_tab(b"hmtx", 0)?;
        let nhmtxs = self.nhmtxs as usize;
        let count = self.glyphs_count as usize;
        let mut g = 0;
        while g < nhmtxs {
            let adv = self.get_ufword()?;
            let lsb = self.get_ufword()? as i16;
            // The C writes past the table if nhmtxs > glyphs_count + 1; those entries are never read.
            if g < self.glyph_tab.len() {
                self.glyph_tab[g].adv_width = adv;
                self.glyph_tab[g].lsb = lsb;
            }
            g += 1;
        }
        if nhmtxs < count {
            let last_adv = if g > 0 { self.glyph_tab[g - 1].adv_width } else { 0 };
            while g < count {
                self.glyph_tab[g].adv_width = last_adv;
                self.glyph_tab[g].lsb = self.get_ufword()? as i16;
                g += 1;
            }
        }
        Ok(())
    }

    fn ttf_read_post(&mut self) -> R<()> {
        let tab = self.ttf_seek_tab(b"post", 0)?;
        self.post_format = self.get_fixed()?;
        let italic_angle = self.get_fixed()?;
        let mut int_part = (italic_angle >> 16) as i64;
        let mut sign = 1i64;
        if int_part > 0x7FFF {
            // a negative number
            int_part = 0x10000 - int_part;
            sign = -1;
        }
        let frac_part = (italic_angle % 0x10000) as i64;
        self.dims[ITALIC_ANGLE_CODE].val =
            ((sign as f64) * (int_part as f64 + frac_part as f64 * 1.0 / 65536.0)) as i32;
        self.dims[ITALIC_ANGLE_CODE].set = true;
        if self.glyph_tab_null {
            return Ok(()); // being called from writeotf()
        }
        self.ttf_skip(2 * TTF_FWORD_SIZE + 5 * TTF_ULONG_SIZE)?;
        match self.post_format {
            0x10000 => {
                // The C goes to NMACGLYPHS regardless of the size of glyph_tab.
                let n = NMACGLYPHS.min(self.glyph_tab.len());
                for g in 0..n {
                    self.glyph_tab[g].name = if g == 0 { GName::Notdef } else { GName::Mac(g as u16) };
                    self.glyph_tab[g].name_index = g as u16;
                }
            }
            0x20000 => {
                let nnames = self.get_ushort()? as usize; // some fonts have this different from nglyphs
                for g in 0..nnames {
                    let v = self.get_ushort()?;
                    if g < self.glyph_tab.len() {
                        self.glyph_tab[g].name_index = v;
                    }
                }
                let length = self.dir_tab[tab].length as i64 - (self.pos as i64 - self.dir_tab[tab].offset as i64);
                self.glyph_name_buf.clear();
                while (self.glyph_name_buf.len() as i64) < length {
                    let mut k = self.get_byte()? as i32;
                    while k > 0 {
                        let c = self.get_char()?;
                        self.glyph_name_buf.push(c as u8);
                        k -= 1;
                    }
                    self.glyph_name_buf.push(0);
                }
                // strend(p) + 1, k times
                self.glyph_name_offs.clear();
                self.glyph_name_offs.push(0);
                for i in 0..self.glyph_name_buf.len() {
                    if self.glyph_name_buf[i] == 0 {
                        self.glyph_name_offs.push(i as u32 + 1);
                    }
                }
                let n = nnames.min(self.glyph_tab.len());
                for g in 0..n {
                    let ni = self.glyph_tab[g].name_index as usize;
                    if ni < NMACGLYPHS {
                        self.glyph_tab[g].name = if ni == 0 { GName::Notdef } else { GName::Mac(ni as u16) };
                    } else {
                        let k = ni - NMACGLYPHS;
                        // Past the last name the C walks off the buffer; an empty name matches nothing.
                        let off = match self.glyph_name_offs.get(k) {
                            Some(&o) => o,
                            None => self.glyph_name_buf.len() as u32,
                        };
                        self.glyph_tab[g].name = GName::Buf(off);
                    }
                }
            }
            f => {
                if f != 0x00030000 {
                    let mut m = b"unsupported format (".to_vec();
                    push_hex(&mut m, f as u64, 8);
                    m.extend_from_slice(b") of `post' table, assuming 3.0");
                    self.warn(m);
                }
                let n = NMACGLYPHS.min(self.glyph_tab.len());
                for g in 0..n {
                    self.glyph_tab[g].name_index = g as u16;
                }
            }
        }
        Ok(())
    }

    fn glyph_name(&self, n: GName) -> &[u8] {
        match n {
            GName::Notdef => b".notdef",
            GName::Mac(i) => MAC_GLYPH_NAMES[i as usize],
            GName::Buf(o) => cstr(&self.glyph_name_buf, o as usize),
        }
    }

    fn ttf_read_loca(&mut self) -> R<()> {
        self.ttf_seek_tab(b"loca", 0)?;
        let n = self.glyphs_count as usize + 1;
        if self.loca_format != 0 {
            for g in 0..n {
                self.glyph_tab[g].offset = self.get_ulong()? as i32;
            }
        } else {
            for g in 0..n {
                self.glyph_tab[g].offset = (self.get_ushort()? as i32) << 1;
            }
        }
        Ok(())
    }

    fn ttf_read_tabdir(&mut self) -> R<()> {
        self.ttf_skip(TTF_FIXED_SIZE)?; // ignore the sfnt number
        self.ntabs = self.get_ushort()?;
        self.ttf_skip(3 * TTF_USHORT_SIZE)?;
        self.dir_tab.clear();
        for _ in 0..self.ntabs {
            let mut tag = [0u8; 4];
            for t in tag.iter_mut() {
                *t = self.get_char()? as u8;
            }
            let checksum = self.get_ulong()?;
            let offset = self.get_ulong()?;
            let length = self.get_ulong()?;
            self.dir_tab.push(DirTab { tag, checksum, offset, length });
        }
        Ok(())
    }

    /// `ttf_read_cmap`: the index in the cache, or None.
    fn ttf_read_cmap(
        &mut self,
        cache: &mut TtfCmapCache,
        ttf_name: &[u8],
        pid: i32,
        eid: i32,
        warn: bool,
    ) -> R<Option<usize>> {
        // look up in ttf_cmap_tree first, return if found
        for (i, e) in cache.entries.iter().enumerate() {
            if e.ttf_name == ttf_name && e.pid as u16 == pid as u16 && e.eid as u16 == eid as u16 {
                return Ok(Some(i));
            }
        }
        // not found, have to read it
        self.ttf_seek_tab(b"cmap", TTF_USHORT_SIZE)?; // skip the table version number (=0)
        let ncmapsubtabs = self.get_ushort()?;
        let cmap_offset = self.pos as i64 - 2 * TTF_USHORT_SIZE as i64;
        let mut found = false;
        for _ in 0..ncmapsubtabs {
            let tmp_pid = self.get_ushort()?;
            let tmp_eid = self.get_ushort()?;
            let tmp_offset = self.get_ulong()?;
            if tmp_pid as i32 == pid && tmp_eid as i32 == eid {
                self.pos = (cmap_offset as u32).wrapping_add(tmp_offset) as usize;
                let format = self.get_ushort()?;
                if format == 4 {
                    found = true;
                    break;
                }
                if warn {
                    self.warn(join(&[b"cmap format ", &dec(format as i64), b" unsupported"]));
                }
                return Ok(None);
            }
        }
        if !found {
            if warn {
                self.warn(join(&[
                    b"cannot find cmap subtable for (pid,eid) = (",
                    &dec(pid as i64),
                    b", ",
                    &dec(eid as i64),
                    b")",
                ]));
            }
            return Ok(None);
        }
        // read_cmap_format_4:
        let mut table: Vec<i32> = vec![-1; 0x10000]; // unassigned yet

        // read the subtable
        let mut length = self.get_ushort()? as i64; // length of subtable
        self.get_ushort()?; // skip the version number
        let seg_count = (self.get_ushort()? / 2) as usize;
        self.get_ushort()?; // skip searchRange
        self.get_ushort()?; // skip entrySelector
        self.get_ushort()?; // skip rangeShift
        let mut end_code = vec![0u16; seg_count];
        let mut start_code = vec![0u16; seg_count];
        let mut id_delta = vec![0u16; seg_count];
        let mut id_range_offset = vec![0u16; seg_count];
        for v in end_code.iter_mut() {
            *v = self.get_ushort()?;
        }
        self.get_ushort()?; // skip reversedPad
        for v in start_code.iter_mut() {
            *v = self.get_ushort()?;
        }
        for v in id_delta.iter_mut() {
            *v = self.get_ushort()?;
        }
        for v in id_range_offset.iter_mut() {
            *v = self.get_ushort()?;
        }
        length -= 8 * TTF_USHORT_SIZE as i64 + 4 * seg_count as i64 * TTF_USHORT_SIZE as i64;
        let n = length / TTF_USHORT_SIZE as i64; // number of glyphID's
        let mut glyph_id: Vec<u16> = Vec::with_capacity(n.max(0) as usize);
        for _ in 0..n {
            glyph_id.push(self.get_ushort()?);
        }
        for s in 0..seg_count {
            let mut i = start_code[s] as i64;
            while i <= end_code[s] as i64 {
                if i == 0xFFFF {
                    break;
                }
                if id_range_offset[s] != 0xFFFF {
                    let index: i64;
                    if id_range_offset[s] == 0 {
                        index = (id_delta[s] as i64 + i) & 0xFFFF;
                    } else {
                        let k = (i - start_code[s] as i64) + id_range_offset[s] as i64 / 2 + s as i64 - seg_count as i64;
                        if !(k >= 0 && k < n) {
                            return Err(FontError::assertion(WRITETTF_C, 648, "ttf_read_cmap", "k >= 0 && k < n"));
                        }
                        let mut x = glyph_id[k as usize] as i64;
                        if x != 0 {
                            x = (x + id_delta[s] as i64) & 0xFFFF;
                        }
                        index = x;
                    }
                    if index >= self.glyphs_count as i64 {
                        return Err(FontError::new(&join(&[
                            b"cmap: glyph index ",
                            &dec(index),
                            b" out of range [0..",
                            &dec(self.glyphs_count as i64),
                            b")",
                        ])));
                    }
                    if table[i as usize] != -1 {
                        self.warn(join(&[
                            b"cmap: multiple glyphs are mapped to unicode ",
                            &hex4(i),
                            b", only ",
                            &dec(table[i as usize] as i64),
                            b" will be used (glyph ",
                            &dec(index),
                            b" being ignored)",
                        ]));
                    } else {
                        table[i as usize] = index as i32;
                    }
                }
                i += 1;
            }
        }
        cache.entries.push(CachedCmap {
            ttf_name: ttf_name.to_vec(),
            pid,
            eid,
            table: table.into_boxed_slice(),
        });
        Ok(Some(cache.entries.len() - 1))
    }

    fn ttf_read_font(&mut self) -> R<()> {
        self.ttf_read_tabdir()?;
        if self.ttf_name_lookup(b"PCLT", false)?.is_none() {
            self.new_ntabs -= 1;
        }
        if self.ttf_name_lookup(b"fpgm", false)?.is_none() {
            self.new_ntabs -= 1;
        }
        if self.ttf_name_lookup(b"cvt ", false)?.is_none() {
            self.new_ntabs -= 1;
        }
        if self.ttf_name_lookup(b"prep", false)?.is_none() {
            self.new_ntabs -= 1;
        }
        self.ttf_read_mapx()?;
        self.ttf_read_head()?;
        self.ttf_read_hhea()?;
        self.ttf_read_pclt()?;
        self.ttf_read_hmtx()?;
        self.ttf_read_post()?;
        self.ttf_read_loca()?;
        self.ttf_read_name()
    }

    fn ttf_reset_chksm(&mut self, tab: usize) {
        self.checksum = 0;
        self.tab_length = 0;
        self.tmp_ulong = 0;
        self.dir_tab[tab].offset = self.ttf_offset() as u32;
        if self.dir_tab[tab].offset % 4 != 0 {
            let tag = self.dir_tab[tab].tag;
            self.warn(join(&[b"offset of `", &tag, b"' is not a multiple of 4"]));
        }
    }

    fn ttf_set_chksm(&mut self, tab: usize) {
        self.dir_tab[tab].length = (self.ttf_offset() - self.dir_tab[tab].offset as i64) as u32;
        self.dir_tab[tab].checksum = self.ttf_getchksm();
    }

    fn ttf_copytab(&mut self, name: &[u8; 4]) -> R<()> {
        let tab = self.ttf_seek_tab(name, 0)?;
        let mut i = self.dir_tab[tab].length as i64;
        self.ttf_reset_chksm(tab);
        while i > 0 {
            self.copy_char()?;
            i -= 1;
        }
        self.ttf_set_chksm(tab);
        Ok(())
    }

    fn ttf_byte_encoding(&mut self, names: Option<&GlyphNames>) {
        self.put_ushort(0); // format number (0: byte encoding table)
        self.put_ushort(BYTE_ENCODING_LENGTH); // length of table
        self.put_ushort(0); // version number
        for c in 0..256 {
            let e = self.ttfenc_tab[c];
            if e.newindex < 256 {
                self.put_byte(e.newindex);
            } else {
                if let (Some(q), Some(n)) = (e.name, names) {
                    let m = join(&[
                        b"glyph `",
                        n.name(q),
                        b"' has been mapped to `.notdef' in `ttf_byte_encoding' cmap table",
                    ]);
                    self.warn(m);
                }
                self.put_byte(0); // notdef
            }
        }
    }

    fn ttf_trimmed_table_map(&mut self) {
        self.put_ushort(6); // format number (6): trimmed table mapping
        self.put_ushort(TRIMMED_TABLE_MAP_LENGTH);
        self.put_ushort(0); // version number (0)
        self.put_ushort(0); // first character code
        self.put_ushort(256); // number of character code in table
        for c in 0..256 {
            let v = self.ttfenc_tab[c].newindex;
            self.put_ushort(v);
        }
    }

    fn ttf_seg_map_delta(&mut self) {
        self.put_ushort(4); // format number (4: segment mapping to delta values)
        self.put_ushort(SEG_MAP_DELTA_LENGTH);
        self.put_ushort(0); // version number
        self.put_ushort(4); // 2*segCount
        self.put_ushort(4); // searchRange
        self.put_ushort(1); // entrySelector
        self.put_ushort(0); // rangeShift
        self.put_ushort(0xF0FF); // endCount[0]
        self.put_ushort(0xFFFF); // endCount[1]
        self.put_ushort(0); // reversedPad
        self.put_ushort(0xF000); // startCount[0]
        self.put_ushort(0xFFFF); // startCount[1]
        self.put_ushort(0); // idDelta[0]
        self.put_ushort(1); // idDelta[1]
        self.put_ushort(2 * TTF_USHORT_SIZE as i64); // idRangeOffset[0]
        self.put_ushort(0); // idRangeOffset[1]
        for c in 0..256 {
            let v = self.ttfenc_tab[c].newindex;
            self.put_ushort(v);
        }
    }

    fn ttf_write_cmap(&mut self, names: Option<&GlyphNames>) -> R<()> {
        let tab = self.ttf_name_lookup(b"cmap", true)?.expect("required");
        // ttf_select_cmap
        let mut new_cmap_tab = [CmapEntry::default(); NEW_CMAP_SIZE];
        new_cmap_tab[0].platform_id = 1; // Macintosh
        new_cmap_tab[0].encoding_id = 0; // Symbol; ignore code page
        new_cmap_tab[0].format = if self.new_glyphs_count < 256 { 0 } else { 6 }; // byte encoding, trimmed table
        new_cmap_tab[1].platform_id = 3; // Microsoft
        new_cmap_tab[1].encoding_id = 0; // Symbol; ignore code page
        new_cmap_tab[1].format = 4; // segment mapping to delta
        self.ttf_reset_chksm(tab);
        self.put_ushort(0); // table version number (0)
        self.put_ushort(NEW_CMAP_SIZE as i64); // number of encoding tables
        let mut offset = 2 * TTF_USHORT_SIZE as i64 + NEW_CMAP_SIZE as i64 * CMAP_ENTRY_LENGTH;
        for ce in new_cmap_tab.iter_mut() {
            ce.offset = offset as u32;
            match ce.format {
                0 => offset += BYTE_ENCODING_LENGTH,
                4 => offset += SEG_MAP_DELTA_LENGTH,
                _ => offset += TRIMMED_TABLE_MAP_LENGTH,
            }
            self.put_ushort(ce.platform_id as i64);
            self.put_ushort(ce.encoding_id as i64);
            self.put_ulong(ce.offset as i64);
        }
        for ce in new_cmap_tab.iter() {
            match ce.format {
                0 => self.ttf_byte_encoding(names),
                4 => self.ttf_seg_map_delta(),
                _ => self.ttf_trimmed_table_map(),
            }
        }
        self.ttf_set_chksm(tab);
        Ok(())
    }

    /// `prepend_subset_tags`.
    fn prepend_subset_tags(&self, index: usize, tag: &[u8; 6], out: &mut Vec<u8>) -> u16 {
        if self.name_tab[index].platform_id == 3 {
            for &t in tag {
                out.push(0);
                out.push(t);
            }
            out.push(0);
            out.push(b'+');
            14
        } else {
            out.extend_from_slice(tag);
            out.push(b'+');
            7
        }
    }

    fn ttf_write_name(&mut self, tag: [u8; 6]) -> R<()> {
        // Only subsetted fonts get here: ttf_copy_font writes the name table with ttf_copytab, so the
        // C's other branch, on new_name_buf == name_buf, is never taken.
        let tab = self.ttf_name_lookup(b"name", true)?.expect("required");
        let name_record_num = self.name_tab.len();
        let mut new_name_buf: Vec<u8> = Vec::new();
        for i in 0..name_record_num {
            let n = self.name_tab[i];
            self.name_tab[i].new_offset = new_name_buf.len() as u16;
            let l: u16 = if (n.name_id == 1 || n.name_id == 3 || n.name_id == 4 || n.name_id == 6)
                && ((n.platform_id == 1 && n.encoding_id == 0)
                    || (n.platform_id == 3 && n.encoding_id == 0)
                    || (n.platform_id == 3 && n.encoding_id == 1))
            {
                self.prepend_subset_tags(i, &tag, &mut new_name_buf)
            } else {
                0
            };
            for k in 0..n.length as usize {
                new_name_buf.push(at(&self.name_buf, n.offset as usize + k));
            }
            self.name_tab[i].new_length = n.length.wrapping_add(l);
        }
        self.ttf_reset_chksm(tab);
        self.put_ushort(0); // Format selector
        self.put_ushort(name_record_num as i64);
        self.put_ushort(3 * TTF_USHORT_SIZE as i64 + name_record_num as i64 * 6 * TTF_USHORT_SIZE as i64);
        for i in 0..name_record_num {
            let r = self.name_tab[i];
            self.put_ushort(r.platform_id as i64);
            self.put_ushort(r.encoding_id as i64);
            self.put_ushort(r.language_id as i64);
            self.put_ushort(r.name_id as i64);
            self.put_ushort(r.new_length as i64);
            self.put_ushort(r.new_offset as i64);
        }
        for &b in &new_name_buf {
            self.put_char(b as i64);
        }
        self.ttf_set_chksm(tab);
        Ok(())
    }

    fn ttf_write_dirtab(&mut self, subsetted: bool) -> R<()> {
        let save_offset = self.ttf_offset() as u32;
        self.fb = TABDIR_OFF as usize;
        if subsetted {
            for name in NEWTABNAMES {
                let tab = match self.ttf_name_lookup(name, false)? {
                    Some(t) => t,
                    None => continue,
                };
                let t = self.dir_tab[tab];
                for k in 0..4 {
                    self.put_char(t.tag[k] as i64);
                }
                self.put_ulong(t.checksum as i64);
                self.put_ulong(t.offset as i64);
                self.put_ulong(t.length as i64);
            }
        } else {
            for tab in 0..self.dir_tab.len() {
                let t = self.dir_tab[tab];
                for k in 0..4 {
                    self.put_char(t.tag[k] as i64);
                }
                self.put_ulong(t.checksum as i64);
                self.put_ulong(t.offset as i64);
                self.put_ulong(t.length as i64);
            }
        }
        // adjust checkSumAdjustment.  `*p` is a char, signed on the oracle: the addition sign-extends.
        self.tmp_ulong = 0;
        self.checksum = 0;
        let mut i: u32 = 0;
        while i < save_offset {
            let b = self.out[self.base + i as usize] as i8 as i32 as u32;
            self.tmp_ulong = (self.tmp_ulong << 8).wrapping_add(b);
            i += 1;
            if i % 4 == 0 {
                self.checksum = self.checksum.wrapping_add(self.tmp_ulong);
                self.tmp_ulong = 0;
            }
        }
        if i % 4 != 0 {
            let m = join(&[b"font length is not a multiple of 4 (", &dec(i as i64), b")"]);
            self.warn(m);
            self.checksum = self.checksum.wrapping_shl(8 * (4 - i % 4));
        }
        let k = 0xB1B0AFBAu32.wrapping_sub(self.checksum);
        self.fb = self.check_sum_adjustment_offset as usize;
        self.put_ulong(k as i64);
        self.fb = save_offset as usize;
        Ok(())
    }

    fn ttf_write_glyf(&mut self) -> R<()> {
        let tab = self.ttf_name_lookup(b"glyf", true)?.expect("required");
        let glyf_offset = self.dir_tab[tab].offset as i64;
        let new_glyf_offset = self.ttf_offset();
        self.ttf_reset_chksm(tab);
        let mut idn = 0usize;
        // N.B.: new_glyphs_count, in the condition, changes in the loop.
        while idn < self.new_glyphs_count as usize {
            let id = self.glyph_index[idn] as usize;
            idn += 1;
            if id + 1 >= self.glyph_tab.len() {
                // A component beyond the table: the C reads past glyph_tab.
                return Err(FontError::new(b"glyf: glyph index out of range"));
            }
            self.glyph_tab[id].newoffset = (self.ttf_offset() - new_glyf_offset) as i32;
            if self.glyph_tab[id].offset != self.glyph_tab[id + 1].offset {
                self.pos = (glyf_offset as u32).wrapping_add(self.glyph_tab[id].offset as u32) as usize;
                let k = self.copy_short()?;
                self.ttf_ncopy(4 * TTF_FWORD_SIZE)?;
                if k < 0 {
                    loop {
                        let flags = self.copy_ushort()?;
                        let idx = self.get_ushort()? as usize;
                        if idx >= self.glyph_tab.len() {
                            return Err(FontError::new(b"glyf: glyph index out of range"));
                        }
                        if self.glyph_tab[idx].newindex < 0 {
                            self.glyph_tab[idx].newindex = self.new_glyphs_count as i16;
                            let n = self.new_glyphs_count as usize;
                            if n >= self.glyph_index.len() {
                                self.glyph_index.resize(n + 1, 0);
                            }
                            self.glyph_index[n] = idx as i64;
                            self.new_glyphs_count = self.new_glyphs_count.wrapping_add(1);
                        }
                        let v = self.glyph_tab[idx].newindex as i64;
                        self.put_ushort(v);
                        if flags & ARG_1_AND_2_ARE_WORDS != 0 {
                            self.ttf_ncopy(2 * TTF_SHORT_SIZE)?;
                        } else {
                            self.ttf_ncopy(TTF_USHORT_SIZE)?;
                        }
                        if flags & WE_HAVE_A_SCALE != 0 {
                            self.ttf_ncopy(TTF_F2DOT14_SIZE)?;
                        } else if flags & WE_HAVE_AN_X_AND_Y_SCALE != 0 {
                            self.ttf_ncopy(2 * TTF_F2DOT14_SIZE)?;
                        } else if flags & WE_HAVE_A_TWO_BY_TWO != 0 {
                            self.ttf_ncopy(4 * TTF_F2DOT14_SIZE)?;
                        }
                        if flags & MORE_COMPONENTS == 0 {
                            if flags & WE_HAVE_INSTRUCTIONS != 0 {
                                let n = self.copy_ushort()? as i32;
                                self.ttf_ncopy(n)?;
                            }
                            break;
                        }
                    }
                } else {
                    let n = self.glyph_tab[id + 1].offset as i64
                        - self.glyph_tab[id].offset as i64
                        - TTF_USHORT_SIZE as i64
                        - 4 * TTF_FWORD_SIZE as i64;
                    self.ttf_ncopy(n as i32)?;
                }
            }
        }
        self.last_glyf_offset = (self.ttf_offset() - new_glyf_offset) as u32;
        self.ttf_set_chksm(tab);
        Ok(())
    }

    fn append_new_glyph(&mut self, c: usize, glyph: usize) -> R<()> {
        if !(glyph > 0 && glyph < self.glyphs_count as usize) {
            return Err(FontError::assertion(
                WRITETTF_C,
                1125,
                "ttf_reindex_glyphs",
                "glyph > glyph_tab && glyph - glyph_tab < glyphs_count",
            ));
        }
        if self.glyph_tab[glyph].newindex < 0 {
            let n = self.new_glyphs_count as usize;
            if n >= self.glyph_index.len() {
                self.glyph_index.resize(n + 1, 0);
            }
            self.glyph_index[n] = glyph as i64;
            self.glyph_tab[glyph].newindex = self.new_glyphs_count as i16;
            self.new_glyphs_count = self.new_glyphs_count.wrapping_add(1);
        }
        self.ttfenc_tab[c].newindex = self.glyph_tab[glyph].newindex as i64;
        Ok(())
    }

    fn ttf_reindex_glyphs(&mut self, inp: &TtfInput<'_>, cache: &mut TtfCmapCache) -> R<()> {
        // Reindexing glyphs: we append the index of used glyphs to `glyph_index' while going through
        // `ttfenc_tab'.  After appending a new entry to `glyph_index' we set field `newindex' of the
        // corresponding entries in both `glyph_tab' and `ttfenc_tab' to the newly created index.
        let mut cmap: Option<usize> = None;
        let mut cmap_not_found = false;
        for c in 0..256 {
            self.ttfenc_tab[c].newindex = 0; // index of ".notdef" glyph

            // handle case of subfonts first
            if inp.subfont.is_some() {
                let code = self.ttfenc_tab[c].code;
                if code == -1 {
                    continue;
                }
                if inp.pid == -1 || inp.eid == -1 {
                    return Err(FontError::assertion(
                        WRITETTF_C,
                        1040,
                        "ttf_reindex_glyphs",
                        "fd_cur->fm->pid != -1 && fd_cur->fm->eid != -1",
                    ));
                }
                if cmap.is_none() && !cmap_not_found {
                    cmap = self.ttf_read_cmap(cache, inp.ff_name, inp.pid, inp.eid, true)?;
                    if cmap.is_none() {
                        cmap_not_found = true;
                    }
                }
                let cm = match cmap {
                    Some(cm) => cm,
                    None => continue,
                };
                let g = cache.entries[cm].table[code as usize];
                if g < 0 {
                    let m = join(&[
                        b"subfont ",
                        inp.tfm_name,
                        b": wrong mapping: character ",
                        &dec(c as i64),
                        b" --> 0x",
                        &hex4(code),
                        b" --> .notdef",
                    ]);
                    self.warn(m);
                    continue;
                }
                self.append_new_glyph(c, g as usize)?;
                continue;
            }

            // handle case of reencoded fonts
            let q = match self.ttfenc_tab[c].name {
                Some(q) => q,
                None => continue,
            };
            let names = inp.fe_names.expect("a name comes from the encoding");
            let e_name = names.name(q);

            // look up by name first
            let mut found = None;
            for g in 0..self.glyphs_count as usize {
                let n = self.glyph_tab[g].name;
                if !matches!(n, GName::Notdef) && self.glyph_name(n) == e_name {
                    found = Some(g);
                    break;
                }
            }
            if let Some(g) = found {
                self.append_new_glyph(c, g)?;
                continue;
            }

            // scan form `uniABCD'
            let mut sc = Scan::new(e_name, 0);
            let mut n: i64 = -1;
            let mut index: i32 = 0;
            if sc.lit(b"uni") {
                if let Some(v) = sc.int(16, usize::MAX) {
                    index = v as i32;
                    n = sc.i as i64;
                }
            }
            if n == e_name.len() as i64 {
                if cmap.is_none() && !cmap_not_found {
                    // need to read the unicode mapping, ie (pid,eid) = (3,1) or (0,3)
                    cmap = self.ttf_read_cmap(cache, inp.ff_name, 3, 1, false)?;
                    if cmap.is_none() {
                        cmap = self.ttf_read_cmap(cache, inp.ff_name, 0, 3, false)?;
                    }
                    if cmap.is_none() {
                        self.warn(b"no unicode mapping found, all `uniXXXX' names will be ignored".to_vec());
                        cmap_not_found = true; // once only
                    }
                }
                let cm = match cmap {
                    Some(cm) => cm,
                    None => continue,
                };
                // t[index] past the table is out of bounds in the C.
                let t = if (0..0x10000).contains(&index) { cache.entries[cm].table[index as usize] } else { -1 };
                if t != -1 {
                    if t >= self.glyphs_count as i32 {
                        let m = join(&[
                            b"`",
                            e_name,
                            b"' is mapped to index ",
                            &dec(t as i64),
                            b" which is out of valid range [0..",
                            &dec(self.glyphs_count as i64),
                            b")",
                        ]);
                        self.warn(m);
                        continue;
                    }
                    self.append_new_glyph(c, t as usize)?;
                } else {
                    let mut m = b"`unicode uni".to_vec();
                    push_hex(&mut m, index as u32 as u64, 4);
                    m.extend_from_slice(b"' is not mapped to any glyph");
                    self.warn(m);
                }
                continue;
            }

            // scan form `index123'
            let mut sc = Scan::new(e_name, 0);
            let mut n: i64 = -1;
            let mut index: i32 = 0;
            if sc.lit(b"index") {
                if let Some(v) = sc.int(0, usize::MAX) {
                    index = v as i32;
                    n = sc.i as i64;
                }
            }
            if n == e_name.len() as i64 {
                if index >= self.glyphs_count as i32 {
                    let m = join(&[
                        b"`",
                        e_name,
                        b"' out of valid range [0..",
                        &dec(self.glyphs_count as i64),
                        b")",
                    ]);
                    self.warn(m);
                    continue;
                }
                // A negative index fails the assert in append_new_glyph, as glyph_tab + index does.
                self.append_new_glyph(c, if index < 0 { 0 } else { index as usize })?;
                continue;
            }

            // not found
            let m = join(&[b"glyph `", e_name, b"' not found"]);
            self.warn(m);
        }
        Ok(())
    }

    fn ttf_write_head(&mut self, subsetted: bool) -> R<()> {
        let tab = self.ttf_seek_tab(b"head", 0)?;
        self.ttf_reset_chksm(tab);
        self.ttf_ncopy(2 * TTF_FIXED_SIZE)?;
        self.check_sum_adjustment_offset = self.ttf_offset() as u32;
        self.put_ulong(0);
        self.ttf_skip(TTF_ULONG_SIZE)?; // skip checkSumAdjustment
        self.ttf_ncopy(
            TTF_ULONG_SIZE + 2 * TTF_USHORT_SIZE + 16 + 4 * TTF_FWORD_SIZE + 2 * TTF_USHORT_SIZE + TTF_SHORT_SIZE,
        )?;
        if subsetted {
            let v = self.loca_format as i64;
            self.put_short(v);
            self.put_short(0);
        } else {
            self.ttf_ncopy(2 * TTF_SHORT_SIZE)?;
        }
        self.ttf_set_chksm(tab);
        Ok(())
    }

    fn ttf_write_hhea(&mut self) -> R<()> {
        let tab = self.ttf_seek_tab(b"hhea", 0)?;
        self.ttf_reset_chksm(tab);
        self.ttf_ncopy(TTF_FIXED_SIZE + 3 * TTF_FWORD_SIZE + TTF_UFWORD_SIZE + 3 * TTF_FWORD_SIZE + 8 * TTF_SHORT_SIZE)?;
        let v = self.new_glyphs_count as i64;
        self.put_ushort(v);
        self.ttf_set_chksm(tab);
        Ok(())
    }

    fn ttf_write_htmx(&mut self) -> R<()> {
        let tab = self.ttf_seek_tab(b"hmtx", 0)?;
        self.ttf_reset_chksm(tab);
        for k in 0..self.new_glyphs_count as usize {
            let g = self.glyph_tab[self.glyph_index[k] as usize];
            self.put_ufword(g.adv_width as i64);
            self.put_ufword(g.lsb as i64);
        }
        self.ttf_set_chksm(tab);
        Ok(())
    }

    fn ttf_write_loca(&mut self) -> R<()> {
        let tab = self.ttf_seek_tab(b"loca", 0)?;
        self.ttf_reset_chksm(tab);
        self.loca_format = 0;
        let count = self.new_glyphs_count as usize;
        if self.last_glyf_offset >= 0x00020000 || (self.last_glyf_offset & 1) != 0 {
            self.loca_format = 1;
        } else {
            for k in 0..count {
                if self.glyph_tab[self.glyph_index[k] as usize].newoffset & 1 != 0 {
                    self.loca_format = 1;
                    break;
                }
            }
        }
        if self.loca_format != 0 {
            for k in 0..count {
                let v = self.glyph_tab[self.glyph_index[k] as usize].newoffset as i64;
                self.put_ulong(v);
            }
            let v = self.last_glyf_offset as i64;
            self.put_ulong(v);
        } else {
            for k in 0..count {
                let v = self.glyph_tab[self.glyph_index[k] as usize].newoffset as i64 / 2;
                self.put_ushort(v);
            }
            let v = self.last_glyf_offset as i64 / 2;
            self.put_ushort(v);
        }
        self.ttf_set_chksm(tab);
        Ok(())
    }

    fn ttf_write_mapx(&mut self) -> R<()> {
        let tab = self.ttf_seek_tab(b"maxp", TTF_FIXED_SIZE + TTF_USHORT_SIZE)?;
        self.ttf_reset_chksm(tab);
        self.put_fixed(0x00010000);
        let v = self.new_glyphs_count as i64;
        self.put_ushort(v);
        self.ttf_ncopy(13 * TTF_USHORT_SIZE)?;
        self.ttf_set_chksm(tab);
        Ok(())
    }

    fn ttf_write_os2(&mut self) -> R<()> {
        let tab = self.ttf_seek_tab(b"OS/2", 0)?;
        self.ttf_reset_chksm(tab);
        let version = self.get_ushort()?;
        if version > 5 {
            let m = join(&[b"unknown version of OS/2 table (", &hex4(version as i64), b")"]);
            self.warn(m);
        }
        self.put_ushort(0x0001); // fix version to 1
        self.ttf_ncopy(2 * TTF_USHORT_SIZE + 13 * TTF_SHORT_SIZE + 10 * TTF_BYTE_SIZE)?;
        self.ttf_skip(4 * TTF_ULONG_SIZE)?; // ulUnicodeRange 1--4
        self.put_ulong(0x00000003); // Basic Latin + Latin-1 Supplement (0x0000--0x00FF)
        self.put_ulong(0x10000000); // Private Use (0xE000--0xF8FF)
        self.put_ulong(0x00000000);
        self.put_ulong(0x00000000);
        self.ttf_ncopy(4 * TTF_CHAR_SIZE + TTF_USHORT_SIZE)?; // achVendID + fsSelection
        self.ttf_skip(2 * TTF_USHORT_SIZE)?;
        self.put_ushort(0x0000); // usFirstCharIndex
        self.put_ushort(0xF0FF); // usLastCharIndex
        self.ttf_ncopy(5 * TTF_USHORT_SIZE)?;
        // for version 0 the OS/2 table ends here, the rest is for version 1
        self.put_ulong(0x80000000); // Symbol Character Set---don't use any code page
        self.put_ulong(0x00000000);
        self.ttf_set_chksm(tab);
        Ok(())
    }

    fn ttf_write_post(&mut self, write_ttf_glyph_names: bool) -> R<()> {
        let tab = self.ttf_seek_tab(b"post", TTF_FIXED_SIZE)?;
        self.ttf_reset_chksm(tab);
        if !write_ttf_glyph_names || self.post_format == 0x00030000 {
            self.put_fixed(0x00030000);
            self.ttf_ncopy(TTF_FIXED_SIZE + 2 * TTF_FWORD_SIZE + 5 * TTF_ULONG_SIZE)?;
        } else {
            self.put_fixed(0x00020000);
            self.ttf_ncopy(TTF_FIXED_SIZE + 2 * TTF_FWORD_SIZE + 5 * TTF_ULONG_SIZE)?;
            let count = self.new_glyphs_count as usize;
            self.put_ushort(count as i64);
            let mut k: i64 = 0;
            for i in 0..count {
                let id = self.glyph_index[i] as usize;
                let g = self.glyph_tab[id];
                let unsafe_name = AMBIGUOUS_NAMES.iter().any(|&n| n == self.glyph_name(g.name));
                if g.name_index as usize >= NMACGLYPHS || unsafe_name {
                    self.glyph_tab[id].name_index = (NMACGLYPHS as i64 + k) as u16;
                    k += 1;
                }
                let v = self.glyph_tab[id].name_index as i64;
                self.put_ushort(v);
            }
            let mut s: Vec<u8> = Vec::with_capacity(64);
            for i in 0..count {
                let g = self.glyph_tab[self.glyph_index[i] as usize];
                if g.name_index as usize >= NMACGLYPHS {
                    s.clear();
                    s.extend_from_slice(self.glyph_name(g.name));
                    self.put_byte(s.len() as i64);
                    for &b in &s {
                        self.put_char(b as i64);
                    }
                }
            }
        }
        self.ttf_set_chksm(tab);
        Ok(())
    }

    fn ttf_init_font(&mut self, n: i32) {
        let mut i = 1;
        let mut k = 0;
        while i <= n {
            i <<= 1;
            k += 1;
        }
        self.put_fixed(0x00010000); // font version
        self.put_ushort(n as i64); // number of tables
        self.put_ushort((i << 3) as i64); // search range
        self.put_ushort((k - 1) as i64); // entry selector
        self.put_ushort(((n << 4) - (i << 3)) as i64); // range shift
        self.fb = (TABDIR_OFF + n * 4 * TTF_ULONG_SIZE) as usize;
    }

    fn ttf_subset_font(&mut self, inp: &TtfInput<'_>, cache: &mut TtfCmapCache, tag: [u8; 6]) -> R<()> {
        self.ttf_init_font(self.new_ntabs as i32);
        for name in [b"PCLT", b"fpgm", b"cvt ", b"prep"] {
            if self.ttf_name_lookup(name, false)?.is_some() {
                self.ttf_copytab(name)?;
            }
        }
        self.ttf_reindex_glyphs(inp, cache)?;
        self.ttf_write_glyf()?;
        self.ttf_write_loca()?;
        self.ttf_write_os2()?;
        self.ttf_write_head(true)?;
        self.ttf_write_hhea()?;
        self.ttf_write_htmx()?;
        self.ttf_write_mapx()?;
        self.ttf_write_name(tag)?;
        self.ttf_write_post(inp.write_ttf_glyph_names)?;
        self.ttf_write_cmap(inp.fe_names)?;
        self.ttf_write_dirtab(true)
    }

    fn ttf_copy_font(&mut self) -> R<()> {
        self.ttf_init_font(self.ntabs as i32);
        for tab in 0..self.dir_tab.len() {
            let tag = self.dir_tab[tab].tag;
            if &tag == b"head" {
                self.ttf_write_head(false)?;
            } else {
                self.ttf_copytab(&tag)?;
            }
        }
        self.ttf_write_dirtab(false)
    }
}

const BYTE_ENCODING_LENGTH: i64 = 256 * TTF_BYTE_SIZE as i64 + 3 * TTF_USHORT_SIZE as i64;
const TRIMMED_TABLE_MAP_LENGTH: i64 = TTF_USHORT_SIZE as i64 * (5 + 256);
const SEG_MAP_DELTA_LENGTH: i64 = (16 + 256) * TTF_USHORT_SIZE as i64;
const CMAP_ENTRY_LENGTH: i64 = 2 * TTF_USHORT_SIZE as i64 + TTF_ULONG_SIZE as i64;

/// `strip_spaces_and_delims`.  The C's `assert(l >= 0 && l < sizeof(buf))` holds too.
fn strip_delims(s: &[u8]) -> R<Vec<u8>> {
    if s.len() >= 256 {
        return Err(FontError::assertion(
            WRITETTF_C,
            326,
            "strip_spaces_and_delims",
            "l >= 0 && l < (int) sizeof(buf)",
        ));
    }
    let mut out = Vec::with_capacity(s.len());
    for &c in s {
        if matches!(c, b'(' | b')' | b'<' | b'>' | b'[' | b']' | b'{' | b'}' | b'/' | b'%') || is_space(c) {
            continue;
        }
        out.push(c);
    }
    // The result goes through xstrdup: it stops at a NUL.
    let l = strlen(&out, 0);
    out.truncate(l);
    Ok(out)
}

fn new_ttf<'a>(font: &'a [u8], fontname: &[u8], dims: &'a mut FontDims, out: &'a mut Vec<u8>) -> Ttf<'a> {
    let base = out.len();
    Ttf {
        font,
        pos: 0,
        dims,
        out,
        base,
        fb: 0,
        warnings: Vec::new(),
        fontname: fontname.to_vec(),
        ntabs: 0,
        upem: 0,
        post_format: 0,
        loca_format: 0,
        last_glyf_offset: 0,
        glyphs_count: 0,
        new_glyphs_count: 2,
        nhmtxs: 0,
        new_ntabs: DEFAULT_NTABS,
        glyph_tab: Vec::new(),
        glyph_tab_null: true,
        glyph_index: Vec::new(),
        name_tab: Vec::new(),
        name_buf: Vec::new(),
        dir_tab: Vec::new(),
        glyph_name_buf: Vec::new(),
        glyph_name_offs: Vec::new(),
        checksum: 0,
        tab_length: 0,
        tmp_ulong: 0,
        check_sum_adjustment_offset: 0,
        ttfenc_tab: [TtfEnc { name: None, code: -1, newindex: 0 }; 256],
    }
}

/// `writettf`.  Appends the font program to `out`.  `gl` is `fd->gl_tree`, `tx` is `fd->tx_tree`; the
/// C adds to both.  An `Err` is `pdftex_fail` or a failed `assert`.
pub fn write_ttf(
    inp: &TtfInput<'_>,
    dims: &mut FontDims,
    gl: &mut GlyphSet,
    tx: &mut CodeSet,
    cmaps: &mut TtfCmapCache,
    out: &mut Vec<u8>,
) -> R<TtfOutput> {
    if inp.subsetted && inp.fe_names.is_none() && inp.subfont.is_none() {
        return Err(FontError::new(b"Subset TrueType must be a reencoded or a subfont"));
    }
    let mut t = new_ttf(inp.font, inp.fontname, dims, out);
    // skip ttc header, prepare for reading first font
    let name = inp.file_name;
    if name.len() >= 4 && ascii_ieq(&name[name.len() - 4..], b".ttc") {
        if t.get_ulong()? != 0x74746366 {
            t.pos = 0;
        } else {
            t.ttf_skip(TTF_FIXED_SIZE)?; // ignore the version
            t.ttf_skip(TTF_ULONG_SIZE)?; // ignore the numFonts
            t.pos = t.get_ulong()? as usize; // goto first font
        }
    }
    t.ttf_read_font()?;
    let mut subset_tag = None;
    if inp.subsetted {
        let tag = t.ttf_copy_encoding(inp, gl, tx)?;
        subset_tag = Some(tag);
        t.ttf_subset_font(inp, cmaps, tag)?;
    } else {
        t.ttf_copy_font()?;
    }
    let length = t.ttf_offset() as i32;
    let end = t.base + t.fb;
    let warnings = core::mem::take(&mut t.warnings);
    let fontname = core::mem::take(&mut t.fontname);
    out.truncate(end);
    Ok(TtfOutput { length, fontname, subset_tag, warnings })
}

/// `writeotf`: appends the `CFF ` table to `out` and reads the metrics into `dims`.  Returns the
/// warnings.  `subsetted` is `is_subsetted(fm)`, which is an error.
pub fn write_otf(font: &[u8], subsetted: bool, dims: &mut FontDims, out: &mut Vec<u8>) -> R<Vec<Vec<u8>>> {
    if subsetted {
        return Err(FontError::new(b"OTF fonts must be included entirely"));
    }
    let mut t = new_ttf(font, b"", dims, out);
    t.ttf_read_tabdir()?;
    // read font parameters
    if t.ttf_name_lookup(b"head", false)?.is_some() {
        t.ttf_read_head()?;
    }
    if t.ttf_name_lookup(b"hhea", false)?.is_some() {
        t.ttf_read_hhea()?;
    }
    if t.ttf_name_lookup(b"PCLT", false)?.is_some() {
        t.ttf_read_pclt()?;
    }
    if t.ttf_name_lookup(b"post", false)?.is_some() {
        t.ttf_read_post()?;
    }
    // copy font file
    let tab = t.ttf_seek_tab(b"CFF ", 0)?;
    let mut i = t.dir_tab[tab].length as i64;
    while i > 0 {
        t.copy_char()?;
        i -= 1;
    }
    Ok(core::mem::take(&mut t.warnings))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tables_have_the_right_sizes() {
        assert_eq!(MAC_GLYPH_NAMES.len(), 258);
        assert_eq!(MAC_GLYPH_NAMES[3], b"space");
        assert_eq!(MAC_GLYPH_NAMES[257], b"dmacron");
        assert_eq!(BYTE_ENCODING_LENGTH, 262);
        assert_eq!(TRIMMED_TABLE_MAP_LENGTH, 522);
        assert_eq!(SEG_MAP_DELTA_LENGTH, 544);
    }

    #[test]
    fn strip() {
        assert_eq!(strip_delims(b"Foo (Bar)/%").unwrap(), b"FooBar".to_vec());
    }

    #[test]
    fn truncated_input_fails() {
        let mut dims = crate::new_font_dims();
        let mut out = Vec::new();
        assert_eq!(
            write_otf(b"abc", false, &mut dims, &mut out).err().unwrap().message,
            b"unexpected EOF".to_vec()
        );
    }
}
