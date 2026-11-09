//! What lib.rs promises, measured: once the buffers have grown, a diagram allocates nothing at all, in the
//! parser, the layout or either backend.  And how long a diagram takes.
//!
//!     cargo test --release -p commute --test alloc -- --nocapture

mod common;

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::time::Instant;

use commute::pdf::{self, PdfSink};
use commute::svg::{self, NoText};
use commute::{BoxDim, BoxRef, Color, Diagram, StandardLatex};
use common::{all_cases, boxes};

struct Counting;

thread_local! {
    static COUNT: Cell<u64> = const { Cell::new(0) };
}

// SAFETY: forwards to the system allocator; the count is a plain thread-local.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        COUNT.with(|c| c.set(c.get() + 1));
        unsafe { System.alloc(l) }
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        unsafe { System.dealloc(p, l) }
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
        COUNT.with(|c| c.set(c.get() + 1));
        unsafe { System.realloc(p, l, n) }
    }
}

#[global_allocator]
static ALLOC: Counting = Counting;

/// Keeps the stream's length, so the compiler cannot drop the backend.
#[derive(Default)]
struct Null(usize);

impl PdfSink for Null {
    fn literal(&mut self, ops: &[u8]) {
        self.0 += ops.len();
    }
    fn place_box(&mut self, _: BoxRef) {
        self.0 += 1;
    }
}

#[test]
fn nothing_is_allocated_once_the_buffers_have_grown() {
    let cases: Vec<_> = all_cases().into_iter().filter(|c| c.status == "OK").collect();
    let mut d = Diagram::new();
    let measured: Vec<(Vec<BoxDim>, Vec<BoxDim>)> = cases
        .iter()
        .map(|c| {
            d.parse(c.preset.as_bytes(), c.source.as_bytes(), c.font.unwrap(), &StandardLatex).unwrap();
            boxes(c, &d)
        })
        .collect();

    let mut scratch = Vec::new();
    let mut svg_out = Vec::new();
    let mut sink = Null::default();
    let pass = |d: &mut Diagram, scratch: &mut Vec<u8>, svg_out: &mut Vec<u8>, sink: &mut Null| {
        for (c, (cells, labels)) in cases.iter().zip(&measured) {
            d.parse(c.preset.as_bytes(), c.source.as_bytes(), c.font.unwrap(), &StandardLatex).unwrap();
            d.layout(cells, labels).unwrap();
            pdf::emit(d, Color::BLACK, scratch, sink);
            svg_out.clear();
            svg::emit(d, Color::BLACK, svg_out, &mut NoText);
        }
    };
    pass(&mut d, &mut scratch, &mut svg_out, &mut sink);
    let before = COUNT.with(Cell::get);
    pass(&mut d, &mut scratch, &mut svg_out, &mut sink);
    let n = COUNT.with(Cell::get) - before;
    println!("allocations in a second pass over {} diagrams: {n}", cases.len());
    assert_eq!(n, 0);
    assert!(sink.0 > 0);
}

/// A style with a value takes one more pass over the diagram, with its text added to the buffer that was there.
#[test]
fn nor_when_styles_take_a_value() {
    let preset = b"/tikz/.cd,w/.style={line width=#1},/tikz/commutative diagrams/.cd,lab/.style={\"#1\"},big/.style={row sep=#1}";
    let env = br#"[big=3em] A \arrow[r,w=1pt,lab=f] \arrow[d,w=2pt,lab=g] & B \\ C"#;
    let font = commute::FontParams::ARTICLE_10PT;
    let mut d = Diagram::new();
    let mut scratch = Vec::new();
    let mut sink = Null::default();
    let mut pass = |d: &mut Diagram| {
        d.parse(preset, env, font, &StandardLatex).unwrap();
        let cells = vec![BoxDim { w: 7.0, h: 7.0, d: 0.0 }; d.cells().len()];
        let labels = vec![BoxDim { w: 3.0, h: 5.0, d: 1.0 }; d.labels().len()];
        let before = COUNT.with(Cell::get);
        d.parse(preset, env, font, &StandardLatex).unwrap();
        d.layout(&cells, &labels).unwrap();
        pdf::emit(d, Color::BLACK, &mut scratch, &mut sink);
        COUNT.with(Cell::get) - before
    };
    pass(&mut d);
    assert_eq!(pass(&mut d), 0);
}

#[test]
fn time_per_diagram() {
    let cases: Vec<_> = all_cases().into_iter().filter(|c| c.status == "OK").collect();
    let mut d = Diagram::new();
    let measured: Vec<(Vec<BoxDim>, Vec<BoxDim>)> = cases
        .iter()
        .map(|c| {
            d.parse(c.preset.as_bytes(), c.source.as_bytes(), c.font.unwrap(), &StandardLatex).unwrap();
            boxes(c, &d)
        })
        .collect();
    let mut scratch = Vec::new();
    let mut sink = Null::default();
    let rounds = 200;

    let square = cases.iter().position(|c| c.file == "01-square").unwrap();
    let (c, (cells, labels)) = (&cases[square], &measured[square]);
    let n = rounds * 100;
    let t = Instant::now();
    for _ in 0..n {
        d.parse(c.preset.as_bytes(), c.source.as_bytes(), c.font.unwrap(), &StandardLatex).unwrap();
    }
    let parse = t.elapsed().as_secs_f64() / f64::from(n) * 1e6;
    let t = Instant::now();
    for _ in 0..n {
        d.layout(cells, labels).unwrap();
    }
    let layout = t.elapsed().as_secs_f64() / f64::from(n) * 1e6;
    let t = Instant::now();
    for _ in 0..n {
        pdf::emit(&d, Color::BLACK, &mut scratch, &mut sink);
    }
    let emit = t.elapsed().as_secs_f64() / f64::from(n) * 1e6;
    let per_square = parse + layout + emit;
    println!("the square of test 01: parse {parse:.2} us, layout {layout:.2} us, PDF {emit:.2} us");

    let t = Instant::now();
    for _ in 0..rounds {
        for (c, (cells, labels)) in cases.iter().zip(&measured) {
            d.parse(c.preset.as_bytes(), c.source.as_bytes(), c.font.unwrap(), &StandardLatex).unwrap();
            d.layout(cells, labels).unwrap();
            pdf::emit(&d, Color::BLACK, &mut scratch, &mut sink);
        }
    }
    let per_diagram = t.elapsed().as_secs_f64() / f64::from(rounds) / cases.len() as f64 * 1e6;
    println!("parse + layout + PDF: the square of test 01 {per_square:.2} us, mean over {} diagrams {per_diagram:.2} us", cases.len());
    assert!(sink.0 > 0);
    // A bound on the order of magnitude, for slow machines and unoptimised builds too: "a diagram takes microseconds".
    assert!(per_diagram < 1000.0, "{per_diagram} us");
}
