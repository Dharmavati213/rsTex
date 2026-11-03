//! Windows.  `cargo check` covers this file on the Linux development machine; running it needs the
//! Windows CI job (DESIGN.md section 12).
//!
//! Windows has no lazily committed zero pages, so the reservation is `VirtualAlloc(MEM_RESERVE)`, and a
//! vectored exception handler commits on first touch, in 2 MiB chunks.  A system call does not take
//! that fault: `ReadFile` into uncommitted pages fails, hence `commit_range`.

use super::{BrokenTime, FileStat, TWO_MIB};
use core::sync::atomic::{AtomicUsize, Ordering};
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use windows_sys::Win32::Foundation::{EXCEPTION_ACCESS_VIOLATION, FILETIME, SYSTEMTIME};
use windows_sys::Win32::System::Diagnostics::Debug::{AddVectoredExceptionHandler, EXCEPTION_POINTERS};
use windows_sys::Win32::System::Memory::{
    MEM_COMMIT, MEM_RELEASE, MEM_RESERVE, PAGE_READWRITE, VirtualAlloc, VirtualFree,
};
use windows_sys::Win32::System::Time::{FileTimeToSystemTime, SystemTimeToTzSpecificLocalTime};

const EXCEPTION_CONTINUE_EXECUTION: i32 = -1;
const EXCEPTION_CONTINUE_SEARCH: i32 = 0;

/// The live reservations, for the fault handler: (aligned base, size, allocation base).  A process has one
/// image, and tests a few.
const SLOTS: usize = 64;
static BASE: [AtomicUsize; SLOTS] = [const { AtomicUsize::new(0) }; SLOTS];
static SIZE: [AtomicUsize; SLOTS] = [const { AtomicUsize::new(0) }; SLOTS];
static ALLOC: [AtomicUsize; SLOTS] = [const { AtomicUsize::new(0) }; SLOTS];
static HANDLER: AtomicUsize = AtomicUsize::new(0);

unsafe extern "system" fn commit_on_touch(info: *mut EXCEPTION_POINTERS) -> i32 {
    // SAFETY: the system passes a valid record to a vectored handler.
    unsafe {
        let rec = (*info).ExceptionRecord;
        if (*rec).ExceptionCode != EXCEPTION_ACCESS_VIOLATION || (*rec).NumberParameters < 2 {
            return EXCEPTION_CONTINUE_SEARCH;
        }
        let addr = (*rec).ExceptionInformation[1];
        for i in 0..SLOTS {
            let base = BASE[i].load(Ordering::Acquire);
            let size = SIZE[i].load(Ordering::Acquire);
            if base != 0 && addr >= base && addr < base + size {
                let chunk = addr & !(TWO_MIB - 1);
                let p = VirtualAlloc(chunk as *const core::ffi::c_void, TWO_MIB, MEM_COMMIT, PAGE_READWRITE);
                return if p.is_null() { EXCEPTION_CONTINUE_SEARCH } else { EXCEPTION_CONTINUE_EXECUTION };
            }
        }
        EXCEPTION_CONTINUE_SEARCH
    }
}

pub fn reserve_zeroed(size: usize) -> *mut u8 {
    let size = size.next_multiple_of(TWO_MIB);
    let total = size + TWO_MIB;
    // SAFETY: a fresh reservation at an address of the system's choice; the result is checked.
    unsafe {
        if HANDLER.load(Ordering::Acquire) == 0 {
            let h = AddVectoredExceptionHandler(1, Some(commit_on_touch));
            HANDLER.store(h as usize, Ordering::Release);
        }
        let base = VirtualAlloc(core::ptr::null(), total, MEM_RESERVE, PAGE_READWRITE);
        if base.is_null() {
            return core::ptr::null_mut();
        }
        let aligned = (base as usize).next_multiple_of(TWO_MIB);
        for i in 0..SLOTS {
            if BASE[i].load(Ordering::Acquire) == 0 {
                ALLOC[i].store(base as usize, Ordering::Release);
                SIZE[i].store(size, Ordering::Release);
                BASE[i].store(aligned, Ordering::Release);
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
    for i in 0..SLOTS {
        if BASE[i].load(Ordering::Acquire) == addr {
            let base = ALLOC[i].load(Ordering::Acquire);
            BASE[i].store(0, Ordering::Release);
            SIZE[i].store(0, Ordering::Release);
            // SAFETY: `base` is what VirtualAlloc returned, by the table.
            unsafe {
                VirtualFree(base as *mut core::ffi::c_void, 0, MEM_RELEASE);
            }
            return;
        }
    }
}

pub fn commit_range(p: *mut u8, size: usize) {
    if size == 0 {
        return;
    }
    let start = (p as usize) & !(TWO_MIB - 1);
    let end = (p as usize + size).next_multiple_of(TWO_MIB);
    // SAFETY: the range is rounded out to chunks of a reservation.
    unsafe {
        VirtualAlloc(start as *const core::ffi::c_void, end - start, MEM_COMMIT, PAGE_READWRITE);
    }
}

pub fn process_init() {}

pub fn advise_huge(_p: *mut u8, _size: usize) {}

pub fn page_size() -> usize {
    4096
}

/// A mapping holds a file open against rename and delete on Windows, and could not lie over a reservation anyway: the caller reads.
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
    Some(FileStat { size: m.len(), mtime, mtime_nsec: 0, change: 0, inode: 0, regular: m.is_file() })
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

pub fn dir_builder_private(_b: &mut std::fs::DirBuilder) {}

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
