//! The `commute` command (DESIGN.md section 10): tightly cropped PDF and SVG of tikz-cd diagrams.
//!
//!     commute [options] FILE
//!
//! FILE holds `\begin{tikzcd}...\end{tikzcd}` environments, or just a diagram's body; `-` is standard input.  Each diagram
//! becomes `<stem>.pdf` and `<stem>.svg`, or `<stem>-<n>.*` if there are several.
//!
//! What is in the boxes is a placeholder until the engine is ready; see typeset.rs.
#![forbid(unsafe_code)]

mod output;
mod tfm;
mod typeset;

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use commute::{BoxDim, Diagram, LabelFont, StandardLatex};

use typeset::{Builtin, Request, Style, TypesetBox, Typesetter};

const USAGE: &str = "usage: commute [options] FILE
  FILE holds \\begin{tikzcd}...\\end{tikzcd} environments, or just a diagram's body; - is standard input
  -o DIR        write into DIR instead of next to FILE (the current directory for standard input)
  --stem NAME   base name of the output files, instead of FILE's
  --pdf         only PDF
  --svg         only SVG
  --preset KEYS keys as in \\tikzcdset{KEYS}, before every diagram
  --version
exit status: 0 all written; 1 some diagrams are outside what commute handles (they need real PGF); 2 usage or I/O error";

struct Args {
    file: String,
    out: Option<PathBuf>,
    stem: Option<String>,
    pdf: bool,
    svg: bool,
    preset: String,
}

fn parse_args() -> Result<Option<Args>, String> {
    let mut a = Args { file: String::new(), out: None, stem: None, pdf: true, svg: true, preset: String::new() };
    let mut it = std::env::args().skip(1);
    let mut have_file = false;
    while let Some(s) = it.next() {
        let mut value = |what: &str| it.next().ok_or_else(|| format!("{what} needs a value"));
        match s.as_str() {
            "-o" => a.out = Some(PathBuf::from(value("-o")?)),
            "--stem" => a.stem = Some(value("--stem")?),
            "--preset" => a.preset = value("--preset")?,
            "--pdf" => a.svg = false,
            "--svg" => a.pdf = false,
            "--version" => {
                println!("commute {}", commute::VERSION);
                return Ok(None);
            }
            "-h" | "--help" => {
                println!("{USAGE}");
                return Ok(None);
            }
            "-" => {
                a.file = s;
                have_file = true;
            }
            _ if s.starts_with('-') => return Err(format!("unknown option {s}")),
            _ => {
                if have_file {
                    return Err("more than one file".to_string());
                }
                a.file = s;
                have_file = true;
            }
        }
    }
    if !have_file {
        return Err("no file".to_string());
    }
    if !a.pdf && !a.svg {
        return Err("--pdf and --svg together write nothing".to_string());
    }
    Ok(Some(a))
}

fn label_font(d: &Diagram, f: LabelFont) -> &[u8] {
    match f {
        LabelFont::Command(s) => d.text(s),
        _ => b"",
    }
}

/// Typeset a parsed diagram's cells and labels.
fn typeset_all<T: Typesetter>(d: &Diagram, t: &mut T) -> (Vec<TypesetBox>, Vec<TypesetBox>) {
    let math = d.math_mode();
    let mut cells = Vec::with_capacity(d.cells().len());
    for (i, c) in d.cells().iter().enumerate() {
        if !c.has_node {
            cells.push(TypesetBox::default());
            continue;
        }
        let runs: Vec<&[u8]> = d.cell_content(i).collect();
        cells.push(t.typeset(&Request { runs: &runs, math, style: Style::Cell(d.text(c.font())) }));
    }
    let mut labels = Vec::with_capacity(d.labels().len());
    for l in d.labels() {
        let text = d.text(l.text);
        labels.push(t.typeset(&Request { runs: &[text], math, style: Style::Label(l.font, label_font(d, l.font)) }));
    }
    (cells, labels)
}

fn dims(b: &[TypesetBox]) -> Vec<BoxDim> {
    b.iter().map(|x| x.dim).collect()
}

fn write_file(path: &Path, bytes: &[u8]) -> Result<(), String> {
    std::fs::write(path, bytes).map_err(|e| format!("{}: {e}", path.display()))
}

fn run(a: &Args) -> Result<bool, String> {
    let src = if a.file == "-" {
        let mut v = Vec::new();
        std::io::stdin().read_to_end(&mut v).map_err(|e| format!("standard input: {e}"))?;
        v
    } else {
        std::fs::read(&a.file).map_err(|e| format!("{}: {e}", a.file))?
    };

    let input = Path::new(&a.file);
    let stem = match &a.stem {
        Some(s) => s.clone(),
        None if a.file == "-" => "diagram".to_string(),
        None => input.file_stem().map_or("diagram".to_string(), |s| s.to_string_lossy().into_owned()),
    };
    let dir = match &a.out {
        Some(d) => d.clone(),
        None if a.file == "-" => PathBuf::from("."),
        None => input.parent().map_or(PathBuf::from("."), Path::to_path_buf),
    };
    if a.out.is_some() {
        std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }

    // The environments in the file, or the whole file as one body.
    let mut ranges = Vec::new();
    let mut from = 0;
    while let Some((s, e)) = commute::find_environment(&src, from) {
        ranges.push((s, e));
        from = commute::after_environment(e);
    }
    if ranges.is_empty() {
        ranges.push((0, src.len()));
    }

    let mut t = Builtin::new();
    if !t.has_metrics {
        eprintln!("commute: Computer Modern TFM files not found, widths are guesses; set COMMUTE_TFM_DIR");
    }
    let font = t.font_params();
    let mut d = Diagram::new();
    let mut all_ok = true;
    for (n, &(s, e)) in ranges.iter().enumerate() {
        let name = if ranges.len() == 1 { stem.clone() } else { format!("{stem}-{}", n + 1) };
        let report = |d: &Diagram, u: commute::Unsupported| {
            eprintln!("commute: diagram {}: not handled: {}: {}", n + 1, u.reason, String::from_utf8_lossy(d.text(u.at)));
        };
        if let Err(u) = d.parse(a.preset.as_bytes(), &src[s..e], font, &StandardLatex) {
            report(&d, u);
            all_ok = false;
            continue;
        }
        let (cells, labels) = typeset_all(&d, &mut t);
        if let Err(u) = d.layout(&dims(&cells), &dims(&labels)) {
            report(&d, u);
            all_ok = false;
            continue;
        }
        if a.pdf {
            write_file(&dir.join(format!("{name}.pdf")), &output::pdf_bytes(&d, &cells, &labels))?;
        }
        if a.svg {
            write_file(&dir.join(format!("{name}.svg")), &output::svg_bytes(&d, &cells, &labels))?;
        }
    }
    Ok(all_ok)
}

fn main() -> ExitCode {
    let args = match parse_args() {
        Ok(Some(a)) => a,
        Ok(None) => return ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("commute: {e}\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    match run(&args) {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::from(1),
        Err(e) => {
            eprintln!("commute: {e}");
            ExitCode::from(2)
        }
    }
}
