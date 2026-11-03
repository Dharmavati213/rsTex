//! The profiler's hooks in the program: `tools/prof/rstex_prof.py` is the other half, and its docstring says how to use them.
//!
//! Behind the cargo feature `prof`, which no shipped build has.  Without it every macro below expands to nothing, and this
//! file holds nothing but those macros.  With it, and with no `RSTEX_PROF*` variable set, the program runs as it does without
//! the feature and prints nothing, which is how `tests/integration/run.py` can judge a prof build.
//!
//! | Variable | Meaning |
//! |---|---|
//! | `RSTEX_PROF_OUT` | The sampler runs, and its raw samples go to this file at exit |
//! | `RSTEX_PROF_COUNTERS` | Counters, phases and scopes go to this file; if any `RSTEX_PROF*` variable is set without it, to stderr |
//! | `RSTEX_PROF` | Set to anything: counters, phases and scopes to stderr, no sampler |
//! | `RSTEX_PROF_HZ` | `setitimer` rate, default 2000.  The kernel fires it at most once per tick, 1000 Hz on this machine's kernel |
//! | `RSTEX_PROF_EVENT` | `instructions:PERIOD`, `cycles:PERIOD` or `l1d-misses:PERIOD`: sample on that counter's overflow instead of the timer (Linux) |
//! | `RSTEX_PROF_T0` | `CLOCK_MONOTONIC` in ns as the spawner read it just before `exec`, the zero of the phase table |
//!
//! The operating system's part is in `platform/`, as everywhere: on Windows all of this does nothing.

/// Called first in `main`.
#[cfg(not(feature = "prof"))]
#[macro_export]
macro_rules! prof_main_start {
    () => {};
}
/// Called first on the engine thread.
#[cfg(not(feature = "prof"))]
#[macro_export]
macro_rules! prof_engine_start {
    () => {};
}
/// Stops everything and writes the results; the process is about to exit.
#[cfg(not(feature = "prof"))]
#[macro_export]
macro_rules! prof_finish {
    () => {};
}
/// A moment of the run, kept the first time it is reached.
#[cfg(not(feature = "prof"))]
#[macro_export]
macro_rules! prof_phase {
    ($name:expr) => {};
}
/// A moment of the run, kept the last time it is reached.
#[cfg(not(feature = "prof"))]
#[macro_export]
macro_rules! prof_phase_last {
    ($name:expr) => {};
}
/// `let _p = prof_scope!("name");` adds the time until the end of the block to that name's total.
#[cfg(not(feature = "prof"))]
#[macro_export]
macro_rules! prof_scope {
    ($name:expr) => {
        ()
    };
}
/// Once a format is in: finds the control sequence `\document`, which `prof_macro_call` watches for.
#[cfg(not(feature = "prof"))]
#[macro_export]
macro_rules! prof_after_format {
    ($tex:expr) => {};
}
/// At the top of `macro_call`: `\document` has no other mark in the engine.
#[cfg(not(feature = "prof"))]
#[macro_export]
macro_rules! prof_macro_call {
    ($tex:expr) => {};
}

#[cfg(feature = "prof")]
#[macro_export]
macro_rules! prof_main_start {
    () => {
        $crate::prof::main_start()
    };
}
#[cfg(feature = "prof")]
#[macro_export]
macro_rules! prof_engine_start {
    () => {
        $crate::prof::engine_start()
    };
}
#[cfg(feature = "prof")]
#[macro_export]
macro_rules! prof_finish {
    () => {
        $crate::prof::finish()
    };
}
#[cfg(feature = "prof")]
#[macro_export]
macro_rules! prof_phase {
    ($name:expr) => {
        $crate::prof::phase($name, false)
    };
}
#[cfg(feature = "prof")]
#[macro_export]
macro_rules! prof_phase_last {
    ($name:expr) => {
        $crate::prof::phase($name, true)
    };
}
#[cfg(feature = "prof")]
#[macro_export]
macro_rules! prof_scope {
    ($name:expr) => {
        $crate::prof::scope($name)
    };
}
#[cfg(feature = "prof")]
#[macro_export]
macro_rules! prof_after_format {
    ($tex:expr) => {
        $tex.prof_after_format()
    };
}
#[cfg(feature = "prof")]
#[macro_export]
macro_rules! prof_macro_call {
    ($tex:expr) => {
        if $tex.cur_cs == $crate::prof::document_cs() {
            $crate::prof::document_reached();
        }
    };
}

