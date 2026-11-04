//! §1477-1509 Part 50: dumping and undumping the tables.
//!
//! **A format load is not a checkpoint restore.**  `load_fmt_file` runs in the middle of §1517, with the
//! command line in `buffer` and the run's `cur_input`, `selector` and `history` live, so it overwrites
//! only what §1485-1507 dump, as WEB does.  docs/PORTING.md, notes on `image.rs`.
//!
//! The file, all little-endian as the image is:
//!
//! ```text
//! magic i32        §1485
//! engine size i32, engine bytes  §1485
//! checksum u64     what `@$` is to WEB: this build's tables and layout
//! FmtRecord        one field for each dump_int and small array of §1485-1507 and §1654
//! for each POD_SECTIONS entry with in_format: start u64, end u64, where in the file its elements start..end are, u64
//! extra size u64, extra bytes    what §1504's dumpimagemeta and dumptounicode put
//! the elements of each section, in the order of the table
//! 69069 i32        §1506's closing check word
//! ```
//!
//! A step of `store_fmt_file` still comes where WEB has it, with its messages, and it does what is seen by
//! the next one; only the bytes go out together at the end, since nothing the dump covers changes
//! after WEB would have written it.  The ranges come from the file when loading, as `hyph_size` and
//! the like may differ in this run.
//!
//! A load puts each section directly in its place in the image.  A long one lies in the file at the same offset as in the image, modulo
//! [`FMT_ALIGN`], so that it can be mapped there instead of being read: see `image_load_bytes`, which a checkpoint's restore uses too.
//!
//! **What of the `v_<module>` structs a format holds**, against §1485-1507.  Everything in them is what the C keeps in
//! statics, so none of them is in the file, and the arena's bytes are unreachable without a `PodVec` to them, apart
//! from these, which it does hold: `v_ext.images`, `v_ext.image_names`, `v_web2c.str_index` and `v_pdffont.gtu_log` with `gtu_sealed`.  Left out, each as the C has it:
//!
//! - `v_pdffont`'s map, font, encoding and VF tables: a run starts with empty trees and reads the map file again.
//! - `v_pdfobj`'s trees: `find_obj` must not find an object of the run that dumped.
//! - `v_pdfship`'s colour, matrix and position stacks, `page_mode`, job id and start time; `v_buildtoks`'s last match;
//!   `v_ext`'s `getllx` values and PNG page group; `v_main`'s banner flag.
//!
//! The load overwrites the arena from 0, so nothing may allocate in it before `load_fmt_file`: nothing in `initialize`,
//! `init_prim`, `init_terminal` or the first line's handling does, and `crates/engine/tests/image.rs` checks `initialize`.
//!
//! DROPPED §1483: dump_int and friends: bytes move through `pod_bytes` and `image_bytes`.
//! DROPPED §1484: undump and friends: the same; §1486's and later range checks are written out.
//! DROPPED §1493-1495: eqtb compression: the section is flat, regions 1-6 and the part above eqtb_size.
#![forbid(unsafe_code)]
#![deny(unreachable_patterns, unused_variables)]
#![allow(non_snake_case, unused_mut, unused_imports, dead_code)]

use crate::access::*;
use crate::consts::*;
use crate::host::*;
use crate::raw::*;
use crate::state::*;

crate::pod_struct! {
    /// POD state private to this module.  Add fields here, not to state.rs.
    pub struct Vars {
        /// The preamble checkpoint's, which has no place of its own in `Tex`.
        pub ck: crate::checkpoint::Vars,
        /// The native diagram path's.
        pub cd: crate::native::Vars,
        /// The native `\\pickup@font`'s.
        pub pf: crate::native_font::Vars,
        /// `RSTEX_NO_SHARE`: crate::capture hands no argument on.  What it decides is in a checkpoint's key, as the driver's `Options` is.
        pub no_share: bool,
    }
}

/// Non-POD state private to this module, in `self.host`.  Rarely needed.
#[derive(Default)]
pub struct HVars {
    /// §1504's and §1505's extra part: `dumpimagemeta` and `dumptounicode` append here while storing,
    /// and the loader leaves there what `undump_image_meta` and `undumptounicode` take from the front.
    pub extra: Vec<u8>,
    /// How far the loader has taken from `extra`.
    pub extra_pos: usize,
    /// The loader wanted more than `extra` has.
    pub extra_short: bool,
}

/// §1485 Web2C TeX's magic constant, "W2TX".
const FMT_MAGIC: i32 = 0x57325458;
/// §1485 `engine_name`, with one or more trailing NULs up to a multiple of four.
const FMT_ENGINE: &[u8; 8] = b"pdftex\0\0";
/// §1506 the closing check word.
const FMT_CLOSING: i32 = 69069;

/// What `build.rs` makes of this build's sources: sixteen hexadecimal digits.
pub const FMT_BUILD_ID: &str = env!("RSTEX_BUILD_ID");

/// §1485 `@$`, which tangle makes a checksum of the string pool: a format holds string numbers and
/// command codes, and what they mean, as well as the layout of the image, can change with any source file.
pub(crate) const FMT_CHECKSUM: u64 = {
    let id = FMT_BUILD_ID.as_bytes();
    let mut h = 0u64;
    let mut i = 0;
    while i < id.len() {
        let d = id[i];
        h = h << 4 | (if d >= b'a' { d - b'a' + 10 } else { d - b'0' }) as u64;
        i += 1;
    }
    h
};

/// The first bytes of any format this build writes.
fn fmt_header() -> [u8; 24] {
    let mut h = [0; 24];
    h[..4].copy_from_slice(&FMT_MAGIC.to_le_bytes());
    h[4..8].copy_from_slice(&(FMT_ENGINE.len() as i32).to_le_bytes());
    h[8..16].copy_from_slice(FMT_ENGINE);
    h[16..24].copy_from_slice(&FMT_CHECKSUM.to_le_bytes());
    h
}

