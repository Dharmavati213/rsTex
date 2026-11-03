//! §672-693 Parts 32a-c: pdfTeX basic, output subroutines, page description.
//!
//! **The stream in progress.**  There is no seek-back: `pdf_begin_stream` turns on [`HVars::holding`], and from
//! there until `write_stream_length` what would go to the file goes to [`HVars::held`] instead, where the
//! `/Length` field can be overwritten by the absolute offset WEB remembers.  That asks nothing of the file but
//! appending, even when `pdf_flush` falls in the middle of the field.  Compressed data waits in
//! [`HVars::zip_in`] and is deflated in one call by `write_zip(true)`: PORTING section 18, since zlib-rs's output
//! depends on where the pieces begin.  `pdf_gone` stays WEB's own count.
//!
//! A procedure includes every module reachable from it, wherever defined: `tools/web.py tangle NAME`.
#![forbid(unsafe_code)]
#![deny(unreachable_patterns, unused_variables)]
#![allow(non_snake_case, unused_mut, unused_imports, dead_code)]

use crate::access::*;
use crate::consts::*;
use crate::host::*;
use crate::raw::*;
use crate::state::*;
use rstex_pdfio::{Deflater, Stream, level_for, write_int};

crate::pod_struct! {
    /// POD state private to this module.  Add fields here, not to state.rs.
    pub struct Vars {}
}

/// Non-POD state private to this module, in `self.host`.  It is all about one stream in progress, and a
/// snapshot is never taken inside a stream.
#[derive(Default)]
pub struct HVars {
    /// Between `pdf_begin_stream` and `write_stream_length`: file bytes go to `held`.
    holding: bool,
    /// The bytes the file has not got yet.  Keeps its capacity from one stream to the next.
    held: Vec<u8>,
    /// The file offset of `held[0]`.
    held_start: i64,
    /// The stream's bytes before deflating, in `zip_writing` state.
    zip_in: Vec<u8>,
    /// `writezip.c`'s `c_stream`, kept and reset.
    deflater: Option<Deflater>,
    /// The stream now being written is a font program: DESIGN section 9's level for it.
    fontfile_stream: bool,
}

impl Tex {
    /// True while a stream is being written: not a moment for a checkpoint.
    pub(crate) fn pdfout_in_stream(&self) -> bool {
        self.host.hv.pdfout.holding
    }

    /// §673
    pub fn get_pdf_compress_level(&mut self) -> i32 {
        self.pdf_compress_level()
    }

    /// §673
    pub fn get_pdf_suppress_warning_dup_map(&mut self) -> i32 {
        self.pdf_suppress_warning_dup_map()
    }

    /// §673
    pub fn get_pdf_suppress_warning_page_group(&mut self) -> i32 {
        self.pdf_suppress_warning_page_group()
    }

    /// §673
    pub fn get_pdf_suppress_ptex_info(&mut self) -> i32 {
        self.pdf_suppress_ptex_info()
    }

    /// §673
    pub fn get_pdf_omit_charset(&mut self) -> i32 {
        self.pdf_omit_charset()
    }

    /// §673
    pub fn get_ptex_use_underscore(&mut self) -> bool {
        self.pdf_ptex_use_underscore() > 0 || self.pdf_major_version() >= 2
    }

    /// §673
    pub fn get_nullfont(&mut self) -> i32 {
        NULL_FONT
    }

    /// §673
    pub fn get_fontbase(&mut self) -> i32 {
        FONT_BASE
    }

    /// §673
    pub fn get_nullcs(&mut self) -> i32 {
        NULL_CS
    }

    /// §673
    pub fn get_nullptr(&mut self) -> i32 {
        NULL
    }

    /// §673
    pub fn get_tex_int(&mut self, code: i32) -> i32 {
        self.int_par(code)
    }

    /// §673
    pub fn get_tex_dimen(&mut self, code: i32) -> i32 {
        self.dimen_par(code)
    }

    /// §673
    pub fn get_x_height(&mut self, f: i32) -> i32 {
        self.x_height(f)
    }

    /// §673
    pub fn get_charwidth(&mut self, f: i32, c: i32) -> i32 {
        if self.is_valid_char(f, c) { self.char_width(f, self.orig_char_info(f, c)) } else { 0 }
    }

    /// §673
    pub fn get_charheight(&mut self, f: i32, c: i32) -> i32 {
        if self.is_valid_char(f, c) { self.char_height(f, self.height_depth(self.orig_char_info(f, c))) } else { 0 }
    }

    /// §673
    pub fn get_chardepth(&mut self, f: i32, c: i32) -> i32 {
        if self.is_valid_char(f, c) { self.char_depth(f, self.height_depth(self.orig_char_info(f, c))) } else { 0 }
    }

    /// §673
    pub fn get_quad(&mut self, f: i32) -> i32 {
        self.quad(f)
    }

    /// §673
    pub fn get_slant(&mut self, f: i32) -> i32 {
        self.slant(f)
    }

    /// §674
    pub fn short_display_n(&mut self, p: i32, m: i32) {
        let mut p = p;
        let mut n: i32;
        let mut i = 0;
        self.font_in_short_display = NULL_FONT;
        if p == NULL {
            return;
        }
        while p > MEM_MIN {
            if self.is_char_node(p) {
                if p <= self.mem_end {
                    if self.font(p) != self.font_in_short_display {
                        if self.font(p) < FONT_BASE || self.font(p) > self.font_max {
                            self.print_char(b'*' as i32);
                        } else {
                            self.print_font_identifier(self.font(p));
                        }
                        self.print_char(b' ' as i32);
                        self.font_in_short_display = self.font(p);
                    }
                    self.print(self.character(p));
                }
            } else {
                if self.ty(p) == GLUE_NODE
                    || self.ty(p) == DISC_NODE
                    || self.ty(p) == PENALTY_NODE
                    || (self.ty(p) == KERN_NODE && self.subtype(p) == EXPLICIT)
                {
                    i += 1;
                }
                if i >= m {
                    return;
                }
                if self.ty(p) == DISC_NODE {
                    self.print(b'|' as i32);
                    self.short_display(self.pre_break(p));
                    self.print(b'|' as i32);
                    self.short_display(self.post_break(p));
                    self.print(b'|' as i32);
                    n = self.replace_count(p);
                    while n > 0 {
                        if self.link(p) != NULL {
                            p = self.link(p);
                        }
                        n -= 1;
                    }
                } else {
                    // §193 Print a short indication of the contents of node p
                    match self.ty(p) {
                        HLIST_NODE | VLIST_NODE | INS_NODE | WHATSIT_NODE | MARK_NODE | ADJUST_NODE | UNSET_NODE => {
                            self.print_s(b"[]");
                        }
                        RULE_NODE => self.print_char(b'|' as i32),
                        GLUE_NODE => {
                            if self.glue_ptr(p) != ZERO_GLUE {
                                self.print_char(b' ' as i32);
                            }
                        }
                        MATH_NODE => {
                            if self.subtype(p) >= L_CODE {
                                self.print_s(b"[]");
                            } else {
                                self.print_char(b'$' as i32);
                            }
                        }
                        LIGATURE_NODE => self.short_display(self.lig_ptr(p)),
                        DISC_NODE => {
                            self.short_display(self.pre_break(p));
                            self.short_display(self.post_break(p));
                            n = self.replace_count(p);
                            while n > 0 {
                                if self.link(p) != NULL {
                                    p = self.link(p);
                                }
                                n -= 1;
                            }
                        }
                        _ => {}
                    }
                }
            }
            p = self.link(p);
            if p == NULL {
                return;
            }
        }
        self.update_terminal();
    }

    /// §678
    pub fn pdf_get_mem(&mut self, s: i32) -> i32 {
        if s > SUP_PDF_MEM_SIZE - self.pdf_mem_ptr {
            self.overflow(b"PDF memory size (pdf_mem_size)", self.pdf_mem_size);
        }
        if self.pdf_mem_ptr + s > self.pdf_mem_size {
            let a = (0.2 * self.pdf_mem_size as f64) as i32;
            if self.pdf_mem_ptr + s > self.pdf_mem_size + a {
                self.pdf_mem_size = self.pdf_mem_ptr + s;
            } else if self.pdf_mem_size < SUP_PDF_MEM_SIZE - a {
                self.pdf_mem_size += a;
            } else {
                self.pdf_mem_size = SUP_PDF_MEM_SIZE;
            }
            // xrealloc_array: pdf_mem is inline, at its capacity.
        }
        let result = self.pdf_mem_ptr;
        self.pdf_mem_ptr += s;
        result
    }

    /// §682
    pub fn fix_int(&mut self, val: i32, min: i32, max: i32) -> i32 {
        if val < min {
            min
        } else if val > max {
            max
        } else {
            val
        }
    }