#[cfg(feature = "prof")]
pub use on::*;

#[cfg(feature = "prof")]
mod on {
    #![forbid(unsafe_code)]
    use crate::platform::{self, prof as sys};
    use std::fmt::Write as _;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicBool, AtomicI32, Ordering::Relaxed};

    struct Phase {
        name: &'static str,
        t_ns: u64,
        cpu_ns: u64,
        instructions: u64,
        cycles: u64,
        minflt: u64,
        majflt: u64,
    }

    struct Scope {
        name: &'static str,
        count: u64,
        total_ns: u64,
        instructions: u64,
        minflt: u64,
        /// More than 0 inside it: a scope re-entered counts as the outer one.
        depth: u32,
        began_ns: u64,
        began_instructions: u64,
        began_minflt: u64,
    }

    struct State {
        t0_ns: Option<u64>,
        main_ns: u64,
        phases: Vec<Phase>,
        scopes: Vec<Scope>,
        out: Option<Vec<u8>>,
        counters_to: Option<Vec<u8>>,
    }

    /// False leaves every hook at one load and one branch, and the program as it would be.
    static ON: AtomicBool = AtomicBool::new(false);
    /// The control sequence to watch for, or a value no control sequence has.
    static DOCUMENT_CS: AtomicI32 = AtomicI32::new(-1);
    static STATE: Mutex<Option<State>> = Mutex::new(None);

    fn env_u64(name: &[u8]) -> Option<u64> {
        std::str::from_utf8(&platform::env_var(name)?).ok()?.trim().parse().ok()
    }

    pub fn main_start() {
        if !sys::AVAILABLE {
            return;
        }
        let main_ns = sys::now_ns();
        let out = platform::env_var(b"RSTEX_PROF_OUT");
        let counters_to = platform::env_var(b"RSTEX_PROF_COUNTERS");
        if out.is_none() && counters_to.is_none() && platform::env_var(b"RSTEX_PROF").is_none() {
            return;
        }
        sys::block_sampler_signal();
        *lock() = Some(State {
            t0_ns: env_u64(b"RSTEX_PROF_T0"),
            main_ns,
            phases: Vec::with_capacity(64),
            scopes: Vec::with_capacity(64),
            out,
            counters_to,
        });
        ON.store(true, Relaxed);
        sys::counters_open();
        // "main entered" is the time on entry; opening the counters shows as a row of its own, so no phase of the program pays for it.
        phase("main entered", false);
        if let Some(s) = lock().as_mut() {
            s.phases[0].t_ns = main_ns;
        }
        phase("counters opened", false);
    }

    pub fn engine_start() {
        if !ON.load(Relaxed) {
            return;
        }
        phase("engine thread started", false);
        let wants_samples = lock().as_ref().is_some_and(|s| s.out.is_some());
        if wants_samples {
            let event = platform::env_var(b"RSTEX_PROF_EVENT");
            sys::sampler_start(env_u64(b"RSTEX_PROF_HZ").unwrap_or(2000), event.as_deref());
        }
    }

    fn lock() -> std::sync::MutexGuard<'static, Option<State>> {
        STATE.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn phase(name: &'static str, keep_last: bool) {
        if !ON.load(Relaxed) {
            return;
        }
        let mut guard = lock();
        let Some(s) = guard.as_mut() else { return };
        let at = s.phases.iter().position(|p| core::ptr::eq(p.name, name) || p.name == name);
        if at.is_some() && !keep_last {
            return;
        }
        let (minflt, majflt) = sys::faults();
        let p = Phase { name, t_ns: sys::now_ns(), cpu_ns: sys::cpu_ns(), instructions: sys::instructions(), cycles: sys::cycles(), minflt, majflt };
        match at {
            Some(i) => {
                // Keeps the table in time order for a phase that recurs.
                s.phases.remove(i);
                s.phases.push(p);
            }
            None => s.phases.push(p),
        }
    }

    /// Ends a scope when dropped.
    pub struct ScopeGuard(Option<usize>);

    pub fn scope(name: &'static str) -> ScopeGuard {
        if !ON.load(Relaxed) {
            return ScopeGuard(None);
        }
        let mut guard = lock();
        let Some(s) = guard.as_mut() else { return ScopeGuard(None) };
        let i = match s.scopes.iter().position(|x| core::ptr::eq(x.name, name) || x.name == name) {
            Some(i) => i,
            None => {
                s.scopes.push(Scope {
                    name,
                    count: 0,
                    total_ns: 0,
                    instructions: 0,
                    minflt: 0,
                    depth: 0,
                    began_ns: 0,
                    began_instructions: 0,
                    began_minflt: 0,
                });
                s.scopes.len() - 1
            }
        };
        let x = &mut s.scopes[i];
        x.depth += 1;
        if x.depth == 1 {
            x.count += 1;
            x.began_minflt = sys::faults().0;
            x.began_instructions = sys::instructions();
            x.began_ns = sys::now_ns();
        }
        ScopeGuard(Some(i))
    }

    impl Drop for ScopeGuard {
        fn drop(&mut self) {
            let Some(i) = self.0 else { return };
            let t = sys::now_ns();
            let mut guard = lock();
            let Some(s) = guard.as_mut() else { return };
            let x = &mut s.scopes[i];
            x.depth -= 1;
            if x.depth == 0 {
                x.total_ns += t.saturating_sub(x.began_ns);
                x.instructions += sys::instructions().saturating_sub(x.began_instructions);
                x.minflt += sys::faults().0.saturating_sub(x.began_minflt);
            }
        }
    }

    #[inline(always)]
    pub fn document_cs() -> i32 {
        DOCUMENT_CS.load(Relaxed)
    }

    #[cold]
    pub fn document_reached() {
        DOCUMENT_CS.store(-1, Relaxed);
        phase("\\begin{document} reached", false);
    }

    impl crate::state::Tex {
        /// `id_lookup` with `no_new_control_sequence` on changes nothing, and the end of `buffer` holds nothing now: there is
        /// one line in it, the command line.  What it overwrites is put back anyway.
        pub fn prof_after_format(&mut self) {
            if !ON.load(Relaxed) || !self.no_new_control_sequence || self.buf_size < 64 {
                return;
            }
            const NAME: &[u8] = b"document";
            let at = self.buf_size - NAME.len() as i32 - 1;
            if at <= self.last {
                return;
            }
            let mut kept = [0; NAME.len()];
            for (i, &c) in NAME.iter().enumerate() {
                kept[i] = self.buffer.at(at + i as i32);
                self.buffer.put(at + i as i32, c as i32);
            }
            let p = self.id_lookup(at, NAME.len() as i32);
            for (i, &c) in kept.iter().enumerate() {
                self.buffer.put(at + i as i32, c);
            }
            if p != crate::consts::UNDEFINED_CONTROL_SEQUENCE {
                DOCUMENT_CS.store(p, Relaxed);
            }
        }
    }

    /// Runs once; the process exits right after, by `process::exit` or by returning from `main`.
    pub fn finish() {
        if !ON.load(Relaxed) {
            return;
        }
        phase("process exit", false);
        ON.store(false, Relaxed);
        DOCUMENT_CS.store(-1, Relaxed);
        sys::sampler_stop();
        let counters = sys::counters_read();
        let Some(s) = lock().take() else { return };
        let t = sys::now_ns();
        let mut text = String::with_capacity(4096);
        let _ = writeln!(text, "rstex-prof\t1");
        if let Some(t0) = s.t0_ns {
            let _ = writeln!(text, "clock\tt0_abs_ns\t{t0}");
        }
        let _ = writeln!(text, "clock\tmain_abs_ns\t{}", s.main_ns);
        let zero = s.t0_ns.unwrap_or(s.main_ns);
        let (cpu0, i0, c0, flt0) = s.phases.first().map_or((0, 0, 0, 0), |p| (p.cpu_ns, p.instructions, p.cycles, p.minflt));
        let _ = writeln!(text, "#phase\tname\tt_ns\tcpu_ns\tinstructions\tcycles\tminflt\tmajflt");
        for p in &s.phases {
            let _ = writeln!(
                text,
                "phase\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
                p.name,
                p.t_ns.saturating_sub(zero),
                p.cpu_ns.saturating_sub(cpu0),
                p.instructions.saturating_sub(i0),
                p.cycles.saturating_sub(c0),
                p.minflt.saturating_sub(flt0),
                p.majflt,
            );
        }
        let _ = writeln!(text, "#scope\tname\tcount\ttotal_ns\tinstructions\tminflt");
        for x in &s.scopes {
            let _ = writeln!(text, "scope\t{}\t{}\t{}\t{}\t{}", x.name, x.count, x.total_ns, x.instructions, x.minflt);
        }
        for (name, ns, count) in rstex_kpse::prof::take() {
            let _ = writeln!(text, "scope\t{name}\t{count}\t{ns}\t0\t0");
        }
        let _ = writeln!(text, "#counter\tname\tvalue\tenabled_ns\trunning_ns");
        for c in &counters {
            let _ = writeln!(text, "counter\t{}\t{}\t{}\t{}", c.name, c.value, c.enabled_ns, c.running_ns);
        }
        let (minflt, majflt) = sys::faults();
        let _ = writeln!(text, "rusage\tminflt\t{minflt}");
        let _ = writeln!(text, "rusage\tmajflt\t{majflt}");
        if let Some(path) = &s.out {
            match sys::sampler_write(path) {
                Ok(info) => {
                    let _ = writeln!(text, "sampler\tsource\t{}", info.source);
                    let _ = writeln!(text, "sampler\tperiod\t{}", info.period);
                    let _ = writeln!(text, "sampler\tsamples\t{}", info.samples);
                    let _ = writeln!(text, "sampler\tdropped\t{}", info.dropped);
                    let _ = writeln!(text, "sampler\tframes\t{}", info.frames);
                }
                Err(e) => {
                    let _ = writeln!(text, "sampler\terror\t{e}");
                }
            }
        }
        // What the runner subtracts from the time of the reap: what is left is `exit` and the kernel's teardown.
        let _ = writeln!(text, "clock\tfinish_begin_abs_ns\t{t}");
        let _ = writeln!(text, "clock\tfinish_abs_ns\t{}", sys::now_ns());
        match &s.counters_to {
            Some(path) => {
                let _ = std::fs::write(platform::path_from_bytes(path), text.as_bytes());
            }
            None => {
                use std::io::Write as _;
                let _ = std::io::stderr().write_all(text.as_bytes());
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        /// The one test of this feature that runs in the crate's shared process, since a sampler and a counter are per process.
        #[test]
        fn samples_reach_the_file_with_the_program_counter_in_them() {
            if !sys::AVAILABLE {
                return;
            }
            let dir = std::env::temp_dir().join(format!("rstex-prof-{}", std::process::id()));
            std::fs::create_dir_all(&dir).unwrap();
            let path = dir.join("samples");
            sys::sampler_start(1000, None);
            let t = sys::now_ns();
            let mut x = 0u64;
            let c = sys::cpu_ns();
            while sys::cpu_ns() - c < 100_000_000 && sys::now_ns() - t < 5_000_000_000 {
                x = x.wrapping_mul(31).wrapping_add(1);
            }
            std::hint::black_box(x);
            sys::sampler_stop();
            let info = sys::sampler_write(&platform::path_to_bytes(&path)).unwrap();
            assert!(info.samples > 0, "no samples in 100 ms of CPU");
            let bytes = std::fs::read(&path).unwrap();
            assert_eq!(&bytes[..8], b"RSTXPROF");
            assert_eq!(bytes[8], 2, "version");
            std::fs::remove_dir_all(&dir).unwrap();
        }

        #[test]
        fn clocks_move_forward() {
            if !sys::AVAILABLE {
                return;
            }
            let a = sys::now_ns();
            let b = sys::now_ns();
            assert!(a > 0 && b >= a);
        }
    }
}
