//! PDF content-stream operators, in the form pgf writes them (spec 13 and 15).  Numbers go through TeX's own printing
//! with pgf's factor, so that a diagram's stroke operators read like pgf's, up to the rounding the spec allows.
//!
//! The stream is cut where a TeX box goes: [`PdfSink::literal`] gets the operators and [`PdfSink::place_box`] says which
//! box comes next.  The operators around a box already hold its `cm`, as pgf's do, so the engine ships the box at the
//! picture's origin; a standalone writer just writes text there.

use crate::BoxRef;
use crate::color::Color;
use crate::diagram::{Diagram, Item, Role};
use crate::geom::{Matrix, Pt, Seg};
use crate::number::{fixed, pgf_bp, pgf_num};
use crate::spec::Dash;
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

/// pdfTeX's positions use the true factor, where pgf's use 0.99627 (spec section 2).
const TRUE_BP: f64 = 7200.0 / 7227.0;

fn word(out: &mut Vec<u8>, s: &str) {
    out.extend_from_slice(s.as_bytes());
    out.push(b' ');
}

fn bp(out: &mut Vec<u8>, v: f64) {
    pgf_bp(out, v);
    out.push(b' ');
}

fn num(out: &mut Vec<u8>, v: f64) {
    pgf_num(out, v);
    out.push(b' ');
}

