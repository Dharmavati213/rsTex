//! SVG output.  One `<path>` per stroke, in drawing order, in points, cropped to the picture's box.
//!
//! Text is the caller's: [`SvgText::place_box`] writes what a box looks like, under a `transform` the backend gives it.
//! The commute tool writes `<text>` for now; DESIGN section 10 wants glyph outlines at M6.

use crate::BoxRef;
use crate::color::Color;
use crate::diagram::{Diagram, Item, Layout};
use crate::geom::{Matrix, Pt, Seg};
use crate::number::fixed;

/// How the caller draws cells and labels.
pub trait SvgText {
    /// Write the box.  In `transform`'s frame the box's reference point is the origin, x runs right and y runs down, as
    /// SVG text expects.  `fill` is the text colour as `#rrggbb`.
    fn place_box(&mut self, out: &mut Vec<u8>, which: BoxRef, transform: &Matrix, fill: &str);
}

/// The caller draws no text.
#[derive(Copy, Clone, Debug, Default)]
pub struct NoText;

impl SvgText for NoText {
    fn place_box(&mut self, _: &mut Vec<u8>, _: BoxRef, _: &Matrix, _: &str) {}
}

const DECIMALS: u32 = 4;

fn put(out: &mut Vec<u8>, s: &str) {
    out.extend_from_slice(s.as_bytes());
}

fn n(out: &mut Vec<u8>, v: f64) {
    fixed(out, v, DECIMALS);
}

/// `#rrggbb` for a colour.  CMYK goes through the naive conversion; SVG has no other.
pub fn hex(c: Color) -> [u8; 7] {
    let (r, g, b) = match c {
        Color::Gray(v) => (v, v, v),
        Color::Rgb(r, g, b) => (r, g, b),
        Color::Cmyk(c, m, y, k) => ((1.0 - c) * (1.0 - k), (1.0 - m) * (1.0 - k), (1.0 - y) * (1.0 - k)),
    };
    const D: &[u8; 16] = b"0123456789abcdef";
    let byte = |v: f64| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
    let (r, g, b) = (byte(r), byte(g), byte(b));
    [
        b'#',
        D[usize::from(r >> 4)],
        D[usize::from(r & 15)],
        D[usize::from(g >> 4)],
        D[usize::from(g & 15)],
        D[usize::from(b >> 4)],
        D[usize::from(b & 15)],
    ]
}

fn hex_str(c: Color, buf: &mut [u8; 7]) -> &str {
    *buf = hex(c);
    // Seven ASCII bytes by construction.
    std::str::from_utf8(&buf[..]).unwrap_or("#000000")
}

/// Picture coordinates to SVG's.
#[inline]
fn view(r: &Layout, p: Pt) -> Pt {
    Pt::new(p.x - r.minx, r.maxy - p.y)
}

fn d_attr(out: &mut Vec<u8>, r: &Layout, segs: &[Seg], m: &Matrix) {
    put(out, " d=\"");
    let pt = |out: &mut Vec<u8>, p: Pt| {
        let v = view(r, m.apply(p));
        n(out, v.x);
        out.push(b' ');
        n(out, v.y);
    };
    for (i, s) in segs.iter().enumerate() {
        if i > 0 {
            out.push(b' ');
        }
        match *s {
            Seg::Move(p) => {
                out.extend_from_slice(b"M ");
                pt(out, p);
            }
            Seg::Line(p) => {
                out.extend_from_slice(b"L ");
                pt(out, p);
            }
            Seg::Curve(a, b, c) => {
                out.extend_from_slice(b"C ");
                pt(out, a);
                out.push(b' ');
                pt(out, b);
                out.push(b' ');
                pt(out, c);
            }
            Seg::Close => out.push(b'Z'),
        }
    }
    out.push(b'"');
}

/// The scope of the current arrow.
#[derive(Copy, Clone)]
struct Scope {
    canvas: Pt,
    stroke: Color,
    opacity: Option<f64>,
    dash: crate::spec::Dash,
}

fn stroke_attrs(out: &mut Vec<u8>, stroke: Color, width: f64, cap: u8, join: u8, sc: &Scope, dashed: bool) {
    let mut h = [0u8; 7];
    put(out, " fill=\"none\" stroke=\"");
    put(out, hex_str(stroke, &mut h));
    put(out, "\" stroke-width=\"");
    n(out, width);
    out.push(b'"');
    if cap == 1 {
        put(out, " stroke-linecap=\"round\"");
    }
    if join == 1 {
        put(out, " stroke-linejoin=\"round\"");
    }
    if dashed && sc.dash.n > 0 {
        put(out, " stroke-dasharray=\"");
        for (i, &v) in sc.dash.values().iter().enumerate() {
            if i > 0 {
                out.push(b' ');
            }
            n(out, v);
        }
        out.push(b'"');
    }
    if let Some(o) = sc.opacity {
        put(out, " opacity=\"");
        n(out, o);
        out.push(b'"');
    }
}

