//! What the `commute` command asks of the engine (DESIGN.md section 10): diagrams from a file, each to a cropped PDF and an SVG, through the native path.
//!
//! The command gives a [`Config`] and runs the engine as `rstex` would, with `\commutesession` as its first line, or at the end of a preamble.  That control sequence is armed, as `\tikzcd`
//! is, and `session_step` is what its fetch does.  So all of this happens with the Rust stack as `main_control` has it, and cells and labels go through the main loop like any others.  Each diagram
//! is a file in memory holding `\tikzcd`, the diagram and `\commutesession`: `native_read` finds its way there as it does in a document, and the next fetch of `\commutesession` finds a box.
//!
//! # Two ways
//!
//! **With the library only** ([`Config::library_only`]).  The format has no tikz-cd, only LaTeX, the math packages and a `\tikzcd` that holds nothing, so that nothing is left to compare and `native_guards`
//! does not run.  What the path declines is said and left out.  Since a format's PDF state is empty, one engine serves the whole file: `session_new_pdf` makes it as new after each PDF.  That is
//! why diagrams of one file share what a cell assigns globally.
//!
//! **With real tikz-cd**, under a preamble.  One diagram per run: `\setbox\commutebox\hbox{\begin{tikzcd}...\end{tikzcd}}`, then `\commutesession` to set the page's size, then a real `\shipout` and
//! whatever `\end{document}` does.  What a package reserves in the PDF at load, as PGF does, is the run's to finish.  What the path declines, real PGF draws, with no SVG.
//!
//! # SVG
//!
//! `commute::svg` draws the strokes.  What is in the boxes is walked here as `pdf_hlist_out` and `pdf_vlist_out` walk them, in the same scaled points and with the same rounding of glue and leaders, and each
//! character becomes its outline from the font file.  It happens before the boxes are shipped, which frees them.  A font that is not Type 1, an image and a `\pdfliteral` have no outline, and so no SVG: the
//! PDF is still written, and the diagram counts as not done.
#![forbid(unsafe_code)]
#![deny(unreachable_patterns, unused_variables)]
#![allow(non_snake_case, unused_mut, unused_imports, dead_code)]

use crate::access::*;
use crate::consts::*;
use crate::host::*;
use crate::native::CD_ARMED;
use crate::raw::*;
use crate::state::*;
use commute::svg::{self, SvgText};
use commute::{BoxRef, Color, Diagram, Matrix, Pt};
use rstex_fontfile::{GlyphNames, OutlineSeg, T1Outlines};
use std::rc::Rc;

/// The first line of a run that has no preamble, and what a driver's preamble ends with.
pub const SESSION_CS: &[u8] = b"commutesession";
/// The box a diagram ends up in: `\newbox`, by the format or the driver.
const BOX_CS: &[u8] = b"commutebox";

/// One diagram.
pub struct Job {
    /// What stands after `\begin{tikzcd}`, up to its `\end{tikzcd}`.
    pub body: Vec<u8>,
    /// Where the output goes, without `.pdf` or `.svg`.
    pub out: Vec<u8>,
    /// The line of the input file it starts on.
    pub line: i32,
}

/// What became of a diagram.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Not reached.
    Pending,
    Written,
    /// The library or `native_vet` said no, for this reason.  Nothing is written.
    Declined(String),
    /// TeX reported an error, this one.  Nothing is written.
    Failed(String),
    /// The PDF is written; there is no SVG, for this reason.
    NoSvg(String),
}

/// What the command asks.
pub struct Config {
    pub jobs: Vec<Job>,
    pub pdf: bool,
    pub svg: bool,
    /// Say on standard output how big each box is.
    pub print_box: bool,
    /// No real tikz-cd in the format.  Otherwise there is one job, under a preamble.
    pub library_only: bool,
    /// Keys as in `\tikzcdset{...}`, before every diagram, for a run with the library only: a preamble says that for itself.
    pub preset: Vec<u8>,
    /// The end of a run with the library only: what became of them all, and the status to exit with.
    pub finish: Option<Box<dyn FnOnce(&[Job], &[Outcome]) -> i32>>,
}

/// `outer` after `inner`.
fn compose(o: &Matrix, i: &Matrix) -> Matrix {
    Matrix {
        a: o.a * i.a + o.c * i.b,
        b: o.b * i.a + o.d * i.b,
        c: o.a * i.c + o.c * i.d,
        d: o.b * i.c + o.d * i.d,
        e: o.a * i.e + o.c * i.f + o.e,
        f: o.b * i.e + o.d * i.f + o.f,
    }
}

/// §103 `print_scaled`, as text.
fn scaled_text(s: i32) -> String {
    let mut out = String::new();
    let mut s = i64::from(s);
    if s < 0 {
        out.push('-');
        s = -s;
    }
    out.push_str(&(s / 65536).to_string());
    out.push('.');
    s = 10 * (s % 65536) + 5;
    let mut delta = 10;
    loop {
        if delta > 65536 {
            s = s + 32768 - 50000;
        }
        out.push(char::from(b'0' + (s / 65536) as u8));
        s = 10 * (s % 65536);
        delta *= 10;
        if s <= delta {
            break;
        }
    }
    out
}

/// A font's outlines, as far as asked for.
struct SvgFont {
    outlines: T1Outlines,
    /// The map's encoding, if it has one.
    names: Option<GlyphNames>,
    /// `SlantFont` and `ExtendFont`, in thousandths, 0 for none.
    slant: i64,
    extend: i64,
    glyphs: Vec<Option<Box<[OutlineSeg]>>>,
}

