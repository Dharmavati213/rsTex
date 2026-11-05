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
    pub mtime_nsec: i64,
    /// When the inode last changed, in nanoseconds: `utime` can set back `mtime` and not this, so an edit that keeps size,
    /// mtime and inode shows here.  0 where there is no such notion: see [`STAT_SEES_CONTENT`].
    pub change: i64,
    /// 0 where there is no such notion.
    pub inode: u64,
    /// A regular file, which can be read again and say the same: not `/dev/stdin`, not a FIFO.
    pub regular: bool,
}

/// True if an equal [`FileStat`] means the file was not written, which a checkpoint trusts for the files in a TeX tree.
/// Where it is false, those are hashed like the others.
pub const STAT_SEES_CONTENT: bool = sys::STAT_SEES_CONTENT;

/// Reserves `size` bytes of lazily zeroed, lazily committed address space, 2 MiB-aligned.
/// On failure, returns null.
pub fn reserve_zeroed(size: usize) -> *mut u8 {
    sys::reserve_zeroed(size)
}

/// What the process does first, before it has allocated much: on Linux, tells the C allocator that it has one busy thread and no memory to
/// give back, which saves the system calls of growing a thread's arena one page at a time.  On Windows, the handler for the image's pages.
pub fn process_init() {
    sys::process_init()
}

/// The smallest stack [`run_on_stack`] settles for: the oracle's depth of recursion, and much more.
const MIN_STACK: usize = 64 << 20;

/// What a full stack gets: as `overflow` would say it, for it is the same thing, but it can only go to stderr.  Nothing can be flushed from where this is printed, so the log ends some way short
/// of here, and the PDF is not written.
pub const STACK_MESSAGE: &str = "! TeX capacity exceeded, sorry [engine stack size]: recursion too deep; the log is cut short and there is no PDF.\n";

/// Runs `f` on a stack of `size` bytes, only reserved until touched, and returns what it returns.  On a new thread, which is how it can be done everywhere; glibc
/// on Linux can do it on this thread, with one thread and no waking of another on some other CPU.  With less if that much cannot be had, as under `ulimit -v`.  A full stack ends the run with
/// [`STACK_MESSAGE`].  What that costs on Unix, a handler and a signal stack, is in docs/BUILD.md.
pub fn run_on_stack(size: usize, f: impl FnOnce() -> i32 + Send + 'static) -> i32 {
    #[cfg(all(target_os = "linux", target_env = "gnu", not(feature = "prof")))]
    return sys::run_on_new_stack(size, f);
    #[cfg(not(all(target_os = "linux", target_env = "gnu", not(feature = "prof"))))]
    {
        let job = std::sync::Arc::new(std::sync::Mutex::new(Some(f)));
        let mut size = size;
        loop {
            let j = job.clone();
            let thread = std::thread::Builder::new().stack_size(size).spawn(move || {
                sys::engine_stack_begins(size);
                let f = j.lock().unwrap_or_else(|e| e.into_inner()).take();
                f.map_or(101, |f| f())
            });
            match thread {
                Ok(t) => return t.join().unwrap_or(101),
                Err(_) if size > MIN_STACK => size /= 4,
                Err(e) => fatal(&format!("rstex: fatal: cannot start the engine's thread: {}\n", os_error_text(&e))),
            }
        }
    }
}

/// `what` on stderr, and the end of the process with the status TeX has for a fatal error: for what comes before TeX can say anything.
pub fn fatal(what: &str) -> ! {
    rstex_kpse::write_stderr(what.as_bytes());
    std::process::exit(1)
}

/// What `strerror` says of `e`, without what Rust adds.
pub fn os_error_text(e: &std::io::Error) -> String {
    let text = e.to_string();
    match text.split_once(" (os error") {
        Some((t, _)) => t.to_string(),
        None => text,
    }
}

/// Lets huge pages back the 2 MiB chunks inside `p..p+size`, part of a reservation, whatever the system's default is.  The rest of a reservation
/// stays with small pages, since a huge page costs the zeroing of 2 MiB however little of it is touched: `Tex::alloc` chooses.  Only a hint.
pub fn advise_huge(p: *mut u8, size: usize) {
    sys::advise_huge(p, size)
}

/// Moves what is in the 2 MiB chunks around `p..p+size` to huge pages now, for a run that has shown it will be long enough to pay for it.
pub fn collapse_huge(p: *mut u8, size: usize) {
    sys::collapse_huge(p, size)
}

/// Asks for the pages of the next `bytes` the allocator hands out, in one go instead of one fault each.  For a short run; only a hint.
pub fn prefault_heap(bytes: usize) {
    sys::prefault_heap(bytes)
}

/// The system's page size.
pub fn page_size() -> usize {
    sys::page_size()
}

