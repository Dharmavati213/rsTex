//! Format auto-build: `rstex doc.tex`, or a `pdflatex` that is rstex, with no format anywhere but TeX Live's gzip files, which this engine cannot read.
//!
//! When the command line names no format, the name from the program name, or from a `%&` line, is looked for on the search path as pdfTeX would.  A file there
//! that this build wrote is used as it stands.  Otherwise the format comes from rstex's cache directory, in a directory for this build and this tree.  It holds, beside
//! the format, a list of the file stamps of everything the format was built from, that the `-recorder` file of the build gave.  If the format is not there, or a stamp
//! differs, it is built, once, by this same executable, as `fmtutil` would, in a directory of its own, and published by rename.  Runs at the same time wait for the
//! one that holds a lock, and whatever happens to that one, a run can go on and build its own: the rename is what makes that safe.  `RSTEX_FMT_VERBOSE` shows
//! the build, which is otherwise silent.
//!
//! `-ini`, `-fmt` and a first line with `&name` never come here: the engine does what they say.
//!
//! `commute` is the other program that builds formats, its own on top of `pdflatex.fmt` by `derived`, and it comes here as well: it says with `set_engine_prefix` how its executable becomes this one.
use rstex_kpse as kpse;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::OnceLock;
use std::time::Duration;

/// What the executable needs first to be `rstex`, if it is another.
static ENGINE_PREFIX: OnceLock<&'static [&'static str]> = OnceLock::new();

pub fn set_engine_prefix(prefix: &'static [&'static str]) {
    let _ = ENGINE_PREFIX.set(prefix);
}

/// A stamp younger than this might not show a change made just after it: as `crates/kpse/src/index.rs`.
const RACY_NS: u64 = 2_000_000_000;
/// How long a run waits for another's build: pdflatex.fmt takes seconds, even on a busy machine.
const WAIT: Duration = Duration::from_secs(600);
/// A lock not touched for this long belongs to a run that died: one that builds touches it each second.
const STALE_LOCK: Duration = Duration::from_secs(30);
const LOCK_TOUCH: Duration = Duration::from_secs(1);
/// What a format's directory is kept for, beyond the newest few, and how many those are.
const KEEP_DIRS: usize = 4;
/// `derived`.
const DERIVED_RECHECK: Duration = Duration::from_secs(60);
const OLD_DIR: Duration = Duration::from_secs(3600);
/// Variables that belong to a document, not to a format, or that would put a date of theirs in it.
const NOT_FOR_A_BUILD: [&str; 7] = ["TEXINPUTS", "TEXFORMATS", "TEXMF_OUTPUT_DIRECTORY", "KPSE_DOT", "SOURCE_DATE_EPOCH", "FORCE_SOURCE_DATE", "RSTEX_FMT_VERBOSE"];
/// For a tree with no `fmtutil.cnf`.  The first argument of each is what `fmtutil` gives every format: -ini, -jobname and -progname come first.
const BUILTIN_RECIPES: [(&[u8], &str); 2] = [(b"pdflatex", "*pdflatex.ini"), (b"pdftex", "*pdfetex.ini")];

/// The format to use for `name`, if not the one that `name` finds on the search path: its path in the cache, built now if need be.  `None` leaves it to the engine.
pub fn resolve(name: &[u8]) -> Option<Vec<u8>> {
    let base = name.strip_suffix(b".fmt").unwrap_or(name);
    if base.is_empty() || base.contains(&b'/') || base.contains(&b'\\') {
        return None;
    }
    let with_ext = [base, b".fmt"].concat();
    if let Some(found) = kpse::find_file(&with_ext, kpse::KPSE_FMT_FORMAT, false) {
        if rstex_engine::image::fmt_file_is_this_builds(&rstex_engine::platform::path_from_bytes(&found)) {
            return None;
        }
    }
    let dir = format_dir()?;
    let fmt = dir.join(rstex_engine::platform::path_from_bytes(&with_ext));
    let deps = dir.join(rstex_engine::platform::path_from_bytes(&[base, b".deps"].concat()));
    let key = kpse::hash64(&with_ext);
    if deps_hold(&deps, key) {
        return Some(rstex_engine::platform::path_to_bytes(&fmt));
    }
    let args = recipe(base)?;
    let job = Build { dir: &dir, fmt: &fmt, deps: &deps, key, name: base, progname: base, args: &args, also: &[], driver: None };
    if !build_once(&job) {
        return None;
    }
    Some(rstex_engine::platform::path_to_bytes(&fmt))
}