/// Non-POD state.
#[derive(Default)]
pub struct HVars {
    pub config: Option<Config>,
    outcomes: Vec<Outcome>,
    next: usize,
    /// The job in progress.
    current: Option<usize>,
    /// The file the jobs share as their name, a string once made.
    file_name: i32,
    /// What the SVG of the current diagram came to, or why it did not.
    svg: Vec<u8>,
    svg_failed: Option<String>,
    /// The native path drew it.
    native: bool,
    /// For each font number, its place in `fonts` plus one, or 0.
    font_slot: Vec<u32>,
    fonts: Vec<SvgFont>,
    /// Font files as read: the SVG and each PDF of a run want them all over again.
    font_files: Vec<(Vec<u8>, Rc<Vec<u8>>)>,
}

/// Where the walk of a box is and what it draws with.
struct Walk<'a> {
    out: &'a mut Vec<u8>,
    /// Box coordinates in points, y down, to the picture's.
    m: Matrix,
    /// The colours on the stack, the top last.
    colors: Vec<[u8; 7]>,
    open: bool,
    /// What `\pdfsave` saved.
    saved: Vec<Matrix>,
    failed: Option<String>,
}

impl Walk<'_> {
    fn begin_path(&mut self) {
        if !self.open {
            self.open = true;
            let c = *self.colors.last().expect("the box's own colour is at the bottom");
            self.out.extend_from_slice(b"<path fill=\"");
            self.out.extend_from_slice(&c);
            self.out.extend_from_slice(b"\" d=\"");
        } else {
            self.out.push(b' ');
        }
    }

    fn end_path(&mut self) {
        if self.open {
            self.open = false;
            self.out.extend_from_slice(b"\"/>\n");
        }
    }

    fn set_color(&mut self, c: [u8; 7]) {
        if self.colors.last() != Some(&c) {
            self.end_path();
        }
        *self.colors.last_mut().expect("the box's own colour is at the bottom") = c;
    }

    fn push_color(&mut self, c: [u8; 7]) {
        if self.colors.last() != Some(&c) {
            self.end_path();
        }
        self.colors.push(c);
    }

    fn pop_color(&mut self) {
        if self.colors.len() > 1 {
            let c = self.colors.pop().expect("more than one");
            if self.colors.last() != Some(&c) {
                self.end_path();
            }
        }
    }

    fn point(&mut self, x: f64, y: f64) {
        let p = self.m.apply(Pt::new(x, y));
        svg::number(self.out, p.x);
        self.out.push(b' ');
        svg::number(self.out, p.y);
    }

    /// A rule: its left, its bottom on the baseline's side, in scaled points, y down.
    fn rule(&mut self, x: i32, y: i32, w: i32, h: i32) {
        let (x0, x1) = (f64::from(x) / 65536.0, (f64::from(x) + f64::from(w)) / 65536.0);
        let (y0, y1) = (f64::from(y) / 65536.0, (f64::from(y) - f64::from(h)) / 65536.0);
        self.begin_path();
        self.out.extend_from_slice(b"M ");
        self.point(x0, y0);
        self.out.extend_from_slice(b" L ");
        self.point(x1, y0);
        self.out.extend_from_slice(b" L ");
        self.point(x1, y1);
        self.out.extend_from_slice(b" L ");
        self.point(x0, y1);
        self.out.extend_from_slice(b" Z");
    }
}

/// `svg::SvgText`, on real boxes.
struct SvgBoxes<'a> {
    t: &'a mut Tex,
    cells: &'a [i32],
    labels: &'a [i32],
}

impl SvgText for SvgBoxes<'_> {
    fn place_box(&mut self, out: &mut Vec<u8>, which: BoxRef, transform: &Matrix, fill: &str) {
        let b = match which {
            BoxRef::Cell(i) => self.cells.get(i as usize).copied(),
            BoxRef::Label(i) => self.labels.get(i as usize).copied(),
        };
        let Some(b) = b.filter(|&b| b != NULL) else { return };
        let mut color = [0u8; 7];
        color.copy_from_slice(&fill.as_bytes()[..7]);
        let mut w = Walk { out, m: *transform, colors: vec![color], open: false, saved: Vec::new(), failed: None };
        if self.t.ty(b) == VLIST_NODE {
            self.t.session_vlist(&mut w, b, 0, 0);
        } else {
            self.t.session_hlist(&mut w, b, 0, 0);
        }
        w.end_path();
        if let Some(why) = w.failed {
            self.t.host.hv.session.svg_failed.get_or_insert(why);
        }
    }
}

impl Tex {
    /// What the command gives, before the engine starts.
    pub fn session_configure(&mut self, config: Option<Config>) {
        if config.is_some() {
            self.host.term_capture = Some(Vec::new());
        }
        self.host.hv.session.config = config;
    }

    #[inline]
    pub(crate) fn session_on(&self) -> bool {
        self.host.hv.session.config.is_some()
    }

    #[inline]
    pub(crate) fn session_library_only(&self) -> bool {
        self.host.hv.session.config.as_ref().is_some_and(|c| c.library_only)
    }

