//! What the program does when the world does not cooperate, through the real program.  None of this needs a TeX tree: `-ini` loads no format.
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn scratch(name: &str) -> PathBuf {
    let d = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("robust-{name}"));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn ini(work: &Path, first_line: &str) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_rstex"));
    c.args(["-ini", "-no-snapshot", first_line]).current_dir(work).env("XDG_CACHE_HOME", work.join("cache")).stdin(Stdio::null());
    c
}

/// A log that cannot be opened: what TeX says, and nothing worse.  Windows has no read-only directory in this sense.
#[cfg(unix)]
#[test]
fn a_directory_that_takes_no_file_stops_the_job_with_tex_own_words() {
    use std::os::unix::fs::PermissionsExt;
    let d = scratch("readonly");
    std::fs::set_permissions(&d, std::fs::Permissions::from_mode(0o555)).unwrap();
    let o = ini(&d, "\\end").env("XDG_CACHE_HOME", d.parent().unwrap().join("robust-readonly-cache")).output().unwrap();
    std::fs::set_permissions(&d, std::fs::Permissions::from_mode(0o755)).unwrap();
    let text = String::from_utf8_lossy(&o.stdout).to_string();
    assert!(text.contains("I can't write on file `texput.log'"), "{text}");
    assert_eq!(o.status.code(), Some(1), "{text}");
}

/// `\openout` on a name that cannot be opened goes the way of §1374: the prompt, and in batch mode the end of the job.
#[test]
fn an_openout_that_fails_asks_for_another_name() {
    let d = scratch("openout");
    let o = ini(&d, "\\batchmode\\immediate\\openout3=no/such/directory/x \\end").output().unwrap();
    let log = String::from_utf8_lossy(&std::fs::read(d.join("texput.log")).unwrap()).to_string();
    assert!(log.contains("*** (job aborted, file error in nonstop mode)"), "{log}");
    assert_eq!(o.status.code(), Some(1), "{log}");
}

/// Control-C: TeX stops at the next token and says so, as the oracle's handler has it, and the job does not just die with its log unwritten.
#[cfg(unix)]
#[test]
fn an_interrupt_gets_tex_interruption() {
    let d = scratch("interrupt");
    use std::io::Read;
    let mut child = ini(&d, "\\catcode`\\{=1 \\catcode`\\}=2 \\def\\a{\\a}\\a").stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
    // The banner is out, so the job has started: the handler comes a moment later, and then the loop, where nothing shows from outside until it ends.
    let mut out = child.stdout.take().unwrap();
    let mut text = Vec::new();
    let mut byte = [0];
    while !text.contains(&b'\n') && out.read(&mut byte).unwrap() == 1 {
        text.push(byte[0]);
    }
    std::thread::sleep(std::time::Duration::from_millis(200));
    Command::new("kill").args(["-INT", &child.id().to_string()]).status().unwrap();
    out.read_to_end(&mut text).unwrap();
    let o = child.wait_with_output().unwrap();
    let text = String::from_utf8_lossy(&text).to_string();
    assert!(text.contains("! Interruption."), "{text}");
    // No terminal to answer on.
    assert!(text.contains("End of file on the terminal") || text.contains("Emergency stop"), "{text}");
    assert_eq!(o.status.code(), Some(1), "{text}");
}

/// A format on a full disk: no format comes of it, which the next run would then find and call bad.  The temporary it writes to is a link to `/dev/full`.
#[cfg(target_os = "linux")]
#[test]
fn a_format_that_cannot_be_written_is_no_format() {
    let d = scratch("full-format");
    std::os::unix::fs::symlink("/dev/full", d.join("x.fmt.rstex-tmp")).unwrap();
    let mut c = Command::new(env!("CARGO_BIN_EXE_rstex"));
    c.args(["-ini", "-no-snapshot", "-jobname=x", "\\dump"]).current_dir(&d).env("XDG_CACHE_HOME", d.join("cache")).stdin(Stdio::null());
    let o = c.output().unwrap();
    let err = String::from_utf8_lossy(&o.stderr).to_string();
    assert!(err.contains("! Could not write"), "{err}");
    assert_eq!(o.status.code(), Some(1), "{err}");
    assert!(!d.join("x.fmt").exists());
}
