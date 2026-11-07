//! Format auto-build: `rstex doc.tex`, or a `pdflatex` that is rstex, with no format anywhere but TeX Live's gzip files, which this engine cannot read.
//!
//! Whatever names the format, the program name, a `%&` line, `-fmt`, or `&name` on the command line or at the `**` prompt, as for `-ini &pdflatex` of mylatexformat, the name is
//! looked for on the search path as pdfTeX would.  A file there that this build wrote is used as it stands, and so is one with a directory in its name.  Otherwise the
//! format comes from rstex's cache directory, in a directory for this build and this tree, if there is a recipe for the name.  It holds, beside
//! the format, a list of the file stamps of everything the format was built from, that the `-recorder` file of the build gave.  If the format is not there, or a stamp
//! differs, it is built, once, by this same executable, as `fmtutil-sys` would, without the user's own trees, in a directory of its own, and published by rename.  Runs at the same time wait for the
//! one that holds a lock on a file.  The system lets go of that when the holder ends, however it ends, so Control-C during a first run costs the next one nothing, and whatever
//! happens otherwise, a run can go on and build its own: the rename is what makes that safe.  A build says `rstex: building the format NAME` on stderr, and nothing goes to the job's log or its stdout.
//! `RSTEX_FMT_VERBOSE` shows the build itself as well, which is otherwise silent.
//!
//! `maininit` asks here about the name that §550 looks for first, before a checkpoint is looked for, and the engine about any other it comes to: `World::lookup_format`.
//! `-ini` with no `&name` and no `%&` line loads no format, and asks nothing.
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

/// What makes this program `rstex`.
pub fn engine_prefix() -> &'static [&'static str] {
    ENGINE_PREFIX.get().copied().unwrap_or(&[])
}

/// A stamp younger than this might not show a change made just after it: as `crates/kpse/src/index.rs`.
const RACY_NS: u64 = 2_000_000_000;
/// How long a run waits for another's build, which is alive, since it holds the lock: pdflatex.fmt takes seconds, even on a busy machine.
const WAIT: Duration = Duration::from_secs(600);
/// A wait longer than this gets a line on stderr: it might not be what anyone expects.
const WAIT_SILENTLY: Duration = Duration::from_secs(10);
/// What a format's directory is kept for, beyond the newest few, and how many those are.
const KEEP_DIRS: usize = 4;
/// `derived`.
const DERIVED_RECHECK: Duration = Duration::from_secs(60);
const OLD_DIR: Duration = Duration::from_secs(3600);
/// Variables that belong to a document, not to a format, or that would put a date of theirs in it.
const NOT_FOR_A_BUILD: [&str; 7] = ["TEXINPUTS", "TEXFORMATS", "TEXMF_OUTPUT_DIRECTORY", "KPSE_DOT", "SOURCE_DATE_EPOCH", "FORCE_SOURCE_DATE", "RSTEX_FMT_VERBOSE"];
/// A user's own trees, which the build looks at no more than `fmtutil-sys` does: a format is what the installation would have built, the oracle's is that, and a package in one of
/// these for one document would stay in the cache's format for every other.
const USER_TREES: [&str; 3] = ["TEXMFHOME", "TEXMFVAR", "TEXMFCONFIG"];
/// For a tree with no `fmtutil.cnf`.  The first argument of each is what `fmtutil` gives every format: -ini, -jobname and -progname come first.
const BUILTIN_RECIPES: [(&[u8], &str); 2] = [(b"pdflatex", "*pdflatex.ini"), (b"pdftex", "*pdfetex.ini")];

/// The format to use for `name`, with or without its `.fmt`, if not the one that `name` finds on the search path: its path in the cache, built now if need be.  `None` leaves it to the engine.
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
    if let Some(dir) = format_dir() {
        let fmt = dir.join(rstex_engine::platform::path_from_bytes(&with_ext));
        let deps = dir.join(rstex_engine::platform::path_from_bytes(&[base, b".deps"].concat()));
        let key = kpse::hash64(&with_ext);
        if deps_hold(&deps, key) {
            return Some(rstex_engine::platform::path_to_bytes(&fmt));
        }
        let args = recipe(base)?;
        let job = Build { dir: &dir, fmt: &fmt, deps: &deps, key, name: base, progname: base, args: &args, also: &[], driver: None };
        match build_once(&job) {
            Built::Yes => return Some(rstex_engine::platform::path_to_bytes(&fmt)),
            Built::Failed => return None,
            Built::NoRoom => {}
        }
    }
    build_for_this_run(base, &recipe(base)?)
}