/// True if the file at `path` is a format of this build, as far as its first bytes say: TeX Live's are gzip files.
pub fn fmt_file_is_this_builds(path: &std::path::Path) -> bool {
    crate::host::file_starts_with(path, &fmt_header())
}

crate::pod_struct! {
    /// One field for each `dump_int` and small array of §1485-1507 and §1654, and what the port has
    /// instead of WEB's structures: `toks_top`, `tok_free`, `hash_tab_size`, `hash_tab_used`, `arena_top`.
    /// `prim` comes last and the count of `i32`s is even, so the record has no padding: a format is
    /// the same bytes for the same run.
    pub struct FmtRecord {
        pub xord: A<u8, 256>,
        pub xchr: A<u8, 256>,
        pub xprn: A<u8, 256>,
        pub max_halfword: i32,
        pub hash_high: i32,
        pub eTeX_mode: i32,
        pub mem_bot: i32,
        pub mem_top: i32,
        pub eqtb_size: i32,
        pub hash_prime: i32,
        pub hyph_prime: i32,
        pub pool_ptr: i32,
        pub str_ptr: i32,
        pub lo_mem_max: i32,
        pub rover: i32,
        pub sa_root: A<i32, 7>,
        pub hi_mem_min: i32,
        pub avail: i32,
        pub var_used: i32,
        pub dyn_used: i32,
        pub par_loc: i32,
        pub write_loc: i32,
        pub hash_used: i32,
        pub cs_count: i32,
        pub fmem_ptr: i32,
        pub font_ptr: i32,
        pub hyph_count: i32,
        pub hyph_next: i32,
        pub hyph_start: i32,
        pub trie_max: i32,
        pub trie_op_ptr: i32,
        pub trie_used: A<i32, 256>,
        pub pdf_mem_size: i32,
        pub pdf_mem_ptr: i32,
        pub obj_tab_size: i32,
        pub obj_ptr: i32,
        pub sys_obj_ptr: i32,
        pub pdf_obj_count: i32,
        pub pdf_xform_count: i32,
        pub pdf_ximage_count: i32,
        pub head_tab_obj: i32,
        pub head_tab_xform: i32,
        pub head_tab_ximage: i32,
        pub pdf_last_obj: i32,
        pub pdf_last_xform: i32,
        pub pdf_last_ximage: i32,
        pub interaction: i32,
        pub format_ident: i32,
        pub toks_top: i32,
        pub tok_free: A<i32, 256>,
        pub hash_tab_size: i32,
        pub hash_tab_used: i32,
        pub arena_top: i32,
        pub str_index_start: i32,
        pub str_index_cap: i32,
        pub prim: A<Word, 2101>,
    }
}

const _: () = assert!(core::mem::offset_of!(FmtRecord, prim) % 8 == 0, "padding before prim");
const _: () = assert!(
    core::mem::offset_of!(FmtRecord, prim) + core::mem::size_of::<A<Word, 2101>>() == core::mem::size_of::<FmtRecord>(),
    "padding after prim"
);

impl FmtRecord {
    /// A record to read into.
    const ZERO: FmtRecord = FmtRecord {
        xord: A([0; 256]),
        xchr: A([0; 256]),
        xprn: A([0; 256]),
        max_halfword: 0,
        hash_high: 0,
        eTeX_mode: 0,
        mem_bot: 0,
        mem_top: 0,
        eqtb_size: 0,
        hash_prime: 0,
        hyph_prime: 0,
        pool_ptr: 0,
        str_ptr: 0,
        lo_mem_max: 0,
        rover: 0,
        sa_root: A([0; 7]),
        hi_mem_min: 0,
        avail: 0,
        var_used: 0,
        dyn_used: 0,
        par_loc: 0,
        write_loc: 0,
        hash_used: 0,
        cs_count: 0,
        fmem_ptr: 0,
        font_ptr: 0,
        hyph_count: 0,
        hyph_next: 0,
        hyph_start: 0,
        trie_max: 0,
        trie_op_ptr: 0,
        trie_used: A([0; 256]),
        pdf_mem_size: 0,
        pdf_mem_ptr: 0,
        obj_tab_size: 0,
        obj_ptr: 0,
        sys_obj_ptr: 0,
        pdf_obj_count: 0,
        pdf_xform_count: 0,
        pdf_ximage_count: 0,
        head_tab_obj: 0,
        head_tab_xform: 0,
        head_tab_ximage: 0,
        pdf_last_obj: 0,
        pdf_last_xform: 0,
        pdf_last_ximage: 0,
        interaction: 0,
        format_ident: 0,
        toks_top: 0,
        tok_free: A([0; 256]),
        hash_tab_size: 0,
        hash_tab_used: 0,
        arena_top: 0,
        str_index_start: 0,
        str_index_cap: 0,
        prim: A([Word::ZERO; 2101]),
    };
}

/// A section at least this long lies in the file at the same offset as in the image, modulo [`FMT_ALIGN`].
const FMT_ALIGN_MIN: usize = 1 << 16;
/// Enough for the page size of every system with `mmap`: 4 KiB, or 16 KiB on Apple's arm64.
const FMT_ALIGN: u64 = 1 << 14;
/// A section read whole, of at least this size, is worth the zeroing of huge pages under it.
const FMT_HUGE_MIN: usize = 1 << 20;
/// What one read takes from the front of the file: the header, the record, the table and, as a rule, the extra part.
const FMT_HEAD: usize = 32 << 10;

/// The front of the file being read, in order; `None` past the end.  In C a short read is `FATAL3 ("Could not undump")`.
struct FmtReader {
    file: FmtIn,
    head: Vec<u8>,
    pos: usize,
}

