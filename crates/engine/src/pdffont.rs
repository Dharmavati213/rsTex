//! §703-726 Part 32e: font processing, and the C side of it that is not in `crates/fontfile`: `mapfile.c`'s
//! trees and `lookup_fontmap`, `writefont.c`, `writeenc.c`, `vfpacket.c`, the top of `subfont.c`, the
//! surroundings of `tounicode.c`, and the C wrappers around `writet1`, `writettf` and `writeotf`.
//!
//! # Where the C's statics went
//!
//! Everything that lives while the document is read is in the image, so it survives a snapshot
//! (docs/PORTING.md, "the rule"): `Vars` holds `PodVec`s in the arena.
//!
//! | C | Here |
//! |---|---|
//! | `fm_entry`, `tfm_tree`, `ps_tree` | `fm_tab`, and two open-addressing tables `tfm_hash`, `ps_hash` |
//! | `ff_tree`, `sfd_tree`, `fe_tree` | `ff_tab`, `sfd_tab` with `sf_tab`, `fe_tab`, searched in a line: they are short |
//! | `fd_tree` | `fd_tab`, `in_tree`.  What the document can reach is `FdRec`; the rest is `Fd` in `HVars`, below |
//! | `vf_array`, `packet_array` | `vf_tab` with `vf_pack`, `packet_stack` |
//! | `glyph_unicode_tree` | `gtu_log`: the pairs as given, replayed into `HVars::gtu` where a fresh `Host` lacks them |
//! | every `char *` | a `Span` of `pool` |
//!
//! `tfm_tree` and `ps_tree` are never walked.  `lookup_fontmap` looks like it walks `ps_tree`, but the keys of
//! that tree are unique, so its two loops stop after the first entry.  So hashing them changes nothing.  They
//! hold thousands of entries, and only they need it.
//!
//! `fo_tree`, `cw_entry`, `st_tree`, `fb_array` and the glyph sets exist only once the fonts are being output, at
//! the end of the run, so they are plain Rust in `HVars`.  `fd_tab` is walked at the end in the order of `comp_fd_entry`, and
//! `fe_tab` and the font dictionaries likewise: sorting there is the in-order walk of the C's tree.
//!
//! `pdf_font_map[f]` is 0 for "not looked up", [`FM_DUMMY`] for `dummy_fm_entry()`, otherwise
//! `(index + 1) * 2`, plus 1 if the entry is a PK font.  `adv_char_width` asks `isscalable` per character, and
//! this answers without touching the arena.  `pdf_init_font` only copies and compares the value.
//!
//! # Other modules' use of this file
//!
//! `pdffont_lookup_fontmap`, `pdffont_epdf_create_fontdescriptor`, `pdffont_is_subsetable`,
//! `pdffont_epdf_mark_glyphs`, `pdffont_embed_whole_font`, `pdffont_get_fd_objnum` and `pdffont_get_fn_objnum` are
//! `epdf.c` and `lookup_fontmap`, for the replay of `crates/image`'s `Op::EpdfFont` and `Op::EpdfFontName`.
//!
//! DROPPED §707: types only.  `char_map_array` and `fm_entry_ptr` have no use here.
//! DROPPED §721-722: constants and a record, generated in consts.rs and state.rs.
#![forbid(unsafe_code)]
#![deny(unreachable_patterns, unused_variables)]
#![allow(non_snake_case, unused_mut, unused_imports, dead_code)]

use crate::access::*;
use crate::consts::*;
use crate::host::*;
use crate::platform;
use crate::raw::*;
use crate::state::*;

use rstex_fontfile as ff;
use rstex_fontfile::{
    CodeSet, FmEntry, FmMode, FontDims, FontError, GlyphNames, GlyphSet, GlyphUnicodeMap, MapType, T1Carry,
    TtfCmapCache,
};

type Arena = A<u64, ARENA_CAP>;

/// `pdf_font_map[f]` for `dummy_fm_entry()`.
const FM_DUMMY: i32 = -1;
/// `nontfm` in mapfile.c.
const NONTFM: &[u8] = b"<nontfm>";
const LINK_TFM: i32 = 1;
const LINK_PS: i32 = 2;
/// `non_existent_path`, §705: `font_area` of a font made by `\pdfcopyfont`.
const NON_EXISTENT_PATH: &[u8] = b"///...";
/// `PRINTF_BUF_SIZE`: what `vsnprintf` leaves of a formatted message.
const PRINTF_MAX: usize = 1023;
/// `SMALL_BUF_SIZE`.
const SMALL_BUF_SIZE: usize = 256;
const SUBFONT_C: &[u8] = b"../../../texk/web2c/pdftexdir/subfont.c";
/// How deep `push_packet_state` goes: §725 stops at `vf_max_recursion` first.
const PACKET_STACK_CAP: usize = 16;

crate::pod_struct! {
    /// A `char *` into `pool`.  `len` -1 is NULL.
    pub struct Span {
        pub off: i32,
        pub len: i32,
    }
}

const NO_SPAN: Span = Span { off: 0, len: -1 };

crate::pod_struct! {
    /// `fm_entry`.
    pub struct FmRec {
        pub tfm_name: Span,
        pub ps_name: Span,
        pub ff_name: Span,
        pub encname: Span,
        pub slant: i64,
        pub extend: i64,
        pub fd_flags: i32,
        pub ty: i32,
        pub pid: i32,
        pub eid: i32,
        pub links: i32,
        /// `subfont`: an index in `sf_tab` plus 1, or 0 for NULL.
        pub subfont: i32,
        /// `in_use`, 0 or 1.  An `i32`, like `_pad`, so that the record has no padding.
        pub in_use: i32,
        pub _pad: i32,
    }
}

impl FmRec {
    #[inline]
    fn is_included(&self) -> bool {
        self.ty & ff::F_INCLUDED as i32 != 0
    }
    #[inline]
    fn is_subsetted(&self) -> bool {
        self.ty & ff::F_SUBSETTED as i32 != 0
    }
    #[inline]
    fn is_std_t1font(&self) -> bool {
        self.ty & ff::F_STDT1FONT as i32 != 0
    }
    #[inline]
    fn is_subfont(&self) -> bool {
        self.ty & ff::F_SUBFONT as i32 != 0
    }
    #[inline]
    fn is_type1(&self) -> bool {
        self.ty & ff::F_TYPE1 as i32 != 0
    }
    #[inline]
    fn is_truetype(&self) -> bool {
        self.ty & ff::F_TRUETYPE as i32 != 0
    }
    #[inline]
    fn is_opentype(&self) -> bool {
        self.ty & ff::F_OTF as i32 != 0
    }
    #[inline]
    fn is_pk(&self) -> bool {
        self.ty & ff::F_PK as i32 != 0
    }
    #[inline]
    fn is_reencoded(&self) -> bool {
        self.encname.len >= 0
    }
    #[inline]
    fn is_fontfile(&self) -> bool {
        self.ff_name.len >= 0
    }
    #[inline]
    fn is_builtin(&self) -> bool {
        !self.is_fontfile()
    }
}

crate::pod_struct! {
    /// `ff_entry`.
    pub struct FfRec {
        pub ff_name: Span,
        pub ff_path: Span,
    }
}

crate::pod_struct! {
    /// `sfd_entry`: the subfonts are `first..first+n` of `sf_tab`, in the C's list order.
    pub struct SfdRec {
        pub name: Span,
        pub first: i32,
        pub n: i32,
    }
}

crate::pod_struct! {
    /// `subfont_entry`.
    pub struct SfRec {
        pub infix: Span,
        pub charcodes: [i64; 256],
    }
}

crate::pod_struct! {
    /// `fe_entry`.  `names` holds, for each code with a name of its own, the code, the name and a 0.
    pub struct FeRec {
        pub name: Span,
        pub names: Span,
        pub fe_objnum: i32,
        pub tx: [u8; 32],
        /// `strcmp(glyph_names[32], "space") == 0`, for `hasspacechar`.
        pub space_at_32: bool,
        pub _pad: [u8; 3],
    }
}

crate::pod_struct! {
    /// The part of `fd_entry` that exists before the fonts are output, for the descriptors of included PDF files.
    pub struct FdRec {
        pub fm: i32,
        pub fd_objnum: i32,
        pub fn_objnum: i32,
        pub stem_v: i32,
        pub stem_v_set: bool,
        pub all_glyphs: bool,
        /// Registered in `fd_tree`.
        pub in_tree: bool,
        pub _pad: u8,
    }
}

crate::pod_struct! {
    /// A name given to `epdf_mark_glyphs`.
    pub struct GlItem {
        pub fd: i32,
        pub name: Span,
    }
}

crate::pod_struct! {
    /// `vf_entry`: the packets of `char_count` characters, from `first` in `vf_pack`.
    pub struct VfEnt {
        pub first: i32,
        pub n: i32,
    }
}

crate::pod_struct! {
    /// POD state private to this module.  Add fields here, not to state.rs.
    pub struct Vars {
        /// `tfm_tree != NULL`: `create_avl_trees` has run.
        pub trees: bool,
        /// `mitem->line != NULL`; only the default map file waits there.
        pub mitem_has_line: bool,
        pub mitem_line: A<u8, 32>,
        pub mitem_line_len: i32,
        pub pool: PodVec<u8>,
        pub fm_tab: PodVec<FmRec>,
        pub tfm_hash: PodVec<i32>,
        pub tfm_hash_used: i32,
        pub ps_hash: PodVec<i32>,
        pub ps_hash_used: i32,
        pub ff_tab: PodVec<FfRec>,
        pub sfd_tab: PodVec<SfdRec>,
        pub sf_tab: PodVec<SfRec>,
        pub fe_tab: PodVec<FeRec>,
        pub fd_tab: PodVec<FdRec>,
        pub gl_items: PodVec<GlItem>,
        pub vf_tab: PodVec<VfEnt>,
        pub vf_pack: PodVec<Span>,
        /// `packet_data_ptr`: an offset in `pool`.
        pub packet_data_ptr: i32,
        pub packet_stack_ptr: i32,
        pub packet_stack: A<[i32; 2], PACKET_STACK_CAP>,
        /// Each `\pdfglyphtounicode` as given: two lengths in 4 bytes each, then two strings.
        pub gtu_log: PodVec<u8>,
        /// The string `non_existent_path`, once `\pdfcopyfont` has needed it.
        pub non_existent_path: i32,
    }
}

/// `fd_entry` while the fonts are output.  `Default` is `new_fd_entry`.
#[derive(Default)]
struct Fd {
    fontname: Vec<u8>,
    subset_tag: Option<[u8; 6]>,
    ff_found: bool,
    ff_objnum: i32,
    write_ttf_glyph_names: bool,
    font_dim: FontDims,
    /// `fe`: an index in `fe_tab`.
    fe: Option<i32>,
    builtin_glyph_names: Option<GlyphNames>,
    tx_tree: Option<CodeSet>,
    gl_tree: GlyphSet,
}

/// `fo_entry`, with `cw->cw_objnum`.
struct Fo {
    fo_objnum: i32,
    tex_font: i32,
    fm: i32,
    fd: i32,
    fe: Option<i32>,
    cw_objnum: i32,
    first_char: i32,
    last_char: i32,
    tounicode_objnum: i32,
}

/// Non-POD state private to this module, in `self.host`.  Nothing here has to survive a snapshot: see the top.
#[derive(Default)]
pub struct HVars {
    /// `cur_file_name`, for the messages.
    cur_file_name: Option<Vec<u8>>,
    /// `fb_array`.
    fb: Vec<u8>,
    /// `st_tree` of `make_subset_tag`.
    used_tags: Vec<[u8; 6]>,
    t1_carry: T1Carry,
    cmaps: TtfCmapCache,
    gtu: GlyphUnicodeMap,
    /// How much of `gtu_log` has reached `gtu`.
    gtu_applied: i32,
    /// `fo_tree`, unsorted.
    fos: Vec<Fo>,
    /// One for each of `fd_tab`.
    fds: Vec<Fd>,
    /// How much of `gl_items` has reached `fds`.
    gl_applied: i32,
}

/// `strtol(b, &e, 10)`: the value and the offset of `e`, which is 0 if there is no number.
fn strtol10(b: &[u8]) -> (i64, usize) {
    let mut i = 0;
    while i < b.len() && matches!(b[i], b' ' | 9..=13) {
        i += 1;
    }
    let neg = i < b.len() && b[i] == b'-';
    if i < b.len() && (b[i] == b'-' || b[i] == b'+') {
        i += 1;
    }
    let start = i;
    let mut v: i64 = 0;
    while i < b.len() && b[i].is_ascii_digit() {
        let d = (b[i] - b'0') as i64;
        // Saturates like strtol, which says LONG_MAX or LONG_MIN.
        v = v.saturating_mul(10);
        v = if neg { v.saturating_sub(d) } else { v.saturating_add(d) };
        i += 1;
    }
    if i == start { (0, 0) } else { (v, i) }
}

/// `strstr`.
fn find_sub(s: &[u8], what: &[u8]) -> Option<usize> {
    s.windows(what.len()).position(|w| w == what)
}

/// The top of `lookup_fontmap`: skips a subset tag, and scans the name extensions.  Valid forms are
/// `<fontname>-Slant_<slant>`, `<fontname>-Slant_<slant>-Extend_<extend>`, `<fontname>-Extend_<extend>`.
/// Returns the name to look up, the slant and the extend.  The C cuts the caller's string there.
fn parse_ps_name(ps_name: &[u8]) -> (&[u8], i64, i64) {
    let mut s = ps_name;
    if ps_name.len() > 7 {
        // check for subsetted name tag
        if ps_name[..6].iter().all(|c| c.is_ascii_uppercase()) && ps_name[6] == b'+' {
            s = &ps_name[7..];
        }
    }
    let mut slant: i64 = 0;
    let mut extend: i64 = 0;
    let mut end = s.len();
    if let Some(a) = find_sub(s, b"-Slant_") {
        let b = &s[a + 7..];
        let (sl, e) = strtol10(b);
        let sl = sl as i32 as i64;
        if e != 0 && e == b.len() {
            slant = sl;
            end = a; // ps_name string ends before "-Slant_"
        } else if e != 0 {
            // only if <slant> is valid number
            if let Some(c) = find_sub(&b[e..], b"-Extend_") {
                let d = &b[e + c + 8..];
                let (ex, e2) = strtol10(d);
                if e2 != 0 && e2 == d.len() {
                    slant = sl;
                    extend = ex as i32 as i64;
                    end = a; // ps_name string ends before "-Slant_"
                }
            }
        }
    } else if let Some(a) = find_sub(s, b"-Extend_") {
        let b = &s[a + 8..];
        let (ex, e) = strtol10(b);
        if e != 0 && e == b.len() {
            extend = ex as i32 as i64;
            end = a; // ps_name string ends before "-Extend_"
        }
    }
    (&s[..end], slant, extend)
}

fn name_hash(name: &[u8]) -> u32 {
    let mut h: u32 = 2166136261;
    for &b in name {
        h ^= b as u32;
        h = h.wrapping_mul(16777619);
    }
    h
}

fn span_eq(pool: PodVec<u8>, arena: &mut Arena, s: Span, key: &[u8]) -> bool {
    if s.len != key.len() as i32 {
        return false;
    }
    for (i, &c) in key.iter().enumerate() {
        if pool.get(arena, s.off + i as i32) != c {
            return false;
        }
    }
    true
}

fn span_hash(pool: PodVec<u8>, arena: &mut Arena, s: Span) -> u32 {
    let mut h: u32 = 2166136261;
    for i in 0..s.len {
        h ^= pool.get(arena, s.off + i) as u32;
        h = h.wrapping_mul(16777619);
    }
    h
}

/// `avl_find(tfm_tree, ...)`: the slot of `name`, or -1.
fn tfm_slot(v: &Vars, arena: &mut Arena, name: &[u8]) -> i32 {
    let tab = v.tfm_hash;
    if tab.cap == 0 {
        return -1;
    }
    let mask = tab.cap - 1;
    let mut i = name_hash(name) as i32 & mask;
    loop {
        let e = tab.get(arena, i);
        if e == 0 {
            return -1;
        }
        if e > 0 {
            let fm = v.fm_tab.get(arena, e - 1);
            if span_eq(v.pool, arena, fm.tfm_name, name) {
                return i;
            }
        }
        i = (i + 1) & mask;
    }
}

/// `avl_find(ps_tree, ...)`, with `comp_fm_entry_ps`: the slot of this key, or -1.
fn ps_slot(v: &Vars, arena: &mut Arena, name: &[u8], slant: i64, extend: i64) -> i32 {
    let tab = v.ps_hash;
    if tab.cap == 0 {
        return -1;
    }
    let mask = tab.cap - 1;
    let mut i = name_hash(name) as i32 & mask;
    loop {
        let e = tab.get(arena, i);
        if e == 0 {
            return -1;
        }
        if e > 0 {
            let fm = v.fm_tab.get(arena, e - 1);
            if fm.slant == slant && fm.extend == extend && span_eq(v.pool, arena, fm.ps_name, name) {
                return i;
            }
        }
        i = (i + 1) & mask;
    }
}

impl Tex {
    // ---- strings and files, as the C has them ----

    /// Appends to `pool`, moving it when full.
    fn pdffont_pool_room(&mut self, n: i32) {
        let pool = self.v_pdffont.pool;
        if pool.len + n > pool.cap {
            let want = (pool.cap * 2).max(pool.len + n).max(1024);
            let bigger = self.arena_vec::<u8>(want);
            self.v_pdffont.pool = pool.moved_to(bigger, &mut self.arena);
        }
    }

    fn pdffont_pool_add(&mut self, s: &[u8]) -> Span {
        self.pdffont_pool_room(s.len() as i32);
        let mut pool = self.v_pdffont.pool;
        let off = pool.len;
        for &c in s {
            pool.push_within_cap(&mut self.arena, c);
        }
        self.v_pdffont.pool = pool;
        Span { off, len: s.len() as i32 }
    }

    fn pdffont_pool_add_opt(&mut self, s: Option<&[u8]>) -> Span {
        match s {
            Some(b) => self.pdffont_pool_add(b),
            None => NO_SPAN,
        }
    }

    fn pdffont_bytes(&mut self, s: Span) -> Vec<u8> {
        let pool = self.v_pdffont.pool;
        let mut v = Vec::with_capacity(s.len.max(0) as usize);
        for i in 0..s.len {
            v.push(pool.get(&mut self.arena, s.off + i));
        }
        v
    }

    fn pdffont_bytes_opt(&mut self, s: Span) -> Option<Vec<u8>> {
        if s.len < 0 { None } else { Some(self.pdffont_bytes(s)) }
    }

    /// `makecstring(s)`: a C string stops at a NUL.
    fn pdffont_make_cstring(&self, s: i32) -> Vec<u8> {
        let b = self.str_pool.range(self.str_start[s], self.str_start[s + 1]);
        let n = b.iter().position(|&c| c == 0).unwrap_or(b.len());
        b[..n].to_vec()
    }

    /// `maketexstring(s)`.
    fn pdffont_make_tex_string(&mut self, s: &[u8]) -> i32 {
        if s.is_empty() {
            return S_EMPTY;
        }
        self.str_room(s.len() as i32);
        for &c in s {
            self.append_char(c as i32);
        }
        self.make_string()
    }

    /// `set_cur_file_name(s)`.  The string it makes stays in the pool, as in the C.
    fn pdffont_set_cur_file_name(&mut self, s: &[u8]) {
        self.host.hv.pdffont.cur_file_name = Some(s.to_vec());
        let n = self.pdffont_make_tex_string(s);
        self.pack_file_name(n, S_EMPTY, S_EMPTY);
    }

    /// `cur_file_name = (char *) nameoffile + 1`.
    fn pdffont_cur_file_name_is_name_of_file(&mut self) {
        let n = self.name_of_file_bytes();
        self.host.hv.pdffont.cur_file_name = Some(n);
    }

    /// `open_input(&f, format, FOPEN_RBIN_MODE)` of openclose.c, for a format `host.rs` has no opener for: reads
    /// `name_of_file`, and sets it to the name found.  `recorder_record_input` is the World's log of the read.
    fn pdffont_open_input(&mut self, format: i32) -> Option<Vec<u8>> {
        let name = self.name_of_file_bytes();
        self.host.full_name_of_file = None;
        // Look in -output-directory first, if the filename is not absolute.
        if let Some(dir) = self.host.world.output_directory.clone() {
            if !platform::is_absolute(&name) {
                let mut p = dir;
                p.push(b'/');
                p.extend_from_slice(&name);
                if let Some(data) = self.host.world.read(&p) {
                    self.set_name_of_file(&p);
                    self.host.full_name_of_file = Some(p);
                    return Some(data);
                }
            }
        }
        let path = self.host.world.lookup(&name, format, true)?;
        // This fopen is not allowed to fail.
        let Some(data) = self.host.world.read(&path) else {
            self.pdffont_fail(b"fopen failed");
        };
        self.host.full_name_of_file = Some(path.clone());
        // Drop a leading ./ unless the user said it.
        let mut shown: &[u8] = &path;
        if shown.len() >= 2 && shown[0] == b'.' && platform::is_dir_sep(shown[1] as i32) {
            if !(name.len() >= 2 && name[0] == b'.' && platform::is_dir_sep(name[1] as i32)) {
                shown = &shown[2..];
            }
        }
        let shown = shown.to_vec();
        self.set_name_of_file(&shown);
        Some(data)
    }

