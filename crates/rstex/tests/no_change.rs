//! A save with nothing changed: `--until-stable` answers it with no pass and nothing written, and each thing that a pass would see differently means a pass.  With no TeX tree, as `until_stable.rs`; `tests/until_stable/run.py` has the same under LaTeX, with a checkpoint under it.
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

const CATCODES: &str = "\\catcode`\\{=1 \\catcode`\\}=2 \\catcode`\\#=6 \\catcode`\\^=7 \\scrollmode\n";
const PAGE: &str = "\\pdfoutput=1 \\pdfpagewidth=100pt \\pdfpageheight=100pt \\shipout\\hbox{\\vrule height 1pt width 1pt}\n";

fn scratch(name: &str) -> PathBuf {
    let d = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("nochange-{name}"));
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(&d).unwrap();
    d
}

fn doc(d: &Path, body: &str) {
    fs::write(d.join("t.tex"), format!("{CATCODES}{body}\\end\n")).unwrap();
}

fn command(d: &Path, flags: &[&str]) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_rstex"));
    c.arg("-ini").arg("--until-stable").args(flags).arg("t.tex").current_dir(d).env("XDG_CACHE_HOME", d.join("cache")).env_remove("TEXINPUTS").env_remove("BIBINPUTS").stdin(Stdio::null());
    c
}

fn run(d: &Path) -> Output {
    command(d, &[]).output().unwrap()
}

fn text(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).to_string()
}

fn answered(o: &Output) -> bool {
    text(o).contains("passes=0 result=stable") && o.status.code() == Some(0)
}

/// A clock for stamps: time for a rewritten file to get another one, and for a file to be old enough that its stamp shows any later change.
fn age() {
    std::thread::sleep(std::time::Duration::from_millis(2100));
}

/// Builds until a save is answered: three runs at most, the first and second saves being what make the record, and older than the stamps of what was just written.
fn settle_with(_d: &Path, go: &dyn Fn() -> Output) {
    age();
    for _ in 0..4 {
        let o = go();
        assert!(o.status.code() == Some(0), "{}", text(&o));
        if answered(&o) {
            return;
        }
    }
    panic!("no save was answered");
}

fn settle(d: &Path) {
    settle_with(d, &|| run(d));
}

/// Name, size, stamps and bytes of each file below a directory.
fn state(d: &Path) -> Vec<(String, std::time::SystemTime, Vec<u8>)> {
    let mut v = Vec::new();
    for e in fs::read_dir(d).unwrap().flatten() {
        if e.path().is_file() {
            v.push((e.file_name().to_string_lossy().to_string(), e.metadata().unwrap().modified().unwrap(), fs::read(e.path()).unwrap()));
        }
    }
    v.sort();
    v
}

/// The document all of this is about: it reads a file, looks for another, writes a third and makes a page.
fn usual(d: &Path) {
    fs::write(d.join("dep.tex"), "\\def\\said{old}\n").unwrap();
    doc(d, &format!("\\input dep.tex \\immediate\\openin3=gen.dat \\ifeof3 \\else\\immediate\\closein3 \\fi{PAGE}\\immediate\\openout4=x.dat \\immediate\\write4{{\\said}}\\immediate\\closeout4 "));
}

#[test]
fn a_save_with_nothing_changed_writes_nothing() {
    let d = scratch("nothing");
    usual(&d);
    settle(&d);
    let before = state(&d);
    let o = run(&d);
    assert!(answered(&o), "{}", text(&o));
    assert_eq!(o.stdout, b"", "nothing on the terminal");
    assert_eq!(text(&o), "rstex: until-stable: passes=0 result=stable\n");
    assert_eq!(state(&d), before);
    // The editor's own save of the same bytes, and a touch.
    fs::write(d.join("t.tex"), fs::read(d.join("t.tex")).unwrap()).unwrap();
    fs::write(d.join("dep.tex"), fs::read(d.join("dep.tex")).unwrap()).unwrap();
    let o = run(&d);
    assert!(answered(&o), "{}", text(&o));
}

