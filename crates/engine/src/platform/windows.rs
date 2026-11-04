//! Windows.  `cargo check` covers this file on the Linux development machine; running it needs the
//! Windows CI job (DESIGN.md section 12, and docs/BUILD.md says what that job has to settle).
//!
//! Windows has no lazily committed zero pages.  An access to a page that is reserved and not committed faults, for a read as for a write.  So the reservation
//! is `VirtualAlloc(MEM_RESERVE)`, and a vectored exception handler commits on first touch, in 2 MiB chunks: a committed page reads as zero, and gets a frame of memory
//! only when touched.  That is what makes every read of the image work as it does on Unix: the file-level reads of `toks` below `NULL_LIST` that `get_next_leaf` makes, a look at a
//! section that was never written, and the `Tex` struct as a whole.  A commit counts against the system's limit, not against the process's memory.
//!
//! **The handler sees only what code in this process touches.**  A system call given an uncommitted page does not fault in user mode: it fails with `ERROR_NOACCESS`, for `WriteFile` from such
//! a page as for `ReadFile` into one.  So whoever hands image memory to the system calls [`commit_range`] on it first: `image_load_bytes` and `image_write_fmt`.  All else goes through a `Vec`.
//!
//! **No mapping.**  `MapViewOfFile` cannot lie over a reservation, cannot be placed in part of one without the placeholders of Windows 10 1803, needs a file offset that is a multiple of the 64 KiB
//! allocation granularity where `image_padding` gives 16 KiB, and a view holds the file against the rename that replaces it.  So [`map_file_private`] says no, and the caller reads.

use super::{BrokenTime, FileStat, TWO_MIB};
use core::sync::atomic::{AtomicPtr, AtomicUsize, Ordering};
use rstex_kpse::names::{bytes_to_wide, console_text, wide_to_bytes};
use std::io::{IsTerminal, Write};
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use windows_sys::Win32::Foundation::{EXCEPTION_ACCESS_VIOLATION, EXCEPTION_STACK_OVERFLOW, FILETIME, SYSTEMTIME};
use windows_sys::Win32::Storage::FileSystem::WriteFile;
use windows_sys::Win32::System::Console::{CTRL_BREAK_EVENT, CTRL_C_EVENT, GetStdHandle, STD_ERROR_HANDLE, SetConsoleCtrlHandler};
use windows_sys::Win32::System::Diagnostics::Debug::{AddVectoredExceptionHandler, EXCEPTION_POINTERS};
use windows_sys::Win32::System::Memory::{MEM_COMMIT, MEM_RELEASE, MEM_RESERVE, PAGE_READWRITE, VirtualAlloc, VirtualFree};
use windows_sys::Win32::System::Threading::{ExitProcess, SetThreadStackGuarantee};
use windows_sys::Win32::System::Time::{FileTimeToSystemTime, SystemTimeToTzSpecificLocalTime};

const EXCEPTION_CONTINUE_EXECUTION: i32 = -1;
const EXCEPTION_CONTINUE_SEARCH: i32 = 0;

/// The live reservations, for the fault handler: (aligned base, size, allocation base).  A run has an image and a checkpoint's copy; the room is for a test binary, whose threads
/// each take a few, and some of its tests leave theirs.  Only the part in use is ever touched.
const SLOTS: usize = 1 << 14;
static BASE: [AtomicUsize; SLOTS] = [const { AtomicUsize::new(0) }; SLOTS];
static SIZE: [AtomicUsize; SLOTS] = [const { AtomicUsize::new(0) }; SLOTS];
static ALLOC: [AtomicUsize; SLOTS] = [const { AtomicUsize::new(0) }; SLOTS];
/// A slot that a thread is filling in.
const TAKEN: usize = 1;
/// How far the slots have been used, which is all the handler looks at.
static USED: AtomicUsize = AtomicUsize::new(0);
static HANDLER: std::sync::Once = std::sync::Once::new();
static HANDLER_CTRL: std::sync::Once = std::sync::Once::new();

/// Says so and ends the process: what can still be done in the handler, with no allocation and little stack.
fn die(msg: &str) -> ! {
    // SAFETY: the handle is the process's own, and the message outlives the call.
    unsafe {
        let mut written = 0;
        WriteFile(GetStdHandle(STD_ERROR_HANDLE), msg.as_ptr(), msg.len() as u32, &mut written, core::ptr::null_mut());
        ExitProcess(1)
    }
}

