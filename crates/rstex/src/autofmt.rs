//! Format auto-build: `rstex doc.tex`, or a `pdflatex` that is rstex, with no format anywhere but TeX Live's gzip files, which this engine cannot read.
//!
//! When the command line names no format, the name from the program name, or from a `%&` line, is looked for on the search path as pdfTeX would.  A file there
//! that this build wrote is used as it stands.  Otherwise the format comes from rstex's cache directory, in a directory for this build and this tree.  It holds, beside
//! the format, a list of the file stamps of everything the format was built from, that the `-recorder` file of the build gave.  If the format is not there, or a stamp
//! differs, it is built, once, by this same executable, as `fmtutil` would, in a directory of its own, and published by rename.  Runs at the same time wait for the
//! one that holds a lock on a file.  The system lets go of that when the holder ends, however it ends, so Control-C during a first run costs the next one nothing, and whatever
//! happens otherwise, a run can go on and build its own: the rename is what makes that safe.  `RSTEX_FMT_VERBOSE` shows the build, which is otherwise silent.
//!
//! `-ini`, `-fmt` and a first line with `&name` never come here: the engine does what they say.
use rstex_kpse as kpse;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

/// A stamp younger than this might not show a change made just after it: as `crates/kpse/src/index.rs`.
const RACY_NS: u64 = 2_000_000_000;
/// How long a run waits for another's build, which is alive, since it holds the lock: pdflatex.fmt takes seconds, even on a busy machine.
const WAIT: Duration = Duration::from_secs(600);
/// A wait longer than this gets a line on stderr: it might not be what anyone expects.
const WAIT_SILENTLY: Duration = Duration::from_secs(10);
/// What a format's directory is kept for, beyond the newest few, and how many those are.
const KEEP_DIRS: usize = 4;
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
    let Some(dir) = format_dir() else {
        if recipe(base).is_some() {
            eprintln!("rstex: no directory to keep the format {} in: the variable that says where a home or a cache is, is not set", String::from_utf8_lossy(base));
        }
        return None;
    };
    let fmt = dir.join(rstex_engine::platform::path_from_bytes(&with_ext));
    let deps = dir.join(rstex_engine::platform::path_from_bytes(&[base, b".deps"].concat()));
    let key = kpse::hash64(&with_ext);
    if deps_hold(&deps, key) {
        return Some(rstex_engine::platform::path_to_bytes(&fmt));
    }
    let args = recipe(base)?;
    if !build_once(&dir, &fmt, &deps, key, base, &args) {
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

/// Builds the format unless another run does it first.  True if it holds now.
fn build_once(dir: &Path, fmt: &Path, deps: &Path, key: u64, base: &[u8], args: &[Vec<u8>]) -> bool {
    if let Err(e) = std::fs::create_dir_all(dir) {
        eprintln!("rstex: cannot make the directory {} for the format: {}", dir.display(), rstex_engine::platform::os_error_text(&e));
        return false;
    }
    let lock = deps.with_extension("lock");
    let started = std::time::Instant::now();
    let mut said = false;
    loop {
        let file = std::fs::OpenOptions::new().write(true).create(true).truncate(false).open(&lock);
        match file.as_ref().map(std::fs::File::try_lock) {
            Ok(Ok(())) => {
                // Between the check that sent us here and the lock, another run may have finished.
                let ok = deps_hold(deps, key) || build(dir, fmt, deps, key, base, args);
                // Another run may have this very file open, and get a lock on it when it is gone: a build too many at the worst.
                let _ = std::fs::remove_file(&lock);
                return ok;
            }
            Ok(Err(std::fs::TryLockError::WouldBlock)) => {}
            _ => return build(dir, fmt, deps, key, base, args), // no locks here: the rename still makes it safe
        }
        drop(file);
        std::thread::sleep(Duration::from_millis(20));
        if deps_hold(deps, key) {
            return true;
        }
        if started.elapsed() > WAIT {
            return build(dir, fmt, deps, key, base, args);
        }
        if !said && started.elapsed() > WAIT_SILENTLY {
            eprintln!("rstex: waiting for another run to build the format {}", String::from_utf8_lossy(base));
            said = true;
        }
    }
}

/// The build itself: in a new directory, so that nothing in the current one is read and nothing is left there.
fn build(dir: &Path, fmt: &Path, deps: &Path, key: u64, base: &[u8], args: &[Vec<u8>]) -> bool {
    let tmp = dir.join(format!("tmp-{}", std::process::id()));
    if std::fs::create_dir_all(&tmp).is_err() {
        return false;
    }
    let Ok(exe) = std::env::current_exe() else { return false };
    let as_os = rstex_engine::platform::path_from_bytes;
    let mut c = Command::new(exe);
    c.arg("-ini");
    c.arg(as_os(&[b"-jobname=", base].concat()));
    c.arg(as_os(&[b"-progname=", base].concat()));
    c.arg("-recorder");
    for a in args {
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
    let ok = c.status().is_ok_and(|s| s.success()) && built.is_file();
    if !ok {
        eprintln!("rstex: the format {} did not build: see {}", String::from_utf8_lossy(base), tmp.join(as_os(&[base, b".log"].concat())).display());
        return false;
    }
    let fls = std::fs::read(tmp.join(as_os(&[base, b".fls"].concat()))).unwrap_or_default();
    let mut list = Vec::new();
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
    let as_bytes = rstex_engine::platform::path_to_bytes;
    if let Err(e) = rstex_engine::platform::publish(&as_bytes(&built), &as_bytes(fmt)) {
        eprintln!("rstex: cannot put the format in place as {}: {}", fmt.display(), rstex_engine::platform::os_error_text(&e));
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
