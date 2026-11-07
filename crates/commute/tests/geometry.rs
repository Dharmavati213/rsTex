//! The gate: for every diagram of tests/cd that the model handles, commute's geometry matches model.py's within the
//! spec's tolerance (docs/TIKZCD-SPEC.md section 1), computed from the same measured boxes.
//!
//!     cargo test -p commute --test geometry -- --nocapture

mod common;

use commute::{Color, Dash, Diagram, Item, Matrix, Pt, Role, Seg, StandardLatex};
use common::{Case, Op, all_cases, boxes, dir};

/// The spec's worst deviations between the model and pdflatex.
const STROKE_BP: f64 = 0.031;
const TEXT_BP: f64 = 0.02;
const BBOX_PT: f64 = 0.0035;
const BP: f64 = 0.99627;

fn comps(c: Color) -> Vec<f64> {
    match c {
        Color::Gray(g) => vec![g],
        Color::Rgb(r, g, b) => vec![r, g, b],
        Color::Cmyk(a, b, c, d) => vec![a, b, c, d],
    }
}

/// The colour in force at the start of a diagram.
fn ambient(c: &Case) -> Vec<f64> {
    if c.file == "15-colors-tikzkeys" && c.index == 6 { vec![0.0, 0.0, 1.0] } else { vec![0.0] }
}

#[derive(Default)]
struct Worst {
    stroke: f64,
    text: f64,
    bbox: f64,
    base: f64,
    strokes: usize,
}

fn flatten(segs: &[Seg], m: &Matrix) -> Vec<(u8, Vec<f64>)> {
    let mut out: Vec<(u8, Vec<f64>)> = segs
        .iter()
        .map(|s| match s.map(m) {
            Seg::Move(p) => (b'm', vec![p.x, p.y]),
            Seg::Line(p) => (b'l', vec![p.x, p.y]),
            Seg::Curve(a, b, c) => (b'c', vec![a.x, a.y, b.x, b.y, c.x, c.y]),
            Seg::Close => (b'h', vec![]),
        })
        .collect();
    // A rebuilt relative curve moves to its start twice, as pgf does; the model has one move.
    if out.len() >= 2 && out[0].0 == b'm' && out[1].0 == b'm' {
        out.remove(0);
    }
    out
}

fn compare_path(tag: &str, got: &[(u8, Vec<f64>)], want: &[(u8, Vec<f64>)], worst: &mut Worst) -> Result<(), String> {
    if got.len() != want.len() {
        return Err(format!("{tag}: {} segments, expected {}", got.len(), want.len()));
    }
    for (g, w) in got.iter().zip(want) {
        if g.0 != w.0 || g.1.len() != w.1.len() {
            return Err(format!("{tag}: segment kind differs"));
        }
        for (a, b) in g.1.iter().zip(&w.1) {
            let dev = (a - b).abs() * BP;
            worst.stroke = worst.stroke.max(dev);
            if dev > STROKE_BP {
                return Err(format!("{tag}: {a} vs {b}, {dev} bp"));
            }
        }
    }
    Ok(())
}

fn close(a: &[f64], b: &[f64], eps: f64) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| (x - y).abs() <= eps)
}

