//! The grammar of spec sections 3 and 5: what is recognised, what it means, and what is refused.

use commute::{AllowAll, BoxDim, Diagram, FontParams, Item, LabelFont, Policy, Seg, StandardLatex, Unsupported, find_environment};

const F: FontParams = FontParams::ARTICLE_10PT;

fn parse(src: &str) -> Result<Diagram, (String, String)> {
    parse_with(src, &StandardLatex)
}

fn parse_with(src: &str, policy: &dyn Policy) -> Result<Diagram, (String, String)> {
    let mut d = Diagram::new();
    match d.parse(b"", src.as_bytes(), F, policy) {
        Ok(()) => Ok(d),
        Err(Unsupported { reason, at }) => Err((reason.to_string(), String::from_utf8_lossy(d.text(at)).into_owned())),
    }
}

fn refused(src: &str) -> String {
    match parse(src) {
        Ok(_) => panic!("accepted: {src}"),
        Err((reason, _)) => reason,
    }
}

fn accepted(src: &str) -> Diagram {
    parse(src).unwrap_or_else(|e| panic!("refused {src}: {e:?}"))
}

fn lay_out(d: &mut Diagram) {
    let cells = vec![BoxDim { w: 8.0, h: 7.0, d: 0.0 }; d.cells().len()];
    let labels = vec![BoxDim { w: 4.0, h: 5.0, d: 1.0 }; d.labels().len()];
    d.layout(&cells, &labels).unwrap();
}

/// The first and last point of the arrow's line.
fn line_of(d: &Diagram, n: usize) -> (commute::Pt, commute::Pt) {
    let mut seen = 0;
    for item in &d.draw_list().items {
        if let Item::Stroke { role: commute::Role::Line, path, .. } = item {
            if seen == n {
                let segs = d.draw_list().path(*path);
                let first = match segs[0] {
                    Seg::Move(p) => p,
                    _ => panic!(),
                };
                let last = match *segs.last().unwrap() {
                    Seg::Line(p) | Seg::Curve(_, _, p) => p,
                    _ => panic!(),
                };
                return (first, last);
            }
            seen += 1;
        }
    }
    panic!("no line {n}");
}

fn content(d: &Diagram, cell: usize) -> String {
    d.cell_content(cell).map(|s| String::from_utf8_lossy(s).into_owned()).collect::<Vec<_>>().join("|")
}

#[test]
fn cells_rows_and_empties() {
    let d = accepted(r"A & B \\ C");
    assert_eq!((d.rows(), d.cols(), d.cells().len()), (2, 2, 3));
    assert_eq!(content(&d, 1), "B");
    assert!(d.cells().iter().all(|c| c.has_node));

    // Truly empty: no node.  {} and a cell with only an arrow: a node.
    let d = accepted(r" & {} & \arrow[l] ");
    let has: Vec<bool> = d.cells().iter().map(|c| c.has_node).collect();
    assert_eq!(has, [false, true, true]);

    // nodes in empty cells.
    let d = accepted(r"[nodes in empty cells] A & \\ & B");
    assert!(d.cells().iter().all(|c| c.has_node));

    // \\ right before the end makes an extra row with one empty cell.
    let d = accepted("A & B \\\\\n");
    assert_eq!(d.rows(), 2);
    assert_eq!(d.cells().len(), 3);
    assert!(!d.cells()[2].has_node);

    // Text around an arrow command stays in the cell, in pieces.
    let d = accepted(r"A \arrow[r] B & C");
    assert_eq!(content(&d, 0), "A|B");
}

#[test]
fn comments_are_not_text() {
    let d = accepted("A % comment & \\\\ {\n  \\arrow[r] % another\n  &\n  B");
    assert_eq!(d.rows(), 1);
    assert_eq!(content(&d, 0), "A");
    assert_eq!(d.arrows(), 1);
    // A comment does not glue a control word to what follows.
    let d = accepted("\\alpha%\nb & c");
    assert_eq!(content(&d, 0), "\\alpha b");
    // An escaped percent is text.
    let d = accepted(r"5\% & c");
    assert_eq!(content(&d, 0), r"5\%");
    // A comment-only line is not an empty line; an empty line is \par.
    accepted("A %\n%\n& B");
    assert_eq!(refused("A\n\n& B"), "empty line (\\par) inside the diagram");
    assert_eq!(refused("A\n  \n& B"), "empty line (\\par) inside the diagram");
    assert_eq!(refused("A %\n\n& B"), "empty line (\\par) inside the diagram");
    assert_eq!(refused(r"A \par & B"), "\\par inside the diagram");
}

