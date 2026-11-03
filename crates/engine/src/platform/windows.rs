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
use core::sync::atomic::{AtomicUsize, Ordering};
use rstex_kpse::names::{bytes_to_wide, console_text, wide_to_bytes};
use std::io::{IsTerminal, Write};
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use windows_sys::Win32::Foundation::{EXCEPTION_ACCESS_VIOLATION, EXCEPTION_STACK_OVERFLOW, FILETIME, SYSTEMTIME};
use windows_sys::Win32::Storage::FileSystem::WriteFile;
use windows_sys::Win32::System::Console::{GetStdHandle, STD_ERROR_HANDLE};
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

pub fn advise_huge(_p: *mut u8, _size: usize) {}

/// The system's page, on the x86-64 and arm64 that Windows runs on.
pub fn page_size() -> usize {
    4096
}

/// See the top of this file.
pub fn map_file_private(_file: &std::fs::File, _offset: u64, _p: *mut u8, _size: usize) -> bool {
    false
}

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

fn to_broken(st: &SYSTEMTIME, gmtoff: i32) -> BrokenTime {
    BrokenTime {
        year: st.wYear as i32,
        month: st.wMonth as i32,
        day: st.wDay as i32,
        hour: st.wHour as i32,
        minute: st.wMinute as i32,
        second: st.wSecond as i32,
        gmtoff,
    }
}

pub fn broken_time(epoch: i64, utc: bool) -> BrokenTime {
    let t = epoch * 10_000_000 + EPOCH_DIFF;
    let ft = FILETIME { dwLowDateTime: t as u32, dwHighDateTime: (t >> 32) as u32 };
    // SAFETY: all-zero SYSTEMTIMEs are valid plain data; the system functions fill them.
    unsafe {
        let mut st: SYSTEMTIME = core::mem::zeroed();
        FileTimeToSystemTime(&ft, &mut st);
        if utc {
            return to_broken(&st, 0);
        }
        let mut lt: SYSTEMTIME = core::mem::zeroed();
        SystemTimeToTzSpecificLocalTime(core::ptr::null(), &st, &mut lt);
        let minutes = |s: &SYSTEMTIME| s.wDay as i32 * 1440 + s.wHour as i32 * 60 + s.wMinute as i32;
        // The zone bias, from the two broken-down times; a month rollover needs the month's end.
        let mut d = minutes(&lt) - minutes(&st);
        if lt.wMonth != st.wMonth {
            d = if d > 0 { d - (st.wDay as i32) * 1440 } else { d + (lt.wDay as i32) * 1440 };
        }
        to_broken(&lt, d * 60)
    }
}

pub const DOSISH: bool = true;
/// `std` gives no change time here, and a write can keep size and mtime, so a checkpoint hashes every file it read.
pub const STAT_SEES_CONTENT: bool = false;
pub const SHELL_QUOTE: u8 = b'"';

/// `cmd /c command`, as `system` and `popen` do.
pub fn shell_command(command: &[u8], env: &[super::EnvVar]) -> std::process::Command {
    let mut c = std::process::Command::new("cmd");
    c.arg("/c").arg(String::from_utf8_lossy(command).as_ref());
    for (name, value) in env {
        c.env(String::from_utf8_lossy(name).as_ref(), String::from_utf8_lossy(value).as_ref());
    }
    c
}

pub fn shell(command: &[u8], env: &[super::EnvVar]) -> i32 {
    shell_command(command, env).status().ok().and_then(|s| s.code()).unwrap_or(-1)
}

/// `system` gives the exit code itself here.
pub fn system_status(code: i32) -> i32 {
    code
}

/// UTF-8 bytes to a wide-character path.
pub fn path_from_bytes(p: &[u8]) -> std::path::PathBuf {
    std::path::PathBuf::from(String::from_utf8_lossy(p).as_ref())
}

pub fn path_to_bytes(p: &std::path::Path) -> Vec<u8> {
    let wide: Vec<u16> = p.as_os_str().encode_wide().collect();
    String::from_utf16_lossy(&wide).into_bytes()
}

pub fn env_var(name: &[u8]) -> Option<Vec<u8>> {
    let v = std::env::var_os(String::from_utf8_lossy(name).as_ref())?;
    let wide: Vec<u16> = std::ffi::OsString::from_wide(&v.encode_wide().collect::<Vec<u16>>()).encode_wide().collect();
    Some(String::from_utf16_lossy(&wide).into_bytes())
}

pub fn file_stat(path: &[u8]) -> Option<FileStat> {
    let m = std::fs::metadata(path_from_bytes(path)).ok()?;
    let mtime = m
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    Some(FileStat { size: m.len(), mtime, mtime_nsec: 0, change: 0, inode: 0 })
}

#[inline(always)]
pub fn is_dir_sep(c: i32) -> bool {
    c == b'/' as i32 || c == b'\\' as i32
}

pub fn is_absolute(name: &[u8]) -> bool {
    match name {
        [b'/' | b'\\', ..] => true,
        [d, b':', ..] if d.is_ascii_alphabetic() => true,
        _ => false,
    }
}

pub fn publish(tmp: &[u8], dest: &[u8]) -> std::io::Result<()> {
    let (tmp, dest) = (path_from_bytes(tmp), path_from_bytes(dest));
    let mut last = Ok(());
    for _ in 0..10 {
        last = std::fs::rename(&tmp, &dest);
        if last.is_ok() {
            return last;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    last
}

pub fn cache_dir() -> Option<Vec<u8>> {
    let mut d = env_var(b"LOCALAPPDATA")?;
    d.extend_from_slice(b"\\rstex\\cache");
    Some(d)
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
