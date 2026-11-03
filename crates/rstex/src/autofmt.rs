//! Format auto-build: `rstex doc.tex`, or a `pdflatex` that is rstex, with no format anywhere but TeX Live's gzip files, which this engine cannot read.
//!
//! When the command line names no format, the name from the program name, or from a `%&` line, is looked for on the search path as pdfTeX would.  A file there
//! that this build wrote is used as it stands.  Otherwise the format comes from rstex's cache directory, in a directory for this build and this tree.  It holds, beside
//! the format, a list of the file stamps of everything the format was built from, that the `-recorder` file of the build gave.  If the format is not there, or a stamp
//! differs, it is built, once, by this same executable, as `fmtutil-sys` would, without the user's own trees, in a directory of its own, and published by rename.  Runs at the same time wait for the
//! one that holds a lock, and whatever happens to that one, a run can go on and build its own: the rename is what makes that safe.  `RSTEX_FMT_VERBOSE` shows
//! the build, which is otherwise silent.
//!
//! `-ini`, `-fmt` and a first line with `&name` never come here: the engine does what they say.
use rstex_kpse as kpse;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

/// A stamp younger than this might not show a change made just after it: as `crates/kpse/src/index.rs`.
const RACY_NS: u64 = 2_000_000_000;
/// How long a run waits for another's build: pdflatex.fmt takes seconds, even on a busy machine.
const WAIT: Duration = Duration::from_secs(600);
/// A lock this old belongs to a run that died.
const STALE_LOCK: Duration = Duration::from_secs(900);
/// What a format's directory is kept for, beyond the newest few, and how many those are.
const KEEP_DIRS: usize = 4;
const OLD_DIR: Duration = Duration::from_secs(3600);
/// Variables that belong to a document, not to a format, or that would put a date of theirs in it.
const NOT_FOR_A_BUILD: [&str; 7] = ["TEXINPUTS", "TEXFORMATS", "TEXMF_OUTPUT_DIRECTORY", "KPSE_DOT", "SOURCE_DATE_EPOCH", "FORCE_SOURCE_DATE", "RSTEX_FMT_VERBOSE"];
/// A user's own trees, which the build looks at no more than `fmtutil-sys` does: a format is what the installation would have built, the oracle's is that, and a package in one of
/// these for one document would stay in the cache's format for every other.
const USER_TREES: [&str; 3] = ["TEXMFHOME", "TEXMFVAR", "TEXMFCONFIG"];
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
    if !build_once(&dir, &fmt, &deps, key, base, &args) {
        return None;
    }
    Some(rstex_engine::platform::path_to_bytes(&fmt))
}

/// True if there is a recipe for `base`, so that a `%&base` line means something though no file has that name.
pub fn can_build(base: &[u8]) -> bool {
    recipe(base).is_some()
}

/// True if this variable says nothing to a build: it is removed for it, or set to nothing.
fn not_for_a_build(name: &str) -> bool {
    // TEXINPUTS.pdflatex and TEXINPUTS_pdflatex too.
    USER_TREES.contains(&name) || NOT_FOR_A_BUILD.iter().any(|n| name == *n || name.starts_with(&format!("{n}.")) || name.starts_with(&format!("{n}_")))
}

/// What the rest of the environment can say about the tree: `TEXMF`, `TEXMFDIST`, `TFMFONTS` and the like reach the build.
fn environment_key() -> u64 {
    let mut vars: Vec<_> = std::env::vars_os()
        .filter(|(k, _)| k.to_str().is_some_and(|k| (k.starts_with("TEX") || k.starts_with("TFM")) && !not_for_a_build(k)))
        .map(|(k, v)| [rstex_engine::platform::path_to_bytes(Path::new(&k)), b"=".to_vec(), rstex_engine::platform::path_to_bytes(Path::new(&v)), b"\n".to_vec()].concat())
        .collect();
    vars.sort();
    kpse::hash64(&vars.concat())
}

/// The directory for this build, this tree and what the environment says of it.
fn format_dir() -> Option<PathBuf> {
    let tree = kpse::with_global(|k| k.tree_key());
    Some(kpse::cache_dir()?.join(format!("fmt-{}-{tree:016x}-{:016x}", rstex_engine::image::FMT_BUILD_ID, environment_key())))
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
    if std::fs::create_dir_all(dir).is_err() {
        return false;
    }
    let lock = deps.with_extension("lock");
    let started = std::time::Instant::now();
    loop {
        match std::fs::OpenOptions::new().write(true).create_new(true).open(&lock) {
            Ok(_) => {
                // Between the check that sent us here and the lock, another run may have finished.
                let ok = deps_hold(deps, key) || build(dir, fmt, deps, key, base, args);
                let _ = std::fs::remove_file(&lock);
                return ok;
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(_) => return build(dir, fmt, deps, key, base, args), // no locks here: the rename still makes it safe
        }
        std::thread::sleep(Duration::from_millis(20));
        if deps_hold(deps, key) {
            return true;
        }
        let age = std::fs::metadata(&lock).and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok());
        if age.is_some_and(|a| a > STALE_LOCK) {
            let _ = std::fs::remove_file(&lock);
        } else if started.elapsed() > WAIT {
            return build(dir, fmt, deps, key, base, args);
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
        if not_for_a_build(&k.to_string_lossy()) {
            c.env_remove(&k);
        }
    }
    // Not to their values in `texmf.cnf`, which name the user's own trees too.
    for t in USER_TREES {
        c.env(t, tmp.join("no-such-tree"));
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