#[test]
fn option_list_forms() {
    // Bracket delimited by the first ] outside braces.
    let d = accepted(r#"A \arrow[r,"{[x]}"] & B"#);
    assert_eq!(d.text(d.labels()[0].text), b"[x]");
    // Commas in braces; = in text.
    let d = accepted(r#"A \arrow[r,"{f,g}","a=b"] & B"#);
    assert_eq!(d.text(d.labels()[0].text), b"f,g");
    assert_eq!(d.text(d.labels()[1].text), b"a=b");
    // Empty items and spaces.
    let d = accepted(r#"A \arrow[ r , , "f" ,] & B"#);
    assert_eq!(d.labels().len(), 1);
    // Options after a label's closing brace are ignored; before it, swap.
    accepted(r#"A \arrow[r,"f"{near start} anything=here] & B"#);
    accepted(r#"A \arrow[r,"f"'{near start}] & B"#);
    accepted(r#"A \arrow[r,"f"' near start] & B"#);
    assert_eq!(refused(r#"A \arrow[r,"f] & B"#), "label without a closing quote");
    assert_eq!(refused(r"A \arrow[r & B"), "unterminated arrow options");
}

#[test]
fn old_syntax() {
    // \arrow{r} and labels.
    let d = accepted(r"A \arrow{r}{f}{g} & B");
    assert_eq!(d.arrows(), 1);
    assert_eq!(d.labels().len(), 2);
    assert_eq!(d.text(d.labels()[1].text), b"g");
    // Options, then the target, then labels with options.
    let d = accepted(r"A \arrow[two heads]{r}[swap]{p}[near end]{q} & B");
    assert_eq!(d.labels().len(), 2);
    // Shortcuts.
    let mut d = accepted(r"A \rar \dar & B \\ C \urar & D \ar[u] \ular \lar");
    assert_eq!(d.arrows(), 6);
    lay_out(&mut d);
    let mut d = accepted(r"A \drar[hook]{f} & \\ & B \ular[dashed]");
    assert_eq!(d.labels().len(), 1);
    lay_out(&mut d);
    // \arrow[r] {B}: the group is the target, placed before the options, so r wins and B is gone from the cell.
    let mut d = accepted(r"A \arrow[r] {B} & C");
    assert_eq!(content(&d, 0), "A");
    lay_out(&mut d);
    // Without brackets or braces the next token would be the target.
    assert_eq!(refused(r"A \arrow & B"), "\\arrow without [ or {");
    assert_eq!(refused(r"A \arrow{r}[swap] f & B"), "positional label options without a label in braces");
}

#[test]
fn control_sequences_that_may_hide_structure_are_refused() {
    for s in [r"\pb & B", r"A \nextrow C", r"A & \cellA", r"\tikzset{x} & B", r"\def\x{y} A", r"\path (0,0); A"] {
        assert_eq!(refused(s), "control sequence that may hide structure", "{s}");
    }
    assert_eq!(refused(r#"A \arrow[r,"\mine"] & B"#), "control sequence that may hide structure");
    assert_eq!(refused(r"\begin{mine} a \end{mine} & B"), "environment that may hide structure");
    // With a policy that knows better, all of them are text.
    let d = parse_with(r"\pb & B", &AllowAll).unwrap();
    assert_eq!(content(&d, 0), r"\pb");
    // What is standard stays.
    accepted(r"\alpha_{\mathrm{x}}^{2} & \frac{a}{b} & \displaystyle\bigoplus_{i\in I} A_i & \begin{pmatrix} a & b \\ c & d \end{pmatrix}");
}

#[test]
fn structure_errors_are_the_oracles_to_report() {
    assert_eq!(refused(r"A \begin{tikzcd} B \end{tikzcd}"), "nested tikzcd");
    assert_eq!(refused(r"A \end{tikzcd}"), "\\end without its \\begin");
    assert_eq!(refused(r"{A & B}"), "cell separator inside a group");
    assert_eq!(refused(r"A }"), "unbalanced braces");
    assert_eq!(refused(r"{A"), "unbalanced braces");
    assert_eq!(refused(r"\begin{matrix} a"), "unclosed environment");
    assert_eq!(refused(r"\begin{matrix} a \end{pmatrix}"), "\\end without its \\begin");
    assert_eq!(refused(r"{\arrow[r]} & B"), "arrow command inside a group");
    assert_eq!(refused(r"$x$ & B"), "$ in a math cell");
    assert_eq!(refused(r"# & B"), "# in a cell");
    assert_eq!(refused(r"\[ a \] & B"), "math shift inside the diagram");
    assert_eq!(refused(r"|x| & B"), "cell starting with | but not |[...]|");
    assert_eq!(refused("A \\"), "a backslash at the end of the diagram");
    assert_eq!(refused(r"A \\* B"), "\\\\*");
    // Text cells take $ and no \\[.
    accepted(r"[math mode=false] $x$ & B");
    accepted(r"\text{$x$} & B");
}

#[test]
fn unknown_keys_are_refused() {
    for s in [
        r"A \arrow[r,Rrightarrow] & B",
        r"A \arrow[r,tail reversed] & B",
        r"A \arrow[r,-Stealth] & B",
        r"A \arrow[r,|->] & B",
        r"A \arrow[r,relative] & B",
        r"A \arrow[r,controls={+(1,1) and +(1,1)}] & B",
        r"A \arrow[r,transform nodes={xshift=1pt}] & B",
        r"A \arrow[r,every arrow] & B",
        r"A \arrow[r,font=\tiny] & B",
        r"A \arrow[r,rounded corners] & B",
        r"A \arrow[r,foo=bar] & B",
    ] {
        assert_eq!(refused(s), "unknown arrow key", "{s}");
    }
    assert_eq!(refused(r#"A \arrow[r,"f"{draw}] & B"#), "unknown label key");
    assert_eq!(refused(r#"A \arrow[r,"f"{auto=right}] & B"#), "unknown label key");
    assert_eq!(refused(r"|[fill=red]| A"), "unknown cell key");
    assert_eq!(refused(r"|[shape=circle]| A"), "unknown cell key");
    assert_eq!(refused(r"[nodes={red}] A"), "unknown diagram option");
    assert_eq!(refused(r"[execute before arrows={\draw (0,0);}] A"), "unknown diagram option");
    assert_eq!(refused(r"[transform shape] A"), "unknown diagram option");
    assert_eq!(refused(r#"["f"] A"#), "label in the diagram's options");
    assert_eq!(refused(r"[arrow style=math font] A"), "arrow style other than Latin Modern and tikz");
    assert_eq!(refused(r"[every arrow/.style={thick}] A"), "tikz-cd's own styles are modified");
    assert_eq!(refused(r"[hook/.style={thick}] A"), "tikz-cd's own styles are modified");
    assert_eq!(refused(r"[1-row diagram/.style={}] A"), "N-row diagram style");
    assert_eq!(refused(r"[row sep/foo=1cm] A"), "separation name tikz-cd does not define");
    assert_eq!(refused(r"A \arrow[r,red!x] & B"), "colour not known without xcolor");
    assert_eq!(refused(r"A \arrow[r,color=mine] & B"), "colour not known without xcolor");
    assert_eq!(refused(r"A \arrow[r,shorten=\the\dimen0] & B"), "expression outside the supported subset of pgfmath");
    assert_eq!(refused(r"A \arrow[r,double distance=2pt] & B"), "double distance without double");
    assert_eq!(refused(r"[baseline=(X)] A"), "baseline coordinate outside the supported forms");
    assert_eq!(refused(r"[baseline=(current bounding box.base)] A"), "baseline coordinate outside the supported forms");
    assert_eq!(refused(r"A \arrow[r,squiggly={segment length=1pt}] & B"), "decoration key other than pre and post length");
    assert_eq!(refused(r"A \arrow[r,squiggly={pre length=2\pgflinewidth}] & B"), "decoration length in terms of the line width");
}

#[test]
fn styles() {
    // A style is its keys.
    let mut d = accepted(r#"[foo/.style={hook,"x"}] A \arrow[r,foo] & B"#);
    assert_eq!(d.labels().len(), 1);
    lay_out(&mut d);
    // Definitions with no effect in this key path are harmless (spec 5.1).
    accepted(r"[every node/.style={scale=.5},column 2/.style={red},row 1/.style={font=\tiny}] A & B");
    // A parameter, a cycle, and exponential growth are refused.
    assert_eq!(refused(r"[foo/.style={#1,#2}] A \arrow[r,foo=x] & B"), "style with a parameter");
    assert_eq!(refused(r"[foo/.style={##1}] A \arrow[r,foo=x] & B"), "style with a parameter");
    assert_eq!(refused(r"[foo/.style={red,#1}] A \arrow[r,foo] & B"), "style with a parameter and no value");
    assert_eq!(refused(r"[foo/.style={foo}] A \arrow[r,foo] & B"), "key lists nested too deeply");
    assert_eq!(
        refused(concat!(
            "[a/.style={b,b,b,b,b,b,b,b},b/.style={c,c,c,c,c,c,c,c},c/.style={d,d,d,d,d,d,d,d},",
            "d/.style={e,e,e,e,e,e,e,e},e/.style={r}] A \\arrow[a] & B"
        )),
        "user styles expand too far"
    );
    // Presets run first, the diagram's options later.
    let mut d = Diagram::new();
    d.parse(br"arrows={thick},every label/.append style={inner sep=3pt},mine/.style={dashed}", br"A \arrow[r,mine,red,thin] & B", F, &StandardLatex).unwrap();
    lay_out(&mut d);
    // A separation set outside the diagram is overwritten by `every diagram`.
    assert_eq!(
        Diagram::new().parse(br"row sep=large", b"A", F, &StandardLatex).unwrap_err().reason,
        "separation set outside the diagram"
    );
}

#[test]
fn separations_only_count_where_the_column_is_created() {
    let mut a = accepted(r"A &[1cm] B \\ C & D");
    lay_out(&mut a);
    let mut b = accepted(r"A & B \\ C &[1cm] D");
    lay_out(&mut b);
    assert!((a.result().matrix_w - b.result().matrix_w - 28.45274).abs() < 1e-3);
    // Between origins drops the widths of inner columns.
    let mut d = accepted(r"[column sep={2cm,between origins}] A & BBBBBB & C");
    lay_out(&mut d);
    let w = d.result().matrix_w;
    assert!((w - (2.0 * 56.90548 + 8.0 + 2.0 * 4.30554)).abs() < 1e-3, "{w}");
    // Named sizes per axis, and dimensions.
    let mut d = accepted(r"[sep=small] A \\ B");
    lay_out(&mut d);
    let mut e = accepted(r"[row sep=0.9em] A \\ B");
    lay_out(&mut e);
    assert_eq!(d.result().matrix_h, e.result().matrix_h);
}

#[test]
fn key_time_values_follow_key_order() {
    let dash_of = |d: &Diagram| {
        d.draw_list().items.iter().find_map(|i| match i {
            Item::ArrowBegin { dash, .. } => Some(dash.values().to_vec()),
            _ => None,
        })
    };
    let mut d = accepted(r"A \arrow[r,thick,dashed] & B");
    lay_out(&mut d);
    let v = dash_of(&d).unwrap();
    assert!((v[0] - 5.6).abs() < 1e-4 && (v[1] - 3.2).abs() < 1e-4);
    let mut d = accepted(r"A \arrow[r,dashed,thick] & B");
    lay_out(&mut d);
    let v = dash_of(&d).unwrap();
    assert!((v[0] - 7.0 * F.rt).abs() < 1e-3);
    // Later keys win per end, and direction keys do not accumulate.
    let mut d = accepted(r"A \arrow[r,d] & B \\ C");
    lay_out(&mut d);
    let (s, e) = line_of(&d, 0);
    assert!((s.x - e.x).abs() < 1e-6, "{s:?} {e:?}");
}

#[test]
fn the_last_double_key_sets_the_double_line() {
    // The inner stroke's width and colour.
    let inner = |src: &str| {
        let mut d = accepted(src);
        lay_out(&mut d);
        d.draw_list()
            .items
            .iter()
            .find_map(|i| match *i {
                Item::Stroke { role: commute::Role::Inner, width, color, .. } => Some((width, color)),
                _ => None,
            })
            .unwrap()
    };
    let (line, white) = inner(r"A \arrow[r,Rightarrow] & B");
    assert!((line - (0.45 * F.ex - F.rt)).abs() < 1e-4, "{line}");
    assert_eq!(white, Some(commute::Color::WHITE));
    // `double line` sets the distance and the inner colour, `double` neither, `double distance` the one and `double=<c>` the other.
    for src in [r"A \arrow[r,double,Rightarrow] & B", r"A \arrow[r,Rightarrow,double] & B", r"A \arrow[r,double distance=2pt,Rightarrow] & B", r"[arrows={double}] A \arrow[r,equal] & B"] {
        assert_eq!(inner(src), (line, white), "{src}");
    }
    assert_eq!(inner(r"A \arrow[r,double=red,Rightarrow] & B"), (line, white));
    assert_eq!(inner(r"A \arrow[r,Rightarrow,double=red] & B"), (line, Some(commute::Color::Rgb(1.0, 0.0, 0.0))));
    assert_eq!(inner(r"A \arrow[r,Rightarrow,double distance=2pt] & B").0, 2.0);
    assert_eq!(inner(r"A \arrow[r,Rightarrow,double,double distance=1pt] & B").0, 1.0);
    assert!((inner(r"A \arrow[r,double] & B").0 - 0.6).abs() < 1e-4);
}

#[test]
fn labels_get_their_settings_after_all_arrow_options() {
    let d = accepted(r#"A \arrow[r,"f",near start,swap,phantom] & B"#);
    let l = &d.labels()[0];
    assert_eq!(l.font, LabelFont::Text);
    let d = accepted(r#"A \arrow[r,"f"{font=\tiny}] & B"#);
    assert!(matches!(d.labels()[0].font, LabelFont::Command(s) if d.text(s) == b"\\tiny"));
    let d = accepted(r#"A \arrow[r,"f"] & B"#);
    assert_eq!(d.labels()[0].font, LabelFont::Script);
    let d = accepted(r#"A \arrow[r,red,"f",draw=blue,"g"{text=green}] & B"#);
    assert_eq!(d.labels()[0].color, Some(commute::Color::Rgb(1.0, 0.0, 0.0)));
    assert_eq!(d.labels()[1].color, Some(commute::Color::Rgb(0.0, 1.0, 0.0)));
    // `text=`, `color=` and a colour by its name all set `\tikz@textcolor`: the last one wins, those of `arrows=` first.
    let d = accepted(r#"[arrows={text=red}] A \arrow[r,text=red,color=blue,"f"] \arrow[d,text=red,blue,"g"] \arrow[dr,blue,text=red,draw=blue,"h"] \arrow[rr,color=blue,"k"] & B & E \\ C & D"#);
    let colors: Vec<_> = d.labels().iter().map(|l| l.color).collect();
    let (red, blue) = (Some(commute::Color::Rgb(1.0, 0.0, 0.0)), Some(commute::Color::Rgb(0.0, 0.0, 1.0)));
    assert_eq!(colors, [blue, blue, red, blue]);
}

#[test]
fn arrow_ends() {
    let layout_err = |s: &str| {
        let mut d = accepted(s);
        let cells = vec![BoxDim::default(); d.cells().len()];
        let labels = vec![BoxDim::default(); d.labels().len()];
        d.layout(&cells, &labels).err().map(|e| e.reason)
    };
    let bad = Some("arrow end is not a node");
    assert_eq!(layout_err(r"A \arrow[r] & "), bad);
    assert_eq!(layout_err(r"A \arrow[rr] & B"), bad);
    assert_eq!(layout_err(r"A \arrow[u] & B"), bad);
    assert_eq!(layout_err(r"A \arrow[to=2-1] & B"), bad);
    assert_eq!(layout_err(r"A \arrow[to=0-1] & B"), bad);
    assert_eq!(layout_err(r"A \arrow[to=X] & B"), bad);
    assert_eq!(layout_err(r"A \arrow[r,from=X] & B"), bad);
    // A name must exist when the arrow runs; an alias can be anywhere.
    assert_eq!(layout_err(r#"A \arrow[to=U] \arrow[r,"f"{name=U}] & B"#), bad);
    assert_eq!(layout_err(r#"A \arrow[r,"f"{name=U}] \arrow[to=U] & B"#), None);
    assert_eq!(layout_err(r"A \arrow[to=X] & |[alias=X]| B"), None);
    assert_eq!(layout_err(r"A \arrow[to=1-2] & B"), None);
    assert_eq!(layout_err(r"A \arrow[to=\tikzcdmatrixname-1-2] & B"), None);
    assert_eq!(layout_err(r"A \arrow[] & B"), None);
    // A loop stays on its cell whatever the target.
    assert_eq!(layout_err(r"A \arrow[rrr,loop] & B"), None);
    // ...unless a later curve key undoes it (\tikz@to@switch@on).
    assert_eq!(layout_err(r"A \arrow[rrr,loop,looseness=2] & B"), bad);
}

#[test]
fn layout_needs_a_diagram_and_enough_boxes() {
    let mut d = Diagram::new();
    assert_eq!(d.layout(&[], &[]).unwrap_err().reason, "layout without a parsed diagram");
    d.parse(b"", b"A & B", F, &StandardLatex).unwrap();
    assert_eq!(d.layout(&[BoxDim::default()], &[]).unwrap_err().reason, "fewer measured boxes than cells and labels");
    d.parse(b"", b"A \\arrow[foo] & B", F, &StandardLatex).unwrap_err();
    assert_eq!(d.layout(&[BoxDim::default(); 2], &[]).unwrap_err().reason, "layout without a parsed diagram");
}

#[test]
fn environments_can_be_found_in_text() {
    let src = br"x \begin{tikzcd} A \end{tikzcd} y \begin{tikzcd}[sep=small] B \end{tikzcd} \begin{tikzcd} C";
    let (a, b) = find_environment(src, 0).unwrap();
    assert_eq!(&src[a..b], b" A ");
    let (c, d) = find_environment(src, commute::after_environment(b)).unwrap();
    assert_eq!(&src[c..d], b"[sep=small] B ");
    assert_eq!(find_environment(src, commute::after_environment(d)), None);
    assert_eq!(find_environment(b"", 5), None);
}

fn with_preset(preset: &str, env: &str) -> Result<Diagram, &'static str> {
    let mut d = Diagram::new();
    d.parse(preset.as_bytes(), env.as_bytes(), FontParams::ARTICLE_10PT, &StandardLatex).map_err(|u| u.reason)?;
    Ok(d)
}

#[test]
fn a_style_takes_a_value() {
    let d = with_preset(r##"lab/.style={"#1"}"##, r"A \arrow[r,lab=f] \arrow[d,lab=g] & B \\ C").unwrap();
    let labels: Vec<&[u8]> = d.labels().iter().map(|l| d.text(l.text)).collect();
    assert_eq!(labels, [&b"f"[..], b"g"]);
    with_preset(r"dflt/.style={draw=#1},dflt/.default=red", r"A \arrow[r,dflt] & B").unwrap();
    // A style that doubles its value in every pass.
    assert_eq!(with_preset(r"x/.style={x=#1#1}", r"A \arrow[r,x=a] & B").err(), Some("key lists nested too deeply"));
}

#[test]
fn keys_have_a_directory() {
    // `\tikzset`: `shift left` is tikz-cd's alone, and `dashed` is another one.
    assert_eq!(
        with_preset(r"/tikz/.cd,s/.style={shift left},/tikz/commutative diagrams/.cd", r"A \arrow[r,s] & B").err(),
        Some("a TikZ key under a name of tikz-cd's")
    );
    with_preset(r"/tikz/.cd,s/.style={dashed},/tikz/commutative diagrams/.cd", r"A \arrow[r,s] & B").unwrap();
    // What is not in the way of a diagram, and what is.
    with_preset(r"/tikz/.cd,own/.cd,a/.initial=1,b=2,/tikz/commutative diagrams/.cd", r"A").unwrap();
    with_preset(r"/other/tree/.cd,every arrow/.style={red},row sep=1cm,/tikz/commutative diagrams/.cd", r"A").unwrap();
    for bad in ["every node", "every path", "every matrix", "column 2", "row 1"] {
        let preset = format!("/tikz/.cd,{bad}/.style={{red}},/tikz/commutative diagrams/.cd");
        assert_eq!(with_preset(&preset, "A").err(), Some("a style TikZ runs by itself"), "{bad}");
    }
    assert_eq!(with_preset(r"/pgf/.cd,x=1", "A").err(), Some("key under /pgf"));
    // A cell's options are in `/tikz`, and a key of tikz-cd's directory is an error there.
    assert_eq!(with_preset(r"s/.style={inner sep=1pt}", r"|[s]| A").err(), Some("unknown cell key"));
    with_preset(r"s/.style={inner sep=1pt}", r"[cells={s}] A").unwrap();
    with_preset(r"/tikz/.cd,s/.style={inner sep=1pt},/tikz/commutative diagrams/.cd", r"|[s]| A").unwrap();
    // `a key` is what TeX read.
    accepted("A \\arrow[r,bend\n    left,shift    right] & B");
}

#[test]
fn keys_made_by_code() {
    with_preset(r"c/.code={\relax}", "A").unwrap();
    assert_eq!(with_preset(r"c/.code={\relax}", r"A \arrow[r,c] & B").err(), Some("key defined by code"));
    assert_eq!(with_preset(r"c/.style 2 args={#1}", r"A \arrow[r,c] & B").err(), Some("key defined by code"));
    assert_eq!(with_preset(r"thick/.code={\relax}", r"A \arrow[r,thick] & B").err(), Some("key defined by code"));
    assert_eq!(with_preset(r"hook/.code={\relax}", "A").err(), Some("tikz-cd's own styles are modified"));
    assert_eq!(with_preset(r"c/.unknown={}", "A").err(), Some("key handler other than those that define a key"));
    with_preset(r"t/.code={\pgfsetarrowsstart{tikzcd to}}", r"A \arrow[r,t] & B").unwrap();
}

#[test]
fn names_of_separations() {
    let spaced = with_preset(r"row sep/roomy/.initial=3cm", r"[row sep=roomy] A \\ B").unwrap();
    assert_eq!(spaced.rows(), 2);
    with_preset(r"row sep/roomy/.initial=3cm,row sep/roomy=4cm", r"[row sep=roomy] A \\ B").unwrap();
    assert_eq!(with_preset(r"row sep/roomy=3cm", "A").err(), Some("separation name tikz-cd does not define"));
}

#[test]
fn what_a_tip_has_become_shows_on_arrows_that_draw_it() {
    with_preset(r"tikzcd left hook/.tip={Bar}", r"A \arrow[r] & B").unwrap();
    assert_eq!(with_preset(r"tikzcd left hook/.tip={Bar}", r"A \arrow[r,hook] & B").err(), Some("a tip redefined by the document"));
    assert_eq!(with_preset(r"cm to/.tip={Bar}", r"A \arrow[r] & B").err(), Some("a tip redefined by the document"));
    with_preset(r"cm to/.tip={Bar}", r"A \arrow[r,dash] & B").unwrap();
    with_preset(r"cm to/.tip={Bar}", r"A \arrow[r,draw=none] & B").unwrap();
    // `>` only shows on `/tikz`'s arrows and under `arrow style=tikz`.
    with_preset("", r"[>=Latex] A \arrow[r] & B").unwrap();
    assert_eq!(with_preset("", r"[>=Latex] A \arrow[r,->] & B").err(), Some("an arrow tip set by >"));
    assert_eq!(with_preset("arrow style=tikz", r"[>=Latex] A \arrow[r] & B").err(), Some("an arrow tip set by >"));
    with_preset("arrow style=tikz", r"[>=stealth] A \arrow[r] & B").unwrap();
    assert_eq!(with_preset("arrow style=tikz,arrow style=Latin Modern", "A").err(), Some("a second arrow style"));
}

#[test]
fn scale_on_the_diagram() {
    accepted(r"[scale=1] A \arrow[r] & B");
    assert_eq!(refused(r"[scale=2] A \arrow[r] & B"), "scale on the diagram");
}

/// A caller that knows the document's keys, as the engine does from its macros.
struct Keys;

impl commute::UserKeys for Keys {
    fn user_key(&self, name: &[u8], out: &mut Vec<u8>) -> commute::UserKey {
        let (place, text): (_, &[u8]) = match name {
            b"mono" => (commute::UserKey::Cd, b"\\pgfkeysalso {hook,#1}\\pgfkeysalso {\"f\"}"),
            b"tsty" => (commute::UserKey::Tikz, b"\\pgfkeysalso {dashed}"),
            b"code" => (commute::UserKey::Cd, b"\\pgfsetlinewidth {1pt}"),
            b"thin" => (commute::UserKey::Tikz, b"\\pgfkeysalso {line width=#1}"),
            b"dflt" => (commute::UserKey::Cd, b"\\pgfkeysalso {line width=#1}"),
            b"curve" => (
                commute::UserKey::Tikz,
                b"\\pgfkeysalso {settings={#1},to path={(\\tikztostart )..controls ($(\\tikztostart )!\\pv {pos}!(\\tikztotarget )!\\pv {height}!270:(\\tikztotarget )$)and ($(\\tikztostart )!1-\\pv {pos}!(\\tikztotarget )!\\pv {height}!270:(\\tikztotarget )$)..(\\tikztotarget )\\tikztonodes }}",
            ),
            b"settings" => (commute::UserKey::Tikz, b"\\tikzset {quiver/.cd,#1}\\def \\pv ##1{\\pgfkeysvalueof {/tikz/quiver/##1}}"),
            _ => return commute::UserKey::Standard,
        };
        out.extend_from_slice(text);
        place
    }

    fn default_value(&self, name: &[u8], out: &mut Vec<u8>) -> bool {
        if name != b"dflt" {
            return false;
        }
        out.extend_from_slice(b"2pt");
        true
    }

    fn color(&self, name: &[u8]) -> Option<commute::Color> {
        (name == b"mine").then_some(commute::Color::Rgb(0.25, 0.5, 0.75))
    }
}

#[test]
fn keys_from_where_they_are() {
    let mut d = Diagram::new();
    let font = FontParams::ARTICLE_10PT;
    let parse = |d: &mut Diagram, env: &str| d.parse_with(b"", env.as_bytes(), font, &StandardLatex, &Keys).map_err(|u| u.reason);
    parse(&mut d, r"A \arrow[r,mono=thick] & B").unwrap();
    assert_eq!(d.labels().len(), 1);
    assert_eq!(d.text(d.labels()[0].text), b"f");
    parse(&mut d, r"A \arrow[r,tsty] \arrow[d,tsty] & B \\ C").unwrap();
    assert_eq!(parse(&mut d, r"A \arrow[r,code] & B"), Err("key defined by code"));
    // A key of TikZ's that the document changed is the document's.
    assert_eq!(parse(&mut d, r"A \arrow[r,thin] & B"), Err("style with a parameter and no value"));
    parse(&mut d, r"A \arrow[r,thin=2pt] & B").unwrap();
    parse(&mut d, r"A \arrow[r,dflt] & B").unwrap();
    // quiver's `curve`, as TeX prints it.
    parse(&mut d, r"A \arrow[r,curve={height=-12pt}] & B").unwrap();
    parse(&mut d, r#"A \arrow[r,mine,"f"{mine},draw=mine] & |[mine]| B"#).unwrap();
    assert_eq!(parse(&mut d, r"A \arrow[r,draw=other] & B"), Err("colour not known without xcolor"));
    // And the next diagram starts from nothing.
    parse(&mut d, r"A \arrow[r,thick] & B").unwrap();
    assert_eq!(d.labels().len(), 0);
}

#[test]
fn quiver_is_known_by_its_text() {
    let quiver = concat!(
        r"/tikz/.cd,curve/.style={settings={#1},to path={(\tikztostart)",
        "\n  .. controls ($(\\tikztostart)!\\pv{pos}!(\\tikztotarget)!\\pv{height}!270:(\\tikztotarget)$)",
        "\n  and ($(\\tikztostart)!1-\\pv{pos}!(\\tikztotarget)!\\pv{height}!270:(\\tikztotarget)$)",
        "\n  .. (\\tikztotarget)\\tikztonodes}},",
        "settings/.code={\\tikzset{quiver/.cd,#1}\n \\def\\pv##1{\\pgfkeysvalueof{/tikz/quiver/##1}}},",
        "quiver/.cd,pos/.initial=0.35,height/.initial=0,/tikz/commutative diagrams/.cd"
    );
    with_preset(quiver, r"A \arrow[r,curve={height=-12pt}] & B").unwrap();
    assert_eq!(with_preset(&format!("{quiver},/tikz/quiver/.cd,pos=0.5"), "A").err(), Some("a default of quiver changed"));
    assert_eq!(with_preset(quiver, r"A \arrow[r,curve={width=2pt}] & B").err(), Some("curve key other than height and pos"));
    assert_eq!(
        with_preset(r"/tikz/.cd,curve/.style={settings={#1},to path={(a)--(b)}},/tikz/commutative diagrams/.cd", r"A \arrow[r,curve=x] & B").err(),
        Some("unknown arrow key")
    );
}