    /// §1517 has loaded the format.  Each run makes the name, so that a format needs to know nothing of it.
    #[cold]
    pub fn session_arm(&mut self) {
        let n = SESSION_CS.len() as i32;
        let at = self.first;
        for (k, &c) in SESSION_CS.iter().enumerate() {
            self.buffer.put(at + k as i32, c as i32);
        }
        let keep = self.no_new_control_sequence;
        self.no_new_control_sequence = false;
        let cs = self.id_lookup(at, n);
        self.no_new_control_sequence = keep;
        self.eqtb[cs] = Word::hb(256, RELAX + CD_ARMED, LEVEL_ONE);
        self.v_image.cd.session_cs = cs;
        if self.session_library_only() {
            self.v_image.cd.on = true;
            self.v_image.cd.report_names = true;
            self.native_after_prefixed();
            self.pdfout_small_files();
        }
    }

    /// `\commutesession` was fetched.
    #[cold]
    pub fn session_step(&mut self) {
        if !self.session_on() || !self.v_image.ck.at_big_switch || self.expand_depth_count != 0 || self.scanner_status != NORMAL {
            return;
        }
        if self.host.hv.session.outcomes.is_empty() {
            let n = self.host.hv.session.config.as_ref().map_or(0, |c| c.jobs.len());
            self.host.hv.session.outcomes = vec![Outcome::Pending; n];
            self.session_first();
        }
        if self.session_library_only() {
            if self.host.hv.session.current.is_some() {
                self.session_library_done();
            }
            self.session_next();
        } else if self.host.hv.session.current.is_some() {
            self.session_page_size();
        } else {
            self.session_next();
        }
        // A `\\write` at the shipout has been through `scan_toks`, and this is still a fetch of `\\relax` to whoever made it.
        self.cur_cmd = RELAX;
        self.cur_chr = 256;
        self.cur_cs = self.v_image.cd.session_cs;
    }

    /// What the first diagram needs: a job name that opens no log, and the name of what is read.
    fn session_first(&mut self) {
        let cs = self.find_cs(BOX_CS);
        if cs != UNDEFINED_CONTROL_SEQUENCE {
            self.v_image.cd.session_box = self.equiv(cs);
        }
        if self.job_name == 0 && self.session_library_only() {
            self.job_name = self.session_string(b"commute");
        }
        self.host.hv.session.file_name = self.session_string(b"commute");
    }

    fn session_string(&mut self, s: &[u8]) -> i32 {
        for &c in s {
            self.str_pool.put(self.pool_ptr, c as i32);
            self.pool_ptr += 1;
        }
        self.make_string()
    }

    /// Opens the next diagram, or if there is none ends a run with the library only.
    fn session_next(&mut self) {
        let k = self.host.hv.session.next;
        let n = self.host.hv.session.config.as_ref().map_or(0, |c| c.jobs.len());
        let library_only = self.session_library_only();
        if k >= n {
            if library_only {
                self.session_end();
            }
            return;
        }
        self.host.hv.session.next = k + 1;
        self.host.hv.session.current = Some(k);
        self.host.hv.session.native = false;
        self.host.hv.session.svg.clear();
        self.host.hv.session.svg_failed = None;
        if library_only {
            self.history = SPOTLESS;
            self.error_count = 0;
        }
        self.native_session_reset();
        let mut data = Vec::new();
        let line;
        {
            let c = self.host.hv.session.config.as_ref().expect("session_on");
            let job = &c.jobs[k];
            line = job.line;
            data.reserve(job.body.len() + 64);
            if library_only {
                data.extend_from_slice(b"\\tikzcd ");
            } else {
                data.extend_from_slice(b"\\setbox\\commutebox\\hbox{\\begin{tikzcd}");
            }
            data.extend_from_slice(&job.body);
            // A comment on the last line would take the end with it.
            let last = job.body.rsplit(|&c| c == b'\n').next().unwrap_or(&[]);
            if last.contains(&b'%') {
                data.push(b'\n');
            }
            if library_only {
                data.extend_from_slice(b"\\endtikzcd\\commutesession\n");
            } else {
                data.extend_from_slice(b"\\end{tikzcd}}\\commutesession\n\\shipout\\box\\commutebox\n");
            }
        }
        self.session_open(data, line);
    }

    /// A file in memory, on the input stack as `start_input` leaves one.
    fn session_open(&mut self, data: Vec<u8>, line: i32) {
        self.begin_file_reading();
        let name = self.host.hv.session.file_name;
        let f = self.cur_file();
        self.host.files[f as usize] = Slot::In(InFile { path: b"commute".to_vec(), data, pos: 0 });
        self.set_name(name);
        self.source_filename_stack[self.in_open] = name;
        self.full_source_filename_stack[self.in_open] = name;
        self.open_parens += 1;
        self.set_state(NEW_LINE);
        // §564 Read the first line of the new file
        self.line = line;
        let _ = self.input_ln(f, false);
        self.firm_up_the_line();
        if self.end_line_char_inactive() {
            self.set_limit(self.limit() - 1);
        } else {
            self.buffer.put(self.limit(), self.end_line_char());
        }
        self.first = self.limit() + 1;
        self.set_loc(self.start());
    }

    fn session_close_file(&mut self) {
        self.end_file_reading();
        self.open_parens -= 1;
    }

    /// What the terminal has been told since the last time, as one line: its first error, or nothing.
    fn session_error(&mut self) -> Option<String> {
        self.host.term_flush();
        let text = self.host.term_capture.take()?;
        self.host.term_capture = Some(Vec::with_capacity(text.capacity()));
        if self.history < ERROR_MESSAGE_ISSUED {
            return None;
        }
        let line = text.split(|&c| c == b'\n').find(|l| l.starts_with(b"! "));
        Some(line.map_or_else(|| "an error".to_string(), |l| String::from_utf8_lossy(l).into_owned()))
    }

