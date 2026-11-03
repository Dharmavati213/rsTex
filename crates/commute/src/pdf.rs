//! PDF content-stream operators, in the form and order pgf writes them (spec 13 and 15).  The numbers go through TeX's own
//! printing with pgf's factor, so that what pgf writes reads the same, token for token, line for line.
//!
//! What pgf does not write is where TeX stands when it writes: a box and some of the literals sit at a position pdfTeX then
//! reaches with a `1 0 0 1 x y cm` of its own.  A sink chooses.  If it says [`PdfSink::tex_positions`], it gets those positions
//! in [`PdfSink::position`] and moves TeX there, and pdfTeX writes what it wrote for real PGF.  Otherwise the operators hold
//! the `cm`, worked out as pdfTeX's `pdf_set_origin` does from the picture's origin, and the box goes at the origin in force.
//!
//! The stream is cut where a TeX box goes: [`PdfSink::literal`] gets the operators and [`PdfSink::place_box`] says which box
//! comes next.

use crate::BoxRef;
use crate::color::Color;
use crate::diagram::{Diagram, Item, Role};
use crate::geom::{Matrix, Seg, sp};
use crate::number::{fixed, pgf_bp, pgf_num, print_milli};
use crate::pgfmath::Sp;
use crate::spec::{Dash, Opt};
use crate::tips::TipForm;

/// Where the stream goes.
pub trait PdfSink {
    /// Operators, ending in a complete token.
    fn literal(&mut self, ops: &[u8]);
    /// Put this box's reference point at the current origin, with the current matrix in force.
    fn place_box(&mut self, which: BoxRef);
    /// Whether this sink moves TeX to the positions it is given, instead of having them written as `cm`.
    fn tex_positions(&self) -> bool {
        false
    }
    /// TeX moves to `(x, y)`, in scaled points from the picture's origin, y up, before the next literal or box.
    fn position(&mut self, _x: i64, _y: i64) {}
}

/// `one_hundred_bp` and `min_bp_val` of pdftex.web.
const ONE_HUNDRED_BP: Sp = 6578176;
const MIN_BP_VAL: Sp = 66;

/// `divide_scaled(s, one_hundred_bp, 5)`: thousandths of a bp, and the value that stands for.
fn divide_scaled(s0: Sp) -> (Sp, Sp) {
    let neg = s0 < 0;
    let s = s0.abs();
    let (mut q, mut r) = (s / ONE_HUNDRED_BP, s % ONE_HUNDRED_BP);
    for _ in 0..5 {
        q = 10 * q + 10 * r / ONE_HUNDRED_BP;
        r = 10 * r % ONE_HUNDRED_BP;
    }
    if 2 * r >= ONE_HUNDRED_BP {
        q += 1;
        r -= ONE_HUNDRED_BP;
    }
    let out = s - r / 100_000;
    if neg { (-q, -out) } else { (q, out) }
}

/// What `\pdfcolorstack` holds on top after the bare colours of `run`, over `below`.
fn top_color(run: &[Opt], below: Color) -> Color {
    run.iter().rev().find_map(bare_color).unwrap_or(below)
}

fn bare_color(o: &Opt) -> Option<Color> {
    if let Opt::Color(c) = *o { Some(c) } else { None }
}

struct Writer<'a, S: PdfSink> {
    out: &'a mut Vec<u8>,
    sink: &'a mut S,
    tex: bool,
    /// Where TeX stands.
    at: (Sp, Sp),
    /// `pdf_origin_h` and `pdf_origin_v`, the matrix in force, when the operators hold the `cm`.
    origin: (Sp, Sp),
}