impl FmtReader {
    fn new(file: FmtIn) -> Option<FmtReader> {
        let mut head = vec![0; FMT_HEAD.min(file.size as usize)];
        if !file.read_at(0, &mut head) {
            return None;
        }
        Some(FmtReader { file, head, pos: 0 })
    }
    fn take(&mut self, n: usize) -> Option<&[u8]> {
        let e = self.pos.checked_add(n)?;
        if e > self.head.len() {
            // An extra part too long for the first read.
            if e as u64 > self.file.size {
                return None;
            }
            let have = self.head.len();
            self.head.resize(e, 0);
            if !self.file.read_at(have as u64, &mut self.head[have..]) {
                return None;
            }
        }
        let s = &self.head[self.pos..e];
        self.pos = e;
        Some(s)
    }
    fn i32(&mut self) -> Option<i32> {
        Some(i32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }
    fn u64(&mut self) -> Option<u64> {
        Some(u64::from_le_bytes(self.take(8)?.try_into().ok()?))
    }
}

impl Tex {
    /// §1480
    pub fn store_fmt_file(&mut self) {
        // §1482 If dumping is not allowed, abort
        if self.save_ptr != 0 {
            self.print_err_s(b"You can't dump inside a group");
            self.help(&[b"`{...\\dump}' is a no-no."]);
            self.succumb();
        }
        // §1508 Create the format_ident, open the format file, and inform the user that dumping has begun
        self.selector = NEW_STRING;
        self.print_s(b" (preloaded format=");
        self.print(self.job_name);
        self.print_char(b' ' as i32);
        self.print_int(self.year());
        self.print_char(b'.' as i32);
        self.print_int(self.month());
        self.print_char(b'.' as i32);
        self.print_int(self.day());
        self.print_char(b')' as i32);
        if self.interaction == BATCH_MODE {
            self.selector = LOG_ONLY;
        } else {
            self.selector = TERM_AND_LOG;
        }
        self.str_room(1);
        self.format_ident = self.make_string();
        self.pack_job_name(S_DOT_FMT);
        while !self.w_open_out(FMT_FILE) {
            self.prompt_file_name(b"format file name", S_DOT_FMT);
        }
        self.print_nl_s(b"Beginning to dump on file ");
        let s = self.w_make_name_string(FMT_FILE);
        self.slow_print(s);
        self.flush_string();
        self.print_nl_s(b"");
        self.slow_print(self.format_ident);
        // §1485 Dump constants for consistency check.  §1874 Dump xord, xchr, and xprn.  In the record.
        // §1654 Dump the eTeX state
        for j in 0..=ETEX_STATES - 1 {
            self.set_eTeX_state(j, 0); // disable all enhancements
        }
        // §1758 Dump the eTeX state
        while self.pseudo_files != NULL {
            self.pseudo_close(); // flush pseudo files
        }
        // §1487 Dump the string pool
        self.print_ln();
        self.print_int(self.str_ptr);
        self.print_s(b" strings of total length ");
        self.print_int(self.pool_ptr);
        // §1489 Dump the dynamic memory
        self.sort_avail();
        self.var_used = 0;
        let mut p = MEM_BOT;
        let mut q = self.rover;
        let mut x = 0;
        loop {
            x = x + q + 2 - p;
            self.var_used = self.var_used + q - p;
            p = q + self.node_size(q);
            q = self.rlink(q);
            if q == self.rover {
                break;
            }
        }
        self.var_used = self.var_used + self.lo_mem_max - p;
        self.dyn_used = self.mem_end + 1 - self.hi_mem_min;
        x = x + self.lo_mem_max + 1 - p;
        x = x + self.mem_end + 1 - self.hi_mem_min;
        p = self.avail;
        while p != NULL {
            self.dyn_used -= 1;
            p = self.link(p);
        }
        self.print_ln();
        self.print_int(x);
        self.print_s(b" memory locations dumped; current usage is ");
        self.print_int(self.var_used);
        self.print_char(b'&' as i32);
        self.print_int(self.dyn_used);
        // §1491 Dump the table of equivalents.  §1496 Dump the hash table.
        self.cs_count = FROZEN_CONTROL_SEQUENCE - 1 - self.hash_used + self.hash_high;
        for p in HASH_BASE..=self.hash_used {
            if self.text(p) != 0 {
                self.cs_count += 1;
            }
        }
        self.print_ln();
        self.print_int(self.cs_count);
        self.print_s(b" multiletter control sequences");
        // Room for what a document adds: growing moves every entry, which would be a page fault for each page of the table in every run.
        while 4 * self.hash_tab_used > self.hash_tab_size {
            self.hash_tab_grow();
        }
        // §1498 Dump the font information.  §1500 Dump the array info for internal font number k.
        for k in NULL_FONT..=self.font_ptr {
            self.print_nl_s(b"\\font");
            self.print_esc(self.font_id_text(k));
            self.print_char(b'=' as i32);
            self.print_file_name(self.font_name[k], self.font_area[k], S_EMPTY);
            if self.font_size[k] != self.font_dsize[k] {
                self.print_s(b" at ");
                self.print_scaled(self.font_size[k]);
                self.print_s(b"pt");
            }
        }
        self.print_ln();
        self.print_int(self.fmem_ptr - 7);
        self.print_s(b" words of font info for ");
        self.print_int(self.font_ptr - FONT_BASE);
        if self.font_ptr != FONT_BASE + 1 {
            self.print_s(b" preloaded fonts");
        } else {
            self.print_s(b" preloaded font");
        }
        // §1502 Dump the hyphenation tables
        if self.hyph_next <= HYPH_PRIME {
            self.hyph_next = self.hyph_size; // minimum value of hyphen_size needed
        }
        self.print_ln();
        self.print_int(self.hyph_count);
        if self.hyph_count != 1 {
            self.print_s(b" hyphenation exceptions");
        } else {
            self.print_s(b" hyphenation exception");
        }
        if self.trie_not_ready {
            self.init_trie();
        }
        self.print_nl_s(b"Hyphenation trie of length ");
        self.print_int(self.trie_max);
        self.print_s(b" has ");
        self.print_int(self.trie_op_ptr);
        if self.trie_op_ptr != 1 {
            self.print_s(b" ops");
        } else {
            self.print_s(b" op");
        }
        self.print_s(b" out of ");
        self.print_int(TRIE_OP_SIZE);
        for k in (0..=255).rev() {
            if self.trie_used[k] > MIN_QUARTERWORD {
                self.print_nl_s(b"  ");
                self.print_int(self.trie_used[k]);
                self.print_s(b" for language ");
                self.print_int(k);
            }
        }
        self.str_index_make();
        self.pdffont_gtu_seal();
        // §1504 Dump pdftex data
        self.host.hv.image.extra.clear();
        self.dumpimagemeta(); // the image information array
        self.print_ln();
        self.print_int(self.pdf_mem_ptr - 1);
        self.print_s(b" words of pdfTeX memory");
        self.print_ln();
        self.print_int(self.sys_obj_ptr);
        self.print_s(b" indirect objects");
        self.dumptounicode();
        // §1506 Dump a couple more things and the closing check word: the bytes go out here, before
        // tracing_stats changes: the format holds the document's value.
        self.image_write_fmt();
        self.set_tracing_stats(0);
        // §1509 Close the format file
        self.w_close(FMT_FILE);
    }