    /// `native_try` said no.  With the library only that is the end of this diagram.
    pub(crate) fn session_declined(&mut self, why: String) {
        if !self.session_library_only() {
            return;
        }
        if let Some(k) = self.host.hv.session.current.take() {
            self.host.hv.session.outcomes[k] = Outcome::Declined(why);
        }
        self.session_close_file();
        self.session_next();
    }

    /// The diagram has reached its `\commutesession`, with the library only.
    fn session_library_done(&mut self) {
        let Some(k) = self.host.hv.session.current.take() else { return };
        let b = self.box_reg(self.v_image.cd.session_box);
        self.set_box_reg(self.v_image.cd.session_box, NULL);
        self.session_close_file();
        let error = self.session_error();
        let in_order = self.cur_level == LEVEL_ONE && self.nest_ptr == 0 && !self.v_image.cd.active;
        let outcome = if let Some(e) = error {
            if b != NULL {
                self.flush_node_list(b);
            }
            Outcome::Failed(e)
        } else if b == NULL || !in_order {
            if b != NULL {
                self.flush_node_list(b);
            }
            Outcome::Failed("TeX did not get to the end of the diagram".to_string())
        } else {
            self.session_write(k, b)
        };
        self.host.hv.session.outcomes[k] = outcome;
        if !in_order {
            // No later diagram could trust what is left.
            self.session_end();
        }
    }

    /// `--box`: width, height and depth as `\\the` prints them, of what a document would get.
    fn session_print_box(&mut self, k: usize, w: i32, h: i32, d: i32) {
        let Some(c) = self.host.hv.session.config.as_ref().filter(|c| c.print_box) else { return };
        let mut line = c.jobs[k].out.clone();
        line.extend_from_slice(format!(" {}pt {}pt {}pt\n", scaled_text(w), scaled_text(h), scaled_text(d)).as_bytes());
        self.host.world.terminal_write(&line);
    }

    /// The SVG, and then the PDF, as the page is the box.
    fn session_write(&mut self, k: usize, b: i32) -> Outcome {
        let (pdf, svg) = self.host.hv.session.config.as_ref().map_or((false, false), |c| (c.pdf, c.svg));
        let mut base = self.host.hv.session.config.as_ref().map_or_else(Vec::new, |c| c.jobs[k].out.clone());
        // `box_end` in a line would have shifted it by this, as `native_finish` says, and `hpack` taken that in.
        let shift = self.native_session_shift();
        let (ht, dp) = ((self.height(b) - shift).max(0), shift.max(0));
        self.session_print_box(k, self.width(b), ht, dp);
        let mut outcome = Outcome::Written;
        if svg {
            if let Some(why) = self.host.hv.session.svg_failed.take() {
                outcome = Outcome::NoSvg(why);
            } else {
                let text = core::mem::take(&mut self.host.hv.session.svg);
                let at = base.len();
                base.extend_from_slice(b".svg");
                if !self.session_save(&base, &text) {
                    outcome = Outcome::Failed(format!("{} cannot be written", String::from_utf8_lossy(&base)));
                }
                base.truncate(at);
                self.host.hv.session.svg = text;
            }
        }
        if pdf {
            base.extend_from_slice(b".pdf");
            self.host.files[PDF_FILE as usize] = Slot::Pdf(PdfOut::new(&base));
            self.output_file_name = self.host.hv.session.file_name;
            self.session_set_page(b);
            self.ship_out(b);
            if !self.main_finish_pdf() {
                outcome = Outcome::Failed(format!("{} cannot be written", String::from_utf8_lossy(&base)));
            }
            self.session_new_pdf();
            if let Some(e) = self.session_error() {
                outcome = Outcome::Failed(e);
            }
        } else {
            self.flush_node_list(b);
        }
        outcome
    }

    /// Publish by rename, as the PDF is: a viewer may have the file open.
    fn session_save(&mut self, path: &[u8], bytes: &[u8]) -> bool {
        let tmp = [path, b".rstex-tmp"].concat();
        std::fs::write(crate::platform::path_from_bytes(&tmp), bytes).is_ok() && crate::platform::publish(&tmp, path).is_ok()
    }

    /// The page is the box.  At one scaled point from the corner, as `standalone` has it: its `\hoffset=-72.27pt` is rounded where `\pdfhorigin=1 true in` is truncated, so the same position in the
    /// stream gets the same rounding, and a line next to a pixel's edge stays on its side of it.
    fn session_set_page(&mut self, b: i32) {
        self.set_h_offset(-self.pdf_h_origin() - 1);
        self.set_v_offset(-self.pdf_v_origin() - 1);
        self.set_pdf_page_width(self.width(b));
        self.set_pdf_page_height(self.height(b) + self.depth(b));
    }

    /// What `pdf::emit` writes for `opacity` refers to these, on this page.
    pub(crate) fn session_page_resources(&mut self, d: &Diagram) {
        let mut text = Vec::new();
        commute::pdf::opacity_resources(d, &mut text);
        if self.pdf_page_resources() != NULL {
            self.delete_token_ref(self.pdf_page_resources());
            self.set_equiv(PDF_PAGE_RESOURCES_LOC, NULL);
        }
        if !text.is_empty() {
            let m = self.tb_mark();
            for &c in &text {
                self.tb_push(if c == b' ' { SPACE_TOKEN } else { OTHER_TOKEN + c as i32 });
            }
            let p = self.tb_commit(m);
            self.set_equiv(PDF_PAGE_RESOURCES_LOC, p);
        }
    }

