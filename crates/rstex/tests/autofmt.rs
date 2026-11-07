//! Format auto-build, through the real program, in a cache directory of its own.  Skipped on a machine with no TeX tree.
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

fn scratch(name: &str) -> PathBuf {
    let d = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("autofmt-{name}"));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// DESIGN.md section 12: the environment variable that puts the cache directory under some directory, and where under it.
#[cfg(windows)]
const CACHE_VAR: (&str, &str) = ("LOCALAPPDATA", "rstex/cache");
#[cfg(target_os = "macos")]
const CACHE_VAR: (&str, &str) = ("HOME", "Library/Caches/rstex");
#[cfg(not(any(windows, target_os = "macos")))]
const CACHE_VAR: (&str, &str) = ("XDG_CACHE_HOME", "rstex");

fn rstex(work: &Path, cache: &Path, args: &[&str]) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_rstex"));
    c.args(args).current_dir(work).env(CACHE_VAR.0, cache).stdin(Stdio::null());
    c
}

fn run(work: &Path, cache: &Path, args: &[&str]) -> Output {
    rstex(work, cache, args).output().unwrap()
}

/// A run with `typed` on its standard input, as at the `**` prompt.
fn run_typed(work: &Path, cache: &Path, args: &[&str], typed: &str) -> Output {
    use std::io::Write;
    let mut c = rstex(work, cache, args).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
    c.stdin.take().unwrap().write_all(typed.as_bytes()).unwrap();
    c.wait_with_output().unwrap()
}

fn text(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).to_string() + &String::from_utf8_lossy(&o.stderr)
}

/// The formats in a cache directory, wherever under it they are.
fn formats(cache: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir(cache.join(CACHE_VAR.1)) else { return out };
    for d in rd.flatten().filter(|e| e.file_name().to_string_lossy().starts_with("fmt-")) {
        for f in std::fs::read_dir(d.path()).unwrap().flatten() {
            if f.path().extension().is_some_and(|x| x == "fmt") {
                out.push(f.path());
            }
        }
    }
    out
}

fn tree_is_there(work: &Path, cache: &Path) -> bool {
    // This asks something of the tree and builds nothing.  Its log goes somewhere else.
    let probe = work.parent().unwrap().join("probe");
    std::fs::create_dir_all(&probe).unwrap();
    run(&probe, cache, &["-ini", "-interaction=batchmode", "\\relax\\input plain \\end"]).status.success()
}

/// A run killed in a build leaves its lock.  The next one must not wait for long.
#[test]
fn a_lock_nobody_keeps_fresh_is_passed_by() {
    let d = scratch("lock");
    let (work, cache) = (d.join("work"), d.join("cache"));
    std::fs::create_dir_all(&work).unwrap();
    if !tree_is_there(&work, &cache) {
        return;
    }
    std::fs::write(work.join("a.tex"), "Hello.\\end\n").unwrap();
    assert!(run(&work, &cache, &["a.tex"]).status.success());
    let fmt = formats(&cache).pop().unwrap();
    std::fs::remove_file(&fmt).unwrap();
    let lock = fmt.with_extension("lock");
    let f = std::fs::File::create(&lock).unwrap();
    f.set_modified(std::time::SystemTime::now() - std::time::Duration::from_secs(3600)).unwrap();
    drop(f);
    let begun = std::time::Instant::now();
    let o = run(&work, &cache, &["a.tex"]);
    assert!(o.status.success(), "{}", text(&o));
    assert!(begun.elapsed() < std::time::Duration::from_secs(300), "it waited");
    assert!(fmt.exists() && !lock.exists());
}

