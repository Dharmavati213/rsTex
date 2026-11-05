//! Everything specific to an operating system in this crate (DESIGN.md section 12).
//!
//! File names are bytes, as TeX sees them: the name itself on Unix, on Windows WTF-8, that is UTF-8 and the surrogates that a name can hold alone: [`crate::names::bytes_to_wide`].
use std::ffi::{OsStr, OsString};
use std::path::PathBuf;

/// What an index compares to see that a path is as it was: mode and existence, size, modification time, change time, identity.  All 0 if there is no such file.
pub type Stamp = [u64; 5];

/// Now `path`'s [`Stamp`].  It follows symbolic links, as the reads do.
#[cfg(unix)]
pub fn file_stamp(path: &[u8]) -> Stamp {
    use std::os::unix::fs::MetadataExt;
    match std::fs::metadata(path_of(path)) {
        Ok(m) => [
            1 + ((m.mode() as u64) << 8),
            m.size(),
            (m.mtime() as u64).wrapping_mul(1_000_000_000).wrapping_add(m.mtime_nsec() as u64),
            (m.ctime() as u64).wrapping_mul(1_000_000_000).wrapping_add(m.ctime_nsec() as u64),
            m.ino(),
        ],
        Err(_) => [0; 5],
    }
}

/// Now `path`'s [`Stamp`], DESIGN.md section 12: Windows has size and modification time.
#[cfg(windows)]
pub fn file_stamp(path: &[u8]) -> Stamp {
    match std::fs::metadata(path_of(path)) {
        Ok(m) => {
            let t = m.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_nanos() as u64);
            [1 + ((m.is_dir() as u64) << 8), m.len(), t, 0, 0]
        }
        Err(_) => [0; 5],
    }
}

/// While an index is being made, every path this crate looks at, with its stamp from before it looked.
#[derive(Default)]
pub struct Seen {
    pub list: Vec<(Vec<u8>, Stamp)>,
    known: std::collections::HashSet<Vec<u8>>,
}

thread_local! {
    static SEEN: std::cell::RefCell<Option<Seen>> = const { std::cell::RefCell::new(None) };
}

/// From now on, [`note`] keeps what it is told.
pub fn observe_start() {
    SEEN.with(|s| *s.borrow_mut() = Some(Seen::default()));
}

/// What was noted so far, and no more of it.
pub fn observe_take() -> Vec<(Vec<u8>, Stamp)> {
    SEEN.with(|s| s.borrow_mut().take()).map_or(Vec::new(), |s| s.list)
}

/// This crate is about to look at `path`.  The stamp comes first: if the file changes after it, the next run sees a stale stamp and that is safe.
pub fn note(path: &[u8]) {
    SEEN.with(|s| {
        if let Some(seen) = s.borrow_mut().as_mut() {
            if seen.known.insert(path.to_vec()) {
                seen.list.push((path.to_vec(), file_stamp(path)));
            }
        }
    });
}

/// `std::fs::read_dir`, noted.
pub fn read_dir(path: &[u8]) -> std::io::Result<std::fs::ReadDir> {
    note(path);
    std::fs::read_dir(path_of(path))
}

/// `std::fs::read`, noted.
pub fn read_file(path: &[u8]) -> std::io::Result<Vec<u8>> {
    note(path);
    std::fs::read(path_of(path))
}

/// Nanoseconds since the epoch.
pub fn now_ns() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos() as u64)
}

/// The bytes of a whole file, for reading only: mapped, where nothing is paid for pages not touched, or read.
pub struct FileBytes {
    #[cfg(unix)]
    map: Option<unix_map::Map>,
    data: Vec<u8>,
    /// When the file was last written, or marked as used, where it comes from a file.
    pub modified: Option<std::time::SystemTime>,
}

impl FileBytes {
    /// Bytes made here.
    pub fn from_vec(data: Vec<u8>) -> FileBytes {
        FileBytes {
            #[cfg(unix)]
            map: None,
            data,
            modified: None,
        }
    }