    /// The diagram is in its box, with real tikz-cd, and a real `\shipout` is next.
    fn session_page_size(&mut self) {
        let Some(k) = self.host.hv.session.current.take() else { return };
        let b = self.box_reg(self.v_image.cd.session_box);
        let (pdf, svg) = self.host.hv.session.config.as_ref().map_or((false, false), |c| (c.pdf, c.svg));
        let out = self.host.hv.session.config.as_ref().map_or_else(Vec::new, |c| c.jobs[k].out.clone());
        let mut outcome = Outcome::Written;
        if svg {
            if !self.host.hv.session.native {
                outcome = Outcome::NoSvg("real PGF drew it, and only the library draws SVG".to_string());
            } else if let Some(why) = self.host.hv.session.svg_failed.take() {
                outcome = Outcome::NoSvg(why);
            } else {
                let text = core::mem::take(&mut self.host.hv.session.svg);
                let path = [&out[..], b".svg"].concat();
                if !self.session_save(&path, &text) {
                    outcome = Outcome::Failed(format!("{} cannot be written", String::from_utf8_lossy(&path)));
                }
            }
        }
        let error = self.session_error();
        if error.is_none() && b != NULL {
            self.session_print_box(k, self.width(b), self.height(b), self.depth(b));
        }
        if error.is_none() && pdf && b != NULL {
            self.session_set_page(b);
            let path = [&out[..], b".pdf"].concat();
            if self.output_file_name == 0 {
                self.host.files[PDF_FILE as usize] = Slot::Pdf(PdfOut::new(&path));
                self.output_file_name = self.host.hv.session.file_name;
            } else if let Slot::Pdf(p) = &mut self.host.files[PDF_FILE as usize] {
                p.path = path;
            }
        }
        if let Some(e) = error {
            outcome = Outcome::Failed(e);
        }
        self.host.hv.session.outcomes[k] = outcome;
    }

    /// A run with the library only ends here.
    fn session_end(&mut self) -> ! {
        let _ = self.session_error();
        let (jobs, finish) = match self.host.hv.session.config.as_mut() {
            Some(c) => (core::mem::take(&mut c.jobs), c.finish.take()),
            None => (Vec::new(), None),
        };
        let code = finish.map_or(0, |f| f(&jobs, &self.host.hv.session.outcomes));
        self.host.term_capture = None;
        self.host.term_buf.clear();
        self.host.hv.session.config = None;
        self.uexit(code)
    }

    /// The end of a run with real tikz-cd: the status says it all, and a diagram that TeX reported has no output.
    #[cold]
    pub fn session_at_exit(&mut self, code: i32) -> i32 {
        let mut message = None;
        if let Some(text) = self.host.term_capture.take() {
            let line = text.split(|&c| c == b'\n').find(|l| l.starts_with(b"! "));
            if let Some(l) = line.filter(|_| code != 0) {
                message = Some(String::from_utf8_lossy(l).into_owned());
            }
        }
        let mut outcomes = core::mem::take(&mut self.host.hv.session.outcomes);
        let at = self.host.hv.session.current;
        let Some(c) = self.host.hv.session.config.as_mut() else { return code };
        let jobs = core::mem::take(&mut c.jobs);
        let Some(f) = c.finish.take() else { return code };
        outcomes.resize(jobs.len(), Outcome::Pending);
        // TeX stopped, in this diagram if it was in one, and after what the run with real tikz-cd has written if not.
        match (message, at) {
            (Some(m), at) => {
                if let Some(o) = outcomes.get_mut(at.unwrap_or(0)) {
                    *o = Outcome::Failed(m);
                }
            }
            (None, Some(k)) if code != 0 => outcomes[k] = Outcome::Failed("TeX stopped here".to_string()),
            _ => {}
        }
        f(&jobs, &outcomes).max(code)
    }

    /// As §620, §681 and §697 have them, for another file.  A font's map entry and an encoding's names stay, and so do the job's name and the kinds of the fonts.
    fn session_new_pdf(&mut self) {
        self.total_pages = 0;
        self.max_v = 0;
        self.max_h = 0;
        self.dead_cycles = 0;
        self.cur_s = -1;
        self.pdf_gone = 0;
        self.pdf_os_mode = false;
        self.pdf_ptr = 0;
        self.pdf_op_ptr = 0;
        self.pdf_os_ptr = 0;
        self.pdf_os_cur_objnum = 0;
        self.pdf_os_cntr = 0;
        self.pdf_os_objidx = 0;
        self.pdf_buf_size = PDF_OP_BUF_SIZE;
        self.pdf_buf_is_os = false;
        self.pdf_seek_write_length = false;
        self.pdf_save_offset = 0;
        self.pdf_last_byte = 0;
        self.zip_write_state = NO_ZIP;
        self.pdf_version_written = false;
        self.init_pdf_output = false;
        self.obj_ptr = 0;
        self.sys_obj_ptr = 0;
        for k in 1..=HEAD_TAB_MAX {
            self.head_tab[k] = 0;
        }
        self.pdf_last_pages = 0;
        self.pdf_last_page = 0;
        self.pdf_last_stream = 0;
        self.pages_tail = 0;
        self.pdf_dest_names_ptr = 0;
        self.pdf_first_outline = 0;
        self.pdf_last_outline = 0;
        self.pdf_parent_outline = 0;
        self.pdf_obj_count = 0;
        self.pdf_xform_count = 0;
        self.pdf_ximage_count = 0;
        self.pdfobj_new_file();
        self.pdfout_new_file();
        self.pdffont_new_file();
    }

