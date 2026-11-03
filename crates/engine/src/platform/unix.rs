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

/// The profiler's clocks, counters and sampler, for `crate::prof`.  With the feature `prof` only.
#[cfg(feature = "prof")]
pub mod prof {
    use crate::platform::prof::{Counter, SamplerInfo};
    use core::sync::atomic::{AtomicPtr, AtomicU64, AtomicUsize, Ordering::Relaxed};
    use std::io::Write;

    pub const AVAILABLE: bool = true;

    fn clock(id: libc::clockid_t) -> u64 {
        // SAFETY: an all-zero `timespec` is valid plain data, which the call fills.
        unsafe {
            let mut ts: libc::timespec = core::mem::zeroed();
            libc::clock_gettime(id, &mut ts);
            ts.tv_sec as u64 * 1_000_000_000 + ts.tv_nsec as u64
        }
    }

    pub fn now_ns() -> u64 {
        clock(libc::CLOCK_MONOTONIC)
    }

    pub fn cpu_ns() -> u64 {
        clock(libc::CLOCK_PROCESS_CPUTIME_ID)
    }

    /// Minor and major page faults so far, those taken inside a system call included.
    pub fn faults() -> (u64, u64) {
        // SAFETY: an all-zero `rusage` is valid plain data, which the call fills.
        unsafe {
            let mut r: libc::rusage = core::mem::zeroed();
            libc::getrusage(libc::RUSAGE_SELF, &mut r);
            (r.ru_minflt as u64, r.ru_majflt as u64)
        }
    }

    fn sampler_signal_set() -> libc::sigset_t {
        // SAFETY: `sigset_t` is plain data, initialised by `sigemptyset`.
        unsafe {
            let mut set: libc::sigset_t = core::mem::zeroed();
            libc::sigemptyset(&mut set);
            libc::sigaddset(&mut set, libc::SIGPROF);
            set
        }
    }

    /// Keeps `main`'s thread out of the sampler: only the sampled thread unblocks the signal, and a new thread inherits the mask.
    pub fn block_sampler_signal() {
        let set = sampler_signal_set();
        // SAFETY: `set` is valid for the call.
        unsafe {
            libc::pthread_sigmask(libc::SIG_BLOCK, &set, core::ptr::null_mut());
        }
    }

    // ---- perf_event_open ----

    /// `struct perf_event_attr` up to `sig_data`: `PERF_ATTR_SIZE_VER7`.
    #[cfg(target_os = "linux")]
    #[repr(C)]
    #[derive(Default)]
    struct PerfEventAttr {
        type_: u32,
        size: u32,
        config: u64,
        sample_period: u64,
        sample_type: u64,
        read_format: u64,
        flags: u64,
        wakeup_events: u32,
        bp_type: u32,
        config1: u64,
        config2: u64,
        branch_sample_type: u64,
        sample_regs_user: u64,
        sample_stack_user: u32,
        clockid: i32,
        sample_regs_intr: u64,
        aux_watermark: u32,
        sample_max_stack: u16,
        reserved_2: u16,
        aux_sample_size: u32,
        reserved_3: u32,
        sig_data: u64,
    }

    #[cfg(target_os = "linux")]
    const _: () = assert!(core::mem::size_of::<PerfEventAttr>() == 128);

    #[cfg(target_os = "linux")]
    const FLAG_DISABLED: u64 = 1 << 0;
    #[cfg(target_os = "linux")]
    const FLAG_INHERIT: u64 = 1 << 1;
    #[cfg(target_os = "linux")]
    const FLAG_EXCLUDE_KERNEL: u64 = 1 << 5;
    #[cfg(target_os = "linux")]
    const FLAG_EXCLUDE_HV: u64 = 1 << 6;
    /// `PERF_FORMAT_TOTAL_TIME_ENABLED | PERF_FORMAT_TOTAL_TIME_RUNNING`.
    #[cfg(target_os = "linux")]
    const READ_TIMES: u64 = 1 | 2;
    #[cfg(target_os = "linux")]
    const PERF_EVENT_IOC_ENABLE: libc::c_ulong = 0x2400;
    #[cfg(target_os = "linux")]
    const PERF_EVENT_IOC_DISABLE: libc::c_ulong = 0x2401;

