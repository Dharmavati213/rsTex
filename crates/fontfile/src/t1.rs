//! `writet1.c`: Type 1 fonts, PFA and PFB, whole or subsetted, in the same bytes as pdfTeX.
//!
//! A literal port: the functions, their order and their names are the C's, with `t1_` kept where it
//! helps.  The line buffer works as in C, a growing array with a separate write pointer and NUL
//! termination, because a line can hold binary charstring data and several tests stop at a NUL
//! and others do not.  `cs_tab` and `subr_tab` hold ranges into one arena where the C has one
//! `malloc` per glyph.
//!
//! `fixedcontent` is false in pdfTeX (it is true for dvips), so its branches are dropped.

use crate::cstr::*;
use crate::{FontDims, FontError, GlyphNames, GlyphSet, CodeSet, make_subset_tag_round};
use crate::{FONTBBOX1_CODE, FONTNAME_CODE, ITALIC_ANGLE_CODE, STEMV_CODE};

type R<T> = Result<T, FontError>;

const WRITET1_C: &str = "../../../texk/web2c/pdftexdir/writet1.c";

const NOTDEF: &[u8] = b".notdef";

/// `standard_glyph_names`: Adobe StandardEncoding.  `""` stands for `notdef`, the pointer.
pub(crate) const STANDARD_GLYPH_NAMES: [&[u8]; 256] = {
    const N: &[u8] = b"";
    [
        N, N, N, N, N, N, N, N, N, N, N, N, N, N, N, N, // 0x00
        N, N, N, N, N, N, N, N, N, N, N, N, N, N, N, N, // 0x10
        b"space", b"exclam", b"quotedbl", b"numbersign", b"dollar", b"percent", b"ampersand", b"quoteright",
        b"parenleft", b"parenright", b"asterisk", b"plus", b"comma", b"hyphen", b"period", b"slash",
        b"zero", b"one", b"two", b"three", b"four", b"five", b"six", b"seven", b"eight", b"nine", b"colon",
        b"semicolon", b"less", b"equal", b"greater", b"question",
        b"at", b"A", b"B", b"C", b"D", b"E", b"F", b"G", b"H", b"I", b"J", b"K", b"L", b"M", b"N", b"O",
        b"P", b"Q", b"R", b"S", b"T", b"U", b"V", b"W", b"X", b"Y", b"Z", b"bracketleft", b"backslash",
        b"bracketright", b"asciicircum", b"underscore",
        b"quoteleft", b"a", b"b", b"c", b"d", b"e", b"f", b"g", b"h", b"i", b"j", b"k", b"l", b"m", b"n", b"o",
        b"p", b"q", b"r", b"s", b"t", b"u", b"v", b"w", b"x", b"y", b"z", b"braceleft", b"bar", b"braceright",
        b"asciitilde", N,
        N, N, N, N, N, N, N, N, N, N, N, N, N, N, N, N, // 0x80
        N, N, N, N, N, N, N, N, N, N, N, N, N, N, N, N, // 0x90
        N, b"exclamdown", b"cent", b"sterling", b"fraction", b"yen", b"florin", b"section", b"currency",
        b"quotesingle", b"quotedblleft", b"guillemotleft", b"guilsinglleft", b"guilsinglright", b"fi", b"fl",
        N, b"endash", b"dagger", b"daggerdbl", b"periodcentered", N, b"paragraph", b"bullet", b"quotesinglbase",
        b"quotedblbase", b"quotedblright", b"guillemotright", b"ellipsis", b"perthousand", N, b"questiondown",
        N, b"grave", b"acute", b"circumflex", b"tilde", b"macron", b"breve", b"dotaccent", b"dieresis", N, b"ring",
        b"cedilla", N, b"hungarumlaut", b"ogonek", b"caron",
        b"emdash", N, N, N, N, N, N, N, N, N, N, N, N, N, N, N, // 0xd0
        N, b"AE", N, b"ordfeminine", N, N, N, N, b"Lslash", b"Oslash", b"OE", b"ordmasculine", N, N, N, N,
        N, b"ae", N, N, N, b"dotlessi", N, N, b"lslash", b"oslash", b"oe", b"germandbls", N, N, N, N,
    ]
};

const T1_BUF_SIZE: usize = 0x10;

const CS_HSTEM: usize = 1;
const CS_VSTEM: usize = 3;
const CS_VMOVETO: usize = 4;
const CS_RLINETO: usize = 5;
const CS_HLINETO: usize = 6;
const CS_VLINETO: usize = 7;
const CS_RRCURVETO: usize = 8;
const CS_CLOSEPATH: usize = 9;
const CS_CALLSUBR: usize = 10;
const CS_RETURN: usize = 11;
const CS_ESCAPE: usize = 12;
const CS_HSBW: usize = 13;
const CS_ENDCHAR: usize = 14;
const CS_RMOVETO: usize = 21;
const CS_HMOVETO: usize = 22;
const CS_VHCURVETO: usize = 30;
const CS_HVCURVETO: usize = 31;
const CS_1BYTE_MAX: usize = CS_HVCURVETO + 1;
const CS_DOTSECTION: usize = CS_1BYTE_MAX;
const CS_VSTEM3: usize = CS_1BYTE_MAX + 1;
const CS_HSTEM3: usize = CS_1BYTE_MAX + 2;
const CS_SEAC: usize = CS_1BYTE_MAX + 6;
const CS_SBW: usize = CS_1BYTE_MAX + 7;
const CS_DIV: usize = CS_1BYTE_MAX + 12;
const CS_CALLOTHERSUBR: usize = CS_1BYTE_MAX + 16;
const CS_POP: usize = CS_1BYTE_MAX + 17;
const CS_SETCURRENTPOINT: usize = CS_1BYTE_MAX + 33;
const CS_2BYTE_MAX: usize = CS_SETCURRENTPOINT + 1;
const CS_MAX: usize = CS_2BYTE_MAX;

#[derive(Copy, Clone)]
struct CcEntry {
    nargs: u8,
    bottom: bool,
    clear: bool,
    valid: bool,
}

/// `cc_tab`, what `cc_init` fills.
const CC_TAB: [CcEntry; CS_MAX] = {
    const fn set(nargs: u8, bottom: bool, clear: bool) -> CcEntry {
        CcEntry { nargs, bottom, clear, valid: true }
    }
    let mut t = [CcEntry { nargs: 0, bottom: false, clear: false, valid: false }; CS_MAX];
    t[CS_HSTEM] = set(2, true, true);
    t[CS_VSTEM] = set(2, true, true);
    t[CS_VMOVETO] = set(1, true, true);
    t[CS_RLINETO] = set(2, true, true);
    t[CS_HLINETO] = set(1, true, true);
    t[CS_VLINETO] = set(1, true, true);
    t[CS_RRCURVETO] = set(6, true, true);
    t[CS_CLOSEPATH] = set(0, false, true);
    t[CS_CALLSUBR] = set(1, false, false);
    t[CS_RETURN] = set(0, false, false);
    t[CS_HSBW] = set(2, true, true);
    t[CS_ENDCHAR] = set(0, false, true);
    t[CS_RMOVETO] = set(2, true, true);
    t[CS_HMOVETO] = set(1, true, true);
    t[CS_VHCURVETO] = set(4, true, true);
    t[CS_HVCURVETO] = set(4, true, true);
    t[CS_DOTSECTION] = set(0, false, true);
    t[CS_VSTEM3] = set(6, true, true);
    t[CS_HSTEM3] = set(6, true, true);
    t[CS_SEAC] = set(5, true, true);
    t[CS_SBW] = set(4, true, true);
    t[CS_DIV] = set(2, false, false);
    t[CS_CALLOTHERSUBR] = set(0, false, false);
    t[CS_POP] = set(0, false, false);
    t[CS_SETCURRENTPOINT] = set(2, true, true);
    t
};

/// A name that is not a range: `name == notdef`, the pointer.
const NAME_NOTDEF: u32 = u32::MAX;

#[derive(Copy, Clone)]
struct CsEntry {
    name_off: u32,
    name_len: u32,
    data: u32,
    len: u16,
    cslen: u16,
    used: bool,
    valid: bool,
}

const CS_EMPTY: CsEntry = CsEntry { name_off: 0, name_len: 0, data: 0, len: 0, cslen: 0, used: false, valid: false };

/// `cs_token_pairs_list`, as `check_cs_token_pair` walks it.  It is a `const char *[][2]` and the
/// loop advances a `const char **` by one, so it looks at overlapping pairs.
const CS_TOKEN_PAIRS: [&[u8]; 8] =
    [b" RD", b"NP", b" -|", b"|", b" RD", b"noaccess put", b" -|", b"noaccess put"];

/// What `cc_stack` holds at most.  The C has 24 and no check on a push: past 24 it corrupts its
/// own statics.  No font in TeX Live gets there; this stops a hostile one safely.
const CC_STACK_SIZE: usize = 256;

/// The deepest subr nesting allowed.  The C has no limit and dies of a stack overflow.
const CS_MARK_DEPTH: u32 = 1000;

/// The C statics that outlive one font: pass the same value to each `write_t1` of a run.
///
/// They only matter for odd fonts (a `pop` with no earlier `callothersubr`, a charstring that leaves
/// numbers on the stack, a font with no `/FontName`), but then the bytes depend on them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct T1Carry {
    last_arg_other_subr3: i64,
    cc_stack: [i64; CC_STACK_SIZE],
    stack_ptr: usize,
    fontname_offset: i64,
}

impl Default for T1Carry {
    fn default() -> T1Carry {
        T1Carry { last_arg_other_subr3: 3, cc_stack: [0; CC_STACK_SIZE], stack_ptr: 0, fontname_offset: 0 }
    }
}