/// Compare a diagram's drawing with the model's.
fn compare(c: &Case, d: &Diagram, worst: &mut Worst) -> Result<(), String> {
    let list = d.draw_list();
    let r = d.result();
    let mut ops = c.ops.iter();
    let mut canvas = Pt::ZERO;
    let mut scope_stroke: Option<Color> = None;
    let mut scope_dash = Dash::default();
    let mut cell_i = 0;
    let mut label_i = 0;
    let amb = ambient(c);

    let want_color = |got: Option<Vec<f64>>, want: &Option<Vec<f64>>, tag: &str| -> Result<(), String> {
        let got = got.unwrap_or_else(|| amb.clone());
        match want {
            Some(w) if !close(&got, w, 1e-4) => Err(format!("{tag}: colour {got:?}, oracle {w:?}")),
            _ => Ok(()),
        }
    };

    for item in &list.items {
        match *item {
            Item::CellBorder { .. } => {} // the model only counts these
            Item::CellBox { cell, x, y, .. } => {
                let s = &d.cells()[cell as usize];
                let (rr, cc) = (s.row as usize + 1, s.col as usize + 1);
                let Some(&(wr, wc, wx, wy)) = c.cell_pos.get(cell_i) else {
                    return Err(format!("cell {rr}-{cc} is not in the model"));
                };
                cell_i += 1;
                if (wr, wc) != (rr, cc) {
                    return Err(format!("cell order: {rr}-{cc}, model {wr}-{wc}"));
                }
                let dev = (x - wx).abs().max((y - wy).abs()) * BP;
                worst.text = worst.text.max(dev);
                if dev > TEXT_BP {
                    return Err(format!("cell {rr}-{cc} at ({x},{y}), model ({wx},{wy})"));
                }
            }
            Item::ArrowBegin { canvas: cv, color, draw_color, dash, .. } => {
                canvas = cv;
                scope_stroke = draw_color.or(color);
                scope_dash = dash;
            }
            Item::Stroke { role, path, width, color, canvas: cv } => {
                let what = match role {
                    Role::Halo => "halo",
                    Role::Line => "line",
                    Role::Inner => "inner",
                };
                let Some(Op::Stroke { what: w, width: ww, cap, join, dash, color: wcol, segs }) = ops.next() else {
                    return Err(format!("extra {what}"));
                };
                if w != what {
                    return Err(format!("{what}, model {w}"));
                }
                if (width - ww).abs() * BP > 1e-3 {
                    return Err(format!("{what} width {width}, model {ww}"));
                }
                if (*cap, *join) != (0, 0) {
                    return Err(format!("{what}: model cap {cap} join {join}"));
                }
                if role != Role::Halo && !close(scope_dash.values(), dash, 2e-3) {
                    return Err(format!("{what} dash {:?}, model {dash:?}", scope_dash.values()));
                }
                let got = flatten(list.path(path), &Matrix::translate(cv.x, cv.y));
                compare_path(what, &got, segs, worst)?;
                want_color(color.or(scope_stroke).map(comps), wcol, what)?;
                worst.strokes += 1;
            }
            Item::Tip { m, path, width, cap, join, .. } => {
                let Some(Op::Stroke { what: w, width: ww, cap: wc, join: wj, color: wcol, segs, .. }) = ops.next() else {
                    return Err("extra tip".to_string());
                };
                if w != "tip" {
                    return Err(format!("tip, model {w}"));
                }
                if (width - ww).abs() * BP > 1e-3 || (cap, join) != (*wc, *wj) {
                    return Err(format!("tip width {width} cap {cap} join {join}, model {ww} {wc} {wj}"));
                }
                let mut world = m;
                world.e += canvas.x;
                world.f += canvas.y;
                compare_path("tip", &flatten(list.path(path), &world), segs, worst)?;
                want_color(scope_stroke.map(comps), wcol, "tip")?;
                worst.strokes += 1;
            }
            Item::LabelFill { m, x, y, w, h, color, .. } => {
                let Some(Op::Fill { x: wx, y: wy, w: ww, h: wh, color: wcol }) = ops.next() else {
                    return Err("extra fill".to_string());
                };
                let p = m.apply(Pt::new(x, y));
                let dev = [p.x + canvas.x - wx, p.y + canvas.y - wy, w - ww, h - wh].iter().fold(0.0f64, |a, b| a.max(b.abs())) * BP;
                worst.stroke = worst.stroke.max(dev);
                if dev > STROKE_BP {
                    return Err(format!("fill off by {dev} bp"));
                }
                want_color(Some(comps(color)), wcol, "fill")?;
                worst.strokes += 1;
            }
            Item::LabelBox { label, m, .. } => {
                let Some(&(wi, wx, wy, wc, ws)) = c.label_pos.get(label_i) else {
                    return Err(format!("label {label} is not in the model"));
                };
                label_i += 1;
                if wi != label as usize {
                    return Err(format!("label order {label}, model {wi}"));
                }
                let dev = (m.e + canvas.x - wx).abs().max((m.f + canvas.y - wy).abs()) * BP;
                worst.text = worst.text.max(dev);
                if dev > TEXT_BP {
                    return Err(format!("label {label} at ({},{}), model ({wx},{wy})", m.e + canvas.x, m.f + canvas.y));
                }
                if (m.a - wc).abs() > 1e-4 || (m.b - ws).abs() > 1e-4 {
                    return Err(format!("label {label} rotation ({},{}), model ({wc},{ws})", m.a, m.b));
                }
            }
            Item::CellEmpty { .. } | Item::Coordinates { .. } => {}
            Item::ArrowEnd => {
                canvas = Pt::ZERO;
                scope_stroke = None;
                scope_dash = Dash::default();
            }
        }
    }
    if ops.next().is_some() {
        return Err("the model draws more".to_string());
    }
    if cell_i != c.cell_pos.len() || label_i != c.label_pos.len() {
        return Err(format!("{cell_i} cells and {label_i} labels, model {} and {}", c.cell_pos.len(), c.label_pos.len()));
    }

    let m = c.matrix.unwrap();
    if !close(&[r.matrix_w, r.matrix_h, r.matrix_d], &m, 2e-3) {
        return Err(format!("matrix {} {} {}, model {m:?}", r.matrix_w, r.matrix_h, r.matrix_d));
    }
    let b = c.bbox.unwrap();
    let dev = [r.minx - b[0], r.miny - b[1], r.maxx - b[2], r.maxy - b[3]].iter().fold(0.0f64, |a, x| a.max(x.abs()));
    worst.bbox = worst.bbox.max(dev);
    if dev > BBOX_PT {
        return Err(format!("picture box [{} {} {} {}], model {b:?}", r.minx, r.miny, r.maxx, r.maxy));
    }
    let o = c.oracle.unwrap();
    let dev = (r.base - o[4]).abs();
    worst.base = worst.base.max(dev);
    if dev > 0.01 {
        return Err(format!("baseline {}, oracle {}", r.base, o[4]));
    }
    Ok(())
}

