
/// §653 `billion`.
pub const BILLION: f64 = 1000000000.0;

/// §653 `vet_glue(x)`: returns the `glue_temp` the macro would set, clamped to +-billion.
#[inline(always)]
pub fn vet_glue(x: f64) -> f64 {
    if x > BILLION {
        BILLION
    } else if x < -BILLION {
        -BILLION
    } else {
        x
    }
}

/// The well-known strings: WEB string literals that must exist as real string numbers, because
/// they are stored in or compared with `str_number` variables.  `get_strings_started` makes them
/// right after the single-character strings, in this order, so `S_EMPTY` is 256 as in tex.ch.
/// Any other literal stays a byte string (`print_s(b"..")`); see docs/PORTING.md.  Files other
/// than strings.rs do not add to this table: at INITEX time use `self.lit_string(b"..")`.
pub const WELL_KNOWN_STRINGS: &[&[u8]] = &[
    b"", b".tex", b".log", b".dvi", b".pdf", b".fls", b".fmt", b".vf", b"texput", b" (INITEX)",
    b"FONT", b"nullfont", b"notexpanded:", b"pdfprimitive", b"endtemplate", b"inaccessible",
    b"endwrite", b"pdftexspace", b"???", b"?",
];
pub const S_EMPTY: i32 = 256;
pub const S_DOT_TEX: i32 = 257;
pub const S_DOT_LOG: i32 = 258;
pub const S_DOT_DVI: i32 = 259;
pub const S_DOT_PDF: i32 = 260;
pub const S_DOT_FLS: i32 = 261;
pub const S_DOT_FMT: i32 = 262;
pub const S_DOT_VF: i32 = 263;
pub const S_TEXPUT: i32 = 264;
pub const S_INITEX_IDENT: i32 = 265;
pub const S_FONT: i32 = 266;
pub const S_NULLFONT: i32 = 267;
pub const S_NOTEXPANDED: i32 = 268;
pub const S_PDFPRIMITIVE: i32 = 269;
pub const S_ENDTEMPLATE: i32 = 270;
pub const S_INACCESSIBLE: i32 = 271;
pub const S_ENDWRITE: i32 = 272;
pub const S_PDFTEXSPACE: i32 = 273;
pub const S_QUESTIONS: i32 = 274;
pub const S_QUESTION: i32 = 275;
/// The first string number after the well-known strings.
pub const S_FIRST_FREE: i32 = S_EMPTY + WELL_KNOWN_STRINGS.len() as i32;

impl Tex {
    // ---- §54-71 printing literals ----

    /// `print("abc")` for a multi-character literal: §59's loop over the string.
    #[inline]
    pub fn print_s(&mut self, s: &[u8]) {
        for &c in s {
            self.print_char(c as i32);
        }
    }
    /// `print_nl("abc")`, §62.
    pub fn print_nl_s(&mut self, s: &[u8]) {
        if self.selector < NO_PRINT
            || (self.term_offset > 0 && odd(self.selector))
            || (self.file_offset > 0 && self.selector >= LOG_ONLY)
        {
            self.print_ln();
        }
        self.print_s(s);
    }
    /// `print_esc("abc")`, §63.
    pub fn print_esc_s(&mut self, s: &[u8]) {
        let c = self.escape_char();
        if c >= 0 && c < 256 {
            self.print(c);
        }
        self.print_s(s);
    }
    /// `print_err("abc")`, §73.
    pub fn print_err_s(&mut self, s: &[u8]) {
        if self.interaction == ERROR_STOP_MODE {
            self.wake_up_terminal();
        }
        if self.file_line_error_style_p != 0 {
            self.print_file_line();
        } else {
            self.print_nl_s(b"! ");
        }
        self.print_s(s);
    }
    /// `print_ignored_err("abc")`, §73.
    pub fn print_ignored_err_s(&mut self, s: &[u8]) {
        self.old_selector_ignored_err = self.selector;
        self.selector = LOG_ONLY;
        self.wlog_cr();
        self.wlog_s(b"ignored: ");
        self.print_s(s);
        self.selector = self.old_selector_ignored_err;
    }
    /// `prompt_input("abc")`, §71.
    pub fn prompt_input_s(&mut self, s: &[u8]) {
        self.wake_up_terminal();
        self.print_s(s);
        self.term_input();
    }
    /// `help0` to `help6`, §79.  Lines are in reading order, as written in WEB: this stores them
    /// in the order `error` expects (`help_line[help_ptr-1]` is printed first).
    pub fn help(&mut self, lines: &[&'static [u8]]) {
        let n = lines.len();
        self.help_ptr = n as i32;
        for (i, l) in lines.iter().enumerate() {
            self.host.help_line[n - 1 - i] = l;
        }
    }
    /// `help_line[k]:="abc"`, §79.
    #[inline]
    pub fn set_help_line(&mut self, k: i32, s: &'static [u8]) {
        self.host.help_line[k as usize] = s;
    }
    /// `help_line[k]`, §79.
    #[inline]
    pub fn help_line(&self, k: i32) -> &'static [u8] {
        self.host.help_line[k as usize]
    }
    /// A new string with these bytes: a WEB literal that must be a string number at INITEX time.
    /// Not for use at run time, where a fresh string per call would leak; see `S_*` above.
    pub fn lit_string(&mut self, s: &[u8]) -> i32 {
        self.str_room(s.len() as i32);
        for &c in s {
            self.append_char(c as i32);
        }
        self.make_string()
    }
    /// §34.  The terminal is written through `Host`, which flushes here.
    #[inline]
    pub fn update_terminal(&mut self) {
        self.host.term_flush();
    }
    /// §34.
    #[inline(always)]
    pub fn clear_terminal(&mut self) {}
    /// §34.
    #[inline(always)]
    pub fn wake_up_terminal(&mut self) {}

