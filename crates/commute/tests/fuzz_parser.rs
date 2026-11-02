//! No input panics, hangs or runs away: random bytes, token soup and mutated real diagrams go through the parser, layout
//! with degenerate measurements, and both backends.  Deterministic: the same seed gives the same inputs.  cargo-fuzz would
//! need a C++ runtime, which the project does not take, so this is a plain test.
//!
//!     cargo test -p commute --test fuzz_parser -- --nocapture
//!     COMMUTE_FUZZ_ITERS=20000000 cargo test --release -p commute --test fuzz_parser -- --nocapture

mod common;

use commute::pdf::{self, Collect};
use commute::svg::{self, NoText};
use commute::{AllowAll, BoxDim, Color, Diagram, FontParams, Policy, StandardLatex};
use common::all_cases;

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        // xorshift64*
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }

    fn pick<'a>(&mut self, xs: &[&'a str]) -> &'a str {
        xs[self.below(xs.len())]
    }
}

/// Fragments that mean something to the parser.
const SOUP: &[&str] = &[
    "\\arrow", "\\ar", "\\rar", "\\lar", "\\dar", "\\uar", "\\urar", "\\ular", "\\drar", "\\dlar", "[", "]", "{", "}", "(", ")",
    "\"", "'", ",", "=", "&", "\\\\", "|", "|[", "]|", " ", "\n", "\n\n", "%", "\\", "#", "$", "^", "_", "~",
    "\\begin{matrix}", "\\end{matrix}", "\\begin", "\\end", "\\begin{tikzcd}", "\\end{tikzcd}", "\\par", "\\foo", "\\alpha", "\\&", "\\,",
    "r", "d", "l", "u", "rr", "dl", "A", "B", "1", "0", ".5", "-", "+", "*", "/", "1pt", "2ex", "3em", "1cm", "1e9", "99999999pt", "0pt",
    "to", "from", "to=", "from=", "1-2", "2-1", "X", "alias=X", "name=X",
    "shift left", "shift right", "shift left=", "shift={(1pt,2pt)}", "xshift=", "yshift=1pt", "transform canvas=", "{xshift=1pt}",
    "bend left", "bend right=", "in=", "out=", "looseness=", "in looseness=", "distance=", "min distance=", "max distance=", "loop", "loop right",
    "start anchor=", "end anchor=", "{[xshift=1ex]north}", "north east", "base", "mid west", "real center", "30", "-400", "1e300",
    "Rightarrow", "hook", "hook'", "two heads", "tail", "harpoon", "harpoon'", "maps to", "dash", "equal", "mapsto", "Mapsto", "rightsquigarrow",
    "squiggly", "squiggly={pre length=0pt}", "squiggly={post length=1000pt}", "dashed", "dotted", "dash pattern=on 1pt off 2pt", "dash pattern=",
    "line width=", "line width=0pt", "line width=.00001pt", "thick", "double", "double distance=", "double line", "draw=none", "draw=red", "red", "red!50!blue", "color=",
    "crossing over", "opacity=", "shorten=", "shorten <=", "shorten >=", "phantom", "description", "marking", "sloped", "swap",
    "near start", "pos=", "above", "below=1pt", "anchor=", "rotate=", "fill=", "font=", "font=\\tiny", "text=blue", "inner sep=", "outer sep=", "minimum width=",
    "row sep=", "column sep=", "sep=", "row sep=huge", "between origins", "between borders", "cramped", "cells=", "arrows=", "labels=", "nodes=", "diagrams=",
    "every arrow/.append style=", "every label/.append style=", "every cell/.append style=", "every matrix/.append style=", "every diagram/.append style=", "a/.style=", "a/.append style=", "a", "b/.style={a,a}", "a/.style={b,b}", "#1",
    "math mode=false", "ampersand replacement=\\&", "background color=", "crossing over clearance=", "baseline=", "baseline=(X.base)", "baseline=(current bounding box.center)",
    "baseline=(\\tikzcdmatrixname-1-1.north)", "arrow style=tikz", "arrow style=Latin Modern", "nodes in empty cells", "text height=", "text depth=", "draw", "rounded corners", "rounded corners=1000pt",
];

fn soup(r: &mut Rng) -> Vec<u8> {
    let n = r.below(40);
    let mut s = String::new();
    if r.below(2) == 0 {
        s.push('[');
    }
    for _ in 0..n {
        s.push_str(r.pick(SOUP));
    }
    s.into_bytes()
}