#[test]
fn a_format_is_built_once_and_found_again() {
    let d = scratch("once");
    let (work, cache) = (d.join("work"), d.join("cache"));
    std::fs::create_dir_all(&work).unwrap();
    if !tree_is_there(&work, &cache) {
        return;
    }
    std::fs::write(work.join("a.tex"), "Hello.\\end\n").unwrap();
    // The first run builds it, and of that only one line on stderr shows, which README.md has for a program to read.
    let o = run(&work, &cache, &["a.tex"]);
    assert!(o.status.success(), "{}", text(&o));
    assert!(text(&o).contains("(preloaded format=pdftex"), "{}", text(&o));
    assert!(work.join("a.pdf").exists(), "{}", text(&o));
    assert_eq!(String::from_utf8_lossy(&o.stderr), "rstex: building the format pdftex\n");
    assert!(!String::from_utf8_lossy(&o.stdout).contains("building the format"), "{}", text(&o));
    let first_log = std::fs::read(work.join("a.log")).unwrap();
    assert!(!String::from_utf8_lossy(&first_log).contains("building the format"));
    let f = formats(&cache);
    assert_eq!(f.len(), 1, "{f:?}");
    assert!(f[0].ends_with("pdftex.fmt"));
    let leftovers: Vec<_> = std::fs::read_dir(f[0].parent().unwrap()).unwrap().flatten().map(|e| e.file_name()).collect();
    assert_eq!(leftovers.len(), 2, "the format and its list: {leftovers:?}");
    let mut work_files: Vec<_> = std::fs::read_dir(&work).unwrap().flatten().map(|e| e.file_name().to_string_lossy().to_string()).collect();
    work_files.sort();
    assert_eq!(work_files, ["a.log", "a.pdf", "a.tex"], "nothing of the build is left here");
    // The second finds it: the same file, not touched, and no word.
    let when = std::fs::metadata(&f[0]).unwrap().modified().unwrap();
    let o = run(&work, &cache, &["a.tex"]);
    assert!(o.status.success(), "{}", text(&o));
    assert_eq!(std::fs::metadata(&f[0]).unwrap().modified().unwrap(), when);
    assert_eq!(String::from_utf8_lossy(&o.stderr), "");
    // RSTEX_FMT_VERBOSE shows a build as well: the same line, and what the build says.
    std::fs::remove_file(&f[0]).unwrap();
    let o = rstex(&work, &cache, &["a.tex"]).env("RSTEX_FMT_VERBOSE", "1").output().unwrap();
    assert!(String::from_utf8_lossy(&o.stderr).lines().any(|l| l == "rstex: building the format pdftex"), "{}", text(&o));
    assert!(text(&o).contains("Beginning to dump on file pdftex.fmt"), "{}", text(&o));
    // A list that does not hold is a new build.
    let deps = f[0].with_extension("deps");
    let mut bytes = std::fs::read(&deps).unwrap();
    let n = bytes.len();
    bytes[n / 2] ^= 1;
    std::fs::write(&deps, &bytes).unwrap();
    let when = std::fs::metadata(&f[0]).unwrap().modified().unwrap();
    std::thread::sleep(std::time::Duration::from_millis(20));
    let o = run(&work, &cache, &["a.tex"]);
    assert!(o.status.success(), "{}", text(&o));
    assert_ne!(std::fs::metadata(&f[0]).unwrap().modified().unwrap(), when);
    assert_ne!(std::fs::read(&deps).unwrap(), bytes);
}