    /// `tex_printf`: `print` of the string, which is `print_char` for each byte.  Its `xfflush(stdout)` is left
    /// to the next `update_terminal`: the bytes and their order on the terminal are the same.
    fn pdffont_tex_printf(&mut self, parts: &[&[u8]]) {
        let mut left = PRINTF_MAX;
        for p in parts {
            let n = p.len().min(left);
            self.print_s(&p[..n]);
            left -= n;
        }
    }

    /// `kpse_invocation_name`.
    fn pdffont_invocation_name(&self) -> Vec<u8> {
        rstex_kpse::with_global(|k| k.invocation_name().to_vec())
    }

    /// `pdftex_warn`, with the message formatted.
    fn pdffont_warn(&mut self, msg: &[u8]) {
        self.print_ln();
        self.print_ln();
        let name = self.pdffont_invocation_name();
        self.pdffont_tex_printf(&[b"pdfTeX warning: ", &name]);
        if let Some(c) = self.host.hv.pdffont.cur_file_name.clone() {
            self.pdffont_tex_printf(&[b" (file ", &c, b")"]);
        }
        self.pdffont_tex_printf(&[b": "]);
        let n = msg.len().min(PRINTF_MAX);
        self.print_s(&msg[..n]);
        self.print_ln();
    }

    /// `safe_print`: `print(*c)`, with `c` a signed `char`.
    fn pdffont_safe_print(&mut self, s: &[u8]) {
        for &c in s {
            self.print(c as i8 as i32);
        }
    }

    /// `pdftex_fail`, with the message formatted.
    fn pdffont_fail(&mut self, msg: &[u8]) -> ! {
        self.print_ln();
        self.pdffont_safe_print(b"!pdfTeX error: ");
        let name = self.pdffont_invocation_name();
        self.pdffont_safe_print(&name);
        if let Some(c) = self.host.hv.pdffont.cur_file_name.clone() {
            self.pdffont_safe_print(b" (file ");
            self.pdffont_safe_print(&c);
            self.pdffont_safe_print(b")");
        }
        self.pdffont_safe_print(b": ");
        let n = msg.len().min(PRINTF_MAX);
        self.pdffont_safe_print(&msg[..n]);
        self.print_ln();
        self.remove_pdffile();
        self.pdffont_safe_print(b" ==> Fatal error occurred, no output PDF file produced!");
        self.print_ln();
        self.uexit(1)
    }

    /// An `Err` from `crates/fontfile`: what the C had printed by then, and the failure.  A failed `assert`
    /// aborts the oracle without flushing its log; here it is a failure like the rest.
    fn pdffont_font_error(&mut self, e: FontError) -> ! {
        for w in e.warnings.iter() {
            self.pdffont_warn(w);
        }
        self.pdffont_fail(&e.message)
    }

    fn pdffont_warnings(&mut self, w: &[Vec<u8>]) {
        for m in w {
            self.pdffont_warn(m);
        }
    }

    // ---- PDF output, as the C has it ----

    /// `pdf_puts`.
    fn pdffont_pdf_puts(&mut self, s: &[u8]) {
        self.pdf_room(s.len() as i32 + 1);
        for &c in s {
            self.pdf_quick_out(c as i32);
        }
        if let Some(&l) = s.last() {
            self.pdf_last_byte = l as i32;
        }
    }

    /// `pdf_printf`: one `vsnprintf` into `print_buf`, so a long result is cut short.
    fn pdffont_pdf_printf(&mut self, parts: &[&[u8]]) {
        let mut b: Vec<u8> = Vec::with_capacity(64);
        for p in parts {
            b.extend_from_slice(p);
        }
        b.truncate(PRINTF_MAX);
        self.pdffont_pdf_puts(&b);
    }

    /// `fb_flush`: `pdflastbyte` stays as it was.
    fn pdffont_fb_flush(&mut self, data: &[u8]) {
        let mut p = 0;
        while p < data.len() {
            let n = ((self.pdf_buf_size - self.pdf_ptr) as usize).min(data.len() - p);
            let a = self.pdf_ptr;
            let b = a + n as i32;
            if self.pdf_buf_is_os {
                self.pdf_os_buf.range_mut(a, b).copy_from_slice(&data[p..p + n]);
            } else {
                self.pdf_op_buf.range_mut(a, b).copy_from_slice(&data[p..p + n]);
            }
            self.pdf_ptr = b;
            if self.pdf_ptr == self.pdf_buf_size {
                self.pdf_flush();
            }
            p += n;
        }
    }

    // ---- mapfile.c ----

    fn pdffont_fm(&mut self, i: i32) -> FmRec {
        self.v_pdffont.fm_tab.get(&mut self.arena, i)
    }

    /// `fm->F = xstrdup(..)`: the entry, once a tree takes it.  `delete_fm_entry` has nothing to do, since a bad
    /// or duplicate line never gets here; what a replace or delete leaves behind stays in the arena.
    fn pdffont_fm_commit(&mut self, fm: &FmEntry<'_>, subfont: i32) -> i32 {
        let rec = FmRec {
            tfm_name: self.pdffont_pool_add(fm.tfm_name),
            ps_name: self.pdffont_pool_add_opt(fm.ps_name),
            ff_name: self.pdffont_pool_add_opt(fm.ff_name),
            encname: self.pdffont_pool_add_opt(fm.encname),
            slant: fm.slant,
            extend: fm.extend,
            fd_flags: fm.fd_flags,
            ty: fm.ty as i32,
            pid: fm.pid as i32,
            eid: fm.eid as i32,
            links: 0,
            subfont,
            in_use: 0,
            _pad: 0,
        };
        let mut t = self.v_pdffont.fm_tab;
        self.arena_push(&mut t, rec);
        self.v_pdffont.fm_tab = t;
        t.len - 1
    }

    /// `avl_probe` into `tfm_tree` or `ps_tree`.
    fn pdffont_tab_insert(&mut self, is_ps: bool, name: &[u8], fm: i32) {
        let (mut tab, mut used) = if is_ps {
            (self.v_pdffont.ps_hash, self.v_pdffont.ps_hash_used)
        } else {
            (self.v_pdffont.tfm_hash, self.v_pdffont.tfm_hash_used)
        };
        if (used + 1) * 2 > tab.cap {
            let cap = (tab.cap * 2).max(1024);
            let mut new_tab = self.arena_vec::<i32>(cap);
            for _ in 0..cap {
                new_tab.push_within_cap(&mut self.arena, 0);
            }
            used = 0;
            for k in 0..tab.len {
                let e = tab.get(&mut self.arena, k);
                if e > 0 {
                    let r = self.v_pdffont.fm_tab.get(&mut self.arena, e - 1);
                    let s = if is_ps { r.ps_name } else { r.tfm_name };
                    let mut i = span_hash(self.v_pdffont.pool, &mut self.arena, s) as i32 & (cap - 1);
                    while new_tab.get(&mut self.arena, i) != 0 {
                        i = (i + 1) & (cap - 1);
                    }
                    new_tab.set(&mut self.arena, i, e);
                    used += 1;
                }
            }
            tab = new_tab;
        }
        let mask = tab.cap - 1;
        let mut i = name_hash(name) as i32 & mask;
        loop {
            let e = tab.get(&mut self.arena, i);
            if e <= 0 {
                if e == 0 {
                    used += 1;
                }
                break;
            }
            i = (i + 1) & mask;
        }
        tab.set(&mut self.arena, i, fm + 1);
        if is_ps {
            self.v_pdffont.ps_hash = tab;
            self.v_pdffont.ps_hash_used = used;
        } else {
            self.v_pdffont.tfm_hash = tab;
            self.v_pdffont.tfm_hash_used = used;
        }
    }