    /// §93 `succumb`.
    pub fn succumb(&mut self) -> ! {
        if self.interaction == ERROR_STOP_MODE {
            self.interaction = SCROLL_MODE;
        }
        if self.log_opened {
            self.error();
        }
        self.history = FATAL_ERROR_STOP;
        self.jump_out();
    }
    /// §81 `do_final_end`.
    pub fn do_final_end(&mut self) -> ! {
        self.update_terminal();
        self.ready_already = 0;
        let code = if self.history != SPOTLESS && self.history != WARNING_ISSUED { 1 } else { 0 };
        self.uexit(code)
    }

    // ---- §105 arithmetic ----

    /// §105 `nx_plus_y(n,x,y)`.
    #[inline(always)]
    pub fn nx_plus_y(&mut self, n: i32, x: i32, y: i32) -> i32 {
        self.mult_and_add(n, x, y, 0o7777777777)
    }
    /// §105 `mult_integers(n,x)`.
    #[inline(always)]
    pub fn mult_integers(&mut self, n: i32, x: i32) -> i32 {
        self.mult_and_add(n, x, 0, 0o17777777777)
    }

    // ---- §140, §152 memory ----

    /// §152.  Compile-time split of lo and hi memory, DESIGN section 4.
    #[inline(always)]
    pub fn is_char_node(&self, p: i32) -> bool {
        p >= MEM_SPLIT
    }
    /// §140 `fast_get_avail(p)` is `let p = self.fast_get_avail()`.
    #[inline(always)]
    pub fn fast_get_avail(&mut self) -> i32 {
        let p = self.avail;
        if p == NULL {
            self.get_avail()
        } else {
            self.avail = self.link(p);
            self.set_link(p, NULL);
            self.dyn_used += 1;
            p
        }
    }

    // ---- §160 ----

    /// §160 `append_list(a)(b)`: `link(tail):=link(a); tail:=b`.
    #[inline(always)]
    pub fn append_list(&mut self, a: i32, b: i32) {
        let t = self.tail();
        self.set_link(t, self.link(a));
        self.set_tail(b);
    }

    // ---- §274 the hash ----

    /// §274 `text(p)`.  `hash` holds only names; `next(p)` no longer exists.
    #[inline(always)]
    pub fn text(&self, p: i32) -> i32 {
        self.hash[p]
    }
    /// §274 `text(p):=s`.
    #[inline(always)]
    pub fn set_text(&mut self, p: i32, s: i32) {
        self.hash[p] = s;
    }
    /// §274 `font_id_text(f)`.
    #[inline(always)]
    pub fn font_id_text(&self, f: i32) -> i32 {
        self.hash[FONT_ID_BASE + f]
    }
    /// §274 `font_id_text(f):=s`.
    #[inline(always)]
    pub fn set_font_id_text(&mut self, f: i32, s: i32) {
        self.hash[FONT_ID_BASE + f] = s;
    }

    // ---- §299 ----