    /// The record of what `store_fmt_file` has come to.
    fn image_record(&self) -> FmtRecord {
        FmtRecord {
            xord: self.xord,
            xchr: self.xchr,
            xprn: self.xprn,
            max_halfword: MAX_HALFWORD,
            hash_high: self.hash_high,
            eTeX_mode: self.eTeX_mode,
            mem_bot: MEM_BOT,
            mem_top: MEM_TOP,
            eqtb_size: EQTB_SIZE,
            hash_prime: HASH_PRIME,
            hyph_prime: HYPH_PRIME,
            pool_ptr: self.pool_ptr,
            str_ptr: self.str_ptr,
            lo_mem_max: self.lo_mem_max,
            rover: self.rover,
            sa_root: self.sa_root,
            hi_mem_min: self.hi_mem_min,
            avail: self.avail,
            var_used: self.var_used,
            dyn_used: self.dyn_used,
            par_loc: self.par_loc,
            write_loc: self.write_loc,
            hash_used: self.hash_used,
            cs_count: self.cs_count,
            fmem_ptr: self.fmem_ptr,
            font_ptr: self.font_ptr,
            hyph_count: self.hyph_count,
            hyph_next: self.hyph_next,
            hyph_start: self.hyph_start,
            trie_max: self.trie_max,
            trie_op_ptr: self.trie_op_ptr,
            trie_used: self.trie_used,
            pdf_mem_size: self.pdf_mem_size,
            pdf_mem_ptr: self.pdf_mem_ptr,
            obj_tab_size: self.obj_tab_size,
            obj_ptr: self.obj_ptr,
            sys_obj_ptr: self.sys_obj_ptr,
            pdf_obj_count: self.pdf_obj_count,
            pdf_xform_count: self.pdf_xform_count,
            pdf_ximage_count: self.pdf_ximage_count,
            head_tab_obj: self.head_tab[OBJ_TYPE_OBJ],
            head_tab_xform: self.head_tab[OBJ_TYPE_XFORM],
            head_tab_ximage: self.head_tab[OBJ_TYPE_XIMAGE],
            pdf_last_obj: self.pdf_last_obj,
            pdf_last_xform: self.pdf_last_xform,
            pdf_last_ximage: self.pdf_last_ximage,
            interaction: self.interaction,
            format_ident: self.format_ident,
            toks_top: self.toks_top,
            tok_free: self.tok_free,
            hash_tab_size: self.hash_tab_size,
            hash_tab_used: self.hash_tab_used,
            arena_top: self.arena_top,
            str_index_start: self.v_web2c.str_index.start,
            str_index_cap: self.v_web2c.str_index.cap,
            prim: self.prim,
        }
    }

    /// The whole file, to `fmt_file`: `store_fmt_file`'s last step, and public for crates/engine/tests/image.rs.
    pub fn image_write_fmt(&mut self) {
        let rec = self.image_record();
        let mut slot = core::mem::take(&mut self.host.files[FMT_FILE as usize]);
        if let Slot::Out(o) = &mut slot {
            o.bytes(&fmt_header());
            o.bytes(pod_bytes(&rec));
            // Where each section goes: the table and the extra part come first, and they say so.
            let count = POD_SECTIONS.iter().filter(|s| s.in_format).count();
            let mut at = o.total + (count * 24 + 8 + self.host.hv.image.extra.len()) as u64;
            for s in POD_SECTIONS.iter().filter(|s| s.in_format) {
                let (a, b) = (s.live)(self);
                at += image_padding(at, s.offset + a * s.elem_size, (b - a) * s.elem_size);
                o.bytes(&(a as u64).to_le_bytes());
                o.bytes(&(b as u64).to_le_bytes());
                o.bytes(&at.to_le_bytes());
                at += ((b - a) * s.elem_size) as u64;
            }
            o.bytes(&(self.host.hv.image.extra.len() as u64).to_le_bytes());
            o.bytes(&self.host.hv.image.extra);
            for s in POD_SECTIONS.iter().filter(|s| s.in_format) {
                let (a, b) = (s.live)(self);
                for _ in 0..image_padding(o.total, s.offset + a * s.elem_size, (b - a) * s.elem_size) {
                    o.byte(0);
                }
                // A long section goes straight to the system, and on Windows that fails for pages nothing has touched: a live range may well hold such.
                let bytes = image_bytes(self, s.offset + a * s.elem_size, (b - a) * s.elem_size);
                crate::platform::commit_range(bytes.as_ptr().cast_mut(), bytes.len());
                o.bytes(bytes);
            }
            o.bytes(&FMT_CLOSING.to_le_bytes());
        }
        self.host.files[FMT_FILE as usize] = slot;
        // `do_dump`, as soon as an `fwrite` comes up short.  A format that did not get written is no format.
        let name = self.name_of_file_bytes();
        if let Slot::Out(o) = &mut self.host.files[FMT_FILE as usize] {
            if o.has_failed() {
                let msg = format!("! Could not write {} 1-byte item(s) to {}.\n", o.total, String::from_utf8_lossy(&name));
                self.host.world.stderr_write(msg.as_bytes());
                self.uexit(1);
            }
        }
    }