    /// Name, `perf_type_id`, config.  All user-mode, which a `perf_event_paranoid` of 2 allows.
    #[cfg(target_os = "linux")]
    const EVENTS: [(&str, u32, u64); 6] = [
        ("instructions", 0, 1),
        ("cycles", 0, 0),
        ("branch-misses", 0, 5),
        ("L1d-misses", 3, 0x10000),
        ("dTLB-misses", 3, 0x10003),
        ("page-faults", 1, 2),
    ];
    #[cfg(target_os = "linux")]
    static COUNTER_FDS: [core::sync::atomic::AtomicI32; 6] = [const { core::sync::atomic::AtomicI32::new(-1) }; 6];

    #[cfg(target_os = "linux")]
    fn perf_event_open(attr: &PerfEventAttr) -> i32 {
        const PERF_FLAG_FD_CLOEXEC: libc::c_ulong = 8;
        // SAFETY: `attr` is a valid attribute block of the size it states.  Self, any CPU, no group.
        unsafe { libc::syscall(libc::SYS_perf_event_open, attr as *const PerfEventAttr, 0, -1, -1, PERF_FLAG_FD_CLOEXEC) as i32 }
    }

    /// Opens the counters, running from here on, for this thread and the threads it starts.
    pub fn counters_open() {
        #[cfg(target_os = "linux")]
        for (i, &(_, type_, config)) in EVENTS.iter().enumerate() {
            let attr = PerfEventAttr {
                type_,
                size: core::mem::size_of::<PerfEventAttr>() as u32,
                config,
                read_format: READ_TIMES,
                flags: FLAG_INHERIT | FLAG_EXCLUDE_KERNEL | FLAG_EXCLUDE_HV,
                ..Default::default()
            };
            COUNTER_FDS[i].store(perf_event_open(&attr), Relaxed);
        }
    }

    #[cfg(target_os = "linux")]
    fn counter_read(i: usize) -> Option<[u64; 3]> {
        let fd = COUNTER_FDS[i].load(Relaxed);
        if fd < 0 {
            return None;
        }
        let mut v = [0u64; 3];
        // SAFETY: `v` holds the three values `READ_TIMES` asks for.
        let n = unsafe { libc::read(fd, v.as_mut_ptr().cast(), 24) };
        if n == 24 { Some(v) } else { None }
    }

    /// User-mode instructions so far.
    #[inline]
    pub fn instructions() -> u64 {
        #[cfg(target_os = "linux")]
        return counter_read(0).map_or(0, |v| v[0]);
        #[cfg(not(target_os = "linux"))]
        0
    }

    /// User-mode cycles so far.
    #[inline]
    pub fn cycles() -> u64 {
        #[cfg(target_os = "linux")]
        return counter_read(1).map_or(0, |v| v[0]);
        #[cfg(not(target_os = "linux"))]
        0
    }

    pub fn counters_read() -> Vec<Counter> {
        #[allow(unused_mut)]
        let mut out = Vec::new();
        #[cfg(target_os = "linux")]
        for (i, &(name, _, _)) in EVENTS.iter().enumerate() {
            if let Some(v) = counter_read(i) {
                out.push(Counter { name, value: v[0], enabled_ns: v[1], running_ns: v[2] });
            }
        }
        out
    }

    // ---- The sampler ----

    /// Capacities of the preallocated arrays.  Address space only, until a sample touches it.
    const SAMPLE_CAP: usize = 1 << 21;
    const FRAME_CAP: usize = 1 << 26;
    const MAX_DEPTH: usize = 128;

    static PCS: AtomicPtr<u64> = AtomicPtr::new(core::ptr::null_mut());
    static WEIGHTS: AtomicPtr<u32> = AtomicPtr::new(core::ptr::null_mut());
    static STARTS: AtomicPtr<u32> = AtomicPtr::new(core::ptr::null_mut());
    static FRAMES: AtomicPtr<u64> = AtomicPtr::new(core::ptr::null_mut());
    static N: AtomicUsize = AtomicUsize::new(0);
    static NF: AtomicUsize = AtomicUsize::new(0);
    static DROPPED: AtomicU64 = AtomicU64::new(0);
    /// The thread the frame-pointer walk is good for, and the top of its stack; 0 if not known, and then no walk.
    static SAMPLED_THREAD: AtomicUsize = AtomicUsize::new(0);
    static STACK_TOP: AtomicUsize = AtomicUsize::new(0);
    /// 0 the timer, 1 instructions, 2 cycles.
    static SOURCE: AtomicU64 = AtomicU64::new(0);
    static PERIOD: AtomicU64 = AtomicU64::new(0);
    /// CPU time at the last sample of the timer.
    static LAST_CPU_NS: AtomicU64 = AtomicU64::new(0);
    #[cfg(target_os = "linux")]
    static OVERFLOW_FD: core::sync::atomic::AtomicI32 = core::sync::atomic::AtomicI32::new(-1);

