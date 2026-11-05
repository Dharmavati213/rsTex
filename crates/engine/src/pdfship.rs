//! §727-815 Part 32f: PDF shipping out.
//!
//! The procedures of Part 32f, and the C side of them: `utils.c`'s colour stacks, its `\pdfsave` and `\pdfsetmatrix`
//! stacks, the job id, `printID`, and texmfmp.c's `start_time_str`.  The tail of the part is `@<Finish the PDF file@>`,
//! which lands in `close_files_and_terminate`, in `main.rs`.
//!
//! # State other modules reach
//!
//! The C keeps these as statics of one file.  Here other modules' functions read them, and they live in this module:
//!
//! - `newcolorstack` (`buildtoks.rs`) is [`Tex::pdfship_new_colorstack`].
//! - `matrixused`, `matrixtransformrect`, `matrixrecalculate` (`ext.rs`) read
//!   `self.v_pdfship.matrix_stack.len`, and [`Tex::pdfship_matrix_top`], which also tests `page_mode`.
//! - `getcreationdate` (`buildtoks.rs`) and `print_ID` and the dates here read [`Tex::pdfship_start_time_str`], whose time
//!   is the one `World::init_start_time` keeps; `init_start_time` (`ext.rs`) sets that.
//! - `flush_jbig2_page0_objects`, here, is `ext.rs`'s `ext_flush_jbig2_page0`, whose are the JBIG2 files and the slots.
#![forbid(unsafe_code)]
#![deny(unreachable_patterns, unused_variables)]
#![allow(non_snake_case, unused_mut, unused_imports, dead_code)]

use crate::access::*;
use crate::consts::*;
use crate::host::*;
use crate::platform;
use crate::raw::*;
use crate::state::*;

crate::pod_struct! {
    /// `colstack_type` of utils.c.  The strings are indices in `col_strs`; 0 is both `NULL` and `""`, which the C treats alike.
    pub struct ColStack {
        pub page_stack: PodVec<i32>,
        pub form_stack: PodVec<i32>,
        pub page_current: i32,
        pub form_current: i32,
        pub form_init: i32,
        pub literal_mode: i32,
        pub page_start: bool,
    }
}

crate::pod_struct! {
    /// A colour string: `len` bytes of `col_chars` at `start`, and the next string with the same hash.
    pub struct ColStr {
        pub start: i32,
        pub len: i32,
        pub next: i32,
    }
}

crate::pod_struct! {
    /// `matrix_entry` of utils.c.
    pub struct MatrixEntry {
        pub a: f64,
        pub b: f64,
        pub c: f64,
        pub d: f64,
        pub e: f64,
        pub f: f64,
    }
}

crate::pod_struct! {
    /// `pos_entry` of utils.c.
    pub struct PosEntry {
        pub pos_h: i32,
        pub pos_v: i32,
        pub matrix_stack: i32,
    }
}

crate::pod_struct! {
    /// POD state private to this module.  Add fields here, not to state.rs.
    ///
    /// None of it goes in a format file, as in the C, whose statics are not dumped.  The strings are interned and never
    /// freed: the C's `xstrdup` and `xfree` on every push and pop only ever hold a few distinct colours.
    pub struct Vars {
        /// `page_mode` of utils.c.
        pub page_mode: bool,
        /// `colstacks`; its length is `colstacks_used`, and 0 means `colstacks_first_init` has not run.
        pub colstacks: PodVec<ColStack>,
        pub col_strs: PodVec<ColStr>,
        pub col_chars: PodVec<u8>,
        pub col_hash: A<i32, 64>,
        /// The string `COLOR_DEFAULT`.
        pub col_default: i32,
        /// `matrix_stack`; its length is `matrix_stack_used`.
        pub matrix_stack: PodVec<MatrixEntry>,
        /// `pos_stack`; its length is `pos_stack_used`.
        pub pos_stack: PodVec<PosEntry>,
        /// `job_id_string`; empty until `setjobid`.
        pub job_id_string: PodVec<u8>,
        /// `start_time_str` of texmfmp.c, and its length, which is 0 until `initstarttime`.
        pub start_time_str: A<u8, 30>,
        pub start_time_len: i32,
    }
}

/// Non-POD state private to this module, in `self.host`.  Rarely needed.
#[derive(Default)]
pub struct HVars {}

/// `MAX_COLORSTACKS` of utils.c.
const MAX_COLORSTACKS: i32 = 32768;
/// `COLOR_DEFAULT` of utils.c.
const COLOR_DEFAULT: &[u8] = b"0 g 0 G";
/// `kpse_src_compile` of kpathsea.
pub(crate) const KPSE_SRC_COMPILE: i32 = 1;
/// The file variable `f` of §772.  There is no pool file here, so that slot is free.
const OBJ_FILE: FileId = POOL_FILE;

/// A C string held in bytes: `strlen` stops at the first NUL.
fn pdfship_c_str(s: &[u8]) -> &[u8] {
    match s.iter().position(|&c| c == 0) {
        Some(n) => &s[..n],
        None => s,
    }
}

/// C's `isspace` in the C locale.
fn pdfship_is_space(c: u8) -> bool {
    c == b' ' || (9..=13).contains(&c)
}

/// `x` times 2 to the `e`, in steps that do not overflow on the way.
fn pdfship_ldexp(x: f64, e: i64) -> f64 {
    let mut x = x;
    let mut e = e.clamp(-100000, 100000);
    while e > 1000 {
        x *= 2f64.powi(1000);
        e -= 1000;
    }
    while e < -1000 {
        x *= 2f64.powi(-1000);
        e += 1000;
    }
    x * 2f64.powi(e as i32)
}

/// `strtod` on what `%lf` collected: the value, or `None` if no prefix of `b` is a number.
fn pdfship_strtod(b: &[u8]) -> Option<f64> {
    let (neg, b) = match b.first() {
        Some(b'-') => (true, &b[1..]),
        Some(b'+') => (false, &b[1..]),
        _ => (false, b),
    };
    let signed = |x: f64| if neg { -x } else { x };
    if b.len() >= 3 && b[0].eq_ignore_ascii_case(&b'n') {
        return Some(signed(f64::NAN));
    }
    if b.len() >= 3 && b[0].eq_ignore_ascii_case(&b'i') {
        return Some(signed(f64::INFINITY));
    }
    if b.len() >= 2 && b[0] == b'0' && b[1].eq_ignore_ascii_case(&b'x') {
        let mut i = 2;
        let (mut m, mut exp, mut sticky, mut dot, mut any) = (0u64, 0i64, false, false, false);
        while i < b.len() {
            if b[i] == b'.' && !dot {
                dot = true;
            } else if let Some(d) = (b[i] as char).to_digit(16) {
                any = true;
                if m >> 60 == 0 {
                    m = m * 16 + d as u64;
                    if dot {
                        exp -= 4;
                    }
                } else {
                    sticky |= d != 0;
                    if !dot {
                        exp += 4;
                    }
                }
            } else {
                break;
            }
            i += 1;
        }
        if !any {
            return Some(signed(0.0)); // "0x" with no digit is the number 0, and then an x
        }
        if i < b.len() && b[i].eq_ignore_ascii_case(&b'p') {
            let mut j = i + 1;
            let eneg = j < b.len() && b[j] == b'-';
            if j < b.len() && (b[j] == b'-' || b[j] == b'+') {
                j += 1;
            }
            if j < b.len() && b[j].is_ascii_digit() {
                let mut e = 0i64;
                while j < b.len() && b[j].is_ascii_digit() {
                    e = (e * 10 + (b[j] - b'0') as i64).min(1000000);
                    j += 1;
                }
                exp += if eneg { -e } else { e };
            }
        }
        if sticky {
            m |= 1;
        }
        return Some(signed(pdfship_ldexp(m as f64, exp)));
    }
    let mut i = 0;
    let mut digits = 0;
    while i < b.len() && b[i].is_ascii_digit() {
        i += 1;
        digits += 1;
    }
    if i < b.len() && b[i] == b'.' {
        i += 1;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
            digits += 1;
        }
    }
    if digits == 0 {
        return None;
    }
    if i < b.len() && b[i].eq_ignore_ascii_case(&b'e') {
        let mut j = i + 1;
        if j < b.len() && (b[j] == b'-' || b[j] == b'+') {
            j += 1;
        }
        if j < b.len() && b[j].is_ascii_digit() {
            while j < b.len() && b[j].is_ascii_digit() {
                j += 1;
            }
            i = j;
        }
    }
    let text = core::str::from_utf8(&b[..i]).ok()?;
    text.parse::<f64>().ok().map(signed)
}

/// One `%lf` of glibc's `vfscanf`, from `s[i..]`: the value and where the input now stands.  It collects characters by
/// its own rules, and `strtod` converts them.  The table in `testdata/pdfship_oracle.tsv` shows what the oracle's accepts.
fn pdfship_scan_lf(s: &[u8], i: usize) -> Option<(f64, usize)> {
    let n = s.len();
    let mut i = i;
    while i < n && pdfship_is_space(s[i]) {
        i += 1;
    }
    if i >= n {
        return None;
    }
    let start = i;
    let mut got_sign = 0;
    if s[i] == b'-' || s[i] == b'+' {
        got_sign = 1;
        i += 1;
        if i >= n {
            return None;
        }
    }
    let lower = |k: usize| if k < n { s[k].to_ascii_lowercase() } else { 0 };
    if lower(i) == b'n' {
        if lower(i + 1) != b'a' || lower(i + 2) != b'n' {
            return None;
        }
        i += 3;
        if i < n && s[i] == b'(' {
            // an n-char-sequence, which must close
            i += 1;
            loop {
                if i >= n {
                    return None;
                }
                let c = s[i];
                i += 1;
                if c == b')' {
                    break;
                }
                if !c.is_ascii_alphanumeric() && c != b'_' {
                    return None;
                }
            }
        }
        return pdfship_strtod(&s[start..i]).map(|x| (x, i));
    }
    if lower(i) == b'i' {
        if lower(i + 1) != b'n' || lower(i + 2) != b'f' {
            return None;
        }
        i += 3;
        if lower(i) == b'i' {
            if lower(i + 1) != b'n' || lower(i + 2) != b'i' || lower(i + 3) != b't' || lower(i + 4) != b'y' {
                return None;
            }
            i += 5;
        }
        return pdfship_strtod(&s[start..i]).map(|x| (x, i));
    }
    let mut exp_char = b'e';
    let mut hexa = false;
    let (mut got_digit, mut got_dot, mut got_e) = (false, false, false);
    if s[i] == b'0' {
        i += 1;
        if lower(i) == b'x' {
            hexa = true;
            exp_char = b'p';
            i += 1;
        } else {
            got_digit = true;
        }
    }
    while i < n {
        let c = s[i];
        if c.is_ascii_digit() {
            got_digit = true;
        } else if !got_e && hexa && c.is_ascii_hexdigit() {
            got_digit = true;
        } else if got_e && s[i - 1].to_ascii_lowercase() == exp_char && (c == b'-' || c == b'+') {
        } else if got_digit && !got_e && c.to_ascii_lowercase() == exp_char {
            got_e = true;
            got_dot = true;
        } else if !got_dot && c == b'.' {
            got_dot = true;
        } else {
            break;
        }
        i += 1;
    }
    // Nothing read, only the 0x of a hexadecimal, or an exponent with no digit: the last two are newer than the C
    // library's sources one usually reads, and the oracle's does this.
    let size = i - start;
    if size == got_sign || (hexa && !got_digit) || (got_e && !s[i - 1].is_ascii_digit()) {
        return None;
    }
    pdfship_strtod(&s[start..i]).map(|x| (x, i))
}

/// `sscanf(s, " %lf %lf %lf %lf %c", ...) == 4`: four numbers, and nothing but white space after them.
fn pdfship_scan_matrix(s: &[u8]) -> Option<[f64; 4]> {
    let mut x = [0.0; 4];
    let mut i = 0;
    for v in x.iter_mut() {
        let (d, j) = pdfship_scan_lf(s, i)?;
        *v = d;
        i = j;
    }
    while i < s.len() && pdfship_is_space(s[i]) {
        i += 1;
    }
    if i < s.len() { None } else { Some(x) }
}

/// `makepdftime` of texmfmp.c.
fn pdfship_make_pdf_time(t: i64, utc: bool) -> Vec<u8> {
    let lt = platform::broken_time(t, utc);
    let gmt = platform::broken_time(t, true);
    let second = if lt.second >= 60 { 59 } else { lt.second };
    let mut s = format!("D:{}{:02}{:02}{:02}{:02}{:02}", lt.year, lt.month, lt.day, lt.hour, lt.minute, second);
    // this calculation method was found in exim's tod.c
    let mut off = 60 * (lt.hour - gmt.hour) + lt.minute - gmt.minute;
    if lt.year != gmt.year {
        off += if lt.year > gmt.year { 1440 } else { -1440 };
    } else if (lt.month, lt.day) != (gmt.month, gmt.day) {
        off += if (lt.month, lt.day) > (gmt.month, gmt.day) { 1440 } else { -1440 };
    }
    if off == 0 {
        s.push('Z');
    } else {
        let off_hours = off / 60;
        let off_mins = (off - off_hours * 60).abs();
        s.push_str(&format!("{:+03}'{:02}'", off_hours, off_mins));
    }
    s.into_bytes()
}

/// `%.*d`: at least `prec` digits, with the sign outside them.
fn pdfship_precision(out: &mut Vec<u8>, n: i32, prec: usize) {
    if n < 0 {
        out.push(b'-');
    }
    out.extend_from_slice(format!("{:0prec$}", (n as i64).abs()).as_bytes());
}

/// The string `setjobid` builds.  `version` is `versionstring`, `VERSION_STRING` but for the oracle's table in a test: it
/// reaches `\pdfuniqueresname` through a CRC, so that name is not the oracle's, and it is the same as `\pdftexbanner`'s.
fn pdfship_job_id(year: i32, month: i32, day: i32, time: i32, name: &[u8], format: &[u8], version: &[u8]) -> Vec<u8> {
    let mut s = Vec::new();
    pdfship_precision(&mut s, year, 4);
    s.push(b'/');
    pdfship_precision(&mut s, month, 2);
    s.push(b'/');
    pdfship_precision(&mut s, day, 2);
    s.push(b' ');
    pdfship_precision(&mut s, time / 60, 2);
    s.push(b':');
    pdfship_precision(&mut s, time % 60, 2);
    for (sep, part) in [(b" ", name), (b" ", format), (b" ", BANNER), (b"", version), (b" ", KPATHSEA_VERSION_STRING)] as [(&[u8], &[u8]); 5] {
        s.extend_from_slice(sep);
        s.extend_from_slice(part);
    }
    s
}

/// zlib's `crc32`.
fn pdfship_crc32(data: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &b in data {
        crc ^= b as u32;
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xEDB88320 & (crc & 1).wrapping_neg());
        }
    }
    !crc
}

/// The tag `getresnameprefix` makes from the job id.
fn pdfship_resname_prefix(job_id: &[u8]) -> [u8; 6] {
    const NAME_STR: &[u8] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";
    let mut crc = pdfship_crc32(job_id) as usize;
    let mut prefix = [0u8; 6];
    for c in prefix.iter_mut() {
        *c = NAME_STR[crc % NAME_STR.len()];
        crc /= NAME_STR.len();
    }
    prefix
}