    /// The contents of `path` in the cache directory, if [`open_private`] takes it.
    #[cfg(unix)]
    pub fn open(path: &std::path::Path) -> Option<FileBytes> {
        use std::os::fd::AsRawFd;
        let (f, md) = open_private_with(path)?;
        let len = md.len() as usize;
        if len == 0 {
            return Some(FileBytes::from_vec(Vec::new()));
        }
        Some(FileBytes { map: Some(unix_map::Map::new(f.as_raw_fd(), len)?), data: Vec::new(), modified: md.modified().ok() })
    }

    /// The contents of `path` in the cache directory, if [`open_private`] takes it.  A mapping would hold the file open against the rename that replaces it.
    #[cfg(windows)]
    pub fn open(path: &std::path::Path) -> Option<FileBytes> {
        use std::io::Read;
        let (mut f, md) = open_private_with(path)?;
        let mut data = Vec::new();
        f.read_to_end(&mut data).ok()?;
        let mut b = FileBytes::from_vec(data);
        b.modified = md.modified().ok();
        Some(b)
    }

    #[inline]
    pub fn bytes(&self) -> &[u8] {
        #[cfg(unix)]
        if let Some(m) = &self.map {
            return m.bytes();
        }
        &self.data
    }
}

/// The only `unsafe` of this crate.
#[cfg(unix)]
#[allow(unsafe_code)]
mod unix_map {
    /// `RLIMIT_FSIZE`: past it a write gets SIGXFSZ, which ends the process.
    pub fn file_size_limit() -> u64 {
        let mut r = libc::rlimit { rlim_cur: libc::RLIM_INFINITY, rlim_max: libc::RLIM_INFINITY };
        // SAFETY: `r` is valid for the call, and on a failure stays what it was.
        unsafe {
            libc::getrlimit(libc::RLIMIT_FSIZE, &mut r);
        }
        r.rlim_cur as u64
    }

    pub fn euid() -> u32 {
        // SAFETY: no arguments to get wrong.
        unsafe { libc::geteuid() }
    }

    /// A private read-only mapping of a whole file, which is never written while it is mapped: index files are published by rename.
    pub struct Map {
        p: *mut libc::c_void,
        len: usize,
    }

    // SAFETY: nothing writes through `p`, and the mapping lives as long as this value.
    unsafe impl Send for Map {}

    impl Map {
        pub fn new(fd: i32, len: usize) -> Option<Map> {
            // SAFETY: a new mapping with no address hint; the result is checked.
            let p = unsafe { libc::mmap(core::ptr::null_mut(), len, libc::PROT_READ, libc::MAP_PRIVATE, fd, 0) };
            if p == libc::MAP_FAILED { None } else { Some(Map { p, len }) }
        }

        #[inline]
        pub fn bytes(&self) -> &[u8] {
            // SAFETY: `p..p+len` is mapped and readable until `drop`.
            unsafe { core::slice::from_raw_parts(self.p as *const u8, self.len) }
        }
    }

    impl Drop for Map {
        fn drop(&mut self) {
            // SAFETY: this value is the mapping's only owner, and no slice of it outlives it.
            unsafe {
                libc::munmap(self.p, self.len);
            }
        }
    }
}

/// TeX sees file names as bytes: see the top of this file.
#[cfg(unix)]
pub fn path_of(p: &[u8]) -> PathBuf {
    use std::os::unix::ffi::OsStrExt;
    PathBuf::from(OsStr::from_bytes(p))
}

#[cfg(windows)]
pub fn path_of(p: &[u8]) -> PathBuf {
    PathBuf::from(os_string(p))
}

#[cfg(unix)]
pub fn os_bytes(s: &OsStr) -> Vec<u8> {
    use std::os::unix::ffi::OsStrExt;
    s.as_bytes().to_vec()
}

#[cfg(windows)]
pub fn os_bytes(s: &OsStr) -> Vec<u8> {
    use std::os::windows::ffi::OsStrExt;
    crate::names::wide_to_bytes(&s.encode_wide().collect::<Vec<u16>>())
}

#[cfg(unix)]
fn os_string(p: &[u8]) -> OsString {
    use std::os::unix::ffi::OsStrExt;
    OsStr::from_bytes(p).to_os_string()
}