    /// Program counter, frame pointer and stack pointer of the interrupted code.
    ///
    /// # Safety
    /// `uc` is what the kernel passed to a `SA_SIGINFO` handler.
    #[inline(always)]
    unsafe fn registers(uc: *const libc::ucontext_t) -> (usize, usize, usize) {
        // SAFETY: by this function's contract.
        unsafe {
            #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
            {
                let g = &(*uc).uc_mcontext.gregs;
                (g[libc::REG_RIP as usize] as usize, g[libc::REG_RBP as usize] as usize, g[libc::REG_RSP as usize] as usize)
            }
            #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
            {
                let m = &(*uc).uc_mcontext;
                (m.pc as usize, m.regs[29] as usize, m.sp as usize)
            }
            #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
            {
                let s = &(*(*uc).uc_mcontext).__ss;
                (s.__pc as usize, s.__fp as usize, s.__sp as usize)
            }
            #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
            {
                let s = &(*(*uc).uc_mcontext).__ss;
                (s.__rip as usize, s.__rbp as usize, s.__rsp as usize)
            }
        }
    }

    /// Records where the signal found the program.  No allocation, and no calls but `pthread_self`, which reads a register, and
    /// `clock_gettime`, which POSIX lists as safe here.  The signal is blocked while this runs and only one thread takes it, so the
    /// relaxed counters have one writer.
    ///
    /// Each signal of the timer records the CPU time since the last.  A tick inside a long system call sets a signal pending once,
    /// however many ticks pass, and it arrives at the return: a 10 ms `read` is one sample.  With the time it is 10 ms, charged there.
    unsafe extern "C" fn on_sigprof(_sig: libc::c_int, _info: *mut libc::siginfo_t, ctx: *mut libc::c_void) {
        let n = N.load(Relaxed);
        if n >= SAMPLE_CAP {
            DROPPED.fetch_add(1, Relaxed);
            return;
        }
        // SAFETY: the kernel gives a valid context; `n` and `nf` are below the capacities the arrays were mapped with; the
        // frame-pointer walk reads only between the interrupted stack pointer and the top of that thread's stack.
        unsafe {
            let (pc, fp, sp) = registers(ctx as *const libc::ucontext_t);
            PCS.load(Relaxed).add(n).write(pc as u64);
            if SOURCE.load(Relaxed) == 0 {
                let now = cpu_ns();
                let since = now.saturating_sub(LAST_CPU_NS.swap(now, Relaxed));
                WEIGHTS.load(Relaxed).add(n).write((since / 1000).min(u32::MAX as u64) as u32);
            }
            let starts = STARTS.load(Relaxed);
            let mut nf = NF.load(Relaxed);
            starts.add(n).write(nf as u32);
            if libc::pthread_self() as usize == SAMPLED_THREAD.load(Relaxed) {
                let frames = FRAMES.load(Relaxed);
                let top = STACK_TOP.load(Relaxed);
                let mut f = fp;
                let mut depth = 0;
                while depth < MAX_DEPTH && nf < FRAME_CAP && f >= sp && f + 16 <= top && f % 8 == 0 {
                    let next = (f as *const usize).read_volatile();
                    let ret = (f as *const usize).add(1).read_volatile();
                    if ret == 0 {
                        break;
                    }
                    frames.add(nf).write(ret as u64);
                    nf += 1;
                    depth += 1;
                    if next <= f {
                        break;
                    }
                    f = next;
                }
            }
            starts.add(n + 1).write(nf as u32);
            NF.store(nf, Relaxed);
            N.store(n + 1, Relaxed);
        }
    }

    fn map_array<T>(count: usize) -> *mut T {
        // SAFETY: an anonymous private mapping with no address hint; the result is checked below.
        let p = unsafe {
            libc::mmap(
                core::ptr::null_mut(),
                count * core::mem::size_of::<T>(),
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_PRIVATE | libc::MAP_ANON,
                -1,
                0,
            )
        };
        if p == libc::MAP_FAILED { core::ptr::null_mut() } else { p.cast() }
    }