#[test]
fn a_change_to_what_it_reads_means_a_pass() {
    let d = scratch("edit");
    usual(&d);
    settle(&d);
    fs::write(d.join("t.tex"), format!("{CATCODES}\\input dep.tex \\immediate\\openin3=gen.dat \\ifeof3 \\else\\immediate\\closein3 \\fi{PAGE}\\immediate\\openout4=x.dat \\immediate\\write4{{\\said x}}\\immediate\\closeout4 \\end\n")).unwrap();
    let o = run(&d);
    assert!(text(&o).contains("result=stable") && !answered(&o), "{}", text(&o));
    assert!(fs::read_to_string(d.join("x.dat")).unwrap().contains("oldx"));

    // An input of the same size, with its stamps put back as far as a program can.
    let d = scratch("same-size");
    usual(&d);
    settle(&d);
    let f = fs::File::options().write(true).open(d.join("dep.tex")).unwrap();
    let was = f.metadata().unwrap().modified().unwrap();
    fs::write(d.join("dep.tex"), "\\def\\said{new}\n").unwrap();
    f.set_modified(was).unwrap();
    let o = run(&d);
    assert!(!answered(&o), "{}", text(&o));
    assert!(fs::read_to_string(d.join("x.dat")).unwrap().contains("new"));
}

#[test]
fn a_file_that_comes_or_goes_means_a_pass() {
    // Looked for and not found, and now there.
    let d = scratch("comes");
    usual(&d);
    settle(&d);
    fs::write(d.join("gen.dat"), "here\n").unwrap();
    assert!(!answered(&run(&d)));

    // Found, and now gone: an error of the document's.
    let d = scratch("goes");
    usual(&d);
    settle(&d);
    fs::remove_file(d.join("dep.tex")).unwrap();
    assert!(!answered(&run(&d)));

    // Found along the search path, and now found earlier on it.
    let d = scratch("shadow");
    fs::create_dir(d.join("elsewhere")).unwrap();
    fs::write(d.join("elsewhere/lib.tex"), "\\def\\said{far}\n").unwrap();
    doc(&d, &format!("\\input lib {PAGE}\\immediate\\openout4=x.dat \\immediate\\write4{{\\said}}\\immediate\\closeout4 "));
    let far = format!(".:{}:", d.join("elsewhere").display());
    let go = || command(&d, &[]).env("TEXINPUTS", &far).output().unwrap();
    settle_with(&d, &go);
    fs::write(d.join("lib.tex"), "\\def\\said{near}\n").unwrap();
    let o = go();
    assert!(!answered(&o), "{}", text(&o));
    assert!(fs::read_to_string(d.join("x.dat")).unwrap().contains("near"));
    fs::remove_file(d.join("lib.tex")).unwrap();
    assert!(!answered(&go()));
    // Further along the path than what it finds: that is no change to what a pass reads.
    settle_with(&d, &go);
    let later = format!(".:{}:{}:", d.join("elsewhere").display(), d.join("later").display());
    let go_later = || command(&d, &[]).env("TEXINPUTS", &later).output().unwrap();
    fs::create_dir(d.join("later")).unwrap();
    settle_with(&d, &go_later);
    fs::write(d.join("later/lib.tex"), "\\def\\said{later}\n").unwrap();
    let o = go_later();
    assert!(answered(&o), "{}", text(&o));

    // By another name in another case: kpathsea looks for that too.
    let d = scratch("case");
    doc(&d, &format!("\\immediate\\openin3=lib.dat \\ifeof3 \\def\\said{{none}}\\else\\immediate\\closein3 \\def\\said{{some}}\\fi{PAGE}\\immediate\\openout4=x.dat \\immediate\\write4{{\\said}}\\immediate\\closeout4 "));
    settle(&d);
    fs::write(d.join("LIB.dat"), "here\n").unwrap();
    let o = run(&d);
    assert!(!answered(&o), "{}", text(&o));

    // A name with a directory in it, in a directory that did not hold it.
    let d = scratch("subdir");
    fs::create_dir_all(d.join("sub")).unwrap();
    doc(&d, &format!("\\immediate\\openin3=sub/lib.dat \\ifeof3 \\else\\immediate\\closein3 \\fi{PAGE}"));
    settle(&d);
    fs::write(d.join("sub/lib.dat"), "here\n").unwrap();
    assert!(!answered(&run(&d)));

    // Beside the outputs, which come first.
    let d = scratch("outdir");
    fs::create_dir(d.join("o")).unwrap();
    usual(&d);
    let go = || command(&d, &["-output-directory=o"]).output().unwrap();
    settle_with(&d, &go);
    fs::write(d.join("o/dep.tex"), "\\def\\said{near}\n").unwrap();
    let o = go();
    assert!(!answered(&o), "{}", text(&o));
    assert!(fs::read_to_string(d.join("o/x.dat")).unwrap().contains("near"));

    // A file that nothing looks for is no change, even if the directory holds something new: every lookup goes again, and they all say the same.
    let d = scratch("swap-file");
    usual(&d);
    settle(&d);
    fs::write(d.join(".t.tex.swp"), "an editor's\n").unwrap();
    let o = run(&d);
    assert!(answered(&o), "{}", text(&o));
}