/// A well-formed diagram with random keys, so that most of them reach layout.
fn plausible(r: &mut Rng) -> Vec<u8> {
    let mut s = String::new();
    let keys = |r: &mut Rng, s: &mut String| {
        for _ in 0..r.below(4) {
            s.push_str(r.pick(SOUP));
            if r.below(2) == 0 {
                s.push_str(r.pick(&["1pt", "2", "30", ".5", "-1ex", "red", "none", "north", "{(1pt,1pt)}", "1000000pt", "0pt"]));
            }
            s.push(',');
        }
    };
    s.push('[');
    keys(r, &mut s);
    s.push(']');
    let (rows, cols) = (1 + r.below(3), 1 + r.below(3));
    for i in 0..rows {
        for j in 0..cols {
            if j > 0 {
                s.push('&');
            }
            if r.below(4) > 0 {
                s.push_str("A ");
            }
            for _ in 0..r.below(3) {
                s.push_str("\\arrow[");
                s.push_str(r.pick(&["r", "d", "l", "u", "rd", "ul", "", "loop", "to=1-1", "from=1-1", "bend left"]));
                s.push(',');
                keys(r, &mut s);
                if r.below(2) == 0 {
                    s.push_str("\"f\"");
                    if r.below(2) == 0 {
                        s.push_str("{name=X,");
                        keys(r, &mut s);
                        s.push('}');
                    }
                }
                s.push(']');
            }
        }
        if i + 1 < rows {
            s.push_str("\\\\");
        }
    }
    s.into_bytes()
}

fn mutate(r: &mut Rng, base: &[u8]) -> Vec<u8> {
    let mut v = base.to_vec();
    for _ in 0..1 + r.below(6) {
        if v.is_empty() {
            break;
        }
        let i = r.below(v.len());
        match r.below(6) {
            0 => v[i] = (r.next() & 0xff) as u8,
            1 => {
                v.remove(i);
            }
            2 => v.insert(i, *b"\\{}[]\"&,=|%#".get(r.below(12)).unwrap()),
            3 => {
                let t = r.pick(SOUP).as_bytes().to_vec();
                v.splice(i..i, t);
            }
            4 => {
                let j = i + r.below((v.len() - i).min(40));
                let seg = v[i..j].to_vec();
                v.splice(j..j, seg);
            }
            _ => v.truncate(i),
        }
    }
    v
}

fn measure(r: &mut Rng) -> BoxDim {
    let pick = |r: &mut Rng| match r.below(12) {
        0 => 0.0,
        1 => f64::NAN,
        2 => f64::INFINITY,
        3 => f64::NEG_INFINITY,
        4 => 1e300,
        5 => -1e300,
        6 => 16383.99999,
        7 => -5.0,
        _ => (r.next() % 2000) as f64 / 100.0,
    };
    BoxDim { w: pick(r), h: pick(r), d: pick(r) }
}

/// Every stage, on one input.
#[allow(clippy::too_many_arguments)]
fn run(d: &mut Diagram, r: &mut Rng, input: &[u8], preset: &[u8], font: FontParams, policy: &dyn Policy, pdf_buf: &mut Vec<u8>, stats: &mut [u64; 3]) {
    if d.parse(preset, input, font, policy).is_err() {
        return;
    }
    stats[0] += 1;
    let cells: Vec<BoxDim> = d.cells().iter().map(|_| measure(r)).collect();
    let labels: Vec<BoxDim> = d.labels().iter().map(|_| measure(r)).collect();
    for s in 0..d.cells().len() {
        let _ = d.cell_content(s).count();
    }
    if d.layout(&cells, &labels).is_err() {
        return;
    }
    stats[1] += 1;
    let mut sink = Collect::default();
    pdf::emit(d, Color::BLACK, pdf_buf, &mut sink);
    let mut depth = 0i64;
    for t in sink.ops.split(|c| *c == b' ' || *c == b'\n') {
        match t {
            b"q" => depth += 1,
            b"Q" => depth -= 1,
            _ => {}
        }
        assert!(depth >= 0, "unbalanced: {}", String::from_utf8_lossy(input));
    }
    assert_eq!(depth, 0, "unbalanced: {}", String::from_utf8_lossy(input));
    let mut out = Vec::new();
    svg::emit(d, Color::BLACK, &mut out, &mut NoText);
    assert!(out.ends_with(b"</svg>\n"));
    stats[2] += 1;
}