/// Parse and lay out a case.  Err is the diagram's Unsupported, as `reason\ttext`.
fn run(c: &Case, d: &mut Diagram) -> Result<(), String> {
    let font = c.font.unwrap_or(commute::FontParams::ARTICLE_10PT);
    if let Err(e) = d.parse(c.preset.as_bytes(), c.source.as_bytes(), font, &StandardLatex) {
        return Err(format!("{}\t{}", e.reason, String::from_utf8_lossy(d.text(e.at))));
    }
    let (cells, labels) = boxes(c, d);
    if let Err(e) = d.layout(&cells, &labels) {
        return Err(format!("{}\t{}", e.reason, String::from_utf8_lossy(d.text(e.at))));
    }
    Ok(())
}

#[test]
fn every_diagram_the_model_handles_matches_it() {
    let cases = all_cases();
    let mut d = Diagram::new();
    let mut worst = Worst::default();
    let (mut den, mut matched) = (0, 0);
    let mut failures = Vec::new();
    for c in cases.iter().filter(|c| c.status == "OK") {
        den += 1;
        let tag = format!("{}#{}", c.file, c.index);
        match run(c, &mut d) {
            Err(e) => failures.push(format!("{tag}: Unsupported: {e}")),
            Ok(()) => match compare(c, &d, &mut worst) {
                Ok(()) => matched += 1,
                Err(e) => failures.push(format!("{tag}: {e}")),
            },
        }
    }
    println!("geometry: {matched}/{den} diagrams match the model, {} stroke operators", worst.strokes);
    println!(
        "worst deviation from the model: stroke {:.6} bp, text {:.6} bp, picture box {:.6} pt, baseline from the oracle {:.6} pt",
        worst.stroke, worst.text, worst.bbox, worst.base
    );
    for f in &failures {
        println!("  {f}");
    }
    assert!(failures.is_empty(), "{} diagrams differ", failures.len());
    assert!(den >= 196, "fixtures lost diagrams: {den}");
}

