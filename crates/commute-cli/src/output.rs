//! Writing one diagram: a cropped single-page PDF, and an SVG.  The PDF is plain and uncompressed, with base fonts.
//! Both are placeholders for text only (see typeset.rs); the strokes are commute's real backends.

use std::io::Write;

use commute::pdf::{self, PdfSink};
use commute::svg::{self, SvgText};
use commute::{BoxRef, Color, Diagram, Matrix};

use crate::typeset::{Face, TypesetBox};

/// pgf's factor for its own numbers; the page is measured the same way (spec section 2).
const BP: f64 = 0.99627;
/// pdfTeX's for text.
const TRUE_BP: f64 = 7200.0 / 7227.0;

/// A number with up to 5 decimals.
fn n(v: f64) -> String {
    let s = format!("{:.5}", if v.is_finite() { v } else { 0.0 });
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" || s.is_empty() { "0".to_string() } else { s.to_string() }
}

fn font_name(f: Face) -> &'static str {
    match f {
        Face::Italic => "F1",
        Face::Roman => "F2",
        Face::Symbol => "F3",
    }
}

struct PageSink<'a> {
    out: Vec<u8>,
    cells: &'a [TypesetBox],
    labels: &'a [TypesetBox],
}

impl PdfSink for PageSink<'_> {
    fn literal(&mut self, ops: &[u8]) {
        self.out.extend_from_slice(ops);
        self.out.push(b'\n');
    }

    fn place_box(&mut self, which: BoxRef) {
        let b = match which {
            BoxRef::Cell(i) => self.cells.get(i as usize),
            BoxRef::Label(i) => self.labels.get(i as usize),
        };
        let Some(b) = b else { return };
        for g in &b.glyphs {
            let _ = writeln!(
                self.out,
                "BT /{} {} Tf {} {} Td <{:02X}> Tj ET",
                font_name(g.face),
                n(g.size * TRUE_BP),
                n(g.x * TRUE_BP),
                n(g.y * TRUE_BP),
                g.pdf
            );
        }
    }
}

/// The PDF of a laid-out diagram.
pub fn pdf_bytes(d: &Diagram, cells: &[TypesetBox], labels: &[TypesetBox]) -> Vec<u8> {
    let r = *d.result();
    let (w, h) = ((r.maxx - r.minx) * BP, (r.maxy - r.miny) * BP);
    let mut sink = PageSink { out: Vec::new(), cells, labels };
    let _ = writeln!(sink.out, "1 0 0 1 {} {} cm", n(-r.minx * BP), n(-r.miny * BP));
    let mut scratch = Vec::new();
    pdf::emit(d, Color::BLACK, &mut scratch, &mut sink);
    let stream = sink.out;

    let mut gs = String::new();
    let mut seen: Vec<u64> = Vec::new();
    for o in d.opacities() {
        if seen.contains(&o.to_bits()) {
            continue;
        }
        seen.push(o.to_bits());
        gs.push_str(&format!("/pgf@CA{0} << /CA {0} >> /pgf@ca{0} << /ca {0} >> ", n(o)));
    }

    let mut objects: Vec<Vec<u8>> = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {} {}] /Contents 4 0 R /Resources << /Font << /F1 5 0 R /F2 6 0 R /F3 7 0 R >> /ExtGState << {gs}>> >> >>",
            n(w),
            n(h)
        )
        .into_bytes(),
    ];
    let mut contents = format!("<< /Length {} >>\nstream\n", stream.len()).into_bytes();
    contents.extend_from_slice(&stream);
    contents.extend_from_slice(b"\nendstream");
    objects.push(contents);
    for base in ["Times-Italic", "Times-Roman", "Symbol"] {
        objects.push(format!("<< /Type /Font /Subtype /Type1 /BaseFont /{base} >>").into_bytes());
    }

    let mut out = b"%PDF-1.5\n".to_vec();
    let mut offsets = Vec::new();
    for (i, o) in objects.iter().enumerate() {
        offsets.push(out.len());
        let _ = writeln!(out, "{} 0 obj", i + 1);
        out.extend_from_slice(o);
        out.extend_from_slice(b"\nendobj\n");
    }
    let xref = out.len();
    let _ = write!(out, "xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1);
    for off in offsets {
        let _ = writeln!(out, "{off:010} 00000 n ");
    }
    let _ = write!(out, "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objects.len() + 1);
    out
}

struct SvgBoxes<'a> {
    cells: &'a [TypesetBox],
    labels: &'a [TypesetBox],
}

fn xml_char(out: &mut Vec<u8>, c: char) {
    let _ = match c {
        '&' => out.write_all(b"&amp;"),
        '<' => out.write_all(b"&lt;"),
        '>' => out.write_all(b"&gt;"),
        _ => write!(out, "{c}"),
    };
}

impl SvgText for SvgBoxes<'_> {
    fn place_box(&mut self, out: &mut Vec<u8>, which: BoxRef, t: &Matrix, fill: &str) {
        let b = match which {
            BoxRef::Cell(i) => self.cells.get(i as usize),
            BoxRef::Label(i) => self.labels.get(i as usize),
        };
        let Some(b) = b else { return };
        if b.glyphs.is_empty() {
            return;
        }
        let _ = writeln!(out, "<g transform=\"matrix({} {} {} {} {} {})\" fill=\"{fill}\" font-family=\"serif\">", n(t.a), n(t.b), n(t.c), n(t.d), n(t.e), n(t.f));
        for g in &b.glyphs {
            let style = if g.face == Face::Italic { " font-style=\"italic\"" } else { "" };
            let _ = write!(out, "<text x=\"{}\" y=\"{}\" font-size=\"{}\"{style}>", n(g.x), n(-g.y), n(g.size));
            xml_char(out, g.unicode);
            let _ = writeln!(out, "</text>");
        }
        let _ = writeln!(out, "</g>");
    }
}

/// The SVG of a laid-out diagram.
pub fn svg_bytes(d: &Diagram, cells: &[TypesetBox], labels: &[TypesetBox]) -> Vec<u8> {
    let mut out = Vec::new();
    svg::emit(d, Color::BLACK, &mut out, &mut SvgBoxes { cells, labels });
    out
}
