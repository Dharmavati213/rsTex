//! The commute command, as a user runs it.  Files go under cargo's temporary directory for tests, and so does the cache with the format, which the first test to get there builds.
//! What needs real TeX does nothing on a machine with no TeX tree.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const SQUARE: &str = "x\n\\begin{tikzcd}\nA \\arrow[r,\"f\"] \\arrow[d,\"g\"'] & B \\arrow[d,\"h\"] \\\\\nC \\arrow[r,\"k\"'] & D\n\\end{tikzcd}\ny\n";

fn tmp(name: &str) -> PathBuf {
    let d = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// DESIGN.md section 12: the environment variable that puts the cache directory under some directory.
const CACHE_VAR: &str = if cfg!(windows) {
    "LOCALAPPDATA"
} else if cfg!(target_os = "macos") {
    "HOME"
} else {
    "XDG_CACHE_HOME"
};

fn commute() -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_commute"));
    c.env(CACHE_VAR, Path::new(env!("CARGO_TARGET_TMPDIR")).join("cache")).stdin(Stdio::null());
    c
}

/// Whether there is a tree to build a format from.
fn tree() -> bool {
    static THERE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *THERE.get_or_init(|| {
        let d = tmp("tree-probe");
        let input = d.join("a.tex");
        std::fs::write(&input, "A").unwrap();
        commute().arg(&input).output().unwrap().status.code() != Some(2)
    })
}

/// Whether real tikz-cd on this tree is what the library has been verified on: `rstex --check-tree`, and the other answer is as much a real expectation as this one.
fn armed() -> bool {
    let out = commute().arg("--check-tree").output().unwrap();
    let said = String::from_utf8_lossy(&out.stdout);
    let armed = said.lines().any(|l| l.starts_with("path=tikz-cd state=armed"));
    assert!(armed || said.lines().any(|l| l.starts_with("path=tikz-cd state=off")), "{said}");
    armed
}

/// A PDF as qpdf writes it out, with the fonts' names in the page's resources left out, and the rest of the file as it stands if there is no qpdf.  A font's number depends on what was loaded before it.
fn pdf_normal(pdf: &Path) -> Vec<u8> {
    let Ok(out) = Command::new("qpdf").args(["--qdf", "--object-streams=disable", "--deterministic-id"]).arg(pdf).arg("-").output() else {
        return Vec::new();
    };
    let mut res = Vec::with_capacity(out.stdout.len());
    let mut i = 0;
    while i < out.stdout.len() {
        res.push(out.stdout[i]);
        if out.stdout[i] == b'/' && out.stdout.get(i + 1) == Some(&b'F') {
            i += 2;
            res.push(b'F');
            while out.stdout.get(i).is_some_and(u8::is_ascii_digit) {
                i += 1;
            }
            continue;
        }
        i += 1;
    }
    // The cross-reference table says where things are, and a name's digits move those.  The rest of the trailer is qpdf's hash of it all.
    if let Some(at) = res.windows(6).rposition(|w| w == b"\nxref\n") {
        res.truncate(at);
    }
    res
}

/// The content stream is uncompressed nowhere in the file, so this takes a look through qpdf if it is there.
fn qpdf_stream(pdf: &Path) -> Option<String> {
    let out = Command::new("qpdf").args(["--qdf", "--object-streams=disable"]).arg(pdf).arg("-").output().ok()?;
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    let s = text.find("stream\n")? + "stream\n".len();
    let e = text[s..].find("endstream")? + s;
    Some(text[s..e].to_string())
}

fn media_box(pdf: &[u8]) -> (f64, f64) {
    let text = String::from_utf8_lossy(pdf);
    let at = text.find("/MediaBox [0 0 ").unwrap() + "/MediaBox [0 0 ".len();
    let rest = &text[at..text[at..].find(']').unwrap() + at];
    let mut it = rest.split_whitespace().map(|x| x.parse::<f64>().unwrap());
    (it.next().unwrap(), it.next().unwrap())
}

