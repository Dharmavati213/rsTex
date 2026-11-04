//! The `commute` command (DESIGN.md section 10): tightly cropped PDF and SVG of tikz-cd diagrams.
//!
//!     commute [options] FILE
//!
//! FILE holds `\begin{tikzcd}...\end{tikzcd}` environments, or just a diagram's body, or a whole document; `-` is standard input.  Each diagram becomes `<stem>.pdf` and `<stem>.svg`, or `<stem>-<n>.*` if there
//! are several.  The cells and labels are real TeX, by the embedded engine, and `crates/engine/src/session.rs` is what this asks of it.  What a run starts from is in `format.rs`.  docs/COMMUTE.md is for the user.
//!
//! `main` is the C runtime's, for the same reason as `rstex`'s.  `--engine` before anything else is what a format's build runs: this program is then `rstex`, so that it needs no other to be installed.
#![cfg_attr(not(test), no_main)]
#![deny(unsafe_code)]
#![cfg_attr(test, allow(dead_code))]

mod format;
mod input;

use rstex_engine::platform;
use rstex_engine::session::{Config, Job, Outcome};
use std::io::Read;
use std::path::{Path, PathBuf};

const USAGE: &str = "usage: commute [options] FILE
  FILE holds \\begin{tikzcd}...\\end{tikzcd} environments, a diagram's body, or a whole document; - is standard input
  -o DIR           write into DIR instead of next to FILE (the current directory for standard input)
  --stem NAME      base name of the output files, instead of FILE's
  --pdf            only PDF
  --svg            only SVG
  --preset KEYS    keys as in \\tikzcdset{KEYS}, before every diagram
  --usepackage P   also load package P, as P or [options]{P}; may be given again
  --preamble FILE  also read FILE as part of the preamble
  --real-pgf       let real PGF draw what commute itself declines (PDF only, and slow)
  --box            print each diagram's width, height and depth
  --version
exit status: 0 all written; 1 some diagrams were not; 2 usage or I/O error";

const STATUS_SOME: i32 = 1;
const STATUS_USAGE: i32 = 2;

#[derive(Clone, Default)]
struct Args {
    file: String,
    out: Option<PathBuf>,
    stem: Option<String>,
    pdf: bool,
    svg: bool,
    preset: String,
    usepackage: Vec<String>,
    preamble: Option<String>,
    real_pgf: bool,
    print_box: bool,
    /// One diagram under real tikz-cd, by number from 1, for a run of this program's own.
    only: Option<usize>,
    /// With `only`: real tikz-cd without a preamble, for what the library declined.
    force_real: bool,
    /// Only from this diagram on, with the library: what a run that TeX stopped did not reach.
    from: Option<usize>,
}

