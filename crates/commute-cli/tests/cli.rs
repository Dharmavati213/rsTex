//! The commute command, as a user runs it.  Files go under cargo's temporary directory for tests.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const SQUARE: &str = "x\n\\begin{tikzcd}\nA \\arrow[r,\"f\"] \\arrow[d,\"g\"'] & B \\arrow[d,\"h\"] \\\\\nC \\arrow[r,\"k\"'] & D\n\\end{tikzcd}\ny\n";

fn tmp(name: &str) -> PathBuf {
    let d = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn commute() -> Command {
    Command::new(env!("CARGO_BIN_EXE_commute"))
}

/// Offsets in the xref table point at the objects, and the content stream's q and Q balance.
fn check_pdf(bytes: &[u8]) -> String {
    assert!(bytes.starts_with(b"%PDF-"));
    assert!(bytes.ends_with(b"%%EOF\n"));
    let text = String::from_utf8_lossy(bytes).into_owned();
    let at = text.rfind("startxref\n").unwrap() + "startxref\n".len();
    let xref: usize = text[at..].lines().next().unwrap().parse().unwrap();
    assert!(text[xref..].starts_with("xref\n"));
    let mut lines = text[xref..].lines().skip(1);
    let count: usize = lines.next().unwrap().split(' ').nth(1).unwrap().parse().unwrap();
    lines.next();
    for i in 1..count {
        let off: usize = lines.next().unwrap()[..10].parse().unwrap();
        assert!(text[off..].starts_with(&format!("{i} 0 obj")), "object {i} at {off}");
    }
    let s = text.find("stream\n").unwrap() + "stream\n".len();
    let e = text.find("\nendstream").unwrap();
    let stream = &text[s..e];
    let mut depth = 0i32;
    for t in stream.split_whitespace() {
        match t {
            "q" => depth += 1,
            "Q" => depth -= 1,
            _ => {}
        }
        assert!(depth >= 0);
    }
    assert_eq!(depth, 0);
    stream.to_string()
}

fn media_box(pdf: &str) -> (f64, f64) {
    let at = pdf.find("/MediaBox [0 0 ").unwrap() + "/MediaBox [0 0 ".len();
    let rest = &pdf[at..pdf[at..].find(']').unwrap() + at];
    let mut it = rest.split_whitespace().map(|x| x.parse::<f64>().unwrap());
    (it.next().unwrap(), it.next().unwrap())
}

#[test]
fn writes_a_cropped_pdf_and_svg() {
    let dir = tmp("square");
    let input = dir.join("square.tex");
    std::fs::write(&input, SQUARE).unwrap();
    let st = commute().arg(&input).status().unwrap();
    assert!(st.success());

    let pdf = std::fs::read(dir.join("square.pdf")).unwrap();
    let stream = check_pdf(&pdf);
    assert!(stream.contains(" m ") && stream.contains(" cm ") && stream.contains(" Tj "));
    assert_eq!(stream.matches(" Tj ").count(), 8, "four cells and four labels");
    let (w, h) = media_box(&String::from_utf8_lossy(&pdf));
    assert!(w > 40.0 && w < 90.0 && h > 30.0 && h < 70.0, "{w} {h}");

    let svg = std::fs::read_to_string(dir.join("square.svg")).unwrap();
    assert!(svg.starts_with("<svg ") && svg.ends_with("</svg>\n"));
    assert_eq!(svg.matches("<path").count(), 8, "four lines and four tips");
    assert_eq!(svg.matches("<text").count(), 8);
    assert_eq!(svg.matches("<g ").count(), svg.matches("</g>").count());
}

#[test]
fn a_body_works_and_stdin_too() {
    let dir = tmp("stdin");
    let mut child = commute()
        .args(["-", "-o"])
        .arg(&dir)
        .args(["--stem", "d", "--svg"])
        .stdin(Stdio::piped())
        .spawn()
        .unwrap();
    use std::io::Write;
    child.stdin.as_mut().unwrap().write_all(br"[sep=large] \alpha \arrow[r,hook] & \mathrm{Hom}").unwrap();
    assert!(child.wait().unwrap().success());
    assert!(dir.join("d.svg").exists());
    assert!(!dir.join("d.pdf").exists());
    let svg = std::fs::read_to_string(dir.join("d.svg")).unwrap();
    assert!(svg.contains('α'));
}

#[test]
fn several_diagrams_are_numbered_and_unsupported_ones_named() {
    let dir = tmp("several");
    let input = dir.join("s.tex");
    let src = "\\begin{tikzcd} A \\arrow[r] & B \\end{tikzcd}\n\\begin{tikzcd} A \\arrow[r,-Stealth] & B \\end{tikzcd}\n\\begin{tikzcd}[baseline=1ex] A \\arrow[to=3-3] \\end{tikzcd}\n\\begin{tikzcd} A \\arrow[r,Rightarrow,\"f\"] & B \\end{tikzcd}\n";
    std::fs::write(&input, src).unwrap();
    let out = commute().arg(&input).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("diagram 2: not handled: unknown arrow key: -Stealth"), "{err}");
    assert!(err.contains("diagram 3: not handled: arrow end is not a node"), "{err}");
    for (n, there) in [(1, true), (2, false), (3, false), (4, true)] {
        assert_eq!(dir.join(format!("s-{n}.pdf")).exists(), there, "{n}");
        assert_eq!(dir.join(format!("s-{n}.svg")).exists(), there, "{n}");
    }
    check_pdf(&std::fs::read(dir.join("s-4.pdf")).unwrap());
}

#[test]
fn opacity_gets_its_resources() {
    let dir = tmp("opacity");
    let input = dir.join("o.tex");
    std::fs::write(&input, r#"A \arrow[r,opacity=0.5,"f"] & B"#).unwrap();
    assert!(commute().arg(&input).status().unwrap().success());
    let pdf = String::from_utf8_lossy(&std::fs::read(dir.join("o.pdf")).unwrap()).into_owned();
    assert!(pdf.contains("/pgf@CA0.5 gs /pgf@ca0.5 gs"));
    assert!(pdf.contains("/pgf@CA0.5 << /CA 0.5 >>"));
    assert!(pdf.contains("/pgf@ca0.5 << /ca 0.5 >>"));
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
    let dir = tmp("qpdf");
    let input = dir.join("q.tex");
    std::fs::write(&input, SQUARE).unwrap();
    assert!(commute().arg(&input).status().unwrap().success());
    match Command::new("qpdf").arg("--check").arg(dir.join("q.pdf")).output() {
        Ok(o) => assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stdout)),
        Err(_) => println!("qpdf is not installed; not checked"),
    }
}
