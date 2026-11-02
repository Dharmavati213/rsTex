//! Reading the tables tests/oracle/gen_commute_tables.py writes.
#![allow(dead_code)]

use commute::{BoxDim, Diagram, FontParams};

/// One diagram of a table.
#[derive(Default, Clone)]
pub struct Case {
    pub file: String,
    pub index: usize,
    pub status: String,
    pub detail: String,
    pub preset: String,
    pub source: String,
    pub font: Option<FontParams>,
    pub cells: Vec<(usize, usize, BoxDim)>,
    pub labels: Vec<(BoxDim, f64)>,
    pub matrix: Option<[f64; 3]>,
    pub bbox: Option<[f64; 4]>,
    pub oracle: Option<[f64; 5]>,
    pub ops: Vec<Op>,
    pub cell_pos: Vec<(usize, usize, f64, f64)>,
    pub label_pos: Vec<(usize, f64, f64, f64, f64)>,
}

#[derive(Clone, Debug)]
pub enum Op {
    Stroke { what: String, width: f64, cap: u8, join: u8, dash: Vec<f64>, color: Option<Vec<f64>>, segs: Vec<(u8, Vec<f64>)> },
    Fill { x: f64, y: f64, w: f64, h: f64, color: Option<Vec<f64>> },
}

fn f(s: &str) -> f64 {
    s.parse().unwrap_or_else(|_| panic!("not a number: {s}"))
}

fn nums(s: &str) -> Vec<f64> {
    s.split_whitespace().map(f).collect()
}

fn unesc(s: &str) -> String {
    s.replace("^^J", "\n").replace("^^I", "\t")
}

pub fn dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata")
}

/// All diagrams of all tables, in file order.
pub fn all_cases() -> Vec<Case> {
    let mut names: Vec<_> = std::fs::read_dir(dir().join("cd")).unwrap().map(|e| e.unwrap().path()).collect();
    names.sort();
    let mut out = Vec::new();
    for p in names {
        let text = std::fs::read_to_string(&p).unwrap();
        for line in text.lines() {
            let r: Vec<&str> = line.split('\t').collect();
            match r[0] {
                "D" => out.push(Case {
                    file: r[1].to_string(),
                    index: r[2].parse().unwrap(),
                    status: r[3].to_string(),
                    detail: r[4].to_string(),
                    ..Case::default()
                }),
                _ => record(out.last_mut().unwrap(), &r),
            }
        }
    }
    out
}

fn record(c: &mut Case, r: &[&str]) {
    match r[0] {
        "P" => c.preset = unesc(r[1]),
        "S" => c.source = unesc(r[1]),
        "F" => c.font = Some(FontParams { ex: f(r[1]), em: f(r[2]), rt: f(r[3]), ax: f(r[4]) }),
        "C" => c.cells.push((r[1].parse().unwrap(), r[2].parse().unwrap(), BoxDim { w: f(r[3]), h: f(r[4]), d: f(r[5]) })),
        "L" => c.labels.push((BoxDim { w: f(r[1]), h: f(r[2]), d: f(r[3]) }, f(r[4]))),
        "M" => c.matrix = Some([f(r[1]), f(r[2]), f(r[3])]),
        "B" => c.bbox = Some([f(r[1]), f(r[2]), f(r[3]), f(r[4])]),
        "O" => c.oracle = Some([f(r[1]), f(r[2]), f(r[3]), f(r[4]), f(r[5])]),
        "X" => c.ops.push(Op::Stroke {
            what: r[1].to_string(),
            width: f(r[2]),
            cap: r[3].parse().unwrap(),
            join: r[4].parse().unwrap(),
            dash: if r[5] == "-" { Vec::new() } else { nums(r[5]) },
            color: if r[6] == "-" { None } else { Some(nums(r[6])) },
            segs: segs(r[7]),
        }),
        "R" => c.ops.push(Op::Fill {
            x: f(r[1]),
            y: f(r[2]),
            w: f(r[3]),
            h: f(r[4]),
            color: if r[5] == "-" { None } else { Some(nums(r[5])) },
        }),
        "T" if r[1] == "cell" => c.cell_pos.push((r[2].parse().unwrap(), r[3].parse().unwrap(), f(r[4]), f(r[5]))),
        "T" => c.label_pos.push((r[2].parse().unwrap(), f(r[3]), f(r[4]), f(r[5]), f(r[6]))),
        other => panic!("unknown record {other}"),
    }
}

fn segs(s: &str) -> Vec<(u8, Vec<f64>)> {
    let mut out: Vec<(u8, Vec<f64>)> = Vec::new();
    for t in s.split_whitespace() {
        match t.as_bytes()[0] {
            b'm' | b'l' | b'c' | b'h' => out.push((t.as_bytes()[0], Vec::new())),
            _ => out.last_mut().unwrap().1.push(f(t)),
        }
    }
    out
}

/// The boxes of a case, parallel to the diagram's cells and labels.
pub fn boxes(c: &Case, d: &Diagram) -> (Vec<BoxDim>, Vec<BoxDim>) {
    let cells = d
        .cells()
        .iter()
        .map(|s| {
            c.cells
                .iter()
                .find(|(r, col, _)| *r == s.row as usize + 1 && *col == s.col as usize + 1)
                .map_or(BoxDim::default(), |x| x.2)
        })
        .collect();
    let mut labels: Vec<BoxDim> = c.labels.iter().map(|l| l.0).collect();
    labels.resize(d.labels().len().max(labels.len()), BoxDim::default());
    (cells, labels)
}
