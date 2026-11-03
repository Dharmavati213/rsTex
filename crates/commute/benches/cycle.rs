//! One cycle of the library, parse, layout and PDF emission, on a square and on a large diagram, with one [`Diagram`] kept
//! between cycles as the engine keeps it.
//!
//!     cargo bench -p commute --bench cycle                     time per cycle
//!     target/release/deps/cycle-* square 100000                that many cycles, for an outside instruction count
//!
//! `tests/corpus/cd/library_cost.py` runs it under user-mode counters: instructions per cycle, with start-up taken out.

use std::hint::black_box;
use std::time::Instant;

use commute::pdf::{self, PdfSink};
use commute::{AllowAll, BoxDim, BoxRef, Color, Diagram, FontParams};

const SQUARE: &str = r#"A \arrow[r,"f"] \arrow[d,"g"'] & B \arrow[d,"h"] \\ C \arrow[r,"i"'] & D"#;

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

/// `n` rows and columns, an arrow to the right and one down from each cell, each with a label.
fn grid(n: usize) -> String {
    let mut s = String::new();
    for r in 0..n {
        for c in 0..n {
            if c > 0 {
                s.push_str(" & ");
            }
            s.push_str("X_{a}");
            if c + 1 < n {
                s.push_str(r#" \arrow[r,"f"]"#);
            }
            if r + 1 < n {
                s.push_str(r#" \arrow[d,"g"']"#);
            }
        }
        if r + 1 < n {
            s.push_str(" \\\\\n");
        }
    }
    s
}

fn cycles(source: &str, n: u64) -> usize {
    let font = FontParams::ARTICLE_10PT;
    let mut d = Diagram::new();
    d.parse(b"", source.as_bytes(), font, &AllowAll).unwrap();
    let cell = BoxDim { w: 7.5, h: 6.8, d: 0.0 };
    let label = BoxDim { w: 4.2, h: 4.9, d: 1.4 };
    let cells = vec![cell; d.cells().len()];
    let labels = vec![label; d.labels().len()];
    let mut scratch = Vec::new();
    let mut sink = Null::default();
    for _ in 0..n {
        d.parse(b"", black_box(source.as_bytes()), font, &AllowAll).unwrap();
        d.layout(&cells, &labels).unwrap();
        pdf::emit(&d, Color::BLACK, &mut scratch, &mut sink);
    }
    black_box(sink.0)
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).filter(|a| !a.starts_with("--")).collect();
    if let [which, n] = &args[..] {
        let source = if which == "square" { SQUARE.to_string() } else { grid(which.parse().unwrap()) };
        cycles(&source, n.parse().unwrap());
        return;
    }
    for (name, source, n) in [("square", SQUARE.to_string(), 20_000), ("10x10", grid(10), 500), ("40x40", grid(40), 20)] {
        let t = Instant::now();
        cycles(&source, n);
        println!("{name}: {:.1} us per cycle", t.elapsed().as_secs_f64() / n as f64 * 1e6);
    }
}