impl<S: PdfSink> Writer<'_, S> {
    /// One `\pdfliteral`, or one `\pgfsysprotocol` flush: pdfTeX ends it with a newline.
    #[inline]
    fn lit(&mut self, s: &str) {
        self.settle();
        self.out.extend_from_slice(s.as_bytes());
        self.out.extend_from_slice(b" \n");
    }

    /// `pdf_set_origin` before a literal, when the sink does not move TeX itself.
    #[inline]
    fn settle(&mut self) {
        if self.tex || self.at == self.origin {
            return;
        }
        let (dx, dy) = (self.at.0 - self.origin.0, self.at.1 - self.origin.1);
        if dx.abs() < MIN_BP_VAL && dy.abs() < MIN_BP_VAL {
            return;
        }
        let (qx, ox) = divide_scaled(dx);
        let (qy, oy) = divide_scaled(dy);
        self.origin.0 += ox;
        self.origin.1 += oy;
        self.out.extend_from_slice(b"1 0 0 1 ");
        print_milli(self.out, qx);
        self.out.push(b' ');
        print_milli(self.out, qy);
        self.out.extend_from_slice(b" cm\n");
    }

    fn move_to(&mut self, x: f64, y: f64) {
        let p = (sp(x), sp(y));
        if p == self.at {
            return;
        }
        self.at = p;
        if self.tex {
            self.flush();
            self.sink.position(p.0, p.1);
        }
    }

    fn flush(&mut self) {
        if !self.out.is_empty() {
            self.sink.literal(self.out);
            self.out.clear();
        }
    }

    fn place(&mut self, which: BoxRef) {
        self.settle();
        self.flush();
        self.sink.place_box(which);
    }

    fn bp(&mut self, v: f64) {
        pgf_bp(self.out, v);
        self.out.push(b' ');
    }

    fn num(&mut self, v: f64) {
        pgf_num(self.out, v);
        self.out.push(b' ');
    }

    /// `\pgfsys@setlinewidth`.
    fn width(&mut self, w: f64) {
        self.settle();
        self.bp(w);
        self.out.extend_from_slice(b"w \n");
    }

    fn color(&mut self, c: Color, stroke: bool) {
        self.settle();
        color_op(self.out, c, stroke);
        self.out.extend_from_slice(b"\n");
    }

    /// `a b c d e f cm`, written by pgf.
    fn cm(&mut self, m: &Matrix, e: f64, f: f64) {
        self.settle();
        self.num(m.a);
        self.num(m.b);
        self.num(m.c);
        self.num(m.d);
        self.bp(e);
        self.bp(f);
        self.out.extend_from_slice(b"cm \n");
    }

    fn path(&mut self, segs: &[Seg]) {
        self.settle();
        for &s in segs {
            seg_ops(self.out, s);
            self.out.extend_from_slice(b"\n");
        }
    }

    /// One thing an arrow's or a node's keys do to the graphics state.
    fn option(&mut self, o: &Opt, width_now: &mut f64, tip_fill: &mut Option<Color>) {
        match *o {
            Opt::Width(v) => {
                self.width(v);
                *width_now = v;
            }
            Opt::Dash(ref v) => {
                self.settle();
                dash_op(self.out, v);
                self.out.extend_from_slice(b"\n");
            }
            Opt::Color(c) => {
                self.color(c, false);
                self.color(c, true);
                *tip_fill = None;
            }
            Opt::SetColor(c) => {
                self.color(c, true);
                self.color(c, false);
                *tip_fill = None;
            }
            Opt::Stroke(c) => {
                self.color(c, true);
                *tip_fill = Some(c);
            }
            Opt::Fill(c) => self.color(c, false),
            Opt::Opacity(v) => {
                self.settle();
                opacity_op(self.out, b"/pgf@CA", v);
                opacity_op(self.out, b"/pgf@ca", v);
            }
            Opt::StrokeOpacity(v) => {
                self.settle();
                opacity_op(self.out, b"/pgf@CA", v);
            }
            Opt::FillOpacity(v) => {
                self.settle();
                opacity_op(self.out, b"/pgf@ca", v);
            }
            Opt::Canvas(x, y) => self.cm(&Matrix::IDENTITY, x, y),
            Opt::Cap(c) => {
                self.settle();
                cap_op(self.out, c);
                self.out.extend_from_slice(b"\n");
            }
            Opt::Join(c) => {
                self.settle();
                join_op(self.out, c);
                self.out.extend_from_slice(b"\n");
            }
        }
    }

    /// A node's options: `\tikz@options` in its `\pgfscope`, and again in the scope of `\tikz@finish` around its path.
    fn node_open(&mut self, opts: &[Opt]) {
        let (mut width, mut tip) = (0.0, None);
        if opts.is_empty() {
            return;
        }
        for o in opts {
            self.option(o, &mut width, &mut tip);
        }
        self.lit("q");
        for o in opts {
            self.option(o, &mut width, &mut tip);
        }
    }

    /// The end of that scope.  `below` is the colour under the node's.
    fn node_path_done(&mut self, opts: &[Opt], below: Color) {
        if !opts.is_empty() {
            self.pop_colors(opts, top_color(opts, below));
            self.lit("Q");
        }
    }

    /// `\color` pushes on `\pdfcolorstack`, and takes it off as its group ends: what is then on top is written, every time.
    fn pop_colors(&mut self, run: &[Opt], below: Color) {
        let mut colors = run.iter().rev().filter_map(bare_color);
        if colors.next().is_none() {
            return;
        }
        for c in colors.chain(core::iter::once(below)) {
            self.color(c, false);
            self.color(c, true);
        }
    }

    /// A box and the scopes pgf has around it: `\pgfsys@hboxsynced`, whose position is TeX's.
    fn boxed(&mut self, which: BoxRef, m: &Matrix, color: Option<Color>, ambient: Color, back: (f64, f64)) {
        self.lit("q");
        self.move_to(m.e, m.f);
        self.settle();
        self.num(m.a);
        self.num(m.b);
        self.num(m.c);
        self.num(m.d);
        self.out.extend_from_slice(b"0.0 0.0 cm \n");
        self.lit("q");
        let c = color.unwrap_or(ambient);
        self.color(c, true);
        self.color(c, false);
        self.place(which);
        self.lit("Q");
        self.move_to(back.0, back.1);
        self.lit("Q");
    }
}