#[test]
fn what_a_pass_left_has_to_be_as_it_left_it() {
    for (what, gone) in [("t.pdf", true), ("t.log", true), ("x.dat", true), ("t.pdf", false), ("t.log", false), ("x.dat", false)] {
        let d = scratch(&format!("out-{what}-{gone}"));
        usual(&d);
        settle(&d);
        if gone {
            fs::remove_file(d.join(what)).unwrap();
        } else {
            let bytes = fs::read(d.join(what)).unwrap();
            fs::write(d.join(what), [&bytes[..], b"\n"].concat()).unwrap();
        }
        let o = run(&d);
        assert!(!answered(&o), "{what} {gone}: {}", text(&o));
        assert!(d.join(what).exists());
    }
    // A pass killed leaves its journal, and the next run has to make all of it again.
    let d = scratch("journal");
    usual(&d);
    settle(&d);
    fs::write(d.join("t.rstex-state"), "rstex-pass 1\nK5 6\nx.datkept\n\n").unwrap();
    assert!(!answered(&run(&d)));
}

#[test]
fn another_run_is_another_run() {
    let d = scratch("flags");
    usual(&d);
    settle(&d);
    assert!(!answered(&command(&d, &["-file-line-error"]).output().unwrap()));
    let o = command(&d, &["-jobname=other"]).output().unwrap();
    assert!(!answered(&o) && d.join("other.log").exists());

    let d = scratch("env");
    usual(&d);
    settle(&d);
    assert!(!answered(&command(&d, &[]).env("max_print_line", "100").output().unwrap()));

    let d = scratch("env-path");
    usual(&d);
    settle(&d);
    assert!(!answered(&command(&d, &[]).env("TEXINPUTS", "/nowhere:").output().unwrap()));

    let d = scratch("other-directory");
    fs::create_dir(d.join("o")).unwrap();
    usual(&d);
    settle(&d);
    assert!(!answered(&command(&d, &["-output-directory=o"]).output().unwrap()));

    // Another program: a copy of this one, as a new build is.
    let d = scratch("program");
    usual(&d);
    settle(&d);
    fs::copy(env!("CARGO_BIN_EXE_rstex"), d.join("rstex")).unwrap();
    let o = Command::new(d.join("rstex")).args(["-ini", "--until-stable", "t.tex"]).current_dir(&d).env("XDG_CACHE_HOME", d.join("cache")).stdin(Stdio::null()).output().unwrap();
    assert!(!answered(&o), "{}", text(&o));

    // What measures or wants a pass: the switch for it, and those for the literal path.
    let d = scratch("switches");
    usual(&d);
    settle(&d);
    assert!(!answered(&command(&d, &[]).env("RSTEX_NO_SHORTCUT", "1").output().unwrap()));
    settle(&d);
    assert!(!answered(&command(&d, &[]).env("RSTEX_NO_SNAPSHOT", "1").output().unwrap()));
    settle(&d);
    assert!(!answered(&command(&d, &["--no-snapshot"]).output().unwrap()));
}