    /// The top of this thread's stack; 0 where the system does not say.
    fn stack_top() -> usize {
        #[cfg(target_os = "linux")]
        // SAFETY: the attribute object is initialised by `pthread_getattr_np` before it is read, and destroyed after.
        unsafe {
            let mut attr: libc::pthread_attr_t = core::mem::zeroed();
            if libc::pthread_getattr_np(libc::pthread_self(), &mut attr) == 0 {
                let mut low: *mut libc::c_void = core::ptr::null_mut();
                let mut size: libc::size_t = 0;
                let ok = libc::pthread_attr_getstack(&attr, &mut low, &mut size) == 0;
                libc::pthread_attr_destroy(&mut attr);
                if ok {
                    return low as usize + size;
                }
            }
        }
        0
    }

    /// Starts sampling this thread: on the overflow of a counter if `event` says `instructions:PERIOD` or `cycles:PERIOD`,
    /// otherwise by `setitimer(ITIMER_PROF)` at `hz`.  A rate of 0 and no event starts nothing.
    pub fn sampler_start(hz: u64, event: Option<&[u8]>) {
        if hz == 0 && event.is_none() {
            return;
        }
        let (pcs, weights, starts, frames) =
            (map_array::<u64>(SAMPLE_CAP), map_array::<u32>(SAMPLE_CAP), map_array::<u32>(SAMPLE_CAP + 1), map_array::<u64>(FRAME_CAP));
        if pcs.is_null() || weights.is_null() || starts.is_null() || frames.is_null() {
            return;
        }
        PCS.store(pcs, Relaxed);
        WEIGHTS.store(weights, Relaxed);
        STARTS.store(starts, Relaxed);
        FRAMES.store(frames, Relaxed);
        N.store(0, Relaxed);
        NF.store(0, Relaxed);
        // SAFETY: reads a register.
        SAMPLED_THREAD.store(unsafe { libc::pthread_self() } as usize, Relaxed);
        STACK_TOP.store(stack_top(), Relaxed);
        let set = sampler_signal_set();
        // SAFETY: `on_sigprof` has the signature `SA_SIGINFO` asks for and stays valid for the life of the process; the
        // other arguments are valid for their calls.
        unsafe {
            let mut sa: libc::sigaction = core::mem::zeroed();
            sa.sa_sigaction = on_sigprof as *const () as usize;
            sa.sa_flags = libc::SA_SIGINFO | libc::SA_RESTART;
            libc::sigemptyset(&mut sa.sa_mask);
            libc::sigaction(libc::SIGPROF, &sa, core::ptr::null_mut());
            libc::pthread_sigmask(libc::SIG_UNBLOCK, &set, core::ptr::null_mut());
        }
        #[cfg(target_os = "linux")]
        if let Some(e) = event {
            if start_overflow(e) {
                return;
            }
        }
        #[cfg(not(target_os = "linux"))]
        let _ = event;
        let hz = hz.max(1);
        PERIOD.store(hz, Relaxed);
        LAST_CPU_NS.store(cpu_ns(), Relaxed);
        let t = libc::timeval { tv_sec: 0, tv_usec: (1_000_000 / hz).max(1) as libc::suseconds_t };
        let it = libc::itimerval { it_interval: t, it_value: t };
        // SAFETY: `it` is valid for the call.
        unsafe {
            libc::setitimer(libc::ITIMER_PROF, &it, core::ptr::null_mut());
        }
    }

    /// `name:period`: `SIGPROF` to this thread every `period` user-mode events.
    #[cfg(target_os = "linux")]
    fn start_overflow(event: &[u8]) -> bool {
        let text = String::from_utf8_lossy(event);
        let Some((name, period)) = text.split_once(':') else { return false };
        let Ok(period) = period.trim().parse::<u64>() else { return false };
        let (source, config) = match name {
            "instructions" => (1, 1),
            "cycles" => (2, 0),
            _ => return false,
        };
        let attr = PerfEventAttr {
            type_: 0,
            size: core::mem::size_of::<PerfEventAttr>() as u32,
            config,
            sample_period: period,
            flags: FLAG_DISABLED | FLAG_EXCLUDE_KERNEL | FLAG_EXCLUDE_HV,
            wakeup_events: 1,
            ..Default::default()
        };
        let fd = perf_event_open(&attr);
        if fd < 0 {
            return false;
        }
        // F_SETSIG and F_SETOWN_EX, with `struct f_owner_ex`: the signal goes to this thread.
        const F_SETSIG: libc::c_int = 10;
        const F_SETOWN_EX: libc::c_int = 15;
        const F_OWNER_TID: libc::c_int = 0;
        #[repr(C)]
        struct FOwnerEx {
            type_: libc::c_int,
            pid: libc::pid_t,
        }
        // SAFETY: `fd` is ours, and the owner block is valid for the call.
        unsafe {
            let owner = FOwnerEx { type_: F_OWNER_TID, pid: libc::syscall(libc::SYS_gettid) as libc::pid_t };
            libc::fcntl(fd, libc::F_SETFL, libc::O_ASYNC | libc::O_NONBLOCK);
            libc::fcntl(fd, F_SETSIG, libc::SIGPROF);
            libc::fcntl(fd, F_SETOWN_EX, &owner as *const FOwnerEx);
            libc::ioctl(fd, PERF_EVENT_IOC_ENABLE as _, 0);
        }
        OVERFLOW_FD.store(fd, Relaxed);
        SOURCE.store(source, Relaxed);
        PERIOD.store(period, Relaxed);
        true
    }

