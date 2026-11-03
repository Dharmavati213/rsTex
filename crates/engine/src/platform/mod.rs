//! Everything specific to an operating system, DESIGN.md section 12.
//!
//! `cfg(unix)`, `cfg(windows)`, `cfg(target_os)`, `libc::` and `windows_sys::` appear here and in
//! `crates/kpse`, and nowhere else.  Linux-only tuning is a `cfg` inside this module, never a semantic
//! difference: every platform must produce the same PDF bytes for the same inputs.
//!
//! Besides `raw.rs`, this is the only part of the engine allowed `unsafe`.

#[cfg(not(all(target_pointer_width = "64", target_endian = "little")))]
compile_error!("rstex supports only 64-bit little-endian targets (DESIGN.md section 12)");

#[cfg(unix)]
mod unix;
#[cfg(unix)]
use unix as sys;

#[cfg(windows)]
mod windows;
#[cfg(windows)]
use windows as sys;

/// Clocks, counters and the sampler for `crate::prof`, with the feature `prof`.
#[cfg(feature = "prof")]
pub mod prof {
    pub use super::sys::prof::*;

    /// One hardware or software counter, as it read at the end of the run.
    pub struct Counter {
        pub name: &'static str,
        pub value: u64,
        pub enabled_ns: u64,
        /// Below `enabled_ns` if the counter was multiplexed.
        pub running_ns: u64,
    }

    /// What the sampler did.
    pub struct SamplerInfo {
        pub source: &'static str,
        /// Hz for the timer, events per sample for a counter.
        pub period: u64,
        pub samples: u64,
        pub dropped: u64,
        pub frames: u64,
    }
}

/// The alignment and commit granularity of the reservation.
pub const TWO_MIB: usize = 2 << 20;

/// Broken-down time, as `struct tm`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BrokenTime {
    pub year: i32,
    pub month: i32,
    pub day: i32,
    pub hour: i32,
    pub minute: i32,
    pub second: i32,
    /// Seconds east of UTC.
    pub gmtoff: i32,
}

/// What a checkpoint compares to decide whether a file changed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileStat {
    pub size: u64,
    pub mtime: i64,
    /// 0 where there is no such notion.
    pub inode: u64,
}

/// Reserves `size` bytes of lazily zeroed, lazily committed address space, 2 MiB-aligned.
/// On failure, returns null.
pub fn reserve_zeroed(size: usize) -> *mut u8 {
    sys::reserve_zeroed(size)
}

/// Gives back a reservation.
///
/// # Safety
/// `p` came from [`reserve_zeroed`] with this `size`, and nothing refers to it any more.
pub unsafe fn release(p: *mut u8, size: usize) {
    unsafe { sys::release(p, size) }
}

/// Makes `p..p+size`, inside a reservation, usable by the kernel as well as by us.  It is needed before
/// a system call fills pages directly: on Windows `ReadFile` into uncommitted pages fails rather than
/// faulting.  A no-op where pages commit on any first touch.
pub fn commit_range(p: *mut u8, size: usize) {
    sys::commit_range(p, size)
}

/// `epoch`, in UTC or in local time.
pub fn broken_time(epoch: i64, utc: bool) -> BrokenTime {
    sys::broken_time(epoch, utc)
}

/// A variable for a child process, as bytes.
pub type EnvVar = (Vec<u8>, Vec<u8>);

/// Runs a command through the system's shell, `\write18`, with `env` added to the environment it
/// inherits; returns its exit status, or -1.
pub fn shell(command: &[u8], env: &[EnvVar]) -> i32 {
    sys::shell(command, env)
}

/// What C's `system` returns for a command that exited with `code`, for the message of `runsystem`.
pub fn system_status(code: i32) -> i32 {
    sys::system_status(code)
}

/// `popen(command, "r")`, up to the end of the output: the bytes it wrote, or `None` if it could not start.
/// Its standard error stays ours.
pub fn popen_read(command: &[u8], env: &[EnvVar]) -> Option<Vec<u8>> {
    use std::io::Read;
    let mut child = sys::shell_command(command, env).stdout(std::process::Stdio::piped()).spawn().ok()?;
    let mut out = Vec::new();
    if let Some(mut o) = child.stdout.take() {
        let _ = o.read_to_end(&mut out);
    }
    let _ = child.wait();
    Some(out)
}