    /// §1481 `too_small`.  The caller goes to bad_fmt.
    fn image_too_small(&mut self, what: &[u8]) {
        self.wake_up_terminal();
        self.wterm_s(b"---! Must increase the ");
        self.wterm_ln_s(what);
    }

    /// §1481
    pub fn load_fmt_file(&mut self) -> bool {
        let slot = core::mem::take(&mut self.host.files[FMT_FILE as usize]);
        if let Slot::Fmt(f) = slot {
            if let Some(mut r) = FmtReader::new(f) {
                if self.image_load(&mut r).is_some() {
                    self.set_prev_depth(self.pdf_ignored_dimen()); // §1481
                    return true; // it worked!
                }
            }
        }
        // bad_fmt:
        self.wake_up_terminal();
        self.wterm_ln_s(b"(Fatal format file error; I'm stymied)");
        false
    }

    /// The one way bytes get into a new image, from a format or a checkpoint: `n` bytes, from `offset` in the file to `at` in the image.  If `mapped`, which [`fmt_mapped`]
    /// says of the section they are in, the whole pages among them are mapped, and the bytes on either side are read: their pages hold another array too.  A file not
    /// laid out for this ([`image_padding`]) is read.  `Some` means they are all there, and past the end of the file a mapping would give SIGBUS when first touched, so that is `None`.
    pub(crate) fn image_load_bytes(&mut self, f: &FmtIn, mapped: bool, at: usize, n: usize, offset: u64) -> Option<()> {
        if offset.checked_add(n as u64)? > f.size {
            return None;
        }
        let page = crate::platform::page_size();
        let first = at.next_multiple_of(page);
        let last = (at + n) & !(page - 1);
        if mapped && first < last && (offset + (first - at) as u64) % page as u64 == 0 {
            let head = first - at;
            if f.map_at(offset + head as u64, image_bytes_mut(self, first, last - first)) {
                let tail = at + n - last;
                let ok = f.read_at(offset, image_bytes_mut(self, at, head)) && f.read_at(offset + (head + (last - first)) as u64, image_bytes_mut(self, last, tail));
                return if ok { Some(()) } else { None };
            }
        }
        // Filled whole, a long section gets huge pages from where it starts: see `Tex::alloc`.
        if n >= FMT_HUGE_MIN {
            let chunk = at & !(crate::platform::TWO_MIB - 1);
            let dst = image_bytes_mut(self, chunk, at + n - chunk);
            crate::platform::advise_huge(dst.as_mut_ptr(), dst.len().next_multiple_of(crate::platform::TWO_MIB));
        }
        let dst = image_bytes_mut(self, at, n);
        if !crate::platform::commit_range(dst.as_mut_ptr(), n) {
            return None;
        }
        if f.read_at(offset, dst) { Some(()) } else { None }
    }