    /// `avl_do_entry`.  "not completely symmetrical with regards to tfm_name and ps_name handling, e. g. a
    /// duplicate tfm_name gives a `goto exit`, and no ps_name link is tried."  The C's result says whether the
    /// entry is to be freed; here an entry no tree takes is never made.
    fn pdffont_avl_do_entry(&mut self, fm: &FmEntry<'_>, subfont: i32, mode: FmMode) {
        let suppress_warn = self.pdf_suppress_warning_dup_map() > 0;
        let mut rec: i32 = -1;
        'exit: {
            // handle tfm_name link
            if fm.tfm_name != NONTFM {
                let slot = tfm_slot(&self.v_pdffont, &mut self.arena, fm.tfm_name);
                if slot >= 0 {
                    let tab = self.v_pdffont.tfm_hash;
                    let p = tab.get(&mut self.arena, slot) - 1;
                    match mode {
                        FmMode::DupIgnore => {
                            if !suppress_warn {
                                let m = [
                                    b"fontmap entry for `".as_slice(),
                                    fm.tfm_name,
                                    b"' already exists, duplicates ignored",
                                ]
                                .concat();
                                self.pdffont_warn(&m);
                            }
                            break 'exit;
                        }
                        FmMode::Replace | FmMode::Delete => {
                            let mut r = self.pdffont_fm(p);
                            if r.in_use != 0 {
                                let m = [
                                    b"fontmap entry for `".as_slice(),
                                    fm.tfm_name,
                                    b"' has been used, replace/delete not allowed",
                                ]
                                .concat();
                                self.pdffont_warn(&m);
                                break 'exit;
                            }
                            tab.set(&mut self.arena, slot, -1);
                            r.links &= !LINK_TFM;
                            self.v_pdffont.fm_tab.set(&mut self.arena, p, r);
                        }
                    }
                }
                if mode != FmMode::Delete {
                    rec = self.pdffont_fm_commit(fm, subfont);
                    self.pdffont_tab_insert(false, fm.tfm_name, rec);
                    let mut r = self.pdffont_fm(rec);
                    r.links |= LINK_TFM;
                    self.v_pdffont.fm_tab.set(&mut self.arena, rec, r);
                }
            }
            // handle ps_name link
            if let Some(ps) = fm.ps_name {
                let slot = ps_slot(&self.v_pdffont, &mut self.arena, ps, fm.slant, fm.extend);
                if slot >= 0 {
                    let tab = self.v_pdffont.ps_hash;
                    let p = tab.get(&mut self.arena, slot) - 1;
                    match mode {
                        FmMode::DupIgnore => break 'exit,
                        FmMode::Replace | FmMode::Delete => {
                            let mut r = self.pdffont_fm(p);
                            if r.in_use != 0 {
                                break 'exit;
                            }
                            tab.set(&mut self.arena, slot, -1);
                            r.links &= !LINK_PS;
                            self.v_pdffont.fm_tab.set(&mut self.arena, p, r);
                        }
                    }
                }
                if mode != FmMode::Delete && fm.is_t1fontfile() && fm.is_included() {
                    if rec < 0 {
                        rec = self.pdffont_fm_commit(fm, subfont);
                    }
                    self.pdffont_tab_insert(true, ps, rec);
                    let mut r = self.pdffont_fm(rec);
                    r.links |= LINK_PS;
                    self.v_pdffont.fm_tab.set(&mut self.arena, rec, r);
                }
            }
        }
    }

    /// `read_sfd`: the subfonts' range in `sf_tab`.
    fn pdffont_read_sfd(&mut self, sfd_name: &[u8]) -> Option<(i32, i32)> {
        // check whether this sfd has been read
        let tab = self.v_pdffont.sfd_tab;
        for i in 0..tab.len {
            let s = tab.get(&mut self.arena, i);
            if span_eq(self.v_pdffont.pool, &mut self.arena, s.name, sfd_name) {
                return Some((s.first, s.n));
            }
        }
        self.pdffont_set_cur_file_name(sfd_name);
        let Some(data) = self.pdffont_open_input(rstex_kpse::KPSE_SFD_FORMAT) else {
            self.pdffont_warn(b"cannot open SFD file for reading");
            self.host.hv.pdffont.cur_file_name = None;
            return None;
        };
        self.pdffont_tex_printf(&[b"{"]);
        self.pdffont_tex_printf(&[sfd_name]);
        let sfd = match ff::read_sfd(&data) {
            Ok(s) => s,
            Err(e) => self.pdffont_font_error(e),
        };
        self.pdffont_tex_printf(&[b"}"]);
        // cur_file_name stays: the C leaves it pointing at the name, until the end of fm_read_info.
        let first = self.v_pdffont.sf_tab.len;
        for sf in sfd.subfonts.iter() {
            let infix = self.pdffont_pool_add(&sf.infix);
            let mut t = self.v_pdffont.sf_tab;
            self.arena_push(&mut t, SfRec { infix, charcodes: *sf.charcodes });
            self.v_pdffont.sf_tab = t;
        }
        let name = self.pdffont_pool_add(sfd_name);
        let n = sfd.subfonts.len() as i32;
        let mut t = self.v_pdffont.sfd_tab;
        self.arena_push(&mut t, SfdRec { name, first, n });
        self.v_pdffont.sfd_tab = t;
        Some((first, n))
    }

    /// `handle_subfont_fm`.
    fn pdffont_handle_subfont_fm(&mut self, fm: &FmEntry<'_>, mode: FmMode) -> bool {
        let Some((prefix, sfd_part)) = ff::subfont_parts(fm.tfm_name) else {
            return false;
        };
        let mut buf = sfd_part.to_vec();
        // check_buf(strlen(buf) + 4, SMALL_BUF_SIZE)
        if buf.len() + 4 > SMALL_BUF_SIZE {
            let m = [b"buffer overflow at file ".as_slice(), SUBFONT_C, b", line 196"].concat();
            self.pdffont_fail(&m);
        }
        buf.extend_from_slice(b".sfd");
        let Some((first, n)) = self.pdffont_read_sfd(&buf) else {
            return false;
        };
        // at this point we know fm is a subfont
        let (pid, eid) = if fm.pid == -1 { (3, 1) } else { (fm.pid, fm.eid) };
        let ty = fm.ty | ff::F_SUBFONT;
        for k in first..first + n {
            let sf = self.v_pdffont.sf_tab.get(&mut self.arena, k);
            let infix = self.pdffont_bytes(sf.infix);
            let mut name = prefix.to_vec();
            name.extend_from_slice(&infix);
            let fm2 = FmEntry {
                tfm_name: &name,
                ps_name: None,
                fd_flags: ff::FD_FLAGS_NOT_SET_IN_MAPLINE,
                slant: 0,
                extend: 0,
                encname: None,
                ff_name: fm.ff_name,
                ty,
                pid,
                eid,
            };
            self.pdffont_avl_do_entry(&fm2, k + 1, mode);
        }
        true
    }

    /// `fm_scan_line`, after its first half.
    fn pdffont_fm_scan_line(&mut self, line: &[u8], mode: FmMode) {
        let scan = match ff::fm_scan_line(line) {
            Ok(s) => s,
            Err(e) => self.pdffont_font_error(e),
        };
        self.pdffont_warnings(&scan.warnings);
        let Some(fm) = scan.entry else {
            return;
        };
        if self.pdffont_handle_subfont_fm(&fm, mode) {
            return; // is this a subfont?
        }
        self.pdffont_avl_do_entry(&fm, 0, mode);
    }

    /// `fm_read_info` for a map file.
    fn pdffont_fm_read_file(&mut self, name: &[u8], mode: FmMode) {
        self.pdffont_set_cur_file_name(name);
        match self.pdffont_open_input(rstex_kpse::KPSE_FONTMAP_FORMAT) {
            None => self.pdffont_warn(b"cannot open font map file"),
            Some(data) => {
                self.pdffont_cur_file_name_is_name_of_file();
                let shown = self.name_of_file_bytes();
                self.pdffont_tex_printf(&[b"{", &shown]);
                // Most of a map file's bytes reach the pool; this saves moving it several times over.
                self.pdffont_pool_room(data.len() as i32);
                let mut pos = 0;
                let mut buf = [0u8; 1024];
                loop {
                    let line = match ff::fm_read_line(&data, &mut pos, &mut buf) {
                        Ok(Some(l)) => l,
                        Ok(None) => break,
                        Err(e) => self.pdffont_font_error(e),
                    };
                    self.pdffont_fm_scan_line(line, mode);
                }
                self.pdffont_tex_printf(&[b"}"]);
            }
        }
    }

    /// `fm_read_info` for the default map file, if it waits; otherwise only `create_avl_trees`.
    fn pdffont_fm_read_default(&mut self) {
        self.v_pdffont.trees = true;
        if !self.v_pdffont.mitem_has_line {
            return; // nothing to do
        }
        let _p = crate::prof_scope!("pdftex.map read and parse");
        let n = self.v_pdffont.mitem_line_len;
        let name = self.v_pdffont.mitem_line.range(0, n).to_vec();
        self.pdffont_fm_read_file(&name, FmMode::DupIgnore);
        self.v_pdffont.mitem_has_line = false; // done with this line
        self.host.hv.pdffont.cur_file_name = None;
    }

    /// `process_map_item`.
    fn pdffont_process_map_item(&mut self, s: &[u8], ty: MapType) {
        let item = ff::process_map_item(s, ty);
        if item.flush_default {
            self.v_pdffont.mitem_has_line = false; // flush default map file name
        }
        if self.v_pdffont.mitem_has_line {
            self.pdffont_fm_read_default(); // read default map file first
        }
        if !item.item.is_empty() {
            // only if real item to process
            self.v_pdffont.trees = true;
            match ty {
                MapType::MapFile => self.pdffont_fm_read_file(item.item, item.mode),
                MapType::MapLine => {
                    self.host.hv.pdffont.cur_file_name = None; // makes pdftex_warn() shorter
                    self.pdffont_fm_scan_line(item.item, item.mode);
                }
            }
            self.host.hv.pdffont.cur_file_name = None;
        }
    }

    /// `fmlookup`: the value for `pdf_font_map[f]`.
    fn pdffont_fmlookup(&mut self, f: i32) -> i32 {
        if !self.v_pdffont.trees {
            self.pdffont_fm_read_default(); // only to read default map file
        }
        let s = self.font_name[f];
        // Look up for full <tfmname>[+-]<expand>.  A C string stops at a NUL.
        let tfm = self.str_pool.range(self.str_start[s], self.str_start[s + 1]);
        let tfm = &tfm[..tfm.iter().position(|&c| c == 0).unwrap_or(tfm.len())];
        let slot = tfm_slot(&self.v_pdffont, &mut self.arena, tfm);
        if slot < 0 {
            return FM_DUMMY;
        }
        let i = self.v_pdffont.tfm_hash.get(&mut self.arena, slot) - 1;
        let mut r = self.pdffont_fm(i);
        r.in_use = 1;
        self.v_pdffont.fm_tab.set(&mut self.arena, i, r);
        (i + 1) * 2 + r.is_pk() as i32
    }

    /// The index in `fm_tab` of a font's entry, after `hasfmentry` said yes.
    #[inline]
    fn pdffont_fm_index(&self, f: i32) -> i32 {
        (self.pdf_font_map[f] >> 1) - 1
    }

    /// External: mapfile.c
    pub fn hasfmentry(&mut self, f: i32) -> bool {
        if self.pdf_font_map[f] == 0 {
            self.pdf_font_map[f] = self.pdffont_fmlookup(f);
        }
        self.pdf_font_map[f] != FM_DUMMY
    }

    /// External: mapfile.c
    #[inline]
    pub fn isscalable(&mut self, f: i32) -> bool {
        self.hasfmentry(f) && self.pdf_font_map[f] & 1 == 0
    }

    /// External: mapfile.c
    pub fn hasspacechar(&mut self, f: i32) -> bool {
        if !self.isscalable(f) {
            return false;
        }
        let i = self.pdffont_fm_index(f);
        let fm = self.pdffont_fm(i);
        // if a font is not re-encoded via its map entry, we assume it has no space char
        if fm.is_reencoded() {
            let fe = self.pdffont_get_fe_entry(fm.encname);
            return self.v_pdffont.fe_tab.get(&mut self.arena, fe).space_at_32;
        }
        false
    }

    /// `check_ff_exist`: `ff_path`.  Only fonts that are needed get here, so the table stays short.
    fn pdffont_check_ff_exist(&mut self, ff_name: &[u8], is_tt: bool) -> Option<Vec<u8>> {
        let tab = self.v_pdffont.ff_tab;
        for i in 0..tab.len {
            let r = tab.get(&mut self.arena, i);
            if span_eq(self.v_pdffont.pool, &mut self.arena, r.ff_name, ff_name) {
                return self.pdffont_bytes_opt(r.ff_path);
            }
        }
        let format = if is_tt { rstex_kpse::KPSE_TRUETYPE_FORMAT } else { rstex_kpse::KPSE_TYPE1_FORMAT };
        let path = self.host.world.lookup(ff_name, format, false);
        let rec = FfRec {
            ff_name: self.pdffont_pool_add(ff_name),
            ff_path: self.pdffont_pool_add_opt(path.as_deref()),
        };
        let mut t = self.v_pdffont.ff_tab;
        self.arena_push(&mut t, rec);
        self.v_pdffont.ff_tab = t;
        path
    }

    /// `lookup_fontmap`: lookup fontmap by ps_name, used for Type1 font replacement when embedding PDF files.
    /// An index in `fm_tab`, or -1.  `fm_valid_for_font_replacement` is its only test.
    pub fn pdffont_lookup_fontmap(&mut self, ps_name: &[u8]) -> i32 {
        if !self.v_pdffont.trees {
            self.pdffont_fm_read_default(); // only to read default map file
        }
        let (s, slant, extend) = parse_ps_name(ps_name);
        let slot = ps_slot(&self.v_pdffont, &mut self.arena, s, slant, extend);
        if slot < 0 {
            return -1; // no entry found
        }
        let i = self.v_pdffont.ps_hash.get(&mut self.arena, slot) - 1;
        let fm = self.pdffont_fm(i);
        let ff_name = self.pdffont_bytes(fm.ff_name);
        if self.pdffont_check_ff_exist(&ff_name, false).is_some() { i } else { -1 }
    }

    /// External: mapfile.c: t is a token list header
    pub fn pdfmapfile(&mut self, t: i32) {
        let s = self.tokens_to_string(t);
        let b = self.pdffont_make_cstring(s);
        self.pdffont_process_map_item(&b, MapType::MapFile);
        self.flush_str(self.last_tokens_string);
    }

    /// External: mapfile.c: t is a token list header
    pub fn pdfmapline(&mut self, t: i32) {
        let s = self.tokens_to_string(t);
        let b = self.pdffont_make_cstring(s);
        self.pdffont_process_map_item(&b, MapType::MapLine);
        self.flush_str(self.last_tokens_string);
    }

    /// External: mapfile.c: map line of the \pdfspacefont
    pub fn pdfmaplinesp(&mut self) {
        self.pdffont_process_map_item(b"=pdftexspace PdfTeX-Space <pdftexspace.pfb", MapType::MapLine);
    }

    /// External: mapfile.c
    pub fn pdf_init_map_file(&mut self, map_name: &[u8]) {
        let n = map_name.len() as i32;
        for (i, &c) in map_name.iter().enumerate() {
            self.v_pdffont.mitem_line.put(i as i32, c as i32);
        }
        self.v_pdffont.mitem_line_len = n;
        self.v_pdffont.mitem_has_line = true;
    }

    // ---- tounicode.c ----

    /// Brings `gtu` up to date with the log, which is all a restored image has.
    fn pdffont_gtu_sync(&mut self) {
        let log = self.v_pdffont.gtu_log;
        let mut at = self.host.hv.pdffont.gtu_applied;
        if at > log.len {
            self.host.hv.pdffont.gtu = GlyphUnicodeMap::new();
            at = 0;
        }
        let mut word = |t: &mut Tex, p: &mut i32| -> i32 {
            let mut v = 0i32;
            for k in 0..4 {
                v |= (log.get(&mut t.arena, *p + k) as i32) << (8 * k);
            }
            *p += 4;
            v
        };
        while at < log.len {
            let a = word(self, &mut at);
            let b = word(self, &mut at);
            let g = self.pdffont_log_bytes(at, a);
            at += a;
            let u = self.pdffont_log_bytes(at, b);
            at += b;
            // It got through once, so it cannot fail; its warning has been printed.
            let _ = self.host.hv.pdffont.gtu.deftounicode(&g, &u);
        }
        self.host.hv.pdffont.gtu_applied = at;
    }

    fn pdffont_log_bytes(&mut self, at: i32, n: i32) -> Vec<u8> {
        let log = self.v_pdffont.gtu_log;
        let mut v = Vec::with_capacity(n as usize);
        for i in 0..n {
            v.push(log.get(&mut self.arena, at + i));
        }
        v
    }

    fn pdffont_log_add(&mut self, s: &[u8]) {
        let mut log = self.v_pdffont.gtu_log;
        for &c in s {
            self.arena_push(&mut log, c);
        }
        self.v_pdffont.gtu_log = log;
    }

    /// External: tounicode.c
    pub fn def_tounicode(&mut self, glyph: i32, unistr: i32) {
        let g = self.pdffont_make_cstring(glyph);
        let u = self.pdffont_make_cstring(unistr);
        self.pdffont_gtu_sync();
        match self.host.hv.pdffont.gtu.deftounicode(&g, &u) {
            Ok(Some(w)) => self.pdffont_warn(&w),
            Ok(None) => {}
            Err(e) => self.pdffont_font_error(e),
        }
        self.pdffont_log_add(&(g.len() as u32).to_le_bytes());
        self.pdffont_log_add(&(u.len() as u32).to_le_bytes());
        self.pdffont_log_add(&g);
        self.pdffont_log_add(&u);
        self.host.hv.pdffont.gtu_applied = self.v_pdffont.gtu_log.len;
    }

    /// What surrounds the stream of `write_tounicode`.  The object number, or 0.
    fn pdffont_write_tounicode(&mut self, names: &GlyphNames, tfmname: &[u8], encname: Option<&[u8]>) -> i32 {
        self.pdffont_gtu_sync();
        if !self.host.hv.pdffont.gtu.exists() {
            self.pdffont_warn(b"no GlyphToUnicode entry has been inserted yet!");
            self.fixed_gen_tounicode = 0;
            return 0;
        }
        let mut out = core::mem::take(&mut self.host.hv.pdffont.fb);
        out.clear();
        let mut gtu = core::mem::take(&mut self.host.hv.pdffont.gtu);
        let r = ff::write_tounicode(names, tfmname, encname, &mut gtu, &mut out);
        self.host.hv.pdffont.gtu = gtu;
        let warnings = match r {
            Ok(w) => w,
            Err(e) => self.pdffont_font_error(e),
        };
        let objnum = self.pdf_new_objnum();
        self.pdf_begin_dict(objnum, 0);
        self.pdf_begin_stream();
        self.pdffont_fb_flush(&out);
        if let Some(&l) = out.last() {
            self.pdf_last_byte = l as i32; // the C writes this with pdf_printf
        }
        self.pdf_end_stream();
        self.host.hv.pdffont.fb = out;
        self.pdffont_warnings(&warnings);
        objnum
    }

    // ---- writeenc.c, and load_enc_file's wrapper in writet1.c ----

    /// `get_fe_entry`.  The file is read once; the C fails rather than return NULL.
    fn pdffont_get_fe_entry(&mut self, encname: Span) -> i32 {
        let tab = self.v_pdffont.fe_tab;
        let s = self.pdffont_bytes(encname);
        for i in 0..tab.len {
            let r = tab.get(&mut self.arena, i);
            if span_eq(self.v_pdffont.pool, &mut self.arena, r.name, &s) {
                return i;
            }
        }
        // load_enc_file
        self.pdffont_set_cur_file_name(&s);
        let Some(data) = self.pdffont_open_input(rstex_kpse::KPSE_ENC_FORMAT) else {
            self.pdffont_fail(b"cannot open encoding file for reading");
        };
        self.pdffont_tex_printf(&[b"{"]);
        self.pdffont_cur_file_name_is_name_of_file();
        let shown = self.name_of_file_bytes();
        self.pdffont_tex_printf(&[&shown]);
        let names = match ff::load_enc_file(&data) {
            Ok(n) => n,
            Err(e) => self.pdffont_font_error(e),
        };
        self.pdffont_tex_printf(&[b"}"]);
        self.host.hv.pdffont.cur_file_name = None;
        self.pdffont_fe_add(encname, &names)
    }

    /// `register_fe_entry`, with `fe->glyph_names` packed into `pool`.
    fn pdffont_fe_add(&mut self, encname: Span, names: &GlyphNames) -> i32 {
        let mut packed: Vec<u8> = Vec::with_capacity(2048);
        for c in 0..256 {
            if !names.is_notdef(c) {
                packed.push(c as u8);
                packed.extend_from_slice(names.name(c));
                packed.push(0);
            }
        }
        let rec = FeRec {
            name: encname,
            names: self.pdffont_pool_add(&packed),
            fe_objnum: 0,
            tx: [0; 32],
            space_at_32: names.name(32) == b"space",
            _pad: [0; 3],
        };
        let mut t = self.v_pdffont.fe_tab;
        self.arena_push(&mut t, rec);
        self.v_pdffont.fe_tab = t;
        t.len - 1
    }

    /// `fe->glyph_names`.
    fn pdffont_fe_glyph_names(&mut self, fe: i32) -> GlyphNames {
        let r = self.v_pdffont.fe_tab.get(&mut self.arena, fe);
        let packed = self.pdffont_bytes(r.names);
        let mut names = GlyphNames::new();
        let mut i = 0;
        while i < packed.len() {
            let start = i + 1;
            let end = start + packed[start..].iter().position(|&c| c == 0).unwrap_or(packed.len() - start);
            names.set(packed[i] as usize, &packed[start..end]);
            i = end + 1;
        }
        names
    }

    /// `write_fontencodings`.
    fn pdffont_write_fontencodings(&mut self) {
        let tab = self.v_pdffont.fe_tab;
        let mut order: Vec<(Vec<u8>, i32)> = Vec::with_capacity(tab.len as usize);
        for i in 0..tab.len {
            let r = tab.get(&mut self.arena, i);
            order.push((self.pdffont_bytes(r.name), i));
        }
        order.sort();
        for (_, i) in order {
            let r = tab.get(&mut self.arena, i);
            if r.fe_objnum != 0 {
                // write_fontencoding, write_enc
                let names = self.pdffont_fe_glyph_names(i);
                self.pdf_begin_dict(r.fe_objnum, 1);
                self.pdffont_pdf_puts(b"/Type /Encoding\n");
                self.pdffont_pdf_puts(b"/Differences [");
                let mut b = Vec::with_capacity(1024);
                ff::write_enc_differences(&names, &CodeSet(r.tx), &mut b);
                self.pdffont_pdf_puts_long(&b);
                self.pdffont_pdf_puts(b"]\n");
                self.pdf_end_dict();
            }
        }
    }

    /// Several of the C's `pdf_printf`s at once, which together may not fit one buffer.
    fn pdffont_pdf_puts_long(&mut self, s: &[u8]) {
        for chunk in s.chunks(1000) {
            self.pdffont_pdf_puts(chunk);
        }
    }

    // ---- writefont.c ----

    /// Makes `fds` as long as `fd_tab`: the descriptors `epdf_create_fontdescriptor` made, with their glyphs.
    fn pdffont_fds_sync(&mut self) {
        let tab = self.v_pdffont.fd_tab;
        let mut k = self.host.hv.pdffont.fds.len() as i32;
        while k < tab.len {
            let r = tab.get(&mut self.arena, k);
            let fm = self.pdffont_fm(r.fm);
            let mut fd = Fd::default();
            fd.fontname = self.pdffont_bytes(fm.ps_name); // just fallback
            if r.stem_v_set {
                fd.font_dim[ff::STEMV_CODE] = ff::IntParm { val: r.stem_v, set: true };
            }
            self.host.hv.pdffont.fds.push(fd);
            k += 1;
        }
        let items = self.v_pdffont.gl_items;
        let mut a = self.host.hv.pdffont.gl_applied;
        while a < items.len {
            let it = items.get(&mut self.arena, a);
            let name = self.pdffont_bytes(it.name);
            self.host.hv.pdffont.fds[it.fd as usize].gl_tree.insert(&name);
            a += 1;
        }
        self.host.hv.pdffont.gl_applied = a;
    }

    /// `lookup_fd_entry`.
    fn pdffont_lookup_fd_entry(&mut self, s: Span, slant: i64, extend: i64) -> i32 {
        let name = self.pdffont_bytes(s);
        let tab = self.v_pdffont.fd_tab;
        for i in 0..tab.len {
            let r = tab.get(&mut self.arena, i);
            if r.in_tree {
                let fm = self.pdffont_fm(r.fm);
                if fm.slant == slant
                    && fm.extend == extend
                    && span_eq(self.v_pdffont.pool, &mut self.arena, fm.ff_name, &name)
                {
                    return i;
                }
            }
        }
        -1
    }

    /// `new_fd_entry` and its `fm`: the record.
    fn pdffont_new_fd(&mut self, fm: i32, in_tree: bool) -> i32 {
        let rec = FdRec {
            fm,
            fd_objnum: 0,
            fn_objnum: 0,
            stem_v: 0,
            stem_v_set: false,
            all_glyphs: false,
            in_tree,
            _pad: 0,
        };
        let mut t = self.v_pdffont.fd_tab;
        self.arena_push(&mut t, rec);
        self.v_pdffont.fd_tab = t;
        t.len - 1
    }

    /// `preset_fontmetrics`.  Only fallback values of font metrics are taken from the TFM info of f.  During
    /// reading of the font file, these values are replaced by metrics from the font, if available.
    fn pdffont_preset_fontmetrics(&mut self, d: &mut FontDims, f: i32) {
        let fs = self.pdf_font_size[f];
        // dividescaled takes an integer, so the angle in degrees is cut to one first.  ff::atan is the same
        // on every platform.
        let slant = self.get_slant(f);
        let angle = -ff::atan(slant as f64 / 65536.0) * (180.0 / core::f64::consts::PI);
        d[ff::ITALIC_ANGLE_CODE].val = self.divide_scaled(angle as i64 as i32, fs, 3);
        let h = self.get_charheight(f, b'h' as i32);
        d[ff::ASCENT_CODE].val = self.divide_scaled(h, fs, 3);
        let cap = self.get_charheight(f, b'H' as i32);
        d[ff::CAPHEIGHT_CODE].val = self.divide_scaled(cap, fs, 3);
        let y = self.get_chardepth(f, b'y' as i32);
        let i = -self.divide_scaled(y, fs, 3);
        d[ff::DESCENT_CODE].val = if i < 0 { i } else { 0 };
        let dot = self.get_charwidth(f, b'.' as i32);
        d[ff::STEMV_CODE].val = self.divide_scaled(dot / 3, fs, 3);
        let x = self.get_x_height(f);
        d[ff::XHEIGHT_CODE].val = self.divide_scaled(x, fs, 3);
        d[ff::FONTBBOX1_CODE].val = 0;
        d[ff::FONTBBOX2_CODE].val = d[ff::DESCENT_CODE].val;
        let q = self.get_quad(f);
        d[ff::FONTBBOX3_CODE].val = self.divide_scaled(q, fs, 3);
        d[ff::FONTBBOX4_CODE].val = if d[ff::CAPHEIGHT_CODE].val > d[ff::ASCENT_CODE].val {
            d[ff::CAPHEIGHT_CODE].val
        } else {
            d[ff::ASCENT_CODE].val
        };
        for k in d.iter_mut().take(ff::INT_KEYS_NUM) {
            k.set = true;
        }
    }

    /// `create_fontdescriptor`.  The caller has run `pdffont_fds_sync`.
    fn pdffont_create_fontdescriptor(&mut self, fm_i: i32, fe: Option<i32>, f: i32, in_tree: bool) -> i32 {
        let fm = self.pdffont_fm(fm_i);
        let i = self.pdffont_new_fd(fm_i, in_tree);
        let mut fd = Fd::default();
        // preset_fontname: just fallback
        fd.fontname = if fm.ps_name.len >= 0 { self.pdffont_bytes(fm.ps_name) } else { self.pdffont_bytes(fm.tfm_name) };
        let mut dims = fd.font_dim;
        self.pdffont_preset_fontmetrics(&mut dims, f);
        fd.font_dim = dims;
        fd.fe = fe; // encoding needed by TrueType writing
        self.host.hv.pdffont.fds.push(fd);
        i
    }

    /// `write_fontname`.
    fn pdffont_write_fontname(&mut self, fd: i32, key: Option<&[u8]>) {
        let d = &self.host.hv.pdffont.fds[fd as usize];
        let (name, tag) = (d.fontname.clone(), d.subset_tag);
        self.pdffont_pdf_puts(b"/");
        if let Some(k) = key {
            self.pdffont_pdf_printf(&[k, b" /"]);
        }
        if let Some(t) = tag {
            self.pdffont_pdf_printf(&[&t, b"+"]);
        }
        self.pdffont_pdf_printf(&[&name, b"\n"]);
    }

    /// `writet1`, up to where `write_fontfile` goes on: the three lengths.
    fn pdffont_writet1(&mut self, fd: &mut Fd, rec: &FdRec, fm: &FmRec) -> (i32, i32, i32) {
        let ff_name = self.pdffont_bytes(fm.ff_name);
        // t1_open_fontfile
        let Some(path) = self.pdffont_check_ff_exist(&ff_name, fm.is_truetype()) else {
            self.pdffont_set_cur_file_name(&ff_name);
            self.pdffont_fail(b"cannot open Type 1 font file for reading");
        };
        self.host.hv.pdffont.cur_file_name = Some(path.clone());
        let Some(data) = self.host.world.read(&path) else {
            self.pdffont_fail(b"cannot open Type 1 font file for reading");
        };
        let subsetted = fm.is_subsetted();
        let (open, close): (&[u8], &[u8]) = if subsetted { (b"<", b">") } else { (b"<<", b">>") };
        self.pdffont_tex_printf(&[open]);
        self.pdffont_tex_printf(&[&path]);
        fd.ff_found = true;
        let mut out = core::mem::take(&mut self.host.hv.pdffont.fb);
        out.clear();
        let mut carry = core::mem::take(&mut self.host.hv.pdffont.t1_carry);
        let tags = core::mem::take(&mut self.host.hv.pdffont.used_tags);
        let preset = core::mem::take(&mut fd.fontname);
        let inp = ff::T1Input {
            font: &data,
            subsetted,
            all_glyphs: rec.all_glyphs,
            slant: fm.slant,
            extend: fm.extend,
            fontname: &preset,
            tx_codes: fd.tx_tree.as_ref(),
            used_tags: &tags,
        };
        let r = {
            let _p = crate::prof_scope!("write_t1: parse and subset");
            ff::write_t1(&inp, &mut fd.font_dim, &mut fd.gl_tree, &mut carry, &mut out)
        };
        self.host.hv.pdffont.t1_carry = carry;
        self.host.hv.pdffont.used_tags = tags;
        let o = match r {
            Ok(o) => o,
            Err(e) => self.pdffont_font_error(e),
        };
        self.pdffont_warnings(&o.warnings);
        self.pdffont_tex_printf(&[close]);
        self.host.hv.pdffont.cur_file_name = None;
        fd.fontname = o.fontname;
        fd.subset_tag = o.subset_tag;
        if let Some(t) = o.subset_tag {
            self.host.hv.pdffont.used_tags.push(t);
        }
        fd.builtin_glyph_names = o.builtin_glyph_names;
        self.host.hv.pdffont.fb = out;
        (o.length1, o.length2, o.length3)
    }

    /// `writettf`: `ttf_length`.
    fn pdffont_writettf(&mut self, fd: &mut Fd, fm: &FmRec) -> i32 {
        let ff_name = self.pdffont_bytes(fm.ff_name);
        let subsetted = fm.is_subsetted();
        self.pdffont_set_cur_file_name(&ff_name);
        if subsetted && fd.fe.is_none() && !fm.is_subfont() {
            self.pdffont_fail(b"Subset TrueType must be a reencoded or a subfont");
        }
        let Some(data) = self.pdffont_open_input(rstex_kpse::KPSE_TRUETYPE_FORMAT) else {
            self.pdffont_fail(b"cannot open TrueType font file for reading");
        };
        self.pdffont_cur_file_name_is_name_of_file();
        let shown = self.name_of_file_bytes();
        if subsetted {
            self.pdffont_tex_printf(&[b"<", &shown]);
        } else {
            self.pdffont_tex_printf(&[b"<<", &shown]);
        }
        fd.ff_found = true;
        self.pdf_save_offset = self.pdf_offset();
        self.pdf_flush();
        let fe_names = fd.fe.map(|e| self.pdffont_fe_glyph_names(e));
        let subfont = if fm.subfont > 0 {
            Some(self.v_pdffont.sf_tab.get(&mut self.arena, fm.subfont - 1).charcodes)
        } else {
            None
        };
        let tfm_name = self.pdffont_bytes(fm.tfm_name);
        let mut out = core::mem::take(&mut self.host.hv.pdffont.fb);
        out.clear();
        let mut cmaps = core::mem::take(&mut self.host.hv.pdffont.cmaps);
        let tags = core::mem::take(&mut self.host.hv.pdffont.used_tags);
        let preset = core::mem::take(&mut fd.fontname);
        let mut tx = fd.tx_tree.unwrap_or_default();
        let inp = ff::TtfInput {
            font: &data,
            file_name: &shown,
            ff_name: &ff_name,
            tfm_name: &tfm_name,
            subsetted,
            fontname: &preset,
            fe_names: fe_names.as_ref(),
            subfont: subfont.as_ref(),
            pid: fm.pid,
            eid: fm.eid,
            write_ttf_glyph_names: fd.write_ttf_glyph_names,
            used_tags: &tags,
        };
        let r = ff::write_ttf(&inp, &mut fd.font_dim, &mut fd.gl_tree, &mut tx, &mut cmaps, &mut out);
        self.host.hv.pdffont.cmaps = cmaps;
        self.host.hv.pdffont.used_tags = tags;
        let o = match r {
            Ok(o) => o,
            Err(e) => self.pdffont_font_error(e),
        };
        self.pdffont_warnings(&o.warnings);
        fd.tx_tree = Some(tx);
        fd.fontname = o.fontname;
        fd.subset_tag = o.subset_tag;
        if let Some(t) = o.subset_tag {
            self.host.hv.pdffont.used_tags.push(t);
        }
        if subsetted {
            self.pdffont_tex_printf(&[b">"]);
        } else {
            self.pdffont_tex_printf(&[b">>"]);
        }
        self.host.hv.pdffont.cur_file_name = None;
        self.host.hv.pdffont.fb = out;
        o.length
    }

    /// `writeotf`.
    fn pdffont_writeotf(&mut self, fd: &mut Fd, fm: &FmRec) {
        let ff_name = self.pdffont_bytes(fm.ff_name);
        self.pdffont_set_cur_file_name(&ff_name);
        if fm.is_subsetted() {
            self.pdffont_fail(b"OTF fonts must be included entirely");
        }
        let Some(data) = self.pdffont_open_input(rstex_kpse::KPSE_OPENTYPE_FORMAT) else {
            self.pdffont_fail(b"cannot open OpenType font file for reading");
        };
        self.pdffont_cur_file_name_is_name_of_file();
        let shown = self.name_of_file_bytes();
        self.pdffont_tex_printf(&[b"<<", &shown]);
        fd.ff_found = true;
        let mut out = core::mem::take(&mut self.host.hv.pdffont.fb);
        out.clear();
        let warnings = match ff::write_otf(&data, false, &mut fd.font_dim, &mut out) {
            Ok(w) => w,
            Err(e) => self.pdffont_font_error(e),
        };
        self.pdffont_warnings(&warnings);
        self.pdffont_tex_printf(&[b">>"]);
        self.host.hv.pdffont.cur_file_name = None;
        self.host.hv.pdffont.fb = out;
    }

    /// `write_fontfile`.
    fn pdffont_write_fontfile(&mut self, fd: &mut Fd, rec: &FdRec, fm: &FmRec) {
        let mut lengths = (0, 0, 0);
        if fm.is_type1() {
            lengths = self.pdffont_writet1(fd, rec, fm);
        } else if fm.is_truetype() {
            lengths.0 = self.pdffont_writettf(fd, fm);
        } else if fm.is_opentype() {
            self.pdffont_writeotf(fd, fm);
        }
        if !fd.ff_found {
            return;
        }
        fd.ff_objnum = self.pdf_new_objnum();
        self.pdf_begin_dict(fd.ff_objnum, 0); // font file stream
        if fm.is_type1() {
            let (a, b, c) = (int_bytes(lengths.0), int_bytes(lengths.1), int_bytes(lengths.2));
            self.pdffont_pdf_printf(&[b"/Length1 ", &a, b"\n/Length2 ", &b, b"\n/Length3 ", &c, b"\n"]);
        } else if fm.is_truetype() {
            self.pdffont_pdf_printf(&[b"/Length1 ", &int_bytes(lengths.0), b"\n"]);
        } else if fm.is_opentype() {
            self.pdffont_pdf_puts(b"/Subtype /Type1C\n");
        }
        self.pdfout_next_stream_is_fontfile();
        self.pdf_begin_stream();
        let out = core::mem::take(&mut self.host.hv.pdffont.fb);
        self.pdffont_fb_flush(&out);
        self.host.hv.pdffont.fb = out;
        self.pdf_end_stream();
    }

    /// `write_fontdescriptor`.
    fn pdffont_write_fontdescriptor(&mut self, i: i32) {
        let mut rec = self.v_pdffont.fd_tab.get(&mut self.arena, i);
        let fm = self.pdffont_fm(rec.fm);
        if fm.is_fontfile() {
            // this will set fd->ff_found if font file is found
            let mut fd = core::mem::take(&mut self.host.hv.pdffont.fds[i as usize]);
            self.pdffont_write_fontfile(&mut fd, &rec, &fm);
            self.host.hv.pdffont.fds[i as usize] = fd;
        }
        if rec.fn_objnum != 0 {
            // write_fontname_object
            self.pdf_begin_obj(rec.fn_objnum, 1);
            self.pdffont_write_fontname(i, None);
            self.pdf_end_obj();
        }
        if rec.fd_objnum == 0 {
            rec.fd_objnum = self.pdf_new_objnum();
            self.v_pdffont.fd_tab.set(&mut self.arena, i, rec);
        }
        self.pdf_begin_dict(rec.fd_objnum, 1);
        self.pdffont_pdf_puts(b"/Type /FontDescriptor\n");
        self.pdffont_write_fontname(i, Some(b"FontName"));
        let ps_name = self.pdffont_bytes_opt(fm.ps_name);
        let tfm_name = self.pdffont_bytes(fm.tfm_name);
        let ff_found = self.host.hv.pdffont.fds[i as usize].ff_found;
        let (fd_flags, w) =
            ff::font_descriptor_flags(fm.fd_flags, ff_found, ps_name.as_deref(), &tfm_name, fm.is_std_t1font());
        if let Some(w) = w {
            self.pdffont_warn(&w);
        }
        self.pdffont_pdf_printf(&[b"/Flags ", &int_bytes(fd_flags), b"\n"]);
        // write_fontmetrics, with fix_fontmetrics in it
        let ff_name = self.pdffont_bytes_opt(fm.ff_name).unwrap_or_default();
        let mut dims = self.host.hv.pdffont.fds[i as usize].font_dim;
        if let Some(w) = ff::fix_fontmetrics(&mut dims, &ff_name) {
            self.pdffont_warn(&w);
        }
        self.host.hv.pdffont.fds[i as usize].font_dim = dims;
        let mut b = Vec::with_capacity(160);
        ff::write_fontmetrics(&dims, &mut b);
        self.pdffont_pdf_puts(&b);
        if ff_found {
            if self.pdf_omit_charset() == 0 && fm.is_subsetted() && fm.is_type1() {
                // Names may appear in any order.  Whether it is output can be controlled by the user at
                // runtime: see the C on why.
                let mut cs = Vec::with_capacity(512);
                for g in self.host.hv.pdffont.fds[i as usize].gl_tree.iter() {
                    cs.push(b'/');
                    cs.extend_from_slice(&g[..g.len().min(PRINTF_MAX - 1)]);
                }
                self.pdffont_pdf_puts(b"/CharSet (");
                self.pdffont_pdf_puts_long(&cs);
                self.pdffont_pdf_puts(b")\n");
            }
            let n = int_bytes(self.host.hv.pdffont.fds[i as usize].ff_objnum);
            if fm.is_type1() {
                self.pdffont_pdf_printf(&[b"/FontFile ", &n, b" 0 R\n"]);
            } else if fm.is_truetype() {
                self.pdffont_pdf_printf(&[b"/FontFile2 ", &n, b" 0 R\n"]);
            } else if fm.is_opentype() {
                self.pdffont_pdf_printf(&[b"/FontFile3 ", &n, b" 0 R\n"]);
            }
        }
        self.pdf_end_dict();
    }

    /// `write_fontdescriptors`: `fd_tree` in order of `comp_fd_entry`.
    fn pdffont_write_fontdescriptors(&mut self) {
        let tab = self.v_pdffont.fd_tab;
        let mut order: Vec<(Vec<u8>, i64, i64, i32)> = Vec::new();
        for i in 0..tab.len {
            let r = tab.get(&mut self.arena, i);
            if r.in_tree {
                let fm = self.pdffont_fm(r.fm);
                order.push((self.pdffont_bytes(fm.ff_name), fm.slant, fm.extend, i));
            }
        }
        order.sort();
        for (_, _, _, i) in order {
            self.pdffont_write_fontdescriptor(i);
        }
    }

    /// `write_fontdictionary`.
    fn pdffont_write_fontdictionary(&mut self, mut fo: Fo) -> Fo {
        let fm = self.pdffont_fm(fo.fm);
        let tfm_name = self.pdffont_bytes(fm.tfm_name);
        // write ToUnicode entry if needed.  Mind the C's parenthesis: `(... && strcmp(tfm_name, "dummy-space")) == 0`
        // is true for that font alone, whatever \pdfgentounicode says.
        if (self.fixed_gen_tounicode > 0 && !self.pdf_font_nobuiltin_tounicode[fo.tex_font])
            || tfm_name == b"dummy-space"
        {
            if let Some(fe) = fo.fe {
                let names = self.pdffont_fe_glyph_names(fe);
                let r = self.v_pdffont.fe_tab.get(&mut self.arena, fe);
                let enc = self.pdffont_bytes(r.name);
                fo.tounicode_objnum = self.pdffont_write_tounicode(&names, &tfm_name, Some(&enc));
            } else if fm.is_type1() {
                let builtin = self.host.hv.pdffont.fds[fo.fd as usize].builtin_glyph_names.take();
                let Some(names) = builtin else {
                    self.pdffont_fail(b"builtin glyph names is empty");
                };
                fo.tounicode_objnum = self.pdffont_write_tounicode(&names, &tfm_name, None);
                self.host.hv.pdffont.fds[fo.fd as usize].builtin_glyph_names = Some(names);
            }
        }
        self.pdf_begin_dict(fo.fo_objnum, 1);
        self.pdffont_pdf_puts(b"/Type /Font\n");
        self.pdffont_pdf_puts(b"/Subtype /");
        if fm.is_type1() {
            self.pdffont_pdf_printf(&[b"Type1", b"\n"]);
        } else if fm.is_truetype() {
            self.pdffont_pdf_printf(&[b"TrueType", b"\n"]);
        } else if fm.is_opentype() {
            self.pdffont_pdf_printf(&[b"Type1", b"\n"]);
        }
        self.pdffont_write_fontname(fo.fd, Some(b"BaseFont"));
        let d = self.v_pdffont.fd_tab.get(&mut self.arena, fo.fd);
        self.pdffont_pdf_printf(&[b"/FontDescriptor ", &int_bytes(d.fd_objnum), b" 0 R\n"]);
        let (a, b, c) = (int_bytes(fo.first_char), int_bytes(fo.last_char), int_bytes(fo.cw_objnum));
        self.pdffont_pdf_printf(&[b"/FirstChar ", &a, b"\n/LastChar ", &b, b"\n/Widths ", &c, b" 0 R\n"]);
        if fm.is_type1() || fm.is_opentype() {
            if let Some(fe) = fo.fe {
                let n = self.v_pdffont.fe_tab.get(&mut self.arena, fe).fe_objnum;
                if n != 0 {
                    self.pdffont_pdf_printf(&[b"/Encoding ", &int_bytes(n), b" 0 R\n"]);
                }
            }
        }
        if fo.tounicode_objnum != 0 {
            self.pdffont_pdf_printf(&[b"/ToUnicode ", &int_bytes(fo.tounicode_objnum), b" 0 R\n"]);
        }
        if self.pdf_font_attr[fo.tex_font] != S_EMPTY {
            self.pdf_print(self.pdf_font_attr[fo.tex_font]);
            self.pdffont_pdf_puts(b"\n");
        }
        self.pdf_end_dict();
        fo
    }

    /// `write_fontdictionaries`: `fo_tree` in order of `comp_fo_entry`.
    fn pdffont_write_fontdictionaries(&mut self) {
        let fos = core::mem::take(&mut self.host.hv.pdffont.fos);
        let mut keyed: Vec<(Vec<u8>, Fo)> = Vec::with_capacity(fos.len());
        for fo in fos {
            let fm = self.pdffont_fm(fo.fm);
            keyed.push((self.pdffont_bytes(fm.tfm_name), fo));
        }
        keyed.sort_by(|a, b| a.0.cmp(&b.0));
        for (_, fo) in keyed {
            let _ = self.pdffont_write_fontdictionary(fo);
        }
    }

    /// External: writefont.c via rstex-fontfile.  Final flush of all font related stuff.
    pub fn write_fontstuff(&mut self) {
        let _p = crate::prof_scope!("write_fontstuff");
        self.pdffont_fds_sync();
        self.pdffont_write_fontdescriptors();
        self.pdffont_write_fontencodings(); // see writeenc.c
        self.pdffont_write_fontdictionaries();
    }

    /// `pdfcharmarked`, for all codes at once.
    fn pdffont_marked(&self, f: i32) -> CodeSet {
        let mut m = [0u8; 32];
        for (i, b) in m.iter_mut().enumerate() {
            *b = self.pdf_char_used[f].at(i as i32) as u8;
        }
        CodeSet(m)
    }

    /// `create_charwidth_array` and `write_charwidth_array`: the object number.
    fn pdffont_write_charwidth_array(&mut self, f: i32, first_char: i32, last_char: i32) -> i32 {
        let cw_objnum = self.pdf_new_objnum();
        self.pdf_begin_obj(cw_objnum, 1);
        self.pdffont_pdf_puts(b"[");
        let mut i = first_char;
        while i <= last_char {
            let w = self.get_charwidth(f, i);
            let w = self.divide_scaled(w, self.pdf_font_size[f], 4);
            // see adv_char_width() in pdftex.web
            self.pdffont_pdf_printf(&[&int_bytes(w / 10)]);
            let j = w % 10;
            if j != 0 {
                self.pdffont_pdf_printf(&[b".", &int_bytes(j)]);
            }
            if i != last_char {
                self.pdffont_pdf_puts(b" ");
            }
            i += 1;
        }
        self.pdffont_pdf_puts(b"]\n");
        self.pdf_end_obj();
        cw_objnum
    }

    /// `create_fontdictionary`.
    fn pdffont_create_fontdictionary(&mut self, fm_i: i32, font_objnum: i32, f: i32) {
        self.pdffont_fds_sync();
        let fm = self.pdffont_fm(fm_i);
        let marked = self.pdffont_marked(f);
        // set fo->first_char and fo->last_char from f
        let (first_char, last_char) = ff::get_char_range(&marked, self.font_bc[f], self.font_ec[f]);
        let mut fe: Option<i32> = None;
        if fm.is_reencoded() {
            // at least the map entry tells so
            let e = self.pdffont_get_fe_entry(fm.encname);
            fe = Some(e);
            if fm.is_type1() || fm.is_opentype() {
                let mut r = self.v_pdffont.fe_tab.get(&mut self.arena, e);
                if r.fe_objnum == 0 {
                    r.fe_objnum = self.pdf_new_objnum(); // then it will be written out
                }
                // mark encoding pairs used by TeX to optimize encoding vector
                let mut tx = CodeSet(r.tx);
                ff::mark_chars(&mut tx, &marked, first_char, last_char);
                r.tx = tx.0;
                self.v_pdffont.fe_tab.set(&mut self.arena, e, r);
            }
        }
        let fd: i32;
        let cw_objnum: i32;
        if !fm.is_builtin() {
            if fm.is_type1() {
                let found = self.pdffont_lookup_fd_entry(fm.ff_name, fm.slant, fm.extend);
                fd = if found < 0 { self.pdffont_create_fontdescriptor(fm_i, fe, f, true) } else { found };
            } else {
                fd = self.pdffont_create_fontdescriptor(fm_i, fe, f, false);
            }
            cw_objnum = self.pdffont_write_charwidth_array(f, first_char, last_char);
            if let Some(e) = fe {
                if fm.is_subsetted() {
                    // mark glyphs from TeX (externally reencoded characters)
                    let names = self.pdffont_fe_glyph_names(e);
                    let gl = &mut self.host.hv.pdffont.fds[fd as usize].gl_tree;
                    ff::mark_reenc_glyphs(&names, &marked, first_char, last_char, gl);
                }
                if !fm.is_type1() {
                    // mark reencoded characters as chars on TeX level
                    let d = &mut self.host.hv.pdffont.fds[fd as usize];
                    let mut tx = d.tx_tree.unwrap_or_default();
                    ff::mark_chars(&mut tx, &marked, first_char, last_char);
                    d.tx_tree = Some(tx);
                    if fm.is_truetype() {
                        d.write_ttf_glyph_names = true;
                    }
                }
            } else {
                // mark non-reencoded characters as chars on TeX level
                let d = &mut self.host.hv.pdffont.fds[fd as usize];
                let mut tx = d.tx_tree.unwrap_or_default();
                ff::mark_chars(&mut tx, &marked, first_char, last_char);
                d.tx_tree = Some(tx);
            }
            if !fm.is_type1() {
                self.pdffont_write_fontdescriptor(fd);
            }
        } else {
            // builtin fonts still need the /Widths array and /FontDescriptor (to avoid error 'font FOO contains
            // bad /BBox')
            cw_objnum = self.pdffont_write_charwidth_array(f, first_char, last_char);
            fd = self.pdffont_create_fontdescriptor(fm_i, fe, f, false);
            self.pdffont_write_fontdescriptor(fd);
            if !fm.is_std_t1font() {
                let ps = self.pdffont_bytes_opt(fm.ps_name).unwrap_or_default();
                let m = [
                    b"font `".as_slice(),
                    &ps,
                    b"' is not a standard font; I suppose it is available to your PDF viewer then",
                ]
                .concat();
                self.pdffont_warn(&m);
            }
        }
        let fo = Fo {
            fo_objnum: font_objnum,
            tex_font: f,
            fm: fm_i,
            fd,
            fe,
            cw_objnum,
            first_char,
            last_char,
            tounicode_objnum: 0,
        };
        if fm.is_type1() {
            self.host.hv.pdffont.fos.push(fo); // register_fo_entry
        } else {
            let _ = self.pdffont_write_fontdictionary(fo);
        }
    }

    /// External: writefont.c via rstex-fontfile
    pub fn do_pdf_font(&mut self, font_objnum: i32, f: i32) {
        let _p = crate::prof_scope!("do_pdf_font");
        // font_is_used: avoid failed assertion in create_fontdictionary
        let marked = self.pdffont_marked(f);
        let (first, last) = ff::get_char_range(&marked, self.font_bc[f], self.font_ec[f]);
        if first > last {
            return;
        }
        if !self.isscalable(f) {
            // fm == NULL || is_pk(fm): writet3.  PK and Type 3 fonts are dropped (PORTING, section 19), so this is
            // writepk's failure to find the file.  The oracle may find one, or have mktexpk make it.
            if self.pdf_font_map[f] != FM_DUMMY {
                let i = self.pdffont_fm_index(f);
                let fm = self.pdffont_fm(i);
                if fm.is_reencoded() {
                    let _ = self.pdffont_get_fe_entry(fm.encname);
                }
            }
            let name = self.pdffont_make_cstring(self.font_name[f]);
            // kpse_magstep_fix(round(fixedpkresolution * (((float) pdffontsize[f]) / fontdsize[f])), ...)
            let d = self.fixed_pk_resolution as f32 * (self.pdf_font_size[f] as f32 / self.font_dsize[f] as f32);
            let dpi = magstep_fix(round(d as f64), self.fixed_pk_resolution);
            let m = [b"Font ".as_slice(), &name, b" at ", &int_bytes(dpi), b" not found"].concat();
            self.host.hv.pdffont.cur_file_name = Some(name);
            self.pdffont_fail(&m);
        }
        let i = self.pdffont_fm_index(f);
        self.pdffont_create_fontdictionary(i, font_objnum, f);
    }

    // ---- epdf.c ----

    /// `is_subsetable`.
    pub fn pdffont_is_subsetable(&mut self, fm: i32) -> bool {
        self.pdffont_fm(fm).is_subsetted()
    }

    /// `epdf_create_fontdescriptor`: an index, which is `fd` for the rest of these.
    pub fn pdffont_epdf_create_fontdescriptor(&mut self, fm_i: i32, stem_v: i32) -> i32 {
        let mut fm = self.pdffont_fm(fm_i);
        let found = self.pdffont_lookup_fd_entry(fm.ff_name, fm.slant, fm.extend);
        if found >= 0 {
            return found;
        }
        fm.in_use = 1;
        self.v_pdffont.fm_tab.set(&mut self.arena, fm_i, fm);
        let i = self.pdffont_new_fd(fm_i, true);
        let mut r = self.v_pdffont.fd_tab.get(&mut self.arena, i);
        r.fd_objnum = self.pdf_new_objnum();
        // stemV must be copied
        r.stem_v = stem_v;
        r.stem_v_set = true;
        self.v_pdffont.fd_tab.set(&mut self.arena, i, r);
        i
    }

    /// `get_fd_objnum`.
    pub fn pdffont_get_fd_objnum(&mut self, fd: i32) -> i32 {
        self.v_pdffont.fd_tab.get(&mut self.arena, fd).fd_objnum
    }

    /// `get_fn_objnum`.
    pub fn pdffont_get_fn_objnum(&mut self, fd: i32) -> i32 {
        let mut r = self.v_pdffont.fd_tab.get(&mut self.arena, fd);
        if r.fn_objnum == 0 {
            r.fn_objnum = self.pdf_new_objnum();
            self.v_pdffont.fd_tab.set(&mut self.arena, fd, r);
        }
        r.fn_objnum
    }

    /// `epdf_mark_glyphs`.  Input charset from xpdf is a string of glyph names including leading slashes, with
    /// optional generic spaces, including tabs, \n, \r, and \f, preceding or between them.  A name twice is
    /// harmless: `gl_tree` is a set when it is built.
    pub fn pdffont_epdf_mark_glyphs(&mut self, fd: i32, charset: &[u8]) {
        fn space(c: u8) -> bool {
            matches!(c, b' ' | b'\t' | b'\n' | b'\r' | 12)
        }
        let mut b = 0;
        while b < charset.len() && space(charset[b]) {
            b += 1;
        }
        let mut s = b + 1;
        while s < charset.len() {
            let mut p = s;
            while p < charset.len() && charset[p] != b'/' && !space(charset[p]) {
                p += 1;
            }
            let name = &charset[s..p];
            if p < charset.len() && space(charset[p]) {
                p += 1;
                while p < charset.len() && space(charset[p]) {
                    p += 1;
                }
            }
            let name = self.pdffont_pool_add(name);
            let mut t = self.v_pdffont.gl_items;
            self.arena_push(&mut t, GlItem { fd, name });
            self.v_pdffont.gl_items = t;
            s = p + 1;
        }
    }

    /// `embed_whole_font`.
    pub fn pdffont_embed_whole_font(&mut self, fd: i32) {
        let mut r = self.v_pdffont.fd_tab.get(&mut self.arena, fd);
        r.all_glyphs = true;
        self.v_pdffont.fd_tab.set(&mut self.arena, fd, r);
    }

    // ---- §705: subroutines to deal with expanded fonts for HZ-algorithm ----

    /// §705
    pub fn set_tag_code(&mut self, f: i32, c: i32, i: i32) {
        if self.is_valid_char(f, c) {
            let mut fixedi = abs(self.fix_int(i, -7, 0));
            let k = self.char_base[f] + c;
            if fixedi >= 4 {
                if self.char_tag(self.char_info(f, c)) == EXT_TAG {
                    let v = self.op_byte(self.char_info(f, c)) - EXT_TAG;
                    self.font_info[k].set_q2(v);
                }
                fixedi -= 4;
            }
            if fixedi >= 2 {
                if self.char_tag(self.char_info(f, c)) == LIST_TAG {
                    let v = self.op_byte(self.char_info(f, c)) - LIST_TAG;
                    self.font_info[k].set_q2(v);
                }
                fixedi -= 2;
            }
            if fixedi >= 1 {
                if self.char_tag(self.char_info(f, c)) == LIG_TAG {
                    let v = self.op_byte(self.char_info(f, c)) - LIG_TAG;
                    self.font_info[k].set_q2(v);
                }
            }
        }
    }

    /// §705
    pub fn set_no_ligatures(&mut self, f: i32) {
        for c in self.font_bc[f]..=self.font_ec[f] {
            if self.char_exists(self.orig_char_info(f, c)) {
                if self.char_tag(self.orig_char_info(f, c)) == LIG_TAG {
                    let v = self.op_byte(self.orig_char_info(f, c)) - LIG_TAG;
                    let k = self.char_base[f] + c;
                    self.font_info[k].set_q2(v);
                }
            }
        }
    }

    /// §705
    pub fn init_font_base(&mut self, v: i32) -> i32 {
        let i = self.pdf_get_mem(256);
        for j in 0..=255 {
            self.pdf_mem[i + j] = v;
        }
        i
    }

    /// §705
    pub fn set_lp_code(&mut self, f: i32, c: i32, i: i32) {
        if self.pdf_font_lp_base[f] == 0 {
            self.pdf_font_lp_base[f] = self.init_font_base(0);
        }
        let v = self.fix_int(i, -1000, 1000);
        self.pdf_mem[self.pdf_font_lp_base[f] + c] = v;
    }

    /// §705
    pub fn set_rp_code(&mut self, f: i32, c: i32, i: i32) {
        if self.pdf_font_rp_base[f] == 0 {
            self.pdf_font_rp_base[f] = self.init_font_base(0);
        }
        let v = self.fix_int(i, -1000, 1000);
        self.pdf_mem[self.pdf_font_rp_base[f] + c] = v;
    }

    /// §705
    pub fn set_ef_code(&mut self, f: i32, c: i32, i: i32) {
        if self.pdf_font_ef_base[f] == 0 {
            self.pdf_font_ef_base[f] = self.init_font_base(1000);
        }
        let v = self.fix_int(i, 0, 1000);
        self.pdf_mem[self.pdf_font_ef_base[f] + c] = v;
    }

    /// §705
    pub fn set_kn_bs_code(&mut self, f: i32, c: i32, i: i32) {
        if self.pdf_font_kn_bs_base[f] == 0 {
            self.pdf_font_kn_bs_base[f] = self.init_font_base(0);
        }
        let v = self.fix_int(i, -1000, 1000);
        self.pdf_mem[self.pdf_font_kn_bs_base[f] + c] = v;
    }

    /// §705
    pub fn set_st_bs_code(&mut self, f: i32, c: i32, i: i32) {
        if self.pdf_font_st_bs_base[f] == 0 {
            self.pdf_font_st_bs_base[f] = self.init_font_base(0);
        }
        let v = self.fix_int(i, -1000, 1000);
        self.pdf_mem[self.pdf_font_st_bs_base[f] + c] = v;
    }

    /// §705
    pub fn set_sh_bs_code(&mut self, f: i32, c: i32, i: i32) {
        if self.pdf_font_sh_bs_base[f] == 0 {
            self.pdf_font_sh_bs_base[f] = self.init_font_base(0);
        }
        let v = self.fix_int(i, -1000, 1000);
        self.pdf_mem[self.pdf_font_sh_bs_base[f] + c] = v;
    }

    /// §705
    pub fn set_kn_bc_code(&mut self, f: i32, c: i32, i: i32) {
        if self.pdf_font_kn_bc_base[f] == 0 {
            self.pdf_font_kn_bc_base[f] = self.init_font_base(0);
        }
        let v = self.fix_int(i, -1000, 1000);
        self.pdf_mem[self.pdf_font_kn_bc_base[f] + c] = v;
    }

    /// §705
    pub fn set_kn_ac_code(&mut self, f: i32, c: i32, i: i32) {
        if self.pdf_font_kn_ac_base[f] == 0 {
            self.pdf_font_kn_ac_base[f] = self.init_font_base(0);
        }
        let v = self.fix_int(i, -1000, 1000);
        self.pdf_mem[self.pdf_font_kn_ac_base[f] + c] = v;
    }

    /// §705 adjust the interword glue `g` after a character `p`.
    pub fn adjust_interword_glue(&mut self, p: i32, g: i32) {
        if !(!self.is_char_node(g) && self.ty(g) == GLUE_NODE) {
            self.pdf_warning(b"adjust_interword_glue", b"g is not a glue", true, true);
            return;
        }
        let mut c = NON_CHAR; // no char before interword glue yet
        let mut f = 0;
        // set_char_and_font(p): set f and c if p is a char or ligature
        if self.is_char_node(p) {
            c = self.character(p);
            f = self.font(p);
        } else if self.ty(p) == LIGATURE_NODE {
            c = self.character(self.lig_char(p));
            f = self.font(self.lig_char(p));
        } else if self.ty(p) == KERN_NODE && self.subtype(p) == AUTO_KERN && self.save_tail != NULL {
            let mut r = self.save_tail;
            while self.link(r) != NULL && self.link(r) != p {
                r = self.link(r);
            }
            if self.link(r) == p {
                // set f and c if r is a char or ligature
                if self.is_char_node(r) {
                    c = self.character(r);
                    f = self.font(r);
                } else if self.ty(r) == LIGATURE_NODE {
                    c = self.character(self.lig_char(r));
                    f = self.font(self.lig_char(r));
                }
            }
        }
        if c == NON_CHAR {
            return;
        }
        let kn = self.get_kn_bs_code(f, c);
        let st = self.get_st_bs_code(f, c);
        let sh = self.get_sh_bs_code(f, c);
        if kn != 0 || st != 0 || sh != 0 {
            let q = self.new_spec(self.glue_ptr(g));
            self.delete_glue_ref(self.glue_ptr(g));
            let d = self.round_xn_over_d(self.quad(f), kn, 1000);
            self.set_width(q, self.width(q) + d);
            let d = self.round_xn_over_d(self.quad(f), st, 1000);
            self.set_stretch(q, self.stretch(q) + d);
            let d = self.round_xn_over_d(self.quad(f), sh, 1000);
            self.set_shrink(q, self.shrink(q) + d);
            self.set_glue_ptr(g, q);
        }
    }

    /// §705 return a pointer to an auto kern node, or null.
    pub fn get_auto_kern(&mut self, f: i32, l: i32, r: i32) -> i32 {
        self.pdfassert(l >= 0 && r >= 0);
        if self.pdf_append_kern() <= 0 && self.pdf_prepend_kern() <= 0 {
            return NULL;
        }
        let mut tmp_w = 0;
        if self.pdf_append_kern() > 0 && l < NON_CHAR {
            let k = self.get_kn_ac_code(f, l);
            if k != 0 {
                tmp_w = self.round_xn_over_d(self.quad(f), k, 1000);
            }
        }
        if self.pdf_prepend_kern() > 0 && r < NON_CHAR {
            let k = self.get_kn_bc_code(f, r);
            if k != 0 {
                tmp_w += self.round_xn_over_d(self.quad(f), k, 1000);
            }
        }
        if tmp_w != 0 {
            let p = self.new_kern(tmp_w);
            self.set_subtype(p, AUTO_KERN);
            return p;
        }
        NULL
    }

    /// §705
    pub fn expand_font_name(&mut self, f: i32, e: i32) -> i32 {
        let old_setting = self.selector;
        self.selector = NEW_STRING;
        self.print(self.font_name[f]);
        if e > 0 {
            self.print(b'+' as i32); // minus sign will be printed by print_int
        }
        self.print_int(e);
        self.selector = old_setting;
        self.make_string()
    }

    /// §705 creates an expanded font from the base font; doesn't load expanded tfm at all.
    pub fn auto_expand_font(&mut self, f: i32, e: i32) -> i32 {
        let k = self.font_ptr + 1;
        self.font_ptr += 1;
        if self.font_ptr >= self.font_max {
            self.overflow(b"maximum internal font number (font_max)", self.font_max);
        }
        self.font_name[k] = self.expand_font_name(f, e);
        self.font_area[k] = self.font_area[f];
        self.set_font_id_text(k, self.font_id_text(f));
        self.hyphen_char[k] = self.hyphen_char[f];
        self.skew_char[k] = self.skew_char[f];
        self.font_bchar[k] = self.font_bchar[f];
        self.font_false_bchar[k] = self.font_false_bchar[f];
        self.font_bc[k] = self.font_bc[f];
        self.font_ec[k] = self.font_ec[f];
        self.font_size[k] = self.font_size[f];
        self.font_dsize[k] = self.font_dsize[f];
        self.font_params[k] = self.font_params[f];
        self.font_glue[k] = self.font_glue[f];
        self.bchar_label[k] = self.bchar_label[f];

        self.char_base[k] = self.char_base[f];
        self.height_base[k] = self.height_base[f];
        self.depth_base[k] = self.depth_base[f];
        self.lig_kern_base[k] = self.lig_kern_base[f];
        self.exten_base[k] = self.exten_base[f];
        self.param_base[k] = self.param_base[f];

        let nw = self.height_base[f] - self.width_base[f];
        let ni = self.lig_kern_base[f] - self.italic_base[f];
        let nk = self.exten_base[f] - (self.kern_base[f] + KERN_BASE_OFFSET);
        if self.fmem_ptr + nw + ni + nk >= self.font_mem_size {
            self.overflow(b"number of words of font memory (font_mem_size)", self.font_mem_size);
        }
        self.width_base[k] = self.fmem_ptr;
        self.italic_base[k] = self.width_base[k] + nw;
        self.kern_base[k] = self.italic_base[k] + ni - KERN_BASE_OFFSET;
        self.fmem_ptr = self.fmem_ptr + nw + ni + nk;

        for i in 0..nw {
            let v = self.font_info[self.width_base[f] + i].sc();
            let v = self.round_xn_over_d(v, 1000 + e, 1000);
            self.font_info[self.width_base[k] + i].set_sc(v);
        }
        for i in 0..ni {
            let v = self.font_info[self.italic_base[f] + i].sc();
            let v = self.round_xn_over_d(v, 1000 + e, 1000);
            self.font_info[self.italic_base[k] + i].set_sc(v);
        }
        for i in 0..nk {
            let v = self.font_info[self.kern_base[f] + KERN_BASE_OFFSET + i].sc();
            let v = self.round_xn_over_d(v, 1000 + e, 1000);
            self.font_info[self.kern_base[k] + KERN_BASE_OFFSET + i].set_sc(v);
        }
        k
    }

    /// §705 set expansion-related parameters for an expanded font `k`, based on the base font `f` and the
    /// expansion amount `e`.
    pub fn copy_expand_params(&mut self, k: i32, f: i32, e: i32) {
        if self.pdf_font_rp_base[f] == 0 {
            self.pdf_font_rp_base[f] = self.init_font_base(0);
        }
        if self.pdf_font_lp_base[f] == 0 {
            self.pdf_font_lp_base[f] = self.init_font_base(0);
        }
        if self.pdf_font_ef_base[f] == 0 {
            self.pdf_font_ef_base[f] = self.init_font_base(1000);
        }
        self.pdf_font_expand_ratio[k] = e;
        self.pdf_font_step[k] = self.pdf_font_step[f];
        self.pdf_font_auto_expand[k] = self.pdf_font_auto_expand[f];
        self.pdf_font_blink[k] = f;
        self.pdf_font_lp_base[k] = self.pdf_font_lp_base[f];
        self.pdf_font_rp_base[k] = self.pdf_font_rp_base[f];
        self.pdf_font_ef_base[k] = self.pdf_font_ef_base[f];

        if self.pdf_font_kn_bs_base[f] == 0 {
            self.pdf_font_kn_bs_base[f] = self.init_font_base(0);
        }
        if self.pdf_font_st_bs_base[f] == 0 {
            self.pdf_font_st_bs_base[f] = self.init_font_base(0);
        }
        if self.pdf_font_sh_bs_base[f] == 0 {
            self.pdf_font_sh_bs_base[f] = self.init_font_base(0);
        }
        if self.pdf_font_kn_bc_base[f] == 0 {
            self.pdf_font_kn_bc_base[f] = self.init_font_base(0);
        }
        if self.pdf_font_kn_ac_base[f] == 0 {
            self.pdf_font_kn_ac_base[f] = self.init_font_base(0);
        }
        self.pdf_font_kn_bs_base[k] = self.pdf_font_kn_bs_base[f];
        self.pdf_font_st_bs_base[k] = self.pdf_font_st_bs_base[f];
        self.pdf_font_sh_bs_base[k] = self.pdf_font_sh_bs_base[f];
        self.pdf_font_kn_bc_base[k] = self.pdf_font_kn_bc_base[f];
        self.pdf_font_kn_ac_base[k] = self.pdf_font_kn_ac_base[f];
    }

    /// `font_area[k] <> non_existent_path`.  WEB compares string numbers; the area of a scanned font ends in a
    /// separator, so no other string has these contents.
    fn pdffont_has_area(&self, k: i32) -> bool {
        let s = self.font_area[k];
        self.str_pool.range(self.str_start[s], self.str_start[s + 1]) != NON_EXISTENT_PATH
    }

    /// §705 looks up for a TFM with name `s` loaded at `fs` size; if found then flushes `s`.
    pub fn tfm_lookup(&mut self, s: i32, fs: i32) -> i32 {
        if fs != 0 {
            for k in FONT_BASE + 1..=self.font_ptr {
                if self.pdffont_has_area(k) && self.str_eq_str(self.font_name[k], s) && self.font_size[k] == fs {
                    self.flush_str(s);
                    return k;
                }
            }
        } else {
            for k in FONT_BASE + 1..=self.font_ptr {
                if self.pdffont_has_area(k) && self.str_eq_str(self.font_name[k], s) {
                    self.flush_str(s);
                    return k;
                }
            }
        }
        NULL_FONT
    }

    /// §705 loads font `f` expanded by `e` thousandths into font memory; `e` is nonzero and is a multiple of
    /// `pdf_font_step[f]`.
    pub fn load_expand_font(&mut self, f: i32, e: i32) -> i32 {
        let s = self.expand_font_name(f, e);
        let mut k = self.tfm_lookup(s, self.font_size[f]);
        if k == NULL_FONT {
            if self.pdf_font_auto_expand[f] {
                k = self.auto_expand_font(f, e);
            } else {
                k = self.read_font_info(NULL_CS, s, S_EMPTY, self.font_size[f]);
            }
        }
        if k != NULL_FONT {
            self.copy_expand_params(k, f, e);
        }
        k
    }

    /// §705 return the multiple of `pdf_font_step[f]` that is nearest to `e`.
    pub fn fix_expand_value(&mut self, f: i32, e: i32) -> i32 {
        let mut e = e;
        if e == 0 {
            return 0;
        }
        let neg: bool;
        let max_expand: i32;
        if e < 0 {
            e = -e;
            neg = true;
            max_expand = -self.pdf_font_expand_ratio[self.pdf_font_shrink[f]];
        } else {
            neg = false;
            max_expand = self.pdf_font_expand_ratio[self.pdf_font_stretch[f]];
        }
        if e > max_expand {
            e = max_expand;
        } else {
            let step = self.pdf_font_step[f];
            if e % step > 0 {
                e = step * self.round_xn_over_d(e, 1, step);
            }
        }
        if neg {
            e = -e;
        }
        e
    }

    /// §705 look up and create if not found an expanded version of `f`; `f` is an expandable font; `e` is
    /// nonzero and is a multiple of `pdf_font_step[f]`.
    pub fn get_expand_font(&mut self, f: i32, e: i32) -> i32 {
        let mut k = self.pdf_font_elink[f];
        while k != NULL_FONT {
            if self.pdf_font_expand_ratio[k] == e {
                return k;
            }
            k = self.pdf_font_elink[k];
        }
        k = self.load_expand_font(f, e);
        self.pdf_font_elink[k] = self.pdf_font_elink[f];
        self.pdf_font_elink[f] = k;
        self.pack_forget_subst_memo();
        k
    }

    /// §705 looks up for font `f` expanded by `e` thousandths, `e` is an arbitrary value between max stretch and
    /// max shrink of `f`; if not found then creates it.
    pub fn expand_font(&mut self, f: i32, e: i32) -> i32 {
        if e == 0 {
            return f;
        }
        let e = self.fix_expand_value(f, e);
        if e == 0 {
            return f;
        }
        if self.pdf_font_elink[f] == NULL_FONT {
            self.pdf_error(b"font expansion", b"uninitialized pdf_font_elink");
        }
        self.get_expand_font(f, e)
    }

    /// §705 expand a font with given parameters.
    pub fn set_expand_params(&mut self, f: i32, auto_expand: bool, stretch_limit: i32, shrink_limit: i32, font_step: i32, expand_ratio: i32) {
        self.pack_forget_subst_memo();
        self.pdf_font_step[f] = font_step;
        self.pdf_font_auto_expand[f] = auto_expand;
        if stretch_limit > 0 {
            self.pdf_font_stretch[f] = self.get_expand_font(f, stretch_limit);
        }
        if shrink_limit > 0 {
            self.pdf_font_shrink[f] = self.get_expand_font(f, -shrink_limit);
        }
        if expand_ratio != 0 {
            self.pdf_font_expand_ratio[f] = expand_ratio;
        }
    }

    /// §705
    pub fn vf_expand_local_fonts(&mut self, f: i32) {
        self.pdfassert(self.pdf_font_type[f] == VIRTUAL_FONT_TYPE);
        for k in 0..self.vf_local_font_num[f] {
            let lf = self.vf_i_fnts[self.vf_default_font[f] + k];
            self.set_expand_params(
                lf,
                self.pdf_font_auto_expand[f],
                self.pdf_font_expand_ratio[self.pdf_font_stretch[f]],
                -self.pdf_font_expand_ratio[self.pdf_font_shrink[f]],
                self.pdf_font_step[f],
                self.pdf_font_expand_ratio[f],
            );
            if self.pdf_font_type[lf] == VIRTUAL_FONT_TYPE {
                self.vf_expand_local_fonts(lf);
            }
        }
    }

    /// §705 read font expansion spec and load expanded font.
    pub fn read_expand_font(&mut self) {
        // read font expansion parameters
        self.scan_font_ident();
        let f = self.cur_val;
        if f == NULL_FONT {
            self.pdf_error(b"font expansion", b"invalid font identifier");
        }
        if self.pdf_font_blink[f] != NULL_FONT {
            self.pdf_error(b"font expansion", b"\\pdffontexpand cannot be used this way (the base font has been expanded)");
        }
        self.scan_optional_equals();
        self.scan_int();
        let mut stretch_limit = self.fix_int(self.cur_val, 0, 1000);
        self.scan_int();
        let mut shrink_limit = self.fix_int(self.cur_val, 0, 500);
        self.scan_int();
        let font_step = self.fix_int(self.cur_val, 0, 100);
        if font_step == 0 {
            self.pdf_error(b"font expansion", b"invalid step");
        }
        stretch_limit -= stretch_limit % font_step;
        if stretch_limit < 0 {
            stretch_limit = 0;
        }
        shrink_limit -= shrink_limit % font_step;
        if shrink_limit < 0 {
            shrink_limit = 0;
        }
        if stretch_limit == 0 && shrink_limit == 0 {
            self.pdf_error(b"font expansion", b"invalid limit(s)");
        }
        let mut auto_expand = false;
        if self.scan_keyword(b"autoexpand") {
            auto_expand = true;
            // §469 Scan an optional space
            self.get_x_token();
            if self.cur_cmd != SPACER {
                self.back_input();
            }
        }

        // check if the font can be expanded
        if self.pdf_font_expand_ratio[f] != 0 {
            self.pdf_error(b"font expansion", b"this font has been expanded by another font so it cannot be used now");
        }
        if self.pdf_font_step[f] != 0 {
            // this font has been expanded, ensure the expansion parameters are identical
            if self.pdf_font_step[f] != font_step {
                self.pdf_error(b"font expansion", b"font has been expanded with different expansion step");
            }
            if (self.pdf_font_stretch[f] == NULL_FONT && stretch_limit != 0)
                || (self.pdf_font_stretch[f] != NULL_FONT
                    && self.pdf_font_expand_ratio[self.pdf_font_stretch[f]] != stretch_limit)
            {
                self.pdf_error(b"font expansion", b"font has been expanded with different stretch limit");
            }
            if (self.pdf_font_shrink[f] == NULL_FONT && shrink_limit != 0)
                || (self.pdf_font_shrink[f] != NULL_FONT
                    && -self.pdf_font_expand_ratio[self.pdf_font_shrink[f]] != shrink_limit)
            {
                self.pdf_error(b"font expansion", b"font has been expanded with different shrink limit");
            }
            if self.pdf_font_auto_expand[f] != auto_expand {
                self.pdf_error(b"font expansion", b"font has been expanded with different auto expansion value");
            }
        } else {
            if self.pdf_font_type[f] != NEW_FONT_TYPE && self.pdf_font_type[f] != VIRTUAL_FONT_TYPE {
                self.pdf_warning(b"font expansion", b"font should be expanded before its first use", true, true);
            }
            self.set_expand_params(f, auto_expand, stretch_limit, shrink_limit, font_step, 0);
            if self.pdf_font_type[f] == VIRTUAL_FONT_TYPE {
                self.vf_expand_local_fonts(f);
            }
        }
    }

    // ---- §706: robust letter spacing using virtual font ----

    /// §706.  `w`, `r`, `vf_z`, `vf_alpha` are `i64`: the oracle's are, and a large `\fontdimen6` goes past 2^31.
    pub fn letter_space_font(&mut self, u: i32, f: i32, e: i32) -> i32 {
        // read a new font and expand the character widths
        let k = self.read_font_info(u, self.font_name[f], S_EMPTY, self.font_size[f]);
        if self.scan_keyword(b"nolig") {
            self.set_no_ligatures(k); // disable ligatures for letter-spaced fonts
        }
        let nw = self.height_base[k] - self.width_base[k];

        if self.quad(k) == 0 && self.quad(f) > 0 {
            let q = self.quad(f);
            self.set_param(QUAD_CODE, k, q);
        }
        if self.quad(k) == 0 {
            self.pdf_warning(b"\\letterspacefont", b"font has zero em size (\\fontdimen6)", true, true);
        }

        for i in 0..nw {
            let d = self.round_xn_over_d(self.quad(k), e, 1000);
            let v = self.font_info[self.width_base[k] + i].sc() + d;
            self.font_info[self.width_base[k] + i].set_sc(v);
        }

        // append, e.g., '+100ls' to font name
        self.str_room(self.length(self.font_name[k]) + 7); // abs(e) <= 1000
        let old_setting = self.selector;
        self.selector = NEW_STRING;
        self.print(self.font_name[k]);
        if e > 0 {
            self.print(b'+' as i32); // minus sign will be printed by print_int
        }
        self.print_int(e);
        self.print_s(b"ls");
        self.selector = old_setting;
        self.font_name[k] = self.make_string();

        // create the corresponding virtual font
        self.allocvffnts();
        self.vf_e_fnts[self.vf_nf] = 0;
        self.vf_i_fnts[self.vf_nf] = f;
        self.vf_nf += 1;
        self.vf_local_font_num[k] = 1;
        self.vf_default_font[k] = self.vf_nf - 1;
        self.pdf_font_type[k] = VIRTUAL_FONT_TYPE;

        let mut vf_z: i64 = self.font_size[f] as i64;
        // vf_replace_z
        let mut vf_alpha: i64 = 16;
        while vf_z >= 0o40000000 {
            vf_z /= 2;
            vf_alpha += vf_alpha;
        }
        let vf_beta: i64 = 256 / vf_alpha;
        vf_alpha *= vf_z;
        let mut w: i64 = self.round_xn_over_d(self.quad(f), e, 2000) as i64;
        if w >= 0 {
            self.set_tmp_b0(0);
        } else {
            self.set_tmp_b0(255);
            w += vf_alpha;
        }
        let mut r: i64 = w * vf_beta;
        self.set_tmp_b1((r / vf_z) as i32);
        r %= vf_z;
        if r == 0 {
            self.set_tmp_b2(0);
        } else {
            r *= 256;
            self.set_tmp_b2((r / vf_z) as i32);
            r %= vf_z;
        }
        if r == 0 {
            self.set_tmp_b3(0);
        } else {
            r *= 256;
            self.set_tmp_b3((r / vf_z) as i32);
        }
        self.vf_packet_base[k] = self.new_vf_packet(k);
        // c is the global: the oracle's loop leaves it one past the end.
        self.c = self.font_bc[k];
        let for_end = self.font_ec[k];
        while self.c <= for_end {
            self.str_room(12);
            self.append_char(RIGHT1 + 3);
            self.append_char(self.tmp_b0());
            self.append_char(self.tmp_b1());
            self.append_char(self.tmp_b2());
            self.append_char(self.tmp_b3());
            if self.c < SET1 {
                self.append_char(self.c);
            } else {
                self.append_char(SET1);
                self.append_char(self.c);
            }
            self.append_char(RIGHT1 + 3);
            self.append_char(self.tmp_b0());
            self.append_char(self.tmp_b1());
            self.append_char(self.tmp_b2());
            self.append_char(self.tmp_b3());
            let s = self.make_string();
            self.storepacket(k, self.c, s);
            self.flush_str(s);
            self.c += 1;
        }
        k
    }

    /// The name for the frozen font identifier, as `new_font` makes it.
    fn pdffont_frozen_font_name(&mut self, u: i32) -> i32 {
        if u >= HASH_BASE {
            self.text(u)
        } else if u >= SINGLE_BASE {
            if u == NULL_CS { S_FONT } else { u - SINGLE_BASE }
        } else {
            let old_setting = self.selector;
            self.selector = NEW_STRING;
            self.print_s(b"FONT");
            self.print(u - ACTIVE_BASE);
            self.selector = old_setting;
            self.str_room(1);
            self.make_string()
        }
    }

    /// §706 letter-space a font by creating a virtual font.
    pub fn new_letterspaced_font(&mut self, a: i32) {
        self.get_r_token();
        let u = self.cur_cs;
        let t = self.pdffont_frozen_font_name(u);
        self.define(a, u, SET_FONT, NULL_FONT);
        self.scan_optional_equals();
        self.scan_font_ident();
        let k = self.cur_val;
        self.scan_int();
        let e = self.fix_int(self.cur_val, -1000, 1000);
        let f = self.letter_space_font(u, k, e);
        self.set_equiv(u, f);
        self.eqtb[FONT_ID_BASE + f] = self.eqtb[u];
        self.set_font_id_text(f, t);
    }

    /// §706
    pub fn is_letterspaced_font(&mut self, f: i32) -> bool {
        if self.pdf_font_type[f] != VIRTUAL_FONT_TYPE {
            return false;
        }
        let mut i = self.str_start[self.font_name[f] + 1] - 1;
        let j = self.str_start[self.font_name[f]];
        if self.str_pool.at(i - 1) != b'l' as i32 || self.str_pool.at(i) != b's' as i32 {
            return false;
        }
        i -= 2;
        while i >= j {
            if self.str_pool.at(i) < b'0' as i32 || self.str_pool.at(i) > b'9' as i32 {
                break;
            }
            i -= 1;
        }
        // done:
        if i < j {
            return false;
        }
        if self.str_pool.at(i) != b'+' as i32 && self.str_pool.at(i) != b'-' as i32 {
            return false;
        }
        true
    }

    /// §706 create a copy of `f` in the font mem.
    pub fn copy_font_info(&mut self, f: i32) -> i32 {
        if self.pdf_font_expand_ratio[f] != 0 || self.pdf_font_step[f] != 0 {
            self.pdf_error(b"\\pdfcopyfont", b"cannot copy an expanded font");
        }
        if self.is_letterspaced_font(f) {
            self.pdf_error(b"\\pdfcopyfont", b"cannot copy a letterspaced font");
        }

        let k = self.font_ptr + 1;
        self.font_ptr += 1;
        if self.font_ptr >= self.font_max {
            self.overflow(b"maximum internal font number (font_max)", self.font_max);
        }
        self.font_name[k] = self.font_name[f];
        // to avoid interferences with new_font() and tfm_lookup().  The oracle has it in the pool from the start;
        // here it is made when first needed.
        if self.v_pdffont.non_existent_path == 0 {
            self.v_pdffont.non_existent_path = self.pdffont_make_tex_string(NON_EXISTENT_PATH);
        }
        self.font_area[k] = self.v_pdffont.non_existent_path;
        self.hyphen_char[k] = self.hyphen_char[f];
        self.skew_char[k] = self.skew_char[f];
        self.font_bchar[k] = self.font_bchar[f];
        self.font_false_bchar[k] = self.font_false_bchar[f];
        self.font_bc[k] = self.font_bc[f];
        self.font_ec[k] = self.font_ec[f];
        self.font_size[k] = self.font_size[f];
        self.font_dsize[k] = self.font_dsize[f];
        self.font_params[k] = self.font_params[f];
        self.font_glue[k] = self.font_glue[f];
        self.bchar_label[k] = self.bchar_label[f];

        // set base addresses
        let bc = self.font_bc[f];
        let ec = self.font_ec[f];
        self.char_base[k] = self.fmem_ptr - bc;
        self.width_base[k] = self.char_base[k] + ec + 1;

        self.height_base[k] = self.width_base[k] + (self.height_base[f] - self.width_base[f]);
        self.depth_base[k] = self.height_base[k] + (self.depth_base[f] - self.height_base[f]);
        self.italic_base[k] = self.depth_base[k] + (self.italic_base[f] - self.depth_base[f]);
        self.lig_kern_base[k] = self.italic_base[k] + (self.lig_kern_base[f] - self.italic_base[f]);
        self.kern_base[k] = self.lig_kern_base[k] + (self.kern_base[f] - self.lig_kern_base[f]);
        self.exten_base[k] = self.kern_base[k] + (self.exten_base[f] - self.kern_base[f]);
        self.param_base[k] = self.exten_base[k] + (self.param_base[f] - self.exten_base[f]);

        // allocate memory for the new font k and copy data from f
        let lf = (self.param_base[f] - self.char_base[f]) + self.font_params[f] + 1;
        if self.fmem_ptr + lf >= self.font_mem_size {
            self.overflow(b"number of words of font memory (font_mem_size)", self.font_mem_size);
        }
        for i in 0..lf {
            self.font_info[self.char_base[k] + bc + i] = self.font_info[self.char_base[f] + bc + i];
        }
        self.fmem_ptr += lf;
        k
    }

    /// §706 make a font copy for further use with font expansion.
    pub fn make_font_copy(&mut self, a: i32) {
        self.get_r_token();
        let u = self.cur_cs;
        let t = self.pdffont_frozen_font_name(u);
        self.define(a, u, SET_FONT, NULL_FONT);
        self.scan_optional_equals();
        self.scan_font_ident();
        let k = self.cur_val;
        let f = self.copy_font_info(k);
        self.set_equiv(u, f);
        self.eqtb[FONT_ID_BASE + f] = self.eqtb[u];
        self.set_font_id_text(f, t);
    }

    // ---- §710-719: reading information from VF file ----

    /// §712
    pub fn vf_error(&mut self, filename: i32, msg: &[u8]) {
        self.str_room(self.length(filename) + 3);
        let old_setting = self.selector;
        self.selector = NEW_STRING;
        self.print(filename);
        self.print_s(b".vf");
        let s = self.make_string();
        self.selector = old_setting;
        let t = self.str_pool.range(self.str_start[s], self.str_start[s + 1]).to_vec();
        self.pdf_error(&t, msg);
    }

    /// §712 read a byte from `vf_file`.
    pub fn vf_byte(&mut self) -> i32 {
        let i = self.getc(VF_FILE);
        if i < 0 {
            self.pdf_error(b"vf", b"unexpected EOF or error");
        }
        i
    }

    /// §712 read `k` bytes as an signed integer from VF file.
    pub fn vf_read_signed(&mut self, k: i32) -> i32 {
        let mut k = k;
        self.pdfassert(k > 0 && k <= 4);
        let mut i = self.vf_byte();
        if i >= 128 {
            i -= 256;
        }
        k -= 1;
        while k > 0 {
            i = i * 256 + self.vf_byte();
            k -= 1;
        }
        i
    }

    /// §712 read `k` bytes as an unsigned integer from VF file.
    pub fn vf_read_unsigned(&mut self, k: i32) -> i32 {
        let mut k = k;
        self.pdfassert(k > 0 && k <= 4);
        let mut i = self.vf_byte();
        if k == 4 && i >= 128 {
            // §710 bad_vf: f here is the global, as WEB has it
            self.vf_error(self.font_name[self.f], b"number too big");
        }
        k -= 1;
        while k > 0 {
            i = i * 256 + self.vf_byte();
            k -= 1;
        }
        i
    }

    /// §712 print a warning message if an error occurs during processing local fonts in VF file.
    pub fn vf_local_font_warning(&mut self, f: i32, k: i32, s: &[u8]) {
        self.print_nl_s(s);
        self.print_s(b" in local font ");
        self.print(self.font_name[k]);
        self.print_s(b" in virtual font ");
        self.print(self.font_name[f]);
        self.print_s(b".vf ignored.");
    }

    /// §712 process a local font in VF file.
    pub fn vf_def_font(&mut self, f: i32) -> i32 {
        let cs = FourQuarters { b0: self.vf_byte(), b1: self.vf_byte(), b2: self.vf_byte(), b3: self.vf_byte() };
        let v = self.vf_read_signed(4);
        let fs = self.store_scaled_f(v, self.font_size[f]);
        let ds = self.vf_read_signed(4) / 0o20;
        let v = self.vf_byte();
        self.set_tmp_b0(v);
        let v = self.vf_byte();
        self.set_tmp_b1(v);
        while self.tmp_b0() > 0 {
            self.set_tmp_b0(self.tmp_b0() - 1);
            let _ = self.vf_byte(); // skip the font path
        }
        self.str_room(self.tmp_b1());
        while self.tmp_b1() > 0 {
            self.set_tmp_b1(self.tmp_b1() - 1);
            let v = self.vf_byte();
            self.append_char(v);
        }
        let s = self.make_string();
        let mut k = self.tfm_lookup(s, fs);
        if k == NULL_FONT {
            k = self.read_font_info(NULL_CS, s, S_EMPTY, fs);
        }
        if k != NULL_FONT {
            let fc = self.font_check[k];
            if (cs.b0 != 0 || cs.b1 != 0 || cs.b2 != 0 || cs.b3 != 0)
                && (fc.b0 != 0 || fc.b1 != 0 || fc.b2 != 0 || fc.b3 != 0)
                && (cs.b0 != fc.b0 || cs.b1 != fc.b1 || cs.b2 != fc.b2 || cs.b3 != fc.b3)
            {
                self.vf_local_font_warning(f, k, b"checksum mismatch");
            }
            if ds != self.font_dsize[k] {
                self.vf_local_font_warning(f, k, b"design size mismatch");
            }
            if self.pdf_font_step[f] != 0 {
                self.set_expand_params(
                    k,
                    self.pdf_font_auto_expand[f],
                    self.pdf_font_expand_ratio[self.pdf_font_stretch[f]],
                    -self.pdf_font_expand_ratio[self.pdf_font_shrink[f]],
                    self.pdf_font_step[f],
                    self.pdf_font_expand_ratio[f],
                );
            }
        }
        k
    }

    /// §712 process VF file with font internal number `f`.
    pub fn do_vf(&mut self, f: i32) {
        const SET4: i32 = SET1 + 3;
        const PUT4: i32 = PUT1 + 3;
        const RIGHT4: i32 = RIGHT1 + 3;
        const W4: i32 = W1 + 3;
        const X4: i32 = X1 + 3;
        const DOWN4: i32 = DOWN1 + 3;
        const Y4: i32 = Y1 + 3;
        const Z4: i32 = Z1 + 3;
        const XXX4: i32 = XXX1 + 3;

        self.pdf_font_type[f] = REAL_FONT_TYPE;
        if self.auto_expand_vf(f) {
            return; // auto-expanded virtual font
        }
        let mut stack_level = 0;
        // §713 Open vf_file, return if not found
        self.pack_file_name(self.font_name[f], S_EMPTY, S_DOT_VF);
        if !self.vf_b_open_in(VF_FILE) {
            return;
        }
        // §714 Process the preamble
        if self.vf_byte() != PRE {
            self.vf_error(self.font_name[f], b"PRE command expected");
        }
        if self.vf_byte() != VF_ID {
            self.vf_error(self.font_name[f], b"wrong id byte");
        }
        let mut cmd_length = self.vf_byte();
        for _ in 1..=cmd_length {
            let _ = self.vf_byte(); // skip the comment
        }
        let v = self.vf_byte();
        self.set_tmp_b0(v);
        let v = self.vf_byte();
        self.set_tmp_b1(v);
        let v = self.vf_byte();
        self.set_tmp_b2(v);
        let v = self.vf_byte();
        self.set_tmp_b3(v);
        let fc = self.font_check[f];
        let (b0, b1, b2, b3) = (self.tmp_b0(), self.tmp_b1(), self.tmp_b2(), self.tmp_b3());
        if (b0 != 0 || b1 != 0 || b2 != 0 || b3 != 0)
            && (fc.b0 != 0 || fc.b1 != 0 || fc.b2 != 0 || fc.b3 != 0)
            && (b0 != fc.b0 || b1 != fc.b1 || b2 != fc.b2 || b3 != fc.b3)
        {
            self.print_nl_s(b"checksum mismatch in font ");
            self.print(self.font_name[f]);
            self.print_s(b".vf ignored");
        }
        if self.vf_read_signed(4) / 0o20 != self.font_dsize[f] {
            self.print_nl_s(b"design size mismatch in font ");
            self.print(self.font_name[f]);
            self.print_s(b".vf ignored");
        }
        self.update_terminal();
        // §715 Process the font definitions
        let mut cmd = self.vf_byte();
        let save_vf_nf = self.vf_nf;
        while cmd >= FNT_DEF1 && cmd <= FNT_DEF1 + 3 {
            self.allocvffnts();
            let v = self.vf_read_unsigned(cmd - FNT_DEF1 + 1);
            self.vf_e_fnts[self.vf_nf] = v;
            let v = self.vf_def_font(f);
            self.vf_i_fnts[self.vf_nf] = v;
            self.vf_nf += 1;
            cmd = self.vf_byte();
        }
        self.vf_default_font[f] = save_vf_nf;
        self.vf_local_font_num[f] = self.vf_nf - save_vf_nf;
        // §716 Allocate memory for the new virtual font
        self.vf_packet_base[f] = self.new_vf_packet(f);
        while cmd <= LONG_CHAR {
            // §717 Build a character packet
            let mut packet_length: i32;
            let cc: i32;
            let tfm_width: i32;
            if cmd == LONG_CHAR {
                packet_length = self.vf_read_unsigned(4);
                cc = self.vf_read_unsigned(4);
                if !self.is_valid_char(f, cc) {
                    self.vf_error(self.font_name[f], b"invalid character code");
                }
                let v = self.vf_read_signed(4);
                tfm_width = self.store_scaled_f(v, self.font_size[f]);
            } else {
                packet_length = cmd;
                cc = self.vf_byte();
                if !self.is_valid_char(f, cc) {
                    self.vf_error(self.font_name[f], b"invalid character code");
                }
                let v = self.vf_read_unsigned(3);
                tfm_width = self.store_scaled_f(v, self.font_size[f]);
            }
            if packet_length < 0 {
                self.vf_error(self.font_name[f], b"negative packet length");
            }
            if packet_length > VF_MAX_PACKET_LENGTH {
                self.vf_error(self.font_name[f], b"packet length too long");
            }
            if tfm_width != self.char_width(f, self.char_info(f, cc)) {
                self.print_nl_s(b"character width mismatch in font ");
                self.print(self.font_name[f]);
                self.print_s(b".vf ignored");
            }
            self.str_room(packet_length);
            while packet_length > 0 {
                cmd = self.vf_byte();
                packet_length -= 1;
                // §719 Cases of DVI commands that can appear in character packet
                if cmd >= SET_CHAR_0 && cmd <= SET_CHAR_0 + 127 {
                    cmd_length = 0;
                } else if (FNT_NUM_0 <= cmd && cmd <= FNT_NUM_0 + 63) || (FNT1 <= cmd && cmd <= FNT1 + 3) {
                    let k: i32;
                    if cmd >= FNT1 {
                        k = self.vf_read_unsigned(cmd - FNT1 + 1);
                        packet_length -= cmd - FNT1 + 1;
                    } else {
                        k = cmd - FNT_NUM_0;
                    }
                    if k >= 256 {
                        self.vf_error(self.font_name[f], b"too many local fonts");
                    }
                    let mut n = 0;
                    while n < self.vf_local_font_num[f] && self.vf_e_fnts[self.vf_default_font[f] + n] != k {
                        n += 1;
                    }
                    if n == self.vf_local_font_num[f] {
                        self.vf_error(self.font_name[f], b"undefined local font");
                    }
                    if k <= 63 {
                        self.append_char(FNT_NUM_0 + k);
                    } else {
                        self.append_char(FNT1);
                        self.append_char(k);
                    }
                    cmd_length = 0;
                    cmd = NOP;
                } else {
                    match cmd {
                        SET_RULE | PUT_RULE => cmd_length = 8,
                        SET1..=SET4 => cmd_length = cmd - SET1 + 1,
                        PUT1..=PUT4 => cmd_length = cmd - PUT1 + 1,
                        RIGHT1..=RIGHT4 => cmd_length = cmd - RIGHT1 + 1,
                        W1..=W4 => cmd_length = cmd - W1 + 1,
                        X1..=X4 => cmd_length = cmd - X1 + 1,
                        DOWN1..=DOWN4 => cmd_length = cmd - DOWN1 + 1,
                        Y1..=Y4 => cmd_length = cmd - Y1 + 1,
                        Z1..=Z4 => cmd_length = cmd - Z1 + 1,
                        XXX1..=XXX4 => {
                            cmd_length = self.vf_read_unsigned(cmd - XXX1 + 1);
                            packet_length -= cmd - XXX1 + 1;
                            if cmd_length > VF_MAX_PACKET_LENGTH {
                                self.vf_error(self.font_name[f], b"packet length too long");
                            }
                            if cmd_length < 0 {
                                self.vf_error(self.font_name[f], b"string of negative length");
                            }
                            self.append_char(XXX1);
                            self.append_char(cmd_length);
                            cmd = NOP; // cmd has been already stored above as xxx1
                        }
                        W0 | X0 | Y0 | Z0 | NOP => cmd_length = 0,
                        PUSH | POP => {
                            cmd_length = 0;
                            if cmd == PUSH {
                                if stack_level == VF_STACK_SIZE {
                                    self.overflow(b"virtual font stack size", VF_STACK_SIZE);
                                } else {
                                    stack_level += 1;
                                }
                            } else if stack_level == 0 {
                                self.vf_error(self.font_name[f], b"more POPs than PUSHs in character");
                            } else {
                                stack_level -= 1;
                            }
                        }
                        _ => self.vf_error(self.font_name[f], b"improver DVI command"),
                    }
                }
                if cmd != NOP {
                    self.append_char(cmd);
                }
                packet_length -= cmd_length;
                while cmd_length > 0 {
                    cmd_length -= 1;
                    let v = self.vf_byte();
                    self.append_char(v);
                }
            }
            if stack_level != 0 {
                self.vf_error(self.font_name[f], b"more PUSHs than POPs in character packet");
            }
            if packet_length != 0 {
                self.vf_error(self.font_name[f], b"invalid packet length or DVI command in packet");
            }
            // §718 Store the packet being built
            let s = self.make_string();
            self.storepacket(f, cc, s);
            self.flush_str(s);
            cmd = self.vf_byte();
        }
        if cmd != POST {
            self.vf_error(self.font_name[f], b"POST command expected");
        }
        self.b_close(VF_FILE);
        self.pdf_font_type[f] = VIRTUAL_FONT_TYPE;
    }

    /// §720
    pub fn pdf_check_vf_cur_val(&mut self) {
        let f = self.cur_val;
        self.do_vf(f);
        if self.pdf_font_type[f] == VIRTUAL_FONT_TYPE {
            self.pdf_error(b"font", b"command cannot be used with virtual font");
        }
    }

    /// §720 check for a virtual auto-expanded font.
    pub fn auto_expand_vf(&mut self, f: i32) -> bool {
        if !self.pdf_font_auto_expand[f] || self.pdf_font_blink[f] == NULL_FONT {
            return false; // not an auto-expanded font
        }
        let bf = self.pdf_font_blink[f];
        if self.pdf_font_type[bf] == NEW_FONT_TYPE {
            // we must process the base font first
            self.do_vf(bf);
        }
        if self.pdf_font_type[bf] != VIRTUAL_FONT_TYPE {
            return false; // not a virtual font
        }

        let e = self.pdf_font_expand_ratio[f];
        for k in 0..self.vf_local_font_num[bf] {
            let lf = self.vf_default_font[bf] + k;
            self.allocvffnts();
            // copy vf local font numbers:
            self.vf_e_fnts[self.vf_nf] = self.vf_e_fnts[lf];
            // definition of local vf fonts are expanded from base fonts:
            let v = self.auto_expand_font(self.vf_i_fnts[lf], e);
            self.vf_i_fnts[self.vf_nf] = v;
            self.copy_expand_params(self.vf_i_fnts[self.vf_nf], self.vf_i_fnts[lf], e);
            self.vf_nf += 1;
        }
        self.vf_packet_base[f] = self.vf_packet_base[bf];
        self.vf_local_font_num[f] = self.vf_local_font_num[bf];
        self.vf_default_font[f] = self.vf_nf - self.vf_local_font_num[f];

        self.pdf_font_type[f] = VIRTUAL_FONT_TYPE;
        true
    }

    // ---- §725-726: processing character packets ----

    /// §725 read `k` bytes as a signed integer from character packet.
    pub fn packet_read_signed(&mut self, k: i32) -> i32 {
        let mut k = k;
        self.pdfassert(k > 0 && k <= 4);
        let mut i = self.packet_byte();
        if i >= 128 {
            i -= 256;
        }
        k -= 1;
        while k > 0 {
            i = i * 256 + self.packet_byte();
            k -= 1;
        }
        i
    }

    /// §725 read `k` bytes as an unsigned integer from character packet.
    pub fn packet_read_unsigned(&mut self, k: i32) -> i32 {
        let mut k = k;
        self.pdfassert(k > 0 && k <= 4);
        let mut i = self.packet_byte();
        if k == 4 && i >= 128 {
            // bad_vf: f here is the global, as WEB has it
            self.vf_error(self.font_name[self.f], b"number too big");
        }
        k -= 1;
        while k > 0 {
            i = i * 256 + self.packet_byte();
            k -= 1;
        }
        i
    }

    /// §725 get `k` bytes from packet as scaled.
    pub fn packet_scaled(&mut self, k: i32, fs: i32) -> i32 {
        let v = self.packet_read_signed(k);
        self.store_scaled_f(v, fs)
    }

    /// §725 typeset the DVI commands in the character packet for character `c` in current font `f`.
    pub fn do_vf_packet(&mut self, vf_f: i32, c: i32) {
        const SET4: i32 = SET1 + 3;
        const PUT4: i32 = PUT1 + 3;
        const RIGHT4: i32 = RIGHT1 + 3;
        const W4: i32 = W1 + 3;
        const X4: i32 = X1 + 3;
        const DOWN4: i32 = DOWN1 + 3;
        const Y4: i32 = Y1 + 3;
        const Z4: i32 = Z1 + 3;
        const XXX4: i32 = XXX1 + 3;

        let mut c = c;
        self.vf_cur_s += 1;
        if self.vf_cur_s > VF_MAX_RECURSION {
            self.overflow(b"max level recursion of virtual fonts", VF_MAX_RECURSION);
        }
        let save_cur_v = self.cur_v;
        let save_cur_h = self.cur_h;
        self.push_packet_state(); // save pointer and length of the current packet
        self.start_packet(vf_f, c); // set pointer and length of the new packet
        let mut f = self.vf_i_fnts[self.vf_default_font[vf_f]];
        let (mut w, mut x, mut y, mut z) = (0, 0, 0, 0);
        while self.vf_packet_length > 0 {
            let cmd = self.packet_byte();
            let char_move: bool;
            // §726 Do typesetting the DVI commands in virtual character packet
            'continue_: {
                'do_char: {
                    if cmd >= SET_CHAR_0 && cmd <= SET_CHAR_0 + 127 {
                        if !self.is_valid_char(f, cmd) {
                            self.char_warning(f, cmd);
                            break 'continue_;
                        }
                        c = cmd;
                        char_move = true;
                        break 'do_char;
                    } else if (FNT_NUM_0 <= cmd && cmd <= FNT_NUM_0 + 63) || cmd == FNT1 {
                        let k = if cmd == FNT1 { self.packet_byte() } else { cmd - FNT_NUM_0 };
                        let mut n = 0;
                        while n < self.vf_local_font_num[vf_f] && self.vf_e_fnts[self.vf_default_font[vf_f] + n] != k {
                            n += 1;
                        }
                        if n == self.vf_local_font_num[vf_f] {
                            self.pdf_error(b"vf", b"local font not found");
                        } else {
                            f = self.vf_i_fnts[self.vf_default_font[vf_f] + n];
                        }
                    } else {
                        match cmd {
                            PUSH => {
                                let p = self.vf_stack_ptr;
                                self.vf_stack[p].stack_h = self.cur_h;
                                self.vf_stack[p].stack_v = self.cur_v;
                                self.vf_stack[p].stack_w = w;
                                self.vf_stack[p].stack_x = x;
                                self.vf_stack[p].stack_y = y;
                                self.vf_stack[p].stack_z = z;
                                self.vf_stack_ptr += 1;
                            }
                            POP => {
                                self.vf_stack_ptr -= 1;
                                let p = self.vf_stack_ptr;
                                self.cur_h = self.vf_stack[p].stack_h;
                                self.cur_v = self.vf_stack[p].stack_v;
                                w = self.vf_stack[p].stack_w;
                                x = self.vf_stack[p].stack_x;
                                y = self.vf_stack[p].stack_y;
                                z = self.vf_stack[p].stack_z;
                            }
                            SET1..=SET4 | PUT1..=PUT4 => {
                                if SET1 <= cmd && cmd <= SET1 + 3 {
                                    let v = self.packet_read_unsigned(cmd - SET1 + 1);
                                    self.set_tmp_int(v);
                                    char_move = true;
                                } else {
                                    let v = self.packet_read_unsigned(cmd - PUT1 + 1);
                                    self.set_tmp_int(v);
                                    char_move = false;
                                }
                                if !self.is_valid_char(f, self.tmp_int()) {
                                    self.char_warning(f, self.tmp_int());
                                    break 'continue_;
                                }
                                c = self.tmp_int();
                                break 'do_char;
                            }
                            SET_RULE | PUT_RULE => {
                                self.rule_ht = self.packet_scaled(4, self.font_size[vf_f]);
                                self.rule_wd = self.packet_scaled(4, self.font_size[vf_f]);
                                if self.rule_wd > 0 && self.rule_ht > 0 {
                                    self.pdf_set_rule(self.cur_h, self.cur_v, self.rule_wd, self.rule_ht);
                                    if cmd == SET_RULE {
                                        self.cur_h += self.rule_wd;
                                    }
                                }
                            }
                            RIGHT1..=RIGHT4 => {
                                let d = self.packet_scaled(cmd - RIGHT1 + 1, self.font_size[vf_f]);
                                self.cur_h += d;
                            }
                            W0 | W1..=W4 => {
                                if cmd > W0 {
                                    w = self.packet_scaled(cmd - W0, self.font_size[vf_f]);
                                }
                                self.cur_h += w;
                            }
                            X0 | X1..=X4 => {
                                if cmd > X0 {
                                    x = self.packet_scaled(cmd - X0, self.font_size[vf_f]);
                                }
                                self.cur_h += x;
                            }
                            DOWN1..=DOWN4 => {
                                let d = self.packet_scaled(cmd - DOWN1 + 1, self.font_size[vf_f]);
                                self.cur_v += d;
                            }
                            Y0 | Y1..=Y4 => {
                                if cmd > Y0 {
                                    y = self.packet_scaled(cmd - Y0, self.font_size[vf_f]);
                                }
                                self.cur_v += y;
                            }
                            Z0 | Z1..=Z4 => {
                                if cmd > Z0 {
                                    z = self.packet_scaled(cmd - Z0, self.font_size[vf_f]);
                                }
                                self.cur_v += z;
                            }
                            XXX1..=XXX4 => {
                                let v = self.packet_read_unsigned(cmd - XXX1 + 1);
                                self.set_tmp_int(v);
                                self.str_room(self.tmp_int());
                                while self.tmp_int() > 0 {
                                    self.set_tmp_int(self.tmp_int() - 1);
                                    let b = self.packet_byte();
                                    self.append_char(b);
                                }
                                let s = self.make_string();
                                self.literal(s, SCAN_SPECIAL, false);
                                self.flush_str(s);
                            }
                            _ => self.pdf_error(b"vf", b"invalid DVI command"),
                        }
                    }
                    break 'continue_;
                }
                // do_char:
                if self.is_valid_char(f, c) {
                    self.output_one_char(f, c);
                } else {
                    self.char_warning(f, c);
                }
                if char_move {
                    self.cur_h += self.char_width(f, self.char_info(f, c));
                }
            }
        }
        self.pop_packet_state(); // restore pointer and length of the previous packet
        self.cur_v = save_cur_v;
        self.cur_h = save_cur_h;
        self.vf_cur_s -= 1;
    }

    // ---- vfpacket.c ----

    /// External: vfpacket.c
    pub fn new_vf_packet(&mut self, f: i32) -> i32 {
        let n = self.font_ec[f] - self.font_bc[f] + 1;
        let mut pack = self.v_pdffont.vf_pack;
        let first = pack.len;
        for _ in 0..n {
            self.arena_push(&mut pack, Span { off: 0, len: 0 });
        }
        self.v_pdffont.vf_pack = pack;
        let mut t = self.v_pdffont.vf_tab;
        self.arena_push(&mut t, VfEnt { first, n });
        self.v_pdffont.vf_tab = t;
        t.len - 1
    }

    /// External: vfpacket.c
    pub fn storepacket(&mut self, f: i32, c: i32, s: i32) {
        let a = self.str_start[s];
        let l = self.str_start[s + 1] - a;
        self.pdffont_pool_room(l);
        let mut pool = self.v_pdffont.pool;
        let off = pool.len;
        for k in 0..l {
            let b = self.str_pool[a + k];
            pool.push_within_cap(&mut self.arena, b);
        }
        self.v_pdffont.pool = pool;
        let e = self.v_pdffont.vf_tab.get(&mut self.arena, self.vf_packet_base[f]);
        self.v_pdffont.vf_pack.set(&mut self.arena, e.first + c - self.font_bc[f], Span { off, len: l });
    }

    /// External: vfpacket.c
    pub fn start_packet(&mut self, f: i32, c: i32) {
        let e = self.v_pdffont.vf_tab.get(&mut self.arena, self.vf_packet_base[f]);
        let p = self.v_pdffont.vf_pack.get(&mut self.arena, e.first + c - self.font_bc[f]);
        self.v_pdffont.packet_data_ptr = p.off;
        self.vf_packet_length = p.len;
    }

    /// External: vfpacket.c
    #[inline]
    pub fn packet_byte(&mut self) -> i32 {
        self.vf_packet_length -= 1;
        let b = self.v_pdffont.pool.get(&mut self.arena, self.v_pdffont.packet_data_ptr);
        self.v_pdffont.packet_data_ptr += 1;
        b as i32
    }

    /// External: vfpacket.c
    pub fn push_packet_state(&mut self) {
        let p = self.v_pdffont.packet_stack_ptr;
        self.v_pdffont.packet_stack[p] = [self.v_pdffont.packet_data_ptr, self.vf_packet_length];
        self.v_pdffont.packet_stack_ptr = p + 1;
    }

    /// External: vfpacket.c
    pub fn pop_packet_state(&mut self) {
        if self.v_pdffont.packet_stack_ptr == 0 {
            self.pdffont_fail(b"packet stack empty, impossible to pop");
        }
        self.v_pdffont.packet_stack_ptr -= 1;
        let [ptr, len] = self.v_pdffont.packet_stack[self.v_pdffont.packet_stack_ptr];
        self.v_pdffont.packet_data_ptr = ptr;
        self.vf_packet_length = len;
    }

    /// External: utils.c.  The C grows these arrays, doubling; here they stay at their capacity.
    pub fn allocvffnts(&mut self) {
        if self.vf_nf >= FONT_CAP as i32 {
            self.overflow(b"virtual font local fonts", FONT_CAP as i32);
        }
    }

    // ---- pdftex.h ----

    /// External: pdftex.h
    #[inline]
    pub fn get_kn_bs_code(&mut self, f: i32, c: i32) -> i32 {
        if self.pdf_font_kn_bs_base[f] == 0 { 0 } else { self.pdf_mem[self.pdf_font_kn_bs_base[f] + c] }
    }

    /// External: pdftex.h
    #[inline]
    pub fn get_st_bs_code(&mut self, f: i32, c: i32) -> i32 {
        if self.pdf_font_st_bs_base[f] == 0 { 0 } else { self.pdf_mem[self.pdf_font_st_bs_base[f] + c] }
    }

    /// External: pdftex.h
    #[inline]
    pub fn get_sh_bs_code(&mut self, f: i32, c: i32) -> i32 {
        if self.pdf_font_sh_bs_base[f] == 0 { 0 } else { self.pdf_mem[self.pdf_font_sh_bs_base[f] + c] }
    }

    /// External: pdftex.h
    #[inline]
    pub fn get_kn_bc_code(&mut self, f: i32, c: i32) -> i32 {
        if self.pdf_font_kn_bc_base[f] == 0 { 0 } else { self.pdf_mem[self.pdf_font_kn_bc_base[f] + c] }
    }

    /// External: pdftex.h
    #[inline]
    pub fn get_kn_ac_code(&mut self, f: i32, c: i32) -> i32 {
        if self.pdf_font_kn_ac_base[f] == 0 { 0 } else { self.pdf_mem[self.pdf_font_kn_ac_base[f] + c] }
    }
}