/// `popen(command, "w")`: the child, with its standard input a pipe.
pub fn popen_write(command: &[u8], env: &[EnvVar]) -> Option<std::process::Child> {
    sys::shell_command(command, env).stdin(std::process::Stdio::piped()).spawn().ok()
}

/// `DOSISH` of kpathsea: the system quotes and names commands as Windows does.
pub const DOSISH: bool = sys::DOSISH;

/// `QUOTE` of texmfmp.c: what a restricted `\write18` puts around each argument.
pub const SHELL_QUOTE: u8 = sys::SHELL_QUOTE;

/// TeX sees file names as bytes; the conversion happens at this boundary.
pub fn path_from_bytes(p: &[u8]) -> std::path::PathBuf {
    sys::path_from_bytes(p)
}

/// The bytes TeX sees for a path.
pub fn path_to_bytes(p: &std::path::Path) -> Vec<u8> {
    sys::path_to_bytes(p)
}

/// An environment variable.
pub fn env_var(name: &[u8]) -> Option<Vec<u8>> {
    sys::env_var(name)
}

/// Size, modification time and identity of a file.
pub fn file_stat(path: &[u8]) -> Option<FileStat> {
    sys::file_stat(path)
}

/// `IS_DIR_SEP`, §542.
#[inline(always)]
pub fn is_dir_sep(c: i32) -> bool {
    sys::is_dir_sep(c)
}

/// True if a file name is absolute, as `kpse_absolute_p`.
pub fn is_absolute(name: &[u8]) -> bool {
    sys::is_absolute(name)
}

/// Publishes a finished file: rename, with a retry on Windows, since a reader may hold it open.
pub fn publish(tmp: &[u8], dest: &[u8]) -> std::io::Result<()> {
    sys::publish(tmp, dest)
}

/// The directory for rstex's caches, without creating it.
pub fn cache_dir() -> Option<Vec<u8>> {
    sys::cache_dir()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reservation_is_aligned_zeroed_and_lazy() {
        // A large reservation costs nothing until touched.
        let size = 1usize << 33;
        let p = reserve_zeroed(size);
        assert!(!p.is_null());
        assert_eq!(p as usize % TWO_MIB, 0);
        commit_range(p, 4096);
        // SAFETY: both offsets are inside the reservation.
        unsafe {
            assert_eq!(p.read(), 0);
            p.add(size - 1).write(7);
            assert_eq!(p.add(size - 1).read(), 7);
            release(p, size);
        }
    }

    #[test]
    fn time_is_broken_down() {
        // 2023-11-14 22:13:20 UTC
        let t = broken_time(1700000000, true);
        assert_eq!((t.year, t.month, t.day, t.hour, t.minute, t.second), (2023, 11, 14, 22, 13, 20));
        assert_eq!(t.gmtoff, 0);
    }

    #[test]
    fn paths_round_trip_as_bytes() {
        let b = b"some/dir/file.tex";
        assert_eq!(path_to_bytes(&path_from_bytes(b)), b);
    }

    #[test]
    fn shell_reports_status() {
        assert_eq!(shell(b"exit 3", &[]), 3);
    }

    #[cfg(unix)]
    #[test]
    fn a_child_sees_what_it_is_given() {
        let env = [(b"RSTEX_PLATFORM_TEST".to_vec(), b"seen".to_vec())];
        assert_eq!(shell(b"test \"$RSTEX_PLATFORM_TEST\" = seen", &env), 0);
        assert_eq!(popen_read(b"echo \"$RSTEX_PLATFORM_TEST\"", &env).unwrap(), b"seen\n");
    }

    #[cfg(unix)]
    #[test]
    fn a_pipe_takes_what_is_written() {
        use std::io::Write;
        let dir = std::env::temp_dir().join(format!("rstex-platform-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("out");
        let cmd = format!("cat > '{}'", path.display());
        let mut child = popen_write(cmd.as_bytes(), &[]).unwrap();
        child.stdin.take().unwrap().write_all(b"abc\n").unwrap();
        child.wait().unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"abc\n");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
