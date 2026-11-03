//! Everything specific to an operating system in this crate (DESIGN.md section 12).
//!
//! File names are bytes, as TeX sees them: the name itself on Unix, UTF-8 on Windows.
use std::ffi::{OsStr, OsString};
use std::path::PathBuf;

/// TeX sees file names as bytes: on Unix they are the name, on Windows UTF-8.
#[cfg(unix)]
pub fn path_of(p: &[u8]) -> PathBuf {
    use std::os::unix::ffi::OsStrExt;
    PathBuf::from(OsStr::from_bytes(p))
}

#[cfg(windows)]
pub fn path_of(p: &[u8]) -> PathBuf {
    PathBuf::from(String::from_utf8_lossy(p).as_ref())
}

#[cfg(unix)]
pub fn os_bytes(s: &OsStr) -> Vec<u8> {
    use std::os::unix::ffi::OsStrExt;
    s.as_bytes().to_vec()
}

#[cfg(windows)]
pub fn os_bytes(s: &OsStr) -> Vec<u8> {
    s.to_string_lossy().into_owned().into_bytes()
}

#[cfg(unix)]
fn os_string(p: &[u8]) -> OsString {
    use std::os::unix::ffi::OsStrExt;
    OsStr::from_bytes(p).to_os_string()
}

#[cfg(windows)]
fn os_string(p: &[u8]) -> OsString {
    OsString::from(String::from_utf8_lossy(p).as_ref())
}

/// `getenv`.  A name std would refuse (empty, or holding `=` or NUL) is simply unset.
pub fn getenv(name: &[u8]) -> Option<Vec<u8>> {
    if name.is_empty() || name.contains(&b'=') || name.contains(&0) {
        return None;
    }
    std::env::var_os(os_string(name)).map(|v| os_bytes(&v))
}

/// The environment variable holding the home directory: tilde.c's `HOMEVAR`.
pub const HOME_VAR: &[u8] = if cfg!(windows) { b"USERPROFILE" } else { b"HOME" };

/// `errno == ENAMETOOLONG`.
fn name_too_long(e: &std::io::Error) -> bool {
    #[cfg(target_os = "linux")]
    const ENAMETOOLONG: i32 = 36;
    #[cfg(all(unix, not(target_os = "linux")))]
    const ENAMETOOLONG: i32 = 63;
    #[cfg(windows)]
    const ENAMETOOLONG: i32 = 206; // ERROR_FILENAME_EXCED_RANGE
    e.raw_os_error() == Some(ENAMETOOLONG)
}

/// What readable.c's `READABLE` answered.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Readable {
    Yes,
    No,
    /// `errno` was ENAMETOOLONG: the caller truncates and tries once more.
    TooLong,
}

/// readable.c's `READABLE`: `access (fn, R_OK) == 0 && stat (fn) == 0 && !S_ISDIR`.
///
/// `std` has no `access`.  A miss costs one `stat`, as in C, and that is by far the common case.  A
/// regular file is opened and closed, which asks the kernel the same question as `access` (for the
/// effective rather than the real user, which differ only in a setuid program).  A fifo or a device
/// must not be opened, since that could block, so its mode bits decide.
#[cfg(unix)]
pub fn readable(name: &[u8]) -> Readable {
    use std::os::unix::fs::MetadataExt;
    let path = path_of(name);
    let md = match std::fs::metadata(&path) {
        Ok(m) => m,
        Err(e) if name_too_long(&e) => return Readable::TooLong,
        Err(_) => return Readable::No,
    };
    if md.is_dir() {
        return Readable::No;
    }
    let ok = if md.is_file() { std::fs::File::open(&path).is_ok() } else { md.mode() & 0o444 != 0 };
    if ok { Readable::Yes } else { Readable::No }
}

/// readable.c's `READABLE` on Windows: `GetFileAttributesW` and not a directory.
#[cfg(windows)]
pub fn readable(name: &[u8]) -> Readable {
    match std::fs::metadata(path_of(name)) {
        Ok(m) => {
            if m.is_dir() {
                Readable::No
            } else {
                Readable::Yes
            }
        }
        Err(e) if name_too_long(&e) => Readable::TooLong,
        Err(_) => Readable::No,
    }
}

/// `fopen(name, FOPEN_A_MODE)`.
pub fn open_append(name: &[u8]) -> Option<std::fs::File> {
    std::fs::OpenOptions::new().append(true).create(true).open(path_of(name)).ok()
}

