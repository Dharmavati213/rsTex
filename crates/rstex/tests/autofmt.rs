//! Format auto-build, through the real program, in a cache directory of its own.  Skipped on a machine with no TeX tree.
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

fn scratch(name: &str) -> PathBuf {
    let d = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("autofmt-{name}"));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn rstex(work: &Path, cache: &Path, args: &[&str]) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_rstex"));
    c.args(args).current_dir(work).env("XDG_CACHE_HOME", cache).stdin(Stdio::null());
    c
}

fn run(work: &Path, cache: &Path, args: &[&str]) -> Output {
    rstex(work, cache, args).output().unwrap()
}

fn text(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).to_string() + &String::from_utf8_lossy(&o.stderr)
}

/// The formats in a cache directory, wherever under it they are.
fn formats(cache: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir(cache.join("rstex")) else { return out };
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

#[test]
fn a_format_is_built_once_and_found_again() {
    let d = scratch("once");
    let (work, cache) = (d.join("work"), d.join("cache"));
    std::fs::create_dir_all(&work).unwrap();
    if !tree_is_there(&work, &cache) {
        return;
    }
    std::fs::write(work.join("a.tex"), "Hello.\\end\n").unwrap();
    // The first run builds it, and nothing of that shows.
    let o = run(&work, &cache, &["a.tex"]);
    assert!(o.status.success(), "{}", text(&o));
    assert!(text(&o).contains("(preloaded format=pdftex"), "{}", text(&o));
    assert!(work.join("a.pdf").exists(), "{}", text(&o));
    let f = formats(&cache);
    assert_eq!(f.len(), 1, "{f:?}");
    assert!(f[0].ends_with("pdftex.fmt"));
    let leftovers: Vec<_> = std::fs::read_dir(f[0].parent().unwrap()).unwrap().flatten().map(|e| e.file_name()).collect();
    assert_eq!(leftovers.len(), 2, "the format and its list: {leftovers:?}");
    let mut work_files: Vec<_> = std::fs::read_dir(&work).unwrap().flatten().map(|e| e.file_name().to_string_lossy().to_string()).collect();
    work_files.sort();
    assert_eq!(work_files, ["a.log", "a.pdf", "a.tex"], "nothing of the build is left here");
    // The second finds it: the same file, not touched.
    let when = std::fs::metadata(&f[0]).unwrap().modified().unwrap();
    let o = run(&work, &cache, &["a.tex"]);
    assert!(o.status.success(), "{}", text(&o));
    assert_eq!(std::fs::metadata(&f[0]).unwrap().modified().unwrap(), when);
    // RSTEX_FMT_VERBOSE shows a build, and only then.
    std::fs::remove_file(&f[0]).unwrap();
    let o = rstex(&work, &cache, &["a.tex"]).env("RSTEX_FMT_VERBOSE", "1").output().unwrap();
    assert!(text(&o).contains("building the format pdftex"), "{}", text(&o));
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
    // Nothing is built for any of these, and they say what pdfTeX says.
    let o = run(&work, &cache, &["-fmt=rstex-no-such-format", "a.tex"]);
    assert!(text(&o).contains("I can't find the format file `rstex-no-such-format.fmt'!"), "{}", text(&o));
    let o = run(&work, &cache, &["-progname=rstex-no-such-format", "a.tex"]);
    assert!(text(&o).contains("I can't find the format file `rstex-no-such-format.fmt'!"), "{}", text(&o));
    let o = run(&work, &cache, &["&rstex-no-such-format", "a.tex"]);
    assert!(text(&o).contains("Sorry, I can't find the format `rstex-no-such-format.fmt'"), "{}", text(&o));
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