    // ------------------------------------------------------------------------------------------- the SVG of the boxes

    /// The diagram's SVG, with its boxes as they are before they go on the picture's list.
    pub(crate) fn session_svg(&mut self, d: &Diagram, ambient: Color, cells: &[i32], labels: &[i32]) {
        self.host.hv.session.native = true;
        if !self.host.hv.session.config.as_ref().is_some_and(|c| c.svg) {
            return;
        }
        let mut out = core::mem::take(&mut self.host.hv.session.svg);
        out.clear();
        svg::emit(d, ambient, &mut out, &mut SvgBoxes { t: &mut *self, cells, labels });
        self.host.hv.session.svg = out;
    }

    /// §648, what it does with a box's list, into `w`.  `(h, v)`: the box's reference point, y down.
    fn session_hlist(&mut self, w: &mut Walk<'_>, this_box: i32, h: i32, v: i32) {
        let (mut cur_g, mut cur_glue) = (0i32, 0.0f64);
        let (g_order, g_sign) = (self.glue_order(this_box), self.glue_sign(this_box));
        let (base_line, left_edge) = (v, h);
        let mut cur_h = h;
        let mut p = self.list_ptr(this_box);
        while p != NULL {
            if self.is_char_node(p) {
                let (f, c) = (self.font(p), self.character(p));
                if self.is_valid_char(f, c) {
                    self.session_glyph(w, f, c, cur_h, base_line);
                    cur_h += self.char_width(f, self.char_info(f, c));
                }
                p = self.link(p);
                continue;
            }
            match self.ty(p) {
                HLIST_NODE | VLIST_NODE => {
                    if self.list_ptr(p) != NULL {
                        let v2 = base_line + self.shift_amount(p);
                        if self.ty(p) == VLIST_NODE {
                            self.session_vlist(w, p, cur_h, v2);
                        } else {
                            self.session_hlist(w, p, cur_h, v2);
                        }
                    }
                    cur_h += self.width(p);
                }
                RULE_NODE => {
                    let (ht, dp, wd) = (self.height(p), self.depth(p), self.width(p));
                    self.session_hrule(w, this_box, cur_h, base_line, ht, dp, wd);
                    cur_h += wd;
                }
                WHATSIT_NODE => self.session_whatsit(w, p, cur_h, base_line),
                GLUE_NODE => {
                    self.round_glue(p, this_box, g_sign, g_order, &mut cur_g, &mut cur_glue);
                    let wd = self.rule_wd;
                    if self.subtype(p) >= A_LEADERS {
                        let leader = self.leader_ptr(p);
                        if self.ty(leader) == RULE_NODE {
                            let (ht, dp) = (self.height(leader), self.depth(leader));
                            self.session_hrule(w, this_box, cur_h, base_line, ht, dp, wd);
                            cur_h += wd;
                        } else if self.width(leader) > 0 && wd > 0 {
                            // §736, which goes on to `next_p` with `edge - 10`.
                            let leader_wd = self.width(leader);
                            let wd2 = wd + 10;
                            let edge = cur_h + wd2;
                            let mut lx = 0;
                            if self.subtype(p) == A_LEADERS {
                                let save_h = cur_h;
                                cur_h = left_edge + leader_wd * ((cur_h - left_edge) / leader_wd);
                                if cur_h < save_h {
                                    cur_h += leader_wd;
                                }
                            } else {
                                let (lq, lr) = (wd2 / leader_wd, wd2 % leader_wd);
                                if self.subtype(p) == C_LEADERS {
                                    cur_h += lr / 2;
                                } else {
                                    lx = lr / (lq + 1);
                                    cur_h += (lr - (lq - 1) * lx) / 2;
                                }
                            }
                            while cur_h + leader_wd <= edge {
                                let v2 = base_line + self.shift_amount(leader);
                                if self.ty(leader) == VLIST_NODE {
                                    self.session_vlist(w, leader, cur_h, v2);
                                } else {
                                    self.session_hlist(w, leader, cur_h, v2);
                                }
                                cur_h += leader_wd + lx;
                            }
                            cur_h = edge - 10;
                        } else {
                            cur_h += wd;
                        }
                    } else {
                        cur_h += wd;
                    }
                }
                MARGIN_KERN_NODE | KERN_NODE | MATH_NODE => cur_h += self.width(p),
                LIGATURE_NODE => {
                    let q = self.lig_char(p);
                    let (f, c) = (self.font(q), self.character(q));
                    if self.is_valid_char(f, c) {
                        self.session_glyph(w, f, c, cur_h, base_line);
                        cur_h += self.char_width(f, self.char_info(f, c));
                    }
                }
                _ => {}
            }
            p = self.link(p);
        }
    }

    /// §734
    #[allow(clippy::too_many_arguments)]
    fn session_hrule(&mut self, w: &mut Walk<'_>, this_box: i32, x: i32, base_line: i32, ht: i32, dp: i32, wd: i32) {
        let ht = if self.is_running(ht) { self.height(this_box) } else { ht };
        let dp = if self.is_running(dp) { self.depth(this_box) } else { dp };
        if ht + dp > 0 && wd > 0 {
            w.rule(x, base_line + dp, wd, ht + dp);
        }
    }