    /// Everything §1481 does up to `bad_fmt`: `None` goes there.
    fn image_load(&mut self, r: &mut FmtReader) -> Option<()> {
        // §1486 Undump constants for consistency check
        let x = r.i32()?;
        if x != FMT_MAGIC {
            return None; // not a format file
        }
        let x = r.i32()?;
        if !(0..=256).contains(&x) {
            return None; // corrupted format file
        }
        let engine = r.take(x as usize)?.to_vec();
        let end = engine.iter().position(|&c| c == 0).unwrap_or(engine.len());
        if engine[..end] != FMT_ENGINE[..6] {
            let name = self.name_of_file_bytes();
            self.wake_up_terminal();
            self.wterm_s(b"---! ");
            self.wterm_s(&name);
            self.wterm_s(b" was written by ");
            self.wterm_ln_s(&engine[..end]);
            return None;
        }
        if r.u64()? != FMT_CHECKSUM {
            // check that strings are the same
            let name = self.name_of_file_bytes();
            self.wake_up_terminal();
            self.wterm_s(b"---! ");
            self.wterm_s(&name);
            self.wterm_ln_s(b" made by different executable version, strings are different");
            return None;
        }
        let mut rec = FmtRecord::ZERO;
        pod_bytes_mut(&mut rec).copy_from_slice(r.take(core::mem::size_of::<FmtRecord>())?);
        // §1875 Undump xord, xchr, and xprn
        if self.host.translate_filename.is_none() {
            self.xord = rec.xord;
            self.xchr = rec.xchr;
            self.xprn = rec.xprn;
            if self.eight_bit_p != 0 {
                for k in 0..=255 {
                    self.xprn.put(k, 1);
                }
            }
        }
        if rec.max_halfword != MAX_HALFWORD {
            return None; // check max_halfword
        }
        if rec.hash_high < 0 || rec.hash_high > SUP_HASH_EXTRA || EQTB_SIZE as i64 + rec.hash_high as i64 >= EQTB_CAP as i64 {
            return None;
        }
        self.hash_high = rec.hash_high;
        // dropped: hash_extra:=hash_high.  It would change what §1514 prints, as hash_high here counts
        // every control sequence, not those past the oracle's hash_size.  eqtb_top is the capacity.
        // eqtb[undefined_control_sequence] comes with the rest of eqtb; entries above eqtb_size are
        // initialised as they are made.
        // §1655 Undump the eTeX state
        if rec.eTeX_mode < 0 || rec.eTeX_mode > 1 {
            return None;
        }
        self.eTeX_mode = rec.eTeX_mode;
        if self.eTeX_ex() {
            // §1813 Initialize variables for eTeX extended mode
            self.max_reg_num = 32767;
            self.max_reg_help_line = 1; // "A register number must be between 0 and 32767."
        } else {
            // §1812 Initialize variables for eTeX compatibility mode
            self.max_reg_num = 255;
            self.max_reg_help_line = 0; // "A register number must be between 0 and 255."
        }
        if rec.mem_bot != MEM_BOT {
            return None;
        }
        if MEM_BOT + 1100 > rec.mem_top || rec.mem_top != MEM_TOP {
            return None;
        }
        self.set_head(CONTRIB_HEAD);
        self.set_tail(CONTRIB_HEAD);
        self.page_tail = PAGE_HEAD; // page initialization
        if rec.eqtb_size != EQTB_SIZE || rec.hash_prime != HASH_PRIME || rec.hyph_prime != HYPH_PRIME {
            return None;
        }
        // §1488 Undump the string pool
        if rec.pool_ptr < 0 {
            return None;
        }
        if rec.pool_ptr > SUP_POOL_SIZE - self.pool_free {
            self.image_too_small(b"string pool size");
            return None;
        }
        self.pool_ptr = rec.pool_ptr;
        if self.pool_size < self.pool_ptr + self.pool_free {
            self.pool_size = self.pool_ptr + self.pool_free;
        }
        if rec.str_ptr < 0 {
            return None;
        }
        if rec.str_ptr > SUP_MAX_STRINGS - self.strings_free {
            self.image_too_small(b"sup strings");
            return None;
        }
        self.str_ptr = rec.str_ptr;
        if self.max_strings < self.str_ptr + self.strings_free {
            self.max_strings = self.str_ptr + self.strings_free;
        }
        self.init_str_ptr = self.str_ptr;
        self.init_pool_ptr = self.pool_ptr;
        // §1490 Undump the dynamic memory
        if rec.lo_mem_max < LO_MEM_STAT_MAX + 1000 || rec.lo_mem_max > HI_MEM_STAT_MIN - 1 {
            return None;
        }
        self.lo_mem_max = rec.lo_mem_max;
        if rec.rover < LO_MEM_STAT_MAX + 1 || rec.rover > self.lo_mem_max {
            return None;
        }
        self.rover = rec.rover;
        if self.eTeX_ex() {
            for k in INT_VAL..=TOK_VAL {
                if rec.sa_root[k] < NULL || rec.sa_root[k] > self.lo_mem_max {
                    return None;
                }
                self.sa_root[k] = rec.sa_root[k];
            }
        }
        // dropped: `if mem_min<mem_bot-2 then make more low memory available`: mem_min is a constant.
        if rec.hi_mem_min < self.lo_mem_max + 1 || rec.hi_mem_min > HI_MEM_STAT_MIN {
            return None;
        }
        self.hi_mem_min = rec.hi_mem_min;
        if rec.avail < NULL || rec.avail > MEM_TOP {
            return None;
        }
        self.avail = rec.avail;
        self.mem_end = MEM_TOP;
        self.var_used = rec.var_used;
        self.dyn_used = rec.dyn_used;
        // §1492 Undump the table of equivalents
        self.par_loc = rec.par_loc;
        self.par_token = CS_TOKEN_FLAG + self.par_loc;
        if rec.write_loc < HASH_BASE || rec.write_loc > self.hash_top {
            return None;
        }
        self.write_loc = rec.write_loc;
        // §1497 Undump the hash table
        self.prim = rec.prim;
        if rec.hash_used < HASH_BASE || rec.hash_used > FROZEN_CONTROL_SEQUENCE {
            return None;
        }
        self.hash_used = rec.hash_used;
        self.cs_count = rec.cs_count;
        // §1499 Undump the font information
        if rec.fmem_ptr < 7 {
            return None;
        }
        if rec.fmem_ptr > SUP_FONT_MEM_SIZE {
            self.image_too_small(b"font mem size");
            return None;
        }
        self.fmem_ptr = rec.fmem_ptr;
        if self.fmem_ptr > self.font_mem_size {
            self.font_mem_size = self.fmem_ptr;
        }
        if rec.font_ptr < FONT_BASE {
            return None;
        }
        if rec.font_ptr > FONT_BASE + MAX_FONT_MAX {
            self.image_too_small(b"font max");
            return None;
        }
        self.font_ptr = rec.font_ptr;
        // §1503 Undump the hyphenation tables: the limits come first, the tables with the sections
        if rec.hyph_count < 0 {
            return None;
        }
        if rec.hyph_count > self.hyph_size {
            self.image_too_small(b"hyph_size");
            return None;
        }
        self.hyph_count = rec.hyph_count;
        if rec.hyph_next < HYPH_PRIME {
            return None;
        }
        if rec.hyph_next > self.hyph_size {
            self.image_too_small(b"hyph_size");
            return None;
        }
        if rec.trie_max < 0 {
            return None;
        }
        if rec.trie_max > self.trie_size {
            self.image_too_small(b"trie size");
            return None;
        }
        self.trie_max = rec.trie_max;
        if rec.hyph_start < 0 || rec.hyph_start > self.trie_max {
            return None;
        }
        self.hyph_start = rec.hyph_start;
        if rec.trie_op_ptr < 0 {
            return None;
        }
        if rec.trie_op_ptr > TRIE_OP_SIZE {
            self.image_too_small(b"trie op size");
            return None;
        }
        self.trie_op_ptr = rec.trie_op_ptr;
        // What the port has instead of WEB's structures.
        if rec.toks_top < 0 || rec.toks_top as usize > TOKS_CAP || rec.hash_tab_size < 0 || rec.hash_tab_size as usize > HASH_TAB_CAP || rec.arena_top < 0 || rec.arena_top as usize > ARENA_CAP {
            return None;
        }
        self.toks_top = rec.toks_top;
        self.tok_free = rec.tok_free;
        self.hash_tab_size = rec.hash_tab_size;
        self.hash_tab_used = rec.hash_tab_used;
        self.arena_top = rec.arena_top;
        if rec.str_index_cap < 0 || (rec.str_index_cap != 0 && rec.str_index_cap.count_ones() != 1) || rec.str_index_start < 0 || rec.str_index_start as i64 + (rec.str_index_cap as i64) / 2 > rec.arena_top as i64 {
            return None;
        }
        self.v_web2c.str_index = PodVec::from_parts(rec.str_index_start, rec.str_index_cap);
        self.v_web2c.str_index.len = rec.str_index_cap;
        // The arrays of §1487-§1505, each from the range the file gives.  §1488's and §1501's
        // undump_checked_things look at each element; the checksum has said this is this build's file.
        for s in POD_SECTIONS.iter().filter(|s| s.in_format) {
            let (a, b) = (r.u64()? as usize, r.u64()? as usize);
            let offset = r.u64()?;
            if a > b || b > s.cap {
                return None;
            }
            self.image_load_bytes(&r.file, fmt_mapped(s.name), s.offset + a * s.elem_size, (b - a) * s.elem_size, offset)?;
        }
        // §1490: the checks on the free list that WEB makes as it reads the nodes.  It reports a messed
        // up file, in which `p` could become garbage.
        let mut q = self.rover;
        loop {
            if self.node_size(q) > self.lo_mem_max - q || self.rlink(q) > self.lo_mem_max || (q >= self.rlink(q) && self.rlink(q) != self.rover) {
                return None;
            }
            q = self.rlink(q);
            if q == self.rover {
                break;
            }
        }
        // §1501 Undump the array info for internal font number k: what is not dumped.
        self.main_init_pdf_font_arrays();
        self.make_pdftex_banner();
        // §1503: the largest occupied location in hyph_word sets hyph_next.  WEB's loop has read
        // hyph_count entries, and it stops at a hyph_link or a location that is out of range.
        let mut j = 0;
        let mut k = 0;
        let mut seen = 0;
        let hyph_end = HYPH_CAP as i32;
        while k < hyph_end && seen < self.hyph_count {
            if self.hyph_word[k] != 0 {
                j = k;
                seen += 1;
                if j >= self.hyph_size || self.hyph_link[j] > self.hyph_size {
                    return None;
                }
                if self.hyph_word[j] < 0 || self.hyph_word[j] > self.str_ptr {
                    return None;
                }
                if self.hyph_list[j] < MIN_HALFWORD || self.hyph_list[j] > MAX_HALFWORD {
                    return None;
                }
            }
            k += 1;
        }
        // j is now the largest occupied location in hyph_word
        j += 1;
        if j < HYPH_PRIME {
            j = HYPH_PRIME;
        }
        self.hyph_next = j;
        if self.hyph_next >= self.hyph_size {
            self.hyph_next = HYPH_PRIME;
        } else if self.hyph_next >= HYPH_PRIME {
            self.hyph_next += 1;
        }
        for k in 0..=255 {
            self.trie_used[k] = MIN_QUARTERWORD;
        }
        let mut k = 256;
        let mut j = self.trie_op_ptr;
        while j > 0 {
            // WEB reads the next language with patterns, going down from k-1
            k -= 1;
            while k >= 0 && rec.trie_used[k] <= MIN_QUARTERWORD {
                k -= 1;
            }
            if k < 0 {
                return None;
            }
            let x = rec.trie_used[k];
            if x > j {
                return None;
            }
            self.trie_used[k] = x;
            j -= x;
            self.op_start[k] = j;
        }
        self.trie_not_ready = false;
        // §1505 Undump pdftex data
        self.host.hv.image.extra = r_extra(r)?;
        self.host.hv.image.extra_pos = 0;
        self.host.hv.image.extra_short = false;
        self.undump_image_meta(self.pdf_major_version(), self.pdf_minor_version(), self.pdf_inclusion_errorlevel());
        self.pdf_mem_size = rec.pdf_mem_size;
        self.pdf_mem_ptr = rec.pdf_mem_ptr;
        self.obj_tab_size = rec.obj_tab_size;
        self.obj_ptr = rec.obj_ptr;
        // Not in WEB, where a damaged file overruns the array.  Here the loop below would.
        if rec.sys_obj_ptr < 0 || rec.sys_obj_ptr as usize >= OBJ_TAB_CAP {
            return None;
        }
        self.sys_obj_ptr = rec.sys_obj_ptr;
        for k in 1..=self.sys_obj_ptr {
            self.obj_tab[k].int2 = -1;
        }
        self.pdf_obj_count = rec.pdf_obj_count;
        self.pdf_xform_count = rec.pdf_xform_count;
        self.pdf_ximage_count = rec.pdf_ximage_count;
        self.head_tab[OBJ_TYPE_OBJ] = rec.head_tab_obj;
        self.head_tab[OBJ_TYPE_XFORM] = rec.head_tab_xform;
        self.head_tab[OBJ_TYPE_XIMAGE] = rec.head_tab_ximage;
        self.pdf_last_obj = rec.pdf_last_obj;
        self.pdf_last_xform = rec.pdf_last_xform;
        self.pdf_last_ximage = rec.pdf_last_ximage;
        self.undumptounicode();
        if self.host.hv.image.extra_short || self.host.hv.image.extra_pos != self.host.hv.image.extra.len() {
            return None;
        }
        // §1507 Undump a couple more things and the closing check word
        if rec.interaction < BATCH_MODE || rec.interaction > ERROR_STOP_MODE {
            return None;
        }
        self.interaction = rec.interaction;
        if self.interaction_option != UNSPECIFIED_MODE {
            self.interaction = self.interaction_option;
        }
        if rec.format_ident < 0 || rec.format_ident > self.str_ptr {
            return None;
        }
        self.format_ident = rec.format_ident;
        let mut closing = [0; 4];
        if !r.file.read_at(r.file.size.checked_sub(4)?, &mut closing) || i32::from_le_bytes(closing) != FMT_CLOSING {
            return None;
        }
        Some(())
    }

