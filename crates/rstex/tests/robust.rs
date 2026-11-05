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

/// `tools/ci/check_tree.py` with a program that says what it is told to: it warns of what is off and of what gives no answer, a crash among those, and never fails the build.
#[cfg(unix)]
#[test]
fn the_warning_on_ci_fires_and_fails_nothing() {
    use std::os::unix::fs::PermissionsExt;
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tools/ci/check_tree.py");
    if Command::new("python3").arg("--version").output().is_err() {
        return;
    }
    let d = scratch("ci-warning");
    let run = |program: &Path| {
        let o = Command::new("python3").arg("-I").arg(&script).arg(program).stdin(Stdio::null()).output().unwrap();
        (o.status.code(), String::from_utf8_lossy(&o.stdout).into_owned())
    };
    for (name, body, warning) in [
        ("armed", "echo path=tikz-cd state=armed profile=x; exit 0", None),
        ("off", "echo 'path=tikz-cd state=off why=100% of it'; exit 1", Some("::warning title=A fast path is off on TeX Live of today::path=tikz-cd state=off why=100%25 of it.")),
        ("nothing", "echo 'rstex: check-tree: no answer: x'; exit 2", Some("::warning title=rstex --check-tree has no answer::status 2: rstex: check-tree: no answer: x")),
        ("panic", "echo thread panicked >&2; exit 101", Some("::warning title=rstex --check-tree has no answer::status 101: thread panicked")),
        ("crash", "kill -SEGV $$", Some("::warning title=rstex --check-tree has no answer::status -11")),
    ] {
        let p = d.join(name);
        std::fs::write(&p, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        let (status, out) = run(&p);
        assert_eq!(status, Some(0), "{name}: {out}");
        match warning {
            Some(w) => assert!(out.lines().any(|l| l.starts_with(w)), "{name}: {out}"),
            None => assert!(!out.contains("::warning"), "{name}: {out}"),
        }
    }
    let (status, out) = run(&d.join("no-such-program"));
    assert_eq!(status, Some(0), "{out}");
    assert!(out.starts_with("::warning title=rstex --check-tree has no answer::"), "{out}");
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