/// A translation as pdfTeX writes it, three decimals with the true factor.
fn pos(out: &mut Vec<u8>, v: f64) {
    fixed(out, v * TRUE_BP, 3);
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

fn path_ops(out: &mut Vec<u8>, segs: &[Seg]) {
    for &s in segs {
        seg_ops(out, s);
    }
}

fn canvas_cm(out: &mut Vec<u8>, canvas: Pt) {
    if canvas != Pt::ZERO {
        word(out, "1.0 0.0 0.0 1.0");
        bp(out, canvas.x);
        bp(out, canvas.y);
        word(out, "cm");
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

/// A tip's setup, with its width if the width in force is not the tip's.
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

/// `q 1 0 0 1 x y cm a b c d 0.0 0.0 cm q <colour>`, the box, `Q 1 0 0 1 -x -y cm Q`.
fn boxed<S: PdfSink>(out: &mut Vec<u8>, sink: &mut S, which: BoxRef, m: &Matrix, color: Option<Color>, ambient: Color) {
    word(out, "q 1 0 0 1");
    pos(out, m.e);
    pos(out, m.f);
    word(out, "cm");
    num(out, m.a);
    num(out, m.b);
    num(out, m.c);
    num(out, m.d);
    word(out, "0.0 0.0 cm q");
    let c = color.unwrap_or(ambient);
    color_op(out, c, true);
    color_op(out, c, false);
    sink.literal(out);
    out.clear();
    sink.place_box(which);
    word(out, "Q 1 0 0 1");
    pos(out, -m.e);
    pos(out, -m.f);
    word(out, "cm Q");
}

fn close_matrix(out: &mut Vec<u8>, mx: f64, my: f64) {
    word(out, "1 0 0 1");
    pos(out, -mx);
    pos(out, -my);
    word(out, "cm Q Q");
}

/// Write the whole picture.  `ambient` is the colour in force at the start of the picture, `scratch` a reusable buffer.
pub fn emit<S: PdfSink>(d: &Diagram, ambient: Color, scratch: &mut Vec<u8>, sink: &mut S) {
    let list = d.draw_list();
    let r = d.result();
    let out = scratch;
    out.clear();

    word(out, "q");
    color_op(out, ambient, true);
    color_op(out, ambient, false);
    word(out, "0.3985 w q");
    let cell_lw = d.cell_line_width();
    if cell_lw != 0.4 {
        bp(out, cell_lw);
        word(out, "w");
    }
    word(out, "q q 1 0 0 1");
    let (mx, my) = (r.matrix_x, r.matrix_y);
    pos(out, mx);
    pos(out, my);
    word(out, "cm 1.0 0.0 0.0 1.0 0.0 0.0 cm");

    let rt = d.font().rt;
    // The matrix's scope stays open until the first arrow or the end.
    let mut in_matrix = true;
    // The width in force, to know when a tip must set its own.
    let mut width_now = rt;
    let mut draw_color = None;
    // Inside the `q L w` of a double line, which holds the tips but not the labels.
    let mut in_double = false;
    // A label's fill leaves its `q` open over the box.
    let mut fill_open = false;

    for item in &list.items {
        if in_double && matches!(item, Item::LabelFill { .. } | Item::LabelBox { .. } | Item::ArrowEnd) {
            word(out, "Q");
            in_double = false;
        }
        if in_matrix && !matches!(item, Item::CellBorder { .. } | Item::CellBox { .. }) {
            close_matrix(out, mx, my);
            in_matrix = false;
        }
        match *item {
            Item::CellBorder { x, y, w, h, rounded, color } => {
                word(out, "q");
                if let Some(c) = color {
                    color_op(out, c, true);
                }
                match rounded {
                    Some(p) => {
                        // Relative to the matrix's origin, like everything inside its scope.
                        let t = Matrix::translate(-mx, -my);
                        for s in list.path(p) {
                            seg_ops(out, s.map(&t));
                        }
                    }
                    None => {
                        bp(out, x - mx);
                        bp(out, y - my);
                        bp(out, w);
                        bp(out, h);
                        word(out, "re");
                    }
                }
                word(out, "S Q");
            }
            Item::CellBox { cell, x, y, color } => {
                let m = Matrix::translate(x - mx, y - my);
                boxed(out, sink, BoxRef::Cell(cell), &m, color, ambient);
            }
            Item::Stroke { role: Role::Halo, path, width, color, canvas } => {
                word(out, "q q");
                if let Some(c) = color {
                    color_op(out, c, true);
                }
                bp(out, width);
                word(out, "w");
                canvas_cm(out, canvas);
                path_ops(out, list.path(path));
                word(out, "S Q Q");
            }
            Item::ArrowBegin { canvas, lw, color, draw_color: dc, opacity, dash } => {
                word(out, "q");
                bp(out, rt);
                word(out, "w");
                if lw != rt {
                    bp(out, lw);
                    word(out, "w");
                }
                width_now = lw;
                if let Some(c) = color {
                    color_op(out, c, false);
                    color_op(out, c, true);
                }
                if let Some(c) = dc {
                    color_op(out, c, true);
                }
                draw_color = dc;
                if let Some(o) = opacity {
                    out.extend_from_slice(b"/pgf@CA");
                    fixed(out, o, 5);
                    word(out, " gs");
                    out.extend_from_slice(b"/pgf@ca");
                    fixed(out, o, 5);
                    word(out, " gs");
                }
                if dash.n > 0 {
                    dash_op(out, &dash);
                }
                canvas_cm(out, canvas);
            }
            Item::Stroke { role: Role::Line, path, width, .. } => {
                if (width - width_now).abs() > 1e-9 {
                    word(out, "q");
                    bp(out, width);
                    word(out, "w");
                    width_now = width;
                    in_double = true;
                }
                path_ops(out, list.path(path));
                word(out, "S");
            }
            Item::Stroke { role: Role::Inner, path, width, color, .. } => {
                word(out, "q");
                bp(out, width);
                word(out, "w");
                if let Some(c) = color {
                    color_op(out, c, true);
                }
                path_ops(out, list.path(path));
                word(out, "S Q");
            }
            Item::Tip { m, path, width, cap, join, form } => {
                word(out, "q");
                if let Some(c) = draw_color {
                    color_op(out, c, false);
                }
                num(out, m.a);
                num(out, m.b);
                num(out, m.c);
                num(out, m.d);
                bp(out, m.e);
                bp(out, m.f);
                word(out, "cm q");
                let own = if (width - width_now).abs() > 1e-6 { Some(width) } else { None };
                tip_setup(out, form, own, cap, join);
                path_ops(out, list.path(path));
                word(out, "S Q Q");
            }
            Item::LabelFill { m, x, y, w, h, color } => {
                word(out, "q");
                color_op(out, color, false);
                word(out, "q");
                color_op(out, color, false);
                if m.is_translation() {
                    bp(out, m.e + x);
                    bp(out, m.f + y);
                    bp(out, w);
                    bp(out, h);
                    word(out, "re");
                } else {
                    // pgf's rectangle when the transform is not a shift: from the top right, counterclockwise, ending
                    // with a move to the bottom left.
                    seg_ops(out, Seg::Move(m.apply(Pt::new(x + w, y + h))));
                    seg_ops(out, Seg::Line(m.apply(Pt::new(x, y + h))));
                    seg_ops(out, Seg::Line(m.apply(Pt::new(x, y))));
                    seg_ops(out, Seg::Line(m.apply(Pt::new(x + w, y))));
                    word(out, "h");
                    seg_ops(out, Seg::Move(m.apply(Pt::new(x, y))));
                }
                word(out, "f Q");
                fill_open = true;
            }
            Item::LabelBox { label, m, color } => {
                if !fill_open {
                    word(out, "q");
                }
                boxed(out, sink, BoxRef::Label(label), &m, color, ambient);
                word(out, "Q");
                fill_open = false;
            }
            Item::ArrowEnd => word(out, "Q"),
        }
    }
    if in_matrix {
        close_matrix(out, mx, my);
    }
    word(out, "Q n Q");
    sink.literal(out);
    out.clear();
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