#[test]
fn what_names_a_format_keeps_its_meaning() {
    let d = scratch("names");
    let (work, cache) = (d.join("work"), d.join("cache"));
    std::fs::create_dir_all(&work).unwrap();
    if !tree_is_there(&work, &cache) {
        return;
    }
    std::fs::write(work.join("a.tex"), "\\end\n").unwrap();
    let before = formats(&cache).len();
    // Nothing is built for any of these, and they say what pdfTeX says.  As the first argument `&name` is the default as well.
    let o = run(&work, &cache, &["-fmt=rstex-no-such-format", "a.tex"]);
    assert!(text(&o).contains("I can't find the format file `rstex-no-such-format.fmt'!"), "{}", text(&o));
    let o = run(&work, &cache, &["-progname=rstex-no-such-format", "a.tex"]);
    assert!(text(&o).contains("I can't find the format file `rstex-no-such-format.fmt'!"), "{}", text(&o));
    let o = run(&work, &cache, &["&rstex-no-such-format", "a.tex"]);
    assert!(text(&o).contains("Sorry, I can't find the format `rstex-no-such-format.fmt'; will try `rstex-no-such-format.fmt'."), "{}", text(&o));
    assert_eq!(formats(&cache).len(), before);
    // -ini loads none.
    let o = run(&work, &cache, &["-ini", "-interaction=batchmode", "\\end"]);
    assert!(o.status.success(), "{}", text(&o));
    assert_eq!(formats(&cache).len(), before);
    // A format of this build that the search finds comes first.
    let o = run(&work, &cache, &["-ini", "-jobname=pdftex", "-progname=pdftex", "-interaction=batchmode", "*pdfetex.ini"]);
    assert!(o.status.success(), "{}", text(&o));
    assert!(work.join("pdftex.fmt").exists());
    let o = run(&work, &cache, &["a.tex"]);
    assert!(o.status.success(), "{}", text(&o));
    assert_eq!(formats(&cache).len(), before);
}

/// TeX Live's formats are gzip files, which this engine cannot read.  `-fmt`, `&name` on the command line or typed at the `**` prompt, and `-ini &name` as in mylatexformat's
/// recipe find them all the same, and each gets this engine's own, as the program's name and a `%&` line do.  A `&name` that finds nothing goes on to the default as in
/// pdfTeX, and the line that says so names it as pdfTeX's does.
#[test]
fn a_format_named_any_way_is_this_engines_own() {
    let d = scratch("named");
    let (work, cache) = (d.join("work"), d.join("cache"));
    std::fs::create_dir_all(&work).unwrap();
    if !tree_is_there(&work, &cache) {
        return;
    }
    std::fs::write(work.join("a.tex"), "\\message{[\\fmtname]}\\end\n").unwrap();
    for args in [&["-fmt=pdftex", "a.tex"][..], &["-fmt=pdftex.fmt", "a.tex"], &["&pdftex", "a.tex"], &["-interaction=nonstopmode", "&pdftex", "a.tex"]] {
        let o = run(&work, &cache, args);
        assert!(o.status.success() && text(&o).contains("[plain]"), "{args:?}: {}", text(&o));
    }
    let o = run_typed(&work, &cache, &[], "&pdftex a\n");
    assert!(o.status.success() && text(&o).contains("[plain]"), "{}", text(&o));
    assert_eq!(formats(&cache).len(), 1, "one build for all of them");
    // A format of one's own on top of it, which then loads as it stands.
    std::fs::write(work.join("mine.tex"), "\\def\\fmtname{mine}\\dump\n").unwrap();
    let o = run(&work, &cache, &["-ini", "-jobname=mine", "&pdftex", "mine.tex"]);
    assert!(o.status.success() && work.join("mine.fmt").exists(), "{}", text(&o));
    let o = run(&work, &cache, &["-fmt=mine", "a.tex"]);
    assert!(o.status.success() && text(&o).contains("[mine]"), "{}", text(&o));
    // A name that finds nothing.  Not the first argument, which would be the default as well.
    let sorry = "Sorry, I can't find the format `rstex-no-such-format.fmt'; will try `pdftex.fmt'.";
    let o = run(&work, &cache, &["-interaction=nonstopmode", "&rstex-no-such-format", "a.tex"]);
    assert!(o.status.success() && text(&o).contains(sorry) && text(&o).contains("[plain]"), "{}", text(&o));
    let o = run_typed(&work, &cache, &[], "&rstex-no-such-format a\n");
    assert!(o.status.success() && text(&o).contains(sorry) && text(&o).contains("[plain]"), "{}", text(&o));
    assert_eq!(formats(&cache).len(), 1);
}