/// How tex-make.c's `maketex` runs a script: no standard input, its standard output is the answer, and stderr is ours or nowhere.  On Unix
/// it is found on the `PATH`, and on Windows beside the program, as `_spawnvp` of `fullbin`.  If it cannot start, `perror` of the child,
/// where its stderr goes, and an empty answer.
pub fn run_mktex(args: &[Vec<u8>], env: &[(Vec<u8>, Vec<u8>)], discard_errors: bool, selfautoloc: &[u8]) -> Vec<u8> {
    use std::io::Write;
    use std::process::{Command, Stdio};
    let prog = if cfg!(windows) { [selfautoloc, b"/", &args[0][..]].concat() } else { args[0].clone() };
    let mut c = Command::new(path_of(&prog));
    c.args(args[1..].iter().map(|a| os_string(a))).stdin(Stdio::null()).stdout(Stdio::piped());
    c.stderr(if discard_errors { Stdio::null() } else { Stdio::inherit() });
    for (name, value) in env {
        c.env(os_string(name), os_string(value));
    }
    match c.output() {
        Ok(o) => o.stdout,
        Err(e) => {
            if !discard_errors {
                // strerror, without what Rust adds.
                let text = e.to_string();
                let text = text.split(" (os error").next().unwrap_or(&text);
                let m = [&args[0][..], b": ", text.as_bytes(), b"\n"].concat();
                let _ = std::io::stderr().write_all(&m);
            }
            Vec::new()
        }
    }
}

/// `kpathsea_dir_p`: a directory, or a symbolic link to one.
pub fn is_dir(name: &[u8]) -> bool {
    std::fs::metadata(path_of(name)).map(|m| m.is_dir()).unwrap_or(false)
}

/// The number of links of a directory, for elt-dirs.c's `st_nlink` leaf test.  0 means unknown.
#[cfg(unix)]
pub fn dir_nlink(name: &[u8]) -> u64 {
    use std::os::unix::fs::MetadataExt;
    std::fs::metadata(path_of(name)).map(|m| m.nlink()).unwrap_or(0)
}

/// Windows has no such count: elt-dirs.c does not use `ST_NLINK_TRICK` there.
#[cfg(windows)]
pub fn dir_nlink(_name: &[u8]) -> u64 {
    0
}

/// True if a directory entry is a directory or a symbolic link to one.  Unlike C this needs a `stat`
/// only for a symbolic link, because the entry's type is already known.
pub fn entry_is_dir(e: &std::fs::DirEntry) -> bool {
    match e.file_type() {
        Ok(t) if t.is_dir() => true,
        Ok(t) if t.is_symlink() => std::fs::metadata(e.path()).map(|m| m.is_dir()).unwrap_or(false),
        Ok(_) => false,
        Err(_) => std::fs::metadata(e.path()).map(|m| m.is_dir()).unwrap_or(false),
    }
}

/// db.c's `same_file_p`: the same device and inode.
#[cfg(unix)]
pub fn same_file(a: &[u8], b: &[u8]) -> bool {
    use std::os::unix::fs::MetadataExt;
    match (std::fs::metadata(path_of(a)), std::fs::metadata(path_of(b))) {
        (Ok(x), Ok(y)) => x.dev() == y.dev() && x.ino() == y.ino(),
        _ => false,
    }
}

/// On Windows two names that differ only in case, which is the only case the caller asks about,
/// name one file.
#[cfg(windows)]
pub fn same_file(a: &[u8], b: &[u8]) -> bool {
    std::fs::metadata(path_of(a)).is_ok() && std::fs::metadata(path_of(b)).is_ok()
}

/// `getpwnam (user)->pw_dir`, by reading `/etc/passwd`, since a `getpwnam` call would need `unsafe`.
/// A user known only to a directory service counts as unknown, and tilde.c then uses `.`.
#[cfg(unix)]
pub fn home_of_user(user: &[u8]) -> Option<Vec<u8>> {
    let text = std::fs::read("/etc/passwd").ok()?;
    for line in text.split(|&c| c == b'\n') {
        let mut fields = line.split(|&c| c == b':');
        if fields.next() == Some(user) {
            return fields.nth(4).map(|d| d.to_vec());
        }
    }
    None
}

/// Windows has no user database to ask here.
#[cfg(windows)]
pub fn home_of_user(_user: &[u8]) -> Option<Vec<u8>> {
    None
}

/// The first executable `prog` along `PATH`, without running anything.
pub fn find_on_path(prog: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    #[cfg(windows)]
    const SUFFIXES: &[&str] = &[".exe", ".cmd", ".bat", ".com"];
    #[cfg(unix)]
    const SUFFIXES: &[&str] = &[""];
    for dir in std::env::split_paths(&path) {
        for s in SUFFIXES {
            let candidate = dir.join(format!("{prog}{s}"));
            if std::fs::metadata(&candidate).map(|m| m.is_file()).unwrap_or(false) {
                return Some(candidate);
            }
        }
    }
    None
}

/// The value of `PATH`, as bytes, for keying the tree cache.
pub fn path_var() -> Vec<u8> {
    std::env::var_os("PATH").map(|v| os_bytes(&v)).unwrap_or_default()
}

/// Size and modification time, to notice that a binary was replaced.
pub fn stamp(path: &std::path::Path) -> Option<(u64, u64)> {
    let md = std::fs::metadata(path).ok()?;
    let t = md.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?;
    Some((md.len(), t.as_secs()))
}