    /// §683
    pub fn check_pdfversion(&mut self) {
        if !self.pdf_version_written {
            self.pdf_version_written = true;
            if self.pdf_major_version() < 1 {
                self.print_err_s(b"pdfTeX error (invalid pdfmajorversion)");
                self.print_ln();
                self.help(&[b"The pdfmajorversion must be 1 or greater.", b"I changed this to 1."]);
                self.int_error(self.pdf_major_version());
                self.set_pdf_major_version(1);
            }
            if self.pdf_minor_version() < 0 || self.pdf_minor_version() > 9 {
                self.print_err_s(b"pdfTeX error (invalid pdfminorversion)");
                self.print_ln();
                self.help(&[b"The pdfminorversion must be between 0 and 9.", b"I changed this to 4."]);
                self.int_error(self.pdf_minor_version());
                self.set_pdf_minor_version(4);
            }
            self.fixed_pdf_major_version = self.pdf_major_version();
            self.fixed_pdf_minor_version = self.pdf_minor_version();
            self.fixed_gamma = self.fix_int(self.pdf_gamma(), 0, 1000000);
            self.fixed_image_gamma = self.fix_int(self.pdf_image_gamma(), 0, 1000000);
            self.fixed_image_hicolor = self.fix_int(self.pdf_image_hicolor(), 0, 1) != 0;
            self.fixed_image_apply_gamma = self.fix_int(self.pdf_image_apply_gamma(), 0, 1);
            self.fixed_pdf_objcompresslevel = self.fix_int(self.pdf_objcompresslevel(), 0, 3);
            self.fixed_pdf_draftmode = self.fix_int(self.pdf_draftmode(), 0, 1);
            self.fixed_inclusion_copy_font = self.fix_int(self.pdf_inclusion_copy_font(), 0, 1);
            if (self.fixed_pdf_major_version > 1 || self.fixed_pdf_minor_version >= 5) && self.fixed_pdf_objcompresslevel > 0 {
                self.pdf_os_enable = true;
            } else {
                if self.fixed_pdf_objcompresslevel > 0 {
                    self.pdf_warning(
                        b"Object streams",
                        b"\\pdfobjcompresslevel > 0 requires PDF-1.5 or greater. Object streams disabled now.",
                        true,
                        true,
                    );
                    self.fixed_pdf_objcompresslevel = 0;
                }
                self.pdf_os_enable = false;
            }
            self.ensure_pdf_open();
            self.fix_pdfoutput();
            self.pdf_print_s(b"%PDF-");
            self.pdf_print_int(self.fixed_pdf_major_version);
            self.pdf_print(b'.' as i32);
            self.pdf_print_int_ln(self.fixed_pdf_minor_version);
            self.pdf_print(b'%' as i32);
            self.pdf_out(208); // 'P' + 128
            self.pdf_out(212); // 'T' + 128
            self.pdf_out(197); // 'E' + 128
            self.pdf_out(216); // 'X' + 128
            self.pdf_print_nl();
        } else if self.fixed_pdf_minor_version != self.pdf_minor_version()
            || self.fixed_pdf_major_version != self.pdf_major_version()
        {
            self.pdf_error(b"setup", b"PDF version cannot be changed after data is written to the PDF file");
        }
    }

    /// §684
    pub fn ensure_pdf_open(&mut self) {
        if self.output_file_name != 0 {
            return;
        }
        if self.job_name == 0 {
            self.open_log_file();
        }
        self.pack_job_name(S_DOT_PDF);
        if self.fixed_pdf_draftmode == 0 {
            while !self.b_open_out(PDF_FILE) {
                self.prompt_file_name(b"file name for output", S_DOT_PDF);
            }
        }
        self.output_file_name = self.b_make_name_string(PDF_FILE);
    }

    /// §685
    #[inline(never)]
    pub fn pdf_flush(&mut self) {
        if !self.pdf_os_mode {
            let saved_pdf_gone = self.pdf_gone;
            match self.zip_write_state {
                NO_ZIP => {
                    if self.pdf_ptr > 0 {
                        if self.fixed_pdf_draftmode == 0 {
                            self.write_pdf(0, self.pdf_ptr - 1);
                        }
                        self.pdf_gone += self.pdf_ptr as i64;
                        self.pdf_last_byte = self.pdfout_buf_at(self.pdf_ptr - 1);
                    }
                }
                ZIP_WRITING => {
                    if self.fixed_pdf_draftmode == 0 {
                        self.write_zip(false);
                    }
                }
                ZIP_FINISH => {
                    if self.fixed_pdf_draftmode == 0 {
                        self.write_zip(true);
                    }
                    self.zip_write_state = NO_ZIP;
                }
                _ => {}
            }
            self.pdf_ptr = 0;
            if saved_pdf_gone > self.pdf_gone {
                self.pdf_error(b"file size", b"File size exceeds architectural limits (pdf_gone wraps around)");
            }
        }
    }

    /// §685
    pub fn pdf_begin_stream(&mut self) {
        // Before the field is printed, since a flush may fall in the middle of it.
        if !self.host.hv.pdfout.holding {
            self.host.hv.pdfout.holding = true;
            self.host.hv.pdfout.held_start = self.pdf_gone;
        }
        self.pdf_print_ln_s(b"/Length           ");
        self.pdf_seek_write_length = true; // fill in length at pdf_end_stream call
        self.pdf_stream_length_offset = self.pdf_offset() - 11;
        self.pdf_stream_length = 0;
        self.pdf_last_byte = 0;
        if self.pdf_compress_level() > 0 {
            self.pdf_print_ln_s(b"/Filter /FlateDecode");
            self.pdf_print_ln_s(b">>");
            self.pdf_print_ln_s(b"stream");
            self.pdf_flush();
            self.zip_write_state = ZIP_WRITING;
        } else {
            self.pdf_print_ln_s(b">>");
            self.pdf_print_ln_s(b"stream");
            self.pdf_save_offset = self.pdf_offset();
        }
    }

    /// §685
    pub fn pdf_end_stream(&mut self) {
        if self.zip_write_state == ZIP_WRITING {
            self.zip_write_state = ZIP_FINISH;
        } else {
            self.pdf_stream_length = self.pdf_offset() - self.pdf_save_offset;
        }
        self.pdf_flush();
        if self.pdf_seek_write_length {
            self.write_stream_length(self.pdf_stream_length, self.pdf_stream_length_offset);
        }
        self.pdf_seek_write_length = false;
        self.pdf_out(PDF_NEW_LINE_CHAR);
        self.pdf_print_ln_s(b"endstream");
        self.pdf_end_obj();
    }

    /// §686
    #[cold]
    pub fn pdf_error(&mut self, t: &[u8], p: &[u8]) -> ! {
        self.normalize_selector();
        self.print_err_s(b"pdfTeX error");
        if !t.is_empty() {
            self.print_s(b" (");
            self.print_s(t);
            self.print_s(b")");
        }
        self.print_s(b": ");
        self.print_s(p);
        self.succumb()
    }

    /// §686
    pub fn pdf_warning(&mut self, t: &[u8], p: &[u8], prepend_nl: bool, append_nl: bool) {
        if self.interaction == ERROR_STOP_MODE {
            self.wake_up_terminal();
        }
        if prepend_nl {
            self.print_ln();
        }
        self.print_s(b"pdfTeX warning");
        if !t.is_empty() {
            self.print_s(b" (");
            self.print_s(t);
            self.print_s(b")");
        }
        self.print_s(b": ");
        self.print_s(p);
        if append_nl {
            self.print_ln();
        }
        if self.history == SPOTLESS {
            self.history = WARNING_ISSUED;
        }
    }

    /// §686
    #[inline(never)]
    pub fn pdf_os_get_os_buf(&mut self, s: i32) {
        if s > SUP_PDF_OS_BUF_SIZE - self.pdf_ptr {
            self.overflow(b"PDF object stream buffer", self.pdf_os_buf_size);
        }
        if self.pdf_ptr + s > self.pdf_os_buf_size {
            let a = (0.2 * self.pdf_os_buf_size as f64) as i32;
            if self.pdf_ptr + s > self.pdf_os_buf_size + a {
                self.pdf_os_buf_size = self.pdf_ptr + s;
            } else if self.pdf_os_buf_size < SUP_PDF_OS_BUF_SIZE - a {
                self.pdf_os_buf_size += a;
            } else {
                self.pdf_os_buf_size = SUP_PDF_OS_BUF_SIZE;
            }
            // xrealloc_array: pdf_os_buf is inline, at its capacity.
            self.pdf_buf_is_os = true;
            self.pdf_buf_size = self.pdf_os_buf_size;
        }
    }

    /// §686
    pub fn remove_last_space(&mut self) {
        if self.pdf_ptr > 0 && self.pdfout_buf_at(self.pdf_ptr - 1) == 32 {
            self.pdf_ptr -= 1;
        }
    }

    /// §686
    pub fn pdf_print_octal(&mut self, n: i32) {
        let mut n = n;
        let mut k = 0;
        loop {
            self.dig[k] = n % 8;
            n /= 8;
            k += 1;
            if n == 0 {
                break;
            }
        }
        if k == 1 {
            self.pdf_out(b'0' as i32);
            self.pdf_out(b'0' as i32);
        }
        if k == 2 {
            self.pdf_out(b'0' as i32);
        }
        while k > 0 {
            k -= 1;
            self.pdf_out(b'0' as i32 + self.dig[k]);
        }
    }

    /// §686
    #[inline]
    pub fn pdf_print_char(&mut self, f: i32, c: i32) {
        self.pdf_mark_char(f, c);
        if c <= 32 || c == 92 || c == 40 || c == 41 || c > 127 {
            self.pdf_out(92); // output a backslash
            self.pdf_print_octal(c);
        } else {
            self.pdf_out(c);
        }
    }

    /// §686
    #[inline]
    pub fn pdf_print(&mut self, s: i32) {
        let mut j = self.str_start[s];
        while j < self.str_start[s + 1] {
            let c = self.str_pool.at(j);
            self.pdf_out(c);
            j += 1;
        }
    }

    /// §686
    pub fn str_in_str(&mut self, s: i32, r: &[u8], i: i32) -> bool {
        if self.length(s) < i + r.len() as i32 {
            return false;
        }
        let mut j = i + self.str_start[s];
        let mut k = 0;
        while j < self.str_start[s + 1] && k < r.len() {
            if self.str_pool.at(j) != r[k] as i32 {
                return false;
            }
            j += 1;
            k += 1;
        }
        true
    }