    pub fn sampler_stop() {
        #[cfg(target_os = "linux")]
        {
            let fd = OVERFLOW_FD.swap(-1, Relaxed);
            if fd >= 0 {
                // SAFETY: `fd` is ours.
                unsafe {
                    libc::ioctl(fd, PERF_EVENT_IOC_DISABLE as _, 0);
                    libc::close(fd);
                }
            }
        }
        let t = libc::timeval { tv_sec: 0, tv_usec: 0 };
        let it = libc::itimerval { it_interval: t, it_value: t };
        // SAFETY: `it` is valid for the call.
        unsafe {
            libc::setitimer(libc::ITIMER_PROF, &it, core::ptr::null_mut());
        }
        block_sampler_signal();
    }

    /// What to subtract from a sampled address for the address `nm` and `addr2line` know: a static-pie binary is loaded
    /// at a random base.
    fn load_base() -> u64 {
        #[cfg(target_os = "linux")]
        // SAFETY: the kernel's auxiliary vector gives the address and count of the program headers, mapped for the life of
        // the process.
        unsafe {
            let phdr = libc::getauxval(libc::AT_PHDR) as *const libc::Elf64_Phdr;
            let n = libc::getauxval(libc::AT_PHNUM) as usize;
            if !phdr.is_null() {
                for i in 0..n {
                    let h = &*phdr.add(i);
                    if h.p_type == libc::PT_PHDR {
                        return phdr as u64 - h.p_vaddr;
                    }
                }
            }
        }
        0
    }

    /// The raw file: `RSTXPROF`, then u64s version, source, period, samples, dropped, frames, load base; then the program
    /// counters as u64, the microseconds of CPU each stands for as u32 (0 for a counter's samples), `samples + 1` offsets into
    /// the frames as u32, and the frames as u64, all little-endian.
    pub fn sampler_write(path: &[u8]) -> std::io::Result<SamplerInfo> {
        let n = N.load(Relaxed);
        let nf = NF.load(Relaxed);
        let source = SOURCE.load(Relaxed);
        let period = PERIOD.load(Relaxed);
        let dropped = DROPPED.load(Relaxed);
        let mut f = std::io::BufWriter::with_capacity(1 << 20, std::fs::File::create(super::path_from_bytes(path))?);
        f.write_all(b"RSTXPROF")?;
        for v in [2, source, period, n as u64, dropped, nf as u64, load_base()] {
            f.write_all(&v.to_le_bytes())?;
        }
        let (pcs, weights, starts, frames) = (PCS.load(Relaxed), WEIGHTS.load(Relaxed), STARTS.load(Relaxed), FRAMES.load(Relaxed));
        if n > 0 {
            // SAFETY: the sampler has stopped; these prefixes of the mapped arrays were written by it, or are zero.
            unsafe {
                f.write_all(core::slice::from_raw_parts(pcs.cast::<u8>(), n * 8))?;
                f.write_all(core::slice::from_raw_parts(weights.cast::<u8>(), n * 4))?;
                f.write_all(core::slice::from_raw_parts(starts.cast::<u8>(), (n + 1) * 4))?;
                f.write_all(core::slice::from_raw_parts(frames.cast::<u8>(), nf * 8))?;
            }
        }
        f.flush()?;
        Ok(SamplerInfo {
            source: match source {
                1 => "instructions",
                2 => "cycles",
                _ => "itimer",
            },
            period,
            samples: n as u64,
            dropped,
            frames: nf as u64,
        })
    }
}
