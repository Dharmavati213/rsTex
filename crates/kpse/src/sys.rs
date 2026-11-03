//! Everything specific to an operating system in this crate (DESIGN.md section 12).
//!
//! File names are bytes, as TeX sees them: the name itself on Unix, UTF-8 on Windows.
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
            [1 + (m.is_dir() as u64) << 8, m.len(), t, 0, 0]
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
}

impl FileBytes {
    /// Bytes made here.
    pub fn from_vec(data: Vec<u8>) -> FileBytes {
        FileBytes {
            #[cfg(unix)]
            map: None,
            data,
        }
    }

    /// The contents of `path`, if it can be opened.
    #[cfg(unix)]
    pub fn open(path: &std::path::Path) -> Option<FileBytes> {
        use std::os::fd::AsRawFd;
        let f = std::fs::File::open(path).ok()?;
        let len = f.metadata().ok()?.len() as usize;
        if len == 0 {
            return Some(FileBytes::from_vec(Vec::new()));
        }
        Some(FileBytes { map: Some(unix_map::Map::new(f.as_raw_fd(), len)?), data: Vec::new() })
    }

    /// The contents of `path`, if it can be read.  A mapping would hold the file open against the rename that replaces it.
    #[cfg(windows)]
    pub fn open(path: &std::path::Path) -> Option<FileBytes> {
        Some(FileBytes::from_vec(std::fs::read(path).ok()?))
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

/// readable.c's `READABLE` on Windows: `GetFileAttributesW` and not a directory.
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
        Err(_) => Readable::No,
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
pub fn publish_path(path: &std::path::Path, data: &[u8]) {
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