    /// §686
    #[inline]
    pub fn pdf_print_int(&mut self, n: impl Into<i64>) {
        let n: i64 = n.into();
        self.pdfout_print_int(n);
    }

    /// §686 `pdf_print_int`, not generic over its argument, so there is one copy of it.
    fn pdfout_print_int(&mut self, n: i64) {
        let mut n = n;
        let mut k = 0;
        if n < 0 {
            self.pdf_out(b'-' as i32);
            if n > -100000000 {
                n = -n;
            } else {
                let mut m = -1 - n;
                n = m / 10;
                m = (m % 10) + 1;
                k = 1;
                if m < 10 {
                    self.dig[0] = m as i32;
                } else {
                    self.dig[0] = 0;
                    n += 1;
                }
            }
        }
        loop {
            self.dig[k] = (n % 10) as i32;
            n /= 10;
            k += 1;
            if n == 0 {
                break;
            }
        }
        self.pdf_room(k);
        while k > 0 {
            k -= 1;
            self.pdf_quick_out(b'0' as i32 + self.dig[k]);
        }
    }

    /// §686
    pub fn pdf_print_two(&mut self, n: i32) {
        let n = abs(n) % 100;
        self.pdf_out(b'0' as i32 + (n / 10));
        self.pdf_out(b'0' as i32 + (n % 10));
    }

    /// §686
    pub fn tokens_to_string(&mut self, p: i32) -> i32 {
        if self.selector == NEW_STRING {
            self.pdf_error(b"tokens", b"tokens_to_string() called while selector = new_string");
        }
        self.old_setting = self.selector;
        self.selector = NEW_STRING;
        self.show_token_list(p + 1, NULL, self.pool_size - self.pool_ptr);
        self.selector = self.old_setting;
        self.last_tokens_string = self.make_string();
        self.last_tokens_string
    }

    /// §689.  `q` is an `i64`, stored with `sat32`: PORTING section 11.
    pub fn divide_scaled(&mut self, s: i32, m: i32, dd: i32) -> i32 {
        let (mut s, mut m) = (s, m);
        let mut sign = 1;
        if s < 0 {
            sign = -sign;
            s = -s;
        }
        if m < 0 {
            sign = -sign;
            m = -m;
        }
        if m == 0 {
            self.pdf_error(b"arithmetic", b"divided by zero");
        } else if m >= MAX_INTEGER / 10 {
            self.pdf_error(b"arithmetic", b"number too big");
        }
        let mut q = (s / m) as i64;
        let mut r = s % m;
        for _ in 1..=dd {
            q = 10 * q + ((10 * r) / m) as i64;
            r = (10 * r) % m;
        }
        if 2 * r >= m {
            q += 1;
            r -= m;
        }
        self.scaled_out = sat32(sign as i64 * (s as i64 - (r / self.ten_pow[dd]) as i64));
        sat32(sign as i64 * q)
    }

    /// §689.  `t`, `u`, `v` are `i64`: PORTING section 11.
    pub fn round_xn_over_d(&mut self, x: i32, n: i32, d: i32) -> i32 {
        let mut x = x;
        let positive = if x >= 0 {
            true
        } else {
            x = -x;
            false
        };
        let (x, n, d) = (x as i64, n as i64, d as i64);
        let t = (x % 0o100000) * n;
        let mut u = (x / 0o100000) * n + (t / 0o100000);
        let mut v = (u % d) * 0o100000 + (t % 0o100000);
        if u / d >= 0o100000 {
            self.arith_error = true;
        } else {
            u = 0o100000 * (u / d) + (v / d);
        }
        v %= d;
        if 2 * v >= d {
            u += 1;
        }
        if positive { sat32(u) } else { sat32(-u) }
    }

    /// §689
    pub fn is_bit_set(&mut self, n: i32, s: i32) -> bool {
        let mut m = 1i64;
        for _ in 1..=s - 1 {
            m *= 2;
        }
        (n as i64 / m) % 2 != 0
    }

    /// §690
    pub fn adv_char_width(&mut self, f: i32, c: i32) {
        let w = self.char_width(f, self.char_info(f, c));
        if self.isscalable(f) {
            let s;
            let mut s_out;
            if self.pdf_cur_Tm_a == 0 {
                s = self.divide_scaled(w, self.pdf_font_size[f], 4);
                s_out = self.scaled_out;
                self.pdf_delta_h += s_out;
            } else {
                let a = self.round_xn_over_d(w, 1000, 1000 + self.pdf_cur_Tm_a);
                s = self.divide_scaled(a, self.pdf_font_size[f], 4);
                let b = self.round_xn_over_d(self.pdf_font_size[f], abs(s), 10000);
                s_out = self.round_xn_over_d(b, 1000 + self.pdf_cur_Tm_a, 1000);
                if s < 0 {
                    s_out = -s_out;
                }
                self.pdf_delta_h += s_out;
            }
            self.adv_char_width_s = round(s as f64 / 10.0);
            self.adv_char_width_s_out = s_out;
        } else {
            let x = self.get_pk_char_width(f, w);
            self.pdf_delta_h += x;
        }
    }

    /// §690
    pub fn pdf_print_real(&mut self, m: i32, d: i32) {
        let (mut m, mut d) = (m, d);
        if m < 0 {
            self.pdf_out(b'-' as i32);
            m = -m;
        }
        self.pdf_print_int(m / self.ten_pow[d]);
        m %= self.ten_pow[d];
        if m > 0 {
            self.pdf_out(b'.' as i32);
            d -= 1;
            while m < self.ten_pow[d] {
                self.pdf_out(b'0' as i32);
                d -= 1;
            }
            while m % 10 == 0 {
                m /= 10;
            }
            self.pdf_print_int(m);
        }
    }

    /// §690
    pub fn pdf_print_bp(&mut self, s: i32) {
        let m = self.divide_scaled(s, self.one_hundred_bp, self.fixed_decimal_digits + 2);
        self.pdf_print_real(m, self.fixed_decimal_digits);
    }

    /// §690
    pub fn pdf_print_mag_bp(&mut self, s: i32) {
        let mut s = s;
        self.prepare_mag();
        if self.mag() != 1000 {
            s = self.round_xn_over_d(s, self.mag(), 1000);
        }
        self.pdf_print_bp(s);
    }

    /// §692
    pub fn pdf_set_origin(&mut self, h: i32, v: i32) {
        if abs(h - self.pdf_origin_h) >= self.min_bp_val || abs(v - self.pdf_origin_v) >= self.min_bp_val {
            self.pdf_print_s(b"1 0 0 1 ");
            self.pdf_print_bp(h - self.pdf_origin_h);
            self.pdf_origin_h += self.scaled_out;
            self.pdf_out(b' ' as i32);
            self.pdf_print_bp(self.pdf_origin_v - v);
            self.pdf_origin_v -= self.scaled_out;
            self.pdf_print_ln_s(b" cm");
        }
        self.pdf_h = self.pdf_origin_h;
        self.pdf_tj_start_h = self.pdf_h;
        self.pdf_v = self.pdf_origin_v;
    }

    /// §692
    pub fn pdf_set_origin_temp(&mut self, h: i32, v: i32) {
        if abs(h - self.pdf_origin_h) >= self.min_bp_val || abs(v - self.pdf_origin_v) >= self.min_bp_val {
            self.pdf_print_s(b"1 0 0 1 ");
            self.pdf_print_bp(h - self.pdf_origin_h);
            self.pdf_out(b' ' as i32);
            self.pdf_print_bp(self.pdf_origin_v - v);
            self.pdf_print_ln_s(b" cm");
        }
    }

    /// §692
    pub fn pdf_end_string(&mut self) {
        if self.pdf_doing_string {
            self.pdf_print_s(b")]TJ");
            self.pdf_doing_string = false;
        }
    }

    /// §692
    pub fn pdf_end_string_nl(&mut self) {
        if self.pdf_doing_string {
            self.pdf_print_ln_s(b")]TJ");
            self.pdf_doing_string = false;
        }
    }

    /// §692
    pub fn get_font_auto_expand_ratio(&mut self, f: i32) -> i32 {
        if self.pdf_font_auto_expand[f] { self.pdf_font_expand_ratio[f] } else { 0 }
    }

    /// §692
    pub fn pdf_set_text_pos(&mut self, v: i32, v_out: i32, f: i32) {
        self.pdf_out(b' ' as i32);
        let pdf_new_Tm_a = self.get_font_auto_expand_ratio(f);
        if pdf_new_Tm_a != 0 || (pdf_new_Tm_a == 0 && self.pdf_cur_Tm_a != 0) {
            self.pdf_print_real(1000 + pdf_new_Tm_a, 3);
            self.pdf_print_s(b" 0 0 1 ");
            self.pdf_print_bp(self.cur_h - self.pdf_origin_h);
            self.pdf_h = self.pdf_origin_h + self.scaled_out;
            self.pdf_out(b' ' as i32);
            self.pdf_print_bp(self.pdf_origin_v - self.cur_v);
            self.pdf_v = self.pdf_origin_v - self.scaled_out;
            self.pdf_print_s(b" Tm");
            self.pdf_cur_Tm_a = pdf_new_Tm_a;
            self.pdfassert(self.pdf_cur_Tm_a > -1000);
        } else {
            self.pdf_print_bp(self.cur_h - self.pdf_tj_start_h); // works only for unexpanded fonts
            self.pdf_h = self.pdf_tj_start_h + self.scaled_out;
            self.pdf_out(b' ' as i32);
            self.pdf_print_real(v, self.fixed_decimal_digits); // use v and v_out to avoid duplicate calculation
            self.pdf_v -= v_out;
            self.pdf_print_s(b" Td");
        }
        self.pdf_tj_start_h = self.pdf_h;
        self.pdf_delta_h = 0;
    }