#[cfg(windows)]
fn os_string(p: &[u8]) -> OsString {
    use std::os::windows::ffi::OsStringExt;
    OsString::from_wide(&crate::names::bytes_to_wide(p))
}

/// `getenv`.  A name std would refuse (empty, or holding `=` or NUL) is simply unset.
pub fn getenv(name: &[u8]) -> Option<Vec<u8>> {
    if name.is_empty() || name.contains(&b'=') || name.contains(&0) {
        return None;
    }
    std::env::var_os(os_string(name)).map(|v| os_bytes(&v))
}

/// All of the environment, as names and values.
pub fn environment() -> Vec<(Vec<u8>, Vec<u8>)> {
    std::env::vars_os().map(|(k, v)| (os_bytes(&k), os_bytes(&v))).collect()
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
    note(name);
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

/// readable.c's `READABLE` on Windows: `GetFileAttributesW` and not a directory.  `metadata` may have nothing to say of a device, of which `GetFileAttributesW` has, so a name of the null device goes by whether it opens.
#[cfg(windows)]
pub fn readable(name: &[u8]) -> Readable {
    note(name);
    match std::fs::metadata(path_of(name)) {
        Ok(m) => {
            if m.is_dir() {
                Readable::No
            } else {
                Readable::Yes
            }
        }
        Err(e) if name_too_long(&e) => Readable::TooLong,
        Err(_) if crate::names::names_null_device(name) && std::fs::File::open(path_of(name)).is_ok() => Readable::Yes,
        Err(_) => Readable::No,
    }
}

/// `FindFirstFile`: what the directory calls what `path` names.
#[cfg(windows)]
#[allow(unsafe_code)]
fn directory_says(path: &[u8]) -> Option<Vec<u8>> {
    use windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE;
    use windows_sys::Win32::Storage::FileSystem::{FindClose, FindFirstFileW, WIN32_FIND_DATAW};
    let mut wide = crate::names::bytes_to_wide(path);
    wide.push(0);
    // SAFETY: the name ends in a zero, an all-zero structure is valid plain data that the system fills, and a handle that came back goes back.
    unsafe {
        let mut data: WIN32_FIND_DATAW = core::mem::zeroed();
        let h = FindFirstFileW(wide.as_ptr(), &mut data);
        if h == INVALID_HANDLE_VALUE {
            return None;
        }
        FindClose(h);
        let n = data.cFileName.iter().position(|&c| c == 0).unwrap_or(data.cFileName.len());
        Some(crate::names::wide_to_bytes(&data.cFileName[..n]))
    }
}

/// The name `kpathsea_find_file_generic` ends with on Windows, if not the one it has.  A directory is asked once in a run, as a run's names have most of their parts in common.  C calls `FindFirstFile` in its narrow form with
/// a name in UTF-8, which finds no name that is not ASCII, and keeps what it had: so does this.
#[cfg(windows)]
pub fn long_path_of(name: &[u8]) -> Option<Vec<u8>> {
    thread_local! {
        static DIRECTORIES: std::cell::RefCell<std::collections::HashMap<Vec<u8>, Vec<u8>>> = std::cell::RefCell::new(std::collections::HashMap::new());
    }
    if !name.is_ascii() {
        return None;
    }
    crate::names::long_path(name, 500, &mut |part, last| {
        if last {
            return directory_says(part);
        }
        DIRECTORIES.with(|d| {
            if let Some(known) = d.borrow().get(part) {
                return Some(known.clone());
            }
            let said = directory_says(part)?;
            d.borrow_mut().insert(part.to_vec(), said.clone());
            Some(said)
        })
    })
}

/// A name stays as it is here.
#[cfg(unix)]
pub fn long_path_of(_name: &[u8]) -> Option<Vec<u8>> {
    None
}

/// A message on standard error.  On a Windows console `std` refuses all of it for one byte that is not UTF-8, which a file name can hold.
pub fn write_stderr(bytes: &[u8]) {
    use std::io::Write;
    #[cfg(windows)]
    {
        use std::io::IsTerminal;
        if std::io::stderr().is_terminal() {
            let _ = std::io::stderr().write_all(String::from_utf8_lossy(bytes).as_bytes());
            return;
        }
    }
    let _ = std::io::stderr().write_all(bytes);
}

/// `fopen(name, FOPEN_A_MODE)`.
pub fn open_append(name: &[u8]) -> Option<std::fs::File> {
    std::fs::OpenOptions::new().append(true).create(true).open(path_of(name)).ok()
}

/// How tex-make.c's `maketex` runs a script: no standard input, its standard output is the answer, and stderr is ours or nowhere.  On Unix
/// it is found on the `PATH`, and on Windows beside the program, as `_spawnvp` of `fullbin`.  If it cannot start, `perror` of the child,
/// where its stderr goes, and an empty answer.
pub fn run_mktex(args: &[Vec<u8>], env: &[(Vec<u8>, Vec<u8>)], discard_errors: bool, selfautoloc: &[u8]) -> Vec<u8> {
    use std::process::{Command, Stdio};
    let prog = if cfg!(windows) {
        let dir: Vec<u8> = selfautoloc.iter().map(|&c| if c == b'/' { b'\\' } else { c }).collect();
        let full = [&dir[..], b"\\", &args[0][..]].concat();
        if !discard_errors {
            write_stderr(&[&b"\nThe command name is "[..], &full, b"\n"].concat());
        }
        full
    } else {
        args[0].clone()
    };
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
                write_stderr(&m);
            }
            Vec::new()
        }
    }
}