unsafe extern "system" fn on_exception(info: *mut EXCEPTION_POINTERS) -> i32 {
    // SAFETY: the system passes a valid record to a vectored handler.
    unsafe {
        let rec = (*info).ExceptionRecord;
        if (*rec).ExceptionCode == EXCEPTION_STACK_OVERFLOW {
            die(super::STACK_MESSAGE);
        }
        if (*rec).ExceptionCode != EXCEPTION_ACCESS_VIOLATION || (*rec).NumberParameters < 2 {
            return EXCEPTION_CONTINUE_SEARCH;
        }
        let addr = (*rec).ExceptionInformation[1];
        for i in 0..USED.load(Ordering::Acquire) {
            let base = BASE[i].load(Ordering::Acquire);
            let size = SIZE[i].load(Ordering::Acquire);
            if base > TAKEN && addr >= base && addr < base + size {
                let chunk = addr & !(TWO_MIB - 1);
                // Committing what another thread has just committed does no harm.
                if VirtualAlloc(chunk as *const core::ffi::c_void, TWO_MIB, MEM_COMMIT, PAGE_READWRITE).is_null() {
                    die("rstex: fatal: memory exhausted: the system commits no more\n");
                }
                return EXCEPTION_CONTINUE_EXECUTION;
            }
        }
        EXCEPTION_CONTINUE_SEARCH
    }
}

fn install_handler() {
    // SAFETY: the handler has the signature asked for and stays for the life of the process.
    HANDLER.call_once(|| unsafe {
        AddVectoredExceptionHandler(1, Some(on_exception));
    });
}

pub fn reserve_zeroed(size: usize) -> *mut u8 {
    install_handler();
    let size = size.next_multiple_of(TWO_MIB);
    let total = size + TWO_MIB;
    // SAFETY: a fresh reservation at an address of the system's choice; the result is checked.
    unsafe {
        let base = VirtualAlloc(core::ptr::null(), total, MEM_RESERVE, PAGE_READWRITE);
        if base.is_null() {
            return core::ptr::null_mut();
        }
        let aligned = (base as usize).next_multiple_of(TWO_MIB);
        for i in 0..SLOTS {
            if BASE[i].compare_exchange(0, TAKEN, Ordering::AcqRel, Ordering::Relaxed).is_ok() {
                ALLOC[i].store(base as usize, Ordering::Release);
                SIZE[i].store(size, Ordering::Release);
                BASE[i].store(aligned, Ordering::Release);
                USED.fetch_max(i + 1, Ordering::AcqRel);
                return aligned as *mut u8;
            }
        }
        VirtualFree(base, 0, MEM_RELEASE);
        core::ptr::null_mut()
    }
}

/// # Safety
/// See `super::release`.
pub unsafe fn release(p: *mut u8, _size: usize) {
    let addr = p as usize;
    for i in 0..USED.load(Ordering::Acquire) {
        if BASE[i].load(Ordering::Acquire) == addr {
            let base = ALLOC[i].load(Ordering::Acquire);
            SIZE[i].store(0, Ordering::Release);
            BASE[i].store(0, Ordering::Release);
            // SAFETY: `base` is what VirtualAlloc returned, by the table.
            unsafe {
                VirtualFree(base as *mut core::ffi::c_void, 0, MEM_RELEASE);
            }
            return;
        }
    }
}

pub fn commit_range(p: *mut u8, size: usize) -> bool {
    if size == 0 {
        return true;
    }
    let start = (p as usize) & !(TWO_MIB - 1);
    let end = (p as usize + size).next_multiple_of(TWO_MIB);
    // SAFETY: the range is rounded out to chunks of a reservation.
    unsafe { !VirtualAlloc(start as *const core::ffi::c_void, end - start, MEM_COMMIT, PAGE_READWRITE).is_null() }
}

pub fn process_init() {
    install_handler();
}

/// What the engine's thread does first: room to run the handler in when its stack is full.
pub fn engine_stack_begins(_size: usize) {
    let mut guarantee = 32 << 10;
    // SAFETY: the argument is valid for the call.  A failure only leaves less room, which the handler may still fit in.
    unsafe {
        SetThreadStackGuarantee(&mut guarantee);
    }
}

/// TeX's `interrupt`.
static INTERRUPT: AtomicPtr<i32> = AtomicPtr::new(core::ptr::null_mut());

/// On a thread of its own, which the system starts for this.
unsafe extern "system" fn on_console_event(event: u32) -> i32 {
    if event != CTRL_C_EVENT && event != CTRL_BREAK_EVENT {
        return 0;
    }
    let p = INTERRUPT.load(Ordering::Relaxed);
    if !p.is_null() {
        // SAFETY: `p` is `interrupt` in the image, which lives as long as the process.  A store of an integer is all this does, as in C.
        unsafe { p.write_volatile(1) };
    }
    1
}

