//! Line ends, on every system, through the real program: DESIGN.md section 12, "Text".  web2c's `input_line` stops at LF, CR or CRLF wherever it runs, c-fopen.h's `FOPEN_W_MODE` is `"wb"`, so that a log and a
//! `\write` file have LF on Windows too, and the same source gives the same bytes under any of them.  None of this needs a TeX tree: `-ini` loads no format.
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

/// DESIGN.md section 12: the environment variable that puts the cache directory under some directory.
const CACHE_VAR: &str = if cfg!(windows) {
    "LOCALAPPDATA"
} else if cfg!(target_os = "macos") {
    "HOME"
} else {
    "XDG_CACHE_HOME"
};

fn scratch(name: &str) -> PathBuf {
    let d = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("line-endings-{name}"));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn run(work: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rstex"))
        .args(["-ini", "-no-snapshot", "-interaction=nonstopmode", "doc.tex"])
        .current_dir(work)
        .env(CACHE_VAR, work.join("cache"))
        .env("SOURCE_DATE_EPOCH", "1700000000")
        .env("FORCE_SOURCE_DATE", "1")
        .stdin(Stdio::null())
        .output()
        .unwrap()
}

/// A document that reads a line and writes a file, with `eol` at each line's end.  A line with blanks at its end and one with none at all count too.
fn document(eol: &str) -> String {
    [
        "\\catcode`\\{=1 \\catcode`\\}=2 \\catcode`\\^=7 \\catcode`\\#=6",
        "\\pdfoutput=1 \\pdfinfoomitdate=1 \\pdftrailerid{}",
        "\\pdfpagewidth=100pt \\pdfpageheight=100pt",
        "\\immediate\\openout3=side.out",
        "\\immediate\\write3{a line}",
        "\\newlinechar=`\\^^J \\immediate\\write3{two^^Jlines}",
        "\\immediate\\closeout3",
        "\\openin4=read.in \\endlinechar=-1",
        "\\read4 to \\x \\message{[\\x]}",
        "\\read4 to \\x \\message{[\\x]}",
        "\\closein4 \\endlinechar=13",
        "\\message{[blanks follow]}    ",
        "\\shipout\\hbox{\\vrule width 10pt height 10pt}",
        "\\end",
    ]
    .join(eol)
}

#[test]
fn any_line_end_gives_the_same_files_and_they_have_lf() {
    let mut results = Vec::new();
    for (name, eol) in [("lf", "\n"), ("crlf", "\r\n"), ("cr", "\r")] {
        let d = scratch(name);
        std::fs::write(d.join("doc.tex"), document(eol)).unwrap();
        std::fs::write(d.join("read.in"), ["first  ", "second", ""].join(eol)).unwrap();
        let o = run(&d);
        let text = String::from_utf8_lossy(&o.stdout).to_string();
        assert_eq!(o.status.code(), Some(0), "{name}: {text}");
        assert!(text.contains("[first] [second] [blanks follow]"), "{name}: {text}");
        let (log, side, pdf) = (std::fs::read(d.join("doc.log")).unwrap(), std::fs::read(d.join("side.out")).unwrap(), std::fs::read(d.join("doc.pdf")).unwrap());
        assert_eq!(side, b"a line\ntwo\nlines\n", "{name}: a \\write file has LF");
        assert!(!log.contains(&b'\r'), "{name}: the log has LF");
        assert!(!o.stdout.contains(&b'\r'), "{name}: DESIGN.md's Terminal row: the bytes, when redirected");
        results.push((log, side, pdf));
    }
    assert!(results[0] == results[1], "LF and CRLF differ");
    assert!(results[0] == results[2], "LF and CR differ");
}

/// `input_line` passes over a byte order mark under `#ifdef WIN32`, and elsewhere it is three characters of the document, which here start a paragraph.  So pdfTeX's answer depends on the system, and this says
/// what each has: the one place a source file means another thing on another system, DESIGN.md section 12.
#[test]
fn a_byte_order_mark_is_for_windows_alone() {
    let d = scratch("mark");
    let source = "\u{feff}\\catcode`\\{=1 \\catcode`\\}=2 \\message{\\ifhmode[in a paragraph]\\else[no paragraph]\\fi}\\end\n";
    std::fs::write(d.join("doc.tex"), source).unwrap();
    let o = run(&d);
    let text = String::from_utf8_lossy(&o.stdout).to_string();
    let want = if rstex_engine::platform::DOSISH { "[no paragraph]" } else { "[in a paragraph]" };
    assert!(text.contains(want), "{text}");
}

/// What `.gitattributes` is for: a checkout that changed the bytes shows here first, by name, before the tests that compare with an oracle's table all fail at once.  This holds on all three systems.
#[test]
fn a_checkout_has_the_bytes_of_the_repository() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().parent().unwrap();
    let read = |rel: &str| std::fs::read(root.join(rel)).unwrap();
    let crlf = read("tests/integration/files/crlf-all.tex");
    assert!(crlf.windows(2).any(|w| w == b"\r\n"), "CRLF is what this case is about");
    assert!(!crlf.iter().enumerate().any(|(i, &c)| c == b'\n' && (i == 0 || crlf[i - 1] != b'\r')), "all of it");
    let cr = read("tests/integration/files/cr-only.tex");
    assert!(cr.contains(&b'\r') && !cr.contains(&b'\n'), "CR alone is what this case is about");
    assert!(!read("tests/cross_os/plain.tex").contains(&b'\r'));
    // Every table of what an oracle said, and every other file made for a test of a crate.
    let mut seen = 0;
    let crates = std::fs::read_dir(root.join("crates")).unwrap();
    for data in crates.flatten().map(|c| c.path().join("testdata")).filter(|d| d.is_dir()) {
        for f in std::fs::read_dir(&data).unwrap().flatten().map(|e| e.path()).filter(|p| p.is_file()) {
            assert!(!std::fs::read(&f).unwrap().windows(2).any(|w| w == b"\r\n"), "{}", f.display());
            seen += 1;
        }
    }
    assert!(seen > 40, "{seen}");
}
