//! What happens to a PDF on the way to the disk: what the oracle's `fopen` and `fwrite` leave behind is what has to be left behind, though here the file is written at
//! the end, by a temporary and a rename.  Each of the cases of `PdfOut::finish`, against what `/usr/bin/pdftex` was seen to do: the numbers of the run are in docs/BUILD.md.
use rstex_engine::host::PdfOut;
use rstex_engine::platform::{path_from_bytes, path_to_bytes};
use std::path::{Path, PathBuf};

fn scratch(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("rstex-output-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn pdf(path: &Path, bytes: &[u8]) -> PdfOut {
    let mut p = PdfOut::new(&path_to_bytes(path));
    p.write(bytes);
    p
}

fn names(d: &Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(d).unwrap().flatten().map(|e| e.file_name().to_string_lossy().to_string()).collect();
    v.sort();
    v
}

#[test]
fn the_file_appears_whole_and_no_temporary_stays() {
    let d = scratch("whole");
    let f = d.join("a.pdf");
    pdf(&f, b"%PDF new").finish().unwrap();
    assert_eq!(std::fs::read(&f).unwrap(), b"%PDF new");
    assert_eq!(names(&d), ["a.pdf"]);
    // Over one that is there.
    pdf(&f, b"%PDF newer").finish().unwrap();
    assert_eq!(std::fs::read(&f).unwrap(), b"%PDF newer");
    assert_eq!(names(&d), ["a.pdf"]);
    std::fs::remove_dir_all(&d).unwrap();
}

#[test]
fn a_directory_of_that_name_is_an_error_and_says_there_are_no_bytes() {
    let d = scratch("dir");
    let f = d.join("a.pdf");
    std::fs::create_dir(&f).unwrap();
    let e = pdf(&f, b"%PDF").finish().unwrap_err();
    assert!(e.kept_in.is_none());
    assert_eq!(names(&d), ["a.pdf"]);
    std::fs::remove_dir_all(&d).unwrap();
}

#[test]
fn a_name_with_non_ascii_and_spaces_goes_through() {
    let d = scratch("names");
    let f = d.join("caf\u{e9} \u{4e2d}\u{6587}.pdf");
    let bytes = path_to_bytes(&f);
    assert_eq!(path_from_bytes(&bytes), f);
    pdf(&f, b"%PDF").finish().unwrap();
    assert_eq!(std::fs::read(&f).unwrap(), b"%PDF");
    std::fs::remove_dir_all(&d).unwrap();
}

#[cfg(unix)]
mod unix {
    use super::*;
    use std::os::unix::fs::{PermissionsExt, symlink};

    fn mode(p: &Path) -> u32 {
        std::fs::metadata(p).unwrap().permissions().mode() & 0o777
    }

    #[test]
    fn the_file_keeps_its_mode() {
        let d = scratch("mode");
        let f = d.join("a.pdf");
        std::fs::write(&f, b"old").unwrap();
        std::fs::set_permissions(&f, std::fs::Permissions::from_mode(0o600)).unwrap();
        pdf(&f, b"%PDF").finish().unwrap();
        assert_eq!(std::fs::read(&f).unwrap(), b"%PDF");
        assert_eq!(mode(&f), 0o600);
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_link_is_written_through() {
        let d = scratch("link");
        let (f, target) = (d.join("a.pdf"), d.join("elsewhere.pdf"));
        std::fs::write(&target, b"old").unwrap();
        symlink("elsewhere.pdf", &f).unwrap();
        pdf(&f, b"%PDF").finish().unwrap();
        assert!(std::fs::symlink_metadata(&f).unwrap().file_type().is_symlink());
        assert_eq!(std::fs::read(&target).unwrap(), b"%PDF");
        assert_eq!(names(&d), ["a.pdf", "elsewhere.pdf"]);
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_second_name_stays_the_same_file() {
        let d = scratch("hard");
        let (f, other) = (d.join("a.pdf"), d.join("b.pdf"));
        std::fs::write(&f, b"old").unwrap();
        std::fs::hard_link(&f, &other).unwrap();
        pdf(&f, b"%PDF").finish().unwrap();
        assert_eq!(std::fs::read(&other).unwrap(), b"%PDF");
        std::fs::remove_dir_all(&d).unwrap();
    }

    /// There is no room for a temporary, and the file can be written: `chmod a-w .`, after a run that made the file.
    #[test]
    fn a_directory_that_takes_no_new_file() {
        let d = scratch("readonly");
        let f = d.join("a.pdf");
        std::fs::write(&f, b"old").unwrap();
        std::fs::set_permissions(&d, std::fs::Permissions::from_mode(0o555)).unwrap();
        // Root may write anywhere, and then this shows nothing.
        let r = pdf(&f, b"%PDF").finish();
        std::fs::set_permissions(&d, std::fs::Permissions::from_mode(0o755)).unwrap();
        r.unwrap();
        assert_eq!(std::fs::read(&f).unwrap(), b"%PDF");
        assert_eq!(names(&d), ["a.pdf"]);
        std::fs::remove_dir_all(&d).unwrap();
    }

    /// `/dev/full`, through a link: what a full disk gives.  Nothing is claimed to be anywhere.
    #[cfg(target_os = "linux")]
    #[test]
    fn a_full_disk_is_an_error() {
        let d = scratch("full");
        let f = d.join("a.pdf");
        symlink("/dev/full", &f).unwrap();
        let e = pdf(&f, b"%PDF").finish().unwrap_err();
        assert_eq!(e.error.raw_os_error(), Some(28), "{}", e.error);
        assert!(e.kept_in.is_none());
        assert_eq!(names(&d), ["a.pdf"]);
        std::fs::remove_dir_all(&d).unwrap();
    }
}

/// What a viewer does on Windows: it holds the file with no sharing.  Neither the rename nor the writing in place can get past that, and then the PDF is in the temporary, which the error names.
#[cfg(windows)]
#[test]
fn a_file_held_open_keeps_the_bytes_under_another_name() {
    use std::os::windows::fs::OpenOptionsExt;
    let d = scratch("held");
    let f = d.join("a.pdf");
    std::fs::write(&f, b"old").unwrap();
    let viewer = std::fs::OpenOptions::new().read(true).share_mode(0).open(&f).unwrap();
    let e = pdf(&f, b"%PDF").finish().unwrap_err();
    drop(viewer);
    let kept = e.kept_in.expect("the bytes are somewhere");
    assert_eq!(std::fs::read(path_from_bytes(&kept)).unwrap(), b"%PDF");
    assert_eq!(std::fs::read(&f).unwrap(), b"old");
    std::fs::remove_dir_all(&d).unwrap();
}