pub fn catch_interrupt(cell: &mut i32) {
    INTERRUPT.store(cell, Ordering::Relaxed);
    // SAFETY: the handler has the signature asked for and stays for the life of the process; adding it twice calls it twice, to the same effect.
    HANDLER_CTRL.call_once(|| unsafe {
        SetConsoleCtrlHandler(Some(on_console_event), 1);
    });
}

pub fn advise_huge(_p: *mut u8, _size: usize) {}

pub fn collapse_huge(_p: *mut u8, _size: usize) {}

/// The system's page, on the x86-64 and arm64 that Windows runs on.
pub fn page_size() -> usize {
    4096
}

/// See the top of this file.
pub fn map_file_private(_file: &std::fs::File, _offset: u64, _p: *mut u8, _size: usize) -> bool {
    false
}

pub fn populate_read(_p: *mut u8, _size: usize) {}

/// No mapping: see the top of this file.
pub fn map_file_view(_file: &std::fs::File, _len: usize) -> *const u8 {
    core::ptr::null()
}

/// # Safety
/// Never called: `map_file_view` gives nothing to unmap.
pub unsafe fn unmap_file_view(_p: *const u8, _len: usize) {}

pub fn read_exact_at(file: &std::fs::File, offset: u64, buf: &mut [u8]) -> bool {
    use std::os::windows::fs::FileExt;
    let mut at = 0;
    while at < buf.len() {
        match file.seek_read(&mut buf[at..], offset + at as u64) {
            Ok(0) | Err(_) => return false,
            Ok(n) => at += n,
        }
    }
    true
}

/// 100 ns units between 1601 and 1970.
const EPOCH_DIFF: i64 = 116444736000000000;

fn to_broken(st: &SYSTEMTIME) -> BrokenTime {
    BrokenTime {
        year: st.wYear as i32,
        month: st.wMonth as i32,
        day: st.wDay as i32,
        hour: st.wHour as i32,
        minute: st.wMinute as i32,
        second: st.wSecond as i32,
        gmtoff: 0,
    }
}

pub fn broken_time(epoch: i64, utc: bool) -> BrokenTime {
    let t = epoch.saturating_mul(10_000_000).saturating_add(EPOCH_DIFF);
    let ft = FILETIME { dwLowDateTime: t as u32, dwHighDateTime: (t >> 32) as u32 };
    // SAFETY: all-zero SYSTEMTIMEs are valid plain data; the system functions fill them.
    unsafe {
        let mut st: SYSTEMTIME = core::mem::zeroed();
        // No such date: as `gmtime_r` leaves a `tm`, or nearly, since a month must index the names of the months.
        if FileTimeToSystemTime(&ft, &mut st) == 0 {
            return BrokenTime { year: 1900, month: 1, day: 1, ..BrokenTime::default() };
        }
        let gmt = to_broken(&st);
        if utc {
            return gmt;
        }
        let mut lt: SYSTEMTIME = core::mem::zeroed();
        if SystemTimeToTzSpecificLocalTime(core::ptr::null(), &st, &mut lt) == 0 {
            return gmt;
        }
        let mut local = to_broken(&lt);
        local.gmtoff = super::zone_offset(&local, &gmt);
        local
    }
}

pub const DOSISH: bool = true;
/// `std` gives no change time here, and a write can keep size and mtime, so a checkpoint hashes every file it read.
pub const STAT_SEES_CONTENT: bool = false;
pub const SHELL_QUOTE: u8 = b'"';

fn wide(b: &[u8]) -> std::ffi::OsString {
    std::ffi::OsString::from_wide(&bytes_to_wide(b))
}

/// What `system` and `popen` run: `%COMSPEC% /c command`, the command as it stands, since `cmd` has its own idea of quotes and those of `CommandLineToArgvW` would only get in its way.
pub fn shell_command(command: &[u8], env: &[super::EnvVar]) -> std::process::Command {
    use std::os::windows::process::CommandExt;
    let mut c = std::process::Command::new(std::env::var_os("COMSPEC").unwrap_or_else(|| "cmd.exe".into()));
    c.arg("/c").raw_arg(wide(command));
    for (name, value) in env {
        c.env(wide(name), wide(value));
    }
    c
}

pub fn shell(command: &[u8], env: &[super::EnvVar]) -> i32 {
    shell_command(command, env).status().ok().and_then(|s| s.code()).unwrap_or(-1)
}

/// A process cannot become another here, so it waits for one, and a job that holds this one holds that too.
pub fn run_again(args: &[std::ffi::OsString]) -> i32 {
    let first = std::env::current_exe().map_or_else(|_| args[0].clone(), std::ffi::OsString::from);
    match std::process::Command::new(&first).args(&args[1..]).status() {
        Ok(s) => s.code().unwrap_or(1),
        Err(e) => {
            rstex_kpse::write_stderr(format!("rstex: cannot run {}: {}\n", args[0].to_string_lossy(), super::os_error_text(&e)).as_bytes());
            1
        }
    }
}