fn word(out: &mut Vec<u8>, s: &str) {
    out.extend_from_slice(s.as_bytes());
    out.push(b' ');
}

fn comp(out: &mut Vec<u8>, v: f64) {
    fixed(out, v, 5);
    out.push(b' ');
}

/// The fill or stroke colour operator.
fn color_op(out: &mut Vec<u8>, c: Color, stroke: bool) {
    match c {
        Color::Gray(g) => {
            comp(out, g);
            word(out, if stroke { "G" } else { "g" });
        }
        Color::Rgb(r, g, b) => {
            comp(out, r);
            comp(out, g);
            comp(out, b);
            word(out, if stroke { "RG" } else { "rg" });
        }
        Color::Cmyk(c, m, y, k) => {
            comp(out, c);
            comp(out, m);
            comp(out, y);
            comp(out, k);
            word(out, if stroke { "K" } else { "k" });
        }
    }
}

fn bp(out: &mut Vec<u8>, v: f64) {
    pgf_bp(out, v);
    out.push(b' ');
}

fn dash_op(out: &mut Vec<u8>, d: &Dash) {
    word(out, "[");
    for &v in d.values() {
        bp(out, v);
    }
    word(out, "] 0.0 d");
}

fn seg_ops(out: &mut Vec<u8>, s: Seg) {
    match s {
        Seg::Move(p) => {
            bp(out, p.x);
            bp(out, p.y);
            word(out, "m");
        }
        Seg::Line(p) => {
            bp(out, p.x);
            bp(out, p.y);
            word(out, "l");
        }
        Seg::Curve(a, b, c) => {
            bp(out, a.x);
            bp(out, a.y);
            bp(out, b.x);
            bp(out, b.y);
            bp(out, c.x);
            bp(out, c.y);
            word(out, "c");
        }
        Seg::Close => word(out, "h"),
    }
}

fn cap_op(out: &mut Vec<u8>, cap: u8) {
    out.push(b'0' + cap);
    word(out, " J");
}

fn join_op(out: &mut Vec<u8>, join: u8) {
    out.push(b'0' + join);
    word(out, " j");
}