#[test]
fn writes_a_cropped_pdf_and_svg() {
    if !tree() {
        return;
    }
    let dir = tmp("square");
    let input = dir.join("square.tex");
    std::fs::write(&input, SQUARE).unwrap();
    let out = commute().arg("--box").arg(&input).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    // Nothing of TeX's reaches the terminal: only what was asked for.
    let said = String::from_utf8_lossy(&out.stdout);
    assert!(said.trim_end().ends_with("pt") && said.lines().count() == 1, "{said}");
    assert!(out.stderr.is_empty());

    let pdf = std::fs::read(dir.join("square.pdf")).unwrap();
    assert!(pdf.starts_with(b"%PDF-") && pdf.ends_with(b"%%EOF\n"));
    let (w, h) = media_box(&pdf);
    assert!(w > 40.0 && w < 90.0 && h > 30.0 && h < 70.0, "{w} {h}");
    if let Some(stream) = qpdf_stream(&dir.join("square.pdf")) {
        assert_eq!(stream.matches(" Tf ").count(), 8, "four cells and four labels: {stream}");
    }
    // Real fonts: a subset of Computer Modern, embedded.
    let text = String::from_utf8_lossy(&pdf);
    assert!(text.contains("+CMMI10") && text.contains("/FontFile"), "{}", &text[..200]);

    let svg = std::fs::read_to_string(dir.join("square.svg")).unwrap();
    assert!(svg.starts_with("<svg ") && svg.ends_with("</svg>\n"));
    assert!(!svg.contains("<text"), "outlines, not text");
    assert_eq!(svg.matches("<path").count(), 16, "four lines, four tips, four cells and four labels");
    assert_eq!(svg.matches("<g ").count(), svg.matches("</g>").count());
}

#[test]
fn a_body_works_and_stdin_too() {
    if !tree() {
        return;
    }
    let dir = tmp("stdin");
    let mut child = commute().args(["-", "-o"]).arg(&dir).args(["--stem", "d", "--svg"]).stdin(Stdio::piped()).spawn().unwrap();
    use std::io::Write;
    child.stdin.as_mut().unwrap().write_all(br"[sep=large] \alpha \arrow[r,hook] & \mathrm{Hom}").unwrap();
    assert!(child.wait().unwrap().success());
    assert!(dir.join("d.svg").exists());
    assert!(!dir.join("d.pdf").exists());
}

#[test]
fn several_diagrams_are_numbered_and_unsupported_ones_named() {
    if !tree() {
        return;
    }
    let dir = tmp("several");
    let input = dir.join("s.tex");
    let src = "\\begin{tikzcd} A \\arrow[r] & B \\end{tikzcd}\n\\begin{tikzcd} A \\arrow[r,-Stealth] & B \\end{tikzcd}\n\\begin{tikzcd}[baseline=1ex] A \\arrow[to=3-3] \\end{tikzcd}\n\\begin{tikzcd} A \\arrow[r,Rightarrow,\"f\"] & \\Unheard \\end{tikzcd}\n\\begin{tikzcd} A \\arrow[r,Rightarrow,\"f\"] & B \\end{tikzcd}\n";
    std::fs::write(&input, src).unwrap();
    let out = commute().arg(&input).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("diagram 2 (line 2): not handled: unknown arrow key: -Stealth"), "{err}");
    assert!(err.contains("diagram 3 (line 3): not handled: arrow end is not a node"), "{err}");
    assert!(err.contains("diagram 4 (line 4): not handled: an undefined control sequence in a cell: \\Unheard"), "{err}");
    for (n, there) in [(1, true), (2, false), (3, false), (4, false), (5, true)] {
        assert_eq!(dir.join(format!("s-{n}.pdf")).exists(), there, "{n}");
        assert_eq!(dir.join(format!("s-{n}.svg")).exists(), there, "{n}");
    }
}

