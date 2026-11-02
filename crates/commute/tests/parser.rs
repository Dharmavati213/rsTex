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
        r"A \arrow[r,/tikz/thick] & B",
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
    assert_eq!(refused(r"[foo/.code={x}] A"), "key handler other than .style and .append style");
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
    assert_eq!(refused(r"[foo/.style={#1}] A \arrow[r,foo=x] & B"), "style with a parameter");
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
    assert!((v[0] - 5.6).abs() < 1e-9 && (v[1] - 3.2).abs() < 1e-9);
    let mut d = accepted(r"A \arrow[r,dashed,thick] & B");
    lay_out(&mut d);
    let v = dash_of(&d).unwrap();
    assert!((v[0] - 7.0 * F.rt).abs() < 1e-9);
    // Later keys win per end, and direction keys do not accumulate.
    let mut d = accepted(r"A \arrow[r,d] & B \\ C");
    lay_out(&mut d);
    let (s, e) = line_of(&d, 0);
    assert!((s.x - e.x).abs() < 1e-6, "{s:?} {e:?}");
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
