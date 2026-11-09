//! The `rstex` command.  All of it is `lib.rs`.
//!
//! `main` is the C runtime's, without Rust's start-up around it, which costs some 25 system calls before anything happens: it reads `/proc/self/maps`, and sets up
//! a handler for a stack overflow on a stack that is not the engine's.  What of it matters is in `platform::process_init` and `platform::run_on_stack`.
#![cfg_attr(not(test), no_main)]
#![deny(unsafe_code)]

use rstex_engine::platform;

/// What `crates/rstex/build.rs` has glibc's static start-up call instead of `_dl_get_origin`, a `readlink` of `/proc/self/exe` and the run's first look at
/// `/proc/self`, some 12 us of a plain hello's 1.8 ms.  `(char *) -1` is what glibc's own gives when it cannot tell, and what it would tell is `$ORIGIN` for the
/// search paths of `dlopen`.  The binary has a `dlopen`: iconv's and gettext's code reach it, to load a converter or a catalog for a locale other than C.  Nothing
/// here leaves the C locale, as pdfTeX does not: no `setlocale`, and `~user` is read from `/etc/passwd`, not asked of NSS.  A dependency that brings in a `dlopen`
/// that can be reached would find `$ORIGIN` no longer expanded in `LD_LIBRARY_PATH`, and the wrap would have to go.
#[cfg(all(target_os = "linux", target_env = "gnu"))]
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
extern "C" fn __wrap__dl_get_origin() -> *const std::ffi::c_char {
    usize::MAX as *const std::ffi::c_char
}

/// The C runtime's call.
#[cfg(not(test))]
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
extern "C" fn main(_argc: i32, _argv: *const *const std::ffi::c_char) -> i32 {
    rstex_engine::prof_main_start!();
    std::panic::set_hook(Box::new(rstex_driver::panic_message));
    platform::process_init();
    let args: Vec<Vec<u8>> = std::env::args_os().map(|a| platform::path_to_bytes(std::path::Path::new(&a))).collect();
    let code = platform::run_on_stack(rstex_driver::ENGINE_STACK, move || rstex_driver::run(args, None));
    rstex_engine::prof_finish!();
    std::process::exit(code);
}