/// Maps `size` bytes of `file` from `offset` over `p..p+size` privately: reads see the file, and a write copies the page.  `p` and `offset` are
/// aligned to [`page_size`], and the range is inside a reservation and holds nothing yet.  False, leaving the range as it was, where the system has no such
/// thing or it fails: the caller reads the bytes instead.
pub fn map_file_private(file: &std::fs::File, offset: u64, p: *mut u8, size: usize) -> bool {
    sys::map_file_private(file, offset, p, size)
}

/// Reads exactly `buf.len()` bytes of `file` at `offset`, without moving a file position.
pub fn read_exact_at(file: &std::fs::File, offset: u64, buf: &mut [u8]) -> bool {
    sys::read_exact_at(file, offset, buf)
}

/// Maps what is at `p..p+size` for reading, so that looking at it takes no fault for each page.  Only a hint.
pub fn populate_read(p: *mut u8, size: usize) {
    sys::populate_read(p, size)
}

/// Gives back a reservation.
///
/// # Safety
/// `p` came from [`reserve_zeroed`] with this `size`, and nothing refers to it any more.
pub unsafe fn release(p: *mut u8, size: usize) {
    unsafe { sys::release(p, size) }
}

/// A private buffer as fast to fill as the image, for a checkpoint's copy of it: a reservation of its own, with huge pages where the system has them, since all of it gets
/// filled.  A worker writes it out while the engine goes on.
pub struct HugeBuf {
    p: *mut u8,
    len: usize,
}

// SAFETY: the buffer is owned, and the raw pointer is only ever used through `&self` or `&mut self`.
unsafe impl Send for HugeBuf {}

impl HugeBuf {
    /// `len` zero bytes, or `None`.
    pub fn new(len: usize) -> Option<HugeBuf> {
        let p = reserve_zeroed(len.max(1));
        if p.is_null() {
            return None;
        }
        advise_huge(p, len);
        let buf = HugeBuf { p, len };
        if commit_range(p, len) { Some(buf) } else { None }
    }

    /// As [`HugeBuf::new`] with small pages and nothing committed, for something sparse: a page not yet touched is zero and costs nothing, on Windows by the handler of every reservation.
    pub fn new_small(len: usize) -> Option<HugeBuf> {
        let p = reserve_zeroed(len.max(1));
        if p.is_null() {
            return None;
        }
        Some(HugeBuf { p, len })
    }

    pub fn as_slice(&self) -> &[u8] {
        // SAFETY: `p..p+len` is ours, committed, and zero or written since.
        unsafe { core::slice::from_raw_parts(self.p, self.len) }
    }

    pub fn as_mut_slice(&mut self) -> &mut [u8] {
        // SAFETY: as above, and `&mut self` makes it the only reference.
        unsafe { core::slice::from_raw_parts_mut(self.p, self.len) }
    }
}

impl Drop for HugeBuf {
    fn drop(&mut self) {
        // SAFETY: from `reserve_zeroed`, with the size it was asked for, and no slice outlives `self`.
        unsafe { release(self.p, self.len.max(1)) }
    }
}

/// A file as a slice, to read: mapped and not copied where the system can do that, and read into memory where not.
pub struct FileView {
    p: *const u8,
    len: usize,
    bytes: Vec<u8>,
}

// SAFETY: it only reads, and the mapping is owned.
unsafe impl Send for FileView {}

impl FileView {
    /// The first `len` bytes of `file`, which must be there and stay there.
    pub fn new(file: &std::fs::File, len: usize) -> Option<FileView> {
        if len == 0 {
            return Some(FileView { p: core::ptr::null(), len: 0, bytes: Vec::new() });
        }
        let p = if env_var(b"RSTEX_PAGE_NO_MAP").is_none() { sys::map_file_view(file, len) } else { core::ptr::null() };
        if !p.is_null() {
            return Some(FileView { p, len, bytes: Vec::new() });
        }
        let mut bytes = vec![0; len];
        if read_exact_at(file, 0, &mut bytes) { Some(FileView { p: core::ptr::null(), len, bytes }) } else { None }
    }

    pub fn as_slice(&self) -> &[u8] {
        if self.p.is_null() {
            return &self.bytes;
        }
        // SAFETY: `p..p+len` is a mapping of ours, for as long as `self` lives.
        unsafe { core::slice::from_raw_parts(self.p, self.len) }
    }
}

impl Drop for FileView {
    fn drop(&mut self) {
        if !self.p.is_null() {
            // SAFETY: from `map_file_view` with this length, and no slice outlives `self`.
            unsafe { sys::unmap_file_view(self.p, self.len) }
        }
    }
}