/// `kpathsea_dir_p`: a directory, or a symbolic link to one.
pub fn is_dir(name: &[u8]) -> bool {
    note(name);
    std::fs::metadata(path_of(name)).map(|m| m.is_dir()).unwrap_or(false)
}

/// The number of links of a directory, for elt-dirs.c's `st_nlink` leaf test.  0 means unknown.
#[cfg(unix)]
pub fn dir_nlink(name: &[u8]) -> u64 {
    use std::os::unix::fs::MetadataExt;
    note(name);
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
    let followed = |e: &std::fs::DirEntry| {
        note(&os_bytes(e.path().as_os_str()));
        std::fs::metadata(e.path()).map(|m| m.is_dir()).unwrap_or(false)
    };
    match e.file_type() {
        Ok(t) if t.is_dir() => true,
        Ok(t) if t.is_symlink() => followed(e),
        Ok(_) => false,
        Err(_) => followed(e),
    }
}

/// db.c's `same_file_p`: the same device and inode.
#[cfg(unix)]
pub fn same_file(a: &[u8], b: &[u8]) -> bool {
    use std::os::unix::fs::MetadataExt;
    note(a);
    note(b);
    match (std::fs::metadata(path_of(a)), std::fs::metadata(path_of(b))) {
        (Ok(x), Ok(y)) => x.dev() == y.dev() && x.ino() == y.ino(),
        _ => false,
    }
}

/// On Windows two names that differ only in case, which is the only case the caller asks about,
/// name one file.
#[cfg(windows)]
pub fn same_file(a: &[u8], b: &[u8]) -> bool {
    note(a);
    note(b);
    std::fs::metadata(path_of(a)).is_ok() && std::fs::metadata(path_of(b)).is_ok()
}