    /// §299 `assign_trace(p,"reassigning")`.
    #[inline]
    pub fn assign_trace(&mut self, p: i32, s: &[u8]) {
        if self.tracing_assigns() > 0 {
            self.restore_trace(p, s);
        }
    }

    // ---- §320 ----

    /// §320 `chr_cmd("abc")` with its implicit `chr_code`: `self.chr_cmd(b"abc", chr_code)`.
    pub fn chr_cmd(&mut self, s: &[u8], chr_code: i32) {
        self.print_s(s);
        self.print(chr_code);
    }

    // ---- §439 ----

    /// §439 `scanned_result(v)(l)`.
    #[inline(always)]
    pub fn scanned_result(&mut self, v: i32, l: i32) {
        self.cur_val = v;
        self.cur_val_level = l;
    }

    // ---- §474 ----

    /// §474 `scan_normal_dimen`.
    #[inline(always)]
    pub fn scan_normal_dimen(&mut self) {
        self.scan_dimen(false, false, false);
    }

    // ---- §558 ----

    /// §558 `ensure_dvi_open`.
    pub fn ensure_dvi_open(&mut self) {
        if self.output_file_name == 0 {
            if self.job_name == 0 {
                self.open_log_file();
            }
            self.pack_job_name(S_DOT_DVI);
            while !self.b_open_out(DVI_FILE) {
                self.prompt_file_name(b"file name for output", S_DOT_DVI);
            }
            self.output_file_name = self.b_make_name_string(DVI_FILE);
        }
    }

    // ---- §580-584 font metrics.  WEB's two-step macros become two-argument methods. ----

    /// §580 `char_info(f)(c)`.  MLTeX is dropped, so this is `orig_char_info`.
    #[inline(always)]
    pub fn char_info(&self, f: i32, c: i32) -> FourQuarters {
        self.font_info[self.char_base[f] + c].qqqq()
    }
    /// §580 `orig_char_info(f)(c)`.
    #[inline(always)]
    pub fn orig_char_info(&self, f: i32, c: i32) -> FourQuarters {
        self.font_info[self.char_base[f] + c].qqqq()
    }
    /// §580 `char_width(f)(q)`.
    #[inline(always)]
    pub fn char_width(&self, f: i32, q: FourQuarters) -> i32 {
        self.font_info[self.width_base[f] + q.b0].sc()
    }
    /// §580 `char_italic(f)(q)`.
    #[inline(always)]
    pub fn char_italic(&self, f: i32, q: FourQuarters) -> i32 {
        self.font_info[self.italic_base[f] + q.b2 / 4].sc()
    }
    /// §580 `char_height(f)(hd)`, where `hd` is `height_depth(q)`.
    #[inline(always)]
    pub fn char_height(&self, f: i32, hd: i32) -> i32 {
        self.font_info[self.height_base[f] + hd / 16].sc()
    }
    /// §580 `char_depth(f)(hd)`, where `hd` is `height_depth(q)`.
    #[inline(always)]
    pub fn char_depth(&self, f: i32, hd: i32) -> i32 {
        self.font_info[self.depth_base[f] + hd % 16].sc()
    }
    /// §583 `char_kern(f)(q)`.
    #[inline(always)]
    pub fn char_kern(&self, f: i32, q: FourQuarters) -> i32 {
        self.font_info[self.kern_base[f] + 256 * q.b2 + q.b3].sc()
    }
    /// §583 `lig_kern_start(f)(q)`.
    #[inline(always)]
    pub fn lig_kern_start(&self, f: i32, q: FourQuarters) -> i32 {
        self.lig_kern_base[f] + q.b3
    }
    /// §583 `lig_kern_restart(f)(q)`.
    #[inline(always)]
    pub fn lig_kern_restart(&self, f: i32, q: FourQuarters) -> i32 {
        self.lig_kern_base[f] + 256 * q.b2 + q.b3 + 32768 - KERN_BASE_OFFSET
    }
    /// §584 `param(k)(f)`.
    #[inline(always)]
    pub fn param(&self, k: i32, f: i32) -> i32 {
        self.font_info[k + self.param_base[f]].sc()
    }
    /// §584 `param(k)(f):=v`.
    #[inline(always)]
    pub fn set_param(&mut self, k: i32, f: i32, v: i32) {
        let i = k + self.param_base[f];
        self.font_info[i].set_sc(v);
    }
    /// §584 `slant(f)`.
    #[inline(always)]
    pub fn slant(&self, f: i32) -> i32 {
        self.param(SLANT_CODE, f)
    }
    /// §584 `space(f)`.
    #[inline(always)]
    pub fn space(&self, f: i32) -> i32 {
        self.param(SPACE_CODE, f)
    }
    /// §584 `space_stretch(f)`.
    #[inline(always)]
    pub fn space_stretch(&self, f: i32) -> i32 {
        self.param(SPACE_STRETCH_CODE, f)
    }
    /// §584 `space_shrink(f)`.
    #[inline(always)]
    pub fn space_shrink(&self, f: i32) -> i32 {
        self.param(SPACE_SHRINK_CODE, f)
    }
    /// §584 `x_height(f)`.
    #[inline(always)]
    pub fn x_height(&self, f: i32) -> i32 {
        self.param(X_HEIGHT_CODE, f)
    }
    /// §584 `quad(f)`.
    #[inline(always)]
    pub fn quad(&self, f: i32) -> i32 {
        self.param(QUAD_CODE, f)
    }
    /// §584 `extra_space(f)`.
    #[inline(always)]
    pub fn extra_space(&self, f: i32) -> i32 {
        self.param(EXTRA_SPACE_CODE, f)
    }