/// A pass that nothing can ask the same of again, or that did not end well, has no record: each save is a pass.
#[test]
fn what_cannot_be_asked_again_has_no_record() {
    let d = scratch("error");
    doc(&d, "\\nosuchmacro ");
    for _ in 0..4 {
        let o = run(&d);
        assert_eq!(o.status.code(), Some(1));
        assert!(text(&o).contains("passes=1 result=error"), "{}", text(&o));
    }
    #[cfg(unix)]
    {
        let d = scratch("program");
        doc(&d, "\\immediate\\write18{true}");
        age();
        for _ in 0..4 {
            let o = command(&d, &["-shell-escape"]).output().unwrap();
            assert_eq!(o.status.code(), Some(0), "{}", text(&o));
            assert!(text(&o).contains("passes=1 result=stable"), "{}", text(&o));
        }
    }
    // Going on from a limit: nothing settled.
    let d = scratch("limit");
    doc(&d, "\\immediate\\openin3=n.dat \\ifeof3 \\count1=0 \\else \\endlinechar=-1 \\read3 to\\a \\count1=\\a\\relax \\endlinechar=13 \\immediate\\closein3 \\fi\\advance\\count1 by 1 \\immediate\\openout4=n.dat \\immediate\\write4{\\the\\count1}\\immediate\\closeout4 ");
    for _ in 0..3 {
        let o = command(&d, &["--max-passes=2"]).output().unwrap();
        assert_eq!(o.status.code(), Some(11), "{}", text(&o));
    }
}

/// What a document asks about a file: its size, which goes by the size, and its date, which goes by its stamp.
#[test]
fn a_question_about_a_file_is_answered_as_it_was_asked() {
    let d = scratch("size");
    fs::write(d.join("other.dat"), "12345\n").unwrap();
    doc(&d, &format!("\\edef\\s{{\\pdffilesize{{other.dat}}}}{PAGE}"));
    settle(&d);
    fs::write(d.join("other.dat"), "54321\n").unwrap();
    assert!(answered(&run(&d)), "the same size");
    fs::write(d.join("other.dat"), "123456\n").unwrap();
    assert!(!answered(&run(&d)), "another size");

    let d = scratch("date");
    fs::write(d.join("other.dat"), "12345\n").unwrap();
    doc(&d, &format!("\\edef\\s{{\\pdffilemoddate{{other.dat}}}}{PAGE}"));
    settle(&d);
    fs::write(d.join("other.dat"), "12345\n").unwrap();
    assert!(!answered(&run(&d)), "another date");

    let d = scratch("absent");
    doc(&d, &format!("\\edef\\s{{\\pdffilesize{{other.dat}}}}{PAGE}"));
    settle(&d);
    fs::write(d.join("other.dat"), "12345\n").unwrap();
    assert!(!answered(&run(&d)), "there now");
}