/// `getpwnam (user)->pw_dir`, by reading `/etc/passwd`, since a `getpwnam` call would need `unsafe`.
/// A user known only to a directory service counts as unknown, and tilde.c then uses `.`.
#[cfg(unix)]
pub fn home_of_user(user: &[u8]) -> Option<Vec<u8>> {
    let text = read_file(b"/etc/passwd").ok()?;
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

/// A `kpsewhich` that a tree of the usual kind has though `PATH` does not say so: a program started from the Finder on macOS has no `/Library/TeX/texbin` in it.  That link is the
/// TeX Live the user chose, which the well-known places, newest year first, might not be.
pub fn known_kpsewhich() -> Option<PathBuf> {
    let p = std::path::Path::new("/Library/TeX/texbin/kpsewhich");
    (cfg!(target_os = "macos") && p.is_file()).then(|| p.to_path_buf())
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

/// The kinds of system that have a rule of their own for caches.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CacheRule {
    /// `%LOCALAPPDATA%\\rstex\\cache`.
    Windows,
    /// `~/Library/Caches/rstex`.
    MacOs,
    /// `$XDG_CACHE_HOME/rstex`, or `~/.cache/rstex`.  An `XDG_CACHE_HOME` that is not absolute is to be ignored, the specification says.
    Xdg,
}

/// The directory for rstex's caches under `rule`, with `get` for the environment: a plain function, so that each system's rule can be tested on any.  DESIGN.md section 12.
pub fn cache_dir_under(rule: CacheRule, get: &dyn Fn(&[u8]) -> Option<Vec<u8>>) -> Option<Vec<u8>> {
    let nonempty = |name: &[u8]| get(name).filter(|v| !v.is_empty());
    match rule {
        CacheRule::Windows => match nonempty(b"LOCALAPPDATA") {
            Some(d) => Some([&d[..], b"\\rstex\\cache"].concat()),
            None => Some([&nonempty(b"USERPROFILE")?[..], b"\\AppData\\Local\\rstex\\cache"].concat()),
        },
        CacheRule::MacOs => Some([&nonempty(b"HOME")?[..], b"/Library/Caches/rstex"].concat()),
        CacheRule::Xdg => match nonempty(b"XDG_CACHE_HOME").filter(|x| x.first() == Some(&b'/')) {
            Some(x) => Some([&x[..], b"/rstex"].concat()),
            None => Some([&nonempty(b"HOME")?[..], b"/.cache/rstex"].concat()),
        },
    }
}

/// What DESIGN.md section 12 calls the directory for rstex's caches on this system.  A name that is not absolute counts as none, as the XDG specification says: a cache below the current
/// directory would be whatever the document's directory holds, and what is in a cache gets mapped and trusted.
fn cache_dir_name() -> Option<PathBuf> {
    let rule = if cfg!(windows) {
        CacheRule::Windows
    } else if cfg!(target_os = "macos") {
        CacheRule::MacOs
    } else {
        CacheRule::Xdg
    };
    Some(path_of(&cache_dir_under(rule, &getenv)?)).filter(|d| d.is_absolute())
}

/// The directory for rstex's caches, made if need be, and only if no one else can write to it.  A checkpoint and a format in it are memory images, mapped and trusted: a hostile
/// one is memory corruption.  `None` means that the run goes without, quietly.  This is a few system calls once in a process.
pub fn cache_dir() -> Option<PathBuf> {
    static CHECKED: std::sync::Mutex<Option<(PathBuf, bool)>> = std::sync::Mutex::new(None);
    let d = cache_dir_name()?;
    let mut c = CHECKED.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((name, ok)) = &*c {
        if *name == d {
            return ok.then_some(d);
        }
    }
    let ok = private_dir(&d);
    *c = Some((d.clone(), ok));
    ok.then_some(d)
}

/// The owner, of the nearest existing directory on the way up from `d`.
#[cfg(unix)]
fn owner_above(d: &std::path::Path) -> Option<u32> {
    use std::os::unix::fs::MetadataExt;
    d.ancestors().find_map(|a| std::fs::metadata(a).ok()).map(|m| m.uid())
}

/// True if a directory with these properties, as `lstat` gives them, can have nothing in it that the user `euid` did not put there.
#[cfg(unix)]
fn dir_can_be_trusted(is_dir: bool, uid: u32, mode: u32, euid: u32) -> bool {
    is_dir && uid == euid && mode & 0o022 == 0
}

#[cfg(unix)]
fn private_dir(d: &std::path::Path) -> bool {
    use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};
    let euid = unix_map::euid();
    if std::fs::symlink_metadata(d).is_err() {
        // root with a user's HOME: a directory it made there would lock that user out, as this check locks root out of theirs.
        if euid == 0 && owner_above(d) != Some(0) {
            return false;
        }
        if let Some(p) = d.parent() {
            let _ = std::fs::create_dir_all(p);
        }
        let _ = std::fs::DirBuilder::new().mode(0o700).create(d);
    }
    let Ok(m) = std::fs::symlink_metadata(d) else { return false };
    if !dir_can_be_trusted(m.file_type().is_dir(), m.uid(), m.mode(), euid) {
        return false;
    }
    // From before there was this check: only its owner could write there, and from now on only its owner reads it, for it holds the text of preambles.
    m.mode() & 0o077 == 0 || std::fs::set_permissions(d, std::fs::Permissions::from_mode(m.mode() & 0o700)).is_ok()
}