/// The value of `/ID`: the digest, in `convertStringToHexString`'s hexadecimal.
fn pdfship_id_hex(parts: &[&[u8]]) -> Vec<u8> {
    let mut m = rstex_pdfio::Md5::new();
    for p in parts {
        m.update(p);
    }
    let mut h = Vec::with_capacity(32);
    for b in m.finish() {
        h.extend_from_slice(format!("{:02X}", b).as_bytes());
    }
    h
}

impl Tex {
    /// §1618 and §1619, for §727's two users.  The result is in `def_ref`.
    fn pdfship_expand_write_toks(&mut self, p: i32) {
        // §1618 Expand macros in the token list and make |link(def_ref)| point to the result
        let q = self.tok_two(RIGHT_BRACE_TOKEN + b'}' as i32, END_WRITE_TOKEN);
        self.ins_list(q);
        self.begin_token_list(self.write_tokens(p), WRITE_TEXT);
        let q = self.tok_one(LEFT_BRACE_TOKEN + b'{' as i32);
        self.ins_list(q);
        let old_mode = self.mode();
        self.set_mode(0); // disable \prevdepth, \spacefactor, \lastskip, \prevgraf
        self.cur_cs = self.write_loc;
        self.scan_toks(false, true); // expand macros, etc.
        self.set_mode(old_mode);
        self.get_token();
        if self.cur_tok != END_WRITE_TOKEN {
            // §1619 Recover from an unbalanced write command
            self.print_err_s(b"Unbalanced write command");
            self.help(&[b"On this page there's a \\write with fewer real {'s than }'s.", b"I can't handle that very well; good luck."]);
            self.error();
            loop {
                self.get_token();
                if self.cur_tok == END_WRITE_TOKEN {
                    break;
                }
            }
        }
        self.end_token_list(); // conserve stack space
    }

    /// §727
    pub fn pdf_out_literal(&mut self, p: i32) {
        let old_setting = self.selector;
        let h;
        if self.subtype(p) == PDF_LATELITERAL_NODE {
            self.pdfship_expand_write_toks(p);
            h = self.def_ref;
        } else {
            h = self.pdf_literal_data(p);
        }
        self.selector = NEW_STRING;
        self.show_token_list(h + 1, NULL, self.pool_size - self.pool_ptr);
        self.selector = old_setting;
        let s = self.make_string();
        self.literal(s, self.pdf_literal_mode(p), false);
        self.flush_str(s);
        if self.subtype(p) == PDF_LATELITERAL_NODE {
            self.flush_ref(self.def_ref);
        }
    }

    /// §727
    pub fn pdf_out_colorstack(&mut self, p: i32) {
        let cmd = self.pdf_colorstack_cmd(p);
        let stack_no = self.pdf_colorstack_stack(p);
        if stack_no >= self.colorstackused() {
            self.print_nl_s(b"");
            self.print_s(b"Color stack ");
            self.print_int(stack_no);
            self.print_s(b" is not initialized for use!");
            self.print_nl_s(b"");
            return;
        }
        let literal_mode;
        match cmd {
            COLORSTACK_SET | COLORSTACK_PUSH => {
                let old_setting = self.selector;
                self.selector = NEW_STRING;
                self.show_token_list(self.pdf_colorstack_data(p) + 1, NULL, self.pool_size - self.pool_ptr);
                self.selector = old_setting;
                let s = self.make_string();
                let literal_mode = if cmd == COLORSTACK_SET { self.colorstackset(stack_no, s) } else { self.colorstackpush(stack_no, s) };
                if self.length(s) > 0 {
                    self.literal(s, literal_mode, false);
                }
                self.flush_str(s);
                return;
            }
            COLORSTACK_POP => literal_mode = self.colorstackpop(stack_no),
            COLORSTACK_CURRENT => literal_mode = self.colorstackcurrent(stack_no),
            _ => self.confusion(b"pdfcolorstack"),
        }
        if self.cur_length() > 0 {
            let s = self.make_string();
            self.literal(s, literal_mode, false);
            self.flush_str(s);
        }
    }

    /// §727
    pub fn pdf_out_colorstack_startpage(&mut self) {
        let mut i = 0;
        let max = self.colorstackused();
        while i < max {
            let start_status = self.colorstackskippagestart(i);
            if start_status == 0 {
                let literal_mode = self.colorstackcurrent(i);
                if self.cur_length() > 0 {
                    let s = self.make_string();
                    self.literal(s, literal_mode, false);
                    self.flush_str(s);
                }
            }
            i += 1;
        }
    }

    /// §727
    pub fn pdf_out_setmatrix(&mut self, p: i32) {
        let old_setting = self.selector;
        self.selector = NEW_STRING;
        self.show_token_list(self.pdf_setmatrix_data(p) + 1, NULL, self.pool_size - self.pool_ptr);
        self.selector = old_setting;
        self.str_room(7);
        self.str_pool.put(self.pool_ptr, 0); // make C string for pdfsetmatrix
        if self.pdfsetmatrix(self.str_start[self.str_ptr], self.cur_h, self.cur_page_height - self.cur_v) == 1 {
            self.str_room(7);
            self.append_char(b' ' as i32);
            self.append_char(b'0' as i32);
            self.append_char(b' ' as i32);
            self.append_char(b'0' as i32);
            self.append_char(b' ' as i32);
            self.append_char(b'c' as i32);
            self.append_char(b'm' as i32);
            let s = self.make_string();
            self.literal(s, SET_ORIGIN, false);
            self.flush_str(s);
        } else {
            self.pdf_error(b"\\pdfsetmatrix", b"Unrecognized format.");
        }
    }

    /// §727
    pub fn pdf_out_save(&mut self, _p: i32) {
        self.checkpdfsave(self.cur_h, self.cur_v);
        self.literal(b'q' as i32, SET_ORIGIN, false);
    }

    /// §727
    pub fn pdf_out_restore(&mut self, _p: i32) {
        self.checkpdfrestore(self.cur_h, self.cur_v);
        self.literal(b'Q' as i32, SET_ORIGIN, false);
    }

    /// §727
    pub fn pdf_special(&mut self, p: i32) {
        let old_setting = self.selector;
        let h;
        if self.subtype(p) == LATESPECIAL_NODE {
            self.pdfship_expand_write_toks(p);
            h = self.def_ref;
        } else {
            h = self.write_tokens(p);
        }
        self.selector = NEW_STRING;
        self.show_token_list(h + 1, NULL, self.pool_size - self.pool_ptr);
        self.selector = old_setting;
        let s = self.make_string();
        self.literal(s, SCAN_SPECIAL, true);
        self.flush_str(s);
        if self.subtype(p) == LATESPECIAL_NODE {
            self.flush_ref(self.def_ref);
        }
    }

    /// §727
    pub fn pdf_print_toks(&mut self, p: i32) {
        let s = self.tokens_to_string(p);
        if self.length(s) > 0 {
            self.pdf_print(s);
        }
        self.flush_str(s);
    }

    /// §727
    pub fn pdf_print_toks_ln(&mut self, p: i32) {
        let s = self.tokens_to_string(p);
        if self.length(s) > 0 {
            self.pdf_print_ln(s);
        }
        self.flush_str(s);
    }

    /// §1641 Save current position to |pdf_last_x_pos|, |pdf_last_y_pos|
    fn pdfship_save_pos(&mut self) {
        self.pdf_last_x_pos = self.cur_h;
        if self.is_shipping_page {
            self.pdf_last_y_pos = self.cur_page_height - self.cur_v;
        } else {
            self.pdf_last_y_pos = self.pdf_xform_height + self.pdf_xform_depth - self.cur_v;
        }
    }