/// Which diagrams are classified Unsupported, and why, must be exactly the table in testdata/unsupported.tsv.
#[test]
fn unsupported_diagrams_are_exactly_the_listed_ones() {
    let table = std::fs::read_to_string(dir().join("unsupported.tsv")).unwrap();
    let mut expected: Vec<String> = table.lines().map(|l| l.trim_end().to_string()).collect();
    expected.sort();
    let mut got = Vec::new();
    let mut d = Diagram::new();
    let cases = all_cases();
    let mut handled_but_model_not = Vec::new();
    for c in &cases {
        match run(c, &mut d) {
            Err(e) => got.push(format!("{}\t{}\t{}", c.file, c.index, e).trim_end().to_string()),
            Ok(()) if c.status != "OK" => handled_but_model_not.push(format!("{}#{} ({}: {})", c.file, c.index, c.status, c.detail)),
            Ok(()) => {}
        }
    }
    got.sort();
    println!("classified Unsupported: {}/{} diagrams", got.len(), cases.len());
    for g in &got {
        println!("  {g}");
    }
    println!("handled here but not by the model: {handled_but_model_not:?}");
    assert_eq!(got, expected);
    // Test 23's bare & in align: an error in the oracle only because of &'s catcode, which spec 3.1 allows a native
    // version to accept.  Nothing else may be handled without a model to compare with.
    // `>=stealth` and `>=latex` the model does not have; tests/corpus/cd/compare_library.py compares them with the oracle.
    assert_eq!(handled_but_model_not, ["20-arrow-styles#3 (SKIP: diagrams key)", "23-unknown-keys#8 (DIFF: matrix dimensions differ)"]);
}

/// Cells get a node exactly when the oracle made one (spec 3.2), and labels are counted the same way.
#[test]
fn nodes_and_labels_agree_with_the_log() {
    let mut d = Diagram::new();
    for c in all_cases().iter().filter(|c| c.status == "OK") {
        let font = c.font.unwrap();
        d.parse(c.preset.as_bytes(), c.source.as_bytes(), font, &StandardLatex).unwrap();
        let tag = format!("{}#{}", c.file, c.index);
        for s in d.cells() {
            let logged = c.cells.iter().any(|(r, col, _)| *r == s.row as usize + 1 && *col == s.col as usize + 1);
            assert_eq!(s.has_node, logged, "{tag}: cell {}-{}", s.row + 1, s.col + 1);
        }
        assert_eq!(d.labels().len(), c.labels.len(), "{tag}: label count");
    }
}

/// The boxes in spec section 12's and test 17's check, computed from the oracle's numbers.
#[test]
fn picture_box_of_the_setbox_diagrams() {
    let mut d = Diagram::new();
    let cases = all_cases();
    let check = |d: &mut Diagram, index: usize, w: f64, h: f64, dp: f64| {
        let c = cases.iter().find(|c| c.file == "17-baseline" && c.index == index).unwrap();
        run(c, d).unwrap();
        let r = d.result();
        assert!((r.width - w).abs() < 2e-3, "width {} {w}", r.width);
        assert!((r.height - h).abs() < 2e-3, "height {} {h}", r.height);
        assert!((r.depth - dp).abs() < 2e-3, "depth {} {dp}", r.depth);
    };
    check(&mut d, 14, 57.17142, 25.88748, 23.15279);
    check(&mut d, 15, 56.80893, 13.22772, 3.65973);
}

