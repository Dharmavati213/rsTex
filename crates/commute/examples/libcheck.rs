//! Feeds measured boxes to the library for `tests/corpus/cd/compare_library.py`, and writes what it computes.
//!
//! Standard input is a list of jobs, standard output one record per job; both are read by that tool only.  A job:
//!
//!     JOB <name>
//!     FONT <ex> <em> <rt> <ax>         scaled points
//!     AMBIENT <n> <components...>      colour at the start of the picture, in thousandths of a unit
//!     CELL <row> <col> <w> <h> <d>     1-based, scaled points; any number of these
//!     LABEL <w> <h> <d>                any number of these, in drawing order
//!     PRESET <bytes>                   then that many bytes and a newline
//!     PARSE <standard|all>             only parse, under that policy; for a whole run, before the first job
//!     ENV <bytes>                      then that many bytes and a newline; the job runs at this line
//!
//! A record is `RES <name> ok`, `RES <name> declined <reason><tab><the text concerned>` or `RES <name> boxes <what is wrong>`;
//! with `ok`, `BOX` and `OPS <bytes>` follow.

use std::io::{BufRead, Write};

use commute::pdf::{self, Collect};
use commute::{AllowAll, BoxDim, Color, Diagram, FontParams, Policy, StandardLatex};

fn sp(s: &str) -> f64 {
    s.parse::<i64>().unwrap_or(0) as f64 / 65536.0
}

fn bytes<R: BufRead>(r: &mut R, n: usize) -> Vec<u8> {
    let mut v = vec![0; n + 1];
    r.read_exact(&mut v).unwrap();
    v.pop();
    v
}

fn main() {
    let stdin = std::io::stdin();
    let mut input = stdin.lock();
    let stdout = std::io::stdout();
    let mut out = std::io::BufWriter::new(stdout.lock());
    let mut d = Diagram::new();
    let mut scratch = Vec::new();
    let (mut name, mut font) = (String::new(), FontParams::ARTICLE_10PT);
    let mut ambient = Color::BLACK;
    let mut cells: Vec<(u32, u32, BoxDim)> = Vec::new();
    let mut labels: Vec<BoxDim> = Vec::new();
    let mut preset = Vec::new();
    let mut only_parse: Option<&dyn Policy> = None;
    let mut line = String::new();
    loop {
        line.clear();
        if input.read_line(&mut line).unwrap() == 0 {
            break;
        }
        let f: Vec<&str> = line.split_whitespace().collect();
        match f.first().copied() {
            Some("JOB") => {
                name = f[1].to_string();
                cells.clear();
                labels.clear();
                preset.clear();
                ambient = Color::BLACK;
            }
            Some("FONT") => font = FontParams { ex: sp(f[1]), em: sp(f[2]), rt: sp(f[3]), ax: sp(f[4]) },
            Some("AMBIENT") => {
                let c: Vec<f64> = f[2..].iter().map(|x| x.parse::<f64>().unwrap() / 1000.0).collect();
                ambient = match c.len() {
                    1 => Color::Gray(c[0]),
                    3 => Color::Rgb(c[0], c[1], c[2]),
                    _ => Color::Cmyk(c[0], c[1], c[2], c[3]),
                };
            }
            Some("CELL") => cells.push((
                f[1].parse().unwrap(),
                f[2].parse().unwrap(),
                BoxDim { w: sp(f[3]), h: sp(f[4]), d: sp(f[5]) },
            )),
            Some("LABEL") => labels.push(BoxDim { w: sp(f[1]), h: sp(f[2]), d: sp(f[3]) }),
            Some("PRESET") => preset = bytes(&mut input, f[1].parse().unwrap()),
            Some("PARSE") => only_parse = Some(if f[1] == "standard" { &StandardLatex } else { &AllowAll }),
            Some("ENV") => {
                let env = bytes(&mut input, f[1].parse().unwrap());
                if let Some(policy) = only_parse {
                    match d.parse(&preset, &env, font, policy) {
                        Ok(()) => writeln!(out, "RES {name} ok").unwrap(),
                        Err(u) => writeln!(out, "RES {name} declined {}", u.reason).unwrap(),
                    }
                    continue;
                }
                run(&mut d, &mut out, &name, font, ambient, &preset, &env, &cells, &labels, &mut scratch);
            }
            _ => {}
        }
    }
    out.flush().unwrap();
}

#[allow(clippy::too_many_arguments)]
fn run<W: Write>(
    d: &mut Diagram,
    out: &mut W,
    name: &str,
    font: FontParams,
    ambient: Color,
    preset: &[u8],
    env: &[u8],
    cells: &[(u32, u32, BoxDim)],
    labels: &[BoxDim],
    scratch: &mut Vec<u8>,
) {
    if let Err(u) = d.parse(preset, env, font, &AllowAll) {
        write!(out, "RES {name} declined {}\t", u.reason).unwrap();
        let at: Vec<u8> = d.text(u.at).iter().copied().filter(|&c| c != b'\n').take(60).collect();
        out.write_all(&at).unwrap();
        writeln!(out).unwrap();
        return;
    }
    let mut cdims = Vec::with_capacity(d.cells().len());
    let mut bad = None;
    let mut used = 0;
    for c in d.cells() {
        let found = cells.iter().find(|x| x.0 == c.row + 1 && x.1 == c.col + 1);
        if c.has_node && found.is_none() {
            bad = Some(format!("no box for the node in {}-{}", c.row + 1, c.col + 1));
        }
        if !c.has_node && found.is_some() {
            bad = Some(format!("a box for a cell without a node, {}-{}", c.row + 1, c.col + 1));
        }
        used += usize::from(found.is_some());
        cdims.push(found.map_or(BoxDim::default(), |x| x.2));
    }
    if used != cells.len() {
        bad = bad.or(Some(format!("{} boxes for {} nodes", cells.len(), used)));
    }
    if labels.len() != d.labels().len() {
        bad = bad.or(Some(format!("{} label boxes for {} labels", labels.len(), d.labels().len())));
    }
    if let Some(b) = bad {
        writeln!(out, "RES {name} boxes {b}").unwrap();
        return;
    }
    if let Err(u) = d.layout(&cdims, labels) {
        writeln!(out, "RES {name} declined {}", u.reason).unwrap();
        return;
    }
    let r = *d.result();
    let mut c = Collect::default();
    pdf::emit(d, ambient, scratch, &mut c);
    let s = |x: f64| (x * 65536.0).round() as i64;
    writeln!(out, "RES {name} ok").unwrap();
    writeln!(out, "BOX {} {} {} {} {} {} {} {}", s(r.minx), s(r.miny), s(r.maxx), s(r.maxy), s(r.base), s(r.width), s(r.height), s(r.depth)).unwrap();
    writeln!(out, "OPS {}", c.ops.len()).unwrap();
    out.write_all(&c.ops).unwrap();
    writeln!(out).unwrap();
}