/// `system` gives the exit code itself here.
pub fn system_status(code: i32) -> i32 {
    code
}

pub fn path_from_bytes(p: &[u8]) -> std::path::PathBuf {
    std::path::PathBuf::from(wide(p))
}

pub fn path_to_bytes(p: &std::path::Path) -> Vec<u8> {
    wide_to_bytes(&p.as_os_str().encode_wide().collect::<Vec<u16>>())
}

pub fn env_var(name: &[u8]) -> Option<Vec<u8>> {
    std::env::var_os(wide(name)).map(|v| wide_to_bytes(&v.encode_wide().collect::<Vec<u16>>()))
}

pub fn file_stat(path: &[u8]) -> Option<FileStat> {
    use std::os::windows::fs::MetadataExt;
    let m = std::fs::metadata(path_from_bytes(path)).ok()?;
    let t = m.last_write_time() as i64 - EPOCH_DIFF;
    Some(FileStat { size: m.len(), mtime: t.div_euclid(10_000_000), mtime_nsec: t.rem_euclid(10_000_000) * 100, change: 0, inode: 0, regular: m.is_file() })
}

#[inline(always)]
pub fn is_dir_sep(c: i32) -> bool {
    c == b'/' as i32 || c == b'\\' as i32
}

pub fn is_absolute(name: &[u8]) -> bool {
    rstex_kpse::names::absolute_p_g::<true>(name, false)
}

/// `remove` of the C runtime gives `EACCES` for what `std` calls this: a file that is open or read only.
pub fn is_access_denied(e: &std::io::Error) -> bool {
    e.kind() == std::io::ErrorKind::PermissionDenied
}

pub fn dir_builder_private(_b: &mut std::fs::DirBuilder) {}

/// A rename cannot go over a file that someone has open without sharing it for deletion: a virus scanner, for a moment, or a PDF viewer, for long.  `std` opens with all sharing.  So
/// the same again a few times, for half a second in all, on what such a clash gives: `ERROR_ACCESS_DENIED`, `ERROR_SHARING_VIOLATION` and `ERROR_LOCK_VIOLATION`.
pub fn publish(tmp: &[u8], dest: &[u8]) -> std::io::Result<()> {
    let (tmp, dest) = (path_from_bytes(tmp), path_from_bytes(dest));
    let mut wait = 1;
    loop {
        let r = std::fs::rename(&tmp, &dest);
        match r {
            Err(ref e) if wait < 256 && matches!(e.raw_os_error(), Some(5 | 32 | 33)) => {
                std::thread::sleep(std::time::Duration::from_millis(wait));
                wait *= 2;
            }
            _ => return r,
        }
    }
}

/// A rename keeps everything about a plain file that a reader of it can see, here.
pub fn replaceable(m: &std::fs::Metadata) -> bool {
    m.file_type().is_file()
}

static STDOUT_TAIL: std::sync::Mutex<Vec<u8>> = std::sync::Mutex::new(Vec::new());

/// On a console `std` takes only UTF-8, and a whole buffer goes missing for one byte that is not; redirected, it writes the bytes as they are, with no `\r` added.
pub fn write_stdout(bytes: &[u8]) {
    let mut out = std::io::stdout().lock();
    let _ = if out.is_terminal() {
        let mut tail = STDOUT_TAIL.lock().unwrap_or_else(|e| e.into_inner());
        out.write_all(console_text(&mut tail, bytes).as_bytes())
    } else {
        out.write_all(bytes)
    };
    let _ = out.flush();
}

/// `crate::prof` does nothing here.
#[cfg(feature = "prof")]
pub mod prof {
    use crate::platform::prof::{Counter, SamplerInfo};

    pub const AVAILABLE: bool = false;

    pub fn now_ns() -> u64 {
        0
    }
    pub fn cpu_ns() -> u64 {
        0
    }
    pub fn faults() -> (u64, u64) {
        (0, 0)
    }
    pub fn instructions() -> u64 {
        0
    }
    pub fn cycles() -> u64 {
        0
    }
    pub fn block_sampler_signal() {}
    pub fn counters_open() {}
    pub fn counters_read() -> Vec<Counter> {
        Vec::new()
    }
    pub fn sampler_start(_hz: u64, _event: Option<&[u8]>) {}
    pub fn sampler_stop() {}
    pub fn sampler_write(_path: &[u8]) -> std::io::Result<SamplerInfo> {
        Err(std::io::Error::from(std::io::ErrorKind::Unsupported))
    }
}