/// A format made by running `driver` on the format `base`, which `resolve` finds: its path, built now if need be, after a word from `before_build`.  Its list holds what `base` was made from as well, so that
/// one look at it is enough on the usual run.  That look is some 200 `stat`s, a tenth of the time of a run of `commute`, which may well be in a loop, so it is taken once in `DERIVED_RECHECK`, which is how long
/// a change to the tree can go unseen.  The list's own modification time says when it last held.
pub fn derived(name: &[u8], base: &[u8], driver: &[u8], before_build: &dyn Fn()) -> Option<Vec<u8>> {
    let dir = format_dir()?;
    let as_os = rstex_engine::platform::path_from_bytes;
    let fmt = dir.join(as_os(&[name, b".fmt"].concat()));
    let deps = dir.join(as_os(&[name, b".deps"].concat()));
    let key = kpse::hash64(&[name, b".fmt"].concat());
    let looked = std::fs::metadata(&deps).and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok());
    if looked.is_some_and(|a| a < DERIVED_RECHECK) && fmt.is_file() {
        return Some(rstex_engine::platform::path_to_bytes(&fmt));
    }
    if deps_hold(&deps, key) {
        if let Ok(f) = std::fs::OpenOptions::new().write(true).open(&deps) {
            let _ = f.set_modified(std::time::SystemTime::now());
        }
        return Some(rstex_engine::platform::path_to_bytes(&fmt));
    }
    before_build();
    let from = resolve(base);
    // In the directory of `fmt` if it was built, and so a name with no blank in it, one level above the build's own directory.
    let first: Vec<u8> = if from.is_some() { [b"&../", base].concat() } else { [b"&", base].concat() };
    let also = kpse::CachedFile::open(&dir.join(as_os(&[base, b".deps"].concat())), kpse::hash64(&[base, b".fmt"].concat())).map(|f| f.front().to_vec()).unwrap_or_default();
    let args = vec![b"-translate-file=cp227.tcx".to_vec(), first, b"driver.tex".to_vec()];
    let job = Build { dir: &dir, fmt: &fmt, deps: &deps, key, name, progname: base, args: &args, also: &also, driver: Some(driver) };
    if !build_once(&job) {
        return None;
    }
    Some(rstex_engine::platform::path_to_bytes(&fmt))
}

/// True if there is a recipe for `base`, so that a `%&base` line means something though no file has that name.
pub fn can_build(base: &[u8]) -> bool {
    recipe(base).is_some()
}

/// The directory for this build and this tree.
fn format_dir() -> Option<PathBuf> {
    let tree = kpse::with_global(|k| k.tree_key());
    Some(kpse::cache_dir()?.join(format!("fmt-{}-{tree:016x}", rstex_engine::image::FMT_BUILD_ID)))
}

/// What `fmtutil` would add to `-ini -jobname=base -progname=base` for a format built by this engine.
fn recipe(base: &[u8]) -> Option<Vec<Vec<u8>>> {
    if let Some(path) = kpse::find_file(b"fmtutil.cnf", kpse::KPSE_WEB2C_FORMAT, true) {
        if let Ok(text) = std::fs::read(rstex_engine::platform::path_from_bytes(&path)) {
            for line in text.split(|&c| c == b'\n') {
                let mut f = line.split(|c| c.is_ascii_whitespace()).filter(|w| !w.is_empty());
                if f.next() == Some(base) && f.next() == Some(b"pdftex") {
                    f.next(); // the hyphenation file, which `fmtutil` only reports
                    return Some(f.map(<[u8]>::to_vec).collect());
                }
            }
            return None;
        }
    }
    let (_, ini) = BUILTIN_RECIPES.iter().find(|(n, _)| *n == base)?;
    Some(vec![b"-translate-file=cp227.tcx".to_vec(), ini.as_bytes().to_vec()])
}