/// latexmk puts its `-outdir` in `TEXMFOUTPUT`, an editor may set `TEXEDIT`, and `TEXMFDOTDIR` is a document's: a new value of one is no reason for another format.
#[test]
fn what_belongs_to_a_document_builds_no_other_format() {
    let d = scratch("document-variables");
    let (work, cache) = (d.join("work"), d.join("cache"));
    std::fs::create_dir_all(&work).unwrap();
    if !tree_is_there(&work, &cache) {
        return;
    }
    std::fs::write(work.join("a.tex"), "Hello.\\end\n").unwrap();
    let o = run(&work, &cache, &["a.tex"]);
    assert!(o.status.success(), "{}", text(&o));
    assert_eq!(formats(&cache).len(), 1);
    let out_b = d.join("out-b");
    for (var, value) in [("TEXMFOUTPUT", Path::new("out-a")), ("TEXMFOUTPUT", &out_b), ("TEXEDIT", Path::new("vi +%d %s")), ("TEXMFDOTDIR", Path::new("."))] {
        let o = rstex(&work, &cache, &["a.tex"]).env(var, value).output().unwrap();
        assert!(o.status.success(), "{var}: {}", text(&o));
        assert_eq!(String::from_utf8_lossy(&o.stderr), "", "{var}={}", value.display());
        assert_eq!(formats(&cache).len(), 1, "{var}={}", value.display());
    }
}

/// A user's `texmf.cnf`, which `TEXMFCNF` puts before the installation's, says where a document's files are, and not where the format's are: the format is built as the
/// installation's is, which pdfTeX loads whatever that file says.  With a path that has nothing of the tree on it, it builds, and with a `pdftexconfig.tex` of the user's
/// on it, that is not in the format.
#[test]
fn a_users_texmf_cnf_does_not_reach_the_format() {
    let d = scratch("user-cnf");
    let (work, cache) = (d.join("work"), d.join("cache"));
    std::fs::create_dir_all(&work).unwrap();
    if !tree_is_there(&work, &cache) {
        return;
    }
    // `;` separates in a `texmf.cnf` on any system, and `/` is a directory's separator on any.
    let at = |p: &str| d.join(p).to_string_lossy().replace('\\', "/");
    let texmfcnf = |dir: &str| format!("{}{}", at(dir), if cfg!(windows) { ";" } else { ":" });
    // What the run wrote, which it takes away.
    let result = || std::fs::read_to_string(work.join("result.txt")).map(|r| r.trim().to_string()).and_then(|r| std::fs::remove_file(work.join("result.txt")).map(|_| r)).unwrap_or_default();
    let write = "\\immediate\\openout1=result.txt \\immediate\\write1{\\the\\pdfpagewidth}\\immediate\\closeout1 \\end\n";
    std::fs::write(work.join("a.tex"), write).unwrap();
    std::fs::write(work.join("b.tex"), format!("\\input mymacros {}", write.replace("{\\the", "{\\mine:\\the"))).unwrap();
    let o = run(&work, &cache, &["a.tex"]);
    assert!(o.status.success(), "{}", text(&o));
    let width = result();
    assert!(width.ends_with("pt"), "{width}");
    std::fs::create_dir_all(d.join("macros")).unwrap();
    std::fs::write(d.join("macros/mymacros.tex"), "\\def\\mine{from macros}\n").unwrap();
    std::fs::create_dir_all(d.join("cnf-macros")).unwrap();
    std::fs::write(d.join("cnf-macros/texmf.cnf"), format!("TEXINPUTS.pdftex = .;{}//\n", at("macros"))).unwrap();
    let o = rstex(&work, &cache, &["b.tex"]).env("TEXMFCNF", texmfcnf("cnf-macros")).output().unwrap();
    assert!(o.status.success(), "{}", text(&o));
    assert_eq!(result(), format!("from macros:{width}"));
    std::fs::create_dir_all(d.join("paper")).unwrap();
    std::fs::write(d.join("paper/pdftexconfig.tex"), "\\pdfpagewidth=100 true mm\n").unwrap();
    std::fs::create_dir_all(d.join("cnf-paper")).unwrap();
    std::fs::write(d.join("cnf-paper/texmf.cnf"), format!("TEXINPUTS.pdftex = .;{}//;$TEXMF/tex/{{plain,generic,}}//\n", at("paper"))).unwrap();
    let o = rstex(&work, &cache, &["a.tex"]).env("TEXMFCNF", texmfcnf("cnf-paper")).output().unwrap();
    assert!(o.status.success(), "{}", text(&o));
    assert_eq!(result(), width);
}