fn parse_args(argv: &[String]) -> Result<Option<Args>, String> {
    let mut a = Args { pdf: true, svg: true, ..Args::default() };
    let mut it = argv.iter();
    let mut have_file = false;
    while let Some(s) = it.next() {
        let mut value = |what: &str| it.next().cloned().ok_or_else(|| format!("{what} needs a value"));
        match s.as_str() {
            "-o" => a.out = Some(PathBuf::from(value("-o")?)),
            "--stem" => a.stem = Some(value("--stem")?),
            "--preset" => a.preset = value("--preset")?,
            "--usepackage" => a.usepackage.push(value("--usepackage")?),
            "--preamble" => a.preamble = Some(value("--preamble")?),
            "--real-pgf" => a.real_pgf = true,
            "--box" => a.print_box = true,
            "--only" => a.only = Some(value("--only")?.parse().map_err(|_| "--only needs a number")?),
            "--force-real" => a.force_real = true,
            "--from" => a.from = Some(value("--from")?.parse().map_err(|_| "--from needs a number")?),
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
                a.file = s.clone();
                have_file = true;
            }
            _ if s.starts_with('-') => return Err(format!("unknown option {s}")),
            _ => {
                if have_file {
                    return Err("more than one file".to_string());
                }
                a.file = s.clone();
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

fn bytes(p: &Path) -> Vec<u8> {
    platform::path_to_bytes(p)
}

/// The stems of the output files.  The directory goes in front of a job's name.
fn job_names(a: &Args, count: usize) -> (PathBuf, String, bool) {
    let input = Path::new(&a.file);
    let stem = match &a.stem {
        Some(s) => s.clone(),
        None if a.file == "-" => "diagram".to_string(),
        None => input.file_stem().map_or("diagram".to_string(), |s| s.to_string_lossy().into_owned()),
    };
    let dir = match &a.out {
        Some(d) => d.clone(),
        None if a.file == "-" => PathBuf::from("."),
        None => match input.parent() {
            Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
            _ => PathBuf::from("."),
        },
    };
    (dir, stem, count > 1)
}

/// Says what became of each, and returns the status.
fn report(jobs: &[Job], outcomes: &[Outcome], numbers: &[usize]) -> i32 {
    let mut code = 0;
    for (k, _) in jobs.iter().enumerate() {
        let n = numbers.get(k).copied().unwrap_or(k + 1);
        let line = jobs[k].line;
        match outcomes.get(k).unwrap_or(&Outcome::Pending) {
            Outcome::Written => {}
            Outcome::Pending => {
                eprintln!("commute: diagram {n} (line {line}): not reached");
                code = code.max(STATUS_SOME);
            }
            Outcome::Declined(why) => {
                eprintln!("commute: diagram {n} (line {line}): not handled: {why}");
                code = code.max(STATUS_SOME);
            }
            Outcome::Failed(why) => {
                eprintln!("commute: diagram {n} (line {line}): {why}");
                code = code.max(STATUS_SOME);
            }
            Outcome::NoSvg(why) => {
                eprintln!("commute: diagram {n} (line {line}): no SVG: {why}");
                code = code.max(STATUS_SOME);
            }
            Outcome::Unwritable(path) => {
                eprintln!("commute: diagram {n} (line {line}): cannot write {path}");
                code = STATUS_USAGE;
            }
        }
    }
    code
}

/// `kpse_set_program_name`, as `maininit` does it again later: the format's search asks the tree before that.
fn kpse_init() {
    rstex_kpse::with_global(|k| {
        k.set_invocation_name(b"commute");
        k.set_program_name(b"pdflatex");
    });
}

fn arg(s: &str) -> Vec<u8> {
    s.as_bytes().to_vec()
}

/// All diagrams of a file with one engine, the library only.  What it declines, real PGF gets if that was asked for.  If TeX stopped so that nothing after a diagram could be trusted, another run takes it from there.
fn run_library(a: &Args, jobs: Vec<Job>, numbers: Vec<usize>, rerun: Vec<String>, stdin_copy: Option<PathBuf>) -> Result<i32, String> {
    let fmt = format::library_format()?;
    // Everything is in the tree: not worth a look through the current directory, however long it is, for each font that is not there by that name.
    rstex_kpse::set_var(b"texmf_casefold_search", b"0");
    let real_pgf = a.real_pgf;
    let finish = Box::new(move |jobs: &[Job], outcomes: &[Outcome]| -> i32 {
        let mut code = 0;
        let mut real = Vec::new();
        let mut rest = None;
        for (k, o) in outcomes.iter().enumerate() {
            match o {
                Outcome::Pending => {
                    rest = Some(numbers[k]);
                    break;
                }
                Outcome::Declined(_) if real_pgf => real.push(numbers[k]),
                _ => code = code.max(report(&jobs[k..=k], &outcomes[k..=k], &numbers[k..=k])),
            }
        }
        for n in real {
            code = code.max(spawn_one(&rerun, &["--only", &n.to_string(), "--force-real"]));
        }
        if let Some(n) = rest {
            code = code.max(spawn_one(&rerun, &["--from", &n.to_string()]));
        }
        if let Some(c) = &stdin_copy {
            let _ = std::fs::remove_file(c);
        }
        code
    });
    let config = Config { jobs, pdf: a.pdf, svg: a.svg, print_box: a.print_box, library_only: true, preset: a.preset.clone().into_bytes(), finish: Some(finish) };
    let argv = vec![arg("commute"), [b"-fmt=", &fmt[..]].concat(), arg("-progname=pdflatex"), arg("-interaction=nonstopmode"), arg("-no-shell-escape"), arg("-no-snapshot"), arg("\\commutesession")];
    Ok(rstex_driver::run(argv, Some(config)))
}

/// Some of this file in a run of its own.
fn spawn_one(rerun: &[String], more: &[&str]) -> i32 {
    let Ok(exe) = std::env::current_exe() else { return STATUS_SOME };
    let mut c = std::process::Command::new(exe);
    c.args(rerun).args(more);
    match c.status() {
        Ok(s) => s.code().unwrap_or(STATUS_SOME),
        Err(e) => {
            eprintln!("commute: cannot start another run: {e}");
            STATUS_SOME
        }
    }
}

/// One diagram under real tikz-cd, with a checkpoint for what comes before it.
fn run_real(a: &Args, job: Job, number: usize, document: Option<&[u8]>) -> Result<i32, String> {
    let mut preamble_text = Vec::new();
    let preamble_file = match &a.preamble {
        Some(p) => {
            // What `\\input` is given: with forward slashes on every system.
            let abs = std::path::absolute(p).map_err(|e| format!("{p}: {e}"))?;
            preamble_text = std::fs::read(&abs).map_err(|e| format!("{p}: {e}"))?;
            let mut b = bytes(&abs);
            if std::path::MAIN_SEPARATOR == '\\' {
                b.iter_mut().filter(|c| **c == b'\\').for_each(|c| *c = b'/');
            }
            Some(b)
        }
        None => None,
    };
    let d = format::driver(document, &a.usepackage, preamble_file.as_deref(), &preamble_text, a.preset.as_bytes())?;
    let finish = Box::new(move |jobs: &[Job], outcomes: &[Outcome]| report(jobs, outcomes, &[number]));
    let config = Config { jobs: vec![job], pdf: a.pdf, svg: a.svg, print_box: a.print_box, library_only: false, preset: Vec::new(), finish: Some(finish) };
    let argv = vec![
        arg("commute"),
        arg("-progname=pdflatex"),
        arg("-interaction=nonstopmode"),
        arg("-jobname=driver"),
        [b"-output-directory=", &d.dir[..]].concat(),
        d.file.clone(),
    ];
    Ok(rstex_driver::run(argv, Some(config)))
}

fn run(argv: Vec<String>) -> i32 {
    let a = match parse_args(&argv) {
        Ok(Some(a)) => a,
        Ok(None) => return 0,
        Err(e) => {
            eprintln!("commute: {e}\n{USAGE}");
            return STATUS_USAGE;
        }
    };
    match run_checked(&a, argv) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("commute: {e}");
            STATUS_USAGE
        }
    }
}

fn run_checked(a: &Args, mut argv: Vec<String>) -> Result<i32, String> {
    let mut stdin_copy = None;
    let src = if a.file == "-" {
        let mut v = Vec::new();
        std::io::stdin().read_to_end(&mut v).map_err(|e| format!("standard input: {e}"))?;
        v
    } else {
        std::fs::read(&a.file).map_err(|e| format!("{}: {e}", a.file))?
    };
    let found = input::read(&src);
    kpse_init();
    let (dir, stem, numbered) = job_names(a, found.diagrams.len());
    if a.out.is_some() {
        std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    if found.diagrams.is_empty() {
        return Err(format!("{}: no diagram", a.file));
    }
    let job = |k: usize| -> Job {
        let f = found.diagrams[k];
        let name = if numbered { format!("{stem}-{}", k + 1) } else { stem.clone() };
        Job { body: src[f.start..f.end].to_vec(), out: bytes(&dir.join(name)), line: f.line }
    };
    let real = a.force_real || !a.usepackage.is_empty() || a.preamble.is_some() || found.preamble.is_some();
    // The file's own preamble, all that comes before `\\begin{document}`, with `standalone` as `article`: `session.rs` sets the page itself, and that class would wrap the box.
    let document: Option<Vec<u8>> = found.preamble.filter(|_| !a.force_real).map(|p| {
        let Some((at, end)) = p.class.filter(|&(at, end)| src[at..end].ends_with(b"{standalone}")) else {
            return src[..p.document].to_vec();
        };
        let class = &src[at..end];
        let size: Option<&[u8]> = [&b"10pt"[..], b"11pt", b"12pt"].into_iter().find(|s| class.windows(s.len()).any(|w| w == *s));
        let new_class = match size {
            Some(s) => [b"\\documentclass[", s, b"]{article}"].concat(),
            None => b"\\documentclass{article}".to_vec(),
        };
        [&src[..at], &new_class[..], &src[end..p.document]].concat()
    });
    if let Some(n) = a.only {
        if n == 0 || n > found.diagrams.len() {
            return Err(format!("--only {n}: there are {} diagrams", found.diagrams.len()));
        }
        return run_real(a, job(n - 1), n, document.as_deref());
    }
    // A later run cannot read standard input again, and a diagram's name must not change.
    let spawn_needed = found.diagrams.len() > 1 || a.real_pgf;
    if a.file == "-" && spawn_needed {
        let cache = rstex_kpse::cache_dir().ok_or("no cache directory")?.join("commute");
        std::fs::create_dir_all(&cache).map_err(|e| format!("{}: {e}", cache.display()))?;
        let copy = cache.join(format!("stdin-{}.tex", std::process::id()));
        std::fs::write(&copy, &src).map_err(|e| format!("{}: {e}", copy.display()))?;
        if a.stem.is_none() {
            argv.extend(["--stem".to_string(), stem.clone()]);
        }
        if a.out.is_none() {
            argv.extend(["-o".to_string(), ".".to_string()]);
        }
        if let Some(x) = argv.iter_mut().find(|x| x.as_str() == "-") {
            *x = copy.to_string_lossy().into_owned();
        }
        stdin_copy = Some(copy);
    }
    let code = if real {
        if found.diagrams.len() == 1 {
            run_real(a, job(0), 1, document.as_deref())?
        } else {
            let mut code = 0;
            for n in 1..=found.diagrams.len() {
                code = code.max(spawn_one(&argv, &["--only", &n.to_string()]));
            }
            code
        }
    } else {
        let first = a.from.map_or(0, |n| n.saturating_sub(1));
        let jobs: Vec<Job> = (first..found.diagrams.len()).map(job).collect();
        let numbers = (first + 1..=found.diagrams.len()).collect();
        return run_library(a, jobs, numbers, argv, stdin_copy);
    };
    if let Some(c) = stdin_copy {
        let _ = std::fs::remove_file(c);
    }
    Ok(code)
}

/// The C runtime's call.
#[cfg(not(test))]
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
extern "C" fn main(_argc: i32, _argv: *const *const std::ffi::c_char) -> i32 {
    rstex_engine::prof_main_start!();
    platform::process_init();
    let raw: Vec<std::ffi::OsString> = std::env::args_os().collect();
    let code = platform::run_on_stack(rstex_driver::ENGINE_STACK, move || {
        rstex_driver::autofmt::set_engine_prefix(&["--engine"]);
        if raw.get(1).is_some_and(|a| a == "--engine") {
            let mut args = vec![b"rstex".to_vec()];
            args.extend(raw[2..].iter().map(|a| platform::path_to_bytes(Path::new(a))));
            return rstex_driver::run(args, None);
        }
        run(raw.iter().skip(1).map(|a| a.to_string_lossy().into_owned()).collect())
    });
    rstex_engine::prof_finish!();
    std::process::exit(code);
}