    /// §692
    pub fn pdf_use_font(&mut self, f: i32, fontnum: i32) {
        let _ = self.divide_scaled(self.font_size[f], self.one_hundred_bp, 6);
        self.pdf_font_size[f] = self.scaled_out;
        self.font_used[f] = true;
        self.pdfassert(fontnum > 0 || (fontnum < 0 && self.pdf_font_num[-fontnum] > 0));
        self.pdf_font_num[f] = fontnum;
        if self.pdf_move_chars() > 0 {
            self.pdf_warning(b"", b"Primitive \\pdfmovechars is obsolete.", true, true);
            self.set_pdf_move_chars(0); // warn only once
        }
    }

    /// §693
    pub fn pdf_init_font(&mut self, f: i32) {
        self.pdfassert(!self.font_used[f]);
        // if f is auto expanded then ensure the base font is initialized
        if self.pdf_font_auto_expand[f] && self.pdf_font_blink[f] != NULL_FONT {
            let b = self.pdf_font_blink[f];
            if !self.isscalable(b) {
                self.pdf_error(b"font expansion", b"auto expansion is only possible with scalable fonts");
            }
            if !self.font_used[b] {
                self.pdf_init_font(b);
            }
            self.pdf_font_map[f] = self.pdf_font_map[b];
        }
        // check whether f can share the font object with some k: same tfm name at another size, or auto
        // expanded from k
        if self.isscalable(f) {
            let mut i = self.head_tab[OBJ_TYPE_FONT];
            while i != 0 {
                let k = self.obj_info(i);
                if self.isscalable(k)
                    && self.pdf_font_map[k] == self.pdf_font_map[f]
                    && (self.str_eq_str(self.font_name[k], self.font_name[f])
                        || (self.pdf_font_auto_expand[f]
                            && self.pdf_font_blink[f] != NULL_FONT
                            && self.str_eq_str(self.font_name[k], self.font_name[self.pdf_font_blink[f]])))
                {
                    self.pdfassert(self.pdf_font_num[k] != 0);
                    if self.pdf_font_num[k] < 0 {
                        self.pdf_use_font(f, self.pdf_font_num[k]);
                    } else {
                        self.pdf_use_font(f, -k);
                    }
                    return;
                }
                i = self.obj_link(i);
            }
        }
        // create a new font object for f
        self.pdf_create_obj(OBJ_TYPE_FONT, f);
        self.pdf_font_has_space_char[f] = self.hasspacechar(f) && self.char_width(f, self.char_info(f, 32)) > self.one_bp;
        self.pdf_use_font(f, self.obj_ptr);
    }

    /// §693
    pub fn pdf_init_font_cur_val(&mut self) {
        self.pdf_init_font(self.cur_val);
    }