/// Makes `p..p+size`, inside a reservation, usable by the kernel as well as by us.  It is needed before
/// a system call fills pages or reads them directly: on Windows `ReadFile` into uncommitted pages, and `WriteFile` from them, fail rather than
/// faulting.  A no-op where pages commit on any first touch.  False if the system has no more to commit.
pub fn commit_range(p: *mut u8, size: usize) -> bool {
    sys::commit_range(p, size)
}

/// Seconds east of UTC, from what the same moment is in local time and in UTC: a zone is less than a day away.  Plain, so that it can be tested where it is not needed.
pub fn zone_offset(local: &BrokenTime, utc: &BrokenTime) -> i32 {
    let clock = |t: &BrokenTime| t.hour * 3600 + t.minute * 60 + t.second;
    let mut off = clock(local) - clock(utc);
    if (local.year, local.month, local.day) != (utc.year, utc.month, utc.day) {
        off += if (local.year, local.month, local.day) > (utc.year, utc.month, utc.day) { 86400 } else { -86400 };
    }
    off
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

/// What `cmd /c` is given for a command on Windows: `kpathsea_fsyscp_system` of knj.c.  `'` is `"`, and a command with a blank in it goes in one more pair of quotes, since `cmd` takes off the first and the last
/// quote of its line when it begins with one: `"a b.exe" "c d"` would otherwise reach it as `a b.exe" "c d`.  `kpathsea_fsyscp_popen` puts in the same pair, and `runpopen` has changed the `'` by then.
/// A plain function, so that it can be tested where it does not apply.
pub fn cmd_argument(command: &[u8]) -> Vec<u8> {
    let blank = command.iter().any(|&c| c == b' ' || c == b'\t');
    let mut out = Vec::with_capacity(command.len() + 2);
    if blank {
        out.push(b'"');
    }
    out.extend(command.iter().map(|&c| if c == b'\'' { b'"' } else { c }));
    if blank {
        out.push(b'"');
    }
    out
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

/// `catch_interrupt` of texmfmp.c: from now on an interrupt from the user sets `*cell`, TeX's `interrupt`, which it looks at between tokens, unless the process was started with
/// interrupts ignored.  `cell` lies in the image, which does not move.
pub fn catch_interrupt(cell: &mut i32) {
    sys::catch_interrupt(cell)
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

/// This program again, in place of this process where the system can do that, with these arguments, of which the first is what it says it was called.  If it returns,
/// it returns the status to exit with: the new program's where it ran as a child, or 1 with a message if it would not start.
pub fn run_again(args: &[std::ffi::OsString]) -> i32 {
    sys::run_again(args)
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

/// A directory that `b` makes is for its owner alone.
pub fn dir_builder_private(b: &mut std::fs::DirBuilder) {
    sys::dir_builder_private(b)
}

/// Publishes a finished file: rename, with a retry on Windows, since a reader may hold it open.
pub fn publish(tmp: &[u8], dest: &[u8]) -> std::io::Result<()> {
    sys::publish(tmp, dest)
}

/// What goes to the terminal, flushed.
pub fn write_stdout(bytes: &[u8]) {
    sys::write_stdout(bytes)
}

/// What goes to standard error.
pub fn write_stderr(bytes: &[u8]) {
    rstex_kpse::write_stderr(bytes)
}

/// True if a rename over this file leaves what `fopen (name, "w")` on it would leave: not a symbolic link, which C writes through, or a device, or a file that has another name too.
pub fn replaceable(m: &std::fs::Metadata) -> bool {
    sys::replaceable(m)
}

/// The directory for rstex's caches, without creating it: the one kpathsea's tables use.
pub fn cache_dir() -> Option<Vec<u8>> {
    rstex_kpse::cache_dir().map(|d| path_to_bytes(&d))
}

/// C's `errno == EACCES` after a call that gave this.
pub fn is_access_denied(e: &std::io::Error) -> bool {
    sys::is_access_denied(e)
}

/// The working directory as `xgetcwd` gives it, with `/` on Windows too.
pub fn current_dir_bytes() -> Vec<u8> {
    let mut d = std::env::current_dir().map(|d| path_to_bytes(&d)).unwrap_or_default();
    if DOSISH {
        for c in d.iter_mut().filter(|c| **c == b'\\') {
            *c = b'/';
        }
    }
    d
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
    fn a_zone_is_what_lies_between_two_clocks() {
        let at = |year, month, day, hour, minute| BrokenTime { year, month, day, hour, minute, ..BrokenTime::default() };
        assert_eq!(zone_offset(&at(2026, 3, 5, 12, 0), &at(2026, 3, 5, 12, 0)), 0);
        assert_eq!(zone_offset(&at(2026, 3, 5, 14, 30), &at(2026, 3, 5, 12, 0)), 9000);
        assert_eq!(zone_offset(&at(2026, 3, 5, 7, 0), &at(2026, 3, 5, 12, 0)), -18000);
        // Across a day, a month and a year, each way.
        assert_eq!(zone_offset(&at(2026, 3, 6, 1, 0), &at(2026, 3, 5, 23, 0)), 7200);
        assert_eq!(zone_offset(&at(2026, 2, 1, 1, 0), &at(2026, 1, 31, 23, 0)), 7200);
        assert_eq!(zone_offset(&at(2026, 1, 1, 1, 0), &at(2025, 12, 31, 23, 0)), 7200);
        assert_eq!(zone_offset(&at(2026, 1, 31, 20, 0), &at(2026, 2, 1, 1, 0)), -18000);
        assert_eq!(zone_offset(&at(2025, 12, 31, 14, 0), &at(2026, 1, 1, 2, 0)), -43200);
    }

    #[test]
    fn what_rust_adds_to_an_error_goes() {
        let e = std::io::Error::from_raw_os_error(2);
        assert!(!os_error_text(&e).contains("os error"), "{e}");
        assert_eq!(os_error_text(&std::io::Error::other("as it is")), "as it is");
    }

    #[test]
    fn paths_round_trip_as_bytes() {
        let b = b"some/dir/file.tex";
        assert_eq!(path_to_bytes(&path_from_bytes(b)), b);
    }

    /// A stack that fills up says so, in a process of its own, and this is that process when the variable says so.
    #[test]
    fn a_full_stack_has_a_message_and_a_status() {
        #[allow(unconditional_recursion)]
        fn deeper(n: u64) -> u64 {
            let mut room = [n; 64];
            core::hint::black_box(&mut room);
            deeper(n + 1) + room[n as usize % 64]
        }
        if env_var(b"RSTEX_TEST_FULL_STACK").is_some() {
            run_on_stack(MIN_STACK, || deeper(0) as i32);
            return;
        }
        let o = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "platform::tests::a_full_stack_has_a_message_and_a_status", "--nocapture", "--test-threads=1"])
            .env("RSTEX_TEST_FULL_STACK", "1")
            .output()
            .unwrap();
        let err = String::from_utf8_lossy(&o.stderr);
        assert!(err.contains(STACK_MESSAGE), "{err}");
        assert_eq!(o.status.code(), Some(1), "{err}");
    }

    /// Any other fault is left alone, and is what it would have been.
    #[cfg(unix)]
    #[test]
    fn another_fault_stays_what_it_is() {
        if env_var(b"RSTEX_TEST_OTHER_FAULT").is_some() {
            run_on_stack(MIN_STACK, || {
                // SAFETY: not at all: this is the fault.
                unsafe { core::ptr::null_mut::<u64>().write_volatile(1) };
                0
            });
            return;
        }
        let o = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "platform::tests::another_fault_stays_what_it_is", "--nocapture", "--test-threads=1"])
            .env("RSTEX_TEST_OTHER_FAULT", "1")
            .output()
            .unwrap();
        let err = String::from_utf8_lossy(&o.stderr);
        assert!(!err.contains(STACK_MESSAGE), "{err}");
        assert_eq!(o.status.code(), None, "a signal ended it: {err}");
    }

    #[test]
    fn shell_reports_status() {
        assert_eq!(shell(b"exit 3", &[]), 3);
    }

    /// What goes to `cmd /c`, whichever system this runs on.
    #[test]
    fn a_command_for_cmd_has_its_quotes() {
        assert_eq!(cmd_argument(b"dir"), b"dir");
        assert_eq!(cmd_argument(b"exit 3"), b"\"exit 3\"");
        assert_eq!(cmd_argument(b"a\tb"), b"\"a\tb\"");
        assert_eq!(cmd_argument(b"\"a b.exe\" \"c d\""), b"\"\"a b.exe\" \"c d\"\"");
        assert_eq!(cmd_argument(b"'a b.exe' 'c d'"), b"\"\"a b.exe\" \"c d\"\"");
        assert_eq!(cmd_argument(b"it's"), b"it\"s");
        assert_eq!(cmd_argument(b""), b"");
    }

    /// `cmd` takes off the first and the last quote of a line that begins with one: this reaches it whole only through [`cmd_argument`].
    #[cfg(windows)]
    #[test]
    fn a_quoted_program_with_quoted_arguments_goes_through_cmd() {
        assert_eq!(shell(b"\"%COMSPEC%\" /c \"exit 4\"", &[]), 4);
        assert_eq!(shell(b"'%COMSPEC%' /c 'exit 5'", &[]), 5);
        assert_eq!(popen_read(b"\"%COMSPEC%\" /c \"echo seen\"", &[]).unwrap().trim_ascii(), b"seen");
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
