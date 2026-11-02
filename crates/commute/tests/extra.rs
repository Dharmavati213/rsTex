//! Keys beyond the model, compared with what pdflatex put in the PDF.  The table is made at run time and is not committed:
//!
//!     python3 -I tests/oracle/check_commute_extra.py
//!     COMMUTE_EXTRA=target/commute-extra/extra.tsv cargo test -p commute --test extra -- --nocapture
//!
//! Without COMMUTE_EXTRA this test does nothing and says so.

use commute::{BoxDim, Color, Diagram, FontParams, Item, Matrix, Pt, Seg, StandardLatex};

const BP: f64 = 0.99627;
const STROKE_BP: f64 = 0.05;
const BBOX_PT: f64 = 0.05;

#[derive(Default)]
struct Case {
    index: usize,
    preset: String,
    source: String,
    font: Option<FontParams>,
    cells: Vec<(usize, usize, BoxDim)>,
    labels: Vec<BoxDim>,
    oracle: [f64; 5],
    ops: Vec<PdfOp>,
    text: Vec<(f64, f64)>,
}

struct PdfOp {
    op: String,
    width: f64,
    cap: u8,
    join: u8,
    dash: Vec<f64>,
    rg_stroke: Vec<f64>,
    rg_fill: Vec<f64>,
    segs: Vec<(String, Vec<f64>)>,
}

fn num(s: &str) -> f64 {
    s.parse().unwrap()
}

fn nums(s: &str) -> Vec<f64> {
    if s == "-" { Vec::new() } else { s.split_whitespace().map(num).collect() }
}

fn read(path: &str) -> Vec<Case> {
    let text = std::fs::read_to_string(path).unwrap();
    let mut out: Vec<Case> = Vec::new();
    let unesc = |s: &str| s.replace("^^J", "\n").replace("^^I", "\t");
    for line in text.lines() {
        let r: Vec<&str> = line.split('\t').collect();
        match r[0] {
            "D" => out.push(Case { index: r[2].parse().unwrap(), ..Case::default() }),
            "P" => out.last_mut().unwrap().preset = unesc(r[1]),
            "S" => out.last_mut().unwrap().source = unesc(r[1]),
            "F" => out.last_mut().unwrap().font = Some(FontParams { ex: num(r[1]), em: num(r[2]), rt: num(r[3]), ax: num(r[4]) }),
            "C" => out.last_mut().unwrap().cells.push((r[1].parse().unwrap(), r[2].parse().unwrap(), BoxDim { w: num(r[3]), h: num(r[4]), d: num(r[5]) })),
            "L" => out.last_mut().unwrap().labels.push(BoxDim { w: num(r[1]), h: num(r[2]), d: num(r[3]) }),
            "O" => out.last_mut().unwrap().oracle = [num(r[1]), num(r[2]), num(r[3]), num(r[4]), num(r[5])],
            "G" => out.last_mut().unwrap().text.push((num(r[1]), num(r[2]))),
            "Y" => {
                let mut segs: Vec<(String, Vec<f64>)> = Vec::new();
                for t in r[8].split_whitespace() {
                    match t {
                        "m" | "l" | "c" | "h" | "re" => segs.push((t.to_string(), Vec::new())),
                        _ => segs.last_mut().unwrap().1.push(num(t)),
                    }
                }
                // pgf doubles the first moveto of a rebuilt curve.
                if segs.len() >= 2 && segs[0].0 == "m" && segs[1].0 == "m" {
                    segs.remove(0);
                }
                out.last_mut().unwrap().ops.push(PdfOp {
                    op: r[1].to_string(),
                    width: num(r[2]),
                    cap: r[3].parse().unwrap(),
                    join: r[4].parse().unwrap(),
                    dash: nums(r[5]),
                    rg_stroke: nums(r[6]),
                    rg_fill: nums(r[7]),
                    segs,
                });
            }
            other => panic!("record {other}"),
        }
    }
    out
}

fn comps(c: Color) -> Vec<f64> {
    match c {
        Color::Gray(g) => vec![g],
        Color::Rgb(r, g, b) => vec![r, g, b],
        Color::Cmyk(a, b, c, d) => vec![a, b, c, d],
    }
}

/// One thing commute draws, as the PDF would hold it, in bp.
struct Drawn {
    op: &'static str,
    width: f64,
    cap: u8,
    join: u8,
    dash: Vec<f64>,
    color: Vec<f64>,
    segs: Vec<(String, Vec<f64>)>,
}