    /// §693
    pub fn pdf_set_font(&mut self, f: i32) {
        self.pdf_f = f;
        if !self.font_used[f] {
            self.pdf_init_font(f);
        }
        // set ff to the tfm number of the font sharing the font object with f
        self.set_ff(f);
        let k = self.ff;
        let mut p = self.pdf_font_list;
        'found: {
            while p != NULL {
                self.set_ff(self.info(p));
                if self.ff == k {
                    break 'found;
                }
                p = self.link(p);
            }
            // f not found in pdf_font_list, append it now
            self.pdf_font_list = self.append_ptr(self.pdf_font_list, f);
        }
        if k == self.pdf_last_f && self.font_size[f] == self.pdf_last_fs {
            return;
        }
        self.pdf_print_s(b"/F");
        self.pdf_print_int(k);
        self.pdf_print_resname_prefix();
        self.pdf_out(b' ' as i32);
        let m = self.divide_scaled(self.font_size[f], self.one_hundred_bp, 6);
        self.pdf_print_real(m, 4);
        self.pdf_print_s(b" Tf");
        self.pdf_last_f = k;
        self.pdf_last_fs = self.font_size[f];
    }

    /// §693
    pub fn pdf_begin_text(&mut self) {
        self.pdf_set_origin(0, self.cur_page_height);
        self.pdf_print_ln_s(b"BT");
        self.pdf_doing_text = true;
        self.pdf_f = NULL_FONT;
        self.pdf_last_f = NULL_FONT;
        self.pdf_last_fs = 0;
        self.pdf_doing_string = false;
        self.pdf_cur_Tm_a = 0;
    }

    /// §693
    pub fn pdf_read_dummy_font(&mut self) {
        if self.pdf_dummy_font == NULL_FONT {
            self.pdf_dummy_font = self.read_font_info(NULL_CS, self.pdf_space_font_name, S_EMPTY, -1000);
            self.pdfmaplinesp();
            self.pdf_mark_char(self.pdf_dummy_font, 32);
        }
    }

    /// §693
    pub fn pdf_insert_interword_space(&mut self) {
        self.pdf_read_dummy_font();
        self.pdf_set_font(self.pdf_dummy_font);
        self.pdf_print_s(b"( )Tj");
    }

    /// §693.  WEB's conditions on `gen_faked_interword_space` and on the expansion ratio are read with C's
    /// precedence, since web2c writes the operators in order: `!g && pdf_f != f`, and `a != b || c != d`.
    pub fn pdf_begin_string(&mut self, f: i32) {
        // In WEB `s_out` is undefined when pdf_cur_Tm_a<>0 and abs(s)>=@'100000, and the oracle reads it at
        // move_looks_like_interword_space all the same.  It is above the threshold on every row of
        // tests/integration/pdfout/interword_tm.tex, of either sign.  Nothing else reads it on that path.
        let mut s_out = MAX_INTEGER;
        let mut s;
        let (v, v_out);
        let mut must_end_string = false;
        let mut must_insert_space = false;
        let mut must_set_text_pos = false;

        if !self.pdf_doing_text {
            self.pdf_begin_text();
            must_set_text_pos = true;
        }

        // delay font switch if we must insert interword space
        if (!self.gen_faked_interword_space && self.pdf_f != f)
            || (self.gen_faked_interword_space && !(self.font_used[f] && self.font_used[self.pdf_f]))
        {
            self.pdf_end_string();
            self.pdf_set_font(f);
        }
        if self.pdf_cur_Tm_a == 0 {
            s = self.divide_scaled(self.cur_h - (self.pdf_tj_start_h + self.pdf_delta_h), self.pdf_font_size[f], 3);
            s_out = self.scaled_out;
        } else {
            let a = self.round_xn_over_d(self.cur_h - (self.pdf_tj_start_h + self.pdf_delta_h), 1000, 1000 + self.pdf_cur_Tm_a);
            s = self.divide_scaled(a, self.pdf_font_size[f], 3);
            if abs(s) < 0o100000 {
                let b = self.round_xn_over_d(self.pdf_font_size[f], abs(s), 1000);
                s_out = self.round_xn_over_d(b, 1000 + self.pdf_cur_Tm_a, 1000);
                if s < 0 {
                    s_out = -s_out;
                }
            }
            // no need to calculate s_out when abs(s)>=@'100000, since the text matrix will be reset below
        }
        if abs(self.cur_v - self.pdf_v) >= self.min_bp_val {
            v = self.divide_scaled(self.pdf_v - self.cur_v, self.one_hundred_bp, self.fixed_decimal_digits + 2);
            v_out = self.scaled_out;
        } else {
            v = 0;
            v_out = 0;
        }

        if !must_set_text_pos {
            must_set_text_pos = v != 0
                || abs(s) >= 0o100000
                || self.get_font_auto_expand_ratio(f) != self.pdf_cur_Tm_a
                || self.get_font_auto_expand_ratio(f) != self.get_font_auto_expand_ratio(self.pdf_f);
        }

        if must_set_text_pos {
            must_end_string = true;
        }

        let move_looks_like_interword_space =
            self.space(f) > self.one_bp && s_out > self.space(f) - self.space_shrink(f) - self.one_bp / 10 && v == 0;

        if self.gen_faked_interword_space && move_looks_like_interword_space {
            must_insert_space = true;
        }

        if must_insert_space {
            // insert a real space char from the font when possible
            if self.pdf_font_has_space_char[f] && self.pdf_doing_string {
                self.pdf_out(b' ' as i32);
                self.adv_char_width(f, 32); // update pdf_delta_h, get adv_char_width_s and adv_char_width_s_out
                s -= self.adv_char_width_s;
                s_out -= self.adv_char_width_s_out;
                self.pdf_mark_char(f, 32);
            } else {
                must_end_string = true;
            }
        }

        if must_end_string {
            self.pdf_end_string();
            // insert a space char from the dummy font if needed
            if must_insert_space && !self.pdf_font_has_space_char[f] {
                self.pdf_insert_interword_space(); // this will change the current font
            }
            self.pdf_set_font(f);
            self.pdf_set_text_pos(v, v_out, f);
            s = 0;
        }

        // do font switch here after interword space could have been inserted
        if self.gen_faked_interword_space && self.pdf_f != f {
            self.pdf_end_string();
            self.pdf_set_font(f);
        }

        if !self.pdf_doing_string {
            self.pdf_print_s(b" [");
            if s == 0 {
                self.pdf_out(b'(' as i32);
            }
        }

        if s != 0 {
            if self.pdf_doing_string {
                self.pdf_out(b')' as i32);
            }
            self.pdf_print_int(-s);
            self.pdf_out(b'(' as i32);
            self.pdf_delta_h += s_out;
        }
        self.pdf_doing_string = true;
    }

    /// §693
    pub fn pdf_insert_fake_space(&mut self) {
        let s = self.gen_faked_interword_space;
        self.gen_faked_interword_space = false; // to prevent inserting another fake space in pdf_begin_string

        self.pdf_read_dummy_font();
        self.pdf_begin_string(self.pdf_dummy_font);
        self.pdf_print(b' ' as i32);
        self.adv_char_width(self.pdf_dummy_font, 32);
        self.pdf_end_string_nl();

        self.gen_faked_interword_space = s;
    }

    /// §693
    pub fn pdf_end_text(&mut self) {
        if self.pdf_doing_text {
            self.pdf_end_string_nl();
            self.pdf_print_ln_s(b"ET");
            self.pdf_doing_text = false;
        }
    }

    /// §693.  `(h + 1)/2` is Pascal's real division, and the oracle truncates what `y - (h + 1)/2` gives when it
    /// passes it as a `scaled`: so 1sp lower than integer division for a positive `y` and an even `h`, and the
    /// same for a negative `y`.  Checked on the oracle's output, tests/integration/pdfout/rules.tex.
    pub fn pdf_set_rule(&mut self, x: i32, y: i32, w: i32, h: i32) {
        self.pdf_end_text();
        self.pdf_print_ln_s(b"q");
        if h <= self.one_bp {
            self.pdf_set_origin_temp(x, (y as f64 - (h as f64 + 1.0) / 2.0) as i32);
            self.pdf_print_s(b"[]0 d 0 J ");
            self.pdf_print_bp(h);
            self.pdf_print_s(b" w 0 0 m ");
            self.pdf_print_bp(w);
            self.pdf_print_ln_s(b" 0 l S");
        } else if w <= self.one_bp {
            self.pdf_set_origin_temp((x as f64 + (w as f64 + 1.0) / 2.0) as i32, y);
            self.pdf_print_s(b"[]0 d 0 J ");
            self.pdf_print_bp(w);
            self.pdf_print_s(b" w 0 0 m 0 ");
            self.pdf_print_bp(h);
            self.pdf_print_ln_s(b" l S");
        } else {
            self.pdf_set_origin_temp(x, y);
            self.pdf_print_s(b"0 0 ");
            self.pdf_print_bp(w);
            self.pdf_out(b' ' as i32);
            self.pdf_print_bp(h);
            self.pdf_print_ln_s(b" re f");
        }
        self.pdf_print_ln_s(b"Q");
    }

    /// §693
    pub fn pdf_rectangle(&mut self, left: i32, top: i32, right: i32, bottom: i32) {
        self.prepare_mag();
        self.pdf_print_s(b"/Rect [");
        self.pdf_print_mag_bp(self.pdf_x(left));
        self.pdf_out(b' ' as i32);
        self.pdf_print_mag_bp(self.pdf_y(bottom));
        self.pdf_out(b' ' as i32);
        self.pdf_print_mag_bp(self.pdf_x(right));
        self.pdf_out(b' ' as i32);
        self.pdf_print_mag_bp(self.pdf_y(top));
        self.pdf_print_ln_s(b"]");
    }

    /// §693
    pub fn slow_print_substr(&mut self, s: i32, max_len: i32) {
        // WEB leaves j undefined on the first path, which is never taken: literal's strings are made ones.
        let mut j = self.str_start[s + 1];
        if s >= self.str_ptr || s < 256 {
            self.print(s);
        } else {
            j = self.str_start[s];
            while j < self.str_start[s + 1] && j <= self.str_start[s] + max_len {
                self.print(self.str_pool.at(j));
                j += 1;
            }
        }
        if j < self.str_start[s + 1] {
            self.print_s(b"..."); // indicate truncation
        }
    }

    /// §693
    pub fn literal(&mut self, s: i32, literal_mode: i32, warn: bool) {
        let mut literal_mode = literal_mode;
        let mut j = self.str_start[s];
        if literal_mode == SCAN_SPECIAL {
            if !(self.str_in_str(s, b"PDF:", 0) || self.str_in_str(s, b"pdf:", 0)) {
                if warn && !(self.str_in_str(s, b"SRC:", 0) || self.str_in_str(s, b"src:", 0) || self.length(s) == 0) {
                    self.print_nl_s(b"Non-PDF special ignored!");
                    self.print_nl_s(b"<special> ");
                    self.slow_print_substr(s, 64);
                    // length of printed line should be <=78; good enough.
                    self.print_ln();
                }
                return;
            }
            j += 4; // length("PDF:")
            if self.str_in_str(s, b"direct:", 4) {
                j += 7;
                literal_mode = DIRECT_ALWAYS;
            } else if self.str_in_str(s, b"page:", 4) {
                j += 5;
                literal_mode = DIRECT_PAGE;
            } else {
                literal_mode = SET_ORIGIN;
            }
        }
        match literal_mode {
            SET_ORIGIN => {
                self.pdf_end_text();
                self.pdf_set_origin(self.cur_h, self.cur_v);
            }
            DIRECT_PAGE => self.pdf_end_text(),
            DIRECT_ALWAYS => self.pdf_end_string_nl(),
            _ => self.confusion(b"literal1"),
        }
        while j < self.str_start[s + 1] {
            let c = self.str_pool.at(j);
            self.pdf_out(c);
            j += 1;
        }
        self.pdf_print_nl();
    }

    // ---- Helpers. ----

    /// `pdf_buf[k]`: PORTING section 20.
    #[inline(always)]
    pub fn pdfout_buf_at(&self, k: i32) -> i32 {
        if self.pdf_buf_is_os { self.pdf_os_buf.at(k) } else { self.pdf_op_buf.at(k) }
    }

    /// The stream begun next holds a font program, which `write_zip` deflates at the level for those (DESIGN
    /// section 9).  For `writefont.c`'s `write_fontfile`, just before its `pdf_begin_stream`.
    pub fn pdfout_next_stream_is_fontfile(&mut self) {
        self.host.hv.pdfout.fontfile_stream = true;
    }

    /// `pdftex_fail` with `writezip.c`'s `check_err` of `deflateInit`, whose error code -2 is `Z_STREAM_ERROR`.
    /// The name is `kpse_invocation_name`, in the oracle `pdftex`, or `pdflatex` when called so.
    #[cold]
    fn pdfout_zlib_fail(&mut self) -> ! {
        let name = rstex_kpse::invocation_name();
        self.print_ln();
        self.print_s(b"!pdfTeX error: ");
        self.print_s(&name);
        self.print_s(b": zlib: deflateInit() failed (error code -2)");
        self.print_ln();
        self.remove_pdffile();
        self.print_s(b" ==> Fatal error occurred, no output PDF file produced!");
        self.print_ln();
        self.uexit(1)
    }

    /// A job that stops without §1513, as `line_too_long` and `call_edit` do: stdio flushes at `exit`, so the oracle's
    /// file has everything `pdf_flush` wrote.  Here that is the chunk list and `held`, and it goes to the file now.  A
    /// stream being deflated is the one thing that differs: its bytes are still in `zip_in`.  If nothing was written
    /// the file stays as `b_open_out` made it, and `remove_pdffile` has already closed a file it removed.
    pub fn pdfout_write_unfinished(&mut self) {
        let host = &mut self.host;
        if let Slot::Pdf(p) = &mut host.files[PDF_FILE as usize] {
            p.write(&host.hv.pdfout.held);
            host.hv.pdfout.held.clear();
            if p.offset() > 0 {
                let _ = p.finish();
            }
        }
    }

    // ---- External functions: the C side of web2c and pdfTeX, called by WEB. ----

    /// External: utils.c writestreamlength.  The field and everything after it are still in `held`.
    pub fn write_stream_length(&mut self, length: i64, offset: i64) {
        if self.fixed_pdf_draftmode == 0 && self.host.hv.pdfout.holding {
            let mut digits = Vec::with_capacity(20);
            write_int(&mut digits, length);
            let host = &mut self.host;
            let at = (offset - host.hv.pdfout.held_start) as usize;
            host.hv.pdfout.held[at..at + digits.len()].copy_from_slice(&digits);
            if let Slot::Pdf(p) = &mut host.files[PDF_FILE as usize] {
                p.write(&host.hv.pdfout.held);
            }
        }
        self.host.hv.pdfout.held.clear();
        self.host.hv.pdfout.holding = false;
        self.host.hv.pdfout.fontfile_stream = false;
    }

    /// External: writezip.c, with the whole stream deflated at the end: PORTING section 18.  As in the C,
    /// `pdf_gone` and `pdf_last_byte` follow the bytes that came out, and `pdf_stream_length` is their number.
    pub fn write_zip(&mut self, finish: bool) {
        // deflateInit refuses a level above 9, at the first call for the stream, and the level stays in a stream.
        if self.pdf_compress_level() > 9 {
            self.pdfout_zlib_fail();
        }
        let z = &mut self.host.hv.pdfout;
        z.zip_in.extend_from_slice(self.pdf_op_buf.range(0, self.pdf_ptr));
        if finish {
            let stream = if z.fontfile_stream { Stream::FontFile } else { Stream::Content };
            // Any level above 0 gives DESIGN section 9's level, and we are here because pdf_begin_stream saw one.
            let level = level_for(1, stream).expect("compressing");
            let d = z.deflater.get_or_insert_with(|| Deflater::new(level));
            if d.level() != level {
                d.reset(level);
            }
            let before = z.held.len();
            let _p = crate::prof_scope!("deflate");
            d.finish(&z.zip_in, &mut z.held);
            z.zip_in.clear();
            let n = z.held.len() - before;
            self.pdf_gone += n as i64;
            self.pdf_last_byte = z.held[z.held.len() - 1] as i32;
            self.pdf_stream_length = n as i64;
        }
    }

    /// External: `pdf_buf[a..=b]` into the PDF chunk list, or into `held` while a stream is in progress.
    pub fn write_pdf(&mut self, a: i32, b: i32) {
        let s = if self.pdf_buf_is_os { self.pdf_os_buf.range(a, b + 1) } else { self.pdf_op_buf.range(a, b + 1) };
        let host = &mut self.host;
        if host.hv.pdfout.holding {
            host.hv.pdfout.held.extend_from_slice(s);
        } else if let Slot::Pdf(p) = &mut host.files[PDF_FILE as usize] {
            p.write(s);
        }
    }

    /// External: pdftex.h: pdf_char_used bit
    #[inline(always)]
    pub fn pdf_mark_char(&mut self, f: i32, c: i32) {
        let b = self.pdf_char_used[f].at(c / 8) | (1 << (c % 8));
        self.pdf_char_used[f].put(c / 8, b);
    }

    /// External: pdftex.h: pdf_char_used bit
    #[inline(always)]
    pub fn pdf_char_marked(&mut self, f: i32, c: i32) -> bool {
        self.pdf_char_used[f].at(c / 8) & (1 << (c % 8)) != 0
    }

    /// External: pdftex.h: confusion if false
    #[inline(always)]
    pub fn pdfassert(&mut self, cond: bool) {
        if !cond {
            self.confusion(b"pdfassert");
        }
    }
}