    // ---- §653 ----

    /// §653 `round_glue`, with its implicit locals made explicit.  Sets the globals `g`, `rule_wd`.
    pub fn round_glue(
        &mut self,
        p: i32,
        this_box: i32,
        g_sign: i32,
        g_order: i32,
        cur_g: &mut i32,
        cur_glue: &mut f64,
    ) {
        self.g = self.glue_ptr(p);
        let g = self.g;
        self.rule_wd = self.width(g) - *cur_g;
        if g_sign != NORMAL {
            if g_sign == STRETCHING {
                if self.stretch_order(g) == g_order {
                    *cur_glue += self.stretch(g) as f64;
                    let glue_temp = vet_glue(self.glue_set(this_box) * *cur_glue);
                    *cur_g = round(glue_temp);
                }
            } else if self.shrink_order(g) == g_order {
                *cur_glue -= self.shrink(g) as f64;
                let glue_temp = vet_glue(self.glue_set(this_box) * *cur_glue);
                *cur_g = round(glue_temp);
            }
        }
        self.rule_wd += *cur_g;
    }

    // ---- §673 ----

    /// §673 `is_valid_char(c)` with its implicit `f`.
    #[inline]
    pub fn is_valid_char(&self, f: i32, c: i32) -> bool {
        self.font_bc[f] <= c && c <= self.font_ec[f] && self.char_exists(self.orig_char_info(f, c))
    }

    // ---- §680 the PDF buffer.  WEB's pdf_buf is a pointer; pdf_buf_is_os says which it is. ----

    /// §680 `pdf_quick_out(c)`.
    #[inline(always)]
    pub fn pdf_quick_out(&mut self, c: i32) {
        if self.pdf_buf_is_os {
            self.pdf_os_buf.put(self.pdf_ptr, c);
        } else {
            self.pdf_op_buf.put(self.pdf_ptr, c);
        }
        self.pdf_ptr += 1;
    }
    /// §680 `pdf_room(n)`.
    #[inline(always)]
    pub fn pdf_room(&mut self, n: i32) {
        if self.pdf_os_mode && n + self.pdf_ptr > self.pdf_buf_size {
            self.pdf_os_get_os_buf(n);
        } else if !self.pdf_os_mode && n > self.pdf_buf_size {
            self.overflow(b"PDF output buffer", PDF_OP_BUF_SIZE);
        } else if !self.pdf_os_mode && n + self.pdf_ptr > self.pdf_buf_size {
            self.pdf_flush();
        }
    }
    /// §680 `pdf_out(c)`.
    #[inline(always)]
    pub fn pdf_out(&mut self, c: i32) {
        self.pdf_room(1);
        self.pdf_quick_out(c);
    }
    /// `pdf_print("abc")`, §686.
    #[inline]
    pub fn pdf_print_s(&mut self, s: &[u8]) {
        for &c in s {
            self.pdf_out(c as i32);
        }
    }
    /// `pdf_print_ln("abc")`, §686.
    #[inline]
    pub fn pdf_print_ln_s(&mut self, s: &[u8]) {
        self.pdf_print_s(s);
        self.pdf_print_nl();
    }
    /// §686 `pdf_print_int_ln(n)`.
    #[inline]
    pub fn pdf_print_int_ln(&mut self, n: impl Into<i64>) {
        self.pdf_print_int(n);
        self.pdf_print_nl();
    }