#[test]
fn many_in_one_file_are_what_each_is_alone() {
    if !tree() {
        return;
    }
    let dir = tmp("many");
    // Fonts that come and go, a colour, a preset that all share.
    let bodies = [
        r#"A \arrow[r,"f"] & \mathbb{B}"#,
        r#"\mathfrak{g} \arrow[d,red,"\alpha"'] \\ \textcolor{blue}{H}"#,
        r#"A \arrow[r,"f"] & \mathbb{B}"#,
        r#"\frac{a}{b} \arrow[r,bend left,dashed] & \sqrt{x} \arrow[l,opacity=0.5,"\xrightarrow{y}"]"#,
    ];
    let all: String = bodies.iter().map(|b| format!("\\begin{{tikzcd}}{b}\\end{{tikzcd}}\n")).collect();
    std::fs::write(dir.join("all.tex"), all).unwrap();
    assert!(commute().args(["--preset", "every arrow/.append style={thick}"]).arg(dir.join("all.tex")).status().unwrap().success());
    for (k, b) in bodies.iter().enumerate() {
        let name = format!("one{k}");
        std::fs::write(dir.join(format!("{name}.tex")), format!("\\begin{{tikzcd}}{b}\\end{{tikzcd}}\n")).unwrap();
        assert!(commute().args(["--preset", "every arrow/.append style={thick}"]).arg(dir.join(format!("{name}.tex"))).status().unwrap().success());
        let alone = std::fs::read(dir.join(format!("{name}.svg"))).unwrap();
        let many = std::fs::read(dir.join(format!("all-{}.svg", k + 1))).unwrap();
        assert!(alone == many, "diagram {k}: its SVG is another in a file of many");
        let (alone, many) = (pdf_normal(&dir.join(format!("{name}.pdf"))), pdf_normal(&dir.join(format!("all-{}.pdf", k + 1))));
        assert!(alone == many, "diagram {k}: its PDF is another in a file of many");
    }
    // And the same bytes on the next run.
    let first = std::fs::read(dir.join("one0.pdf")).unwrap();
    assert!(commute().args(["--preset", "every arrow/.append style={thick}"]).arg(dir.join("one0.tex")).status().unwrap().success());
    assert!(first == std::fs::read(dir.join("one0.pdf")).unwrap());
}

#[test]
fn a_preamble_and_a_package_and_a_whole_document() {
    if !tree() {
        return;
    }
    let dir = tmp("preamble");
    std::fs::write(dir.join("pre.tex"), "\\newcommand{\\Thing}{\\mathscr{T}}\n\\tikzcdset{arrow style=tikz}\n").unwrap();
    std::fs::write(dir.join("d.tex"), "\\Thing \\arrow[r] & B \\\\ C & D").unwrap();
    let out = commute().current_dir(&dir).args(["--usepackage", "mathrsfs", "--preamble", "pre.tex", "d.tex"]).output().unwrap();
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(dir.join("d.pdf").exists(), "{err}");
    // A tree that the library is verified on has it draw this.  On any other real PGF does: a PDF as right as ever, no SVG, the status that means that, and where to ask.
    let want = if armed() { Some(0) } else { Some(3) };
    assert_eq!(out.status.code(), want, "{err}");
    if want == Some(0) {
        assert!(dir.join("d.svg").exists(), "{err}");
    } else {
        assert!(!dir.join("d.svg").exists(), "{err}");
        assert!(err.contains("no SVG: real PGF drew it") && err.contains("--check-tree"), "{err}");
    }
    // A second run comes from the checkpoint, and writes the same.
    let first = std::fs::read(dir.join("d.pdf")).unwrap();
    std::fs::remove_file(dir.join("d.pdf")).unwrap();
    assert_eq!(commute().current_dir(&dir).args(["--usepackage", "mathrsfs", "--preamble", "pre.tex", "d.tex"]).status().unwrap().code(), want);
    assert!(first == std::fs::read(dir.join("d.pdf")).unwrap());

    std::fs::write(
        dir.join("doc.tex"),
        "\\documentclass[12pt]{article}\n\\usepackage{tikz-cd}\n\\newcommand{\\Mine}{M}\n\\begin{document}\ntext\n\\begin{tikzcd}\\Mine \\arrow[r] & B\\end{tikzcd}\nmore\n\\begin{tikzcd}\\Mine\\end{tikzcd}\n\\end{document}\n",
    )
    .unwrap();
    let out = commute().current_dir(&dir).arg("doc.tex").output().unwrap();
    assert_eq!(out.status.code(), want, "{}", String::from_utf8_lossy(&out.stderr));
    assert!(dir.join("doc-1.pdf").exists() && dir.join("doc-2.pdf").exists());
}

