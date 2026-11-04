//! What the engine starts from.  Without a preamble, commute's own format: LaTeX as `standalone` has it, the math packages, and what is needed so that the library alone can do a diagram.  It is made once, on
//! `pdflatex.fmt`, and it stops after `\begin{document}`, so that a run has only to load it.  With a preamble, a file in the cache directory for `rstex_engine::session` to end, that a checkpoint takes care of.
use rstex_kpse as kpse;
use std::io::IsTerminal;
use std::path::PathBuf;

/// The format's source.  Its hash is in the format's name.  `\arrow` and the others only have to exist: `native_read` looks names up and makes none, and the library reads them as bytes.  `\pgf@layerlist` and
/// `\ifpgfpicture` are read by `native_checks`.  xcolor and graphicx are what `\usepackage{tikz-cd}` brings along.  The rest keeps a PDF the same from one run to the next, and the fonts that every diagram uses in place.
const DRIVER: &str = r"\documentclass{article}
\usepackage{amsmath,amssymb}
\usepackage{graphicx,xcolor}
\nofiles
\def\tikzcd{}
\def\endtikzcd{}
\let\arrow\relax \let\ar\relax \let\rar\relax \let\lar\relax \let\dar\relax \let\uar\relax
\let\urar\relax \let\ular\relax \let\drar\relax \let\dlar\relax \let\tikzcdmatrixname\relax
\newbox\commutebox
\makeatletter
\def\pgf@layerlist{main}
\newif\ifpgfpicture
\makeatother
\pdfinfoomitdate=1
\pdfsuppressptexinfo=-1
\pdfgentounicode=0
\pdfobjcompresslevel=0
\begin{document}
{\sbox0{$\relax$}}%
\everyjob{}%
\dump
";

/// The format, built now if need be, or a message.
pub fn library_format() -> Result<Vec<u8>, String> {
    let name = format!("commute-{:016x}", kpse::hash64(DRIVER.as_bytes()));
    let note = || {
        if std::io::stderr().is_terminal() {
            eprintln!("commute: building its format, once; this takes some seconds");
        }
    };
    rstex_driver::autofmt::derived(name.as_bytes(), b"pdflatex", DRIVER.as_bytes(), &note).ok_or_else(|| "cannot build a format: is a TeX Live tree installed?".to_string())
}

/// What a document with more than the format has starts from.
pub struct Driver {
    pub file: Vec<u8>,
    pub dir: Vec<u8>,
}

/// A driver in the cache directory.  `document` is the whole preamble of a file that has one, class and all: its packages may come with options of their own, so what a diagram needs comes after it, where a package that is
/// there already does no harm.  `preamble_file` is `--preamble`, which may count on tikz-cd and the math packages, so they come before it, unless `preamble_text` loads one itself, perhaps with options.
pub fn driver(document: Option<&[u8]>, usepackage: &[String], preamble_file: Option<&[u8]>, preamble_text: &[u8], preset: &[u8]) -> Result<Driver, String> {
    let mut text: Vec<u8> = Vec::with_capacity(document.map_or(0, <[u8]>::len) + 512);
    let names = |t: &[u8], name: &[u8]| t.windows(name.len()).any(|w| w == name);
    let what_a_diagram_needs = b"\\usepackage{amsmath,amssymb}\n\\usepackage{tikz-cd}\n\\usetikzlibrary{decorations.pathmorphing}\n";
    match document {
        Some(d) => {
            text.extend_from_slice(d);
            text.push(b'\n');
            text.extend_from_slice(what_a_diagram_needs);
        }
        None => {
            text.extend_from_slice(b"\\documentclass{article}\n");
            if !names(preamble_text, b"amsmath") {
                text.extend_from_slice(b"\\usepackage{amsmath}\n");
            }
            if !names(preamble_text, b"amssymb") {
                text.extend_from_slice(b"\\usepackage{amssymb}\n");
            }
            text.extend_from_slice(b"\\usepackage{tikz-cd}\n\\usetikzlibrary{decorations.pathmorphing}\n");
        }
    }
    for p in usepackage {
        text.extend_from_slice(b"\\usepackage");
        if !p.starts_with('[') {
            text.push(b'{');
        }
        text.extend_from_slice(p.as_bytes());
        if !p.starts_with('[') {
            text.push(b'}');
        }
        text.push(b'\n');
    }
    if let Some(f) = preamble_file {
        text.extend_from_slice(b"\\input{");
        text.extend_from_slice(f);
        text.extend_from_slice(b"}\n");
    }
    if !preset.is_empty() {
        text.extend_from_slice(b"\\tikzcdset{");
        text.extend_from_slice(preset);
        text.extend_from_slice(b"}\n");
    }
    text.extend_from_slice(b"\\nofiles\n\\newbox\\commutebox\n\\pdfinfoomitdate=1\n\\pdfsuppressptexinfo=-1\n\\begin{document}\\relax\n\\commutesession\n\\end{document}\n");
    let dir: PathBuf = kpse::cache_dir().ok_or("no cache directory")?.join("commute").join(format!("{:016x}", kpse::hash64(&text)));
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let file = dir.join("driver.tex");
    if std::fs::read(&file).ok().as_deref() != Some(&text[..]) {
        let tmp = dir.join(format!("driver.tmp-{}", std::process::id()));
        std::fs::write(&tmp, &text).map_err(|e| format!("{}: {e}", tmp.display()))?;
        std::fs::rename(&tmp, &file).map_err(|e| format!("{}: {e}", file.display()))?;
    }
    Ok(Driver { file: rstex_engine::platform::path_to_bytes(&file), dir: rstex_engine::platform::path_to_bytes(&dir) })
}