    // ---- External functions: the C side of web2c and pdfTeX, called by WEB. ----

    /// External: writeimg.c.  The table is `v_ext.images` and `v_ext.image_names`, in the arena, which is in the
    /// format; so their `PodVec`s go in the extra part, and nothing else of `v_ext`, whose other fields are statics that
    /// the C does not dump.
    pub fn dumpimagemeta(&mut self) {
        let (images, names) = (self.v_ext.images, self.v_ext.image_names);
        self.image_extra_put_pod(&images);
        self.image_extra_put_pod(&names);
    }

    /// External: tounicode.c.  `glyph_unicode_tree` is `v_pdffont.gtu_log`, in the arena, from which `pdffont.rs` makes
    /// the table again.  The rest of `v_pdffont` stays out: as in the C the map tables start empty on every run.
    pub fn dumptounicode(&mut self) {
        let (log, sealed, upto) = (self.v_pdffont.gtu_log, self.v_pdffont.gtu_sealed, self.v_pdffont.gtu_sealed_upto);
        self.image_extra_put_pod(&log);
        self.image_extra_put_pod(&sealed);
        self.image_extra_put_pod(&upto);
    }

    /// External: tounicode.c
    pub fn undumptounicode(&mut self) {
        let (mut log, mut sealed, mut upto) = (self.v_pdffont.gtu_log, self.v_pdffont.gtu_sealed, self.v_pdffont.gtu_sealed_upto);
        if self.image_extra_take_pod(&mut log) && self.image_extra_take_pod(&mut sealed) && self.image_extra_take_pod(&mut upto) {
            self.v_pdffont.gtu_log = log;
            self.v_pdffont.gtu_sealed = sealed;
            self.v_pdffont.gtu_sealed_upto = upto;
        } else {
            self.host.hv.image.extra_short = true;
        }
    }