/// A format made by running `driver` on the format `base`, which `resolve` finds: its path, built now if need be, after a word from `before_build`.  Its list holds what `base` was made from as well, so that
/// one look at it is enough on the usual run.  That look is some 200 `stat`s, a tenth of the time of a run of `commute`, which may well be in a loop, so it is taken once in `DERIVED_RECHECK`, which is how long
/// a change to the tree can go unseen.  The list's own modification time says when it last held.  With no cache it can be kept in, there is none: `commute` then says so.
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
    if build_once(&job) != Built::Yes {
        return None;
    }
    Some(rstex_engine::platform::path_to_bytes(&fmt))
}

/// What came of a build.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Built {
    Yes,
    /// The format does not build: the run has said so.
    Failed,
    /// The cache directory cannot be written.
    NoRoom,
}

/// No cache directory to keep a format in, or none that can be written: built for this run alone, in a new directory beside its outputs, since a document must compile
/// whatever state the cache is in.  The directory is made here with mode 0700, and that fails if the name is there, so even in a shared place no one else can reach into it.
/// The run removes it as soon as it has the format: `World::format_scratch`.
fn build_for_this_run(base: &[u8], args: &[Vec<u8>]) -> Option<Vec<u8>> {
    let near = kpse::var_value(b"TEXMF_OUTPUT_DIRECTORY").map_or_else(|| PathBuf::from("."), |d| rstex_engine::platform::path_from_bytes(&d));
    let name = format!(".rstex-format-{}-{}", std::process::id(), kpse::now_ns());
    let mut dir = std::env::current_dir().ok()?.join(near).join(&name);
    if !rstex_engine::platform::can_work_in(&dir) {
        dir = std::env::temp_dir().join(name);
    }
    let mut b = std::fs::DirBuilder::new();
    rstex_engine::platform::dir_builder_private(&mut b);
    if let Err(e) = b.create(&dir) {
        eprintln!("rstex: no place to build the format {}: no cache directory can be used, and {}: {}", String::from_utf8_lossy(base), dir.display(), rstex_engine::platform::os_error_text(&e));
        return None;
    }
    if verbose() {
        eprintln!("rstex: no cache directory can be used: this run builds the format {} for itself", String::from_utf8_lossy(base));
    }
    let fmt = dir.join(rstex_engine::platform::path_from_bytes(&[base, b".fmt"].concat()));
    if !run_build(&dir, base, base, args) {
        return None;
    }
    *SCRATCH.lock().unwrap_or_else(|e| e.into_inner()) = Some(rstex_engine::platform::path_to_bytes(&dir));
    Some(rstex_engine::platform::path_to_bytes(&fmt))
}

/// The directory of [`build_for_this_run`], for the run to remove.
static SCRATCH: std::sync::Mutex<Option<Vec<u8>>> = std::sync::Mutex::new(None);

/// See [`SCRATCH`].
pub fn take_scratch() -> Option<Vec<u8>> {
    SCRATCH.lock().unwrap_or_else(|e| e.into_inner()).take()
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
    let Some(f) = kpse::CachedFile::open(deps, key) else {
        if verbose() && deps.exists() {
            eprintln!("rstex: the list {} is there and cannot be used", deps.display());
        }
        return false;
    };
    let mut b = f.front();
    while !b.is_empty() {
        let Some((head, rest)) = b.split_at_checked(44) else { return false };
        let st: kpse::Stamp = core::array::from_fn(|i| u64::from_le_bytes(head[i * 8..i * 8 + 8].try_into().unwrap()));
        let len = u32::from_le_bytes(head[40..44].try_into().unwrap()) as usize;
        let Some((path, rest)) = rest.split_at_checked(len) else { return false };
        let now = kpse::file_stamp(path);
        if now != st {
            if verbose() {
                eprintln!("rstex: {} has changed since the format was built: {st:?} is now {now:?}", String::from_utf8_lossy(path));
            }
            return false;
        }
        b = rest;
    }
    !f.front().is_empty()
}