/// A machine set up for PDF output as §681, §688 and §792 would, for the tests of this module and of pdfobj.
#[cfg(test)]
pub(crate) mod pdfkit {
    use super::*;
    use crate::toks::testkit::machine;

    pub const PAGE_HEIGHT: i32 = 400 * 65536;

    pub fn pdf_machine() -> &'static mut Tex {
        let t = machine();
        t.pdf_buf_size = PDF_OP_BUF_SIZE;
        t.pdf_os_buf_size = INF_PDF_OS_BUF_SIZE;
        t.one_bp = 65782;
        t.one_hundred_bp = 6578176;
        t.one_hundred_inch = 473628672;
        t.one_inch = 226;
        t.ten_pow[0] = 1;
        for i in 1..=9 {
            t.ten_pow[i] = 10 * t.ten_pow[i - 1];
        }
        t.set_mag(1000);
        t.pdf_version_written = true; // nothing here tests the header
        digits(t, 3);
        t.host.files[PDF_FILE as usize] = Slot::Pdf(PdfOut::new(b"unused.pdf"));
        t
    }

    /// §792's `fixed_decimal_digits` and `min_bp_val`.
    pub fn digits(t: &mut Tex, d: i32) {
        t.fixed_decimal_digits = d;
        t.min_bp_val = t.divide_scaled(t.one_hundred_bp, t.ten_pow[d + 2], 0);
    }

    /// What is in the buffer now, which is then emptied without being written.
    pub fn taken(t: &mut Tex) -> String {
        let s = String::from_utf8_lossy(t.pdf_op_buf.range(0, t.pdf_ptr)).into_owned();
        t.pdf_ptr = 0;
        s
    }

    /// The file so far, after a flush.
    pub fn file(t: &mut Tex) -> Vec<u8> {
        t.pdf_flush();
        t.pdf_out_file().unwrap().contents()
    }

    /// A string in the pool, as a document would make it.
    pub fn string(t: &mut Tex, s: &[u8]) -> i32 {
        for &c in s {
            t.str_room(1);
            t.append_char(c as i32);
        }
        t.make_string()
    }
}

#[cfg(test)]
mod tests {
    use super::pdfkit::*;
    use super::*;
    use crate::toks::testkit::*;
    use rstex_pdfio::{deflate, inflate, write_fw_int, write_out_bytes, write_real};

