//! `rstex --check-tree`: which of the paths that know the installed macros by their hash work on this tree, and for each that does not, what stands in the way and what to do.  DESIGN.md section 10, "Versions"; `docs/BUILD.md` for the user.
//!
//! All of it is an ordinary run of a small document of its own, by this same program, in a directory of the cache: `native_tree_report` in the engine leaves what the tables say in a file, and the log says what became of its diagram.  So what is
//! reported is what a document gets, and no other run hears of any of this: a path that is off costs time and changes neither a log nor a terminal.
//!
//! What it prints is for people, and the lines that begin `path=` are for programs: `path=NAME state=armed|off|absent`, fields separated by a blank, with `why=` the last field and running to the end of the line.  Exit status 0 if all that is installed is armed, 1 if something is off,
//! 2 if there is no answer, as with no TeX tree.
#![deny(unsafe_code)]

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const DOCUMENT: &str = r"\documentclass{article}
\IfFileExists{microtype.sty}{\usepackage{microtype}}{\typeout{[[tree-absent microtype]]}}
\IfFileExists{tikz-cd.sty}{\usepackage{tikz-cd}}{\typeout{[[tree-absent tikz-cd]]}}
\makeatletter
\begin{document}
\typeout{[[tree LaTeX \fmtversion/pl\patch@level]]}
\@ifundefined{pgfversion}{}{\typeout{[[tree PGF \pgfversion/\pgfversiondate]]}}
\@ifundefined{ver@tikz-cd.sty}{}{\typeout{[[tree tikz-cd \csname ver@tikz-cd.sty\endcsname]]}}
\@ifundefined{ver@microtype.sty}{}{\typeout{[[tree microtype \csname ver@microtype.sty\endcsname]]}}
Text.
\IfFileExists{tikz-cd.sty}{\input{check-diagram}}{}
\end{document}
";
const DIAGRAM: &str = "\\begin{tikzcd} A \\arrow[r, \"f\"] & B \\end{tikzcd}\n";

/// What a package is that a macro of this name is from.
fn package_of(macro_name: &str) -> &'static str {
    if macro_name.contains("tikzcd") || macro_name.contains("commutative diagrams") {
        "tikz-cd"
    } else if ["pgf", "tikz", "Pgf"].iter().any(|p| macro_name.contains(p)) {
        "PGF"
    } else if macro_name.starts_with("MT@") {
        "microtype"
    } else {
        "LaTeX"
    }
}

/// `2021/05/04 v1.0 Commutative diagrams with TikZ` to `2021/05/04/v1.0`, so that a field has no blank.
fn short_version(v: &str) -> String {
    v.split_whitespace().take(2).collect::<Vec<_>>().join("/")
}

/// The fields of the engine's report that begin with this word.
fn fields<'a>(report: &'a str, word: &str) -> Option<Vec<&'a str>> {
    report.lines().find_map(|l| {
        let mut f = l.split('\t');
        (f.next() == Some(word)).then(|| f.collect())
    })
}

/// What the log says between `[[word ` and `]]`, a line break of the log's taken out.
fn said(log: &str, word: &str) -> Option<String> {
    let from = log.find(&format!("[[{word} "))? + word.len() + 3;
    let to = log[from..].find("]]")? + from;
    Some(log[from..to].replace('\n', ""))
}

/// What the log says became of the diagram, if it did not take the path.
fn fallback_reason(log: &str) -> Option<String> {
    let from = log.find("[[commute fallback]] ")? + "[[commute fallback]] ".len();
    let to = log[from..].find('\n').map_or(log.len(), |k| k + from);
    Some(log[from..to].trim_end_matches(['%', ' ']).to_string())
}

fn absent(log: &str, package: &str) -> bool {
    log.contains(&format!("[[tree-absent {package}]]"))
}

/// Where the tree is, from what the run read: for people.
fn tree_root(fls: &str) -> Option<String> {
    let line = fls.lines().find(|l| l.ends_with("article.cls"))?;
    let path = line.strip_prefix("INPUT ")?;
    let at = path.find("/tex/latex/")?;
    Some(path[..at].to_string())
}

struct Work {
    dir: PathBuf,
    /// Nothing of a run of this program can stay in a temporary directory: there is no cache to keep it in.
    remove: bool,
}