#[cfg(windows)]
fn private_dir(d: &std::path::Path) -> bool {
    let _ = std::fs::create_dir_all(d);
    std::fs::symlink_metadata(d).is_ok_and(|m| m.file_type().is_dir())
}

/// True if a file with these properties, from the file it was opened as, is the user `euid`'s and no one else's to write.
#[cfg(unix)]
fn file_can_be_trusted(is_file: bool, uid: u32, mode: u32, euid: u32) -> bool {
    is_file && uid == euid && mode & 0o022 == 0
}

/// A file of the cache directory as `o` opens it, with what the check has already asked of the system: not through a symbolic link, not waiting on a FIFO, and only a regular file that no one else could write.
#[cfg(unix)]
fn open_checked(o: &mut std::fs::OpenOptions, path: &std::path::Path) -> Option<(std::fs::File, std::fs::Metadata)> {
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
    let f = o.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK).open(path).ok()?;
    let m = f.metadata().ok()?;
    file_can_be_trusted(m.is_file(), m.uid(), m.mode(), unix_map::euid()).then_some((f, m))
}

#[cfg(windows)]
fn open_checked(o: &mut std::fs::OpenOptions, path: &std::path::Path) -> Option<(std::fs::File, std::fs::Metadata)> {
    let f = o.open(path).ok()?;
    let m = f.metadata().ok()?;
    m.is_file().then_some((f, m))
}

/// A file of the cache directory, to read, as [`open_checked`] takes it, and what it has learnt of the file.
fn open_private_with(path: &std::path::Path) -> Option<(std::fs::File, std::fs::Metadata)> {
    open_checked(std::fs::OpenOptions::new().read(true), path)
}

/// A file of the cache directory, to read.
pub fn open_private(path: &std::path::Path) -> Option<std::fs::File> {
    open_private_with(path).map(|(f, _)| f)
}

/// A file of the cache directory that is written in place, made if it is not there, for no one else to read.
pub fn open_private_to_update(path: &std::path::Path) -> Option<std::fs::File> {
    let mut o = std::fs::OpenOptions::new();
    o.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut o, 0o600);
    open_checked(&mut o, path).map(|(f, _)| f)
}

/// A new file of the cache directory, to write: for no one else to read, and made here, so that nothing already there is followed or written through.  A temporary of a run that died
/// with this process id goes first.
pub fn create_private(path: &std::path::Path) -> std::io::Result<std::fs::File> {
    let _ = std::fs::remove_file(path);
    let mut o = std::fs::OpenOptions::new();
    o.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut o, 0o600);
    o.open(path)
}

/// True if a file of `len` bytes may be written at all.  A sandbox can have a limit that a document's outputs stay under and a cache file does not, and it is the cache's job to go without.
pub fn may_write_file_of(len: u64) -> bool {
    #[cfg(unix)]
    return len <= unix_map::file_size_limit();
    #[cfg(windows)]
    {
        let _ = len;
        true
    }
}

