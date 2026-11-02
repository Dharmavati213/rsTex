//! Linux and macOS.

use super::{BrokenTime, FileStat, TWO_MIB};
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::os::unix::fs::MetadataExt;

pub fn reserve_zeroed(size: usize) -> *mut u8 {
    let size = size.next_multiple_of(TWO_MIB);
    let total = size + TWO_MIB;
    #[cfg(target_os = "linux")]
    let flags = libc::MAP_PRIVATE | libc::MAP_ANONYMOUS | libc::MAP_NORESERVE;
    #[cfg(not(target_os = "linux"))]
    let flags = libc::MAP_PRIVATE | libc::MAP_ANON;
    // SAFETY: an anonymous private mapping with no address hint; the result is checked.
    let base = unsafe {
        libc::mmap(core::ptr::null_mut(), total, libc::PROT_READ | libc::PROT_WRITE, flags, -1, 0)
    };
    if base == libc::MAP_FAILED {
        return core::ptr::null_mut();
    }
    let addr = base as usize;
    let aligned = addr.next_multiple_of(TWO_MIB);
    // SAFETY: both trims are inside the mapping obtained above; `aligned..aligned+size` stays.
    unsafe {
        if aligned > addr {
            libc::munmap(base, aligned - addr);
        }
        let end = addr + total;
        let keep_end = aligned + size;
        if end > keep_end {
            libc::munmap(keep_end as *mut libc::c_void, end - keep_end);
        }
        // A failure only loses the performance hint.
        #[cfg(target_os = "linux")]
        libc::madvise(aligned as *mut libc::c_void, size, libc::MADV_HUGEPAGE);
    }
    aligned as *mut u8
}

/// # Safety
/// See `super::release`.
pub unsafe fn release(p: *mut u8, size: usize) {
    // SAFETY: by this function's contract.
    unsafe {
        libc::munmap(p as *mut libc::c_void, size.next_multiple_of(TWO_MIB));
    }
}

pub fn commit_range(_p: *mut u8, _size: usize) {}

pub fn broken_time(epoch: i64, utc: bool) -> BrokenTime {
    // SAFETY: an all-zero `tm` is valid plain data; the libc functions fill it.
    unsafe {
        let mut tm: libc::tm = core::mem::zeroed();
        let t: libc::time_t = epoch as libc::time_t;
        if utc {
            libc::gmtime_r(&t, &mut tm);
        } else {
            libc::localtime_r(&t, &mut tm);
        }
        BrokenTime {
            year: tm.tm_year + 1900,
            month: tm.tm_mon + 1,
            day: tm.tm_mday,
            hour: tm.tm_hour,
            minute: tm.tm_min,
            second: tm.tm_sec,
            gmtoff: tm.tm_gmtoff as i32,
        }
    }
}

pub const DOSISH: bool = false;
pub const SHELL_QUOTE: u8 = b'\'';

/// `sh -c command`, as `system` and `popen` do.
pub fn shell_command(command: &[u8], env: &[super::EnvVar]) -> std::process::Command {
    let mut c = std::process::Command::new("sh");
    c.arg("-c").arg(std::ffi::OsStr::from_bytes(command));
    for (name, value) in env {
        c.env(std::ffi::OsStr::from_bytes(name), std::ffi::OsStr::from_bytes(value));
    }
    c
}

pub fn shell(command: &[u8], env: &[super::EnvVar]) -> i32 {
    shell_command(command, env).status().ok().and_then(|s| s.code()).unwrap_or(-1)
}

/// `WEXITSTATUS` undone.
pub fn system_status(code: i32) -> i32 {
    if code < 0 { code } else { code << 8 }
}

pub fn path_from_bytes(p: &[u8]) -> std::path::PathBuf {
    std::path::PathBuf::from(std::ffi::OsStr::from_bytes(p))
}

pub fn path_to_bytes(p: &std::path::Path) -> Vec<u8> {
    p.as_os_str().as_bytes().to_vec()
}

pub fn env_var(name: &[u8]) -> Option<Vec<u8>> {
    std::env::var_os(std::ffi::OsStr::from_bytes(name)).map(|v| v.into_vec())
}

pub fn file_stat(path: &[u8]) -> Option<FileStat> {
    std::fs::metadata(path_from_bytes(path)).ok().map(|m| FileStat {
        size: m.size(),
        mtime: m.mtime(),
        inode: m.ino(),
    })
}

#[inline(always)]
pub fn is_dir_sep(c: i32) -> bool {
    c == b'/' as i32
}

pub fn is_absolute(name: &[u8]) -> bool {
    name.first() == Some(&b'/')
}

pub fn publish(tmp: &[u8], dest: &[u8]) -> std::io::Result<()> {
    std::fs::rename(path_from_bytes(tmp), path_from_bytes(dest))
}

pub fn cache_dir() -> Option<Vec<u8>> {
    #[cfg(target_os = "macos")]
    {
        let mut h = env_var(b"HOME")?;
        h.extend_from_slice(b"/Library/Caches/rstex");
        Some(h)
    }
    #[cfg(not(target_os = "macos"))]
    {
        if let Some(mut x) = env_var(b"XDG_CACHE_HOME") {
            if !x.is_empty() {
                x.extend_from_slice(b"/rstex");
                return Some(x);
            }
        }
        let mut h = env_var(b"HOME")?;
        h.extend_from_slice(b"/.cache/rstex");
        Some(h)
    }
}