fn verbose() -> bool {
    std::env::var_os("RSTEX_FMT_VERBOSE").is_some_and(|v| !v.is_empty())
}

/// Why the cache cannot have this format.  The run goes on without it, so this is said only when asked for.
fn no_room(why: &str) {
    if verbose() {
        eprintln!("rstex: {why}");
    }
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

/// Builds the format unless another run does it first.
fn build_once(job: &Build<'_>) -> Built {
    let (dir, deps, key, base) = (job.dir, job.deps, job.key, job.name);
    if let Err(e) = std::fs::create_dir_all(dir) {
        no_room(&format!("cannot make the directory {} for the format: {}", dir.display(), rstex_engine::platform::os_error_text(&e)));
        return Built::NoRoom;
    }
    let lock = deps.with_extension("lock");
    let started = std::time::Instant::now();
    let mut said = false;
    loop {
        let file = std::fs::OpenOptions::new().write(true).create(true).truncate(false).open(&lock);
        match file.as_ref().map(std::fs::File::try_lock) {
            Ok(Ok(())) => {
                // Between the check that sent us here and the lock, another run may have finished.
                let built = if deps_hold(deps, key) { Built::Yes } else { build(job) };
                // Another run may have this very file open, and get a lock on it when it is gone: a build too many at the worst.
                let _ = std::fs::remove_file(&lock);
                return built;
            }
            Ok(Err(std::fs::TryLockError::WouldBlock)) => {}
            _ => return build(job), // no locks here: the rename still makes it safe
        }
        drop(file);
        std::thread::sleep(Duration::from_millis(20));
        if deps_hold(deps, key) {
            return Built::Yes;
        }
        if started.elapsed() > WAIT {
            return build(job);
        }
        if !said && started.elapsed() > WAIT_SILENTLY {
            eprintln!("rstex: waiting for another run to build the format {}", String::from_utf8_lossy(base));
            said = true;
        }
    }
}

/// This same program, building the format in the directory `tmp`, which holds nothing else.  True if the format is there.  Otherwise it says so, and leaves the log there.
fn run_build(tmp: &Path, base: &[u8], progname: &[u8], args: &[Vec<u8>]) -> bool {
    let Ok(exe) = std::env::current_exe() else { return false };
    let as_os = rstex_engine::platform::path_from_bytes;
    let mut c = Command::new(exe);
    c.args(ENGINE_PREFIX.get().copied().unwrap_or(&[]));
    c.arg("-ini");
    c.arg(as_os(&[b"-jobname=", base].concat()));
    c.arg(as_os(&[b"-progname=", progname].concat()));
    c.arg("-recorder");
    for a in args {
        c.arg(as_os(a));
    }
    c.current_dir(&tmp).stdin(Stdio::null());
    // A program that has run this one may want to say why this run is a long one: README.md has this line.  commute has a word of its own.
    if verbose() || ENGINE_PREFIX.get().is_none() {
        eprintln!("rstex: building the format {}", String::from_utf8_lossy(base));
    }
    if !verbose() {
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
    let built = tmp.join(as_os(&[base, b".fmt"].concat()));
    let ok = c.status().is_ok_and(|s| s.success()) && built.is_file();
    if !ok {
        eprintln!("rstex: the format {} did not build: see {}", String::from_utf8_lossy(base), tmp.join(as_os(&[base, b".log"].concat())).display());
    }
    ok
}

/// True if a file the build read says something about the format.  A relative name is the build's own, in its directory.  A device is no file: LaTeX reads `/dev/null`, whose
/// times on macOS are those of the last write by anyone, so that a format listing it would never hold, and would not even be listed as settled.
fn goes_on_the_list(path: &[u8]) -> bool {
    rstex_engine::platform::is_absolute(path) && !std::fs::metadata(rstex_engine::platform::path_from_bytes(path)).is_ok_and(|m| !m.is_file() && !m.is_dir())
}

/// The build itself: in a new directory, so that nothing in the current one is read and nothing is left there.
fn build(job: &Build<'_>) -> Built {
    let (dir, fmt, deps, key, base) = (job.dir, job.fmt, job.deps, job.key, job.name);
    let mut tmp = dir.join(format!("tmp-{}", std::process::id()));
    // A cache under a long name, which a user's home and a project's depth can come to on Windows, is no working directory for a process, though all else takes it.  So elsewhere, and the file comes here.
    // Not for a format that starts from another by a name relative to its directory.
    let apart = !rstex_engine::platform::can_work_in(&tmp);
    if apart {
        if job.driver.is_some() {
            return Built::NoRoom;
        }
        tmp = std::env::temp_dir().join(format!("rstex-format-{}-{}", std::process::id(), kpse::now_ns()));
    }
    let _ = std::fs::remove_dir_all(&tmp); // of a run that died with this process id
    if std::fs::create_dir_all(&tmp).is_err() {
        return Built::NoRoom;
    }
    if let Some(d) = job.driver {
        if std::fs::write(tmp.join("driver.tex"), d).is_err() {
            return Built::NoRoom;
        }
    }
    let as_os = rstex_engine::platform::path_from_bytes;
    let begun = kpse::now_ns();
    if !run_build(&tmp, base, job.progname, job.args) {
        return Built::Failed;
    }
    let built = tmp.join(as_os(&[base, b".fmt"].concat()));
    let fls = std::fs::read(tmp.join(as_os(&[base, b".fls"].concat()))).unwrap_or_default();
    let mut list = job.also.to_vec();
    let mut settled = true;
    for line in fls.split(|&c| c == b'\n') {
        let Some(path) = line.strip_prefix(b"INPUT ") else { continue };
        if !goes_on_the_list(path) {
            continue;
        }
        let st = kpse::file_stamp(path);
        let quiet = st[2].max(st[3]) + RACY_NS < begun;
        if !quiet && verbose() {
            eprintln!("rstex: {} changed around this build, so the next run builds again", String::from_utf8_lossy(path));
        }
        settled &= quiet;
        put_dep(&mut list, path, &st);
    }
    // Its stamp is about to be trusted, and it has no sum: its bytes reach the disk before its name does, or a crash here leaves a format of zeros with a stamp that holds.
    let _ = std::fs::OpenOptions::new().write(true).open(&built).and_then(|f| f.sync_all());
    let as_bytes = rstex_engine::platform::path_to_bytes;
    let built = if apart {
        let here = dir.join(format!("tmp-{}.fmt", std::process::id()));
        if let Err(e) = std::fs::copy(&built, &here) {
            no_room(&format!("cannot put the format in {}: {}", here.display(), rstex_engine::platform::os_error_text(&e)));
            let _ = std::fs::remove_dir_all(&tmp);
            return Built::NoRoom;
        }
        here
    } else {
        built
    };
    if let Err(e) = rstex_engine::platform::publish(&as_bytes(&built), &as_bytes(fmt)) {
        no_room(&format!("cannot put the format in place as {}: {}", fmt.display(), rstex_engine::platform::os_error_text(&e)));
        return Built::NoRoom;
    }
    put_dep(&mut list, &rstex_engine::platform::path_to_bytes(fmt), &kpse::file_stamp(&rstex_engine::platform::path_to_bytes(fmt)));
    // Without the list the next run builds again, which is right for what has only just changed.  This run goes on, with what it has just built.
    if settled {
        kpse::publish_path(deps, &kpse::make_cached_file(key, &list, &[]));
    }
    let _ = std::fs::remove_dir_all(&tmp);
    prune(dir);
    Built::Yes
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_device_says_nothing_about_a_format() {
        let exe = std::env::current_exe().unwrap();
        assert!(goes_on_the_list(&rstex_engine::platform::path_to_bytes(&exe)));
        assert!(!goes_on_the_list(b"./texsys.aux"));
        #[cfg(unix)]
        assert!(!goes_on_the_list(b"/dev/null"));
    }
}