/// Publishes a cache file by temporary and rename.  Failure is harmless: the answer is asked again.
pub fn publish_path(path: &std::path::Path, data: &[u8]) {
    let Some(dir) = path.parent() else { return };
    if !may_write_file_of(data.len() as u64) || std::fs::create_dir_all(dir).is_err() {
        return;
    }
    let tmp = path.with_extension(format!("tmp{}", std::process::id()));
    if create_private(&tmp).and_then(|mut f| std::io::Write::write_all(&mut f, data)).is_err() {
        let _ = std::fs::remove_file(&tmp);
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

/// How many files of one kind the cache directory keeps, those younger than [`CACHE_KEEP_YOUNG`] aside.  Each of a kind is for another environment or search path, and an index is 2 MB.
const CACHE_KEEP: usize = 8;
const CACHE_KEEP_YOUNG: u64 = 3600;
/// A temporary file this old belongs to a run that died.
const CACHE_STALE_TMP: u64 = 86400;
/// A file in use is marked as used this seldom, for [`cache_prune`].
const CACHE_MARK_USED: u64 = 86400;

fn age_of(t: Option<std::time::SystemTime>) -> u64 {
    t.and_then(|t| std::time::SystemTime::now().duration_since(t).ok()).map_or(0, |d| d.as_secs())
}

/// A file of the cache is in use: its time says so once a day, so that [`cache_prune`] can tell, and no run pays a system call for it.
pub fn cache_mark_used(path: &std::path::Path, modified: Option<std::time::SystemTime>) {
    if age_of(modified) > CACHE_MARK_USED {
        if let Ok(f) = std::fs::OpenOptions::new().write(true).open(path) {
            let _ = f.set_modified(std::time::SystemTime::now());
        }
    }
}

/// A file named `prefix...` has just been published in `dir`, the only moment the directory grows, so this is when it is pruned: the newest few of that kind stay, and what
/// is older goes unless it is quite new.  A temporary file of a run that died goes too.  A run with a file open keeps it; on Windows the removal fails, and it is for next time.
pub fn cache_prune(dir: &std::path::Path, prefix: &str) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let mut kind = Vec::new();
    for e in rd.flatten() {
        let name = e.file_name();
        let Some(name) = name.to_str() else { continue };
        if !name.starts_with(prefix) {
            continue;
        }
        let Ok(md) = e.metadata() else { continue };
        let age = age_of(md.modified().ok());
        if e.path().extension().and_then(|x| x.to_str()).is_some_and(|x| x.starts_with("tmp")) {
            if age > CACHE_STALE_TMP {
                let _ = std::fs::remove_file(e.path());
            }
            continue;
        }
        kind.push((age, e.path()));
    }
    kind.sort();
    for (age, path) in kind.into_iter().skip(CACHE_KEEP) {
        if age > CACHE_KEEP_YOUNG {
            let _ = std::fs::remove_file(path);
        }
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn env<'a>(vars: &'a [(&'a [u8], &'a [u8])]) -> impl Fn(&[u8]) -> Option<Vec<u8>> + 'a {
        move |name| vars.iter().find(|(k, _)| *k == name).map(|(_, v)| v.to_vec())
    }

    #[test]
    fn where_a_cache_goes_on_each_system() {
        let at = |rule, vars: &[(&[u8], &[u8])]| cache_dir_under(rule, &env(vars));
        assert_eq!(at(CacheRule::Xdg, &[(b"HOME", b"/h")]).unwrap(), b"/h/.cache/rstex");
        assert_eq!(at(CacheRule::Xdg, &[(b"HOME", b"/h"), (b"XDG_CACHE_HOME", b"/x")]).unwrap(), b"/x/rstex");
        assert_eq!(at(CacheRule::Xdg, &[(b"HOME", b"/h"), (b"XDG_CACHE_HOME", b"relative")]).unwrap(), b"/h/.cache/rstex");
        assert_eq!(at(CacheRule::Xdg, &[(b"HOME", b"/h"), (b"XDG_CACHE_HOME", b"")]).unwrap(), b"/h/.cache/rstex");
        assert_eq!(at(CacheRule::Xdg, &[(b"HOME", b"")]), None);
        assert_eq!(at(CacheRule::Xdg, &[]), None);
        assert_eq!(at(CacheRule::MacOs, &[(b"HOME", b"/Users/u"), (b"XDG_CACHE_HOME", b"/x")]).unwrap(), b"/Users/u/Library/Caches/rstex");
        assert_eq!(at(CacheRule::MacOs, &[]), None);
        assert_eq!(at(CacheRule::Windows, &[(b"LOCALAPPDATA", b"C:\\Users\\u\\AppData\\Local")]).unwrap(), b"C:\\Users\\u\\AppData\\Local\\rstex\\cache");
        assert_eq!(at(CacheRule::Windows, &[(b"USERPROFILE", b"C:\\Users\\u")]).unwrap(), b"C:\\Users\\u\\AppData\\Local\\rstex\\cache");
        assert_eq!(at(CacheRule::Windows, &[(b"HOME", b"/h")]), None);
    }

    #[cfg(unix)]
    #[test]
    fn a_cache_is_trusted_only_if_no_one_else_can_write_to_it() {
        const ME: u32 = 1000;
        assert!(dir_can_be_trusted(true, ME, 0o40700, ME));
        assert!(dir_can_be_trusted(true, ME, 0o40755, ME), "only its owner can write: it gets made private");
        assert!(!dir_can_be_trusted(false, ME, 0o120777, ME), "a symbolic link, as lstat says");
        assert!(!dir_can_be_trusted(true, 1001, 0o40700, ME), "another owner");
        assert!(!dir_can_be_trusted(true, ME, 0o40770, ME), "writable by a group");
        assert!(!dir_can_be_trusted(true, ME, 0o40777, ME), "writable by anyone");
        assert!(!dir_can_be_trusted(true, ME, 0o41777, ME), "a sticky one is still shared");
        assert!(file_can_be_trusted(true, ME, 0o100600, ME));
        assert!(file_can_be_trusted(true, ME, 0o100644, ME), "as earlier versions left them");
        assert!(!file_can_be_trusted(false, ME, 0o010600, ME), "a FIFO");
        assert!(!file_can_be_trusted(true, 1001, 0o100600, ME), "another owner");
        assert!(!file_can_be_trusted(true, ME, 0o100620, ME), "writable by a group");
        assert!(!file_can_be_trusted(true, ME, 0o100606, ME), "writable by anyone");
    }

    /// A file last marked `age` seconds ago.
    fn aged(path: &std::path::Path, age: u64) {
        std::fs::write(path, b"x").unwrap();
        let t = std::time::SystemTime::now() - std::time::Duration::from_secs(age);
        std::fs::OpenOptions::new().write(true).open(path).unwrap().set_modified(t).unwrap();
    }

    #[test]
    fn a_kind_keeps_its_newest_and_what_is_quite_new() {
        let d = std::env::temp_dir().join(format!("rstex-kpse-prune-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        for k in 0..12u64 {
            aged(&d.join(format!("kpse-index-{k:02}")), CACHE_KEEP_YOUNG + 100 + k * 100);
            aged(&d.join(format!("map-{k:02}")), 100 + k);
        }
        aged(&d.join("kpse-index-99.tmp123"), CACHE_STALE_TMP + 100);
        aged(&d.join("kpse-index-98.tmp124"), 100);
        aged(&d.join("kpse-tree"), CACHE_STALE_TMP * 100);
        cache_prune(&d, "kpse-index-");
        for k in 0..12 {
            assert_eq!(d.join(format!("kpse-index-{k:02}")).exists(), k < CACHE_KEEP, "by age, the newest stay: {k}");
            assert!(d.join(format!("map-{k:02}")).exists(), "another kind is not touched");
        }
        assert!(!d.join("kpse-index-99.tmp123").exists(), "a run that died");
        assert!(d.join("kpse-index-98.tmp124").exists(), "a run in progress");
        assert!(d.join("kpse-tree").exists());
        // All of a kind that is younger than the hour stays, whatever their number: a run with another environment may be using it.
        cache_prune(&d, "map-");
        for k in 0..12 {
            assert!(d.join(format!("map-{k:02}")).exists());
        }
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_use_marks_a_file_once_a_day() {
        let d = std::env::temp_dir().join(format!("rstex-kpse-mark-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let p = d.join("f");
        aged(&p, CACHE_MARK_USED + 100);
        let old = std::fs::metadata(&p).unwrap().modified().ok();
        cache_mark_used(&p, old);
        assert!(age_of(std::fs::metadata(&p).unwrap().modified().ok()) < 100, "marked");
        aged(&p, 100);
        let young = std::fs::metadata(&p).unwrap().modified().ok();
        cache_mark_used(&p, young);
        assert_eq!(std::fs::metadata(&p).unwrap().modified().ok(), young, "too soon");
        std::fs::remove_dir_all(&d).unwrap();
    }
}
