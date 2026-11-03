//! The build id: `RSTEX_BUILD_ID`, which `image.rs` puts in every format, a hash of what decides what a format means and how it is laid out.  That is every
//! source file of the crates that end up in the engine, the lock file, the compiler and the features.  It says nothing of flags that only change speed,
//! so a profile-guided binary and a plain one of the same sources read each other's formats, as they may.
use std::path::{Path, PathBuf};

const CRATES: [&str; 5] = ["engine", "kpse", "fontfile", "pdfio", "image"];

fn fnv(mut h: u64, s: &[u8]) -> u64 {
    for &b in s {
        h = (h ^ b as u64).wrapping_mul(0x100000001b3);
    }
    h
}

fn sources(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            sources(&p, out);
        } else {
            out.push(p);
        }
    }
}

fn main() {
    let here = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").expect("cargo says"));
    let root = here.parent().and_then(Path::parent).expect("crates/engine in a workspace");
    let mut files = vec![root.join("Cargo.lock"), root.join("Cargo.toml")];
    for c in CRATES {
        let d = root.join("crates").join(c);
        files.push(d.join("Cargo.toml"));
        println!("cargo:rerun-if-changed={}", d.join("src").display());
        sources(&d.join("src"), &mut files);
    }
    files.push(here.join("build.rs"));
    files.sort();
    let mut h = 0xcbf29ce484222325;
    for f in &files {
        println!("cargo:rerun-if-changed={}", f.display());
        // By the path inside the tree, so that a worktree or another checkout of the same sources has the same id.
        h = fnv(h, f.strip_prefix(root).unwrap_or(f).to_string_lossy().as_bytes());
        h = fnv(h, &std::fs::read(f).unwrap_or_default());
    }
    let rustc = std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into());
    if let Ok(v) = std::process::Command::new(rustc).arg("-vV").output() {
        h = fnv(h, &v.stdout);
    }
    let mut vars: Vec<String> = std::env::vars().filter(|(k, _)| k.starts_with("CARGO_FEATURE_") || k == "TARGET").map(|(k, v)| format!("{k}={v}")).collect();
    vars.sort();
    for v in vars {
        h = fnv(h, v.as_bytes());
    }
    println!("cargo:rustc-env=RSTEX_BUILD_ID={h:016x}");
}