/// What bibtex would be given, which TeX never reads: found where it finds it.
#[test]
fn a_bibliography_that_only_bibtex_reads() {
    let body = format!("\\immediate\\openin3=t.bbl \\immediate\\closein3 {PAGE}\\immediate\\openout4=t.aux \\immediate\\write4{{\\string\\citation{{a}}}}\\immediate\\write4{{\\string\\bibstyle{{plain}}}}\\immediate\\write4{{\\string\\bibdata{{refs}}}}\\immediate\\closeout4 ");
    let setup = |name: &str| {
        let d = scratch(name);
        fs::create_dir(d.join("bibs")).unwrap();
        fs::write(d.join("bibs/refs.bib"), "@book{a, title={A}}\n").unwrap();
        doc(&d, &body);
        let bibs = d.join("bibs").display().to_string() + ":";
        let go = {
            let d = d.clone();
            move || command(&d, &[]).env("BIBINPUTS", &bibs).env("BSTINPUTS", d.join("bibs")).output().unwrap()
        };
        // The line that names bibtex goes out once, and a program's result is then what it is.
        let o = go();
        assert_eq!(o.status.code(), Some(10), "{}", text(&o));
        fs::write(d.join("t.bbl"), "\\relax\n").unwrap();
        (d, go)
    };
    let settled = |go: &dyn Fn() -> Output| {
        age();
        for _ in 0..5 {
            let o = go();
            if answered(&o) {
                return;
            }
            assert!(o.status.code() == Some(0) || o.status.code() == Some(10), "{}", text(&o));
            if o.status.code() == Some(10) {
                continue;
            }
        }
        panic!("no save was answered");
    };
    let (d, go) = setup("bib-edit");
    settled(&go);
    fs::write(d.join("bibs/refs.bib"), "@book{a, title={B}}\n").unwrap();
    let o = go();
    assert_eq!(o.status.code(), Some(10), "{}", text(&o));
    assert!(text(&o).contains("tool=bibtex reason=source-changed"), "{}", text(&o));

    // Found earlier, on the way bibtex goes: beside the outputs.
    let (d, go) = setup("bib-shadow");
    settled(&go);
    fs::write(d.join("refs.bib"), "@book{a, title={C}}\n").unwrap();
    let o = go();
    assert_eq!(o.status.code(), Some(10), "{}", text(&o));

    // Another search path, which finds another one: the environment is another run's, and what it finds counts.
    let (d, go) = setup("bib-path");
    settled(&go);
    fs::create_dir(d.join("other")).unwrap();
    fs::write(d.join("other/refs.bib"), "@book{a, title={E}}\n").unwrap();
    let other = d.join("other").display().to_string() + ":";
    let o = command(&d, &[]).env("BIBINPUTS", &other).env("BSTINPUTS", d.join("bibs")).output().unwrap();
    assert_eq!(o.status.code(), Some(10), "{}", text(&o));

    // Gone.
    let (d, go) = setup("bib-gone");
    settled(&go);
    fs::remove_file(d.join("bibs/refs.bib")).unwrap();
    assert!(!answered(&go()));

    // The bibliography that came with a document, with no .bib anywhere, until there is one.
    let d = scratch("bib-new");
    fs::write(d.join("t.bbl"), "\\relax\n").unwrap();
    doc(&d, &body);
    settle(&d);
    fs::write(d.join("refs.bib"), "@book{a, title={D}}\n").unwrap();
    let o = run(&d);
    assert_eq!(o.status.code(), Some(10), "{}", text(&o));
}

#[test]
fn a_damaged_record_is_a_pass() {
    let d = scratch("damage");
    usual(&d);
    settle(&d);
    let good = fs::read(d.join("t.rstex-state")).unwrap();
    assert!(good.starts_with(b"rstex-stable 1\n"), "{}", String::from_utf8_lossy(&good[..20]));
    let mut flipped = good.clone();
    let middle = good.len() / 2;
    flipped[middle] ^= 1;
    fs::write(d.join("t.rstex-state"), &flipped).unwrap();
    assert!(!answered(&run(&d)));
    for cut in [0, 1, 14, 15, 16, 30, good.len() / 2, good.len() - 1] {
        let d = scratch(&format!("damage-{cut}"));
        usual(&d);
        settle(&d);
        let good = fs::read(d.join("t.rstex-state")).unwrap();
        fs::write(d.join("t.rstex-state"), &good[..cut.min(good.len() - 1)]).unwrap();
        let o = run(&d);
        assert_eq!(o.status.code(), Some(0), "{cut}");
        assert!(!answered(&o), "{cut}");
    }
}

/// An edit costs no record: only a line.
#[test]
fn an_edit_leaves_a_line_and_no_record() {
    let d = scratch("line");
    usual(&d);
    assert_eq!(run(&d).status.code(), Some(0));
    assert!(fs::read(d.join("t.rstex-state")).unwrap().starts_with(b"rstex-seen 1 "));
    assert!(fs::metadata(d.join("t.rstex-state")).unwrap().len() < 64);
    fs::write(d.join("dep.tex"), "\\def\\said{new}\n").unwrap();
    assert_eq!(run(&d).status.code(), Some(0));
    assert!(fs::read(d.join("t.rstex-state")).unwrap().starts_with(b"rstex-seen 1 "));
    // And a plain run has none of this.
    let d = scratch("plain");
    usual(&d);
    let o = Command::new(env!("CARGO_BIN_EXE_rstex")).args(["-ini", "t.tex"]).current_dir(&d).env("XDG_CACHE_HOME", d.join("cache")).stdin(Stdio::null()).output().unwrap();
    assert_eq!(o.status.code(), Some(0));
    assert!(!d.join("t.rstex-state").exists());
}