/// Write the picture as a standalone SVG document into `out`.
pub fn emit<T: SvgText>(d: &Diagram, ambient: Color, out: &mut Vec<u8>, text: &mut T) {
    let list = d.draw_list();
    let r = *d.result();
    put(out, "<svg xmlns=\"http://www.w3.org/2000/svg\" version=\"1.1\" width=\"");
    n(out, r.width);
    put(out, "pt\" height=\"");
    n(out, r.maxy - r.miny);
    put(out, "pt\" viewBox=\"0 0 ");
    n(out, r.width);
    out.push(b' ');
    n(out, r.maxy - r.miny);
    // PDF's default miter limit, where SVG's is 4: a squiggly line's corners must not turn into bevels.
    put(out, "\" stroke-miterlimit=\"10\">\n");

    let plain = Scope { canvas: Pt::ZERO, stroke: ambient, opacity: None, dash: Default::default() };
    let mut sc = plain;
    let mut text_color = ambient;
    let mut h = [0u8; 7];
    for item in &list.items {
        match *item {
            Item::CellBorder { x, y, w, h: ht, rounded, color, .. } => {
                put(out, "<path");
                match rounded {
                    Some(p) => d_attr(out, &r, list.path(p), &Matrix::IDENTITY),
                    None => {
                        let rect = [
                            Seg::Move(Pt::new(x, y)),
                            Seg::Line(Pt::new(x + w, y)),
                            Seg::Line(Pt::new(x + w, y + ht)),
                            Seg::Line(Pt::new(x, y + ht)),
                            Seg::Close,
                        ];
                        d_attr(out, &r, &rect, &Matrix::IDENTITY);
                    }
                }
                stroke_attrs(out, color.unwrap_or(ambient), d.cell_line_width(), 0, 0, &plain, false);
                put(out, "/>\n");
            }
            Item::CellBox { cell, x, y, color, .. } => {
                let v = view(&r, Pt::new(x, y));
                let t = Matrix::translate(v.x, v.y);
                let fill = hex_str(color.unwrap_or(ambient), &mut h);
                text.place_box(out, BoxRef::Cell(cell), &t, fill);
            }
            Item::ArrowBegin { canvas, color, draw_color, opacity, dash, .. } => {
                sc = Scope { canvas, stroke: draw_color.or(color).unwrap_or(ambient), opacity, dash };
                text_color = color.unwrap_or(ambient);
            }
            Item::Stroke { role, path, width, color, canvas } => {
                put(out, "<path");
                d_attr(out, &r, list.path(path), &Matrix::translate(canvas.x, canvas.y));
                let dashed = !matches!(role, crate::diagram::Role::Halo);
                stroke_attrs(out, color.unwrap_or(sc.stroke), width, 0, 0, &sc, dashed);
                put(out, "/>\n");
            }
            Item::Tip { m, path, width, cap, join, .. } => {
                let mut world = m;
                world.e += sc.canvas.x;
                world.f += sc.canvas.y;
                put(out, "<path");
                d_attr(out, &r, list.path(path), &world);
                stroke_attrs(out, sc.stroke, width, cap, join, &sc, false);
                put(out, "/>\n");
            }
            Item::LabelFill { m, x, y, w, h: ht, color, .. } => {
                let mut world = m;
                world.e += sc.canvas.x;
                world.f += sc.canvas.y;
                let rect = [
                    Seg::Move(Pt::new(x, y)),
                    Seg::Line(Pt::new(x + w, y)),
                    Seg::Line(Pt::new(x + w, y + ht)),
                    Seg::Line(Pt::new(x, y + ht)),
                    Seg::Close,
                ];
                put(out, "<path");
                d_attr(out, &r, &rect, &world);
                put(out, " fill=\"");
                put(out, hex_str(color, &mut h));
                put(out, "\"/>\n");
            }
            Item::LabelBox { label, m, color } => {
                let v = view(&r, Pt::new(m.e + sc.canvas.x, m.f + sc.canvas.y));
                // y runs down inside: x' = a x - c y, y' = -b x + d y.
                let t = Matrix { a: m.a, b: -m.b, c: -m.c, d: m.d, e: v.x, f: v.y };
                let fill = hex_str(color.unwrap_or(text_color), &mut h);
                text.place_box(out, BoxRef::Label(label), &t, fill);
            }
            Item::ArrowEnd => sc = plain,
            Item::CellEmpty { .. } | Item::Coordinates { .. } => {}
        }
    }
    put(out, "</svg>\n");
}