    /// External: writeimg.c.  `ext_undump_images` does for each image what the C does here, in its order.  It
    /// comes before §1505 restores `obj_ptr` and the like, as in the C.
    pub fn undump_image_meta(&mut self, pdfmajorversion: i32, pdfminorversion: i32, pdfinclusionerrorlevel: i32) {
        let (mut images, mut names) = (self.v_ext.images, self.v_ext.image_names);
        if self.image_extra_take_pod(&mut images) && self.image_extra_take_pod(&mut names) {
            self.v_ext.images = images;
            self.v_ext.image_names = names;
            self.ext_undump_images(pdfmajorversion, pdfminorversion, pdfinclusionerrorlevel);
        } else {
            self.host.hv.image.extra_short = true;
        }
    }

    /// Appends to the extra part of a format being stored: for `dumpimagemeta` and `dumptounicode`.
    pub fn image_extra_put(&mut self, bytes: &[u8]) {
        self.host.hv.image.extra.extend_from_slice(bytes);
    }

    /// `image_extra_put` of a POD value, a `PodVec` above all.
    pub fn image_extra_put_pod<T: Pod>(&mut self, x: &T) {
        self.host.hv.image.extra.extend_from_slice(pod_bytes(x));
    }

    /// `image_extra_take` of a POD value; false, leaving it alone, past the end.
    pub fn image_extra_take_pod<T: Pod>(&mut self, x: &mut T) -> bool {
        match self.image_extra_take(core::mem::size_of::<T>()) {
            Some(s) => {
                pod_bytes_mut(x).copy_from_slice(s);
                true
            }
            None => false,
        }
    }

    /// Takes the next `n` bytes of the extra part of a format being loaded: for `undump_image_meta`
    /// and `undumptounicode`.  `None` past the end.
    pub fn image_extra_take(&mut self, n: usize) -> Option<&[u8]> {
        let h = &mut self.host.hv.image;
        let s = h.extra.get(h.extra_pos..h.extra_pos + n)?;
        h.extra_pos += n;
        Some(s)
    }
}

/// The extra part of the file.
fn r_extra(r: &mut FmtReader) -> Option<Vec<u8>> {
    let n = r.u64()? as usize;
    Some(r.take(n)?.to_vec())
}

impl Tex {
    /// The chunks a short document must not pay for, as `raw_alloc` says, are where a long one takes most of its dTLB misses: the
    /// scalars and `eqtb`, the two ends of `mem` and the font arrays.  This moves those to huge pages: 70% fewer on SGA3.
    pub fn image_collapse_hot(&mut self) {
        let mem = core::mem::offset_of!(Tex, mem);
        for offset in [0, mem, mem + core::mem::size_of::<A<Word, MEM_CAP>>() - 1, core::mem::offset_of!(Tex, char_base), core::mem::offset_of!(Tex, pdf_char_used)] {
            let p = image_bytes_mut(self, offset, 1).as_mut_ptr();
            crate::platform::collapse_huge(p, 1);
        }
    }
}

/// The zero bytes between what a file holds, up to `at`, and `n` bytes that go at `image_at` in the image.  Both file formats lay out what is long enough this way, so that it can be mapped.
pub(crate) fn image_padding(at: u64, image_at: usize, n: usize) -> u64 {
    if n < FMT_ALIGN_MIN {
        return 0;
    }
    (image_at as u64).wrapping_sub(at) & (FMT_ALIGN - 1)
}

/// Whether a load maps this section over the image instead of reading it, if it is long enough to have a page of its own.  What a run does not touch then costs it
/// nothing: `font_info` is 5 MB in the LaTeX format, most of it expl3's `\\fontdimen` tables, and a document that breaks no lines touches no trie.
///
/// `toks`, `eqtb` and `hash_tab` are read, since a run writes to most of their pages anyway: each use of a macro changes a reference count in `toks`, and a new name
/// lands anywhere in `hash_tab`.  Measured in alternating runs at load 40 on a one-page document and on the thesis.  Reading them with huge pages under what is
/// over 1 MiB: 32.5 and 2779 ms of CPU, 688 and 1791 page faults.  With small pages: 33.7 and 2894 ms, 1443 and 2552 faults.  Mapping them and copying each page at
/// once with `MADV_POPULATE_WRITE`: 32.7 and 2869 ms, 1654 and 3056 faults.  Mapping them and taking a fault for each page as it is written is no better for the
/// one and clearly worse for the other.
pub(crate) fn fmt_mapped(name: &str) -> bool {
    matches!(name, "hash" | "str_pool" | "str_start" | "font_info" | "trie_trl" | "trie_tro" | "trie_trc" | "arena")
}