    // ---- §816 ----

    /// §816 `hpack(p, natural)`.
    #[inline(always)]
    pub fn hpack_natural(&mut self, p: i32) -> i32 {
        self.hpack(p, 0, ADDITIONAL)
    }
    /// §844 `vpack(p, natural)`.
    #[inline(always)]
    pub fn vpack_natural(&mut self, p: i32) -> i32 {
        self.vpackage(p, 0, ADDITIONAL, MAX_DIMEN)
    }
    /// §844 `vpack(p, h, m)`.
    #[inline(always)]
    pub fn vpack(&mut self, p: i32, h: i32, m: i32) -> i32 {
        self.vpackage(p, h, m, MAX_DIMEN)
    }
    /// §823 `left_pw(p)`.
    #[inline(always)]
    pub fn left_pw(&mut self, p: i32) -> i32 {
        self.char_pw(p, LEFT_SIDE)
    }
    /// §823 `right_pw(p)`.
    #[inline(always)]
    pub fn right_pw(&mut self, p: i32) -> i32 {
        self.char_pw(p, RIGHT_SIDE)
    }

    // ---- §876 math parameters.  `s` is a size code, as in `axis_height(cur_size)`. ----

    /// §876 `mathsy(k)(s)`.
    #[inline(always)]
    pub fn mathsy(&self, k: i32, s: i32) -> i32 {
        self.font_info[k + self.param_base[self.fam_fnt(2 + s)]].sc()
    }
    /// §876.
    #[inline(always)]
    pub fn math_x_height(&self, s: i32) -> i32 {
        self.mathsy(5, s)
    }
    /// §876.
    #[inline(always)]
    pub fn math_quad(&self, s: i32) -> i32 {
        self.mathsy(6, s)
    }
    /// §876.
    #[inline(always)]
    pub fn num1(&self, s: i32) -> i32 {
        self.mathsy(8, s)
    }
    /// §876.
    #[inline(always)]
    pub fn num2(&self, s: i32) -> i32 {
        self.mathsy(9, s)
    }
    /// §876.
    #[inline(always)]
    pub fn num3(&self, s: i32) -> i32 {
        self.mathsy(10, s)
    }
    /// §876.
    #[inline(always)]
    pub fn denom1(&self, s: i32) -> i32 {
        self.mathsy(11, s)
    }
    /// §876.
    #[inline(always)]
    pub fn denom2(&self, s: i32) -> i32 {
        self.mathsy(12, s)
    }
    /// §876.
    #[inline(always)]
    pub fn sup1(&self, s: i32) -> i32 {
        self.mathsy(13, s)
    }
    /// §876.
    #[inline(always)]
    pub fn sup2(&self, s: i32) -> i32 {
        self.mathsy(14, s)
    }
    /// §876.
    #[inline(always)]
    pub fn sup3(&self, s: i32) -> i32 {
        self.mathsy(15, s)
    }
    /// §876.
    #[inline(always)]
    pub fn sub1(&self, s: i32) -> i32 {
        self.mathsy(16, s)
    }
    /// §876.
    #[inline(always)]
    pub fn sub2(&self, s: i32) -> i32 {
        self.mathsy(17, s)
    }
    /// §876.
    #[inline(always)]
    pub fn sup_drop(&self, s: i32) -> i32 {
        self.mathsy(18, s)
    }
    /// §876.
    #[inline(always)]
    pub fn sub_drop(&self, s: i32) -> i32 {
        self.mathsy(19, s)
    }
    /// §876.
    #[inline(always)]
    pub fn delim1(&self, s: i32) -> i32 {
        self.mathsy(20, s)
    }
    /// §876.
    #[inline(always)]
    pub fn delim2(&self, s: i32) -> i32 {
        self.mathsy(21, s)
    }
    /// §876.
    #[inline(always)]
    pub fn axis_height(&self, s: i32) -> i32 {
        self.mathsy(22, s)
    }

    // ---- §1392 assignments.  `a` is the prefix local of prefixed_command. ----