/// `%i`.
fn int_bytes(v: i32) -> Vec<u8> {
    v.to_string().into_bytes()
}

/// `magstep` of kpathsea's magstep.c.
fn magstep(n: i32, bdpi: i32) -> i32 {
    let (mut n, neg) = if n < 0 { (-n, true) } else { (n, false) };
    let mut t: f64;
    if n & 1 != 0 {
        n &= !1;
        t = 1.095445115;
    } else {
        t = 1.0;
    }
    while n > 8 {
        n -= 8;
        t *= 2.0736;
    }
    while n > 0 {
        n -= 2;
        t *= 1.2;
    }
    (0.5 + if neg { bdpi as f64 / t } else { bdpi as f64 * t }) as i32
}

/// `kpse_magstep_fix(dpi, bdpi, NULL)`: the true dpi of `dpi` with a base resolution of `bdpi`.
fn magstep_fix(dpi: i32, bdpi: i32) -> i32 {
    // The C's dpi is unsigned, so a negative one compares as large.
    let sign = if (dpi as u32) < bdpi as u32 { -1 } else { 1 };
    let mut real_dpi = 0;
    let mut m = 0;
    while real_dpi == 0 && m < 40 {
        let mdpi = magstep(m * sign, bdpi);
        if (mdpi - dpi).abs() <= 1 {
            real_dpi = mdpi; // if this magstep matches, quit
        } else if (mdpi - dpi) * sign > 0 {
            real_dpi = dpi; // if gone too far, quit
        }
        m += 1;
    }
    if real_dpi != 0 { real_dpi } else { dpi }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::toks::testkit::*;

    fn with_font(t: &mut Tex, f: i32, name: &[u8]) {
        for &c in name {
            t.append_char(c as i32);
        }
        t.font_name[f] = t.make_string();
    }

    fn pool_str(t: &mut Tex, s: &[u8]) -> i32 {
        for &c in s {
            t.append_char(c as i32);
        }
        t.make_string()
    }

    /// What `tests/oracle/gen_pdffont_tables.py` got from the oracle's log, row by row in one run: the map
    /// trees' warnings, their wrapped lines, duplicates, replacing, deleting, and entries in use.
    #[test]
    fn map_lines_against_the_oracle() {
        rstex_kpse::with_global(|k| k.set_invocation_name(b"pdftex"));
        let t = machine();
        print_setup(t);
        let mut fonts = 0;
        for row in include_str!("../testdata/pdffont_map_oracle.tsv").lines() {
            let f: Vec<&str> = row.split('\t').collect();
            let want = f[2].replace('|', "\n");
            let _ = printed(t);
            match f[0] {
                "line" => t.pdffont_process_map_item(f[1].as_bytes(), MapType::MapLine),
                "suppress" => t.set_pdf_suppress_warning_dup_map(f[1].parse().unwrap()),
                "use" => {
                    fonts += 1;
                    with_font(t, fonts, f[1].as_bytes());
                    assert!(t.hasfmentry(fonts), "{}", f[1]);
                }
                _ => panic!("{}", f[0]),
            }
            assert_eq!(printed(t), want, "after {} {}", f[0], f[1]);
        }
    }

    #[test]
    fn a_map_entry_is_found_and_flagged() {
        rstex_kpse::with_global(|k| k.set_invocation_name(b"pdftex"));
        let t = machine();
        print_setup(t);
        t.pdffont_process_map_item(b"aa AA <aa.pfb", MapType::MapLine);
        t.pdffont_process_map_item(b"+pk", MapType::MapLine);
        t.pdffont_process_map_item(b"+bb BB 4 \".2 SlantFont .9 ExtendFont\" <8r.enc <bb.pfb", MapType::MapLine);
        t.pdffont_process_map_item(b"+cc CC <cc.ttf", MapType::MapLine);
        with_font(t, 1, b"aa");
        with_font(t, 2, b"pk");
        with_font(t, 3, b"none");
        with_font(t, 4, b"bb");
        assert!(t.hasfmentry(1) && t.isscalable(1));
        assert!(t.hasfmentry(2) && !t.isscalable(2), "a PK entry is not scalable");
        assert!(!t.hasfmentry(3) && !t.isscalable(3));
        assert_eq!(t.pdf_font_map[3], FM_DUMMY);
        assert!(t.isscalable(4));
        let i = t.pdffont_fm_index(1);
        assert_eq!(t.pdffont_fm(i).in_use, 1);
        let i = t.pdffont_fm_index(4);
        let fm = t.pdffont_fm(i);
        assert_eq!((fm.slant, fm.extend, fm.fd_flags), (200, 900, 4));
        assert_eq!(t.pdffont_bytes(fm.encname), b"8r.enc");
        assert_eq!(t.pdffont_bytes(fm.ff_name), b"bb.pfb");
        assert!(fm.is_included() && fm.is_subsetted() && fm.is_type1() && fm.is_reencoded() && !fm.is_builtin());
        // Only a Type 1 with a file enters the ps tree; a font name is not a ps name.
        assert!(ps_slot(&t.v_pdffont, &mut t.arena, b"AA", 0, 0) >= 0);
        assert!(ps_slot(&t.v_pdffont, &mut t.arena, b"BB", 0, 0) < 0, "other slant");
        assert!(ps_slot(&t.v_pdffont, &mut t.arena, b"BB", 200, 900) >= 0);
        assert!(ps_slot(&t.v_pdffont, &mut t.arena, b"CC", 0, 0) < 0, "TrueType");
    }

    #[test]
    fn many_entries_make_the_tables_grow() {
        rstex_kpse::with_global(|k| k.set_invocation_name(b"pdftex"));
        let t = machine();
        print_setup(t);
        t.pdffont_process_map_item(b"x", MapType::MapLine);
        for k in 0..3000 {
            let l = format!("+f{k} P{k} <f{k}.pfb");
            t.pdffont_process_map_item(l.as_bytes(), MapType::MapLine);
        }
        for k in (0..3000).step_by(3) {
            let l = format!("-f{k}");
            t.pdffont_process_map_item(l.as_bytes(), MapType::MapLine);
        }
        for k in (0..3000).step_by(3) {
            let l = format!("=f{k} Q{k} <g{k}.pfb");
            t.pdffont_process_map_item(l.as_bytes(), MapType::MapLine);
        }
        assert_eq!(printed(t), "");
        for k in 0..3000 {
            let n = format!("f{k}");
            let slot = tfm_slot(&t.v_pdffont, &mut t.arena, n.as_bytes());
            assert!(slot >= 0, "{}", n);
            let tab = t.v_pdffont.tfm_hash;
            let i = tab.get(&mut t.arena, slot) - 1;
            let fm = t.pdffont_fm(i);
            let want = if k % 3 == 0 { format!("g{k}.pfb") } else { format!("f{k}.pfb") };
            assert_eq!(t.pdffont_bytes(fm.ff_name), want.as_bytes());
            // -f{k} has no ps name, so it leaves P{k} in the tree; the C does the same.
            let p = format!("P{k}");
            assert!(ps_slot(&t.v_pdffont, &mut t.arena, p.as_bytes(), 0, 0) >= 0, "{}", p);
        }
    }

    /// TeX Live's own map file, line by line as `pdffont_fm_read_file` reads it.  The oracle gives no warning for it
    /// (see the integration case map-default-lazy), and these lines are what most documents use.
    #[test]
    fn the_real_map_file_goes_in() {
        let t = machine();
        print_setup(t);
        let Some(path) = t.host.world.lookup(b"pdftex.map", rstex_kpse::KPSE_FONTMAP_FORMAT, true) else {
            return;
        };
        let Some(data) = t.host.world.read(&path) else {
            return;
        };
        t.pdffont_pool_room(data.len() as i32);
        let mut pos = 0;
        let mut buf = [0u8; 1024];
        while let Some(line) = ff::fm_read_line(&data, &mut pos, &mut buf).unwrap() {
            t.pdffont_fm_scan_line(line, FmMode::DupIgnore);
        }
        assert_eq!(printed(t), "");
        assert!(t.v_pdffont.fm_tab.len > 1000);
        // Lines we know, as a font would ask for them.
        with_font(t, 1, b"cmr10");
        with_font(t, 2, b"ptmr8r");
        with_font(t, 3, b"no-such-font");
        assert!(t.isscalable(1) && t.isscalable(2) && !t.hasfmentry(3));
        let i = t.pdffont_fm_index(1);
        let fm = t.pdffont_fm(i);
        assert_eq!(t.pdffont_bytes(fm.ps_name), b"CMR10");
        assert_eq!(t.pdffont_bytes(fm.ff_name), b"cmr10.pfb");
        assert!(fm.is_type1() && fm.is_subsetted() && !fm.is_reencoded());
        let i = t.pdffont_fm_index(2);
        let fm = t.pdffont_fm(i);
        assert_eq!(t.pdffont_bytes(fm.encname), b"8r.enc");
        assert!(ps_slot(&t.v_pdffont, &mut t.arena, b"CMR10", 0, 0) >= 0);
        // Most of the size of a run's image should not go here.
        assert!((t.arena_top as usize) < (1 << 20), "{}", t.arena_top);
    }

    #[test]
    fn name_extensions_of_lookup_fontmap() {
        assert_eq!(parse_ps_name(b"CMR10"), (&b"CMR10"[..], 0, 0));
        assert_eq!(parse_ps_name(b"ABCDEF+CMR10"), (&b"CMR10"[..], 0, 0));
        assert_eq!(parse_ps_name(b"ABCDEFG+CMR10"), (&b"ABCDEFG+CMR10"[..], 0, 0));
        assert_eq!(parse_ps_name(b"abcdef+CMR10"), (&b"abcdef+CMR10"[..], 0, 0));
        assert_eq!(parse_ps_name(b"ABCDEF+"), (&b"ABCDEF+"[..], 0, 0), "needs more than 7 characters");
        assert_eq!(parse_ps_name(b"ABCDEF+x"), (&b"x"[..], 0, 0));
        assert_eq!(parse_ps_name(b"Foo-Slant_167"), (&b"Foo"[..], 167, 0));
        assert_eq!(parse_ps_name(b"Foo-Slant_-167"), (&b"Foo"[..], -167, 0));
        assert_eq!(parse_ps_name(b"Foo-Extend_900"), (&b"Foo"[..], 0, 900));
        assert_eq!(parse_ps_name(b"Foo-Slant_167-Extend_900"), (&b"Foo"[..], 167, 900));
        assert_eq!(parse_ps_name(b"AAAAAA+Foo-Slant_1-Extend_2"), (&b"Foo"[..], 1, 2));
        // Not valid: left whole.
        assert_eq!(parse_ps_name(b"Foo-Slant_"), (&b"Foo-Slant_"[..], 0, 0));
        assert_eq!(parse_ps_name(b"Foo-Slant_x"), (&b"Foo-Slant_x"[..], 0, 0));
        assert_eq!(parse_ps_name(b"Foo-Slant_1x"), (&b"Foo-Slant_1x"[..], 0, 0));
        assert_eq!(parse_ps_name(b"Foo-Slant_1-Extend_x"), (&b"Foo-Slant_1-Extend_x"[..], 0, 0));
        assert_eq!(parse_ps_name(b"Foo-Extend_1-Slant_2"), (&b"Foo-Extend_1"[..], 2, 0), "the slant is sought first");
        assert_eq!(parse_ps_name(b"Foo-Extend_1x"), (&b"Foo-Extend_1x"[..], 0, 0));
    }

    #[test]
    fn magsteps_as_in_kpathsea() {
        assert_eq!(magstep(0, 600), 600);
        assert_eq!(magstep(1, 600), 657);
        assert_eq!(magstep(2, 600), 720);
        assert_eq!(magstep(-2, 600), 500);
        assert_eq!(magstep(10, 600), 1493);
        assert_eq!(magstep_fix(600, 600), 600);
        assert_eq!(magstep_fix(721, 600), 720);
        assert_eq!(magstep_fix(700, 600), 700);
        assert_eq!(magstep_fix(499, 600), 500);
        assert_eq!(magstep_fix(72, 72), 72);
    }

    #[test]
    fn strtol() {
        assert_eq!(strtol10(b""), (0, 0));
        assert_eq!(strtol10(b"x"), (0, 0));
        assert_eq!(strtol10(b"-"), (0, 0));
        assert_eq!(strtol10(b"12x"), (12, 2));
        assert_eq!(strtol10(b" \t+12"), (12, 5));
        assert_eq!(strtol10(b"-12"), (-12, 3));
        assert_eq!(strtol10(b"99999999999999999999"), (i64::MAX, 20));
        assert_eq!(strtol10(b"-99999999999999999999"), (i64::MIN, 21));
    }

    #[test]
    fn packets_come_back_and_nest() {
        let t = machine();
        print_setup(t);
        t.font_bc[1] = 65;
        t.font_ec[1] = 70;
        t.font_bc[2] = 0;
        t.font_ec[2] = 1;
        assert_eq!(t.new_vf_packet(1), 0);
        assert_eq!(t.new_vf_packet(2), 1);
        t.vf_packet_base[1] = 0;
        t.vf_packet_base[2] = 1;
        let s = pool_str(t, b"abc");
        t.storepacket(1, 66, s);
        let s = pool_str(t, b"xy");
        t.storepacket(2, 1, s);
        t.start_packet(1, 65);
        assert_eq!(t.vf_packet_length, 0, "a character with no packet");
        t.start_packet(1, 66);
        assert_eq!(t.vf_packet_length, 3);
        assert_eq!(t.packet_byte(), b'a' as i32);
        t.push_packet_state();
        t.start_packet(2, 1);
        assert_eq!(t.vf_packet_length, 2);
        assert_eq!(t.packet_byte(), b'x' as i32);
        assert_eq!(t.vf_packet_length, 1);
        t.pop_packet_state();
        assert_eq!(t.vf_packet_length, 2);
        assert_eq!(t.packet_byte(), b'b' as i32);
        assert_eq!(t.packet_byte(), b'c' as i32);
        assert_eq!(t.vf_packet_length, 0);
    }

    #[test]
    fn codes_default_and_read_the_table() {
        let t = machine();
        assert_eq!(t.get_kn_bs_code(1, 65), 0);
        assert_eq!(t.get_st_bs_code(1, 65), 0);
        assert_eq!(t.get_sh_bs_code(1, 65), 0);
        assert_eq!(t.get_kn_bc_code(1, 65), 0);
        assert_eq!(t.get_kn_ac_code(1, 65), 0);
        t.pdf_font_kn_bs_base[1] = 100;
        t.pdf_font_st_bs_base[1] = 400;
        t.pdf_font_sh_bs_base[1] = 700;
        t.pdf_font_kn_bc_base[1] = 1000;
        t.pdf_font_kn_ac_base[1] = 1300;
        for (b, v) in [(100, 1), (400, 2), (700, 3), (1000, 4), (1300, 5)] {
            t.pdf_mem[b + 65] = v;
        }
        assert_eq!(
            [t.get_kn_bs_code(1, 65), t.get_st_bs_code(1, 65), t.get_sh_bs_code(1, 65), t.get_kn_bc_code(1, 65), t.get_kn_ac_code(1, 65)],
            [1, 2, 3, 4, 5]
        );
    }

    #[test]
    fn expand_names_and_letterspaced_names() {
        let t = machine();
        print_setup(t);
        with_font(t, 1, b"cmr10");
        let s = t.expand_font_name(1, 20);
        assert_eq!(t.str_pool.range(t.str_start[s], t.str_start[s + 1]), b"cmr10+20");
        let s = t.expand_font_name(1, -5);
        assert_eq!(t.str_pool.range(t.str_start[s], t.str_start[s + 1]), b"cmr10-5");
        let s = t.expand_font_name(1, 0);
        assert_eq!(t.str_pool.range(t.str_start[s], t.str_start[s + 1]), b"cmr100");

        for (name, ty, want) in [
            (&b"cmr10+100ls"[..], VIRTUAL_FONT_TYPE, true),
            (b"cmr10-5ls", VIRTUAL_FONT_TYPE, true),
            (b"cmr10+ls", VIRTUAL_FONT_TYPE, true),
            (b"+1ls", VIRTUAL_FONT_TYPE, true),
            (b"cmr10+100ls", REAL_FONT_TYPE, false),
            (b"cmr10100ls", VIRTUAL_FONT_TYPE, false),
            (b"100ls", VIRTUAL_FONT_TYPE, false),
            (b"cmr10", VIRTUAL_FONT_TYPE, false),
        ] {
            with_font(t, 2, name);
            t.pdf_font_type[2] = ty;
            assert_eq!(t.is_letterspaced_font(2), want, "{:?}", String::from_utf8_lossy(name));
        }
    }

    #[test]
    fn tfm_lookup_when_nothing_matches_keeps_the_string() {
        let t = machine();
        print_setup(t);
        with_font(t, 1, b"cmr10");
        t.font_area[1] = S_EMPTY;
        t.font_size[1] = 655360;
        with_font(t, 2, b"cmr12");
        t.font_area[2] = pool_str(t, NON_EXISTENT_PATH);
        t.font_size[2] = 655360;
        t.font_ptr = 2;
        let before = t.str_ptr;
        let s = pool_str(t, b"cmr11");
        assert_eq!(t.tfm_lookup(s, 655360), NULL_FONT);
        assert_eq!(t.tfm_lookup(s, 0), NULL_FONT);
        assert_eq!(t.str_ptr, before + 1);
        // A copy has that area, so it is not found by name.
        let s = pool_str(t, b"cmr12");
        assert_eq!(t.tfm_lookup(s, 655360), NULL_FONT);
        assert_eq!(t.tfm_lookup(s, 0), NULL_FONT);
        assert!(t.pdffont_has_area(1));
        assert!(!t.pdffont_has_area(2));
    }

    #[test]
    fn expansion_values_on_a_multiple_of_the_step() {
        let t = machine();
        t.pdf_font_step[1] = 5;
        t.pdf_font_stretch[1] = 2;
        t.pdf_font_shrink[1] = 3;
        t.pdf_font_expand_ratio[2] = 20;
        t.pdf_font_expand_ratio[3] = -15;
        assert_eq!(t.fix_expand_value(1, 0), 0);
        assert_eq!(t.fix_expand_value(1, 10), 10);
        assert_eq!(t.fix_expand_value(1, -10), -10);
        assert_eq!(t.fix_expand_value(1, 21), 20, "the limit");
        assert_eq!(t.fix_expand_value(1, -16), -15, "the limit");
        assert_eq!(t.fix_expand_value(1, 20), 20);
        t.pdf_font_elink[1] = 2;
        t.pdf_font_elink[2] = 3;
        t.pdf_font_elink[3] = 0;
        assert_eq!(t.get_expand_font(1, 20), 2);
        assert_eq!(t.get_expand_font(1, -15), 3);
        assert_eq!(t.expand_font(1, 0), 1);
        assert_eq!(t.expand_font(1, 21), 2);
    }

    #[test]
    fn an_encoding_survives_the_arena() {
        let t = machine();
        print_setup(t);
        let names = ff::load_enc_file(b"/E [ /.notdef /a /b /.notdef /longglyphname\n/space ] def\n").unwrap();
        let n = t.pdffont_pool_add(b"x.enc");
        let fe = t.pdffont_fe_add(n, &names);
        let back = t.pdffont_fe_glyph_names(fe);
        for c in 0..256 {
            assert_eq!(names.is_notdef(c), back.is_notdef(c), "{c}");
            assert_eq!(names.name(c), back.name(c), "{c}");
        }
        assert!(!t.v_pdffont.fe_tab.get(&mut t.arena, fe).space_at_32);
        let mut src = b"/E [".to_vec();
        for _ in 0..32 {
            src.extend_from_slice(b" /.notdef");
        }
        src.extend_from_slice(b" /space ] def\n");
        let names = ff::load_enc_file(&src).unwrap();
        let fe = t.pdffont_fe_add(n, &names);
        assert!(t.v_pdffont.fe_tab.get(&mut t.arena, fe).space_at_32);
    }

    /// A restored image has the log and an empty `Host`.
    #[test]
    fn the_glyph_map_replays_from_its_log() {
        rstex_kpse::with_global(|k| k.set_invocation_name(b"pdftex"));
        let t = machine();
        print_setup(t);
        let pairs: [(&[u8], &[u8]); 6] = [
            (b"a", b"0061"),
            (b"b", b"110000"),
            (b"c", b"0063 0064"),
            (b"a", b"0041"),
            (b"", b"0061"),
            (b"d", b"zork"),
        ];
        assert!(!t.host.hv.pdffont.gtu.exists());
        let g = pool_str(t, b"");
        let u = pool_str(t, b"0061");
        t.def_tounicode(g, u);
        assert!(!t.host.hv.pdffont.gtu.exists(), "only an invalid pair so far");
        assert_eq!(printed(t), "\n\npdfTeX warning: pdftex: ToUnicode: invalid parameter(s): `' => `0061'\n");
        for (a, b) in pairs {
            let g = pool_str(t, a);
            let u = pool_str(t, b);
            t.def_tounicode(g, u);
        }
        let _ = printed(t);
        let mut direct = GlyphUnicodeMap::new();
        let _ = direct.deftounicode(b"", b"0061");
        for (a, b) in pairs {
            let _ = direct.deftounicode(a, b);
        }
        let mut names = GlyphNames::new();
        for (c, n) in [&b"a"[..], b"b", b"c", b"d"].iter().enumerate() {
            names.set(c, n);
        }
        let (mut x, mut y) = (Vec::new(), Vec::new());
        ff::write_tounicode(&names, b"f", None, &mut direct, &mut x).unwrap();
        // A fresh Host: nothing has been applied.
        t.host.hv.pdffont.gtu = GlyphUnicodeMap::new();
        t.host.hv.pdffont.gtu_applied = 0;
        t.pdffont_gtu_sync();
        assert_eq!(t.host.hv.pdffont.gtu_applied, t.v_pdffont.gtu_log.len);
        assert!(t.host.hv.pdffont.gtu.exists());
        let mut gtu = core::mem::take(&mut t.host.hv.pdffont.gtu);
        ff::write_tounicode(&names, b"f", None, &mut gtu, &mut y).unwrap();
        assert_eq!(x, y);
        assert_eq!(printed(t), "", "the replay gives no warnings");
    }

    #[test]
    fn glyphs_from_an_included_pdf() {
        let t = machine();
        print_setup(t);
        t.pdffont_epdf_mark_glyphs(7, b" /a/b /c\t/d\n\r /e/");
        t.pdffont_epdf_mark_glyphs(8, b"");
        let items = t.v_pdffont.gl_items;
        let mut got = Vec::new();
        for i in 0..items.len {
            let it = items.get(&mut t.arena, i);
            assert_eq!(it.fd, 7);
            got.push(t.pdffont_bytes(it.name));
        }
        assert_eq!(got, vec![b"a".to_vec(), b"b".to_vec(), b"c".to_vec(), b"d".to_vec(), b"e".to_vec()]);
    }

    #[test]
    fn pdf_printing_as_the_c_has_it() {
        let t = machine();
        t.pdf_buf_size = PDF_OP_BUF_SIZE;
        t.pdffont_pdf_puts(b"abc");
        assert_eq!(t.pdf_op_buf.range(0, 3), b"abc");
        assert_eq!(t.pdf_last_byte, b'c' as i32);
        t.pdffont_pdf_printf(&[b"de", &int_bytes(-12), b"\n"]);
        assert_eq!(t.pdf_op_buf.range(3, 9), b"de-12\n");
        assert_eq!(t.pdf_last_byte, b'\n' as i32);
        // One vsnprintf: the end of a long one is lost.
        t.pdf_ptr = 0;
        t.pdffont_pdf_printf(&[&[b'x'; 2000], b"\n"]);
        assert_eq!(t.pdf_ptr, PRINTF_MAX as i32);
        // fb_flush leaves pdflastbyte alone.
        t.pdf_ptr = 0;
        t.pdf_last_byte = 0;
        t.pdffont_fb_flush(b"binary");
        assert_eq!(t.pdf_op_buf.range(0, 6), b"binary");
        assert_eq!((t.pdf_ptr, t.pdf_last_byte), (6, 0));
    }

    #[test]
    fn warnings_and_their_blank_lines() {
        rstex_kpse::with_global(|k| k.set_invocation_name(b"pdftex"));
        let t = machine();
        print_setup(t);
        t.pdffont_warn(b"one");
        assert_eq!(printed(t), "\n\npdfTeX warning: pdftex: one\n");
        t.host.hv.pdffont.cur_file_name = Some(b"f.map".to_vec());
        t.pdffont_warn(b"two");
        assert_eq!(printed(t), "\n\npdfTeX warning: pdftex (file f.map): two\n");
        t.pdffont_warn(&[b'y'; 2000]);
        let long = printed(t);
        assert_eq!(long.matches('y').count(), PRINTF_MAX);
    }
}