    fn table() -> Vec<Vec<&'static str>> {
        include_str!("../testdata/pdfout_oracle.tsv").lines().map(|l| l.split('\t').collect()).collect()
    }

    fn num(s: &str) -> i32 {
        s.parse().unwrap()
    }

    /// tests/oracle/gen_pdfout_tables.py: what the oracle wrote for these numbers.
    #[test]
    fn number_output_matches_the_oracle() {
        let t = pdf_machine();
        let rows = table();
        assert!(rows.len() > 1000);
        let mut kinds = [0; 5];
        for r in &rows {
            match r[0] {
                "bp" => {
                    kinds[0] += 1;
                    digits(t, num(r[1]));
                    t.pdf_print_bp(num(r[2]));
                    assert_eq!(taken(t), r[3], "pdf_print_bp({}) with {} digits", r[2], r[1]);
                }
                "font" => {
                    kinds[1] += 1;
                    let m = t.divide_scaled(num(r[1]), t.one_hundred_bp, 6);
                    t.pdf_print_real(m, 4);
                    assert_eq!(taken(t), r[2], "font size {}", r[1]);
                }
                "magbp" => {
                    kinds[2] += 1;
                    t.set_mag(num(r[1]));
                    t.mag_set = 0;
                    digits(t, num(r[2]));
                    t.pdf_print_mag_bp(num(r[3]));
                    assert_eq!(taken(t), r[4], "pdf_print_mag_bp({}) at mag {} with {} digits", r[3], r[1], r[2]);
                }
                "origin" => {
                    kinds[3] += 1;
                    digits(t, num(r[1]));
                    t.pdf_origin_h = 0;
                    t.pdf_origin_v = PAGE_HEIGHT;
                    for m in r[2].split(',') {
                        let (h, v) = m.split_once(':').unwrap();
                        t.pdf_set_origin(num(h), num(v));
                    }
                    let got: Vec<String> = taken(t)
                        .lines()
                        .map(|l| l.strip_prefix("1 0 0 1 ").unwrap().strip_suffix(" cm").unwrap().to_string())
                        .collect();
                    assert_eq!(got.join(";"), r[3], "moves {} with {} digits", r[2], r[1]);
                }
                "rule" => {
                    kinds[4] += 1;
                    digits(t, num(r[1]));
                    t.pdf_origin_h = 0;
                    t.pdf_origin_v = PAGE_HEIGHT;
                    t.pdf_set_rule(num(r[2]), num(r[3]), num(r[4]), num(r[5]));
                    let got = taken(t);
                    let inner = got.strip_prefix("q\n").unwrap().strip_suffix("Q\n").unwrap();
                    let inner: Vec<&str> = inner.split_whitespace().collect();
                    assert_eq!(inner.join(" "), r[6], "pdf_set_rule({},{},{},{}) with {} digits", r[2], r[3], r[4], r[5], r[1]);
                }
                k => panic!("unknown row {k}"),
            }
        }
        assert!(kinds.iter().all(|&n| n > 0), "{kinds:?}");
        Tex::dealloc(t);
    }

    #[test]
    fn pdf_print_int_at_its_limits() {
        let t = pdf_machine();
        for n in [
            0i64, 1, -1, 9, 10, -10, 99999999, -99999999, -100000000, -100000001, -100000009, -100000010, 2147483647, -2147483647,
            1 << 40, -(1 << 40), 999999999999999, i64::MAX, i64::MIN + 1,
        ] {
            t.pdf_print_int(n);
            assert_eq!(taken(t), n.to_string());
        }
        t.pdf_print_int(7i32);
        t.pdf_print_int_ln(-7i32);
        assert_eq!(taken(t), "7-7\n");
        Tex::dealloc(t);
    }

    /// Against crates/pdfio, whose tables came from the oracle.
    #[test]
    fn the_other_number_forms_agree_with_pdfio() {
        let t = pdf_machine();
        for &(n, w) in &[(0i64, 10), (1, 10), (1234567890, 10), (12345678901, 10), (65535, 5), (65535, 2), (1 << 40, 12)] {
            t.pdf_print_fw_int(n, w);
            let mut v = Vec::new();
            write_fw_int(&mut v, n, w as usize);
            assert_eq!(taken(t).as_bytes(), &v[..], "pdf_print_fw_int({n},{w})");
        }
        for &(n, w) in &[(0i64, 2), (255, 1), (256, 2), (65535, 2), (70000, 3), (16777216, 4), (1 << 39, 5), (258, 1)] {
            t.pdf_out_bytes(n, w);
            let mut v = Vec::new();
            write_out_bytes(&mut v, n, w as usize);
            assert_eq!(t.pdf_op_buf.range(0, t.pdf_ptr), &v[..], "pdf_out_bytes({n},{w})");
            t.pdf_ptr = 0;
        }
        for &(m, d) in &[(0, 0), (5, 0), (0, 3), (1, 3), (10, 3), (100, 3), (1001, 3), (1500, 3), (-1500, 3), (-1, 3), (99999, 4), (1000000000, 9)] {
            t.pdf_print_real(m, d);
            let mut v = Vec::new();
            write_real(&mut v, m, d as usize);
            assert_eq!(taken(t).as_bytes(), &v[..], "pdf_print_real({m},{d})");
        }
        Tex::dealloc(t);
    }

    #[test]
    fn small_forms() {
        let t = pdf_machine();
        for (n, s) in [(0, "000"), (7, "007"), (8, "010"), (63, "077"), (64, "100"), (255, "377")] {
            t.pdf_print_octal(n);
            assert_eq!(taken(t), s);
        }
        for (n, s) in [(0, "00"), (5, "05"), (42, "42"), (100, "00"), (-7, "07"), (12345, "45")] {
            t.pdf_print_two(n);
            assert_eq!(taken(t), s);
        }
        for c in 0..256 {
            assert!(!t.pdf_char_marked(2, c));
            t.pdf_print_char(2, c);
            assert!(t.pdf_char_marked(2, c));
            let want = if c <= 32 || c == 92 || c == 40 || c == 41 || c > 127 {
                format!("\\{c:03o}")
            } else {
                (c as u8 as char).to_string()
            };
            assert_eq!(taken(t), want);
            assert!(c == 255 || !t.pdf_char_marked(2, c + 1), "only that bit");
        }
        t.pdf_print_s(b"a b ");
        t.remove_last_space();
        t.remove_last_space();
        assert_eq!(taken(t), "a b");
        t.remove_last_space();
        assert_eq!(t.pdf_ptr, 0);
        Tex::dealloc(t);
    }

    #[test]
    fn divide_scaled_leaves_the_rounded_amount_in_scaled_out() {
        let t = pdf_machine();
        // §792, the two ends of min_bp_val
        assert_eq!(t.divide_scaled(t.one_hundred_bp, t.ten_pow[6], 0), 7);
        assert_eq!(t.scaled_out, 6578176 + 421824);
        assert_eq!(t.divide_scaled(t.one_hundred_bp, t.ten_pow[2], 0), 65782);
        assert_eq!(t.divide_scaled(-655360, 6578176, 5), -9963);
        let a = t.scaled_out;
        assert_eq!(t.divide_scaled(655360, -6578176, 5), -9963);
        assert_eq!(t.scaled_out, a);
        assert_eq!(t.divide_scaled(-655360, -6578176, 5), 9963);
        assert_eq!(t.scaled_out, -a);
        assert!(a < 0 && (a + 655360).abs() < 66, "{a}");
        // A value whose quotient passes 2^31: sat32, with the right sign.
        assert_eq!(t.divide_scaled(1 << 30, 1, 3), i32::MAX);
        assert_eq!(t.divide_scaled(-(1 << 30), 1, 3), -i32::MAX);
        assert_eq!(t.divide_scaled(0, 7, 4), 0);
        assert_eq!(t.scaled_out, 0);
        Tex::dealloc(t);
    }

    #[test]
    fn round_xn_over_d_rounds_half_away_and_sets_arith_error() {
        let t = pdf_machine();
        for &(x, n, d, want) in &[
            (0, 1500, 1000, 0),
            (1, 1500, 1000, 2),
            (3, 1500, 1000, 5),
            (-3, 1500, 1000, -5),
            (2, 1000, 1500, 1),
            (1, 1000, 1500, 1),
            (1, 1000, 3000, 0),
            (65536, 1500, 1000, 98304),
            (1000000, 333, 1000, 333000),
            (1073741823, 1000, 1000, 1073741823),
        ] {
            t.arith_error = false;
            assert_eq!(t.round_xn_over_d(x, n, d), want, "{x} {n} {d}");
            assert!(!t.arith_error);
        }
        t.round_xn_over_d(1 << 30, 1 << 20, 1);
        assert!(t.arith_error);
        Tex::dealloc(t);
    }

    #[test]
    fn small_functions() {
        let t = pdf_machine();
        assert_eq!((t.fix_int(5, 0, 9), t.fix_int(-1, 0, 9), t.fix_int(10, 0, 9), t.fix_int(0, 0, 9), t.fix_int(9, 0, 9)), (5, 0, 9, 0, 9));
        // The oracle's answers for the first of these are in tests/integration/pdfout/misc.tex.
        for (n, s, want) in [
            (0, 1, false),
            (1, 1, true),
            (2, 1, false),
            (3, 1, true),
            (-1, 1, true),
            (-2, 1, false),
            (-3, 1, true),
            (2, 2, true),
            (1, 2, false),
            (i32::MAX, 31, true),
            (1, 33, false),
        ] {
            assert_eq!(t.is_bit_set(n, s), want, "{n} {s}");
        }
        t.pdf_font_auto_expand[1] = true;
        t.pdf_font_expand_ratio[1] = 20;
        t.pdf_font_expand_ratio[2] = 30;
        assert_eq!((t.get_font_auto_expand_ratio(1), t.get_font_auto_expand_ratio(2)), (20, 0));
        t.pdf_mem_ptr = 1;
        t.pdf_mem_size = 10000;
        assert_eq!(t.pdf_get_mem(9999), 1);
        assert_eq!((t.pdf_mem_ptr, t.pdf_mem_size), (10000, 10000));
        assert_eq!(t.pdf_get_mem(1), 10000);
        assert_eq!((t.pdf_mem_ptr, t.pdf_mem_size), (10001, 12000));
        assert_eq!(t.pdf_get_mem(5000), 10001);
        assert_eq!((t.pdf_mem_ptr, t.pdf_mem_size), (15001, 15001));
        Tex::dealloc(t);
    }

    #[test]
    fn literal_ends_what_each_mode_ends() {
        let t = pdf_machine();
        print_setup(t);
        let s = string(t, b"PDF:direct:abc");
        assert!(t.str_in_str(s, b"PDF:", 0) && t.str_in_str(s, b"direct:", 4) && t.str_in_str(s, b"abc", 11) && t.str_in_str(s, b"", 14));
        assert!(!t.str_in_str(s, b"pdf:", 0) && !t.str_in_str(s, b"abcd", 11) && !t.str_in_str(s, b"x", 14));

        let origin = ")]TJ\nET\n1 0 0 1 0 100 cm\n";
        let page = ")]TJ\nET\n";
        let direct = ")]TJ\n";
        let cases: &[(&[u8], i32, &str, &str)] = &[
            (b"x", SET_ORIGIN, origin, "x\n"),
            (b"x", DIRECT_PAGE, page, "x\n"),
            (b"x", DIRECT_ALWAYS, direct, "x\n"),
            (b"pdf:x", SCAN_SPECIAL, origin, "x\n"),
            (b"PDF:x", SCAN_SPECIAL, origin, "x\n"),
            (b"PDF:direct:x", SCAN_SPECIAL, direct, "x\n"),
            (b"pdf:direct:x", SCAN_SPECIAL, direct, "x\n"),
            (b"pdf:page:x", SCAN_SPECIAL, page, "x\n"),
            (b"PDF:page:", SCAN_SPECIAL, page, "\n"),
            (b"pdf:", SCAN_SPECIAL, origin, "\n"),
        ];
        for &(body, mode, before, after) in cases {
            t.pdf_origin_h = 0;
            t.pdf_origin_v = PAGE_HEIGHT;
            t.cur_h = 0;
            t.cur_v = PAGE_HEIGHT - t.one_hundred_bp;
            t.pdf_doing_text = true;
            t.pdf_doing_string = true;
            let s = string(t, body);
            t.literal(s, mode, true);
            let want = before.to_string() + after;
            assert_eq!(taken(t), want, "{:?}", String::from_utf8_lossy(body));
            assert!(!t.pdf_doing_string);
            assert_eq!(t.pdf_doing_text, before == direct);
        }
        Tex::dealloc(t);
    }

    #[test]
    fn other_specials_are_ignored_with_a_message_for_some() {
        let t = pdf_machine();
        print_setup(t);
        for (body, warn, msg) in [
            (&b"foo"[..], true, "\nNon-PDF special ignored!\n<special> foo\n"),
            (b"foo", false, ""),
            (b"src:1 a.tex", true, ""),
            (b"SRC:1 a.tex", true, ""),
            (b"", true, ""),
            (b"pdf", true, "\nNon-PDF special ignored!\n<special> pdf\n"),
        ] {
            let s = string(t, body);
            t.term_offset = 1;
            t.literal(s, SCAN_SPECIAL, warn);
            assert_eq!(printed(t), msg);
            assert_eq!(taken(t), "");
        }
        // 65 characters print whole, 66 are cut.  tests/integration/pdfout/literal.tex has the oracle's.
        for (n, cut) in [(63, false), (64, false), (65, false), (66, true)] {
            let body = vec![b'x'; n];
            let s = string(t, &body);
            t.slow_print_substr(s, 64);
            let want = "x".repeat(n.min(65)) + if cut { "..." } else { "" };
            assert_eq!(printed(t), want);
        }
        let s = string(t, b"\x80");
        t.slow_print_substr(s, 64);
        assert_eq!(printed(t), "^^80");
        Tex::dealloc(t);
    }

    /// Everything a stream does around its data, with no compression, against the bytes pdfTeX writes: the oracle
    /// writes this frame for \immediate\pdfobj stream, tests/integration/pdfout/streams.tex.
    #[test]
    fn a_plain_stream_gets_its_length_in_the_blanks() {
        let t = pdf_machine();
        t.set_pdf_compress_level(0);
        t.pdf_print_s(b"1 0 obj\n<<\n");
        t.pdf_begin_stream();
        t.pdf_print_s(b"hello");
        t.pdf_end_stream();
        assert_eq!(String::from_utf8(file(t)).unwrap(), "1 0 obj\n<<\n/Length 5         \n>>\nstream\nhello\nendstream\nendobj\n");
        assert_eq!(t.pdf_offset(), 63);
        assert_eq!((t.pdf_stream_length, t.pdf_seek_write_length), (5, false));
        Tex::dealloc(t);
    }

    /// The length goes into the right place wherever pdf_flush falls: before the field, in it, after it.
    #[test]
    fn a_flush_in_the_middle_of_the_length_field_does_no_harm() {
        for compress in [0, 9] {
            for gap in 0..40 {
                let t = pdf_machine();
                t.set_pdf_compress_level(compress);
                let pad = vec![b'.'; (PDF_OP_BUF_SIZE - 30 + gap) as usize];
                t.pdf_print_s(&pad);
                t.pdf_begin_stream();
                t.pdf_print_s(b"hello");
                t.pdf_end_stream();
                let f = file(t);
                assert_eq!(f.len() as i64, t.pdf_offset(), "pdf_gone counts the file");
                let at = pad.len();
                assert!(f[at..].starts_with(b"/Length "), "gap {gap}");
                let digits_end = at + 8 + f[at + 8..].iter().position(|&c| c == b' ').unwrap();
                let len: i64 = std::str::from_utf8(&f[at + 8..digits_end]).unwrap().parse().unwrap();
                assert_eq!(len, t.pdf_stream_length);
                assert_eq!(f[at + 18], b'\n', "the field is 10 wide");
                let head: &[u8] = if compress > 0 { b"/Filter /FlateDecode\n>>\nstream\n" } else { b">>\nstream\n" };
                assert!(f[at + 19..].starts_with(head));
                let body = at + 19 + head.len();
                let data = &f[body..body + len as usize];
                if compress > 0 {
                    let mut out = Vec::new();
                    assert_eq!(inflate(data, &mut out), Ok(data.len()));
                    assert_eq!(out, b"hello");
                } else {
                    assert_eq!(data, b"hello");
                }
                assert_eq!(&f[body + len as usize..], b"\nendstream\nendobj\n");
                Tex::dealloc(t);
            }
        }
    }

    /// PORTING section 18: one deflate of the whole stream, so the bytes do not depend on the flushes.
    #[test]
    fn compressed_bytes_do_not_depend_on_where_the_flushes_fell() {
        let data: Vec<u8> = (0..60000u32).map(|i| b"BT /F1 9.9626 Tf 72 720 Td ET\n"[(i % 30) as usize] ^ ((i >> 11) as u8 & 1)).collect();
        let mut whole = Vec::new();
        deflate(&data, 6, &mut whole);
        for start in [0, 1, 100, 16383] {
            let t = pdf_machine();
            t.set_pdf_compress_level(9);
            t.pdf_print_s(&vec![b'.'; start]);
            t.pdf_begin_stream();
            t.pdf_print_s(&data);
            t.pdf_end_stream();
            let f = file(t);
            assert_eq!(t.pdf_stream_length as usize, whole.len());
            let at = f.windows(7).position(|w| w == b"stream\n").unwrap() + 7;
            assert_eq!(&f[at..at + whole.len()], &whole[..], "start {start}");
            assert_eq!(&f[at + whole.len()..], b"\nendstream\nendobj\n");
            assert_eq!(f.len() as i64, t.pdf_offset());
            assert_eq!(t.pdf_last_byte, b'\n' as i32);
            Tex::dealloc(t);
        }
    }

    #[test]
    fn a_stream_after_a_stream_and_an_empty_one() {
        let t = pdf_machine();
        t.set_pdf_compress_level(1);
        t.pdf_begin_stream();
        t.pdf_end_stream();
        let f1 = file(t);
        let mut empty = Vec::new();
        deflate(b"", 6, &mut empty);
        let head = b"/Length 8         \n/Filter /FlateDecode\n>>\nstream\n";
        assert_eq!(empty.len(), 8);
        assert_eq!(&f1[..head.len()], head);
        assert_eq!(&f1[head.len()..head.len() + 8], &empty[..]);
        t.set_pdf_compress_level(0);
        t.pdf_begin_stream();
        t.pdf_print_s(b"ab");
        t.pdf_end_stream();
        t.set_pdf_compress_level(7);
        t.pdf_begin_stream();
        t.pdf_print_s(b"abc");
        t.pdf_end_stream();
        let f = file(t);
        let plain = b"/Length 2         \n>>\nstream\nab\nendstream\nendobj\n";
        assert_eq!(&f[f1.len()..f1.len() + plain.len()], plain);
        let at = f1.len() + plain.len();
        assert!(f[at..].starts_with(b"/Length 11        \n/Filter"), "{:?}", String::from_utf8_lossy(&f[at..at + 20]));
        Tex::dealloc(t);
    }

    /// DESIGN section 9: a font program goes at the level for those, and only the stream that asked.
    #[test]
    fn a_fontfile_stream_has_its_own_level() {
        let t = pdf_machine();
        t.set_pdf_compress_level(9);
        let data: Vec<u8> = (0..5000u32).map(|i| (i.wrapping_mul(2654435761) >> 25) as u8 % 16).collect();
        let (mut l1, mut l6) = (Vec::new(), Vec::new());
        deflate(&data, 1, &mut l1);
        deflate(&data, 6, &mut l6);
        assert_ne!(l1, l6);
        for (font, want) in [(true, &l1), (false, &l6), (true, &l1), (false, &l6)] {
            let before = file(t).len();
            if font {
                t.pdfout_next_stream_is_fontfile();
            }
            t.pdf_begin_stream();
            t.pdf_print_s(&data);
            t.pdf_end_stream();
            let f = file(t);
            let at = before + f[before..].windows(7).position(|w| w == b"stream\n").unwrap() + 7;
            assert_eq!(&f[at..at + want.len()], &want[..]);
        }
        Tex::dealloc(t);
    }

    /// writejpg.c and the PDF inclusion call pdf_end_stream with no pdf_begin_stream: nothing is held or patched.
    #[test]
    fn an_end_with_no_begin_only_ends() {
        let t = pdf_machine();
        t.set_pdf_compress_level(9);
        t.pdf_print_s(b"/Length 3\n>>\nstream\nabc");
        t.pdf_end_stream();
        assert_eq!(String::from_utf8(file(t)).unwrap(), "/Length 3\n>>\nstream\nabc\nendstream\nendobj\n");
        Tex::dealloc(t);
    }

    #[test]
    fn draft_mode_writes_nothing_and_counts_what_is_not_compressed() {
        let t = pdf_machine();
        t.fixed_pdf_draftmode = 1;
        t.set_pdf_compress_level(0);
        t.pdf_begin_stream();
        t.pdf_print_s(b"hello");
        t.pdf_end_stream();
        let plain = 29 + 5 + 18;
        assert_eq!((file(t).len(), t.pdf_gone), (0, plain));
        t.set_pdf_compress_level(9);
        t.pdf_begin_stream();
        t.pdf_print_s(&vec![b'x'; 40000]);
        t.pdf_end_stream();
        assert_eq!(file(t).len(), 0);
        // the header and the end of the object; the compressed part is never counted
        assert_eq!(t.pdf_gone, plain + 19 + 21 + 3 + 7 + 18);
        assert_eq!(t.zip_write_state, NO_ZIP);
        t.fixed_pdf_draftmode = 0;
        t.pdf_print_s(b"z");
        assert_eq!(file(t), b"z");
        Tex::dealloc(t);
    }

    #[test]
    fn the_object_stream_buffer_grows_as_web_says() {
        let t = pdf_machine();
        t.pdf_os_enable = true;
        t.fixed_pdf_objcompresslevel = 2;
        t.pdf_os_switch(true);
        assert!(t.pdf_os_mode && t.pdf_buf_is_os);
        t.pdf_out(b'a' as i32);
        assert_eq!((t.pdf_ptr, t.pdf_os_buf_size, t.pdf_buf_size), (1, 1, 1));
        t.pdf_out(b'b' as i32);
        assert_eq!((t.pdf_ptr, t.pdf_os_buf_size), (2, 2));
        t.pdf_room(1000);
        assert_eq!(t.pdf_os_buf_size, 1002, "by what it takes");
        t.pdf_ptr = 1002;
        t.pdf_room(1);
        assert_eq!((t.pdf_os_buf_size, t.pdf_buf_size), (1202, 1202), "by 0.2 of itself");
        t.pdf_flush();
        assert_eq!(t.pdf_ptr, 1002, "pdf_flush does nothing in object stream mode");
        assert_eq!(t.pdfout_buf_at(1), b'b' as i32);
        t.pdf_print_s(b"x ");
        t.remove_last_space();
        assert_eq!(t.pdfout_buf_at(t.pdf_ptr - 1), b'x' as i32);
        t.pdf_os_switch(false);
        assert!(!t.pdf_os_mode && !t.pdf_buf_is_os);
        assert_eq!((t.pdf_ptr, t.pdf_os_ptr, t.pdf_buf_size), (0, 1003, PDF_OP_BUF_SIZE));
        t.pdf_os_switch(true);
        assert_eq!((t.pdf_ptr, t.pdf_buf_size), (1003, 1202));
        Tex::dealloc(t);
    }

    /// §193's kinds of node and §674's count, on a real list.
    #[test]
    fn short_display_n_stops_at_its_count() {
        let t = crate::mem::memkit::mem_machine();
        print_setup(t);
        t.font_max = 10;
        let name = string(t, b"f");
        t.set_font_id_text(1, name);
        let chars = |t: &mut Tex, c: u8| {
            let p = t.get_avail();
            t.set_font(p, 1);
            t.set_character(p, c as i32);
            p
        };
        let a = chars(t, b'a');
        let g1 = t.new_glue(FIL_GLUE);
        let b = chars(t, b'b');
        let r = t.new_rule();
        let k = t.new_kern(5);
        t.set_subtype(k, EXPLICIT);
        let c = chars(t, b'c');
        let g2 = t.new_glue(FIL_GLUE);
        let d = chars(t, b'd');
        for (x, y) in [(a, g1), (g1, b), (b, r), (r, k), (k, c), (c, g2), (g2, d)] {
            t.set_link(x, y);
        }
        t.short_display_n(a, 1);
        assert_eq!(printed(t), "\\f a");
        t.short_display_n(a, 2);
        assert_eq!(printed(t), "\\f a b|");
        t.short_display_n(a, 3);
        assert_eq!(printed(t), "\\f a b|c");
        t.short_display_n(a, 4);
        assert_eq!(printed(t), "\\f a b|c d");
        t.short_display_n(NULL, 4);
        assert_eq!(printed(t), "");
        Tex::dealloc(t);
    }
}