/// How a font used the statics of [`T1Carry`], for a cache of [`write_t1`]: its result does not depend on them beyond `last_arg_in`, if `reusable`, and these are the changes it made.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CarryUse {
    /// Nothing else that an earlier font left was read: the stack was empty at the start, it was never raised by a negative pop, and an offset was set before it was used.
    pub reusable: bool,
    /// `last_arg_other_subr3` was read before this font wrote it, with this value.
    pub last_arg_in: Option<i64>,
    /// The value it wrote last.
    pub last_arg_out: Option<i64>,
    /// `t1_fontname_offset`, if it set one.
    pub fontname_offset: Option<i64>,
    /// `stack_ptr` at the end.
    pub stack_ptr: usize,
    /// The start of `cc_stack`, as far as it was written.
    pub cc: Vec<i64>,
}

impl T1Carry {
    /// Whether a font that was `reusable` may stand for a run from here.
    pub fn fits(&self, u: &CarryUse) -> bool {
        self.stack_ptr == 0 && u.reusable && u.last_arg_in.is_none_or(|v| v == self.last_arg_other_subr3)
    }

    /// What the font would have changed.
    pub fn apply(&mut self, u: &CarryUse) {
        if let Some(v) = u.last_arg_out {
            self.last_arg_other_subr3 = v;
        }
        if let Some(v) = u.fontname_offset {
            self.fontname_offset = v;
        }
        self.stack_ptr = u.stack_ptr;
        self.cc_stack[..u.cc.len()].copy_from_slice(&u.cc);
    }
}

/// What `writet1` reads from its `fd_entry` and `fm_entry`.
pub struct T1Input<'a> {
    /// The whole PFA or PFB.
    pub font: &'a [u8],
    /// `is_subsetted(fm)`.
    pub subsetted: bool,
    /// `fd->all_glyphs`, set only by PDF inclusion.
    pub all_glyphs: bool,
    /// `fm_slant(fm)`.
    pub slant: i64,
    /// `fm_extend(fm)`.
    pub extend: i64,
    /// `fd->fontname` on entry, the fallback from `preset_fontname`.
    pub fontname: &'a [u8],
    /// `fd->tx_tree`: characters of non-reencoded fonts using this descriptor; None for NULL.
    pub tx_codes: Option<&'a CodeSet>,
    /// `st_tree` of `make_subset_tag`: tags already given out this run.
    pub used_tags: &'a [[u8; 6]],
}

/// What `writet1` leaves in `t1_length1..3` and in the `fd_entry`.
pub struct T1Output {
    pub length1: i32,
    pub length2: i32,
    pub length3: i32,
    /// `fd->fontname`, without the tag.
    pub fontname: Vec<u8>,
    /// `fd->subset_tag`; the caller adds it to `used_tags`.
    pub subset_tag: Option<[u8; 6]>,
    /// `fd->builtin_glyph_names`: None for a font not subsetted.
    pub builtin_glyph_names: Option<GlyphNames>,
    /// `pdftex_warn` messages, in order.
    pub warnings: Vec<Vec<u8>>,
    /// See there.
    pub carry_use: CarryUse,
}

struct T1<'a> {
    inp: &'a T1Input<'a>,
    dims: &'a mut FontDims,
    gl: &'a mut GlyphSet,
    carry: &'a mut T1Carry,
    out: &'a mut Vec<u8>,
    base: usize,

    pos: usize,
    eof: bool,

    line: Vec<u8>,
    lp: usize,

    fontname: Vec<u8>,
    subset_tag: Option<[u8; 6]>,
    warnings: Vec<Vec<u8>>,

    t1_length1: i32,
    t1_length2: i32,
    t1_save_offset: i32,

    t1_dr: u16,
    t1_er: u16,
    t1_cslen: u16,
    t1_len_iv: i16,
    cs_start: usize,

    t1_pfa: bool,
    t1_cs: bool,
    t1_scan: bool,
    t1_eexec_encrypt: bool,
    t1_synthetic: bool,
    t1_in_eexec: i32,
    t1_block_length: i64,
    last_hexbyte: u8,

    enc_standard: bool,

    arena: Vec<u8>,
    cs_tab: Vec<CsEntry>,
    cs_notdef: Option<usize>,
    cs_dict_start: Vec<u8>,
    cs_dict_end: Vec<u8>,
    cs_count: i32,
    cs_size: i32,
    cs_size_pos: usize,

    subr_tab: Vec<CsEntry>,
    subr_tab_null: bool,
    subr_array_start: Vec<u8>,
    subr_array_end: Vec<u8>,
    subr_max: i32,
    subr_size: i32,
    subr_size_pos: usize,

    cs_token_pair: Option<usize>,

    use_clean: bool,
    use_last_arg_in: Option<i64>,
    use_last_arg_out: Option<i64>,
    use_fontname_offset: Option<i64>,
    use_cc_hi: usize,
}

/// `writet1`.  Appends the font program to `out`; the lengths count from where it stood.  `gl` is
/// `fd->gl_tree`, read and extended (the tag's and `/CharSet`'s glyphs, with the base and accent of
/// every `seac`); `dims` is `fd->font_dim`, overwritten with what the font says.  An `Err` is
/// `pdftex_fail`, or a failed `assert`: the C does not come back from either.
pub fn write_t1(
    inp: &T1Input<'_>,
    dims: &mut FontDims,
    gl: &mut GlyphSet,
    carry: &mut T1Carry,
    out: &mut Vec<u8>,
) -> R<T1Output> {
    let base = out.len();
    let carry_clean = carry.stack_ptr == 0;
    let mut t = T1 {
        inp,
        dims,
        gl,
        carry,
        out,
        base,
        pos: 0,
        eof: false,
        line: vec![0; 4096],
        lp: 0,
        fontname: inp.fontname.to_vec(),
        subset_tag: None,
        warnings: Vec::new(),
        t1_length1: 0,
        t1_length2: 0,
        t1_save_offset: 0,
        t1_dr: 55665,
        t1_er: 55665,
        t1_cslen: 0,
        t1_len_iv: 4,
        cs_start: 0,
        t1_pfa: false,
        t1_cs: false,
        t1_scan: true,
        t1_eexec_encrypt: false,
        t1_synthetic: false,
        t1_in_eexec: 0,
        t1_block_length: 0,
        last_hexbyte: 0,
        enc_standard: false,
        arena: Vec::with_capacity(inp.font.len()),
        cs_tab: Vec::new(),
        cs_notdef: None,
        cs_dict_start: Vec::new(),
        cs_dict_end: Vec::new(),
        cs_count: 0,
        cs_size: 0,
        cs_size_pos: 0,
        subr_tab: Vec::new(),
        subr_tab_null: true,
        subr_array_start: Vec::new(),
        subr_array_end: Vec::new(),
        subr_max: 0,
        subr_size: 0,
        subr_size_pos: 0,
        cs_token_pair: None,
        use_clean: carry_clean,
        use_last_arg_in: None,
        use_last_arg_out: None,
        use_fontname_offset: None,
        use_cc_hi: 0,
    };
    t.t1_check_pfa();
    match t.run() {
        Ok(builtin) => {
            let carry_use = CarryUse {
                reusable: t.use_clean && (t.use_fontname_offset.is_some() || !inp.subsetted),
                last_arg_in: t.use_last_arg_in,
                last_arg_out: t.use_last_arg_out,
                fontname_offset: t.use_fontname_offset,
                stack_ptr: t.carry.stack_ptr,
                cc: t.carry.cc_stack[..t.use_cc_hi].to_vec(),
            };
            Ok(T1Output {
                length1: t.t1_length1,
                length2: t.t1_length2,
                length3: 0,
                fontname: t.fontname,
                subset_tag: t.subset_tag,
                builtin_glyph_names: builtin,
                warnings: t.warnings,
                carry_use,
            })
        }
        Err(mut e) => {
            e.warnings = t.warnings;
            Err(e)
        }
    }
}

fn fail(msg: &[u8]) -> FontError {
    FontError::new(msg)
}

#[inline]
fn cdecrypt(cipher: u8, cr: &mut u16) -> u8 {
    let plain = cipher ^ (*cr >> 8) as u8;
    *cr = (cipher as u32).wrapping_add(*cr as u32).wrapping_mul(52845).wrapping_add(22719) as u16;
    plain
}

#[inline]
fn cencrypt(plain: u8, cr: &mut u16) -> u8 {
    let cipher = plain ^ (*cr >> 8) as u8;
    *cr = (cipher as u32).wrapping_add(*cr as u32).wrapping_mul(52845).wrapping_add(22719) as u16;
    cipher
}

fn hexval(c: u8) -> i32 {
    match c {
        b'A'..=b'F' => (c - b'A') as i32 + 10,
        b'a'..=b'f' => (c - b'a') as i32 + 10,
        b'0'..=b'9' => (c - b'0') as i32,
        _ => -1,
    }
}