/// Runs `prog args...` and returns its standard output.  `TEXMFCNF` and `KPSE_DOT` are removed, because
/// the question is what the program would use by default, and the answer gets cached.
pub fn capture(prog: &std::path::Path, args: &[&str]) -> Option<Vec<u8>> {
    let out = std::process::Command::new(prog)
        .args(args)
        .env_remove("TEXMFCNF")
        .env_remove("KPSE_DOT")
        .env("KPATHSEA_WARNING", "0")
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    Some(out.stdout)
}

/// The directory for rstex's caches, as in DESIGN.md section 12 and the engine's `platform`.
pub fn cache_dir() -> Option<PathBuf> {
    fn nonempty(name: &[u8]) -> Option<Vec<u8>> {
        getenv(name).filter(|v| !v.is_empty())
    }
    #[cfg(target_os = "macos")]
    {
        let home = nonempty(b"HOME")?;
        Some(path_of(&home).join("Library/Caches/rstex"))
    }
    #[cfg(windows)]
    {
        let base = nonempty(b"LOCALAPPDATA")?;
        Some(path_of(&base).join("rstex").join("cache"))
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        if let Some(x) = nonempty(b"XDG_CACHE_HOME") {
            return Some(path_of(&x).join("rstex"));
        }
        let home = nonempty(b"HOME")?;
        Some(path_of(&home).join(".cache/rstex"))
    }
}

/// Publishes a cache file by temporary and rename.  Failure is harmless: the answer is asked again.
pub fn publish_cache(path: &std::path::Path, data: &[u8]) {
    let Some(dir) = path.parent() else { return };
    if std::fs::create_dir_all(dir).is_err() {
        return;
    }
    let tmp = path.with_extension(format!("tmp{}", std::process::id()));
    if std::fs::write(&tmp, data).is_err() {
        return;
    }
    // A reader may hold the old file open on Windows; try a few times.
    for _ in 0..if cfg!(windows) { 5 } else { 1 } {
        if std::fs::rename(&tmp, path).is_ok() {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let _ = std::fs::remove_file(&tmp);
}

/// Well-known places for a TeX tree, in order of preference: each is a directory that would hold
/// `web2c/texmf.cnf`.  `texlive_roots` lists parents of year directories, newest year first.
pub fn well_known_trees() -> Vec<Vec<u8>> {
    let mut out: Vec<Vec<u8>> = Vec::new();
    let mut years = |root: &[u8]| {
        let Ok(rd) = std::fs::read_dir(path_of(root)) else { return };
        let mut ys: Vec<Vec<u8>> = rd
            .filter_map(|e| e.ok())
            .map(|e| os_bytes(&e.file_name()))
            .filter(|n| !n.is_empty() && n.iter().all(u8::is_ascii_digit))
            .collect();
        ys.sort();
        for y in ys.iter().rev() {
            let mut d = root.to_vec();
            d.push(b'/');
            d.extend_from_slice(y);
            d.extend_from_slice(b"/texmf-dist");
            out.push(d);
        }
    };
    #[cfg(windows)]
    {
        let drive = getenv(b"SystemDrive").unwrap_or_else(|| b"C:".to_vec());
        let mut root = drive;
        root.extend_from_slice(b"/texlive");
        years(&root);
        if let Some(mut d) = getenv(b"APPDATA") {
            d.extend_from_slice(b"/TinyTeX/texmf-dist");
            out.push(d);
        }
    }
    #[cfg(unix)]
    {
        years(b"/usr/local/texlive");
        years(b"/opt/texlive");
        let home = getenv(b"HOME").unwrap_or_default();
        #[cfg(target_os = "macos")]
        {
            out.push(b"/Library/TeX/Root/texmf-dist".to_vec());
            let mut d = home.clone();
            d.extend_from_slice(b"/Library/TinyTeX/texmf-dist");
            out.push(d);
        }
        #[cfg(not(target_os = "macos"))]
        {
            let mut d = home.clone();
            d.extend_from_slice(b"/.TinyTeX/texmf-dist");
            out.push(d);
        }
        out.push(b"/usr/share/texmf-dist".to_vec());
        out.push(b"/usr/share/texlive/texmf-dist".to_vec());
        out.push(b"/usr/share/texmf".to_vec());
    }
    out
}

/// Well-known MiKTeX installation directories, in case `initexmf` is not on `PATH`.
pub fn well_known_miktex() -> Vec<Vec<u8>> {
    #[allow(unused_mut)]
    let mut out = Vec::new();
    #[cfg(windows)]
    {
        if let Some(mut d) = getenv(b"LOCALAPPDATA") {
            d.extend_from_slice(b"/Programs/MiKTeX");
            out.push(d);
        }
        if let Some(mut d) = getenv(b"ProgramFiles") {
            d.extend_from_slice(b"/MiKTeX");
            out.push(d);
        }
    }
    #[cfg(target_os = "macos")]
    {
        out.push(b"/Applications/MiKTeX Console.app/Contents/Resources/miktex-dist".to_vec());
    }
    out
}