/// What a tree in no profile gets, on any tree: one macro of PGF's that is not as the table has it, and a preamble can do that as well as an update can.
#[test]
fn a_macro_not_as_verified_gives_a_pdf_and_says_why_there_is_no_svg() {
    if !tree() {
        return;
    }
    let dir = tmp("changed");
    std::fs::write(dir.join("pre.tex"), "\\makeatletter\\let\\orig@pgfscope\\pgfscope\\def\\pgfscope{\\orig@pgfscope}\\makeatother\n").unwrap();
    std::fs::write(dir.join("d.tex"), "A \\arrow[r] & B").unwrap();
    let out = commute().current_dir(&dir).args(["--preamble", "pre.tex", "d.tex"]).output().unwrap();
    let err = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(3), "{err}");
    assert!(dir.join("d.pdf").exists() && !dir.join("d.svg").exists(), "{err}");
    assert!(err.contains("no SVG: real PGF drew it") && err.contains("not as installed: pgfscope") && err.contains("--check-tree"), "{err}");
    // Who asked for the PDF has all of it.
    assert!(commute().current_dir(&dir).args(["--pdf", "--preamble", "pre.tex", "d.tex"]).status().unwrap().success());
}

/// xcolor keeps a colour defined in hsb as hsb, and PGF's `\pgf@setfillcolor` and its like stop on a model they have no operators for: real PGF's error, on any tree, and no files.
#[test]
fn a_colour_pgf_cannot_set_is_its_error() {
    if !tree() {
        return;
    }
    let dir = tmp("hsb");
    std::fs::write(dir.join("pre.tex"), "\\usepackage{xcolor}\n\\definecolor{cc}{hsb}{0.6,0.5,0.8}\n\\definecolor{ok}{rgb}{0.4,0.56,0.8}\n").unwrap();
    std::fs::write(dir.join("d.tex"), "\\begin{tikzcd} A \\arrow[d, \"g\" {fill=cc}] \\\\ B \\end{tikzcd}\n\\begin{tikzcd} A \\arrow[d, \"g\" {fill=ok}] \\\\ B \\end{tikzcd}\n").unwrap();
    let out = commute().current_dir(&dir).args(["--preamble", "pre.tex", "d.tex"]).output().unwrap();
    let err = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(1), "{err}");
    assert!(err.contains("diagram 1 (line 1): ! Package pgf Error: Unsupported color model `hsb'"), "{err}");
    assert!(!dir.join("d-1.pdf").exists() && !dir.join("d-1.svg").exists(), "{err}");
    assert!(dir.join("d-2.pdf").exists(), "{err}");
}

#[test]
fn the_tree_is_reported() {
    if !tree() {
        return;
    }
    let out = commute().arg("--check-tree").output().unwrap();
    let said = String::from_utf8_lossy(&out.stdout);
    for path in ["path=tikz-cd ", "path=pickup@font ", "path=checkpoint "] {
        assert!(said.lines().any(|l| l.starts_with(path)), "{said}");
    }
    assert!(said.contains("LaTeX "), "{said}");
    let off = said.lines().any(|l| l.starts_with("path=") && l.contains(" state=off"));
    assert_eq!(out.status.code(), Some(off as i32), "{said}");
}

