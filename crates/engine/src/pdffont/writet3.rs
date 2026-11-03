//! `writet3.c`: a font with no font file, written as Type 3, from a `.pgc` file if there is one, or else from a PK bitmap.  `kpse_find_pk` and
//! `mktexpk` are `crates/kpse`'s, and reading a PK file `crates/fontfile`'s `pk_read_char`.  The C's statics live for one `writet3`,
//! as `T3`.  A font comes here at the end of the run, so nothing of this is in the image.
use super::*;

/// `T3_BUF_SIZE`.
const T3_BUF_SIZE: usize = 1024;
const WRITET3_C: &[u8] = b"../../../texk/web2c/pdftexdir/writet3.c";
const T3_BEGIN_GLYPH_STR: &[u8] = b"\\pdfglyph";
const T3_END_GLYPH_STR: &[u8] = b"\\endglyph";
const T3_FONT_SCALE_STR: &[u8] = b"\\pdffontscale";

/// `t3_file`, `t3_line_array` and the rest of the statics.
struct T3 {
    file: Vec<u8>,
    pos: usize,
    /// `feof(t3_file)`: only once a read has met the end.
    eof: bool,
    line: Vec<u8>,
    image_used: bool,
    char_procs: [i32; 256],
    char_widths: [f32; 256],
    glyph_num: i32,
    font_scale: f32,
    /// `t3_b0`, `t3_b1`, `t3_b2`, `t3_b3`.
    b: [i32; 4],
    is_pk_font: bool,
}