/// The run that held the lock was killed.  That leaves the file, and no lock, so the next run does not wait for anybody.
#[test]
fn a_lock_file_left_behind_makes_nobody_wait() {
    let d = scratch("killed");
    let (work, cache) = (d.join("work"), d.join("cache"));
    std::fs::create_dir_all(&work).unwrap();
    if !tree_is_there(&work, &cache) {
        return;
    }
    std::fs::write(work.join("a.tex"), "\\end\n").unwrap();
    assert!(run(&work, &cache, &["a.tex"]).status.success());
    let f = formats(&cache);
    assert_eq!(f.len(), 1, "{f:?}");
    std::fs::remove_file(&f[0]).unwrap();
    std::fs::write(f[0].with_extension("lock"), b"").unwrap();
    let start = std::time::Instant::now();
    let o = run(&work, &cache, &["a.tex"]);
    assert!(o.status.success(), "{}", text(&o));
    assert!(start.elapsed().as_secs() < 120, "it waited");
    assert!(!text(&o).contains("waiting for another run"), "{}", text(&o));
    assert!(f[0].exists());
    assert!(!f[0].with_extension("lock").exists());
}

/// A cache directory that cannot be made: the document compiles, on a format built for it alone that is gone afterwards, and `RSTEX_FMT_VERBOSE` says why.
#[test]
fn a_cache_that_cannot_be_made_costs_only_a_build() {
    let d = scratch("nocache");
    let work = d.join("work");
    std::fs::create_dir_all(&work).unwrap();
    if !tree_is_there(&work, &d.join("probe-cache")) {
        return;
    }
    // Under a file, which no directory can be.
    let not_a_dir = d.join("file");
    std::fs::write(&not_a_dir, b"").unwrap();
    std::fs::write(work.join("a.tex"), "Hello.\\end\n").unwrap();
    let o = rstex(&work, &not_a_dir, &["a.tex"]).env("RSTEX_FMT_VERBOSE", "1").output().unwrap();
    assert!(o.status.success(), "{}", text(&o));
    assert!(text(&o).contains("no cache directory can be used"), "{}", text(&o));
    let mut work_files: Vec<_> = std::fs::read_dir(&work).unwrap().flatten().map(|e| e.file_name().to_string_lossy().to_string()).collect();
    work_files.sort();
    assert_eq!(work_files, ["a.log", "a.pdf", "a.tex"], "nothing of the build is left here");
}

/// Several at once, on an empty cache: none fails, and they agree.
#[test]
fn runs_at_the_same_time_share_one_build() {
    let d = scratch("many");
    let cache = d.join("cache");
    if !tree_is_there(&d.join("w0"), &cache) {
        return;
    }
    let mut children = Vec::new();
    for i in 0..4 {
        let work = d.join(format!("w{i}"));
        std::fs::create_dir_all(&work).unwrap();
        std::fs::write(work.join("a.tex"), "Hello.\\end\n").unwrap();
        children.push((work.clone(), rstex(&work, &cache, &["a.tex"]).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap()));
    }
    for (work, c) in children {
        let o = c.wait_with_output().unwrap();
        assert!(o.status.success(), "{}", text(&o));
        assert!(work.join("a.pdf").exists());
    }
    assert_eq!(formats(&cache).len(), 1);
}