/// With the library alone the page is what the PGF of this tree writes, whatever LaTeX stands beside it, as on a tree that is half updated: here a `pgf.revision.tex` that comes first on the path, on any tree.
#[test]
fn the_library_writes_lengths_as_the_pgf_beside_it_does() {
    if !tree() {
        return;
    }
    for (version, width) in [("3.1.11a", "0.3985 w"), ("3.1.12", "0.39851 w")] {
        let d = tmp(&format!("pgf-{version}"));
        std::fs::write(d.join("pgf.revision.tex"), format!("\\def\\pgfrevision{{{version}}}\n\\def\\pgfversion{{{version}}}\n\\def\\pgfrevisiondate{{2026-01-01}}\n\\def\\pgfversiondate{{2026-01-01}}\n")).unwrap();
        let input = d.join("a.tex");
        std::fs::write(&input, SQUARE).unwrap();
        let separator = if cfg!(windows) { ';' } else { ':' };
        let out = commute().current_dir(&d).env("XDG_CACHE_HOME", d.join("cache")).env("TEXINPUTS", format!("{}{separator}", d.display())).arg(&input).output().unwrap();
        assert_eq!(out.status.code(), Some(0), "{}", String::from_utf8_lossy(&out.stderr));
        let Some(stream) = qpdf_stream(&d.join("a-1.pdf")) else { return };
        assert!(stream.contains(width), "{version}: {stream}");
    }
}

#[test]
fn real_pgf_when_asked_for() {
    if !tree() {
        return;
    }
    let dir = tmp("realpgf");
    std::fs::write(dir.join("r.tex"), "A \\arrow[r,-Stealth] & B").unwrap();
    let out = commute().current_dir(&dir).arg("r.tex").output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(!dir.join("r.pdf").exists());
    let out = commute().current_dir(&dir).args(["--real-pgf", "r.tex"]).output().unwrap();
    assert!(dir.join("r.pdf").exists(), "{}", String::from_utf8_lossy(&out.stderr));
    // Real PGF draws no SVG here, and that is said.
    assert_eq!(out.status.code(), Some(3));
    assert!(String::from_utf8_lossy(&out.stderr).contains("no SVG"));
    assert!(commute().current_dir(&dir).args(["--real-pgf", "--pdf", "r.tex"]).status().unwrap().success());
}

/// What TeX cannot finish does not take the rest of the file with it.
#[test]
fn the_next_diagram_survives_the_one_before() {
    if !tree() {
        return;
    }
    let dir = tmp("survives");
    let input = dir.join("s.tex");
    // What only running shows: a `^` and a `_` in text, which TeX recovers from by a formula of its own.  A `\\write` at the shipout scans tokens in the middle of a step.  Vertical mode in a math cell, which would leave TeX where no later diagram could begin, and `\\frac` wanting what is not there, are said before anything runs.
    let src = "\\begin{tikzcd}\\texttt{x_y}\\end{tikzcd}\n\\begin{tikzcd}\\write18{x}\\end{tikzcd}\n\\begin{tikzcd}\\texttt{u^v}\\end{tikzcd}\n\\begin{tikzcd}A\\end{tikzcd}\n\\begin{tikzcd}\\vskip 1pt\\end{tikzcd}\n\\begin{tikzcd}\\frac{a}\\end{tikzcd}\n\\begin{tikzcd}B\\end{tikzcd}\n";
    std::fs::write(&input, src).unwrap();
    let out = commute().arg(&input).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("diagram 1 (line 1): ! "), "{err}");
    assert!(err.contains("diagram 3 (line 3): ! "), "{err}");
    assert!(err.contains("diagram 5 (line 5): not handled"), "{err}");
    assert!(err.contains("diagram 6 (line 6): not handled"), "{err}");
    assert!(!err.contains("not reached"), "{err}");
    for (n, there) in [(1, false), (2, true), (3, false), (4, true), (5, false), (6, false), (7, true)] {
        assert_eq!(dir.join(format!("s-{n}.pdf")).exists(), there, "{n}: {err}");
    }
}