    /// §740.  `(h, v)` as for `session_hlist`.
    fn session_vlist(&mut self, w: &mut Walk<'_>, this_box: i32, h: i32, v: i32) {
        let (mut cur_g, mut cur_glue) = (0i32, 0.0f64);
        let (g_order, g_sign) = (self.glue_order(this_box), self.glue_sign(this_box));
        let left_edge = h;
        let mut cur_v = v - self.height(this_box);
        let top_edge = cur_v;
        let mut p = self.list_ptr(this_box);
        while p != NULL {
            if self.is_char_node(p) {
                break;
            }
            match self.ty(p) {
                HLIST_NODE | VLIST_NODE => {
                    cur_v += self.height(p);
                    if self.list_ptr(p) != NULL {
                        let h2 = left_edge + self.shift_amount(p);
                        if self.ty(p) == VLIST_NODE {
                            self.session_vlist(w, p, h2, cur_v);
                        } else {
                            self.session_hlist(w, p, h2, cur_v);
                        }
                    }
                    cur_v += self.depth(p);
                }
                RULE_NODE => {
                    let (ht, dp, wd) = (self.height(p), self.depth(p), self.width(p));
                    let wd = if self.is_running(wd) { self.width(this_box) } else { wd };
                    cur_v += ht + dp;
                    if ht + dp > 0 && wd > 0 {
                        w.rule(left_edge, cur_v, wd, ht + dp);
                    }
                }
                WHATSIT_NODE => self.session_whatsit(w, p, left_edge, cur_v),
                GLUE_NODE => {
                    // §744
                    let g = self.glue_ptr(p);
                    let mut ht = self.width(g) - cur_g;
                    if g_sign == STRETCHING {
                        if self.stretch_order(g) == g_order {
                            cur_glue += self.stretch(g) as f64;
                            cur_g = round(vet_glue(self.glue_set(this_box) * cur_glue));
                        }
                    } else if g_sign == SHRINKING && self.shrink_order(g) == g_order {
                        cur_glue -= self.shrink(g) as f64;
                        cur_g = round(vet_glue(self.glue_set(this_box) * cur_glue));
                    }
                    ht += cur_g;
                    if self.subtype(p) >= A_LEADERS {
                        let leader = self.leader_ptr(p);
                        if self.ty(leader) == RULE_NODE {
                            let wd = self.width(leader);
                            let wd = if self.is_running(wd) { self.width(this_box) } else { wd };
                            cur_v += ht;
                            if ht > 0 && wd > 0 {
                                w.rule(left_edge, cur_v, wd, ht);
                            }
                        } else if self.height(leader) + self.depth(leader) > 0 && ht > 0 {
                            // §745
                            let leader_ht = self.height(leader) + self.depth(leader);
                            let ht2 = ht + 10;
                            let edge = cur_v + ht2;
                            let mut lx = 0;
                            if self.subtype(p) == A_LEADERS {
                                let save_v = cur_v;
                                cur_v = top_edge + leader_ht * ((cur_v - top_edge) / leader_ht);
                                if cur_v < save_v {
                                    cur_v += leader_ht;
                                }
                            } else {
                                let (lq, lr) = (ht2 / leader_ht, ht2 % leader_ht);
                                if self.subtype(p) == C_LEADERS {
                                    cur_v += lr / 2;
                                } else {
                                    lx = lr / (lq + 1);
                                    cur_v += (lr - (lq - 1) * lx) / 2;
                                }
                            }
                            while cur_v + leader_ht <= edge {
                                let h2 = left_edge + self.shift_amount(leader);
                                let v2 = cur_v + self.height(leader);
                                if self.ty(leader) == VLIST_NODE {
                                    self.session_vlist(w, leader, h2, v2);
                                } else {
                                    self.session_hlist(w, leader, h2, v2);
                                }
                                cur_v += leader_ht + lx;
                            }
                            cur_v = edge - 10;
                        } else {
                            cur_v += ht;
                        }
                    } else {
                        cur_v += ht;
                    }
                }
                KERN_NODE => cur_v += self.width(p),
                _ => {}
            }
            p = self.link(p);
        }
    }

    /// What a whatsit means for the picture.  Links, destinations, `\write` and the like draw nothing.
    fn session_whatsit(&mut self, w: &mut Walk<'_>, p: i32, x: i32, y: i32) {
        match self.subtype(p) {
            PDF_COLORSTACK_NODE => {
                if self.pdf_colorstack_stack(p) != self.native_session_color_stack() {
                    return;
                }
                match self.pdf_colorstack_cmd(p) {
                    COLORSTACK_POP => w.pop_color(),
                    cmd @ (COLORSTACK_PUSH | COLORSTACK_SET) => {
                        let text = self.session_toks_text(self.pdf_colorstack_data(p));
                        let Some(c) = crate::native::native_color_of_operators(&text) else {
                            w.failed.get_or_insert_with(|| "a colour in a form that cannot be read".to_string());
                            return;
                        };
                        let hex = svg::hex(c);
                        if cmd == COLORSTACK_PUSH {
                            w.push_color(hex);
                        } else {
                            w.set_color(hex);
                        }
                    }
                    _ => {}
                }
            }
            PDF_SAVE_NODE => w.saved.push(w.m),
            PDF_RESTORE_NODE => {
                if let Some(m) = w.saved.pop() {
                    w.m = m;
                }
            }
            PDF_SETMATRIX_NODE => {
                let text = self.session_toks_text(self.pdf_setmatrix_data(p));
                let s = String::from_utf8_lossy(&text);
                let v: Vec<f64> = s.split_ascii_whitespace().filter_map(|x| x.parse().ok()).collect();
                let [a, b, c, d] = v[..] else {
                    w.failed.get_or_insert_with(|| "a \\pdfsetmatrix in a form that cannot be read".to_string());
                    return;
                };
                // Around where TeX stands, y up in PDF and y down here.
                let (px, py) = (f64::from(x) / 65536.0, f64::from(y) / 65536.0);
                let m = Matrix { a, b: -b, c: -c, d, e: px - a * px + c * py, f: py + b * px - d * py };
                w.m = compose(&w.m, &m);
            }
            PDF_LITERAL_NODE | PDF_LATELITERAL_NODE => {
                if self.pdf_literal_data(p) != NULL && !self.session_toks_text(self.pdf_literal_data(p)).is_empty() {
                    w.failed.get_or_insert_with(|| "a \\pdfliteral in a cell has no outline".to_string());
                }
            }
            PDF_REFXIMAGE_NODE | PDF_REFXFORM_NODE => {
                w.failed.get_or_insert_with(|| "an image in a cell has no outline".to_string());
            }
            _ => {}
        }
    }

