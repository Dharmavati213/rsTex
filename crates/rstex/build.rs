//! glibc's static start-up asks `/proc/self/exe` where the program lies, for a `$ORIGIN` that only `dlopen` reads, and the link sends that call to
//! `__wrap__dl_get_origin` in `main.rs`, which says it does not know: see there.  Only a static link has the call to send.  With a dynamic one the dynamic loader
//! makes it, the option finds no reference, and it does nothing.
fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    let os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let env = std::env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
    if os == "linux" && env == "gnu" {
        println!("cargo:rustc-link-arg-bins=-Wl,--wrap=_dl_get_origin");
    }
}