/// Spec section 14's worked example.
#[test]
fn worked_example() {
    let c = all_cases().into_iter().find(|c| c.file == "01-square").unwrap();
    let mut d = Diagram::new();
    run(&c, &mut d).unwrap();
    let r = *d.result();
    assert!((r.matrix_w - 57.64163).abs() < 1e-3);
    assert!((r.matrix_h - 42.64579).abs() < 1e-3);
    assert!((r.matrix_d - 3.65971).abs() < 1e-3);
    assert!((r.width - 58.66599).abs() < 2e-3);
    assert!((r.height - 25.88748).abs() < 2e-3);
    assert!((r.depth - 26.35966).abs() < 2e-3);
}

/// `\pgfpointshapeborder` warns where it looks for the border towards the node's own centre, and only real PGF writes that: an arrow from a node to itself whose end
/// without an anchor looks towards the other end's `center` warns as one without anchors does.  Cells of `(` of cmr10, whose middle is on the axis, and of `A`, whose is not.
#[test]
fn an_end_towards_its_own_centre_is_what_pgf_warns_of() {
    let paren = commute::BoxDim { w: 3.8889, h: 7.5, d: 2.5 };
    let letter = commute::BoxDim { w: 7.50002, h: 6.83331, d: 0.0 };
    let mut d = Diagram::new();
    for (src, cell, warns) in [
        (r"A \arrow[to=1-1]", letter, true),
        (r"A \arrow[to=1-1, start anchor=center]", letter, true),
        (r"A \arrow[to=1-1, end anchor=center, bend left]", letter, true),
        (r"A \arrow[loop, start anchor=center, bend left]", letter, true),
        (r"|[alias=Z]| A \arrow[from=Z, to=Z, start anchor=center, shift left]", letter, true),
        (r"( \arrow[to=1-1, start anchor=real center]", paren, true),
        (r"A \arrow[to=1-1, start anchor=real center]", letter, false),
        (r"A \arrow[to=1-1, start anchor=north]", letter, false),
        (r"A \arrow[to=1-1, start anchor=center, end anchor=center]", letter, false),
        (r"A \arrow[loop right, start anchor=center]", letter, false),
        (r"A \arrow[to=1-1, out=60, in=30, start anchor=center]", letter, false),
        (r"A \arrow[r, end anchor=center] & B", letter, false),
    ] {
        d.parse(b"", src.as_bytes(), commute::FontParams::ARTICLE_10PT, &StandardLatex).unwrap();
        let cells = vec![cell; d.cells().len()];
        let labels = vec![commute::BoxDim::default(); d.labels().len()];
        d.layout(&cells, &labels).unwrap();
        assert_eq!(d.has_arrow_to_itself(), warns, "{src}");
    }
}

/// A separation more negative than the cells are wide or high: the sums go below zero, and the box of `\hbox{\vbox{\halign...}}` does not (spec section 7).
#[test]
fn the_matrix_box_is_never_negative() {
    let letter = commute::BoxDim { w: 7.50002, h: 6.83331, d: 0.0 };
    let mut d = Diagram::new();
    for (src, w, h) in [
        (r"[column sep=-3cm] A & B", Some(0.0), None),
        (r"[row sep=-3cm] A \\ B", None, Some(0.0)),
        (r"[sep=-3cm] A & B \\ C & D", Some(0.0), Some(0.0)),
    ] {
        d.parse(b"", src.as_bytes(), commute::FontParams::ARTICLE_10PT, &StandardLatex).unwrap();
        d.layout(&vec![letter; d.cells().len()], &[]).unwrap();
        let r = *d.result();
        assert!(r.matrix_w >= 0.0 && r.matrix_h >= 0.0 && r.matrix_d >= 0.0, "{src}: {r:?}");
        if let Some(w) = w {
            assert_eq!(r.matrix_w, w, "{src}");
        }
        if let Some(h) = h {
            assert_eq!(r.matrix_h, h, "{src}");
        }
    }
}