#[test]
fn no_input_panics() {
    let iters: u64 = std::env::var("COMMUTE_FUZZ_ITERS").ok().and_then(|s| s.parse().ok()).unwrap_or(150_000);
    let corpus: Vec<(Vec<u8>, Vec<u8>)> = all_cases().into_iter().map(|c| (c.preset.into_bytes(), c.source.into_bytes())).collect();
    let mut r = Rng(0x9E37_79B9_7F4A_7C15);
    let mut d = Diagram::new();
    let mut pdf_buf = Vec::new();
    let mut stats = [0u64; 3];
    let fonts = [
        FontParams::ARTICLE_10PT,
        FontParams { ex: 0.0, em: 0.0, rt: 0.0, ax: 0.0 },
        FontParams { ex: 1e9, em: 1e9, rt: 1e-9, ax: -1e9 },
        FontParams { ex: f64::NAN, em: f64::INFINITY, rt: f64::NAN, ax: f64::NEG_INFINITY },
    ];
    for i in 0..iters {
        let font = if i % 8 == 0 { fonts[r.below(fonts.len())] } else { FontParams::ARTICLE_10PT };
        let (input, preset): (Vec<u8>, Vec<u8>) = match i % 8 {
            0 => {
                let n = r.below(120);
                ((0..n).map(|_| (r.next() & 0xff) as u8).collect(), Vec::new())
            }
            1 | 2 => (soup(&mut r), Vec::new()),
            3 => (soup(&mut r), soup(&mut r)),
            4 | 5 => (plausible(&mut r), Vec::new()),
            _ => {
                let (p, b) = &corpus[r.below(corpus.len())];
                (mutate(&mut r, b), p.clone())
            }
        };
        if i % 2 == 0 {
            run(&mut d, &mut r, &input, &preset, font, &StandardLatex, &mut pdf_buf, &mut stats);
        } else {
            run(&mut d, &mut r, &input, &preset, font, &AllowAll, &mut pdf_buf, &mut stats);
        }
    }
    println!("fuzz: {iters} inputs, {} parsed, {} laid out, {} through both backends", stats[0], stats[1], stats[2]);
    assert!(stats[2] > iters / 50, "too few inputs reach the backends: {stats:?}");
}

/// Adversarial shapes, each once.
#[test]
fn pathological_inputs_are_refused_or_survive() {
    let mut d = Diagram::new();
    let font = FontParams::ARTICLE_10PT;
    let long = |s: &str, n: usize| s.repeat(n);
    let inputs: Vec<String> = vec![
        long("{", 200_000),
        long("}", 200_000),
        long("[", 200_000),
        long("\\", 200_001),
        long("&", 200_000),
        long("\\\\", 100_000),
        long("\\arrow[r]", 20_000),
        long("\\arrow[", 20_000),
        format!("\\arrow[r,{}]", long("r", 200_000)),
        format!("\\arrow[{}]", long("l,", 100_000)),
        format!("\\arrow[r,shorten={}1pt{}]", long("(", 50_000), long(")", 50_000)),
        format!("\\arrow[r,shorten={}1pt]", long("-", 100_000)),
        format!("\\arrow[{}r{}]", long("nodes={", 10_000), long("}", 10_000)),
        format!("[cells={{{}inner sep=1pt{}}}]A", long("nodes={", 10_000), long("}", 10_000)),
        r"[a/.style={a}]\arrow[a]".to_string(),
        r"[a/.style={b,b,b,b,b,b,b,b},b/.style={c,c,c,c,c,c,c,c},c/.style={d,d,d,d,d,d,d,d},d/.style={e,e,e,e,e,e,e,e},e/.style={f,f,f,f,f,f,f,f},f/.style={g,g,g,g,g,g,g,g},g/.style={r}]\arrow[a]".to_string(),
        r"[column sep=100000pt]A\arrow[r,squiggly,line width=.00001pt]&B".to_string(),
        r"[column sep=100000pt]A\arrow[r,squiggly,line width=0pt]&B".to_string(),
        r"[column sep=100000pt]A\arrow[r,squiggly,bend left,line width=.00001pt]&B".to_string(),
        r"A\arrow[r,start anchor=1e300]&B".to_string(),
        format!("A\\arrow[r,in={},out=9]&B", long("9", 400)),
        format!("A\\arrow[{}]", long("to=1-1,", 10_000)),
        long("\\begin{matrix}", 100),
        long("\\end{matrix}", 100),
        String::from_utf8_lossy(&[0xff, 0xfe, b'\\', 0xff, b'{', 0x80, b'}']).into_owned(),
    ];
    let mut scratch = Vec::new();
    let mut out = Vec::new();
    for s in &inputs {
        for policy in [&StandardLatex as &dyn Policy, &AllowAll] {
            if d.parse(b"", s.as_bytes(), font, policy).is_ok() {
                let cells = vec![BoxDim { w: 5.0, h: 5.0, d: 1.0 }; d.cells().len()];
                let labels = vec![BoxDim::default(); d.labels().len()];
                if d.layout(&cells, &labels).is_ok() {
                    let mut sink = Collect::default();
                    pdf::emit(&d, Color::BLACK, &mut scratch, &mut sink);
                    out.clear();
                    svg::emit(&d, Color::BLACK, &mut out, &mut NoText);
                }
            }
        }
    }
}