impl T1<'_> {
    /// The body of `writet1`.
    fn run(&mut self) -> R<Option<GlyphNames>> {
        if !self.inp.subsetted {
            self.t1_include()?;
            return Ok(None);
        }
        let builtin = self.t1_subset_ascii_part()?;
        self.t1_start_eexec()?;
        self.cs_init();
        self.t1_read_subrs()?;
        self.t1_subset_charstrings()?;
        self.t1_subset_end()?;
        Ok(Some(builtin))
    }

    fn warn(&mut self, msg: Vec<u8>) {
        self.warnings.push(crate::truncate_message(msg));
    }

    /// `t1_offset`, `fb_offset`.
    #[inline]
    fn offset(&self) -> i32 {
        (self.out.len() - self.base) as i32
    }

    #[inline]
    fn t1_getchar(&mut self) -> i32 {
        match self.inp.font.get(self.pos) {
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

    fn t1_check_pfa(&mut self) {
        // t1_getchar and t1_ungetchar: an empty file leaves feof set, as ungetc(EOF) does nothing.
        match self.inp.font.first() {
            Some(&c) => self.t1_pfa = c != 128,
            None => {
                self.eof = true;
                self.t1_pfa = true;
            }
        }
    }

    #[inline]
    fn t1_getbyte(&mut self) -> R<i32> {
        let mut c = self.t1_getchar();
        if self.t1_pfa {
            return Ok(c);
        }
        if self.t1_block_length == 0 {
            if c != 128 {
                return Err(fail(b"invalid marker"));
            }
            c = self.t1_getchar();
            if c == 3 {
                while !self.eof {
                    self.t1_getchar();
                }
                return Ok(-1);
            }
            let b0 = (self.t1_getchar() & 0xff) as i64;
            let b1 = (self.t1_getchar() & 0xff) as i64;
            let b2 = (self.t1_getchar() & 0xff) as i64;
            let b3 = (self.t1_getchar() & 0xff) << 24;
            // The last shift is an `int`: bit 31 makes it negative, which then widens to long.
            self.t1_block_length = b0 | (b1 << 8) | (b2 << 16) | b3 as i64;
            c = self.t1_getchar();
        }
        self.t1_block_length -= 1;
        Ok(c)
    }

    #[inline]
    fn edecrypt(&mut self, mut cipher: u8) -> R<u8> {
        if self.t1_pfa {
            while cipher == 10 || cipher == 13 {
                cipher = self.t1_getbyte()? as u8;
            }
            let lo = self.t1_getbyte()? as u8;
            cipher = ((hexval(cipher) << 4) + hexval(lo)) as u8;
            self.last_hexbyte = cipher;
        }
        let plain = cipher ^ (self.t1_dr >> 8) as u8;
        self.t1_dr = (cipher as u32).wrapping_add(self.t1_dr as u32).wrapping_mul(52845).wrapping_add(22719) as u16;
        Ok(plain)
    }

    /// `alloc_array(t1_line, n, T1_BUF_SIZE)`.
    #[inline]
    fn alloc_line(&mut self, n: i64) -> R<()> {
        let want = self.lp as i64 + n;
        if want as u32 as usize > self.line.len() {
            let mut limit = self.line.len().max(T1_BUF_SIZE) * 2;
            if want as u32 as usize > limit {
                limit = want as usize;
            }
            if limit > i32::MAX as usize {
                return Err(fail(b"t1_line_array exceeds size limit"));
            }
            self.line.resize(limit, 0);
        }
        Ok(())
    }

    #[inline]
    fn prefix(&self, s: &[u8]) -> bool {
        str_prefix(&self.line, 0, s)
    }

    /// `t1_suffix`.
    #[inline]
    fn suffix(&self, s: &[u8]) -> bool {
        str_suffix(&self.line, 0, self.lp, s)
    }

    /// `t1_charstrings`: `strstr(t1_line_array, charstringname)`.
    #[inline]
    fn t1_charstrings(&self) -> bool {
        strstr(&self.line, 0, b"/CharStrings").is_some()
    }

    /// `t1_subrs`.
    #[inline]
    fn t1_subrs(&self) -> bool {
        self.prefix(b"/Subrs")
    }

    /// `t1_end_eexec`.
    #[inline]
    fn t1_end_eexec(&self) -> bool {
        self.suffix(b"mark currentfile closefile")
    }

    /// `eol`: the string at `s` gets a final newline; returns where it ends.
    fn eol(&mut self, s: usize) -> usize {
        let mut p = s + strlen(&self.line, s);
        if p - s > 1 && self.line[p - 1] != 10 {
            if p + 2 > self.line.len() {
                self.line.resize(p + 2, 0);
            }
            self.line[p] = 10;
            p += 1;
            self.line[p] = 0;
        }
        p
    }

    /// `remove_eol` and the text of a failure that quotes the line.
    fn fail_line(&self, before: &[u8], after: &[u8]) -> FontError {
        let mut l = cstr(&self.line, 0);
        if l.last() == Some(&10) {
            l = &l[..l.len() - 1];
        }
        let mut m = before.to_vec();
        m.extend_from_slice(l);
        m.extend_from_slice(after);
        FontError::new(&m)
    }

    /// `t1_scan_num(p, r)`; the second value is `*r`.
    fn t1_scan_num(&self, mut p: usize) -> R<(f32, usize)> {
        if at(&self.line, p) == b' ' {
            p += 1;
        }
        let mut sc = Scan::new(&self.line, p);
        let f = match sc.float() {
            Some(f) => f,
            None => return Err(self.fail_line(b"a number expected: `", b"'")),
        };
        let mut r = p;
        loop {
            let c = at(&self.line, r);
            if c.is_ascii_digit() || c == b'.' || c == b'e' || c == b'E' || c == b'+' || c == b'-' {
                r += 1;
            } else {
                break;
            }
        }
        Ok((f, r))
    }

    fn t1_getline(&mut self) -> R<()> {
        const EEXEC_STR: &[u8; 17] = b"currentfile eexec";
        const EEXEC_LEN: i32 = 17;
        let mut eexec_scan: i32;
        loop {
            if self.eof {
                return Err(fail(b"unexpected end of file"));
            }
            self.lp = 0;
            self.alloc_line(1)?;
            self.t1_cslen = 0;
            eexec_scan = 0;
            let mut c = self.t1_getbyte()?;
            if c == -1 {
                // goto exit; the rest of t1_getline is about t1_buf_array, sized on use here.
                return Ok(());
            }
            while !self.eof {
                if self.t1_in_eexec == 1 {
                    c = self.edecrypt(c as u8)? as i32;
                }
                self.alloc_line(1)?;
                // append_char_to_buf
                if c == 9 {
                    c = 32;
                }
                if c == 13 || c == -1 {
                    c = 10;
                }
                if c != 32 || (self.lp > 0 && self.line[self.lp - 1] != 32) {
                    self.line[self.lp] = c as u8;
                    self.lp += 1;
                }
                if self.t1_in_eexec == 0 && eexec_scan >= 0 && eexec_scan < EEXEC_LEN {
                    if self.line[eexec_scan as usize] == EEXEC_STR[eexec_scan as usize] {
                        eexec_scan += 1;
                    } else {
                        eexec_scan = -1;
                    }
                }
                if c == 10 || (self.t1_pfa && eexec_scan == EEXEC_LEN && c == 32) {
                    break;
                }
                if self.t1_cs
                    && self.t1_cslen == 0
                    && self.lp > 4
                    && (self.suffix(b" RD ") || self.suffix(b" -| "))
                {
                    let mut p = self.lp as isize - 5;
                    while p >= 0 && self.line[p as usize] != b' ' {
                        p -= 1;
                    }
                    if p < 0 {
                        // The C runs off the front of its array here.
                        return Err(self.fail_line(b"a number expected: `", b"'"));
                    }
                    let (f, _) = self.t1_scan_num(p as usize + 1)?;
                    let mut l = ftoi(f);
                    self.t1_cslen = l as u16;
                    self.cs_start = self.lp;
                    self.alloc_line(l as i64)?;
                    // Most of a font is this: a run of bytes inside one block of a PFB goes faster than each through `t1_getbyte`.
                    if !self.t1_pfa && l > 0 && (l as i64) <= self.t1_block_length && self.pos + l as usize <= self.inp.font.len() {
                        let n = l as usize;
                        let mut dr = self.t1_dr;
                        let src = &self.inp.font[self.pos..self.pos + n];
                        let dst = &mut self.line[self.lp..self.lp + n];
                        for (d, &cipher) in dst.iter_mut().zip(src) {
                            *d = cipher ^ (dr >> 8) as u8;
                            dr = (cipher as u32).wrapping_add(dr as u32).wrapping_mul(52845).wrapping_add(22719) as u16;
                        }
                        self.t1_dr = dr;
                        self.pos += n;
                        self.t1_block_length -= l as i64;
                        self.lp += n;
                        l = 0;
                    }
                    while l > 0 {
                        l -= 1;
                        let b = self.t1_getbyte()? as u8;
                        let d = self.edecrypt(b)?;
                        self.line[self.lp] = d;
                        self.lp += 1;
                    }
                }
                c = self.t1_getbyte()?;
            }
            self.alloc_line(2)?;
            // append_eol
            if self.lp > 1 && self.line[self.lp - 1] != 10 {
                self.line[self.lp] = 10;
                self.lp += 1;
            }
            if self.lp > 2 && self.line[self.lp - 2] == 32 {
                self.line[self.lp - 2] = 10;
                self.lp -= 1;
            }
            self.line[self.lp] = 0;
            if self.lp < 2 {
                continue; // goto restart
            }
            if eexec_scan == EEXEC_LEN {
                self.t1_in_eexec = 1;
            }
            return Ok(());
        }
    }

    fn t1_putline(&mut self) {
        if self.lp <= 1 {
            return;
        }
        if self.t1_eexec_encrypt {
            self.out.reserve(self.lp);
            let mut er = self.t1_er;
            for i in 0..self.lp {
                let cipher = self.line[i] ^ (er >> 8) as u8;
                er = (cipher as u32).wrapping_add(er as u32).wrapping_mul(52845).wrapping_add(22719) as u16;
                self.out.push(cipher);
            }
            self.t1_er = er;
        } else {
            self.out.extend_from_slice(&self.line[..self.lp]);
        }
    }

    /// `t1_puts` with a string that is not `t1_line_array`.
    fn t1_puts(&mut self, s: &[u8]) {
        if s.len() + 1 > self.line.len() {
            self.line.resize(s.len() + 1, 0);
        }
        self.line[..s.len()].copy_from_slice(s);
        self.line[s.len()] = 0;
        self.lp = s.len();
        self.t1_putline();
    }

    fn t1_check_block_len(&mut self, decrypt: bool) -> R<()> {
        if self.t1_block_length == 0 {
            return Ok(());
        }
        let mut c = self.t1_getbyte()?;
        if decrypt {
            c = self.edecrypt(c as u8)? as i32;
        }
        let l = self.t1_block_length as i32;
        if !(l == 0 && (c == 10 || c == 13)) {
            let mut m = Vec::new();
            push_i64(&mut m, l as i64 + 1);
            m.extend_from_slice(b" bytes more than expected");
            return Err(FontError::new(&m));
        }
        Ok(())
    }

    fn t1_start_eexec(&mut self) -> R<()> {
        self.t1_length1 = self.offset() - self.t1_save_offset;
        self.t1_save_offset = self.offset();
        if !self.t1_pfa {
            self.t1_check_block_len(false)?;
        }
        self.lp = 0;
        self.alloc_line(4)?;
        for _ in 0..4 {
            let b = self.t1_getbyte()? as u8;
            self.edecrypt(b)?;
            self.line[self.lp] = 0;
            self.lp += 1;
        }
        self.t1_eexec_encrypt = true;
        self.t1_putline();
        Ok(())
    }

    fn t1_stop_eexec(&mut self) -> R<()> {
        self.t1_length2 = self.offset() - self.t1_save_offset;
        self.t1_save_offset = self.offset();
        self.t1_eexec_encrypt = false;
        if !self.t1_pfa {
            self.t1_check_block_len(true)?;
        } else {
            let b = self.t1_getbyte()? as u8;
            let c = self.edecrypt(b)?;
            if !(c == 10 || c == 13) {
                if self.last_hexbyte == 0 {
                    self.t1_puts(b"00");
                } else {
                    return Err(fail(b"unexpected data after eexec"));
                }
            }
        }
        self.t1_cs = false;
        self.t1_in_eexec = 2;
        Ok(())
    }

    fn t1_modify_fm(&mut self) -> R<()> {
        let slant = self.inp.slant;
        let extend = self.inp.extend;
        let mut p = match strchr(&self.line, 0, b'[') {
            Some(p) => p,
            None => match strchr(&self.line, 0, b'{') {
                Some(p) => p,
                None => return Err(self.fail_line(b"FontMatrix: an array expected: `", b"'")),
            },
        };
        let c = self.line[p];
        p += 1;
        let mut r: Vec<u8> = self.line[..p].to_vec();
        let mut a = [0f32; 6];
        for v in a.iter_mut() {
            let (f, q) = self.t1_scan_num(p)?;
            *v = f;
            p = q;
        }
        if slant != 0 {
            // do_slant(a, fm_slant * 1E-3): in double, narrowed on each store
            let s = slant as f64 * 1E-3;
            a[0] = (a[0] as f64 + a[1] as f64 * s) as f32;
            a[2] = (a[2] as f64 + a[3] as f64 * s) as f32;
            a[4] = (a[4] as f64 + a[5] as f64 * s) as f32;
        }
        if extend != 0 {
            // do_extend(a, fm_extend * 1E-3)
            let e = extend as f64 * 1E-3;
            a[0] = (a[0] as f64 * e) as f32;
            a[2] = (a[2] as f64 * e) as f32;
            a[4] = (a[4] as f64 * e) as f32;
        }
        for v in a {
            push_g(&mut r, v as f64);
            r.push(b' ');
        }
        let close = if c == b'[' { b']' } else { b'}' };
        while at(&self.line, p) != close && at(&self.line, p) != 0 {
            p += 1;
        }
        if at(&self.line, p) == 0 {
            let mut before = b"FontMatrix: cannot find the corresponding character to '".to_vec();
            before.push(c);
            before.extend_from_slice(b"': `");
            return Err(self.fail_line(&before, b"'"));
        }
        r.extend_from_slice(cstr(&self.line, p));
        self.set_line(&r);
        self.lp = self.eol(0);
        Ok(())
    }

    /// `strcpy(t1_line_array, s)`.
    fn set_line(&mut self, s: &[u8]) {
        if s.len() + 3 > self.line.len() {
            self.line.resize(s.len() + 3, 0);
        }
        self.line[..s.len()].copy_from_slice(s);
        self.line[s.len()] = 0;
    }

    fn t1_modify_italic(&mut self) -> R<()> {
        let slant = self.inp.slant;
        if slant == 0 {
            return Ok(());
        }
        let p = match strchr(&self.line, 0, b' ') {
            Some(p) => p,
            None => return Err(self.fail_line(b"a number expected: `", b"'")),
        };
        let mut b: Vec<u8> = self.line[..p + 1].to_vec();
        let (f, r) = self.t1_scan_num(p + 1)?;
        let a = (f as f64 - atan(slant as f64 * 1E-3) * (180.0 / core::f64::consts::PI)) as f32;
        push_g(&mut b, a as f64);
        b.extend_from_slice(cstr(&self.line, r));
        self.set_line(&b);
        self.lp = self.eol(0);
        self.dims[ITALIC_ANGLE_CODE].val = zround(a as f64) as i32;
        self.dims[ITALIC_ANGLE_CODE].set = true;
        Ok(())
    }

    fn t1_scan_keys(&mut self) -> R<()> {
        if self.inp.extend != 0 || self.inp.slant != 0 {
            if self.prefix(b"/FontMatrix") {
                return self.t1_modify_fm();
            }
            if self.prefix(b"/ItalicAngle") {
                return self.t1_modify_italic();
            }
        }
        if self.prefix(b"/FontType") {
            let (f, _) = self.t1_scan_num(9)?;
            let i = ftoi(f);
            if i != 1 {
                let mut m = b"Type".to_vec();
                push_i64(&mut m, i as i64);
                m.extend_from_slice(b" fonts unsupported by pdfTeX");
                return Err(FontError::new(&m));
            }
            return Ok(());
        }
        let mut k = 0;
        while k < crate::FONT_KEYS_NUM {
            let t1name = crate::T1_NAMES[k];
            if !t1name.is_empty() && str_prefix(&self.line, 1, t1name) {
                break;
            }
            k += 1;
        }
        if k == crate::FONT_KEYS_NUM {
            return Ok(());
        }
        let mut p = crate::T1_NAMES[k].len() + 1;
        if at(&self.line, p) == b' ' {
            p += 1;
        }
        if k == FONTNAME_CODE {
            if at(&self.line, p) != b'/' {
                return Err(self.fail_line(b"a name expected: `", b"'"));
            }
            p += 1;
            let r = p; // skip the slash
            let mut q = p;
            while q < self.line.len() && self.line[q] != b' ' && self.line[q] != 10 {
                q += 1;
            }
            // xstrdup of a C string: an embedded NUL ends the name.
            let mut name = cstr(&self.line[..q], p).to_vec();
            let slant = self.inp.slant;
            let extend = self.inp.extend;
            if slant != 0 {
                name.extend_from_slice(b"-Slant_");
                push_i64(&mut name, slant as i32 as i64);
            }
            if extend != 0 {
                name.extend_from_slice(b"-Extend_");
                push_i64(&mut name, extend as i32 as i64);
            }
            self.fontname = name;
            // At this moment we cannot call make_subset_tag yet, as the encoding is not read; so
            // mark the offset of the tag and write it later.
            if self.inp.subsetted {
                self.carry.fontname_offset = self.offset() as i64 + r as i64;
                self.use_fontname_offset = Some(self.carry.fontname_offset);
                let tail: Vec<u8> = cstr(&self.line, q).to_vec();
                let mut l: Vec<u8> = self.line[..r].to_vec();
                l.extend_from_slice(b"ABCDEF+");
                l.extend_from_slice(&self.fontname);
                l.extend_from_slice(&tail);
                self.set_line(&l);
                self.lp = self.eol(r);
            }
            return Ok(());
        }
        if (k == STEMV_CODE || k == FONTBBOX1_CODE) && (at(&self.line, p) == b'[' || at(&self.line, p) == b'{') {
            p += 1;
        }
        if k == FONTBBOX1_CODE {
            for i in 0..4 {
                let (f, r) = self.t1_scan_num(p)?;
                self.dims[k + i].val = ftoi(f);
                self.dims[k + i].set = true;
                p = r;
            }
            return Ok(());
        }
        let (f, _) = self.t1_scan_num(p)?;
        self.dims[k].val = ftoi(f);
        self.dims[k].set = true;
        Ok(())
    }

    fn t1_scan_param(&mut self) -> R<()> {
        if !self.t1_scan || at(&self.line, 0) != b'/' {
            return Ok(());
        }
        if self.prefix(b"/lenIV") {
            let (f, _) = self.t1_scan_num(6)?;
            self.t1_len_iv = ftoi(f) as i16;
            if self.t1_len_iv < 0 {
                return Err(fail(b"negative value of lenIV is not supported"));
            }
            return Ok(());
        }
        self.t1_scan_keys()
    }

    /// `t1_builtin_enc`, for the case where `/Encoding` is the prefix of the line.
    fn t1_builtin_enc(&mut self) -> R<GlyphNames> {
        let mut names = GlyphNames::new();
        if self.suffix(b"def") {
            // predefined encoding
            let mut sc = Scan::new(&self.line, 9);
            if let Some((a, b)) = sc.word(255) {
                if &self.line[a..b] == b"StandardEncoding" {
                    self.enc_standard = true;
                    for (i, n) in STANDARD_GLYPH_NAMES.iter().enumerate() {
                        if !n.is_empty() {
                            names.set(i, n);
                        }
                    }
                    return Ok(names);
                }
                let mut m = b"cannot subset font (unknown predefined encoding `".to_vec();
                m.extend_from_slice(cstr(&self.line[..b], a));
                m.extend_from_slice(b"')");
                return Err(FontError::new(&m));
            }
            return Err(FontError::new(b"cannot subset font (unknown predefined encoding `')"));
        }
        // Two forms:  /Encoding [/a /b /c...] readonly def
        // and         /Encoding 256 array 0 1 255 {1 index exch /.notdef put} for
        //             dup 0 /x put
        //             ...
        //             readonly def
        self.enc_standard = false;
        if self.prefix(b"/Encoding [") || self.prefix(b"/Encoding[") {
            let mut counter = 0;
            let mut r = strchr(&self.line, 0, b'[').expect("the prefix has one") + 1;
            if at(&self.line, r) == b' ' {
                r += 1;
            }
            loop {
                while at(&self.line, r) == b'/' {
                    r += 1;
                    let start = r;
                    loop {
                        let c = at(&self.line, r);
                        if c == 32 || c == 10 || c == b']' || c == b'/' || r >= self.line.len() {
                            break;
                        }
                        r += 1;
                    }
                    let name = cstr(&self.line[..r], start);
                    let is_notdef = name == NOTDEF;
                    let name_end = start + name.len();
                    if at(&self.line, r) == b' ' {
                        r += 1;
                    }
                    if counter > 255 {
                        return Err(fail(b"encoding vector contains more than 256 names"));
                    }
                    if !is_notdef {
                        names.set(counter, &self.line[start..name_end]);
                    }
                    counter += 1;
                }
                let c = at(&self.line, r);
                if c != 10 && c != b'%' {
                    if str_prefix(&self.line, r, b"] def") || str_prefix(&self.line, r, b"] readonly def") {
                        break;
                    }
                    return Err(self.fail_line(b"a name or `] def' or `] readonly def' expected: `", b"'"));
                }
                self.t1_getline()?;
                r = 0;
            }
        } else {
            let mut p = match strchr(&self.line, 0, 10) {
                Some(p) => p,
                None => return Err(fail(b"unexpected end of file")),
            };
            loop {
                if at(&self.line, p) == 10 {
                    self.t1_getline()?;
                    p = 0;
                }
                // dup <index> <glyph> put
                if let Some((i, word)) = {
                    let mut sc = Scan::new(&self.line, p);
                    if sc.lit(b"dup ") {
                        match sc.int(0, usize::MAX) {
                            Some(i) => sc.word(255).map(|w| (i as i32, w)),
                            None => None,
                        }
                    } else {
                        None
                    }
                } {
                    if at(&self.line, word.0) == b'/' && (0..256).contains(&i) {
                        let name = cstr(&self.line[..word.1], word.0 + 1);
                        if name != NOTDEF {
                            let end = word.0 + 1 + name.len();
                            let name = &self.line[word.0 + 1..end];
                            names.set(i as usize, name);
                        }
                        match strstr(&self.line, p, b" put") {
                            Some(q) => p = q + 4,
                            None => return Err(fail(b"invalid pfb, no put found in dup")),
                        }
                        if at(&self.line, p) == b' ' {
                            p += 1;
                        }
                        continue;
                    }
                }
                // dup dup <to> exch <from> get put
                let ba = {
                    let mut sc = Scan::new(&self.line, p);
                    if sc.lit(b"dup dup ") {
                        match sc.int(0, usize::MAX) {
                            Some(b) if sc.lit(b" exch ") => sc.int(0, usize::MAX).map(|a| (b as i32, a as i32)),
                            _ => None,
                        }
                    } else {
                        None
                    }
                };
                if let Some((b, a)) = ba {
                    if (0..256).contains(&a) && (0..256).contains(&b) {
                        names.copy(a as usize, b as usize);
                        match strstr(&self.line, p, b" get put") {
                            Some(q) => p = q + 8,
                            None => return Err(fail(b"invalid pfb, no get put found in dup dup")),
                        }
                        if at(&self.line, p) == b' ' {
                            p += 1;
                        }
                        continue;
                    }
                }
                // dup dup <from> <size> getinterval <to> exch putinterval
                let acb = {
                    let mut sc = Scan::new(&self.line, p);
                    if sc.lit(b"dup dup ") {
                        match sc.int(0, usize::MAX) {
                            Some(a) if sc.lit(b" ") => match sc.int(0, usize::MAX) {
                                Some(c) if sc.lit(b" getinterval ") => {
                                    sc.int(0, usize::MAX).map(|b| (a as i32, c as i32, b as i32))
                                }
                                _ => None,
                            },
                            _ => None,
                        }
                    } else {
                        None
                    }
                };
                if let Some((a, c, b)) = acb {
                    if (0..256).contains(&a) && (0..256).contains(&b) && (0..256).contains(&c) {
                        for i in 0..c {
                            // The C checks only the ends: past 255 it reads and writes beyond the
                            // array.  Nothing there is ever read back.
                            if a + i < 256 && b + i < 256 {
                                names.copy((a + i) as usize, (b + i) as usize);
                            }
                        }
                        match strstr(&self.line, p, b" putinterval") {
                            Some(q) => p = q + 12,
                            None => return Err(fail(b"invalid pfb, no putinterval found in dup dup")),
                        }
                        if at(&self.line, p) == b' ' {
                            p += 1;
                        }
                        continue;
                    }
                }
                // `def' or `readonly def'
                if (p == 0 || at(&self.line, p - 1) == b' ') && cstr(&self.line, p) == b"def\n" {
                    return Ok(names);
                }
                // skip an unrecognizable word
                loop {
                    let c = at(&self.line, p);
                    if c == b' ' || c == 10 || p >= self.line.len() {
                        break;
                    }
                    p += 1;
                }
                if p >= self.line.len() {
                    return Err(fail(b"unexpected end of file"));
                }
                if at(&self.line, p) == b' ' {
                    p += 1;
                }
            }
        }
        Ok(names)
    }

    fn t1_include(&mut self) -> R<()> {
        loop {
            self.t1_getline()?;
            self.t1_scan_param()?;
            self.t1_putline();
            if self.t1_in_eexec != 0 {
                break;
            }
        }
        self.t1_start_eexec()?;
        loop {
            self.t1_getline()?;
            self.t1_scan_param()?;
            self.t1_putline();
            if self.t1_charstrings() || self.t1_subrs() {
                break;
            }
        }
        self.t1_cs = true;
        loop {
            self.t1_getline()?;
            self.t1_putline();
            if self.t1_end_eexec() {
                break;
            }
        }
        self.t1_stop_eexec()
    }

    #[inline]
    fn check_subr(&self, subr: i64) -> R<()> {
        if subr >= self.subr_size as i64 || subr < 0 {
            let mut m = b"Subrs array: entry index out of range (".to_vec();
            push_i64(&mut m, subr as i32 as i64);
            m.push(b')');
            return Err(FontError::new(&m));
        }
        Ok(())
    }

    fn check_cs_token_pair(&self, data: usize, end: usize) -> Option<usize> {
        let d = &self.arena;
        for i in 0..CS_TOKEN_PAIRS.len() - 1 {
            if d[data..end].starts_with(CS_TOKEN_PAIRS[i]) && str_suffix(d, data, end, CS_TOKEN_PAIRS[i + 1]) {
                return Some(i);
            }
        }
        None
    }

    fn cs_store(&mut self, is_subr: bool) -> R<()> {
        let mut p = 0;
        while p < self.line.len() && self.line[p] != b' ' {
            p += 1;
        }
        let name_end = p;
        let mut slot = 0;
        if is_subr {
            let (f, _) = self.t1_scan_num(p + 1)?;
            let subr = ftoi(f);
            self.check_subr(subr as i64)?;
            slot = subr as usize;
        } else {
            if self.cs_tab.len() as i64 + 1 > self.cs_size as i64 {
                let mut m = b"CharStrings dict: more entries than dict size (".to_vec();
                push_i64(&mut m, self.cs_size as i64);
                m.push(b')');
                return Err(FontError::new(&m));
            }
        }
        let mut e = CS_EMPTY;
        if !is_subr {
            // strcmp and xstrdup of the token without its slash: an embedded NUL ends it.
            let name = cstr(&self.line[..name_end], 1);
            if name == NOTDEF {
                e.name_off = NAME_NOTDEF;
            } else {
                e.name_off = self.arena.len() as u32;
                e.name_len = name.len() as u32;
                let (off, len) = (1usize, name.len());
                self.arena.extend_from_slice(&self.line[off..off + len]);
            }
        }
        let cslen = self.t1_cslen as usize;
        let data = self.arena.len();
        // " RD " and the charstring data
        let (a, b) = (self.cs_start - 4, self.cs_start + cslen);
        if b > self.line.len() {
            return Err(fail(b"unexpected end of file"));
        }
        self.arena.extend_from_slice(&self.line[a..b]);
        // the end of the line
        let mut q = b;
        while q < self.line.len() && self.line[q] != 10 {
            q += 1;
        }
        self.arena.extend_from_slice(&self.line[b..q.min(self.line.len())]);
        self.arena.push(10);
        let end = self.arena.len();
        if is_subr && self.cs_token_pair.is_none() {
            self.cs_token_pair = self.check_cs_token_pair(data, end);
        }
        e.data = data as u32;
        e.len = (end - data) as u16;
        e.cslen = self.t1_cslen;
        e.valid = true;
        e.used = false;
        if is_subr {
            // An entry already marked keeps `used`: nothing is marked yet while reading.
            let n = self.subr_tab[slot];
            e.used = n.used;
            self.subr_tab[slot] = e;
        } else {
            self.cs_tab.push(e);
        }
        Ok(())
    }

    #[inline]
    fn name_of(&self, e: &CsEntry) -> &[u8] {
        if e.name_off == NAME_NOTDEF {
            NOTDEF
        } else {
            &self.arena[e.name_off as usize..(e.name_off + e.name_len) as usize]
        }
    }

    fn cs_fail(&self, cs_name: Option<&[u8]>, subr: i32, msg: &[u8]) -> FontError {
        let mut m = Vec::new();
        match cs_name {
            None => {
                m.extend_from_slice(b"Subr (");
                push_i64(&mut m, subr as i64);
                m.extend_from_slice(b"): ");
            }
            Some(n) => {
                m.extend_from_slice(b"CharString (/");
                m.extend_from_slice(n);
                m.extend_from_slice(b"): ");
            }
        }
        m.extend_from_slice(msg);
        FontError::new(&m)
    }

    fn stack_error(&self, n: i64) -> FontError {
        let mut m = b"CharString: invalid access (".to_vec();
        push_i64(&mut m, n as i32 as i64);
        m.extend_from_slice(b") to stack (");
        push_i64(&mut m, self.carry.stack_ptr as i64);
        m.extend_from_slice(b" entries)");
        FontError::new(&m)
    }

    /// `cc_pop`, in which a negative count raises the stack.
    #[inline]
    fn cc_pop(&mut self, n: i64) -> R<()> {
        if n < 0 {
            self.use_clean = false; // what lies above the stack was left there
        }
        if (self.carry.stack_ptr as i64) < n {
            return Err(self.stack_error(n));
        }
        let sp = self.carry.stack_ptr as i64 - n;
        if sp > CC_STACK_SIZE as i64 {
            return Err(self.stack_error(n));
        }
        self.carry.stack_ptr = sp as usize;
        Ok(())
    }

    /// `cc_get(-k)`.  An index below the stack reads what lies before it in C: never a valid font.
    #[inline]
    fn cc_top(&self, k: usize) -> i64 {
        if self.carry.stack_ptr >= k {
            self.carry.cc_stack[self.carry.stack_ptr - k]
        } else {
            0
        }
    }

    #[inline]
    fn cc_push(&mut self, v: i64) -> R<()> {
        if self.carry.stack_ptr >= CC_STACK_SIZE {
            return Err(self.stack_error(self.carry.stack_ptr as i64));
        }
        self.carry.cc_stack[self.carry.stack_ptr] = v;
        self.carry.stack_ptr += 1;
        self.use_cc_hi = self.use_cc_hi.max(self.carry.stack_ptr);
        Ok(())
    }

    /// Fixes a return-less subr by appending `CS_RETURN`.
    fn append_cs_return(&mut self, idx: usize) {
        let e = self.subr_tab[idx];
        let start = e.data as usize;
        let cslen = e.cslen as usize;
        let new_data = self.arena.len();
        // decrypt the data, append CS_RETURN, encrypt again; the first four bytes are " RD ".
        self.arena.extend_from_within(start..start + 4);
        let mut dr: u16 = 4330;
        let mut er: u16 = 4330;
        for i in 0..cslen {
            let plain = cdecrypt(self.arena[start + 4 + i], &mut dr);
            let c = cencrypt(plain, &mut er);
            self.arena.push(c);
        }
        let c = cencrypt(CS_RETURN as u8, &mut er);
        self.arena.push(c);
        self.arena.extend_from_within(start + 4 + cslen..start + e.len as usize);
        let e = &mut self.subr_tab[idx];
        e.data = new_data as u32;
        e.len = e.len.wrapping_add(1);
        e.cslen = e.cslen.wrapping_add(1);
    }

    /// `cs_mark`: `cs_name` None is `mark_subr(subr)`.
    fn cs_mark(&mut self, cs_name: Option<&[u8]>, subr: i32, depth: u32) -> R<()> {
        if depth > CS_MARK_DEPTH {
            return Err(self.cs_fail(cs_name, subr, b"subroutines nested too deeply"));
        }
        let mut idx = 0;
        match cs_name {
            None => {
                self.check_subr(subr as i64)?;
                idx = subr as usize;
                let e = &self.subr_tab[idx];
                // only marked CharString entries and invalid entries can be skipped;
                // valid marked subrs must be parsed to keep the stack in sync
                if !e.valid {
                    return Ok(());
                }
            }
            Some(name) => {
                if self.cs_notdef.is_some() && name == NOTDEF {
                    idx = self.cs_notdef.expect("just tested");
                } else {
                    let n = self.cs_tab.len();
                    let mut found = false;
                    while idx < n {
                        let e = self.cs_tab[idx];
                        if self.name_of(&e) == name {
                            found = true;
                            break;
                        }
                        idx += 1;
                    }
                    if !found {
                        let mut m = b"glyph `".to_vec();
                        m.extend_from_slice(name);
                        m.extend_from_slice(b"' undefined");
                        self.warn(m);
                        return Ok(());
                    }
                    if self.cs_tab[idx].name_off == NAME_NOTDEF {
                        self.cs_notdef = Some(idx);
                    }
                }
                let e = &self.cs_tab[idx];
                if !e.valid || e.used {
                    return Ok(());
                }
            }
        }
        let e = if cs_name.is_some() {
            self.cs_tab[idx].used = true;
            self.cs_tab[idx]
        } else {
            self.subr_tab[idx].used = true;
            self.subr_tab[idx]
        };
        let mut cr: u16 = 4330;
        let mut cs_len = e.cslen as i32;
        let mut data = e.data as usize + 4;
        let mut last_cmd = 0;
        macro_rules! cs_getchar {
            () => {{
                let b = at(&self.arena, data);
                data += 1;
                cdecrypt(b, &mut cr)
            }};
        }
        for _ in 0..self.t1_len_iv {
            let _ = cs_getchar!();
            cs_len -= 1;
        }
        while cs_len > 0 {
            cs_len -= 1;
            let mut b = cs_getchar!() as usize;
            if b >= 32 {
                let a: i64;
                if b <= 246 {
                    a = b as i64 - 139;
                } else if b <= 250 {
                    cs_len -= 1;
                    a = (((b - 247) << 8) + 108) as i64 + cs_getchar!() as i64;
                } else if b <= 254 {
                    cs_len -= 1;
                    a = -(((b - 251) << 8) as i64) - 108 - cs_getchar!() as i64;
                } else {
                    cs_len -= 4;
                    let b3 = cs_getchar!();
                    let b2 = cs_getchar!();
                    let b1 = cs_getchar!();
                    let b0 = cs_getchar!();
                    a = i32::from_be_bytes([b3, b2, b1, b0]) as i64;
                }
                self.cc_push(a)?;
            } else {
                if b == CS_ESCAPE {
                    b = cs_getchar!() as usize + CS_1BYTE_MAX;
                    cs_len -= 1;
                }
                if b >= CS_MAX {
                    let mut m = b"command value out of range: ".to_vec();
                    push_i64(&mut m, b as i64);
                    return Err(self.cs_fail(cs_name, subr, &m));
                }
                let cc = CC_TAB[b];
                if !cc.valid {
                    let mut m = b"command not valid: ".to_vec();
                    push_i64(&mut m, b as i64);
                    return Err(self.cs_fail(cs_name, subr, &m));
                }
                if cc.bottom {
                    let n = self.carry.stack_ptr as i64;
                    if n < cc.nargs as i64 {
                        let mut m = b"less arguments on stack (".to_vec();
                        push_i64(&mut m, n);
                        m.extend_from_slice(b") than required (");
                        push_i64(&mut m, cc.nargs as i64);
                        m.push(b')');
                        return Err(self.cs_fail(cs_name, subr, &m));
                    } else if n > cc.nargs as i64 {
                        let mut m = b"more arguments on stack (".to_vec();
                        push_i64(&mut m, n);
                        m.extend_from_slice(b") than required (");
                        push_i64(&mut m, cc.nargs as i64);
                        m.push(b')');
                        return Err(self.cs_fail(cs_name, subr, &m));
                    }
                }
                last_cmd = b;
                match b {
                    CS_CALLSUBR => {
                        let a1 = self.cc_top(1);
                        self.cc_pop(1)?;
                        self.cs_mark(None, a1 as i32, depth + 1)?;
                        // check_subr looked at the int; subr_tab[a1] is indexed by the long.
                        if a1 < 0 || a1 >= self.subr_size as i64 || !self.subr_tab[a1 as usize].valid {
                            let mut m = b"cannot call subr (".to_vec();
                            push_i64(&mut m, a1 as i32 as i64);
                            m.push(b')');
                            return Err(self.cs_fail(cs_name, subr, &m));
                        }
                    }
                    CS_DIV => {
                        self.cc_pop(2)?;
                        self.cc_push(0)?;
                    }
                    CS_CALLOTHERSUBR => {
                        if self.cc_top(1) == 3 {
                            self.carry.last_arg_other_subr3 = self.cc_top(3);
                            self.use_last_arg_out = Some(self.carry.last_arg_other_subr3);
                        }
                        let a1 = self.cc_top(2) + 2;
                        self.cc_pop(a1)?;
                    }
                    CS_POP => {
                        // The only case when we care about the value pushed is when POP follows
                        // CALLOTHERSUBR (changing hints by OtherSubrs[3]).
                        let v = self.carry.last_arg_other_subr3;
                        if self.use_last_arg_out.is_none() && self.use_last_arg_in.is_none() {
                            self.use_last_arg_in = Some(v);
                        }
                        self.cc_push(v)?;
                    }
                    CS_SEAC => {
                        let a1 = self.carry.cc_stack[3];
                        let a2 = self.carry.cc_stack[4];
                        self.carry.stack_ptr = 0;
                        if !(0..256).contains(&a1) || !(0..256).contains(&a2) {
                            // standard_glyph_names[] out of bounds in C
                            return Err(self.cs_fail(cs_name, subr, b"seac: character code out of range"));
                        }
                        let n1 = STANDARD_GLYPH_NAMES[a1 as usize];
                        let n2 = STANDARD_GLYPH_NAMES[a2 as usize];
                        let n1 = if n1.is_empty() { NOTDEF } else { n1 };
                        let n2 = if n2.is_empty() { NOTDEF } else { n2 };
                        self.cs_mark(Some(n1), 0, depth + 1)?;
                        self.cs_mark(Some(n2), 0, depth + 1)?;
                        // base and accent characters are needed in CharSet
                        self.gl.insert(n1);
                        self.gl.insert(n2);
                    }
                    _ => {
                        if cc.clear {
                            self.carry.stack_ptr = 0;
                        }
                    }
                }
            }
        }
        if cs_name.is_none() && last_cmd != CS_RETURN {
            let mut m = b"last command in subr `".to_vec();
            push_i64(&mut m, subr as i64);
            m.extend_from_slice(b"' is not a RETURN; I will add it now but please consider fixing the font");
            self.warn(m);
            self.append_cs_return(idx);
        }
        Ok(())
    }

    fn t1_subset_ascii_part(&mut self) -> R<GlyphNames> {
        self.t1_getline()?;
        while !self.prefix(b"/Encoding") {
            self.t1_scan_param()?;
            let skip = self.prefix(b"/UniqueID") && {
                let l = strlen(&self.line, 0);
                l >= 4 && &self.line[l - 4..l - 1] == b"def"
            };
            if !skip {
                self.t1_putline();
            }
            self.t1_getline()?;
        }
        let glyph_names = self.t1_builtin_enc()?;
        // take over collected non-reencoded characters from TeX
        if let Some(tx) = self.inp.tx_codes {
            for c in 0..256 {
                if tx.contains(c) {
                    self.gl.insert(glyph_names.name(c));
                }
            }
        }
        let (tag, warning, round) = make_subset_tag_round(self.gl, &self.fontname, self.inp.used_tags);
        self.subset_tag = Some(tag);
        if round != 1 {
            self.use_clean = false; // the tag says what else was there
        }
        if let Some(w) = warning {
            self.warn(w);
        }
        if self.carry.fontname_offset == 0 {
            return Err(FontError::assertion(WRITET1_C, 1353, "t1_subset_ascii_part", "t1_fontname_offset != 0"));
        }
        // strncpy(fb_array + t1_fontname_offset, subset_tag, 6).  A stale offset from an earlier font
        // can point past the output so far, where the C's bytes are later overwritten.
        let at_off = self.base + self.carry.fontname_offset as usize;
        for (i, &b) in tag.iter().enumerate() {
            if at_off + i < self.out.len() {
                self.out[at_off + i] = b;
            }
        }
        // now really all glyphs needed from this font are in gl
        if self.enc_standard {
            self.t1_puts(b"/Encoding StandardEncoding def\n");
        } else {
            self.t1_puts(b"/Encoding 256 array\n0 1 255 {1 index exch /.notdef put} for\n");
            // create_t1_glyph_tree: per name, the first code that has it, sorted by name
            let mut order: Vec<u16> = Vec::with_capacity(256);
            for i in 0..256 {
                if !glyph_names.is_notdef(i) {
                    order.push(i as u16);
                }
            }
            order.sort_by(|&a, &b| {
                glyph_names.name(a as usize).cmp(glyph_names.name(b as usize)).then(a.cmp(&b))
            });
            let mut j = 0;
            let mut l: Vec<u8> = Vec::with_capacity(64);
            let mut walk = 0;
            for g in 0..self.gl.len() {
                let glyph = self.gl.get(g);
                while walk < order.len() && glyph_names.name(order[walk] as usize) < glyph {
                    walk += 1;
                }
                if walk < order.len() && glyph_names.name(order[walk] as usize) == glyph {
                    l.clear();
                    l.extend_from_slice(b"dup ");
                    push_i64(&mut l, order[walk] as i64);
                    l.extend_from_slice(b" /");
                    l.extend_from_slice(glyph);
                    l.extend_from_slice(b" put\n");
                    let t = core::mem::take(&mut l);
                    self.t1_puts(&t);
                    l = t;
                    j += 1;
                }
            }
            if j == 0 {
                // We didn't mark anything for the Encoding array.  We add "dup 0 /.notdef put" for
                // compatibility with Acrobat 5.0.
                self.t1_puts(b"dup 0 /.notdef put\n");
            }
            self.t1_puts(b"readonly def\n");
        }
        loop {
            self.t1_getline()?;
            self.t1_scan_param()?;
            if !self.prefix(b"/UniqueID") {
                // ignore UniqueID for subsetted fonts
                self.t1_putline();
            }
            if self.t1_in_eexec != 0 {
                break;
            }
        }
        Ok(glyph_names)
    }

    fn cs_init(&mut self) {
        self.cs_tab.clear();
        self.cs_dict_start.clear();
        self.cs_dict_end.clear();
        self.cs_count = 0;
        self.cs_size = 0;
        self.cs_size_pos = 0;
        self.cs_token_pair = None;
        self.subr_tab.clear();
        self.subr_tab_null = true;
        self.subr_array_start.clear();
        self.subr_array_end.clear();
        self.subr_max = 0;
        self.subr_size = 0;
        self.subr_size_pos = 0;
    }

    fn t1_read_subrs(&mut self) -> R<()> {
        self.t1_getline()?;
        while !(self.t1_charstrings() || self.t1_subrs()) {
            self.t1_scan_param()?;
            if !self.prefix(b"/UniqueID") {
                self.t1_putline();
            }
            self.t1_getline()?;
        }
        loop {
            // found:
            self.t1_cs = true;
            self.t1_scan = false;
            if !self.t1_subrs() {
                return Ok(());
            }
            self.subr_size_pos = 6 + 1;
            // subr_size_pos points to the number indicating dict size after "/Subrs"
            let (f, _) = self.t1_scan_num(self.subr_size_pos)?;
            self.subr_size = ftoi(f);
            if self.subr_size == 0 {
                while !self.t1_charstrings() {
                    self.t1_getline()?;
                }
                return Ok(());
            }
            if self.subr_size < 0 || self.subr_size as i64 > (1 << 24) {
                // xtalloc of that many entries: the C's malloc gives up, or the machine does.
                let mut m = b"Subrs array: entry index out of range (".to_vec();
                push_i64(&mut m, self.subr_size as i64);
                m.push(b')');
                return Err(FontError::new(&m));
            }
            self.subr_tab.clear();
            self.subr_tab.resize(self.subr_size as usize, CS_EMPTY);
            self.subr_tab_null = false;
            self.subr_array_start = cstr(&self.line, 0).to_vec();
            self.t1_getline()?;
            while self.t1_cslen != 0 {
                self.cs_store(true)?;
                self.t1_getline()?;
            }
            // mark the first four entries without parsing
            let mut i = 0;
            while i < self.subr_size as usize && i < 4 {
                self.subr_tab[i].used = true;
                i += 1;
            }
            // The end of the Subrs array might have more than one line so we need to concatenate
            // them to subr_array_end.  Unfortunately some fonts don't have the Subrs array followed
            // by the CharStrings dict immediately (synthetic fonts).  If we cannot find CharStrings
            // in next POST_SUBRS_SCAN lines then we will treat the font as synthetic and ignore
            // everything until next Subrs is found.
            const POST_SUBRS_SCAN: i32 = 5;
            let mut end: Vec<u8> = Vec::new();
            let mut i = 0;
            while i < POST_SUBRS_SCAN {
                if self.t1_charstrings() {
                    break;
                }
                end.extend_from_slice(cstr(&self.line, 0));
                self.t1_getline()?;
                i += 1;
            }
            self.subr_array_end = end;
            if i == POST_SUBRS_SCAN {
                // CharStrings not found; suppose synthetic font
                self.cs_init();
                self.t1_cs = false;
                self.t1_synthetic = true;
                while !(self.t1_charstrings() || self.t1_subrs()) {
                    self.t1_getline()?;
                }
                continue; // goto found
            }
            return Ok(());
        }
    }

    /// Makes the line `first` with its size number replaced by `count`, `t1_flush_cs`'s first step.
    fn flush_start_line(&mut self, start_line: &[u8], size_pos: usize, count: u32) {
        let mut l: Vec<u8> = Vec::with_capacity(start_line.len() + 12);
        let mut p = 0;
        while p < size_pos {
            l.push(at(start_line, p));
            p += 1;
        }
        while at(start_line, p).is_ascii_digit() {
            p += 1;
        }
        push_u64(&mut l, count as u64);
        l.extend_from_slice(cstr(start_line, p));
        self.set_line(&l);
        self.lp = self.eol(0);
        self.t1_putline();
    }

    /// `sprintf(t1_line_array, prefix); memcpy(.., data, len); t1_line_ptr = ..; t1_putline()`.
    fn put_entry_line(&mut self, e: &CsEntry) {
        let len = e.len as usize;
        let need = self.lp + len + 1;
        if need > self.line.len() {
            self.line.resize(need, 0);
        }
        let start = e.data as usize;
        if len > 0 {
            self.line[self.lp..self.lp + len].copy_from_slice(&self.arena[start..start + len]);
        }
        self.lp += len;
        self.t1_putline();
    }

    /// `t1_subr_flush`.
    fn t1_subr_flush(&mut self) -> R<()> {
        let count = self.subr_max + 1;
        let start = core::mem::take(&mut self.subr_array_start);
        self.flush_start_line(&start, self.subr_size_pos, count as u32);

        // create return_cs to replace unused subrs
        let mut cr: u16 = 4330;
        let mut return_cs: Vec<u8> = Vec::with_capacity(self.t1_len_iv as usize + 1);
        for _ in 0..self.t1_len_iv {
            return_cs.push(cencrypt(0, &mut cr));
        }
        return_cs.push(cencrypt(CS_RETURN as u8, &mut cr));
        let cs_len = return_cs.len() as u16;

        let pair = self.cs_token_pair.expect("t1_subset_charstrings checks it");
        let pair0 = CS_TOKEN_PAIRS[pair];
        let pair1 = CS_TOKEN_PAIRS[pair + 1];
        let mut l: Vec<u8> = Vec::with_capacity(64);
        for k in 0..count.max(0) as usize {
            let e = self.subr_tab[k];
            l.clear();
            l.extend_from_slice(b"dup ");
            push_u64(&mut l, k as u64);
            l.push(b' ');
            if e.used {
                push_u64(&mut l, e.cslen as u64);
                self.set_line(&l);
                self.lp = l.len();
                self.put_entry_line(&e);
            } else {
                // replace unused subrs by return_cs
                push_u64(&mut l, cs_len as u64);
                l.extend_from_slice(pair0);
                l.push(b' ');
                l.extend_from_slice(&return_cs);
                self.set_line(&l);
                self.lp = l.len();
                self.t1_putline();
                l.clear();
                l.push(b' ');
                l.extend_from_slice(pair1);
                self.set_line(&l);
                self.lp = self.eol(0);
                self.t1_putline();
            }
        }
        let end = core::mem::take(&mut self.subr_array_end);
        self.set_line(cstr(&end, 0));
        self.lp = self.eol(0);
        self.t1_putline();
        Ok(())
    }

    /// `t1_cs_flush`.
    fn t1_cs_flush(&mut self) {
        let start = core::mem::take(&mut self.cs_dict_start);
        self.flush_start_line(&start, self.cs_size_pos, self.cs_count as u32);
        let mut l: Vec<u8> = Vec::with_capacity(64);
        for k in 0..self.cs_tab.len() {
            let e = self.cs_tab[k];
            if e.used {
                l.clear();
                l.push(b'/');
                l.extend_from_slice(self.name_of(&e));
                l.push(b' ');
                push_u64(&mut l, e.cslen as u64);
                self.set_line(&l);
                self.lp = l.len();
                self.put_entry_line(&e);
            }
        }
        let end = core::mem::take(&mut self.cs_dict_end);
        self.set_line(cstr(&end, 0));
        self.lp = self.eol(0);
        self.t1_putline();
    }

    fn t1_mark_glyphs(&mut self) -> R<()> {
        if self.t1_synthetic || self.inp.all_glyphs {
            // mark everything
            for e in self.cs_tab.iter_mut() {
                if e.valid {
                    e.used = true;
                }
            }
            if !self.subr_tab_null {
                for e in self.subr_tab.iter_mut() {
                    if e.valid {
                        e.used = true;
                    }
                }
                self.subr_max = self.subr_size - 1;
            }
            return Ok(());
        }
        self.cs_mark(Some(NOTDEF), 0, 0)?;
        // avl_t_first, avl_t_next: seac adds to the tree under the traverser, which carries on with
        // the successor in the changed tree.
        let mut cur: Vec<u8> = Vec::with_capacity(64);
        let mut i = 0;
        while i < self.gl.len() {
            cur.clear();
            cur.extend_from_slice(self.gl.get(i));
            let before = self.gl.len();
            self.cs_mark(Some(&cur), 0, 0)?;
            if self.gl.len() != before {
                i = self.gl.position(&cur).expect("marking only adds");
            }
            i += 1;
        }
        if !self.subr_tab_null {
            self.subr_max = -1;
            for (k, e) in self.subr_tab.iter().enumerate() {
                if e.used && k as i32 > self.subr_max {
                    self.subr_max = k as i32;
                }
            }
        }
        Ok(())
    }

    fn t1_check_unusual_charstring(&mut self) -> R<()> {
        let p = strstr(&self.line, 0, b"/CharStrings").expect("the caller found it") + 12;
        // if no number follows "/CharStrings", let's read the next line
        let mut sc = Scan::new(&self.line, p);
        if sc.int(0, usize::MAX).is_none() {
            let mut b = cstr(&self.line, 0).to_vec();
            // t1_getline always appends EOL; change it to space before appending the next line
            if let Some(last) = b.last_mut() {
                *last = b' ';
            }
            self.t1_getline()?;
            b.extend_from_slice(cstr(&self.line, 0));
            self.set_line(&b);
            self.lp = self.eol(0);
        }
        Ok(())
    }

    fn t1_subset_charstrings(&mut self) -> R<()> {
        // At this point the line contains "/CharStrings".  When we hit a case like this:
        //   dup/CharStrings
        //   229 dict dup begin
        // we read the next line and concatenate before moving on.
        self.t1_check_unusual_charstring()?;
        // A NUL right after the name can take it away in t1_check_unusual_charstring: the C then goes
        // on from a null pointer, and crashes.
        self.cs_size_pos = match strstr(&self.line, 0, b"/CharStrings") {
            Some(at) => at + 12 + 1,
            None => return Err(self.fail_line(b"a number expected: `", b"'")),
        };
        // cs_size_pos points to the number indicating dict size after "/CharStrings"
        let (f, _) = self.t1_scan_num(self.cs_size_pos)?;
        self.cs_size = ftoi(f);
        self.cs_tab.clear();
        self.cs_notdef = None;
        self.cs_dict_start = cstr(&self.line, 0).to_vec();
        self.t1_getline()?;
        while self.t1_cslen != 0 {
            self.cs_store(false)?;
            self.t1_getline()?;
        }
        self.cs_dict_end = cstr(&self.line, 0).to_vec();
        self.t1_mark_glyphs()?;
        if !self.subr_tab_null {
            if self.cs_token_pair.is_none() {
                return Err(fail(b"This Type 1 font uses mismatched subroutine begin/end token pairs."));
            }
            self.t1_subr_flush()?;
        }
        self.cs_count = self.cs_tab.iter().filter(|e| e.used).count() as i32;
        self.t1_cs_flush();
        Ok(())
    }

    fn t1_subset_end(&mut self) -> R<()> {
        if self.t1_synthetic {
            // copy to "dup /FontName get exch definefont pop"
            while strstr(&self.line, 0, b"definefont").is_none() {
                self.t1_getline()?;
                self.t1_putline();
            }
            while !self.t1_end_eexec() {
                self.t1_getline()?; // ignore the rest
            }
            self.t1_putline(); // write "mark currentfile closefile"
        } else {
            while !self.t1_end_eexec() {
                // copy to "mark currentfile closefile"
                self.t1_getline()?;
                self.t1_putline();
            }
        }
        self.t1_stop_eexec()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cc_tab_matches_the_c() {
        assert_eq!(CS_MAX, 66);
        assert_eq!(CS_SEAC, 38);
        assert_eq!(CS_POP, 49);
        assert!(CC_TAB[CS_SEAC].valid && CC_TAB[CS_SEAC].bottom && CC_TAB[CS_SEAC].nargs == 5);
        assert!(!CC_TAB[CS_ESCAPE].valid);
        assert!(!CC_TAB[2].valid);
    }

    #[test]
    fn standard_names() {
        assert_eq!(STANDARD_GLYPH_NAMES[65], b"A");
        assert_eq!(STANDARD_GLYPH_NAMES[0x27], b"quoteright");
        assert_eq!(STANDARD_GLYPH_NAMES[0xfb], b"germandbls");
        assert_eq!(STANDARD_GLYPH_NAMES[0xc1], b"grave");
        assert_eq!(STANDARD_GLYPH_NAMES[0xd0], b"emdash");
        assert_eq!(STANDARD_GLYPH_NAMES[0xe1], b"AE");
        assert_eq!(STANDARD_GLYPH_NAMES[0xf1], b"ae");
        assert!(STANDARD_GLYPH_NAMES[0x7f].is_empty());
    }

    #[test]
    fn crypt_round_trip() {
        let mut a = 4330u16;
        let mut b = 4330u16;
        for p in 0..=255u8 {
            let c = cencrypt(p, &mut a);
            assert_eq!(cdecrypt(c, &mut b), p);
        }
    }

    #[test]
    fn empty_input_fails_like_the_c() {
        let mut dims = crate::new_font_dims();
        let mut gl = GlyphSet::new();
        let mut carry = T1Carry::default();
        let mut out = Vec::new();
        let inp = T1Input {
            font: b"",
            subsetted: true,
            all_glyphs: false,
            slant: 0,
            extend: 0,
            fontname: b"x",
            tx_codes: None,
            used_tags: &[],
        };
        let e = write_t1(&inp, &mut dims, &mut gl, &mut carry, &mut out).err().expect("fails");
        assert_eq!(e.message, b"unexpected end of file");
    }
}