fn segs_bp(segs: &[Seg], m: &Matrix) -> Vec<(String, Vec<f64>)> {
    segs.iter()
        .map(|s| match s.map(m) {
            Seg::Move(p) => ("m".to_string(), vec![p.x * BP, p.y * BP]),
            Seg::Line(p) => ("l".to_string(), vec![p.x * BP, p.y * BP]),
            Seg::Curve(a, b, c) => ("c".to_string(), vec![a.x * BP, a.y * BP, b.x * BP, b.y * BP, c.x * BP, c.y * BP]),
            Seg::Close => ("h".to_string(), vec![]),
        })
        .collect()
}

fn drawn(d: &Diagram) -> Vec<Drawn> {
    let list = d.draw_list();
    let mut out = Vec::new();
    let (mut canvas, mut stroke, mut dash) = (Pt::ZERO, None, Vec::new());
    for item in &list.items {
        match *item {
            Item::CellBorder { x, y, w, h, rounded, color } => {
                let segs = match rounded {
                    Some(p) => segs_bp(list.path(p), &Matrix::IDENTITY),
                    None => vec![("re".to_string(), vec![x * BP, y * BP, w * BP, h * BP])],
                };
                out.push(Drawn { op: "S", width: d.cell_line_width(), cap: 0, join: 0, dash: vec![], color: color.map_or(vec![0.0], comps), segs });
            }
            Item::ArrowBegin { canvas: c, color, draw_color, dash: ds, .. } => {
                canvas = c;
                stroke = draw_color.or(color);
                dash = ds.values().iter().map(|v| v * BP).collect();
            }
            Item::Stroke { role, path, width, color, canvas: c } => {
                let dashes = if role == commute::Role::Halo { vec![] } else { dash.clone() };
                out.push(Drawn {
                    op: "S",
                    width,
                    cap: 0,
                    join: 0,
                    dash: dashes,
                    color: color.or(stroke).map_or(vec![0.0], comps),
                    segs: segs_bp(list.path(path), &Matrix::translate(c.x, c.y)),
                });
            }
            Item::Tip { m, path, width, cap, join, .. } => {
                let mut w = m;
                w.e += canvas.x;
                w.f += canvas.y;
                out.push(Drawn { op: "S", width, cap, join, dash: vec![], color: stroke.map_or(vec![0.0], comps), segs: segs_bp(list.path(path), &w) });
            }
            Item::LabelFill { m, x, y, w, h, color } => {
                let segs = if m.is_translation() {
                    let p = m.apply(Pt::new(x, y));
                    vec![("re".to_string(), vec![(p.x + canvas.x) * BP, (p.y + canvas.y) * BP, w * BP, h * BP])]
                } else {
                    let mut t = m;
                    t.e += canvas.x;
                    t.f += canvas.y;
                    segs_bp(
                        &[
                            Seg::Move(Pt::new(x + w, y + h)),
                            Seg::Line(Pt::new(x, y + h)),
                            Seg::Line(Pt::new(x, y)),
                            Seg::Line(Pt::new(x + w, y)),
                            Seg::Close,
                            Seg::Move(Pt::new(x, y)),
                        ],
                        &t,
                    )
                };
                out.push(Drawn { op: "f", width: 0.0, cap: 0, join: 0, dash: vec![], color: comps(color), segs });
            }
            Item::ArrowEnd => {
                canvas = Pt::ZERO;
                stroke = None;
                dash.clear();
            }
            _ => {}
        }
    }
    out
}