fn work_dir() -> Option<Work> {
    if let Some(cache) = rstex_kpse::cache_dir() {
        let dir = cache.join("check-tree");
        if std::fs::create_dir_all(&dir).is_ok() {
            return Some(Work { dir, remove: false });
        }
    }
    let dir = std::env::temp_dir().join(format!("rstex-check-tree-{}", std::process::id()));
    std::fs::create_dir_all(&dir).ok()?;
    Some(Work { dir, remove: true })
}

fn read(dir: &Path, name: &str) -> String {
    String::from_utf8_lossy(&std::fs::read(dir.join(name)).unwrap_or_default()).into_owned()
}

/// Prints the answer, and returns the status.
pub fn check() -> i32 {
    match report() {
        Ok(code) => code,
        Err(why) => {
            println!("rstex: check-tree: no answer: {why}");
            2
        }
    }
}

fn report() -> Result<i32, String> {
    let work = work_dir().ok_or("no directory to work in")?;
    let dir = &work.dir;
    for (name, text) in [("check.tex", DOCUMENT), ("check-diagram.tex", DIAGRAM)] {
        std::fs::write(dir.join(name), text).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    // Two of these at once have one directory, and the second waits.  Nothing else ever holds this.
    let lock = std::fs::File::create(dir.join("lock")).ok();
    if let Some(l) = &lock {
        let _ = l.lock();
    }
    let report_file = dir.join("report");
    let _ = std::fs::remove_file(&report_file);
    let _ = std::fs::remove_file(dir.join("check.log"));
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let out = Command::new(exe)
        .args(crate::autofmt::engine_prefix())
        .args(["-progname=pdflatex", "-interaction=batchmode", "-no-shell-escape", "-recorder", "--commute-report", "check.tex"])
        .current_dir(dir)
        .env("RSTEX_TREE_REPORT", "report")
        .env("RSTEX_CHECKPOINT_POLICY", "always")
        .env("RSTEX_CHECKPOINT_TRACE", "1")
        .env_remove("RSTEX_NO_COMMUTE")
        .env_remove("RSTEX_NO_PF")
        .env_remove("RSTEX_NO_SNAPSHOT")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| e.to_string())?;
    let log = read(dir, "check.log");
    let engine_report = read(dir, "report");
    let fls = read(dir, "check.fls");
    let trace = String::from_utf8_lossy(&out.stderr).into_owned();
    if work.remove {
        let _ = std::fs::remove_dir_all(dir);
    }
    if !log.contains("[[tree LaTeX") {
        let first = trace.lines().next().unwrap_or("the run left no log");
        return Err(format!("no LaTeX here, or it did not get to \\begin{{document}}: {first}"));
    }
    if let Some(root) = tree_root(&fls) {
        println!("rstex: check-tree: tree {root}");
    }
    let version = |word: &str| said(&log, &format!("tree {word}"));
    let latex = version("LaTeX").unwrap_or_default();
    let pgf = version("PGF");
    let cd = version("tikz-cd").map(|v| short_version(&v));
    let microtype = version("microtype").map(|v| short_version(&v));
    let mut versions = format!("LaTeX {latex}");
    for (name, v) in [("PGF", &pgf), ("tikz-cd", &cd), ("microtype", &microtype)] {
        if let Some(v) = v {
            versions += &format!(", {name} {v}");
        }
    }
    println!("rstex: check-tree: {versions}");
    let version_of = |package: &str| -> String {
        match package {
            "PGF" => pgf.clone(),
            "tikz-cd" => cd.clone(),
            "microtype" => microtype.clone(),
            _ => Some(latex.clone()),
        }
        .unwrap_or_else(|| "?".to_string())
    };
    let mut all_armed = true;

    // tikz-cd.
    if absent(&log, "tikz-cd") {
        println!("path=tikz-cd state=absent package=tikz-cd");
    } else {
        let ran = fields(&engine_report, "ran").and_then(|f| f.first().copied()) == Some("1");
        let profile = fields(&engine_report, "profile").unwrap_or_default();
        let id = profile.first().copied().unwrap_or("?");
        if ran && log.contains("[[commute native]]") {
            println!("path=tikz-cd state=armed profile={id}");
        } else {
            all_armed = false;
            let guard = fields(&engine_report, "guard").unwrap_or_default();
            let differ = guard.first().copied().unwrap_or("?");
            let of = guard.get(1).copied().unwrap_or("?");
            let first = guard.get(2).copied().filter(|m| !m.is_empty());
            let why = fallback_reason(&log).unwrap_or_else(|| "the diagram did not take the path".to_string());
            let mut line = format!("path=tikz-cd state=off nearest={id} differ={differ}/{of}");
            if let Some(m) = first {
                let p = package_of(m);
                line += &format!(" macro={m} package={p} version={}", version_of(p));
            }
            println!("{line} why={why}");
            if first.is_some() {
                let names: Vec<String> = guard[2..guard.len().min(5)].iter().map(|m| format!("\\{m}")).collect();
                println!("  {} are not as in the nearest set this rstex knows, of PGF {} and LaTeX {}.", names.join(", "), profile.get(1).unwrap_or(&"?"), profile.get(2).unwrap_or(&"?"));
            }
            println!("  So every tikzcd here is drawn by real PGF.  What comes out is right, and each diagram costs some 270 times the instructions.");
            println!("  A newer rstex may know this tree.  From a checkout of rstex's sources, `python3 -I tests/cd/native/new_profile.py` makes the table entry and says whether it may be used: docs/BUILD.md.");
        }
    }

    // \pickup@font.
    if absent(&log, "microtype") {
        println!("path=pickup@font state=absent package=microtype");
    } else {
        let pf = fields(&engine_report, "pf").unwrap_or_default();
        if pf.first().copied() == Some("1") {
            println!("path=pickup@font state=armed");
        } else {
            all_armed = false;
            let differ = pf.get(2).copied().unwrap_or("?");
            let first = pf.get(3).copied().filter(|m| !m.is_empty());
            let mut line = format!("path=pickup@font state=off differ={differ}");
            if let Some(m) = first {
                let p = package_of(m);
                line += &format!(" macro={m} package={p} version={}", version_of(p));
            }
            println!("{line}");
            println!("  So with microtype each font selection runs the real macros.  What comes out is right, and it takes longer.");
            println!("  A newer rstex may know this tree.  From a checkout of rstex's sources, `python3 -I tests/native_font/gen_table.py` makes the table entry: docs/BUILD.md.");
        }
    }

    // A checkpoint knows two places by what they do with the clock, expl3's and pgfmath's: DESIGN.md section 7.
    if trace.contains("checkpoint: taken") || trace.contains("checkpoint: restored") {
        println!("path=checkpoint state=armed");
    } else if let Some(line) = trace.lines().find_map(|l| l.strip_prefix("rstex: checkpoint: no checkpoint: ")) {
        all_armed = false;
        println!("path=checkpoint state=off why={line}");
        println!("  So each run reads the whole of its preamble.  What comes out is right, and a short document takes many times as long.");
    } else {
        all_armed = false;
        println!("path=checkpoint state=off why=no cache directory, or the run did not get to a checkpoint");
    }
    Ok(if all_armed { 0 } else { 1 })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_macro_is_some_package() {
        assert_eq!(package_of("endpgfscope"), "PGF");
        assert_eq!(package_of("endtikzcd"), "tikz-cd");
        assert_eq!(package_of("pgf@sys@bp"), "PGF");
        assert_eq!(package_of("tikz@do@fig"), "PGF");
        assert_eq!(package_of("pgfk@/tikz/commutative diagrams/every arrow/.@cmd"), "tikz-cd");
        assert_eq!(package_of("MT@split@name"), "microtype");
        assert_eq!(package_of("selectfont"), "LaTeX");
    }

    #[test]
    fn what_the_log_says() {
        let log = "x\n[[tree PGF 3.1.1\n2/2026-08-01]]\ny\n[[tree-absent microtype]]\n";
        assert_eq!(said(log, "tree PGF").as_deref(), Some("3.1.12/2026-08-01"));
        assert!(absent(log, "microtype"));
        assert!(!absent(log, "tikz-cd"));
        assert_eq!(short_version("2021/05/04 v1.0 Commutative diagrams with TikZ"), "2021/05/04/v1.0");
        assert_eq!(fields("a\t1\t2\nb\t3\n", "b"), Some(vec!["3"]));
        assert_eq!(fallback_reason("x\n[[commute fallback]] a macro: y %\nz").as_deref(), Some("a macro: y"));
        assert_eq!(tree_root("INPUT /x/texmf-dist/tex/latex/base/article.cls\n").as_deref(), Some("/x/texmf-dist"));
    }
}
