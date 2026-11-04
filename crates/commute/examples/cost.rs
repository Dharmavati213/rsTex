//! A typical diagram through the library, COUNT times, for `tools/prof/native_cost.py`: parse, layout and PDF operators, with boxes of a size a cell has.
//!
//!     cargo run --release -p commute --example cost -- COUNT

use commute::pdf::{self, PdfSink};
use commute::{AllowAll, BoxDim, BoxRef, Color, Diagram, FontParams};

const BODY: &[u8] = br#"
  A \arrow[r, "f"] \arrow[d, "g"'] & B \arrow[d, "h"] \arrow[r, hook] & C \arrow[d, "k", two heads] \\
  D \arrow[r, "i"'] & E \arrow[r, Rightarrow] & F
"#;

struct Null(usize);

impl PdfSink for Null {
    fn literal(&mut self, ops: &[u8]) {
        self.0 += ops.len();
    }

    fn place_box(&mut self, _: BoxRef) {
        self.0 += 1;
    }
}

fn main() {
    let n: usize = std::env::args().nth(1).and_then(|a| a.parse().ok()).unwrap_or(1000);
    let mut d = Diagram::new();
    let mut ops = Vec::new();
    let mut total = 0;
    for _ in 0..n {
        d.parse(&[], BODY, FontParams::ARTICLE_10PT, &AllowAll).unwrap();
        let cells: Vec<BoxDim> = d.cells().iter().map(|_| BoxDim { w: 7.5, h: 6.8, d: 0.0 }).collect();
        let labels: Vec<BoxDim> = d.labels().iter().map(|_| BoxDim { w: 4.0, h: 4.7, d: 1.3 }).collect();
        d.layout(&cells, &labels).unwrap();
        let mut sink = Null(0);
        pdf::emit(&d, Color::Gray(0.0), &mut ops, &mut sink);
        total += sink.0;
    }
    println!("{} bytes and boxes", total / n.max(1));
}
