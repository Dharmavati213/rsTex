//! The `rstex` command.  All of it is `lib.rs`.
//!
//! `main` is the C runtime's, without Rust's start-up around it, which costs some 25 system calls before anything happens: it reads `/proc/self/maps`, and sets up
//! a handler for a stack overflow, which here would only change how a crash looks.  What of it matters is in `platform::process_init`.
#![cfg_attr(not(test), no_main)]
#![deny(unsafe_code)]

use rstex_engine::platform;

/// The C runtime's call.
#[cfg(not(test))]
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
extern "C" fn main(_argc: i32, _argv: *const *const std::ffi::c_char) -> i32 {
    rstex_engine::prof_main_start!();
    platform::process_init();
    let args: Vec<Vec<u8>> = std::env::args_os().map(|a| platform::path_to_bytes(std::path::Path::new(&a))).collect();
    let code = platform::run_on_stack(rstex_driver::ENGINE_STACK, move || rstex_driver::run(args, None));
    rstex_engine::prof_finish!();
    std::process::exit(code);
}