    /// §729.  §728 is its forward declaration, which Rust does not need.
    pub fn pdf_hlist_out(&mut self) {
        let mut cur_g: i32 = 0;
        let mut cur_glue: f64 = 0.0;
        let this_box = self.temp_ptr;
        let g_order = self.glue_order(this_box);
        let g_sign = self.glue_sign(this_box);
        let mut p = self.list_ptr(this_box);
        self.cur_s += 1;
        let base_line = self.cur_v;
        let mut prev_p = this_box + LIST_OFFSET;
        // §1714 Initialize |hlist_out| for mixed direction typesetting
        if self.eTeX_ex() {
            // §1710 Initialize the LR stack
            self.put_LR(BEFORE); // this will never match
            if self.box_lr(this_box) == DLIST {
                if self.cur_dir == RIGHT_TO_LEFT {
                    self.cur_dir = LEFT_TO_RIGHT;
                    self.cur_h = self.cur_h - self.width(this_box);
                } else {
                    self.set_box_lr(this_box, 0);
                }
            }
            if self.cur_dir == RIGHT_TO_LEFT && self.box_lr(this_box) != REVERSED {
                // §1721 Reverse the complete hlist and set the subtype to |reversed|
                let save_h = self.cur_h;
                self.temp_ptr = p;
                p = self.new_kern(0);
                self.set_sync_tag(p + MEDIUM_NODE_SIZE, 0); // SyncTeX: do nothing, it is too late
                self.set_link(prev_p, p);
                self.cur_h = 0;
                let r = self.reverse(this_box, NULL, &mut cur_g, &mut cur_glue);
                self.set_link(p, r);
                self.set_width(p, -self.cur_h);
                self.cur_h = save_h;
                self.set_box_lr(this_box, REVERSED);
            }
        }
        let mut left_edge = self.cur_h;
        // §730 Create link annotations for the current hbox if needed
        for i in 1..=self.pdf_link_stack_ptr {
            debug_assert!(self.is_running(self.pdf_width(self.pdf_link_stack[i].link_node)));
            if self.pdf_link_stack[i].nesting_level == self.cur_s && self.gen_running_link {
                self.append_link(this_box, left_edge, base_line, i);
            }
        }
        // §1918 Start hlist SyncTeX information record
        self.synctex_hlist(this_box);
        while p != NULL {
            // §731 Output node |p| for |pdf_hlist_out| and move to the next node
            'reswitch: loop {
                if self.is_char_node(p) {
                    loop {
                        let f = self.font(p);
                        let c = self.character(p);
                        self.f = f;
                        self.c = c;
                        if self.is_valid_char(f, c) {
                            self.output_one_char(f, c);
                            self.cur_h = self.cur_h + self.char_width(f, self.char_info(f, c));
                        }
                        // dropped: MLTeX, §1887.  The label |continue| is here.
                        prev_p = self.link(prev_p); // N.B.: not |prev_p:=p|, |p| might be |lig_trick|
                        p = self.link(p);
                        if !self.is_char_node(p) {
                            break;
                        }
                    }
                    // §1921 Record current point SyncTeX information
                    self.synctex_current();
                    break 'reswitch;
                }
                // §732 Output the non-|char_node| |p| for |pdf_hlist_out| and move to the next node
                'next_p: {
                    'move_past: {
                        'fin_rule: {
                            match self.ty(p) {
                                HLIST_NODE | VLIST_NODE => {
                                    // §733 (\pdfTeX) Output a box in an hlist
                                    if self.list_ptr(p) == NULL {
                                        // §1920 Record void list SyncTeX information
                                        if self.ty(p) == VLIST_NODE {
                                            self.synctex_void_vlist(p, this_box);
                                        } else {
                                            self.synctex_void_hlist(p, this_box);
                                        }
                                        self.cur_h = self.cur_h + self.width(p);
                                    } else {
                                        self.cur_v = base_line + self.shift_amount(p); // shift the box down
                                        self.temp_ptr = p;
                                        let edge = self.cur_h + self.width(p);
                                        if self.cur_dir == RIGHT_TO_LEFT {
                                            self.cur_h = edge;
                                        }
                                        if self.ty(p) == VLIST_NODE {
                                            self.pdf_vlist_out();
                                        } else {
                                            self.pdf_hlist_out();
                                        }
                                        self.cur_h = edge;
                                        self.cur_v = base_line;
                                    }
                                }
                                RULE_NODE => {
                                    self.rule_ht = self.height(p);
                                    self.rule_dp = self.depth(p);
                                    self.rule_wd = self.width(p);
                                    break 'fin_rule;
                                }
                                WHATSIT_NODE => {
                                    // §1645 Output the whatsit node |p| in |pdf_hlist_out|
                                    match self.subtype(p) {
                                        PDF_LITERAL_NODE | PDF_LATELITERAL_NODE => self.pdf_out_literal(p),
                                        PDF_COLORSTACK_NODE => self.pdf_out_colorstack(p),
                                        PDF_SETMATRIX_NODE => self.pdf_out_setmatrix(p),
                                        PDF_SAVE_NODE => self.pdf_out_save(p),
                                        PDF_RESTORE_NODE => self.pdf_out_restore(p),
                                        PDF_REFOBJ_NODE => self.pdf_obj_list = self.append_ptr(self.pdf_obj_list, self.pdf_obj_objnum(p)),
                                        PDF_REFXFORM_NODE => {
                                            // §1647 Output a Form node in a hlist
                                            self.cur_v = base_line;
                                            let edge = self.cur_h;
                                            self.out_form(p);
                                            self.cur_h = edge + self.pdf_width(p);
                                            self.cur_v = base_line;
                                        }
                                        PDF_REFXIMAGE_NODE => {
                                            // §1646 Output a Image node in a hlist
                                            self.cur_v = base_line + self.pdf_depth(p);
                                            let edge = self.cur_h;
                                            self.out_image(p);
                                            self.cur_h = edge + self.pdf_width(p);
                                            self.cur_v = base_line;
                                        }
                                        PDF_ANNOT_NODE => self.do_annot(p, this_box, left_edge, base_line),
                                        PDF_START_LINK_NODE => self.do_link(p, this_box, left_edge, base_line),
                                        PDF_END_LINK_NODE => self.end_link(),
                                        PDF_DEST_NODE => self.do_dest(p, this_box, left_edge, base_line),
                                        PDF_THREAD_NODE => self.do_thread(p, this_box, left_edge, base_line),
                                        PDF_START_THREAD_NODE => self.pdf_error(b"ext4", b"\\pdfstartthread ended up in hlist"),
                                        PDF_END_THREAD_NODE => self.pdf_error(b"ext4", b"\\pdfendthread ended up in hlist"),
                                        PDF_SAVE_POS_NODE => self.pdfship_save_pos(),
                                        SPECIAL_NODE | LATESPECIAL_NODE => self.pdf_special(p),
                                        PDF_SNAP_REF_POINT_NODE => {
                                            // §1642 Save current position to |pdf_snapx_refpos|, |pdf_snapy_refpos|
                                            self.pdf_snapx_refpos = self.cur_h;
                                            self.pdf_snapy_refpos = self.cur_v;
                                        }
                                        PDF_SNAPY_COMP_NODE | PDF_SNAPY_NODE => {} // snapy nodes do nothing in hlist
                                        PDF_INTERWORD_SPACE_ON_NODE => self.gen_faked_interword_space = true,
                                        PDF_INTERWORD_SPACE_OFF_NODE => self.gen_faked_interword_space = false,
                                        PDF_FAKE_SPACE_NODE => self.pdf_insert_fake_space(),
                                        PDF_RUNNING_LINK_OFF_NODE => self.gen_running_link = false,
                                        PDF_RUNNING_LINK_ON_NODE => self.gen_running_link = true,
                                        _ => self.out_what(p),
                                    }
                                }
                                GLUE_NODE => {
                                    // §735 (\pdfTeX) Move right or output leaders
                                    self.round_glue(p, this_box, g_sign, g_order, &mut cur_g, &mut cur_glue);
                                    if self.eTeX_ex() {
                                        // §1699 Handle a glue node for mixed direction typesetting
                                        let g = self.g;
                                        if (g_sign == STRETCHING && self.stretch_order(g) == g_order) || (g_sign == SHRINKING && self.shrink_order(g) == g_order) {
                                            self.fast_delete_glue_ref(g);
                                            if self.subtype(p) < A_LEADERS {
                                                self.set_ty(p, KERN_NODE);
                                                self.set_width(p, self.rule_wd);
                                            } else {
                                                self.g = self.get_node(GLUE_SPEC_SIZE);
                                                let g = self.g;
                                                self.set_stretch_order(g, FILLL + 1); // will never match
                                                self.set_shrink_order(g, FILLL + 1);
                                                self.set_width(g, self.rule_wd);
                                                self.set_stretch(g, 0);
                                                self.set_shrink(g, 0);
                                                self.set_glue_ptr(p, g);
                                            }
                                        }
                                    }
                                    if self.subtype(p) >= A_LEADERS {
                                        // §736 (\pdfTeX) Output leaders in an hlist, |goto fin_rule| if a rule or to |next_p| if done
                                        let leader_box = self.leader_ptr(p);
                                        if self.ty(leader_box) == RULE_NODE {
                                            self.rule_ht = self.height(leader_box);
                                            self.rule_dp = self.depth(leader_box);
                                            break 'fin_rule;
                                        }
                                        let leader_wd = self.width(leader_box);
                                        if leader_wd > 0 && self.rule_wd > 0 {
                                            self.rule_wd = self.rule_wd + 10; // compensate for floating-point rounding
                                            if self.cur_dir == RIGHT_TO_LEFT {
                                                self.cur_h = self.cur_h - 10;
                                            }
                                            let edge = self.cur_h + self.rule_wd;
                                            let mut lx = 0;
                                            // §655 Let |cur_h| be the position of the first box, and set |leader_wd+lx| to the spacing
                                            if self.subtype(p) == A_LEADERS {
                                                let save_h = self.cur_h;
                                                self.cur_h = left_edge + leader_wd * ((self.cur_h - left_edge) / leader_wd);
                                                if self.cur_h < save_h {
                                                    self.cur_h = self.cur_h + leader_wd;
                                                }
                                            } else {
                                                self.lq = self.rule_wd / leader_wd; // the number of box copies
                                                self.lr = self.rule_wd % leader_wd; // the remaining space
                                                if self.subtype(p) == C_LEADERS {
                                                    self.cur_h = self.cur_h + (self.lr / 2);
                                                } else {
                                                    lx = self.lr / (self.lq + 1);
                                                    self.cur_h = self.cur_h + ((self.lr - (self.lq - 1) * lx) / 2);
                                                }
                                            }
                                            while self.cur_h + leader_wd <= edge {
                                                // §737 (\pdfTeX) Output a leader box at |cur_h|, then advance |cur_h| by |leader_wd+lx|
                                                self.cur_v = base_line + self.shift_amount(leader_box);
                                                let save_h = self.cur_h;
                                                self.temp_ptr = leader_box;
                                                if self.cur_dir == RIGHT_TO_LEFT {
                                                    self.cur_h = self.cur_h + leader_wd;
                                                }
                                                let outer_doing_leaders = self.doing_leaders;
                                                self.doing_leaders = true;
                                                if self.ty(leader_box) == VLIST_NODE {
                                                    self.pdf_vlist_out();
                                                } else {
                                                    self.pdf_hlist_out();
                                                }
                                                self.doing_leaders = outer_doing_leaders;
                                                self.cur_v = base_line;
                                                self.cur_h = save_h + leader_wd + lx;
                                            }
                                            if self.cur_dir == RIGHT_TO_LEFT {
                                                self.cur_h = edge;
                                            } else {
                                                self.cur_h = edge - 10;
                                            }
                                            break 'next_p;
                                        }
                                    }
                                    break 'move_past;
                                }
                                MARGIN_KERN_NODE => self.cur_h = self.cur_h + self.width(p),
                                KERN_NODE => {
                                    // §1923 Record |kern_node| SyncTeX information
                                    self.synctex_kern(p, this_box);
                                    self.cur_h = self.cur_h + self.width(p);
                                }
                                MATH_NODE => {
                                    // §1924 Record |math_node| SyncTeX information
                                    self.synctex_math(p, this_box);
                                    // §1716 Handle a math node in |hlist_out|
                                    if self.eTeX_ex() {
                                        // §1717 Adjust the LR stack for the |hlist_out| routine; if necessary reverse an hlist segment and |goto reswitch|
                                        if self.end_LR(p) {
                                            if self.info(self.LR_ptr) == self.end_LR_type(p) {
                                                self.pop_LR();
                                            } else if self.subtype(p) > L_CODE {
                                                self.LR_problems += 1;
                                            }
                                        } else {
                                            self.push_LR(p);
                                            if self.LR_dir(p) != self.cur_dir {
                                                // §1722 Reverse an hlist segment and |goto reswitch|
                                                let save_h = self.cur_h;
                                                self.temp_ptr = self.link(p);
                                                self.rule_wd = self.width(p);
                                                self.free_node(p, MEDIUM_NODE_SIZE); // SyncTeX: p is a |math_node|
                                                self.cur_dir = self.reflected();
                                                p = self.new_edge(self.cur_dir, self.rule_wd);
                                                self.set_link(prev_p, p);
                                                self.cur_h = self.cur_h - left_edge + self.rule_wd;
                                                let e = self.new_edge(self.reflected(), 0);
                                                let r = self.reverse(this_box, e, &mut cur_g, &mut cur_glue);
                                                self.set_link(p, r);
                                                self.set_edge_dist(p, self.cur_h);
                                                self.cur_dir = self.reflected();
                                                self.cur_h = save_h;
                                                continue 'reswitch;
                                            }
                                        }
                                        self.set_ty(p, KERN_NODE);
                                    }
                                    self.cur_h = self.cur_h + self.width(p);
                                }
                                LIGATURE_NODE => {
                                    // §826 Make node |p| look like a |char_node| and |goto reswitch|
                                    self.mem[LIG_TRICK] = self.mem[self.lig_char(p)];
                                    self.set_link(LIG_TRICK, self.link(p));
                                    p = LIG_TRICK;
                                    continue 'reswitch;
                                }
                                EDGE_NODE => {
                                    // §1720 Cases of |hlist_out| that arise in mixed direction text only
                                    self.cur_h = self.cur_h + self.width(p);
                                    left_edge = self.cur_h + self.edge_dist(p);
                                    self.cur_dir = self.subtype(p);
                                }
                                _ => {}
                            }
                            break 'next_p;
                        }
                        // fin_rule:
                        // §734 (\pdfTeX) Output a rule in an hlist
                        if self.is_running(self.rule_ht) {
                            self.rule_ht = self.height(this_box);
                        }
                        if self.is_running(self.rule_dp) {
                            self.rule_dp = self.depth(this_box);
                        }
                        self.rule_ht = self.rule_ht + self.rule_dp; // this is the rule thickness
                        if self.rule_ht > 0 && self.rule_wd > 0 {
                            // we don't output empty rules
                            self.cur_v = base_line + self.rule_dp;
                            self.pdf_set_rule(self.cur_h, self.cur_v, self.rule_wd, self.rule_ht);
                            self.cur_v = base_line;
                        }
                    }
                    // move_past:
                    self.cur_h = self.cur_h + self.rule_wd;
                    // §1922 Record horizontal |rule_node| or |glue_node| SyncTeX information
                    self.synctex_horizontal_rule_or_glue(p, this_box);
                }
                // next_p:
                prev_p = p;
                p = self.link(p);
                break 'reswitch;
            }
        }
        // §1919 Finish hlist SyncTeX information record
        self.synctex_tsilh(this_box);
        // §1715 Finish |hlist_out| for mixed direction typesetting
        if self.eTeX_ex() {
            // §1718 Check for LR anomalies at the end of |hlist_out|
            while self.info(self.LR_ptr) != BEFORE {
                if self.info(self.LR_ptr) > L_CODE {
                    self.LR_problems = self.LR_problems + 10000;
                }
                self.pop_LR();
            }
            self.pop_LR();
            if self.box_lr(this_box) == DLIST {
                self.cur_dir = RIGHT_TO_LEFT;
            }
        }
        self.cur_s -= 1;
    }

    /// §738
    pub fn pdf_vlist_out(&mut self) {
        let mut cur_g: i32 = 0;
        let mut cur_glue: f64 = 0.0;
        let this_box = self.temp_ptr;
        let g_order = self.glue_order(this_box);
        let g_sign = self.glue_sign(this_box);
        let mut p = self.list_ptr(this_box);
        self.cur_s += 1;
        let left_edge = self.cur_h;
        // §1916 Start vlist SyncTeX information record
        self.synctex_vlist(this_box);
        self.cur_v = self.cur_v - self.height(this_box);
        let top_edge = self.cur_v;
        // §739 Create thread for the current vbox if needed
        if self.last_thread != NULL && self.is_running(self.pdf_thread_dp) && self.pdf_thread_level == self.cur_s {
            self.append_thread(this_box, left_edge, top_edge + self.height(this_box));
        }
        while p != NULL {
            // §740 Output node |p| for |pdf_vlist_out| and move to the next node
            if self.is_char_node(p) {
                self.confusion(b"pdfvlistout");
            }
            // §741 Output the non-|char_node| |p| for |pdf_vlist_out|
            'next_p: {
                'move_past: {
                    'fin_rule: {
                        match self.ty(p) {
                            HLIST_NODE | VLIST_NODE => {
                                // §742 (\pdfTeX) Output a box in a vlist
                                if self.list_ptr(p) == NULL {
                                    self.cur_v = self.cur_v + self.height(p);
                                    // §1920 Record void list SyncTeX information
                                    if self.ty(p) == VLIST_NODE {
                                        self.synctex_void_vlist(p, this_box);
                                    } else {
                                        self.synctex_void_hlist(p, this_box);
                                    }
                                    self.cur_v = self.cur_v + self.depth(p);
                                } else {
                                    self.cur_v = self.cur_v + self.height(p);
                                    let save_v = self.cur_v;
                                    if self.cur_dir == RIGHT_TO_LEFT {
                                        self.cur_h = left_edge - self.shift_amount(p);
                                    } else {
                                        self.cur_h = left_edge + self.shift_amount(p); // shift the box right
                                    }
                                    self.temp_ptr = p;
                                    if self.ty(p) == VLIST_NODE {
                                        self.pdf_vlist_out();
                                    } else {
                                        self.pdf_hlist_out();
                                    }
                                    self.cur_v = save_v + self.depth(p);
                                    self.cur_h = left_edge;
                                }
                            }
                            RULE_NODE => {
                                self.rule_ht = self.height(p);
                                self.rule_dp = self.depth(p);
                                self.rule_wd = self.width(p);
                                break 'fin_rule;
                            }
                            WHATSIT_NODE => {
                                // §1639 Output the whatsit node |p| in |pdf_vlist_out|
                                match self.subtype(p) {
                                    PDF_LITERAL_NODE | PDF_LATELITERAL_NODE => self.pdf_out_literal(p),
                                    PDF_COLORSTACK_NODE => self.pdf_out_colorstack(p),
                                    PDF_SETMATRIX_NODE => self.pdf_out_setmatrix(p),
                                    PDF_SAVE_NODE => self.pdf_out_save(p),
                                    PDF_RESTORE_NODE => self.pdf_out_restore(p),
                                    PDF_REFOBJ_NODE => self.pdf_obj_list = self.append_ptr(self.pdf_obj_list, self.pdf_obj_objnum(p)),
                                    PDF_REFXFORM_NODE => {
                                        // §1644 Output a Form node in a vlist
                                        self.cur_v = self.cur_v + self.pdf_height(p);
                                        let save_v = self.cur_v;
                                        self.cur_h = left_edge;
                                        self.out_form(p);
                                        self.cur_v = save_v + self.pdf_depth(p);
                                        self.cur_h = left_edge;
                                    }
                                    PDF_REFXIMAGE_NODE => {
                                        // §1643 Output a Image node in a vlist
                                        self.cur_v = self.cur_v + self.pdf_height(p) + self.pdf_depth(p);
                                        let save_v = self.cur_v;
                                        self.cur_h = left_edge;
                                        self.out_image(p);
                                        self.cur_v = save_v;
                                        self.cur_h = left_edge;
                                    }
                                    PDF_ANNOT_NODE => self.do_annot(p, this_box, left_edge, top_edge + self.height(this_box)),
                                    PDF_START_LINK_NODE => self.pdf_error(b"ext4", b"\\pdfstartlink ended up in vlist"),
                                    PDF_END_LINK_NODE => self.pdf_error(b"ext4", b"\\pdfendlink ended up in vlist"),
                                    PDF_DEST_NODE => self.do_dest(p, this_box, left_edge, top_edge + self.height(this_box)),
                                    PDF_THREAD_NODE | PDF_START_THREAD_NODE => self.do_thread(p, this_box, left_edge, top_edge + self.height(this_box)),
                                    PDF_END_THREAD_NODE => self.end_thread(),
                                    PDF_SAVE_POS_NODE => self.pdfship_save_pos(),
                                    SPECIAL_NODE | LATESPECIAL_NODE => self.pdf_special(p),
                                    PDF_SNAP_REF_POINT_NODE => {
                                        // §1642 Save current position to |pdf_snapx_refpos|, |pdf_snapy_refpos|
                                        self.pdf_snapx_refpos = self.cur_h;
                                        self.pdf_snapy_refpos = self.cur_v;
                                    }
                                    PDF_SNAPY_COMP_NODE => self.do_snapy_comp(p, this_box),
                                    PDF_SNAPY_NODE => self.do_snapy(p),
                                    PDF_INTERWORD_SPACE_ON_NODE => self.gen_faked_interword_space = true,
                                    PDF_INTERWORD_SPACE_OFF_NODE => self.gen_faked_interword_space = false,
                                    PDF_FAKE_SPACE_NODE => self.pdf_insert_fake_space(),
                                    PDF_RUNNING_LINK_OFF_NODE => self.gen_running_link = false,
                                    PDF_RUNNING_LINK_ON_NODE => self.gen_running_link = true,
                                    _ => self.out_what(p),
                                }
                            }
                            GLUE_NODE => {
                                // §744 (\pdfTeX) Move down or output leaders
                                self.g = self.glue_ptr(p);
                                let g = self.g;
                                self.rule_ht = self.width(g) - cur_g;
                                if g_sign != NORMAL {
                                    if g_sign == STRETCHING {
                                        if self.stretch_order(g) == g_order {
                                            cur_glue = cur_glue + self.stretch(g) as f64;
                                            let glue_temp = vet_glue(self.glue_set(this_box) * cur_glue);
                                            cur_g = round(glue_temp);
                                        }
                                    } else if self.shrink_order(g) == g_order {
                                        cur_glue = cur_glue - self.shrink(g) as f64;
                                        let glue_temp = vet_glue(self.glue_set(this_box) * cur_glue);
                                        cur_g = round(glue_temp);
                                    }
                                }
                                self.rule_ht = self.rule_ht + cur_g;
                                if self.subtype(p) >= A_LEADERS {
                                    // §745 (\pdfTeX) Output leaders in a vlist, |goto fin_rule| if a rule or to |next_p| if done
                                    let leader_box = self.leader_ptr(p);
                                    if self.ty(leader_box) == RULE_NODE {
                                        self.rule_wd = self.width(leader_box);
                                        self.rule_dp = 0;
                                        break 'fin_rule;
                                    }
                                    let leader_ht = self.height(leader_box) + self.depth(leader_box);
                                    if leader_ht > 0 && self.rule_ht > 0 {
                                        self.rule_ht = self.rule_ht + 10; // compensate for floating-point rounding
                                        let edge = self.cur_v + self.rule_ht;
                                        let mut lx = 0;
                                        // §664 Let |cur_v| be the position of the first box, and set |leader_ht+lx| to the spacing
                                        if self.subtype(p) == A_LEADERS {
                                            let save_v = self.cur_v;
                                            self.cur_v = top_edge + leader_ht * ((self.cur_v - top_edge) / leader_ht);
                                            if self.cur_v < save_v {
                                                self.cur_v = self.cur_v + leader_ht;
                                            }
                                        } else {
                                            self.lq = self.rule_ht / leader_ht; // the number of box copies
                                            self.lr = self.rule_ht % leader_ht; // the remaining space
                                            if self.subtype(p) == C_LEADERS {
                                                self.cur_v = self.cur_v + (self.lr / 2);
                                            } else {
                                                lx = self.lr / (self.lq + 1);
                                                self.cur_v = self.cur_v + ((self.lr - (self.lq - 1) * lx) / 2);
                                            }
                                        }
                                        while self.cur_v + leader_ht <= edge {
                                            // §746 (\pdfTeX) Output a leader box at |cur_v|, then advance |cur_v| by |leader_ht+lx|
                                            if self.cur_dir == RIGHT_TO_LEFT {
                                                self.cur_h = left_edge - self.shift_amount(leader_box);
                                            } else {
                                                self.cur_h = left_edge + self.shift_amount(leader_box);
                                            }
                                            self.cur_v = self.cur_v + self.height(leader_box);
                                            let save_v = self.cur_v;
                                            self.temp_ptr = leader_box;
                                            let outer_doing_leaders = self.doing_leaders;
                                            self.doing_leaders = true;
                                            if self.ty(leader_box) == VLIST_NODE {
                                                self.pdf_vlist_out();
                                            } else {
                                                self.pdf_hlist_out();
                                            }
                                            self.doing_leaders = outer_doing_leaders;
                                            self.cur_h = left_edge;
                                            self.cur_v = save_v - self.height(leader_box) + leader_ht + lx;
                                        }
                                        self.cur_v = edge - 10;
                                        break 'next_p;
                                    }
                                }
                                break 'move_past;
                            }
                            KERN_NODE => self.cur_v = self.cur_v + self.width(p),
                            _ => {}
                        }
                        break 'next_p;
                    }
                    // fin_rule:
                    // §743 (\pdfTeX) Output a rule in a vlist, |goto next_p|
                    if self.is_running(self.rule_wd) {
                        self.rule_wd = self.width(this_box);
                    }
                    self.rule_ht = self.rule_ht + self.rule_dp; // this is the rule thickness
                    self.cur_v = self.cur_v + self.rule_ht;
                    if self.rule_ht > 0 && self.rule_wd > 0 {
                        // we don't output empty rules
                        if self.cur_dir == RIGHT_TO_LEFT {
                            self.cur_h = self.cur_h - self.rule_wd;
                        }
                        self.pdf_set_rule(self.cur_h, self.cur_v, self.rule_wd, self.rule_ht);
                        self.cur_h = left_edge;
                    }
                    break 'next_p;
                }
                // move_past:
                self.cur_v = self.cur_v + self.rule_ht;
            }
            // next_p:
            p = self.link(p);
        }
        // §1917 Finish vlist SyncTeX information record
        self.synctex_tsilv(this_box);
        self.cur_s -= 1;
    }

    /// §747
    pub fn fix_pdfoutput(&mut self) {
        if !self.fixed_pdfoutput_set {
            self.fixed_pdfoutput = self.pdf_output();
            self.fixed_pdfoutput_set = true;
        } else if self.fixed_pdfoutput != self.pdf_output() {
            self.pdf_error(b"setup", b"\\pdfoutput can only be changed before anything is written to the output");
        }
        if self.fixed_pdfoutput_set {
            self.fix_pdf_draftmode();
        }
    }

    /// §748
    pub fn fix_pdf_draftmode(&mut self) {
        if !self.fixed_pdf_draftmode_set {
            self.fixed_pdf_draftmode = self.pdf_draftmode();
            self.fixed_pdf_draftmode_set = true;
        } else if self.fixed_pdf_draftmode != self.pdf_draftmode() {
            self.pdf_error(b"setup", b"\\pdfdraftmode can only be changed before anything is written to the output");
        }
        if self.fixed_pdf_draftmode_set && self.fixed_pdf_draftmode > 0 {
            self.fixed_pdf_draftmode_set = true;
            self.set_pdf_compress_level(0);
            self.fixed_pdf_objcompresslevel = 0;
        }
    }

    /// §749.  The first test is strict, so `s` at the very end of `t` is not found: `/MediaBox` alone in `\pdfpageattr`
    /// still gets the default one.
    pub fn substr_of_str(&mut self, s: &[u8], t: i32) -> bool {
        let mut k = self.str_start[t];
        'next: while k < self.str_start[t + 1] - s.len() as i32 {
            let mut kk = k;
            for &c in s {
                if c as i32 != self.str_pool.at(kk) {
                    k += 1;
                    continue 'next;
                }
                kk += 1;
            }
            return true;
        }
        false
    }

    /// §750
    pub fn pdf_ship_out(&mut self, p: i32, shipping_page: bool) {
        if self.tracing_output() > 0 {
            self.print_nl_s(b"");
            self.print_ln();
            self.print_s(b"Completed box being shipped out");
        }
        if !self.init_pdf_output {
            // §792 Initialize variables for PDF output
            self.check_pdfversion();
            self.prepare_mag();
            self.fixed_decimal_digits = self.fix_int(self.pdf_decimal_digits(), 0, 4);
            self.min_bp_val = self.divide_scaled(self.one_hundred_bp, self.ten_pow[self.fixed_decimal_digits + 2], 0);
            if self.pdf_pk_resolution() == 0 {
                // if not set from format file or by user: take it from texmf.cnf
                self.set_pdf_pk_resolution(self.pk_dpi);
            }
            self.fixed_pk_resolution = self.fix_int(self.pdf_pk_resolution(), 72, 8000);
            self.pk_scale_factor = self.divide_scaled(72, self.fixed_pk_resolution, 5 + self.fixed_decimal_digits);
            if self.pdf_pk_mode() != NULL {
                let s = self.tokens_to_string(self.pdf_pk_mode());
                let mode = self.make_cstring(s);
                self.kpse_init_prog(b"PDFTEX", self.fixed_pk_resolution, &mode, b"");
                self.flush_string();
            } else {
                self.kpse_init_prog(b"PDFTEX", self.fixed_pk_resolution, b"", b"");
            }
            self.kpse_set_program_enabled(rstex_kpse::KPSE_PK_FORMAT, true, KPSE_SRC_COMPILE);
            self.set_job_id(self.year(), self.month(), self.day(), self.time());
            if self.pdf_unique_resname() > 0 && self.pdf_resname_prefix == 0 {
                self.pdf_resname_prefix = self.get_resname_prefix();
            }
            self.init_pdf_output = true;
        }
        self.is_shipping_page = shipping_page;
        if shipping_page {
            if self.term_offset > self.max_print_line - 9 {
                self.print_ln();
            } else if self.term_offset > 0 || self.file_offset > 0 {
                self.print_char(b' ' as i32);
            }
            self.print_char(b'[' as i32);
            let mut j = 9;
            while self.count(j) == 0 && j > 0 {
                j -= 1;
            }
            for k in 0..=j {
                self.print_int(self.count(k));
                if k < j {
                    self.print_char(b'.' as i32);
                }
            }
            self.update_terminal();
        }
        if self.tracing_output() > 0 {
            if shipping_page {
                self.print_char(b']' as i32);
            }
            self.begin_diagnostic();
            self.show_box(p);
            self.end_diagnostic(true);
        }
        self.pdf_output_value = self.pdf_output(); // SyncTeX: we assume that |pdf_output| is properly set up
        if shipping_page {
            self.synctex_sheet(self.mag());
        } else {
            self.synctex_pdfxform(p);
        }
        // §751 (\pdfTeX) Ship box |p| out
        'done: {
            // §669 Update the values of |max_h| and |max_v|; but if the page is too large, |goto done|
            let (ht, dp, wd) = (self.height(p) as i64, self.depth(p) as i64, self.width(p) as i64);
            if ht > MAX_DIMEN as i64
                || dp > MAX_DIMEN as i64
                || ht + dp + self.v_offset() as i64 > MAX_DIMEN as i64
                || wd + self.h_offset() as i64 > MAX_DIMEN as i64
            {
                self.print_err_s(b"Huge page cannot be shipped out");
                self.help(&[b"The page just created is more than 18 feet tall or", b"more than 18 feet wide, so I suspect something went wrong."]);
                self.error();
                if self.tracing_output() <= 0 {
                    self.begin_diagnostic();
                    self.print_nl_s(b"The following box has been deleted:");
                    self.show_box(p);
                    self.end_diagnostic(true);
                }
                break 'done;
            }
            if ht + dp + self.v_offset() as i64 > self.max_v as i64 {
                self.max_v = (ht + dp + self.v_offset() as i64) as i32;
            }
            if wd + self.h_offset() as i64 > self.max_h as i64 {
                self.max_h = (wd + self.h_offset() as i64) as i32;
            }
            // §752 Initialize variables as |pdf_ship_out| begins
            self.fix_pdfoutput();
            self.temp_ptr = p;
            self.prepare_mag();
            let pdf_last_resources = self.pdf_new_objnum();
            self.pdf_page_group_val = 0;
            // §753 Reset resource lists
            self.pdfship_reset_resource_lists();
            if !shipping_page {
                self.pdf_xform_width = self.width(p);
                self.pdf_xform_height = self.height(p);
                self.pdf_xform_depth = self.depth(p);
                self.pdf_begin_dict(self.pdf_cur_form, 0);
                self.pdf_last_stream = self.pdf_cur_form;
                self.cur_v = self.height(p);
                self.cur_h = 0;
                self.pdf_origin_h = 0;
                self.pdf_origin_v = self.pdf_xform_height + self.pdf_xform_depth;
            } else {
                // §755 Calculate page dimensions and margins
                self.cur_h_offset = sat32(self.pdf_h_origin() as i64 + self.h_offset() as i64);
                self.cur_v_offset = sat32(self.pdf_v_origin() as i64 + self.v_offset() as i64);
                if self.pdf_page_width() != 0 {
                    self.cur_page_width = self.pdf_page_width();
                } else {
                    self.cur_page_width = sat32(wd + 2 * self.cur_h_offset as i64);
                }
                if self.pdf_page_height() != 0 {
                    self.cur_page_height = self.pdf_page_height();
                } else {
                    self.cur_page_height = sat32(ht + dp + 2 * self.cur_v_offset as i64);
                }
                self.pdf_last_page = self.get_obj(OBJ_TYPE_PAGE, self.total_pages + 1, false);
                self.set_obj_aux(self.pdf_last_page, 1); // mark that this page has been created
                self.pdf_new_dict(OBJ_TYPE_OTHERS, 0, 0);
                self.pdf_last_stream = self.obj_ptr;
                self.cur_h = self.cur_h_offset;
                self.cur_v = sat32(ht + self.cur_v_offset as i64);
                self.pdf_origin_h = 0;
                self.pdf_origin_v = self.cur_page_height;
                // §754 Reset PDF mark lists
                self.pdf_annot_list = NULL;
                self.pdf_link_list = NULL;
                self.pdf_dest_list = NULL;
                self.pdf_bead_list = NULL;
                self.last_thread = NULL;
            }
            if !shipping_page {
                // §756 Write out Form stream header
                self.pdf_print_ln_s(b"/Type /XObject");
                self.pdf_print_ln_s(b"/Subtype /Form");
                if self.obj_xform_attr(self.pdf_cur_form) != NULL {
                    let a = self.obj_xform_attr(self.pdf_cur_form);
                    self.pdf_print_toks_ln(a);
                    self.delete_token_ref(a);
                    self.set_obj_xform_attr(self.pdf_cur_form, NULL);
                }
                self.pdf_print_s(b"/BBox [");
                self.pdf_print_s(b"0 0 ");
                self.pdf_print_bp(self.pdf_xform_width);
                self.pdf_out(b' ' as i32);
                self.pdf_print_bp(self.pdf_xform_height + self.pdf_xform_depth);
                self.pdf_print_ln_s(b"]");
                self.pdf_print_ln_s(b"/FormType 1");
                self.pdf_print_ln_s(b"/Matrix [1 0 0 1 0 0]");
                self.pdf_indirect_ln(b"Resources", pdf_last_resources);
            }
            // §757 Start stream of page/form contents
            self.pdf_begin_stream();
            if shipping_page {
                // §758 Adjust transformation matrix for the magnification ratio
                self.prepare_mag();
                if self.mag() != 1000 {
                    self.pdf_print_real(self.mag(), 3);
                    self.pdf_print_s(b" 0 0 ");
                    self.pdf_print_real(self.mag(), 3);
                    self.pdf_print_ln_s(b" 0 0 cm");
                }
            }
            self.pdfshipoutbegin(shipping_page);
            if shipping_page {
                self.pdf_out_colorstack_startpage();
            }
            if self.ty(p) == VLIST_NODE {
                self.pdf_vlist_out();
            } else {
                self.pdf_hlist_out();
            }
            if shipping_page {
                self.total_pages += 1;
                if self.host.checkpoint.page.active {
                    self.v_image.ck.page_due = true;
                }
            }
            self.cur_s = -1;
            // §759 Finish shipping
            // §760 Finish stream of page/form contents
            self.pdf_end_text();
            self.pdfshipoutend(shipping_page);
            self.pdf_end_stream();
            if shipping_page {
                // §769 Write out page object
                self.pdf_begin_dict(self.pdf_last_page, 1);
                self.pdf_print_ln_s(b"/Type /Page");
                self.pdf_indirect_ln(b"Contents", self.pdf_last_stream);
                self.pdf_indirect_ln(b"Resources", pdf_last_resources);
                let mut mediabox_given = false;
                if self.pdf_page_attr() != NULL {
                    let s = self.tokens_to_string(self.pdf_page_attr());
                    mediabox_given = self.substr_of_str(b"/MediaBox", s);
                    self.flush_str(s);
                }
                if !mediabox_given {
                    self.pdf_print_s(b"/MediaBox [0 0 ");
                    self.pdf_print_mag_bp(self.cur_page_width);
                    self.pdf_out(b' ' as i32);
                    self.pdf_print_mag_bp(self.cur_page_height);
                    self.pdf_print_ln_s(b"]");
                }
                if self.pdf_page_attr() != NULL {
                    self.pdf_print_toks_ln(self.pdf_page_attr());
                }
                // §770 Generate parent pages object
                if self.total_pages % PAGES_TREE_KIDS_MAX == 1 {
                    self.pdf_create_obj(OBJ_TYPE_PAGES, PAGES_TREE_KIDS_MAX);
                    self.pdf_last_pages = self.obj_ptr;
                }
                self.pdf_indirect_ln(b"Parent", self.pdf_last_pages);
                if self.pdf_page_group_val > 0 {
                    self.pdf_print_s(b"/Group ");
                    self.pdf_print_int(self.pdf_page_group_val);
                    self.pdf_print_ln_s(b" 0 R");
                }
                // §771 Generate array of annotations or beads in page
                if self.pdf_annot_list != NULL || self.pdf_link_list != NULL {
                    self.pdf_print_s(b"/Annots [ ");
                    let mut k = self.pdf_annot_list;
                    while k != NULL {
                        self.pdf_print_int(self.info(k));
                        self.pdf_print_s(b" 0 R ");
                        k = self.link(k);
                    }
                    k = self.pdf_link_list;
                    while k != NULL {
                        self.pdf_print_int(self.info(k));
                        self.pdf_print_s(b" 0 R ");
                        k = self.link(k);
                    }
                    self.pdf_print_ln_s(b"]");
                }
                if self.pdf_bead_list != NULL {
                    let mut k = self.pdf_bead_list;
                    self.pdf_print_s(b"/B [ ");
                    while k != NULL {
                        self.pdf_print_int(self.info(k));
                        self.pdf_print_s(b" 0 R ");
                        k = self.link(k);
                    }
                    self.pdf_print_ln_s(b"]");
                }
                self.pdf_end_dict();
            }
            // §761 Write out resource lists
            // §773 Write out pending raw objects
            if self.pdf_obj_list != NULL {
                let mut k = self.pdf_obj_list;
                while k != NULL {
                    if !self.is_obj_written(self.info(k)) {
                        self.pdf_write_obj(self.info(k));
                    }
                    k = self.link(k);
                }
            }
            // §779 Write out pending images
            if self.pdf_ximage_list != NULL {
                let mut k = self.pdf_ximage_list;
                while k != NULL {
                    if !self.is_obj_written(self.info(k)) {
                        self.pdf_write_image(self.info(k));
                    }
                    k = self.link(k);
                }
            }
            // §775 Write out pending forms
            if self.pdf_xform_list != NULL {
                let mut k = self.pdf_xform_list;
                while k != NULL {
                    if !self.is_obj_written(self.info(k)) {
                        self.saved_pdf_cur_form = self.pdf_cur_form;
                        self.pdf_cur_form = self.info(k);
                        // §776 Save resource lists
                        let save_font_list = self.pdf_font_list;
                        let save_obj_list = self.pdf_obj_list;
                        let save_xform_list = self.pdf_xform_list;
                        let save_ximage_list = self.pdf_ximage_list;
                        let save_text_procset = self.pdf_text_procset;
                        let save_image_procset = self.pdf_image_procset;
                        // §753 Reset resource lists
                        self.pdfship_reset_resource_lists();
                        self.pdf_ship_out(self.obj_xform_box(self.pdf_cur_form), false);
                        self.pdf_cur_form = self.saved_pdf_cur_form;
                        // §777 Restore resource lists
                        self.pdf_font_list = save_font_list;
                        self.pdf_obj_list = save_obj_list;
                        self.pdf_xform_list = save_xform_list;
                        self.pdf_ximage_list = save_ximage_list;
                        self.pdf_text_procset = save_text_procset;
                        self.pdf_image_procset = save_image_procset;
                    }
                    k = self.link(k);
                }
            }
            if shipping_page {
                // §780 Write out pending PDF marks
                self.pdf_origin_h = 0;
                self.pdf_origin_v = self.cur_page_height;
                // §781 Write out PDF annotations
                if self.pdf_annot_list != NULL {
                    let mut k = self.pdf_annot_list;
                    while k != NULL {
                        let i = self.obj_annot_ptr(self.info(k)); // |i| points to |pdf_annot_node|
                        self.pdf_begin_dict(self.info(k), 1);
                        self.pdf_print_ln_s(b"/Type /Annot");
                        self.pdf_print_toks_ln(self.pdf_annot_data(i));
                        self.pdf_rectangle(self.pdf_left(i), self.pdf_top(i), self.pdf_right(i), self.pdf_bottom(i));
                        self.pdf_end_dict();
                        k = self.link(k);
                    }
                }
                // §782 Write out PDF link annotations
                if self.pdf_link_list != NULL {
                    let mut k = self.pdf_link_list;
                    while k != NULL {
                        let i = self.obj_annot_ptr(self.info(k));
                        self.pdf_begin_dict(self.info(k), 1);
                        self.pdf_print_ln_s(b"/Type /Annot");
                        if self.pdf_action_type(self.pdf_link_action(i)) != PDF_ACTION_USER {
                            self.pdf_print_ln_s(b"/Subtype /Link");
                        }
                        if self.pdf_link_attr(i) != NULL {
                            self.pdf_print_toks_ln(self.pdf_link_attr(i));
                        }
                        self.pdf_rectangle(self.pdf_left(i), self.pdf_top(i), self.pdf_right(i), self.pdf_bottom(i));
                        if self.pdf_action_type(self.pdf_link_action(i)) != PDF_ACTION_USER {
                            self.pdf_print_s(b"/A ");
                        }
                        self.write_action(self.pdf_link_action(i));
                        self.pdf_end_dict();
                        k = self.link(k);
                    }
                    // §783 Flush |pdf_start_link_node|'s created by |append_link|
                    k = self.pdf_link_list;
                    while k != NULL {
                        let i = self.obj_annot_ptr(self.info(k));
                        // nodes with |info = max_halfword| were created by |append_link| and
                        // must be flushed here, as they are not linked in any list
                        if self.info(i) == MAX_HALFWORD {
                            self.flush_whatsit_node(i, PDF_START_LINK_NODE);
                        }
                        k = self.link(k);
                    }
                }
                // §784 Write out PDF mark destinations
                if self.pdf_dest_list != NULL {
                    let mut k = self.pdf_dest_list;
                    while k != NULL {
                        if self.is_obj_written(self.info(k)) {
                            self.pdf_error(b"ext5", b"destination has been already written (this shouldn't happen)");
                        } else {
                            let i = self.obj_dest_ptr(self.info(k));
                            if self.pdf_dest_named_id(i) > 0 && self.pdf_dest_objnum(i) == NULL {
                                self.pdf_begin_dict(self.info(k), 1);
                                self.pdf_print_s(b"/D ");
                            } else {
                                self.pdf_begin_obj(self.info(k), 1);
                            }
                            self.pdf_out(b'[' as i32);
                            if self.pdf_dest_objnum(i) == NULL {
                                self.pdf_print_int(self.pdf_last_page);
                            } else {
                                self.pdf_print_int(self.pdf_dest_objnum(i));
                            }
                            self.pdf_print_s(b" 0 R ");
                            match self.pdf_dest_type(i) {
                                PDF_DEST_XYZ => {
                                    self.pdf_print_s(b"/XYZ ");
                                    self.pdf_print_mag_bp(self.pdf_x(self.pdf_left(i)));
                                    self.pdf_out(b' ' as i32);
                                    self.pdf_print_mag_bp(self.pdf_y(self.pdf_top(i)));
                                    self.pdf_out(b' ' as i32);
                                    if self.pdf_dest_xyz_zoom(i) == PDF_ZOOM_NULL {
                                        self.pdf_print_s(b"null");
                                    } else {
                                        self.pdf_print_int(self.pdf_dest_xyz_zoom(i) / 1000);
                                        self.pdf_out(b'.' as i32);
                                        self.pdf_print_int(self.pdf_dest_xyz_zoom(i) % 1000);
                                    }
                                }
                                PDF_DEST_FIT => self.pdf_print_s(b"/Fit"),
                                PDF_DEST_FITH => {
                                    self.pdf_print_s(b"/FitH ");
                                    self.pdf_print_mag_bp(self.pdf_y(self.pdf_top(i)));
                                }
                                PDF_DEST_FITV => {
                                    self.pdf_print_s(b"/FitV ");
                                    self.pdf_print_mag_bp(self.pdf_x(self.pdf_left(i)));
                                }
                                PDF_DEST_FITB => self.pdf_print_s(b"/FitB"),
                                PDF_DEST_FITBH => {
                                    self.pdf_print_s(b"/FitBH ");
                                    self.pdf_print_mag_bp(self.pdf_y(self.pdf_top(i)));
                                }
                                PDF_DEST_FITBV => {
                                    self.pdf_print_s(b"/FitBV ");
                                    self.pdf_print_mag_bp(self.pdf_x(self.pdf_left(i)));
                                }
                                PDF_DEST_FITR => {
                                    self.pdf_print_s(b"/FitR ");
                                    self.pdf_print_rect_spec(i);
                                }
                                _ => self.pdf_error(b"ext5", b"unknown dest type"),
                            }
                            self.pdf_print_ln_s(b"]");
                            if self.pdf_dest_named_id(i) > 0 && self.pdf_dest_objnum(i) == NULL {
                                self.pdf_end_dict();
                            } else {
                                self.pdf_end_obj();
                            }
                        }
                        k = self.link(k);
                    }
                }
                // §786 Write out PDF bead rectangle specifications
                if self.pdf_bead_list != NULL {
                    let mut k = self.pdf_bead_list;
                    while k != NULL {
                        self.pdf_new_obj(OBJ_TYPE_OTHERS, 0, 1);
                        self.pdf_out(b'[' as i32);
                        let i = self.obj_bead_data(self.info(k)); // pointer to a whatsit or whatsit-like node
                        self.pdf_print_rect_spec(i);
                        if self.info(i) == MAX_HALFWORD {
                            // not a whatsit node, so must be destroyed here
                            self.flush_whatsit_node(i, PDF_START_THREAD_NODE);
                        }
                        self.pdf_print_ln_s(b"]");
                        self.set_obj_bead_rect(self.info(k), self.obj_ptr); // rewrite |obj_bead_data|
                        self.pdf_end_obj();
                        k = self.link(k);
                    }
                }
            }
            // §762 Write out resources dictionary
            self.pdf_begin_dict(pdf_last_resources, 1);
            // §763 Print additional resources
            if shipping_page {
                if self.pdf_page_resources() != NULL {
                    self.pdf_print_toks_ln(self.pdf_page_resources());
                }
            } else if self.obj_xform_resources(self.pdf_cur_form) != NULL {
                let r = self.obj_xform_resources(self.pdf_cur_form);
                self.pdf_print_toks_ln(r);
                self.delete_token_ref(r);
                self.set_obj_xform_resources(self.pdf_cur_form, NULL);
            }
            // §766 Generate font resources
            if self.pdf_font_list != NULL {
                self.pdf_print_s(b"/Font << ");
                let mut k = self.pdf_font_list;
                while k != NULL {
                    self.pdf_print_s(b"/F");
                    self.set_ff(self.info(k));
                    self.pdf_print_int(self.ff);
                    self.pdf_print_resname_prefix();
                    self.pdf_out(b' ' as i32);
                    self.pdf_print_int(self.pdf_font_num[self.ff]);
                    self.pdf_print_s(b" 0 R ");
                    k = self.link(k);
                }
                self.pdf_print_ln_s(b">>");
                self.pdf_text_procset = true;
            }
            // §767 Generate XObject resources
            if self.pdf_xform_list != NULL || self.pdf_ximage_list != NULL {
                self.pdf_print_s(b"/XObject << ");
                let mut k = self.pdf_xform_list;
                while k != NULL {
                    self.pdf_print_s(b"/Fm");
                    self.pdf_print_int(self.obj_info(self.info(k)));
                    self.pdf_print_resname_prefix();
                    self.pdf_out(b' ' as i32);
                    self.pdf_print_int(self.info(k));
                    self.pdf_print_s(b" 0 R ");
                    k = self.link(k);
                }
                k = self.pdf_ximage_list;
                while k != NULL {
                    self.pdf_print_s(b"/Im");
                    self.pdf_print_int(self.obj_info(self.info(k)));
                    self.pdf_print_resname_prefix();
                    self.pdf_out(b' ' as i32);
                    self.pdf_print_int(self.info(k));
                    self.pdf_print_s(b" 0 R ");
                    self.update_image_procset(self.obj_ximage_data(self.info(k)));
                    k = self.link(k);
                }
                self.pdf_print_ln_s(b">>");
            }
            // §768 Generate ProcSet if desired
            if self.pdf_omit_procset() < 0 || (self.pdf_omit_procset() == 0 && self.pdf_major_version() < 2) {
                self.pdf_print_s(b"/ProcSet [ /PDF");
                if self.pdf_text_procset {
                    self.pdf_print_s(b" /Text");
                }
                if self.check_image_b(self.pdf_image_procset) {
                    self.pdf_print_s(b" /ImageB");
                }
                if self.check_image_c(self.pdf_image_procset) {
                    self.pdf_print_s(b" /ImageC");
                }
                if self.check_image_i(self.pdf_image_procset) {
                    self.pdf_print_s(b" /ImageI");
                }
                self.pdf_print_ln_s(b" ]");
            }
            self.pdf_end_dict();
            // §764 Flush resource lists
            self.mem_flush_list(self.pdf_font_list);
            self.mem_flush_list(self.pdf_obj_list);
            self.mem_flush_list(self.pdf_xform_list);
            self.mem_flush_list(self.pdf_ximage_list);
            if shipping_page {
                // §765 Flush PDF mark lists
                self.mem_flush_list(self.pdf_annot_list);
                self.mem_flush_list(self.pdf_link_list);
                self.mem_flush_list(self.pdf_dest_list);
                self.mem_flush_list(self.pdf_bead_list);
            }
        }
        // done:
        if shipping_page {
            self.synctex_teehs();
        } else {
            self.synctex_mrofxfdp();
        }
        if self.eTeX_ex() {
            // §1730 Check for LR anomalies at the end of |ship_out|
            if self.LR_problems > 0 {
                // §1713 Report LR problems
                self.print_ln();
                self.print_nl_s(b"\\endL or \\endR problem (");
                self.print_int(self.LR_problems / 10000);
                self.print_s(b" missing, ");
                self.print_int(self.LR_problems % 10000);
                self.print_s(b" extra");
                self.LR_problems = 0;
                self.print_char(b')' as i32);
                self.print_ln();
            }
            if self.LR_ptr != NULL || self.cur_dir != LEFT_TO_RIGHT {
                self.confusion(b"LR3");
            }
        }
        if self.tracing_output() <= 0 && shipping_page {
            self.print_char(b']' as i32);
        }
        self.dead_cycles = 0;
        self.update_terminal(); // progress report
        // §667 Flush the box from memory, showing statistics if requested
        if self.tracing_stats() > 1 {
            self.print_nl_s(b"Memory usage before: ");
            self.print_int(self.var_used);
            self.print_char(b'&' as i32);
            let d = self.web_dyn_used();
            self.print_int(d);
            self.print_char(b';' as i32);
        }
        self.flush_node_list(p);
        if self.tracing_stats() > 1 {
            self.print_s(b" after: ");
            self.print_int(self.var_used);
            self.print_char(b'&' as i32);
            let d = self.web_dyn_used();
            self.print_int(d);
            self.print_s(b"; still untouched: ");
            let u = self.web_untouched();
            self.print_int(u);
            self.print_ln();
        }
    }

    /// §753 Reset resource lists
    fn pdfship_reset_resource_lists(&mut self) {
        self.pdf_font_list = NULL;
        self.pdf_obj_list = NULL;
        self.pdf_xform_list = NULL;
        self.pdf_ximage_list = NULL;
        self.pdf_text_procset = false;
        self.pdf_image_procset = 0;
    }

    /// §772
    pub fn pdf_write_obj(&mut self, n: i32) {
        let t = self.obj_obj_data(n);
        let s = self.tokens_to_string(t);
        // `\pdfrefobj` of an object only reserved so far: the oracle's `delete_toks` lands on a harmless word of `mem`, and here block 0 is not to be freed.
        if t != NULL {
            self.delete_token_ref(t);
        }
        self.set_obj_obj_data(n, NULL);
        if self.obj_obj_is_stream(n) > 0 {
            self.pdf_begin_dict(n, 0);
            if self.obj_obj_stream_attr(n) != NULL {
                let a = self.obj_obj_stream_attr(n);
                self.pdf_print_toks_ln(a);
                self.delete_token_ref(a);
                self.set_obj_obj_stream_attr(n, NULL);
            }
            self.pdf_begin_stream();
        } else {
            self.pdf_begin_obj(n, 1);
        }
        if self.obj_obj_is_file(n) > 0 {
            self.cur_name = s;
            self.cur_area = S_EMPTY;
            self.cur_ext = S_EMPTY;
            self.pack_cur_name();
            if !self.tex_b_openin(OBJ_FILE) {
                self.print_nl_s(b"! ");
                self.print(s);
                self.print_s(b" not found.");
                self.pdf_error(b"ext5", b"cannot open file for embedding");
            }
            self.print_s(b"<<");
            self.print(s);
            if !self.feof(OBJ_FILE) {
                // at least one byte available
                while !self.feof(OBJ_FILE) {
                    let c = self.getc(OBJ_FILE);
                    self.pdf_out(c);
                }
                if self.obj_obj_is_stream(n) == 0 && self.pdf_ptr > 0 {
                    let last = if self.pdf_buf_is_os { self.pdf_os_buf.at(self.pdf_ptr - 1) } else { self.pdf_op_buf.at(self.pdf_ptr - 1) };
                    if last != 10 {
                        self.pdf_out(10);
                    }
                }
            }
            self.print_s(b">>");
            self.b_close(OBJ_FILE);
        } else if self.obj_obj_is_stream(n) > 0 {
            self.pdf_print(s);
        } else {
            self.pdf_print_ln(s);
        }
        if self.obj_obj_is_stream(n) > 0 {
            self.pdf_end_stream();
        } else {
            self.pdf_end_obj();
        }
        self.flush_str(s);
    }

    /// §772
    pub fn flush_whatsit_node(&mut self, p: i32, s: i32) {
        self.set_ty(p, WHATSIT_NODE);
        self.set_subtype(p, s);
        if self.link(p) != NULL {
            self.pdf_error(b"flush_whatsit_node", b"link(p) is not null");
        }
        self.flush_node_list(p);
    }

    /// §778
    pub fn pdf_write_image(&mut self, n: i32) {
        self.pdf_begin_dict(n, 0);
        if self.obj_ximage_attr(n) != NULL {
            let a = self.obj_ximage_attr(n);
            self.pdf_print_toks_ln(a);
            self.delete_token_ref(a);
            self.set_obj_ximage_attr(n, NULL);
        }
        if self.fixed_pdf_draftmode == 0 {
            self.write_image(self.obj_ximage_data(n));
        }
        self.delete_image(self.obj_ximage_data(n));
    }

    /// §785
    pub fn pdf_print_rect_spec(&mut self, r: i32) {
        self.pdf_print_mag_bp(self.pdf_x(self.pdf_left(r)));
        self.pdf_out(b' ' as i32);
        self.pdf_print_mag_bp(self.pdf_y(self.pdf_bottom(r)));
        self.pdf_out(b' ' as i32);
        self.pdf_print_mag_bp(self.pdf_x(self.pdf_right(r)));
        self.pdf_out(b' ' as i32);
        self.pdf_print_mag_bp(self.pdf_y(self.pdf_top(r)));
    }

    /// §791
    pub fn ship_out(&mut self, p: i32) {
        crate::prof_phase!("first shipout");
        self.fix_pdfoutput();
        if self.pdf_output() > 0 {
            self.pdf_ship_out(p, true);
        } else {
            self.dvi_ship_out(p);
        }
        crate::prof_phase!("first shipout done");
        crate::prof_phase_last!("end of last page");
        // About a millisecond, for 1% or so of what is left: past the length of a paper, it pays.
        if self.total_pages == 64 {
            self.image_collapse_hot();
        }
    }

    /// §793's `get_next_char`: the character at `j`, undoing the escapes of a PDF string, and the next `j`.
    fn pdfship_get_next_char(&self, j: i32, e: i32) -> (i32, i32) {
        let mut j = j;
        let mut c = self.str_pool.at(j);
        j += 1;
        if c == 92 && j < e {
            c = self.str_pool.at(j);
            j += 1;
            if (48..=55).contains(&c) {
                c = c - 48;
                if j < e && (48..=55).contains(&self.str_pool.at(j)) {
                    c = 8 * c + self.str_pool.at(j) - 48;
                    j += 1;
                    if j < e && (48..=55).contains(&self.str_pool.at(j)) && c < 32 {
                        c = 8 * c + self.str_pool.at(j) - 48;
                        j += 1;
                    }
                }
            } else {
                match c {
                    98 => c = 8,   // b: backspace
                    102 => c = 12, // f: form feed
                    110 => c = 10, // n: line feed
                    114 => c = 13, // r: carriage return
                    116 => c = 9,  // t: horizontal tab
                    _ => {}        // nothing to do for \, (, )
                }
            }
        }
        (c, j)
    }

    /// §793
    pub fn str_less_str(&mut self, s1: i32, s2: i32) -> bool {
        let mut j1 = self.str_start[s1];
        let mut j2 = self.str_start[s2];
        let e1 = j1 + self.length(s1);
        let e2 = j2 + self.length(s2);
        while j1 < e1 && j2 < e2 {
            let (c1, n1) = self.pdfship_get_next_char(j1, e1); // get next character of first string
            let (c2, n2) = self.pdfship_get_next_char(j2, e2); // get next character of second string
            j1 = n1;
            j2 = n2;
            if c1 < c2 {
                return true;
            } else if c1 > c2 {
                return false;
            }
        }
        j1 >= e1 && j2 < e2 // compare string lengths
    }

    /// §793
    pub fn sort_dest_names(&mut self, l: i32, r: i32) {
        let mut i = l;
        let mut j = r;
        let s = self.dest_names[(l + r) / 2].objname;
        loop {
            while self.str_less_str(self.dest_names[i].objname, s) {
                i += 1;
            }
            while self.str_less_str(s, self.dest_names[j].objname) {
                j -= 1;
            }
            if i <= j {
                let e = self.dest_names[i];
                self.dest_names[i] = self.dest_names[j];
                self.dest_names[j] = e;
                i += 1;
                j -= 1;
            }
            if i > j {
                break;
            }
        }
        if l < j {
            self.sort_dest_names(l, j);
        }
        if i < r {
            self.sort_dest_names(i, r);
        }
    }

    /// §795
    pub fn pdf_fix_dest(&mut self, k: i32) {
        if self.obj_dest_ptr(k) != NULL {
            return;
        }
        self.pdf_warning(b"dest", b"", true, false);
        if self.obj_info(k) < 0 {
            self.print_s(b"name{");
            self.print(-self.obj_info(k));
            self.print_s(b"}");
        } else {
            self.print_s(b"num");
            self.print_int(self.obj_info(k));
        }
        self.print_s(b" has been referenced but does not exist, replaced by a fixed one");
        self.print_ln();
        self.print_ln();
        self.pdf_begin_obj(k, 1);
        self.pdf_out(b'[' as i32);
        self.pdf_print_int(self.head_tab[OBJ_TYPE_PAGE]);
        self.pdf_print_ln_s(b" 0 R /Fit]");
        self.pdf_end_obj();
    }

    /// §797
    pub fn pdf_fix_struct_dest(&mut self, k: i32) {
        if self.obj_dest_ptr(k) != NULL {
            return;
        }
        self.pdf_warning(b"structure dest", b"", false, false);
        if self.obj_info(k) < 0 {
            self.print_s(b"name{");
            self.print(-self.obj_info(k));
            self.print_s(b"}");
        } else {
            self.print_s(b"num");
            self.print_int(self.obj_info(k));
        }
        self.print_s(b" has been referenced but does not exist");
        self.print_ln();
        self.print_ln();
    }

    /// §807
    pub fn pdf_print_info(&mut self) {
        self.pdf_new_dict(OBJ_TYPE_OTHERS, 0, 3); // keep Info readable unless explicitly forced
        let mut s = 0;
        let mut creator_given = false;
        let mut producer_given = false;
        let mut creationdate_given = false;
        let mut moddate_given = false;
        let mut trapped_given = false;
        if self.pdf_info_toks != NULL {
            s = self.tokens_to_string(self.pdf_info_toks);
            creator_given = self.substr_of_str(b"/Creator", s);
            producer_given = self.substr_of_str(b"/Producer", s);
            creationdate_given = self.substr_of_str(b"/CreationDate", s);
            moddate_given = self.substr_of_str(b"/ModDate", s);
            trapped_given = self.substr_of_str(b"/Trapped", s);
        }
        if !producer_given {
            // §808 Print the Producer key
            self.pdf_print_s(b"/Producer (pdfTeX-");
            self.pdf_print_int(PDFTEX_VERSION / 100);
            self.pdf_out(b'.' as i32);
            self.pdf_print_int(PDFTEX_VERSION % 100);
            self.pdf_out(b'.' as i32);
            self.pdf_print_s(PDFTEX_REVISION);
            self.pdf_print_ln_s(b")");
        }
        if self.pdf_info_toks != NULL {
            if self.length(s) > 0 {
                self.pdf_print_ln(s);
            }
            self.flush_str(s);
            self.delete_token_ref(self.pdf_info_toks);
            self.pdf_info_toks = NULL;
        }
        if !creator_given {
            // pdf_str_entry_ln("Creator", "TeX"), printed in its pieces: Appendix B
            self.pdf_out(b'/' as i32);
            self.pdf_print_s(b"Creator");
            self.pdf_out(b' ' as i32);
            self.pdf_print_ln_s(b"(TeX)");
        }
        if self.pdf_info_omit_date() == 0 {
            if !creationdate_given {
                // §809 Print the CreationDate key
                self.print_creation_date();
            }
            if !moddate_given {
                // §810 Print the ModDate key
                self.print_mod_date();
            }
        }
        if !trapped_given {
            self.pdf_print_ln_s(b"/Trapped /False");
        }
        if self.pdf_suppress_ptex_info() % 2 == 0 {
            if self.get_ptex_use_underscore() {
                self.pdf_str_entry_ln(b"PTEX_Fullbanner", self.pdftex_banner);
            } else {
                self.pdf_str_entry_ln(b"PTEX.Fullbanner", self.pdftex_banner);
            }
        }
        self.pdf_end_dict();
    }

    // ---- The warnings and failures of utils.c. ----

    /// `pdftex_warn`, without its optional " (file x)", which only a font or image reader sets.
    pub fn pdfship_warn(&mut self, msg: &[u8]) {
        self.print_ln();
        self.print_ln();
        self.print_s(b"pdfTeX warning: ");
        let name = rstex_kpse::invocation_name();
        self.print_s(&name);
        self.print_s(b": ");
        self.print_s(msg);
        self.print_ln();
    }

    /// `pdftex_fail`, likewise.
    pub fn pdfship_fail(&mut self, msg: &[u8]) -> ! {
        self.print_ln();
        self.print_s(b"!pdfTeX error: ");
        let name = rstex_kpse::invocation_name();
        self.print_s(&name);
        self.print_s(b": ");
        self.print_s(msg);
        self.print_ln();
        self.remove_pdffile();
        self.print_s(b" ==> Fatal error occurred, no output PDF file produced!");
        self.print_ln();
        self.uexit(1)
    }

    // ---- Colour stacks: utils.c. ----

    /// `init_colorstacks` and `colstacks_first_init`.
    fn pdfship_init_colorstacks(&mut self) {
        if self.v_pdfship.colstacks.len > 0 {
            return;
        }
        // String 0 is NULL, and string 1 is COLOR_DEFAULT.
        let mut chars = self.v_pdfship.col_chars;
        for &c in COLOR_DEFAULT {
            self.arena_push(&mut chars, c);
        }
        self.v_pdfship.col_chars = chars;
        let mut strs = self.v_pdfship.col_strs;
        self.arena_push(&mut strs, ColStr { start: 0, len: 0, next: 0 });
        self.arena_push(&mut strs, ColStr { start: 0, len: COLOR_DEFAULT.len() as i32, next: 0 });
        self.v_pdfship.col_strs = strs;
        self.v_pdfship.col_hash[pdfship_col_hash(COLOR_DEFAULT.iter().copied())] = 1;
        self.v_pdfship.col_default = 1;
        let mut stacks = self.v_pdfship.colstacks;
        let entry = ColStack {
            page_stack: PodVec::EMPTY,
            form_stack: PodVec::EMPTY,
            page_current: 1,
            form_current: 1,
            form_init: 1,
            literal_mode: DIRECT_ALWAYS,
            page_start: true,
        };
        self.arena_push(&mut stacks, entry);
        self.v_pdfship.colstacks = stacks;
    }

    /// `xstrdup(makecstring(s))`: the number of the colour string with the text of string `s` up to any NUL.
    fn pdfship_col_intern(&mut self, s: i32) -> i32 {
        let b = self.str_start[s];
        let mut n = 0;
        while n < self.length(s) && self.str_pool.at(b + n) != 0 {
            n += 1;
        }
        if n == 0 {
            return 0;
        }
        let bytes = self.str_pool.as_slice();
        let h = pdfship_col_hash(bytes[b as usize..(b + n) as usize].iter().copied());
        let mut k = self.v_pdfship.col_hash[h];
        while k != 0 {
            let e = self.v_pdfship.col_strs.get(&mut self.arena, k);
            if e.len == n {
                let mut i = 0;
                while i < n {
                    let c = self.v_pdfship.col_chars.get(&mut self.arena, e.start + i);
                    if c as i32 != self.str_pool.at(b + i) {
                        break;
                    }
                    i += 1;
                }
                if i == n {
                    return k;
                }
            }
            k = e.next;
        }
        let mut chars = self.v_pdfship.col_chars;
        let start = chars.len;
        for i in 0..n {
            let c = self.str_pool.at(b + i) as u8;
            self.arena_push(&mut chars, c);
        }
        self.v_pdfship.col_chars = chars;
        let mut strs = self.v_pdfship.col_strs;
        let k = strs.len;
        let next = self.v_pdfship.col_hash[h];
        self.arena_push(&mut strs, ColStr { start, len: n, next });
        self.v_pdfship.col_strs = strs;
        self.v_pdfship.col_hash[h] = k;
        k
    }

    /// `put_cstring_on_strpool(poolptr, str)`.
    fn pdfship_put_col_str(&mut self, k: i32) {
        if k == 0 {
            return;
        }
        let e = self.v_pdfship.col_strs.get(&mut self.arena, k);
        let start = self.pool_ptr;
        self.pool_ptr = start + e.len;
        if self.pool_ptr >= self.pool_size {
            self.pool_ptr = self.pool_size;
            return; // error by str_toks that calls str_room(1)
        }
        for i in 0..e.len {
            let c = self.v_pdfship.col_chars.get(&mut self.arena, e.start + i);
            self.str_pool.put(start + i, c as i32);
        }
    }

    /// External: utils.c
    pub fn colorstackused(&mut self) -> i32 {
        self.pdfship_init_colorstacks();
        self.v_pdfship.colstacks.len
    }

    /// `newcolorstack` of utils.c, which `buildtoks.rs` calls: the state is here.
    pub fn pdfship_new_colorstack(&mut self, s: i32, literal_mode: i32, page_start: bool) -> i32 {
        self.pdfship_init_colorstacks();
        if self.v_pdfship.colstacks.len == MAX_COLORSTACKS {
            return -1;
        }
        let k = self.pdfship_col_intern(s);
        let entry = ColStack {
            page_stack: PodVec::EMPTY,
            form_stack: PodVec::EMPTY,
            page_current: k,
            form_current: k,
            form_init: k,
            literal_mode,
            page_start,
        };
        let mut stacks = self.v_pdfship.colstacks;
        let colstack_num = stacks.len;
        self.arena_push(&mut stacks, entry);
        self.v_pdfship.colstacks = stacks;
        colstack_num
    }

    /// External: utils.c
    pub fn colorstackset(&mut self, colstack_no: i32, s: i32) -> i32 {
        let mut e = self.v_pdfship.colstacks.get(&mut self.arena, colstack_no);
        let k = self.pdfship_col_intern(s);
        if self.v_pdfship.page_mode {
            e.page_current = k;
        } else {
            e.form_current = k;
        }
        self.v_pdfship.colstacks.set(&mut self.arena, colstack_no, e);
        e.literal_mode
    }

    /// External: utils.c
    pub fn colorstackpush(&mut self, colstack_no: i32, s: i32) -> i32 {
        let mut e = self.v_pdfship.colstacks.get(&mut self.arena, colstack_no);
        let k = self.pdfship_col_intern(s);
        if self.v_pdfship.page_mode {
            let mut v = e.page_stack;
            self.arena_push(&mut v, e.page_current);
            e.page_stack = v;
            e.page_current = k;
        } else {
            let mut v = e.form_stack;
            self.arena_push(&mut v, e.form_current);
            e.form_stack = v;
            e.form_current = k;
        }
        self.v_pdfship.colstacks.set(&mut self.arena, colstack_no, e);
        e.literal_mode
    }

    /// External: utils.c
    pub fn colorstackpop(&mut self, colstack_no: i32) -> i32 {
        let mut e = self.v_pdfship.colstacks.get(&mut self.arena, colstack_no);
        let k;
        if self.v_pdfship.page_mode {
            if e.page_stack.len == 0 {
                self.pdfship_warn(format!("pop empty color page stack {}", colstack_no as u32).as_bytes());
                return e.literal_mode;
            }
            k = e.page_stack.get(&mut self.arena, e.page_stack.len - 1);
            e.page_stack.len -= 1;
            e.page_current = k;
        } else {
            if e.form_stack.len == 0 {
                self.pdfship_warn(format!("pop empty color form stack {}", colstack_no as u32).as_bytes());
                return e.literal_mode;
            }
            k = e.form_stack.get(&mut self.arena, e.form_stack.len - 1);
            e.form_stack.len -= 1;
            e.form_current = k;
        }
        self.v_pdfship.colstacks.set(&mut self.arena, colstack_no, e);
        self.pdfship_put_col_str(k);
        e.literal_mode
    }

    /// External: utils.c
    pub fn colorstackcurrent(&mut self, colstack_no: i32) -> i32 {
        let e = self.v_pdfship.colstacks.get(&mut self.arena, colstack_no);
        self.pdfship_put_col_str(if self.v_pdfship.page_mode { e.page_current } else { e.form_current });
        e.literal_mode
    }

    /// `colorstackpagestart` of utils.c.  Its one caller has just set `page_mode`, so in the C, too, it stops at once.
    fn pdfship_colorstack_page_start(&mut self) {
        if self.v_pdfship.page_mode {
            return; // see procedure pdf_out_colorstack_startpage
        }
        for i in 0..self.v_pdfship.colstacks.len {
            let mut e = self.v_pdfship.colstacks.get(&mut self.arena, i);
            e.form_stack.len = 0;
            e.form_current = e.form_init;
            self.v_pdfship.colstacks.set(&mut self.arena, i, e);
        }
    }

    /// External: utils.c
    pub fn colorstackskippagestart(&mut self, colstack_no: i32) -> i32 {
        let e = self.v_pdfship.colstacks.get(&mut self.arena, colstack_no);
        if !e.page_start {
            return 1;
        }
        if e.page_current == 0 {
            return 0;
        }
        if e.page_current == self.v_pdfship.col_default {
            return 2;
        }
        0
    }

    // ---- The stacks of \pdfsave and \pdfsetmatrix: utils.c. ----

    /// External: utils.c
    pub fn checkpdfsave(&mut self, cur_h: i32, cur_v: i32) {
        let x = PosEntry { pos_h: cur_h, pos_v: cur_v, matrix_stack: if self.v_pdfship.page_mode { self.v_pdfship.matrix_stack.len } else { 0 } };
        let mut v = self.v_pdfship.pos_stack;
        self.arena_push(&mut v, x);
        self.v_pdfship.pos_stack = v;
    }

    /// External: utils.c
    pub fn checkpdfrestore(&mut self, cur_h: i32, cur_v: i32) {
        if self.v_pdfship.pos_stack.len == 0 {
            self.pdfship_warn(b"\\pdfrestore: missing \\pdfsave");
            return;
        }
        let x = self.v_pdfship.pos_stack.get(&mut self.arena, self.v_pdfship.pos_stack.len - 1);
        self.v_pdfship.pos_stack.len -= 1;
        let diff_h = cur_h - x.pos_h;
        let diff_v = cur_v - x.pos_v;
        if diff_h != 0 || diff_v != 0 {
            self.pdfship_warn(format!("Misplaced \\pdfrestore by ({}sp, {}sp)", diff_h as u32, diff_v as u32).as_bytes());
        }
        if self.v_pdfship.page_mode {
            self.v_pdfship.matrix_stack.len = x.matrix_stack;
        }
    }

    /// External: utils.c
    pub fn pdfshipoutbegin(&mut self, shipping_page: bool) {
        self.v_pdfship.pos_stack.len = 0; // start with empty stack
        self.v_pdfship.page_mode = shipping_page;
        if shipping_page {
            self.pdfship_colorstack_page_start();
        }
    }

    /// External: utils.c
    pub fn pdfshipoutend(&mut self, shipping_page: bool) {
        if self.v_pdfship.pos_stack.len > 0 {
            let which = if shipping_page { "page" } else { "form" };
            let msg = format!("{} unmatched \\pdfsave after {} shipout", self.v_pdfship.pos_stack.len as u32, which);
            self.pdfship_fail(msg.as_bytes());
        }
    }

    /// External: utils.c.  The argument of `\pdfsetmatrix` starts at `str_pool[in_]` and ends at a NUL.
    pub fn pdfsetmatrix(&mut self, in_: i32, cur_h: i32, cur_v: i32) -> i32 {
        if self.v_pdfship.page_mode {
            let text = pdfship_c_str(&self.str_pool.as_slice()[in_ as usize..]);
            let Some([a, b, c, d]) = pdfship_scan_matrix(text) else {
                return 0; // failure
            };
            // calculate this transformation matrix
            let e = cur_h as f64 * (1.0 - a) - cur_v as f64 * c;
            let f = cur_v as f64 * (1.0 - d) - cur_h as f64 * b;
            let v = self.v_pdfship.matrix_stack;
            let z = if v.len > 0 {
                let y = v.get(&mut self.arena, v.len - 1);
                MatrixEntry {
                    a: a * y.a + b * y.c,
                    b: a * y.b + b * y.d,
                    c: c * y.a + d * y.c,
                    d: c * y.b + d * y.d,
                    e: e * y.a + f * y.c + y.e,
                    f: e * y.b + f * y.d + y.f,
                }
            } else {
                MatrixEntry { a, b, c, d, e, f }
            };
            let mut v = v;
            self.arena_push(&mut v, z);
            self.v_pdfship.matrix_stack = v;
        }
        1 // success
    }

    /// The matrix `do_matrixtransform` of utils.c uses, when `page_mode && matrix_stack_used > 0`, the test of
    /// `matrixtransformrect`, which is in `ext.rs`.
    pub fn pdfship_matrix_top(&mut self) -> Option<MatrixEntry> {
        let v = self.v_pdfship.matrix_stack;
        if self.v_pdfship.page_mode && v.len > 0 {
            Some(v.get(&mut self.arena, v.len - 1))
        } else {
            None
        }
    }

    // ---- The job id, /ID, dates. ----

    /// `initstarttime` and `start_time_str` of texmfmp.c, made once.  With `SOURCE_DATE_EPOCH` the time is in UTC.
    pub fn pdfship_start_time_str(&mut self) -> ([u8; 30], usize) {
        if self.v_pdfship.start_time_len == 0 {
            let (t, source_date_epoch_set) = self.host.world.init_start_time();
            let s = pdfship_make_pdf_time(t, source_date_epoch_set);
            for (k, &c) in s.iter().enumerate() {
                self.v_pdfship.start_time_str[k as i32] = c;
            }
            self.v_pdfship.start_time_len = s.len() as i32;
        }
        let mut out = [0u8; 30];
        let n = self.v_pdfship.start_time_len as usize;
        out[..n].copy_from_slice(&self.v_pdfship.start_time_str.as_slice()[..n]);
        (out, n)
    }

    /// External: utils.c printID, /ID
    pub fn print_ID(&mut self, filename: i32) {
        let (time, n) = self.pdfship_start_time_str();
        let name = self.make_cstring(filename);
        let id = pdfship_id_hex(&[&time[..n], pdfship_c_str(&name)]);
        self.pdfship_print_id(&id);
    }

    /// External: utils.c printIDalt
    pub fn print_ID_alt(&mut self, toks: i32) {
        let s = self.tokens_to_string(toks);
        let text = self.make_cstring(s);
        self.flush_str(s);
        let text = pdfship_c_str(&text);
        if text.is_empty() {
            return;
        }
        let id = pdfship_id_hex(&[text]);
        self.pdfship_print_id(&id);
    }

    /// `pdf_printf("/ID [<%s> <%s>]", id, id)`.
    fn pdfship_print_id(&mut self, id: &[u8]) {
        self.pdf_print_s(b"/ID [<");
        self.pdf_print_s(id);
        self.pdf_print_s(b"> <");
        self.pdf_print_s(id);
        self.pdf_print_s(b">]");
    }

    /// External: utils.c setjobid
    pub fn set_job_id(&mut self, year: i32, month: i32, day: i32, time: i32) {
        if self.v_pdfship.job_id_string.len > 0 {
            return;
        }
        let name = self.make_cstring(self.job_name);
        let format = self.make_cstring(self.format_ident);
        let s = pdfship_job_id(year, month, day, time, pdfship_c_str(&name), pdfship_c_str(&format), VERSION_STRING);
        let mut v = self.v_pdfship.job_id_string;
        for &c in &s {
            self.arena_push(&mut v, c);
        }
        self.v_pdfship.job_id_string = v;
    }

    /// External: utils.c
    pub fn get_resname_prefix(&mut self) -> i32 {
        let v = self.v_pdfship.job_id_string;
        let mut job_id = Vec::with_capacity(v.len as usize);
        for i in 0..v.len {
            job_id.push(v.get(&mut self.arena, i));
        }
        let prefix = pdfship_resname_prefix(&job_id);
        self.str_room(prefix.len() as i32);
        for &c in &prefix {
            self.append_char(c as i32);
        }
        self.make_string()
    }

    /// External: utils.c makecstring.  The bytes of string `s`; the C adds a NUL, so `strlen` of it stops at any NUL inside.
    pub fn make_cstring(&mut self, s: i32) -> Vec<u8> {
        let b = self.str_start[s] as usize;
        let n = self.length(s) as usize;
        self.str_pool.as_slice()[b..b + n].to_vec()
    }

    /// External: `kpathsea_init_prog`.  `mode` empty stands for `nil`: an empty `\pdfpkmode` only changes `MAKETEX_MODE`, for PK fonts, which are dropped.
    pub fn kpse_init_prog(&mut self, prefix: &[u8], dpi: i32, mode: &[u8], _fallback: &[u8]) {
        rstex_kpse::init_prog(prefix, dpi as u32, if mode.is_empty() { None } else { Some(mode) });
    }

    /// External: mktex is dropped: no-op
    pub fn kpse_set_program_enabled(&mut self, fmt: i32, value: bool, level: i32) {
        rstex_kpse::set_program_enabled(fmt, value, level);
    }

    /// External: utils.c: frees C-side tables; a no-op here
    pub fn libpdffinish(&mut self) {}

    /// External: utils.c: on fatal error, and for dangling objects when no page came out.  The test leaves out the C's
    /// `!kpathsea_debug`: `-kpathsea-debug` is accepted and ignored here.
    pub fn remove_pdffile(&mut self) {
        if self.output_file_name != 0 && self.fixed_pdf_draftmode == 0 {
            self.pdf_close_and_remove();
        }
    }

    /// External: utils.c
    pub fn garbage_warning(&mut self) {
        self.pdfship_warn(b"dangling objects discarded, no output file produced.");
        self.remove_pdffile();
    }

    /// External: texmfmp.c
    pub fn print_creation_date(&mut self) {
        let (time, n) = self.pdfship_start_time_str();
        self.pdf_print_s(b"/CreationDate (");
        self.pdf_print_s(&time[..n]);
        self.pdf_print_s(b")\n");
    }

    /// External: texmfmp.c
    pub fn print_mod_date(&mut self) {
        let (time, n) = self.pdfship_start_time_str();
        self.pdf_print_s(b"/ModDate (");
        self.pdf_print_s(&time[..n]);
        self.pdf_print_s(b")\n");
    }

    /// External: `flushjbig2page0objects`, writejbig2.c.
    pub fn flush_jbig2_page0_objects(&mut self) {
        self.ext_flush_jbig2_page0();
    }
}

