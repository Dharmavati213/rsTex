//! The PDF backend writes an arrow's operators in pgf's form (spec section 15): for every diagram in pdf_forms.tsv, the
//! same operators in the same order, and numbers within the spec's tolerance.
//!
//!     cargo test -p commute --test pdf_forms -- --nocapture

mod common;

use commute::pdf::{self, Collect};
use commute::{Color, Diagram, StandardLatex};
use common::{all_cases, boxes, dir};

const STROKE_BP: f64 = 0.031;

fn arrows_of(c: &common::Case, d: &mut Diagram) -> String {
    d.parse(c.preset.as_bytes(), c.source.as_bytes(), c.font.unwrap(), &StandardLatex).unwrap();
    let (cells, labels) = boxes(c, d);
    d.layout(&cells, &labels).unwrap();
    let mut sink = Collect::default();
    let mut scratch = Vec::new();
    pdf::emit(d, Color::BLACK, &mut scratch, &mut sink);
    let s = String::from_utf8(sink.ops).unwrap();
    // After the matrix's scope, up to the end of the picture.  It closes where the depth comes back to the picture's two.
    let at = s.rfind("%cell").unwrap_or(0);
    let mut depth = s[..at].split_whitespace().fold(0, |d, t| match t {
        "q" => d + 1,
        "Q" => d - 1,
        _ => d,
    });
    let mut from = at;
    for t in s[at..].split_inclusive(char::is_whitespace) {
        from += t.len();
        match t.trim() {
            "q" => depth += 1,
            "Q" => depth -= 1,
            _ => {}
        }
        if depth == 2 {
            break;
        }
    }
    let tail = &s[from..];
    // The picture's own end, and the position pdfTeX writes in front of it, if any.
    let tail = tail[..tail.rfind("Q \nn \nQ").unwrap_or(tail.len())].trim();
    let tail = tail.strip_prefix("1 0 0 1").map_or(tail, |t| t[t.find("cm").map_or(0, |i| i + 2)..].trim());
    tail.to_string()
}

#[test]
fn single_arrows_read_like_pgf() {
    let cases = all_cases();
    let table = std::fs::read_to_string(dir().join("pdf_forms.tsv")).unwrap();
    let mut d = Diagram::new();
    let (mut compared, mut identical, mut worst, mut tokens, mut same_tokens) = (0, 0, 0.0f64, 0, 0);
    let mut failures = Vec::new();
    for line in table.lines() {
        let r: Vec<&str> = line.split('\t').collect();
        let (file, index, want) = (r[0], r[1].parse::<usize>().unwrap(), r[2]);
        let c = cases.iter().find(|c| c.file == file && c.index == index).unwrap();
        let got = arrows_of(c, &mut d);
        compared += 1;
        // The table has no text of labels, and the stream marks each box with a comment.
        let g: Vec<&str> = got.split_whitespace().filter(|t| !t.starts_with('%')).collect();
        let w: Vec<&str> = want.split_whitespace().collect();
        if g == w {
            identical += 1;
        }
        if g.len() != w.len() {
            failures.push(format!("{file}#{index}: {} tokens, pgf {}\n  got  {}\n  want {want}", g.len(), w.len(), g.join(" ")));
            continue;
        }
        for (a, b) in g.iter().zip(&w) {
            tokens += 1;
            if a == b {
                same_tokens += 1;
                continue;
            }
            match (a.parse::<f64>(), b.parse::<f64>()) {
                (Ok(x), Ok(y)) => {
                    worst = worst.max((x - y).abs());
                    if (x - y).abs() > STROKE_BP {
                        failures.push(format!("{file}#{index}: {a} vs {b}"));
                    }
                }
                _ => failures.push(format!("{file}#{index}: {a} vs {b}\n  got  {got}\n  want {want}")),
            }
        }
    }
    println!("pdf forms: {compared} diagrams, {identical} byte-identical, {same_tokens}/{tokens} tokens identical, worst number {worst:.5}");
    for f in &failures {
        println!("  {f}");
    }
    assert!(failures.is_empty(), "{} differences", failures.len());
    assert!(compared >= 50);
}

/// The whole-picture wrapper of spec section 13 is balanced, whatever the diagram draws.
#[test]
fn every_stream_is_balanced() {
    let mut d = Diagram::new();
    let mut scratch = Vec::new();
    for c in all_cases().iter().filter(|c| c.status == "OK") {
        let (font, tag) = (c.font.unwrap(), format!("{}#{}", c.file, c.index));
        d.parse(c.preset.as_bytes(), c.source.as_bytes(), font, &StandardLatex).unwrap();
        let (cells, labels) = boxes(c, &d);
        d.layout(&cells, &labels).unwrap();
        let mut sink = Collect::default();
        pdf::emit(&d, Color::BLACK, &mut scratch, &mut sink);
        let s = String::from_utf8(sink.ops).unwrap();
        let mut depth = 0i32;
        for t in s.split_whitespace() {
            match t {
                "q" => depth += 1,
                "Q" => depth -= 1,
                _ => {}
            }
            assert!(depth >= 0, "{tag}: Q without q in {s}");
        }
        assert_eq!(depth, 0, "{tag}: unbalanced: {s}");
        let n_boxes = s.matches("%cell").count() + s.matches("%label").count();
        let want = d.cells().iter().filter(|x| x.has_node).count() + d.labels().len();
        assert_eq!(n_boxes, want, "{tag}: boxes placed");
    }
}

/// The same bytes on every platform (DESIGN section 12).  This test pins them on the machine that made the fixtures; on
/// another platform a different hash means libm or float formatting crept in.  A deliberate change of output changes the
/// hashes here, and only then.
#[test]
fn output_bytes_are_pinned() {
    fn fnv(h: &mut u64, bytes: &[u8]) {
        for &b in bytes {
            *h = (*h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01B3);
        }
    }
    let (mut hp, mut hs) = (0xCBF2_9CE4_8422_2325u64, 0xCBF2_9CE4_8422_2325u64);
    let mut d = Diagram::new();
    let mut scratch = Vec::new();
    for c in all_cases().iter().filter(|c| c.status == "OK") {
        d.parse(c.preset.as_bytes(), c.source.as_bytes(), c.font.unwrap(), &StandardLatex).unwrap();
        let (cells, labels) = boxes(c, &d);
        d.layout(&cells, &labels).unwrap();
        let mut sink = Collect::default();
        pdf::emit(&d, Color::BLACK, &mut scratch, &mut sink);
        fnv(&mut hp, &sink.ops);
        let mut out = Vec::new();
        commute::svg::emit(&d, Color::BLACK, &mut out, &mut commute::svg::NoText);
        fnv(&mut hs, &out);
    }
    println!("pdf {hp:#018x} svg {hs:#018x}");
    assert_eq!((hp, hs), (PDF_HASH, SVG_HASH));
}

const PDF_HASH: u64 = 0x5c7f_5932_ba9f_0b1a;
const SVG_HASH: u64 = 0x123e_f9a3_d657_01e6;
