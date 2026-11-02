//! A subset of kpathsea, and later the binary path index (DESIGN.md section 6, M2).
//!
//! The interface is what `rstex-engine`'s World calls.  At M0 the search covers only the current
//! directory.  It needs TeX Live's data files but none of its binaries; finding the tree follows
//! DESIGN.md section 12.  With `platform/` in the engine, this crate is where OS differences may live.
#![forbid(unsafe_code)]

/// Kpathsea's format codes, as used by WEB and openclose.c.
pub const KPSE_TEX_FORMAT: i32 = 26;
pub const KPSE_TFM_FORMAT: i32 = 3;
pub const KPSE_VF_FORMAT: i32 = 33;
pub const KPSE_FMT_FORMAT: i32 = 10;

fn suffix(format: i32) -> &'static [u8] {
    match format {
        KPSE_TEX_FORMAT => b".tex",
        KPSE_TFM_FORMAT => b".tfm",
        KPSE_VF_FORMAT => b".vf",
        KPSE_FMT_FORMAT => b".fmt",
        _ => b"",
    }
}

/// TeX sees file names as bytes: on Unix they are the name, on Windows UTF-8.
#[cfg(unix)]
fn path_of(p: &[u8]) -> std::path::PathBuf {
    use std::os::unix::ffi::OsStrExt;
    std::path::PathBuf::from(std::ffi::OsStr::from_bytes(p))
}

#[cfg(windows)]
fn path_of(p: &[u8]) -> std::path::PathBuf {
    std::path::PathBuf::from(String::from_utf8_lossy(p).as_ref())
}

fn exists(p: &[u8]) -> bool {
    std::fs::metadata(path_of(p)).map(|m| m.is_file()).unwrap_or(false)
}

/// `kpse_find_file`.  `must_exist` asks for a search of the disk rather than only the databases.
pub fn find_file(name: &[u8], format: i32, _must_exist: bool) -> Option<Vec<u8>> {
    let s = suffix(format);
    // TeX tries name.suffix first for .tex, and the name as given next.
    let mut with = name.to_vec();
    with.extend_from_slice(s);
    if !name.ends_with(s) && exists(&with) {
        return Some(with);
    }
    if exists(name) {
        return Some(name.to_vec());
    }
    None
}

/// `kpse_in_name_ok`: `openin_any = a` by default.
pub fn in_name_ok(_name: &[u8]) -> bool {
    true
}

/// `kpse_out_name_ok`: `openout_any = p` by default: no dotfiles, no parent directories.
pub fn out_name_ok(name: &[u8]) -> bool {
    let base = name.rsplit(|&c| c == b'/').next().unwrap_or(name);
    if base.starts_with(b".") && base != b"." && base != b".." {
        return false;
    }
    !name.split(|&c| c == b'/').any(|c| c == b"..")
}

/// A boolean from `texmf.cnf`.
pub fn yesno(_var: &[u8]) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn out_names() {
        assert!(out_name_ok(b"a.tex"));
        assert!(out_name_ok(b"sub/a.tex"));
        assert!(!out_name_ok(b".rc"));
        assert!(!out_name_ok(b"../a.tex"));
    }

    #[test]
    fn missing_file() {
        assert_eq!(find_file(b"/rstex/surely/not/here", KPSE_TEX_FORMAT, true), None);
    }
}