    fn session_toks_text(&mut self, p: i32) -> Vec<u8> {
        let s = self.tokens_to_string(p);
        let text = self.str_pool.range(self.str_start[s], self.str_start[s + 1]).to_vec();
        self.flush_str(s);
        text
    }

    /// A font file, from the disk once in a session.
    pub(crate) fn session_font_file(&mut self, path: &[u8]) -> Option<Rc<Vec<u8>>> {
        if !self.session_on() {
            return self.host.world.read(path).map(Rc::new);
        }
        if let Some((_, d)) = self.host.hv.session.font_files.iter().find(|f| f.0 == path) {
            return Some(d.clone());
        }
        let d = Rc::new(self.host.world.read(path)?);
        self.host.hv.session.font_files.push((path.to_vec(), d.clone()));
        Some(d)
    }

    /// The outline of a character, `(x, y)` being where it stands, y down.
    fn session_glyph(&mut self, w: &mut Walk<'_>, f: i32, c: i32, x: i32, y: i32) {
        let Some(slot) = self.session_font(f, w) else { return };
        if self.host.hv.session.fonts[slot].glyphs[c as usize].is_none() {
            let font = &self.host.hv.session.fonts[slot];
            let name: &[u8] = match &font.names {
                Some(n) => n.name(c as usize),
                None => font.outlines.encoding_name(c as usize),
            };
            let path = font.outlines.glyph(name).unwrap_or_default();
            self.host.hv.session.fonts[slot].glyphs[c as usize] = Some(path.into_boxed_slice());
        }
        let size = f64::from(self.font_size[f]) / 65536.0;
        let (ox, oy) = (f64::from(x) / 65536.0, f64::from(y) / 65536.0);
        let font = &self.host.hv.session.fonts[slot];
        let fm = font.outlines.font_matrix;
        // The map's `ExtendFont` and `SlantFont` are applied to the font's matrix, as `writet1` writes them.
        let extend = if font.extend != 0 { font.extend as f64 / 1000.0 } else { 1.0 };
        let slant = font.slant as f64 / 1000.0;
        let (a, b, cc, d) = (fm[0] * extend, fm[1], (fm[2] + fm[3] * slant), fm[3]);
        let at = |u: f64, v: f64| (ox + (a * u + cc * v) * size, oy - (b * u + d * v) * size);
        let path: &[OutlineSeg] = font.glyphs[c as usize].as_deref().unwrap_or(&[]);
        if path.is_empty() {
            return;
        }
        w.begin_path();
        for (k, s) in path.iter().enumerate() {
            if k > 0 {
                w.out.push(b' ');
            }
            match *s {
                OutlineSeg::Move(u, v) => {
                    w.out.extend_from_slice(b"M ");
                    let (px, py) = at(u, v);
                    w.point(px, py);
                }
                OutlineSeg::Line(u, v) => {
                    w.out.extend_from_slice(b"L ");
                    let (px, py) = at(u, v);
                    w.point(px, py);
                }
                OutlineSeg::Curve(u1, v1, u2, v2, u3, v3) => {
                    w.out.extend_from_slice(b"C ");
                    let (px, py) = at(u1, v1);
                    w.point(px, py);
                    w.out.push(b' ');
                    let (px, py) = at(u2, v2);
                    w.point(px, py);
                    w.out.push(b' ');
                    let (px, py) = at(u3, v3);
                    w.point(px, py);
                }
                OutlineSeg::Close => w.out.push(b'Z'),
            }
        }
    }

    /// The font's place among those read, or why it has no outlines.
    fn session_font(&mut self, f: i32, w: &mut Walk<'_>) -> Option<usize> {
        let k = f as usize;
        let slots = &mut self.host.hv.session.font_slot;
        if slots.len() <= k {
            slots.resize(k + 1, 0);
        }
        if slots[k] != 0 {
            return Some(slots[k] as usize - 1);
        }
        let source = match self.pdffont_outline_source(f) {
            Ok(s) => s,
            Err(why) => {
                w.failed.get_or_insert(why);
                return None;
            }
        };
        let Some(outlines) = T1Outlines::parse(&source.file) else {
            w.failed.get_or_insert_with(|| format!("font file {} cannot be read", String::from_utf8_lossy(&source.path)));
            return None;
        };
        let hv = &mut self.host.hv.session;
        hv.fonts.push(SvgFont { outlines, names: source.names, slant: source.slant, extend: source.extend, glyphs: vec![None; 256] });
        hv.font_slot[k] = hv.fonts.len() as u32;
        Some(hv.fonts.len() - 1)
    }
}