/// A tip's setup, with its width if its body sets one.
fn tip_setup(out: &mut Vec<u8>, form: TipForm, width: Option<f64>, cap: u8, join: u8) {
    let set_width = |out: &mut Vec<u8>| {
        if let Some(w) = width {
            bp(out, w);
            word(out, "w");
        }
    };
    match form {
        TipForm::WidthFirst => {
            set_width(out);
            word(out, "[ ] 0.0 d");
            cap_op(out, cap);
            join_op(out, join);
        }
        TipForm::WidthLast => {
            word(out, "[ ] 0.0 d");
            cap_op(out, cap);
            join_op(out, join);
            set_width(out);
        }
        TipForm::WidthMiddle => {
            word(out, "[ ] 0.0 d");
            set_width(out);
            cap_op(out, cap);
            join_op(out, join);
        }
        TipForm::JoinFirst => {
            word(out, "[ ] 0.0 d");
            join_op(out, join);
            cap_op(out, cap);
            set_width(out);
        }
    }
}

/// Write the whole picture.  `ambient` is the colour in force at the start of the picture, `scratch` a reusable buffer.
pub fn emit<S: PdfSink>(d: &Diagram, ambient: Color, scratch: &mut Vec<u8>, sink: &mut S) {
    let list = d.draw_list();
    let r = d.result();
    scratch.clear();
    let tex = sink.tex_positions();
    let mut w = Writer { out: scratch, sink, tex, at: (0, 0), origin: (0, 0) };

    // `\pgfpicture`: its scope, the colour, 0.4pt; `\scope` of the picture.
    w.lit("q");
    w.color(ambient, true);
    w.color(ambient, false);
    w.lit("0.3985 w");
    w.lit("q");
    let cell_lw = d.cell_line_width();
    if cell_lw != 0.4 && sp(cell_lw) != crate::keys::CELL_LW {
        w.width(cell_lw);
    }
    // The matrix: its `\pgfscope`, `\pgfsys@hboxsynced` and the box's scope.
    w.lit("q");
    w.lit("q");
    let (mx, my) = (r.matrix_x, r.matrix_y);
    w.move_to(mx, my);
    w.settle();
    w.out.extend_from_slice(b"1.0 0.0 0.0 1.0 0.0 0.0 cm \n");
    w.lit("q");

    let rt = d.font().rt;
    // The matrix's scope stays open until the first arrow or the end.
    let mut in_matrix = true;
    // The width in force.
    let mut width_now = rt;
    // `\pgf@arrows@color@setup`: a tip's fill when stroke and fill differ.
    let mut tip_fill = None;
    // Inside the `q L w` of a double line, which holds the tips but not the labels.
    let mut in_double = false;
    // The arrow's options: what its colours do lasts until its end.
    let mut arrow_opts: &[Opt] = &[];
    // A label's fill and a cell's border, to be drawn in the node's scope.
    let mut fill = None;
    let mut border = None;

    let close_matrix = |w: &mut Writer<'_, S>| {
        w.move_to(mx, my);
        w.lit("Q");
        w.move_to(0.0, 0.0);
        w.lit("Q");
        w.lit("Q");
    };

    for item in &list.items {
        if in_double && matches!(item, Item::LabelFill { .. } | Item::LabelBox { .. } | Item::ArrowEnd) {
            w.lit("Q");
            in_double = false;
        }
        if in_matrix && !matches!(item, Item::CellBorder { .. } | Item::CellBox { .. } | Item::CellEmpty { .. }) {
            close_matrix(&mut w);
            in_matrix = false;
        }
        match *item {
            Item::CellBorder { x, y, w: bw, h, rounded, ox, oy, .. } => border = Some((x, y, bw, h, ox, oy, rounded)),
            Item::CellBox { cell, x, y, color, ox, oy, opts } => {
                let opts = list.options(opts);
                w.move_to(ox, oy);
                w.lit("q");
                w.lit("q");
                w.node_open(opts);
                // Relative to the cell's origin, where the cell's picture is.
                if let Some((x, y, bw, h, ox, oy, rounded)) = border.take() {
                    match rounded {
                        Some(p) => {
                            let t = Matrix::translate(-ox, -oy);
                            w.settle();
                            for s in list.path(p) {
                                seg_ops(w.out, s.map(&t));
                                w.out.extend_from_slice(b"\n");
                            }
                        }
                        None => {
                            w.settle();
                            w.bp(x - ox);
                            w.bp(y - oy);
                            w.bp(bw);
                            w.bp(h);
                            w.out.extend_from_slice(b"re \n");
                        }
                    }
                    w.lit("S");
                }
                w.node_path_done(opts, ambient);
                w.boxed(BoxRef::Cell(cell), &Matrix::translate(x, y), color, ambient, (ox, oy));
                w.pop_colors(opts, ambient);
                w.lit("Q");
                w.lit("Q");
            }
            Item::CellEmpty { ox, oy } => {
                w.move_to(ox, oy);
                w.lit("q");
                w.lit("Q");
            }
            Item::Coordinates { n } => {
                for _ in 0..n {
                    w.lit("q");
                    w.lit("Q");
                }
            }
            Item::Stroke { role: Role::Halo, path, width, color, .. } => {
                w.lit("q");
                w.lit("q");
                if let Some(c) = color {
                    w.color(c, true);
                }
                w.width(width);
                w.path(list.path(path));
                w.lit("S");
                w.lit("Q");
                w.lit("Q");
            }
            Item::ArrowBegin { opts, .. } => {
                w.lit("q");
                tip_fill = None;
                arrow_opts = list.options(opts);
                for o in arrow_opts {
                    w.option(o, &mut width_now, &mut tip_fill);
                }
            }
            Item::Stroke { role: Role::Line, path, width, .. } => {
                if (width - width_now).abs() > 1e-9 {
                    w.lit("q");
                    w.width(width);
                    width_now = width;
                    in_double = true;
                }
                w.path(list.path(path));
                w.lit("S");
            }
            Item::Stroke { role: Role::Inner, path, width, color, .. } => {
                w.lit("q");
                w.width(width);
                if let Some(c) = color {
                    w.color(c, true);
                }
                w.path(list.path(path));
                w.lit("S");
                w.lit("Q");
            }
            Item::Tip { m, path, width, own_width, cap, join, form } => {
                w.lit("q");
                if let Some(c) = tip_fill {
                    w.color(c, false);
                }
                w.cm(&m, m.e, m.f);
                word(w.out, "q");
                tip_setup(w.out, form, if own_width { Some(width) } else { None }, cap, join);
                for &s in list.path(path) {
                    seg_ops(w.out, s);
                }
                w.out.extend_from_slice(b"S Q  \n");
                w.lit("Q");
            }
            Item::LabelFill { m, x, y, w: bw, h, path, .. } => fill = Some((m, x, y, bw, h, path)),
            Item::LabelBox { label, m, color, opts } => {
                let opts = list.options(opts);
                let below = top_color(arrow_opts, ambient);
                w.lit("q");
                w.node_open(opts);
                if let Some((m, x, y, bw, h, path)) = fill.take() {
                    if m.is_translation() {
                        w.settle();
                        w.bp(m.e + x);
                        w.bp(m.f + y);
                        w.bp(bw);
                        w.bp(h);
                        w.out.extend_from_slice(b"re \n");
                    } else {
                        w.path(list.path(path));
                    }
                    w.lit("f");
                }
                w.node_path_done(opts, below);
                w.boxed(BoxRef::Label(label), &m, color, ambient, (0.0, 0.0));
                w.pop_colors(opts, below);
                w.lit("Q");
            }
            Item::ArrowEnd => {
                w.pop_colors(arrow_opts, ambient);
                w.lit("Q");
                width_now = rt;
            }
        }
    }
    if in_matrix {
        close_matrix(&mut w);
    }
    w.lit("Q");
    w.lit("n");
    w.lit("Q");
    w.flush();
}

fn opacity_op(out: &mut Vec<u8>, name: &[u8], v: f64) {
    out.extend_from_slice(name);
    fixed(out, v, 5);
    out.extend_from_slice(b" gs \n");
}

/// Collects the stream into one buffer, with a comment line at each box.  For tests and for looking at the stream.
#[derive(Default)]
pub struct Collect {
    pub ops: Vec<u8>,
}

impl PdfSink for Collect {
    fn literal(&mut self, ops: &[u8]) {
        self.ops.extend_from_slice(ops);
    }

    fn place_box(&mut self, which: BoxRef) {
        use std::io::Write;
        let _ = match which {
            BoxRef::Cell(i) => writeln!(self.ops, "%cell{i}"),
            BoxRef::Label(i) => writeln!(self.ops, "%label{i}"),
        };
    }
}