/// The chain of `col_hash` for these bytes.
fn pdfship_col_hash(bytes: impl Iterator<Item = u8>) -> i32 {
    let mut h = 2166136261u32;
    for c in bytes {
        h = (h ^ c as u32).wrapping_mul(16777619);
    }
    (h % 64) as i32
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::toks;

    /// The token machine, with room for strings.
    fn machine() -> &'static mut Tex {
        let t = toks::testkit::machine();
        t.max_strings = 10000;
        t
    }

    /// A string in the pool, through the real `make_string`.
    fn string(t: &mut Tex, s: &[u8]) -> i32 {
        for &c in s {
            t.append_char(c as i32);
        }
        t.make_string()
    }

    /// The oracle's table: its lines are tab separated, kind first.  A tab in a field is written `<TAB>`.
    fn table(kind: &str) -> Vec<Vec<String>> {
        include_str!("../testdata/pdfship_oracle.tsv")
            .lines()
            .map(|l| l.split('\t').map(|f| f.replace("<TAB>", "\t")).collect::<Vec<String>>())
            .filter(|f| f[0] == kind)
            .collect()
    }

    #[test]
    fn matrix_formats_against_the_oracle() {
        let rows = table("matrix");
        assert!(rows.len() > 20);
        for r in rows {
            let got = pdfship_scan_matrix(r[1].as_bytes()).is_some();
            assert_eq!(got, r[2] == "1", "\\pdfsetmatrix{{{}}}", r[1]);
        }
    }

    #[test]
    fn matrix_values_as_strtod_reads_them() {
        let m = |s: &str| pdfship_scan_matrix(s.as_bytes());
        assert_eq!(m("1 2 3 4"), Some([1.0, 2.0, 3.0, 4.0]));
        assert_eq!(m(" +1.5 -.5 1e2 2E-1 "), Some([1.5, -0.5, 100.0, 0.2]));
        assert_eq!(m("0x10 0x.8 0x1p3 0X1.8P1"), Some([16.0, 0.5, 8.0, 3.0]));
        assert_eq!(m("1e 2 3 4"), None);
        assert_eq!(m("1e+ 2 3 4"), None);
        let i = m("inf -INF Infinity 0").unwrap();
        assert!(i[0] == f64::INFINITY && i[1] == f64::NEG_INFINITY && i[2] == f64::INFINITY);
        assert!(m("nan 0 0 0").unwrap()[0].is_nan());
        assert_eq!(m("1 2 3"), None);
        assert_eq!(m("1 2 3 4 5"), None);
        assert_eq!(m("a 2 3 4"), None);
        assert_eq!(m("infx 2 3 4"), None);
        assert_eq!(m("0x 2 3 4"), None);
    }

    #[test]
    fn dest_name_order_against_the_oracle() {
        // The oracle writes the names of dests in the order of its sort.
        let rows = table("sorted");
        assert!(rows.len() > 10);
        let t = machine();
        let mut names: Vec<i32> = Vec::new();
        for r in &rows {
            names.push(string(t, r[1].as_bytes()));
        }
        for (i, &a) in names.iter().enumerate() {
            for (j, &b) in names.iter().enumerate() {
                if i < j {
                    assert!(!t.str_less_str(b, a), "{} before {}", rows[j][1], rows[i][1]);
                }
            }
        }
    }

    #[test]
    fn dest_names_sort() {
        let rows = table("sorted");
        let t = machine();
        let mut names: Vec<i32> = Vec::new();
        for r in rows.iter().rev() {
            names.push(string(t, r[1].as_bytes()));
        }
        for (i, &s) in names.iter().enumerate() {
            t.dest_names[i as i32].objname = s;
            t.dest_names[i as i32].objnum = i as i32;
        }
        t.sort_dest_names(0, names.len() as i32 - 1);
        let mut prev = t.dest_names[0].objname;
        for i in 1..names.len() as i32 {
            let s = t.dest_names[i].objname;
            assert!(!t.str_less_str(s, prev));
            prev = s;
        }
    }

    #[test]
    fn get_next_char_undoes_the_escapes() {
        let t = machine();
        for (text, want) in [
            (&b"a"[..], &[97][..]),
            (b"\\b\\f\\n\\r\\t", &[8, 12, 10, 13, 9]),
            (b"\\(\\)\\\\\\z", &[40, 41, 92, 122]),
            (b"\\101\\1012\\7\\77\\777", &[65, 65, 50, 7, 63, 63, 55]),
            (b"\\477", &[39, 55]),
            (b"a\\", &[97, 92]),
        ] {
            let s = string(t, text);
            let (mut j, e) = (t.str_start[s], t.str_start[s + 1]);
            let mut got = Vec::new();
            while j < e {
                let (c, n) = t.pdfship_get_next_char(j, e);
                got.push(c);
                j = n;
            }
            assert_eq!(got, want, "{}", String::from_utf8_lossy(text));
        }
    }

    #[test]
    fn substr_of_str_misses_the_very_end() {
        // The oracle's behaviour: see pagebox.tex under tests/integration/pdfship.
        let t = machine();
        for (hay, needle, want) in [
            (&b"/MediaBox"[..], &b"/MediaBox"[..], false),
            (b"/MediaBox ", b"/MediaBox", true),
            (b" /MediaBox", b"/MediaBox", false),
            (b"x/MediaBox y", b"/MediaBox", true),
            (b"/Media", b"/MediaBox", false),
            (b"", b"/MediaBox", false),
            (b"/mediabox ", b"/MediaBox", false),
        ] {
            let s = string(t, hay);
            assert_eq!(t.substr_of_str(needle, s), want, "{}", String::from_utf8_lossy(hay));
        }
    }

    #[test]
    fn pdf_time_against_the_oracle() {
        let rows = table("time");
        assert!(rows.len() >= 4);
        for r in rows {
            let t: i64 = r[1].parse().unwrap();
            assert_eq!(String::from_utf8(pdfship_make_pdf_time(t, true)).unwrap(), r[2]);
        }
    }

    #[test]
    fn pdf_time_in_local_time_says_its_offset() {
        // Whatever zone this machine is in: what makepdftime works out must be what the library says.
        for t in [0, 1000000000, 1700000000, 1719792000] {
            let s = String::from_utf8(pdfship_make_pdf_time(t, false)).unwrap();
            let off = platform::broken_time(t, false).gmtoff / 60;
            let want = if off == 0 { "Z".to_string() } else { format!("{:+03}'{:02}'", off / 60, (off % 60).abs()) };
            assert!(s.ends_with(&want), "{} wanted {}", s, want);
        }
    }

    #[test]
    fn trailer_id_against_the_oracle() {
        let rows = table("id");
        assert!(rows.len() >= 2);
        for r in rows {
            let id = pdfship_id_hex(&[r[1].as_bytes(), r[2].as_bytes()]);
            assert_eq!(String::from_utf8(id).unwrap(), r[3]);
        }
        let rows = table("idalt");
        assert!(rows.len() >= 2);
        for r in rows {
            let id = pdfship_id_hex(&[r[1].as_bytes()]);
            assert_eq!(String::from_utf8(id).unwrap(), r[2]);
        }
    }

    #[test]
    fn resname_prefix_against_the_oracle() {
        let rows = table("resname");
        assert!(rows.len() >= 3);
        for r in rows {
            let n = |k: usize| r[k].parse::<i32>().unwrap();
            let job = pdfship_job_id(n(1), n(2), n(3), n(4), r[5].as_bytes(), b" (INITEX)", b" (TeX Live 2026/Arch Linux)");
            assert_eq!(String::from_utf8(pdfship_resname_prefix(&job).to_vec()).unwrap(), r[6], "{:?}", r);
        }
    }

    #[test]
    fn job_id_numbers_keep_c_precision() {
        let j = pdfship_job_id(-5, 1, 22, 125, b"n", b"f", VERSION_STRING);
        assert!(j.starts_with(b"-0005/01/22 02:05 n f This is pdfTeX, Version"), "{}", String::from_utf8_lossy(&j));
    }

    #[test]
    fn colour_stacks_follow_utils_c() {
        let t = machine();
        t.v_pdfship.page_mode = true;
        assert_eq!(t.colorstackused(), 1);
        // Stack 0 starts at COLOR_DEFAULT, and the page does not repeat that.
        assert_eq!(t.colorstackskippagestart(0), 2);
        let a = string(t, b"1 0 0 rg");
        assert_eq!(t.pdfship_new_colorstack(a, SET_ORIGIN, false), 1);
        assert_eq!(t.colorstackused(), 2);
        assert_eq!(t.colorstackskippagestart(1), 1);
        let e = string(t, b"");
        assert_eq!(t.pdfship_new_colorstack(e, DIRECT_PAGE, true), 2);
        assert_eq!(t.colorstackskippagestart(2), 0);

        let current = |t: &mut Tex, n: i32| {
            let b = t.pool_ptr;
            let m = t.colorstackcurrent(n);
            let s: Vec<u8> = t.str_pool.as_slice()[b as usize..t.pool_ptr as usize].to_vec();
            t.pool_ptr = b;
            (m, s)
        };
        assert_eq!(current(t, 1), (SET_ORIGIN, b"1 0 0 rg".to_vec()));
        assert_eq!(current(t, 2), (DIRECT_PAGE, Vec::new()));

        // Page and form have separate currents and stacks.
        let b = string(t, b"0 1 0 rg");
        assert_eq!(t.colorstackpush(1, b), SET_ORIGIN);
        assert_eq!(current(t, 1).1, b"0 1 0 rg");
        t.v_pdfship.page_mode = false;
        assert_eq!(current(t, 1).1, b"1 0 0 rg");
        let c = string(t, b"x");
        t.colorstackset(1, c);
        assert_eq!(current(t, 1).1, b"x");
        t.v_pdfship.page_mode = true;
        assert_eq!(current(t, 1).1, b"0 1 0 rg");

        // Pop gives the string to the caller on the pool.
        let before = t.pool_ptr;
        assert_eq!(t.colorstackpop(1), SET_ORIGIN);
        assert_eq!(&t.str_pool.as_slice()[before as usize..t.pool_ptr as usize], b"1 0 0 rg");
        t.pool_ptr = before;
        assert_eq!(current(t, 1).1, b"1 0 0 rg");

        // Pop on empty leaves the pool, and warns.
        let before = t.pool_ptr;
        assert_eq!(t.colorstackpop(1), SET_ORIGIN);
        assert_eq!(t.pool_ptr, before);

        // A NUL ends the C string; the empty string is NULL.
        let d = string(t, b"ab\0cd");
        t.colorstackset(1, d);
        assert_eq!(current(t, 1).1, b"ab");
        let d = string(t, b"\0cd");
        t.colorstackset(1, d);
        assert_eq!(current(t, 1).1, b"");
        // Setting back to the default on a page-start stack is what makes it skip.
        let d = string(t, b"0 g 0 G");
        t.colorstackset(0, d);
        assert_eq!(t.colorstackskippagestart(0), 2);
        let d = string(t, b"0 g");
        t.colorstackset(0, d);
        assert_eq!(t.colorstackskippagestart(0), 0);
    }

    #[test]
    fn colour_strings_are_not_stored_twice() {
        let t = machine();
        t.v_pdfship.page_mode = true;
        t.colorstackused();
        let a = string(t, b"1 0 0 rg");
        let top = t.arena_top;
        for _ in 0..1000 {
            t.colorstackpush(0, a);
        }
        // The stack grew, the strings did not: 8 bytes of text is one word.
        for _ in 0..1000 {
            let b = t.pool_ptr;
            t.colorstackpop(0);
            t.pool_ptr = b;
        }
        let used = t.arena_top - top;
        assert!(used < 2000, "{}", used);
        assert_eq!(t.v_pdfship.col_strs.len, 3);
    }

    #[test]
    fn pop_past_many_pushes_returns_in_order() {
        let t = machine();
        t.v_pdfship.page_mode = true;
        t.colorstackused();
        for k in 0..100 {
            let s = string(t, format!("{} g", k).as_bytes());
            t.colorstackpush(0, s);
        }
        for k in (0..99).rev() {
            let b = t.pool_ptr;
            t.colorstackpop(0);
            let got = t.str_pool.as_slice()[b as usize..t.pool_ptr as usize].to_vec();
            t.pool_ptr = b;
            assert_eq!(got, format!("{} g", k).as_bytes());
        }
        let b = t.pool_ptr;
        t.colorstackpop(0);
        assert_eq!(&t.str_pool.as_slice()[b as usize..t.pool_ptr as usize], b"0 g 0 G");
    }

    #[test]
    fn pdfsave_stack_keeps_the_matrix_level() {
        let t = machine();
        t.pdfshipoutbegin(true);
        let m = |t: &mut Tex, s: &[u8]| {
            let b = t.pool_ptr;
            for &c in s {
                t.str_pool.put(t.pool_ptr, c as i32);
                t.pool_ptr += 1;
            }
            t.str_pool.put(t.pool_ptr, 0);
            let r = t.pdfsetmatrix(b, 100, 200);
            t.pool_ptr = b;
            r
        };
        assert_eq!(m(t, b"1 0 0 1"), 1);
        assert_eq!(t.v_pdfship.matrix_stack.len, 1);
        t.checkpdfsave(5, 6);
        assert_eq!(m(t, b"2 0 0 2"), 1);
        assert_eq!(m(t, b"bad"), 0);
        assert_eq!(t.v_pdfship.matrix_stack.len, 2);
        // The new matrix is about the current point, on top of the old.
        let top = t.pdfship_matrix_top().unwrap();
        assert_eq!((top.a, top.d, top.e, top.f), (2.0, 2.0, -100.0, -200.0));
        t.checkpdfrestore(5, 6);
        assert_eq!(t.v_pdfship.matrix_stack.len, 1);
        assert_eq!(t.v_pdfship.pos_stack.len, 0);
        // In a form nothing is parsed or pushed, and the matrix does not apply.
        t.pdfshipoutbegin(false);
        assert_eq!(m(t, b"bad"), 1);
        assert_eq!(t.v_pdfship.matrix_stack.len, 1);
        assert!(t.pdfship_matrix_top().is_none());
        // A shipout does not reset the matrix stack, as in the C.
        t.pdfshipoutbegin(true);
        assert!(t.pdfship_matrix_top().is_some());
    }

    #[test]
    fn crc32_is_zlibs() {
        assert_eq!(pdfship_crc32(b""), 0);
        assert_eq!(pdfship_crc32(b"123456789"), 0xCBF43926);
    }
}
