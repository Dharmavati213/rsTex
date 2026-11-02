//! The SVG backend: one path per stroke in drawing order, and everything inside the cropped view box.

mod common;

use commute::svg::{self, SvgText, hex};
use commute::{BoxRef, Color, Diagram, Item, Matrix, StandardLatex};
use common::{all_cases, boxes};

#[derive(Default)]
struct Marks(Vec<(BoxRef, Matrix)>);

impl SvgText for Marks {
    fn place_box(&mut self, out: &mut Vec<u8>, which: BoxRef, t: &Matrix, fill: &str) {
        self.0.push((which, *t));
        assert!(fill.len() == 7 && fill.starts_with('#'));
        out.extend_from_slice(b"<!--box-->\n");
    }
}

#[test]
fn paths_match_the_draw_list_and_stay_in_the_view_box() {
    let mut d = Diagram::new();
    let mut total = 0;
    for c in all_cases().iter().filter(|c| c.status == "OK") {
        let tag = format!("{}#{}", c.file, c.index);
        d.parse(c.preset.as_bytes(), c.source.as_bytes(), c.font.unwrap(), &StandardLatex).unwrap();
        let (cells, labels) = boxes(c, &d);
        d.layout(&cells, &labels).unwrap();
        let mut out = Vec::new();
        let mut marks = Marks::default();
        svg::emit(&d, Color::BLACK, &mut out, &mut marks);
        let s = String::from_utf8(out).unwrap();
        assert!(s.starts_with("<svg xmlns=\"http://www.w3.org/2000/svg\""), "{tag}");
        assert!(s.ends_with("</svg>\n"), "{tag}");

        let list = d.draw_list();
        let drawn = list
            .items
            .iter()
            .filter(|i| matches!(i, Item::CellBorder { .. } | Item::Stroke { .. } | Item::Tip { .. } | Item::LabelFill { .. }))
            .count();
        assert_eq!(s.matches("<path").count(), drawn, "{tag}");
        assert_eq!(marks.0.len(), d.cells().iter().filter(|x| x.has_node).count() + d.labels().len(), "{tag}");
        total += drawn;

        // transform canvas is outside the box on purpose (spec 12); so are the stroke's own half widths, by a little.
        let canvas = list.items.iter().any(|i| matches!(i, Item::ArrowBegin { canvas, .. } if *canvas != commute::Pt::ZERO));
        if canvas {
            continue;
        }
        let r = d.result();
        let (w, h) = (r.maxx - r.minx, r.maxy - r.miny);
        for path in s.split("<path d=\"").skip(1) {
            let d_attr = &path[..path.find('"').unwrap()];
            let nums: Vec<f64> = d_attr.split_whitespace().filter_map(|t| t.parse().ok()).collect();
            for xy in nums.chunks(2) {
                assert!(xy[0] > -0.01 && xy[0] < w + 0.01, "{tag}: x {} outside {w}", xy[0]);
                assert!(xy[1] > -0.01 && xy[1] < h + 0.01, "{tag}: y {} outside {h}", xy[1]);
            }
        }
    }
    assert!(total > 900);
}

#[test]
fn text_transform_runs_y_down() {
    let mut d = Diagram::new();
    d.parse(b"", br#"A \arrow[rd,"f"{sloped}] & \\ & B"#, commute::FontParams::ARTICLE_10PT, &StandardLatex).unwrap();
    let cells = vec![commute::BoxDim { w: 8.0, h: 7.0, d: 0.0 }; d.cells().len()];
    d.layout(&cells, &[commute::BoxDim { w: 4.0, h: 5.0, d: 1.0 }]).unwrap();
    let mut marks = Marks::default();
    svg::emit(&d, Color::BLACK, &mut Vec::new(), &mut marks);
    let (_, t) = marks.0.iter().find(|(b, _)| matches!(b, BoxRef::Label(_))).unwrap();
    // The arrow goes right and down, so the text's x axis points right and down on the screen, and its y axis (screen
    // down in SVG text's frame) points left and down.
    assert!(t.a > 0.0 && t.b > 0.0, "{t:?}");
    assert!(t.c < 0.0 && t.d > 0.0, "{t:?}");
    assert!((t.a * t.d - t.b * t.c - 1.0).abs() < 1e-9, "a rotation: {t:?}");
}

#[test]
fn colours() {
    assert_eq!(&hex(Color::BLACK), b"#000000");
    assert_eq!(&hex(Color::WHITE), b"#ffffff");
    assert_eq!(&hex(Color::Rgb(1.0, 0.5, 0.0)), b"#ff8000");
    assert_eq!(&hex(Color::Cmyk(1.0, 0.0, 0.0, 0.0)), b"#00ffff");
    assert_eq!(&hex(Color::Gray(f64::NAN)), b"#000000");
    assert_eq!(&hex(Color::Gray(7.0)), b"#ffffff");
}