    /// §1392 `define(p,t,e)`.
    #[inline]
    pub fn define(&mut self, a: i32, p: i32, t: i32, e: i32) {
        if a >= 4 {
            self.geq_define(p, t, e);
        } else {
            self.eq_define(p, t, e);
        }
    }
    /// §1392 `word_define(p,w)`.
    #[inline]
    pub fn word_define(&mut self, a: i32, p: i32, w: i32) {
        if a >= 4 {
            self.geq_word_define(p, w);
        } else {
            self.eq_word_define(p, w);
        }
    }

    // ---- the POD arena, for what were C statics and `^` arrays; see PodVec in raw.rs ----

    /// A new vector of `T` in the image's arena, with room for `cap` elements.
    pub fn arena_vec<T: Pod>(&mut self, cap: i32) -> PodVec<T> {
        let words = (cap as usize * core::mem::size_of::<T>()).div_ceil(8) as i32;
        let start = self.arena_top;
        if start as usize + words as usize > ARENA_CAP {
            self.overflow(b"PDF arena size", ARENA_CAP as i32);
        }
        self.arena_top = start + words;
        PodVec::from_parts(start, cap)
    }
    /// Appends to a vector, doubling it when it is full.  The arena is a bump allocator and gives nothing back, so a
    /// vector that moves leaves its old place unused: doubling that way uses twice what the vector holds.  The last vector
    /// in the arena has nothing above it and grows in place, so one that grows alone, like `avl_nodes` under a million
    /// `\pdfobj`s, wastes nothing.
    pub fn arena_push<T: Pod>(&mut self, v: &mut PodVec<T>, x: T) {
        if v.len == v.cap {
            let cap = v.cap.max(8) * 2;
            let size = core::mem::size_of::<T>();
            let words = (v.cap as usize * size).div_ceil(8);
            if v.cap > 0 && v.start as usize + words == self.arena_top as usize {
                let top = v.start as usize + (cap as usize * size).div_ceil(8);
                if top > ARENA_CAP {
                    self.overflow(b"PDF arena size", ARENA_CAP as i32);
                }
                self.arena_top = top as i32;
                v.cap = cap;
            } else {
                let bigger = self.arena_vec::<T>(cap);
                let moved = v.moved_to(bigger, &mut self.arena);
                *v = moved;
            }
        }
        v.push_within_cap(&mut self.arena, x);
    }

    // ---- §726 ----

    /// §726 `output_one_char(c)` with its implicit `f`.
    pub fn output_one_char(&mut self, f: i32, c: i32) {
        if self.pdf_font_type[f] == NEW_FONT_TYPE {
            self.do_vf(f);
        }
        if self.pdf_font_type[f] == VIRTUAL_FONT_TYPE {
            self.do_vf_packet(f, c);
        } else {
            self.pdf_begin_string(f);
            self.pdf_print_char(f, c);
            self.adv_char_width(f, c);
        }
    }

    // ---- §1839 sparse array assignments.  `a` is the prefix local; `e` says sparse. ----

    /// §1839 `sa_define(sp,sv)(p,t,v)`.
    pub fn sa_define(&mut self, a: i32, e: bool, sp: i32, sv: i32, p: i32, t: i32, v: i32) {
        if e {
            if a >= 4 {
                self.gsa_def(sp, sv);
            } else {
                self.sa_def(sp, sv);
            }
        } else {
            self.define(a, p, t, v);
        }
    }
    /// §1839 `sa_word_define(p,w)`.
    pub fn sa_word_define(&mut self, a: i32, e: bool, p: i32, w: i32) {
        if e {
            if a >= 4 {
                self.gsa_w_def(p, w);
            } else {
                self.sa_w_def(p, w);
            }
        } else {
            self.word_define(a, p, w);
        }
    }
    /// §1839 `sa_def_box`.
    pub fn sa_def_box(&mut self, a: i32) {
        self.find_sa_element(BOX_VAL, self.cur_val, true);
        if a >= 4 {
            self.gsa_def(self.cur_ptr, self.cur_box);
        } else {
            self.sa_def(self.cur_ptr, self.cur_box);
        }
    }

    // ---- §1820 ----

    /// §1820 `fetch_box(p)` is `let p = self.fetch_box()`.
    pub fn fetch_box(&mut self) -> i32 {
        if self.cur_val < 256 {
            self.box_reg(self.cur_val)
        } else {
            self.find_sa_element(BOX_VAL, self.cur_val, false);
            if self.cur_ptr == NULL {
                NULL
            } else {
                self.sa_ptr(self.cur_ptr)
            }
        }
    }
}