/// One entry of a `.deps` file, in its front: the stamp, the path's length, the path.
fn put_dep(out: &mut Vec<u8>, path: &[u8], st: &kpse::Stamp) {
    for x in st {
        out.extend_from_slice(&x.to_le_bytes());
    }
    out.extend_from_slice(&(path.len() as u32).to_le_bytes());
    out.extend_from_slice(path);
}

/// True if the list is there, sound, and each file has the stamp it says.
fn deps_hold(deps: &Path, key: u64) -> bool {
    let Some(f) = kpse::CachedFile::open(deps, key) else { return false };
    let mut b = f.front();
    while !b.is_empty() {
        let Some((head, rest)) = b.split_at_checked(44) else { return false };
        let st: kpse::Stamp = core::array::from_fn(|i| u64::from_le_bytes(head[i * 8..i * 8 + 8].try_into().unwrap()));
        let len = u32::from_le_bytes(head[40..44].try_into().unwrap()) as usize;
        let Some((path, rest)) = rest.split_at_checked(len) else { return false };
        if kpse::file_stamp(path) != st {
            return false;
        }
        b = rest;
    }
    !f.front().is_empty()
}

fn verbose() -> bool {
    std::env::var_os("RSTEX_FMT_VERBOSE").is_some_and(|v| !v.is_empty())
}

/// One format to build.
struct Build<'a> {
    dir: &'a Path,
    fmt: &'a Path,
    deps: &'a Path,
    key: u64,
    name: &'a [u8],
    progname: &'a [u8],
    args: &'a [Vec<u8>],
    /// Entries for its list from the format it starts from.
    also: &'a [u8],
    /// A first file of its own, `driver.tex` in the build's directory.
    driver: Option<&'a [u8]>,
}

/// Builds the format unless another run does it first.  True if it holds now.
fn build_once(job: &Build<'_>) -> bool {
    let (dir, deps, key) = (job.dir, job.deps, job.key);
    if std::fs::create_dir_all(dir).is_err() {
        return false;
    }
    let lock = deps.with_extension("lock");
    let started = std::time::Instant::now();
    loop {
        match std::fs::OpenOptions::new().write(true).create_new(true).open(&lock) {
            Ok(_) => {
                // Between the check that sent us here and the lock, another run may have finished.
                let ok = deps_hold(deps, key) || build(job);
                let _ = std::fs::remove_file(&lock);
                return ok;
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(_) => return build(job), // no locks here: the rename still makes it safe
        }
        std::thread::sleep(Duration::from_millis(20));
        if deps_hold(deps, key) {
            return true;
        }
        let age = std::fs::metadata(&lock).and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok());
        if age.is_some_and(|a| a > STALE_LOCK) {
            let _ = std::fs::remove_file(&lock);
        } else if started.elapsed() > WAIT {
            return build(job);
        }
    }
}