fn compare(c: &Case, d: &Diagram, cells: &[BoxDim], worst: &mut f64, worst_text: &mut f64, text_counts: &mut (u32, u32)) -> Result<(), String> {
    let got = drawn(d);
    let want = &c.ops;
    if got.len() != want.len() {
        return Err(format!("{} operators, PDF {}", got.len(), want.len()));
    }
    for (i, (g, w)) in got.iter().zip(want).enumerate() {
        let tag = format!("op {i}");
        if g.op != w.op {
            return Err(format!("{tag}: {} vs {}", g.op, w.op));
        }
        if g.op == "S" {
            if (g.width * BP - w.width).abs() > 2e-3 {
                return Err(format!("{tag}: width {} vs {}", g.width * BP, w.width));
            }
            if (g.cap, g.join) != (w.cap, w.join) {
                return Err(format!("{tag}: cap/join {} {} vs {} {}", g.cap, g.join, w.cap, w.join));
            }
            if g.dash.len() != w.dash.len() || g.dash.iter().zip(&w.dash).any(|(a, b)| (a - b).abs() > 3e-3) {
                return Err(format!("{tag}: dash {:?} vs {:?}", g.dash, w.dash));
            }
            if g.color.len() != w.rg_stroke.len() || g.color.iter().zip(&w.rg_stroke).any(|(a, b)| (a - b).abs() > 1e-3) {
                return Err(format!("{tag}: colour {:?} vs {:?}", g.color, w.rg_stroke));
            }
        } else if g.color.len() != w.rg_fill.len() || g.color.iter().zip(&w.rg_fill).any(|(a, b)| (a - b).abs() > 1e-3) {
            return Err(format!("{tag}: fill {:?} vs {:?}", g.color, w.rg_fill));
        }
        if g.segs.len() != w.segs.len() {
            return Err(format!("{tag}: {:?} vs {:?}", g.segs, w.segs));
        }
        for (a, b) in g.segs.iter().zip(&w.segs) {
            if a.0 != b.0 || a.1.len() != b.1.len() {
                return Err(format!("{tag}: {:?} vs {:?}", g.segs, w.segs));
            }
            for (x, y) in a.1.iter().zip(&b.1) {
                let dev = (x - y).abs();
                *worst = worst.max(dev);
                if dev > STROKE_BP {
                    return Err(format!("{tag}: {x} vs {y}\n     got {:?}\n    want {:?}", g.segs, w.segs));
                }
            }
        }
    }
    // Where the boxes go.  A box whose first glyph is not at its origin has no text run there, as in model.py's check.
    const TRUE_BP: f64 = 7200.0 / 7227.0;
    let (mut canvas, mut near, mut far) = (Pt::ZERO, 0, 0);
    for item in &d.draw_list().items {
        let at = match *item {
            Item::ArrowBegin { canvas: cv, .. } => {
                canvas = cv;
                continue;
            }
            Item::CellBox { cell, x, y, .. } if cells[cell as usize].w > 0.0 => (x, y),
            Item::LabelBox { label, m, .. } if c.labels[label as usize].w > 0.0 => (m.e + canvas.x, m.f + canvas.y),
            _ => continue,
        };
        let best = c.text.iter().map(|t| (at.0 * TRUE_BP - t.0).abs().max((at.1 * TRUE_BP - t.1).abs())).fold(f64::INFINITY, f64::min);
        *worst_text = worst_text.max(if best <= 0.03 { best } else { 0.0 });
        if best <= 0.03 {
            near += 1;
        } else {
            far += 1;
        }
    }
    *text_counts = (text_counts.0 + near, text_counts.1 + far);
    let r = d.result();
    let o = c.oracle;
    let bb = [r.minx - o[0], r.miny - o[1], r.maxx - o[2], r.maxy - o[3]];
    if bb.iter().any(|x| x.abs() > BBOX_PT) {
        return Err(format!("box [{} {} {} {}] vs {:?}", r.minx, r.miny, r.maxx, r.maxy, &o[..4]));
    }
    if (r.base - o[4]).abs() > BBOX_PT {
        return Err(format!("baseline {} vs {}", r.base, o[4]));
    }
    Ok(())
}

#[test]
fn keys_beyond_the_model_match_pdflatex() {
    let Ok(path) = std::env::var("COMMUTE_EXTRA") else {
        println!("COMMUTE_EXTRA is not set; nothing compared");
        return;
    };
    let cases = read(&path);
    let mut d = Diagram::new();
    let (mut worst, mut worst_text, mut text_counts) = (0.0f64, 0.0f64, (0, 0));
    let (mut ok, mut failures, mut unsupported) = (0, Vec::new(), Vec::new());
    for c in &cases {
        if let Err(e) = d.parse(c.preset.as_bytes(), c.source.as_bytes(), c.font.unwrap(), &StandardLatex) {
            unsupported.push(format!("#{}: {} {}", c.index, e.reason, String::from_utf8_lossy(d.text(e.at))));
            continue;
        }
        let cells: Vec<BoxDim> = d
            .cells()
            .iter()
            .map(|s| c.cells.iter().find(|(r, col, _)| *r == s.row as usize + 1 && *col == s.col as usize + 1).map_or(BoxDim::default(), |x| x.2))
            .collect();
        if let Err(e) = d.layout(&cells, &c.labels) {
            let (reason, at) = (e.reason, e.at);
            unsupported.push(format!("#{}: {reason} {}", c.index, String::from_utf8_lossy(d.text(at))));
            continue;
        }
        match compare(c, &d, &cells, &mut worst, &mut worst_text, &mut text_counts) {
            Ok(()) => ok += 1,
            Err(e) => failures.push(format!("#{}: {e}", c.index)),
        }
    }
    println!("extra: {ok}/{} match pdflatex, worst {worst:.5} bp", cases.len());
    println!("boxes: {} at a text run, worst {worst_text:.5} bp; {} with no text run at their origin", text_counts.0, text_counts.1);
    for u in &unsupported {
        println!("  unsupported {u}");
    }
    for f in &failures {
        println!("  {f}");
    }
    assert!(failures.is_empty());
}