impl T3 {
    /// `t3_getchar`.
    fn getchar(&mut self) -> i32 {
        match self.file.get(self.pos) {
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

    /// `t3_prefix`.
    fn prefix(&self, s: &[u8]) -> bool {
        self.line.starts_with(s)
    }

    /// `update_bbox`.
    fn update_bbox(&mut self, llx: i32, lly: i32, urx: i32, ury: i32) {
        if self.glyph_num == 0 {
            self.b = [llx, lly, urx, ury];
        } else {
            self.b[0] = self.b[0].min(llx);
            self.b[1] = self.b[1].min(lly);
            self.b[2] = self.b[2].max(urx);
            self.b[3] = self.b[3].max(ury);
        }
    }
}

impl Tex {
    /// `t3_getline`.
    fn writet3_getline(&mut self, t: &mut T3) {
        loop {
            t.line.clear();
            let mut c = t.getchar();
            while !t.eof {
                // append_char_to_buf
                if c == 9 {
                    c = 32;
                }
                if c == 13 {
                    c = 10;
                }
                if c != 32 || t.line.last().is_some_and(|&l| l != 32) {
                    t.line.push(c as u8);
                }
                if c == 10 {
                    break;
                }
                c = t.getchar();
            }
            // append_eol
            if t.line.len() + 2 > T3_BUF_SIZE {
                let m = [b"buffer overflow at file ".as_slice(), WRITET3_C, b", line 66"].concat();
                self.pdffont_fail(&m);
            }
            if t.line.len() > 1 && t.line.last() != Some(&10) {
                t.line.push(10);
            }
            let n = t.line.len();
            if n > 2 && t.line[n - 2] == 32 {
                t.line[n - 2] = 10;
                t.line.pop();
            }
            if (t.line.len() < 2 || t.line.first() == Some(&b'%')) && !t.eof {
                continue;
            }
            return;
        }
    }

    /// `t3_check_eof`.
    fn writet3_check_eof(&mut self, t: &T3) {
        if t.eof {
            self.pdffont_fail(b"unexpected end of file");
        }
    }

    /// `t3_write_glyph`.
    fn writet3_write_glyph(&mut self, t: &mut T3, f: i32) {
        self.writet3_getline(t);
        if !t.prefix(T3_BEGIN_GLYPH_STR) {
            return;
        }
        let args = t.line.get(T3_BEGIN_GLYPH_STR.len() + 1..).unwrap_or(b"");
        let Some([glyph_index, width, _height, _depth, llx, lly, urx, ury]) = ff::pgc_glyph_preamble(args) else {
            let mut shown = t.line.clone();
            if shown.last() == Some(&10) {
                shown.pop();
            }
            let m = [b"invalid glyph preamble: `".as_slice(), &shown, b"'"].concat();
            self.pdffont_fail(&m);
        };
        if glyph_index < self.font_bc[f] || glyph_index > self.font_ec[f] {
            return;
        }
        if !self.pdf_char_marked(f, glyph_index) {
            while !t.prefix(T3_END_GLYPH_STR) {
                self.writet3_check_eof(t);
                self.writet3_getline(t);
            }
            return;
        }
        t.update_bbox(llx, lly, urx, ury);
        t.glyph_num += 1;
        self.pdf_new_dict(0, 0, 0);
        let g = glyph_index as usize;
        t.char_procs[g] = self.obj_ptr;
        t.char_widths[g] = if width == 0 {
            (self.char_width(f, self.char_info(f, glyph_index)) as f32 / t.font_scale) / self.pdf_font_size[f] as f32
        } else {
            width as f32
        };
        self.pdf_begin_stream();
        self.writet3_getline(t);
        let w = t.char_widths[g] as i32;
        self.pdffont_pdf_printf(&[
            &int_bytes(w),
            b" 0 ",
            &int_bytes(llx),
            b" ",
            &int_bytes(lly),
            b" ",
            &int_bytes(urx),
            b" ",
            &int_bytes(ury),
            b" d1\nq\n",
        ]);
        while !t.prefix(T3_END_GLYPH_STR) {
            self.writet3_check_eof(t);
            if t.prefix(b"BI") {
                t.image_used = true;
            }
            for i in 0..t.line.len() {
                self.pdf_out(t.line[i] as i32);
            }
            self.writet3_getline(t);
        }
        self.pdffont_pdf_puts(b"Q\n");
        self.pdf_end_stream();
    }

    /// `get_pk_font_scale`.
    fn writet3_pk_font_scale(&mut self, f: i32) -> i32 {
        let a = self.divide_scaled(self.pdf_font_size[f], self.one_hundred_bp, self.fixed_decimal_digits + 2);
        self.divide_scaled(self.pk_scale_factor, a, 0)
    }

    /// `pk_char_width`.
    fn writet3_pk_char_width(&mut self, f: i32, w: i32) -> i32 {
        let scale = self.writet3_pk_font_scale(f);
        let a = self.divide_scaled(w, self.pdf_font_size[f], 7);
        self.divide_scaled(a, scale, 0)
    }

    /// External: `getpkcharwidth`, `writet3.c`.
    pub fn get_pk_char_width(&mut self, f: i32, w: i32) -> i32 {
        let scale = self.writet3_pk_font_scale(f);
        let width = self.writet3_pk_char_width(f, w);
        ((scale as f64 / 100000.0) * (width as f64 / 100.0) * self.pdf_font_size[f] as f64) as i32
    }

    /// `writepk`.
    fn writet3_writepk(&mut self, t: &mut T3, f: i32) {
        let d = self.fixed_pk_resolution as f32 * (self.pdf_font_size[f] as f32 / self.font_dsize[f] as f32);
        let (dpi, _) = rstex_kpse::magstep_fix(round(d as f64) as u32, self.fixed_pk_resolution as u32);
        let dpi = dpi as i32;
        let name = self.pdffont_make_cstring(self.font_name[f]);
        self.host.hv.pdffont.cur_file_name = Some(name.clone());
        self.update_terminal();
        let discard = self.v_pdffont.make_tex_discard_errors;
        let found = self.host.world.find_pk(&name, dpi, discard).filter(|(_, g)| {
            rstex_kpse::names::file_str_eq(&name, &g.name) && rstex_kpse::bitmap_tolerance(g.dpi as f32 as f64, dpi as f32 as f64)
        });
        let Some((path, _)) = found else {
            let m = [b"Font ".as_slice(), &name, b" at ", &int_bytes(dpi), b" not found"].concat();
            self.pdffont_fail(&m);
        };
        let Some(data) = self.host.world.read(&path) else {
            self.pdffont_fail(b"fopen failed");
        };
        t.image_used = true;
        t.is_pk_font = true;
        self.pdffont_tex_printf(&[b" <", &path]);
        let mut cd = ff::CharDesc::default();
        let mut check_preamble = true;
        loop {
            match ff::pk_read_char(&data, &mut t.pos, check_preamble, &mut cd) {
                Ok(true) => {}
                Ok(false) => break,
                Err(e) => self.pdffont_font_error(e),
            }
            check_preamble = false;
            // A code beyond a byte is a bad file, whose bit the C reads from whatever comes after `pdf_char_used[f]`.
            if !(0..256).contains(&cd.charcode) || !self.pdf_char_marked(f, cd.charcode) {
                continue;
            }
            let c = cd.charcode as usize;
            let w = self.char_width(f, self.char_info(f, cd.charcode));
            t.char_widths[c] = self.writet3_pk_char_width(f, w) as f32;
            let is_null_glyph = cd.cwidth < 1 || cd.cheight < 1;
            if is_null_glyph {
                cd.cwidth = round(t.char_widths[c] as f64 / 100.0);
                cd.xescape = cd.cwidth;
                cd.cheight = 1;
                cd.xoff = 0;
                cd.yoff = 0;
            }
            let llx = -cd.xoff;
            let lly = cd.yoff - cd.cheight + 1;
            let urx = cd.cwidth + llx + 1;
            let ury = cd.cheight + lly;
            t.update_bbox(llx, lly, urx, ury);
            t.glyph_num += 1;
            self.pdf_new_dict(0, 0, 0);
            t.char_procs[c] = self.obj_ptr;
            self.pdf_begin_stream();
            self.pdf_print_real(t.char_widths[c] as i32, 2);
            self.pdffont_pdf_printf(&[
                b" 0 ",
                &int_bytes(llx),
                b" ",
                &int_bytes(lly),
                b" ",
                &int_bytes(urx),
                b" ",
                &int_bytes(ury),
                b" d1\n",
            ]);
            if !is_null_glyph {
                let (cwidth, cheight) = (int_bytes(cd.cwidth), int_bytes(cd.cheight));
                self.pdffont_pdf_printf(&[
                    b"q\n",
                    &cwidth,
                    b" 0 0 ",
                    &cheight,
                    b" ",
                    &int_bytes(llx),
                    b" ",
                    &int_bytes(lly),
                    b" cm\nBI\n",
                ]);
                self.pdffont_pdf_printf(&[b"/W ", &cwidth, b"\n/H ", &cheight, b"\n"]);
                self.pdffont_pdf_puts(b"/IM true\n/BPC 1\n/D [1 0]\nID ");
                let cw = (cd.cwidth + 7) / 8;
                let rw = (cd.cwidth + 15) / 16;
                let mut row = 0;
                for _ in 0..cd.cheight {
                    for _ in 0..rw - 1 {
                        self.pdf_out(cd.raster[row] / 256);
                        self.pdf_out(cd.raster[row] % 256);
                        row += 1;
                    }
                    self.pdf_out(cd.raster[row] / 256);
                    if 2 * rw == cw {
                        self.pdf_out(cd.raster[row] % 256);
                    }
                    row += 1;
                }
                self.pdffont_pdf_puts(b"\nEI\nQ\n");
            }
            self.pdf_end_stream();
        }
        self.host.hv.pdffont.cur_file_name = None;
    }

    /// `remove_duplicate_glyph_names`: the duplicates render the output invalid.  We give a warning, since this should only happen with
    /// ill-defined .enc files that should get fixed.
    fn writet3_remove_duplicates(&mut self, fe: i32, names: &mut GlyphNames, encname: &[u8]) {
        let mut seen: Vec<Vec<u8>> = Vec::with_capacity(256);
        let mut changed = false;
        for i in 0..256 {
            if names.is_notdef(i) {
                continue;
            }
            let g = names.name(i).to_vec();
            if seen.contains(&g) {
                let m = [encname, b": duplicate glyph name at position ", &int_bytes(i as i32), b": ", &g].concat();
                self.pdffont_warn(&m);
                names.clear(i);
                changed = true;
            } else {
                seen.push(g);
            }
        }
        if changed {
            self.pdffont_fe_set_glyph_names(fe, names);
        }
    }

    /// `writet3`.
    pub(super) fn pdffont_writet3(&mut self, fm_i: Option<i32>, objnum: i32, f: i32) {
        let mut t = T3 {
            file: Vec::new(),
            pos: 0,
            eof: false,
            line: Vec::new(),
            image_used: false,
            char_procs: [0; 256],
            char_widths: [0.0; 256],
            glyph_num: 0,
            font_scale: 0.0,
            b: [0; 4],
            is_pk_font: false,
        };
        let fm = fm_i.map(|i| self.pdffont_fm(i));
        let fe = match &fm {
            Some(r) if r.is_reencoded() => Some(self.pdffont_get_fe_entry(r.encname)),
            _ => None,
        };
        let mut glyph_names = fe.map(|e| self.pdffont_fe_glyph_names(e));
        if let (Some(e), Some(names), Some(r)) = (fe, &mut glyph_names, &fm) {
            let encname = self.pdffont_bytes(r.encname);
            self.writet3_remove_duplicates(e, names, &encname);
        }
        let ext = self.pdffont_make_tex_string(b".pgc");
        self.pack_file_name(self.font_name[f], S_EMPTY, ext);
        let s = self.make_name_string();
        let cur = self.pdffont_make_cstring(s);
        self.host.hv.pdffont.cur_file_name = Some(cur);
        match self.pdffont_open_input(rstex_kpse::KPSE_MISCFONTS_FORMAT) {
            None => self.writet3_writepk(&mut t, f),
            Some(data) => {
                t.file = data;
                let shown = self.name_of_file_bytes();
                self.pdffont_tex_printf(&[b"<", &shown]);
                self.writet3_getline(&mut t);
                let scale = if t.prefix(T3_FONT_SCALE_STR) {
                    ff::pgc_font_scale(t.line.get(T3_FONT_SCALE_STR.len() + 1..).unwrap_or(b""))
                } else {
                    None
                };
                match scale {
                    Some(v) if v > 0.0 && v <= 1000.0 => t.font_scale = v,
                    _ => self.pdffont_fail(b"missing or invalid font scale"),
                }
                while !t.eof {
                    self.writet3_write_glyph(&mut t, f);
                }
            }
        }
        // write_font_dict:
        let mut i = self.font_bc[f];
        while i <= self.font_ec[f] {
            if self.pdf_char_marked(f, i) {
                break;
            }
            i += 1;
        }
        let first_char = i;
        i = self.font_ec[f];
        while i > first_char {
            if self.pdf_char_marked(f, i) {
                break;
            }
            i -= 1;
        }
        let last_char = i;
        // write ToUnicode entry if we can
        let mut tounicode_objnum = 0;
        if self.fixed_gen_tounicode > 0 && !self.pdf_font_nobuiltin_tounicode[f] {
            if let (Some(e), Some(names), Some(r)) = (fe, &glyph_names, &fm) {
                let tfm_name = self.pdffont_bytes(r.tfm_name);
                let rec = self.v_pdffont.fe_tab.get(&mut self.arena, e);
                let enc = self.pdffont_bytes(rec.name);
                tounicode_objnum = self.pdffont_write_tounicode(names, &tfm_name, Some(&enc));
            }
        }
        self.pdf_begin_dict(objnum, 1); // Type 3 font dictionary
        self.pdffont_pdf_puts(b"/Type /Font\n/Subtype /Type3\n");
        self.pdffont_pdf_printf(&[b"/Name /F", &int_bytes(f), b"\n"]);
        if self.pdf_font_attr[f] != S_EMPTY {
            self.pdf_print(self.pdf_font_attr[f]);
            self.pdffont_pdf_puts(b"\n");
        }
        if t.is_pk_font {
            let pk_font_scale = self.writet3_pk_font_scale(f);
            self.pdffont_pdf_puts(b"/FontMatrix [");
            self.pdf_print_real(pk_font_scale, 5);
            self.pdffont_pdf_puts(b" 0 0 ");
            self.pdf_print_real(pk_font_scale, 5);
            self.pdffont_pdf_puts(b" 0 0]\n");
        } else {
            let g = ff::pgc_print_g(t.font_scale);
            self.pdffont_pdf_printf(&[b"/FontMatrix [", &g, b" 0 0 ", &g, b" 0 0]\n"]);
        }
        self.pdffont_pdf_printf(&[
            b"/FontBBox [ ",
            &int_bytes(t.b[0]),
            b" ",
            &int_bytes(t.b[1]),
            b" ",
            &int_bytes(t.b[2]),
            b" ",
            &int_bytes(t.b[3]),
            b" ]\n",
        ]);
        self.pdffont_pdf_printf(&[b"/Resources << /ProcSet [ /PDF ", if t.image_used { b"/ImageB " } else { b"" }, b"] >>\n"]);
        self.pdffont_pdf_printf(&[b"/FirstChar ", &int_bytes(first_char), b"\n/LastChar ", &int_bytes(last_char), b"\n"]);
        let wptr = self.pdf_new_objnum();
        let eptr = self.pdf_new_objnum();
        let cptr = self.pdf_new_objnum();
        self.pdffont_pdf_printf(&[
            b"/Widths ",
            &int_bytes(wptr),
            b" 0 R\n/Encoding ",
            &int_bytes(eptr),
            b" 0 R\n/CharProcs ",
            &int_bytes(cptr),
            b" 0 R\n",
        ]);
        if tounicode_objnum != 0 {
            self.pdffont_pdf_printf(&[b"/ToUnicode ", &int_bytes(tounicode_objnum), b" 0 R\n"]);
        }
        self.pdf_end_dict();
        self.pdf_begin_obj(wptr, 1); // chars width array
        self.pdffont_pdf_puts(b"[");
        for i in first_char..=last_char {
            if t.is_pk_font {
                self.pdf_print_real(t.char_widths[i as usize] as i32, 2);
                self.pdffont_pdf_puts(b" ");
            } else {
                self.pdffont_pdf_printf(&[&int_bytes(t.char_widths[i as usize] as i32), b" "]);
            }
        }
        self.pdffont_pdf_puts(b"]\n");
        self.pdf_end_obj();
        let glyph = |names: &Option<GlyphNames>, i: i32| -> Vec<u8> {
            match names {
                Some(n) if !n.is_notdef(i as usize) => [b"/".as_slice(), n.name(i as usize)].concat(),
                _ => [b"/a".as_slice(), &int_bytes(i)].concat(),
            }
        };
        self.pdf_begin_dict(eptr, 1); // encoding dictionary
        self.pdffont_pdf_printf(&[b"/Type /Encoding\n/Differences [", &int_bytes(first_char)]);
        let mut is_notdef;
        if t.char_procs[first_char as usize] == 0 {
            self.pdffont_pdf_printf(&[b"/.notdef"]);
            is_notdef = true;
        } else {
            self.pdffont_pdf_printf(&[&glyph(&glyph_names, first_char)]);
            is_notdef = false;
        }
        for i in first_char + 1..=last_char {
            if t.char_procs[i as usize] == 0 {
                if !is_notdef {
                    self.pdffont_pdf_printf(&[b" ", &int_bytes(i), b"/.notdef"]);
                    is_notdef = true;
                }
            } else {
                if is_notdef {
                    self.pdffont_pdf_printf(&[b" ", &int_bytes(i)]);
                    is_notdef = false;
                }
                self.pdffont_pdf_printf(&[&glyph(&glyph_names, i)]);
            }
        }
        self.pdffont_pdf_puts(b"]\n");
        self.pdf_end_dict();
        self.pdf_begin_dict(cptr, 1); // CharProcs dictionary
        for i in first_char..=last_char {
            if t.char_procs[i as usize] != 0 {
                self.pdffont_pdf_printf(&[&glyph(&glyph_names, i), b" ", &int_bytes(t.char_procs[i as usize]), b" 0 R\n"]);
            }
        }
        self.pdf_end_dict();
        self.pdffont_tex_printf(&[b">"]);
        self.host.hv.pdffont.cur_file_name = None;
    }
}