/// The build itself: in a new directory, so that nothing in the current one is read and nothing is left there.
fn build(job: &Build<'_>) -> bool {
    let (dir, fmt, deps, key, base) = (job.dir, job.fmt, job.deps, job.key, job.name);
    let tmp = dir.join(format!("tmp-{}", std::process::id()));
    if std::fs::create_dir_all(&tmp).is_err() {
        return false;
    }
    if let Some(d) = job.driver {
        if std::fs::write(tmp.join("driver.tex"), d).is_err() {
            return false;
        }
    }
    let Ok(exe) = std::env::current_exe() else { return false };
    let as_os = rstex_engine::platform::path_from_bytes;
    let mut c = Command::new(exe);
    c.args(ENGINE_PREFIX.get().copied().unwrap_or(&[]));
    c.arg("-ini");
    c.arg(as_os(&[b"-jobname=", base].concat()));
    c.arg(as_os(&[b"-progname=", job.progname].concat()));
    c.arg("-recorder");
    for a in job.args {
        c.arg(as_os(a));
    }
    c.current_dir(&tmp).stdin(Stdio::null());
    if verbose() {
        eprintln!("rstex: building the format {}", String::from_utf8_lossy(base));
    } else {
        c.stdout(Stdio::null()).stderr(Stdio::null());
    }
    for (k, _) in std::env::vars_os() {
        // TEXINPUTS.pdflatex and TEXINPUTS_pdflatex too.
        let k_text = k.to_string_lossy();
        if NOT_FOR_A_BUILD.iter().any(|n| k_text == *n || k_text.starts_with(&format!("{n}.")) || k_text.starts_with(&format!("{n}_"))) {
            c.env_remove(&k);
        }
    }
    let begun = kpse::now_ns();
    let built = tmp.join(as_os(&[base, b".fmt"].concat()));
    let ok = run_touching(&mut c, &deps.with_extension("lock")) && built.is_file();
    if !ok {
        eprintln!("rstex: the format {} did not build: see {}", String::from_utf8_lossy(base), tmp.join(as_os(&[base, b".log"].concat())).display());
        return false;
    }
    let fls = std::fs::read(tmp.join(as_os(&[base, b".fls"].concat()))).unwrap_or_default();
    let mut list = job.also.to_vec();
    let mut settled = true;
    for line in fls.split(|&c| c == b'\n') {
        let Some(path) = line.strip_prefix(b"INPUT ") else { continue };
        // A relative name is the build's own, in its directory.
        if !rstex_engine::platform::is_absolute(path) {
            continue;
        }
        let st = kpse::file_stamp(path);
        settled &= st[2].max(st[3]) + RACY_NS < begun;
        put_dep(&mut list, path, &st);
    }
    if std::fs::rename(&built, fmt).is_err() {
        return false;
    }
    put_dep(&mut list, &rstex_engine::platform::path_to_bytes(fmt), &kpse::file_stamp(&rstex_engine::platform::path_to_bytes(fmt)));
    // Without the list the next run builds again, which is right for what has only just changed.  This run goes on, with what it has just built.
    if settled {
        kpse::publish_path(deps, &kpse::make_cached_file(key, &list, &[]));
    }
    let _ = std::fs::remove_dir_all(&tmp);
    prune(dir);
    true
}

/// Runs the build, keeping the lock fresh, so that a run that waits knows it from one that was killed, which someone's Ctrl-C can have done at any time.
fn run_touching(c: &mut Command, lock: &Path) -> bool {
    let Ok(mut child) = c.spawn() else { return false };
    let mut touched = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(s)) => return s.success(),
            Ok(None) => {}
            Err(_) => return false,
        }
        std::thread::sleep(Duration::from_millis(10));
        if touched.elapsed() >= LOCK_TOUCH {
            touched = std::time::Instant::now();
            if let Ok(f) = std::fs::OpenOptions::new().write(true).open(lock) {
                let _ = f.set_modified(std::time::SystemTime::now());
            }
        }
    }
}

/// Old builds' formats go, in time: each is ten megabytes or more.
fn prune(current: &Path) {
    let Some(parent) = current.parent() else { return };
    let Ok(rd) = std::fs::read_dir(parent) else { return };
    let mut dirs: Vec<(std::time::SystemTime, PathBuf)> = rd
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().starts_with("fmt-") && e.path() != current)
        .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
        .collect();
    dirs.sort();
    dirs.reverse();
    for (when, d) in dirs.into_iter().skip(KEEP_DIRS - 1) {
        if when.elapsed().is_ok_and(|a| a > OLD_DIR) {
            let _ = std::fs::remove_dir_all(d);
        }
    }
}