/// Under a preamble real tikz-cd runs, and the library may still draw the diagram; an error TeX reports, in the preamble or in a cell, leaves no file of it, as with the library only.
#[test]
fn an_error_under_a_preamble_writes_nothing() {
    if !tree() {
        return;
    }
    for (name, preamble, body) in [("pre", "\\undefinedinthepreamble\n", "A \\arrow[r] & B"), ("cell", "\\newcommand{\\Mine}{M}\n", "\\text{a_b} \\arrow[r] & B")] {
        let dir = tmp(&format!("error-{name}"));
        std::fs::write(dir.join("p.tex"), preamble).unwrap();
        std::fs::write(dir.join("d.tex"), body).unwrap();
        let out = commute().current_dir(&dir).args(["--preamble", "p.tex", "d.tex"]).output().unwrap();
        let err = String::from_utf8_lossy(&out.stderr);
        assert_eq!(out.status.code(), Some(1), "{name}: {err}");
        assert!(err.contains("diagram 1 (line 1): ! "), "{name}: {err}");
        assert!(!dir.join("d.pdf").exists(), "{name}: {err}");
        assert!(!dir.join("d.svg").exists(), "{name}: {err}");
    }
}

#[test]
fn opacity_gets_its_resources() {
    if !tree() {
        return;
    }
    let dir = tmp("opacity");
    let input = dir.join("o.tex");
    std::fs::write(&input, r#"A \arrow[r,opacity=0.5,"f"] & B"#).unwrap();
    assert!(commute().arg(&input).status().unwrap().success());
    let pdf = String::from_utf8_lossy(&std::fs::read(dir.join("o.pdf")).unwrap()).into_owned();
    assert!(pdf.contains("/pgf@CA0.5 << /CA 0.5 >>"), "{}", &pdf[..300]);
    assert!(pdf.contains("/pgf@ca0.5 << /ca 0.5 >>"));
    let svg = std::fs::read_to_string(dir.join("o.svg")).unwrap();
    assert!(svg.contains("opacity=\"0.5\""));
}

#[test]
fn a_file_that_cannot_be_written_is_an_error() {
    if !tree() {
        return;
    }
    let dir = tmp("unwritable");
    let input = dir.join("u.tex");
    std::fs::write(&input, "A").unwrap();
    std::fs::create_dir(dir.join("u.svg")).unwrap();
    let out = commute().arg(&input).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("cannot write"));
}

#[test]
fn errors_have_their_status() {
    let dir = tmp("errors");
    let code = |args: &[&str]| commute().args(args).output().unwrap().status.code();
    assert_eq!(code(&[]), Some(2));
    assert_eq!(code(&["--nonsense", "x"]), Some(2));
    assert_eq!(code(&["a", "b"]), Some(2));
    assert_eq!(code(&["--pdf", "--svg", "x"]), Some(2));
    assert_eq!(code(&["-o"]), Some(2));
    assert_eq!(code(&[dir.join("missing.tex").to_str().unwrap()]), Some(2));
    assert_eq!(code(&["--version"]), Some(0));
    assert_eq!(code(&["--help"]), Some(0));
}

/// The file passes qpdf if there is one.
#[test]
fn qpdf_where_available() {
    if !tree() {
        return;
    }
    let dir = tmp("qpdf");
    let input = dir.join("q.tex");
    std::fs::write(&input, SQUARE).unwrap();
    assert!(commute().arg(&input).status().unwrap().success());
    match Command::new("qpdf").arg("--check").arg(dir.join("q.pdf")).output() {
        Ok(o) => assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stdout)),
        Err(_) => println!("qpdf is not installed; not checked"),
    }
}
