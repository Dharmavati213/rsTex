//! What a pass of `--until-stable` leaves when it is killed, as an editor does at each newer save, and that a pass still has what the oracle has under each name while it runs.  With no TeX tree, as `until_stable.rs`; `tests/until_stable/kill.py` is the same on a long LaTeX document, at random moments.
//!
//! A document here says when it has got as far as a test wants by making `spinning.mark`, and then spins: no clock in any of it.
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};

const CATCODES: &str = "\\catcode`\\{=1 \\catcode`\\}=2 \\catcode`\\#=6 \\catcode`\\^=7 \\scrollmode\n";
const PAGE: &str = "\\pdfoutput=1 \\pdfpagewidth=100pt \\pdfpageheight=100pt \\pdfcompresslevel=0 \\pdfobjcompresslevel=0 \\shipout\\hbox{\\vrule height 1pt width 1pt}\n";
/// More than a buffer of the stream holds, so that part of it is on the disk.
fn long(what: &str) -> String {
    (0..300).map(|k| format!("\\immediate\\write4{{\\def\\string\\said{{{what}}}\\string\\def\\string\\n{{{k}}}\\string\\relax}}\n")).collect()
}
const SPIN: &str = "\\immediate\\openout9=spinning.mark \\immediate\\closeout9 \\def\\spin{\\spin}\\spin\n";
const MARK: &str = "spinning.mark";

fn scratch(name: &str) -> PathBuf {
    let d = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("killed-{name}"));
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(&d).unwrap();
    d
}

fn doc(d: &Path, body: &str) {
    fs::write(d.join("t.tex"), format!("{CATCODES}{body}\\end\n")).unwrap();
}

fn command(d: &Path, cwd: &Path, flags: &[&str], name: &str) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_rstex"));
    c.arg("-ini").args(flags).arg(name).current_dir(cwd).env("XDG_CACHE_HOME", d.join("cache")).env("SOURCE_DATE_EPOCH", "1000000000").env("FORCE_SOURCE_DATE", "1").stdin(Stdio::null());
    c
}

fn run(d: &Path, flags: &[&str]) -> Output {
    command(d, d, flags, "t.tex").output().unwrap()
}

fn stable(d: &Path) -> Output {
    let o = run(d, &["--until-stable"]);
    assert_eq!(o.status.code(), Some(0), "{}", String::from_utf8_lossy(&o.stderr));
    o
}

/// Has `until` hold, or the program end, which is then a failure.
fn wait_for(child: &mut Child, what: &str, until: &dyn Fn() -> bool) {
    let mut n = 0;
    while !until() {
        assert!(child.try_wait().unwrap().is_none(), "the run ended before {what}");
        std::thread::sleep(std::time::Duration::from_millis(2));
        n += 1;
        assert!(n < 30_000, "waited too long for {what}");
    }
}

fn killed_at_the_mark(d: &Path, cwd: &Path, flags: &[&str], name: &str, look: &dyn Fn()) {
    let mark = cwd.join(MARK);
    let _ = fs::remove_file(&mark);
    let mut child = command(d, cwd, flags, name).stdout(Stdio::null()).stderr(Stdio::null()).spawn().unwrap();
    wait_for(&mut child, "the mark", &|| mark.exists());
    look();
    child.kill().unwrap();
    child.wait().unwrap();
    let _ = fs::remove_file(&mark);
}

fn read(d: &Path, name: &str) -> Vec<u8> {
    fs::read(d.join(name)).unwrap_or_else(|e| panic!("{name}: {e}"))
}

/// All that a directory holds, but for what is rstex's cache and the source.
fn listing(d: &Path) -> Vec<(String, Vec<u8>)> {
    let mut v: Vec<(String, Vec<u8>)> = fs::read_dir(d)
        .unwrap()
        .flatten()
        .filter(|e| e.path().is_file())
        .map(|e| (e.file_name().to_string_lossy().to_string(), fs::read(e.path()).unwrap()))
        .filter(|e| e.0 != "t.tex" && e.0 != MARK && !e.0.ends_with(".rstex-state") && !e.0.ends_with(".rstex-tools"))
        .collect();
    v.sort();
    v
}

fn names(d: &Path) -> Vec<String> {
    listing(d).into_iter().map(|e| e.0).collect()
}

fn nothing_left(d: &Path) {
    let left: Vec<String> = names(d).into_iter().filter(|n| n.contains(".rstex-tmp")).collect();
    assert!(left.is_empty(), "{left:?}");
    for e in fs::read_dir(d).unwrap().flatten() {
        if e.file_name().to_string_lossy().ends_with(".rstex-state") {
            assert!(!fs::read(e.path()).unwrap().starts_with(b"rstex-pass"), "a journal stayed");
        }
    }
}

fn says(o: &Output, what: &str) -> bool {
    String::from_utf8_lossy(&o.stdout).contains(what)
}

/// The document of `t.tex` that reads `x.dat`, says what it found, and writes it.  Written to be stable after one more pass for the same `what`.
fn reader(what: &str, end: &str) -> String {
    format!("\\def\\said{{none}}\\immediate\\openin3=x.dat \\ifeof3 \\else\\immediate\\closein3 \\input x.dat \\fi\\immediate\\write16{{said=\\said}}{PAGE}\\immediate\\openout4=x.dat {}\\immediate\\closeout4 {end}", long(what))
}

#[test]
fn a_killed_pass_leaves_the_pdf_and_the_next_run_finds_what_the_last_one_left() {
    let d = scratch("whole");
    doc(&d, &reader("one", ""));
    stable(&d);
    let (pdf, dat) = (read(&d, "t.pdf"), read(&d, "x.dat"));
    assert!(pdf.starts_with(b"%PDF") && dat.len() > 4096);
    nothing_left(&d);
    // The stream's first buffers go to the disk and the rest waits, as for the oracle, and then the mark: a file cut in the middle of a line, under the name pdfTeX gives it.
    let cut = format!("\\def\\said{{none}}{PAGE}\\immediate\\openout4=x.dat {}{SPIN}", long("two"));
    doc(&d, &cut);
    killed_at_the_mark(&d, &d, &["--until-stable"], "t.tex", &|| {
        assert_eq!(read(&d, "t.pdf"), pdf, "a viewer has the PDF of the last run");
        let now = read(&d, "x.dat");
        assert!(now.len() > 1000 && now.len() < dat.len() && now != dat[..now.len()] && now.last() != Some(&b'\n'), "{}", now.len());
        assert!(kept_in_journal(&d, "t", &dat), "and what the pass found is there");
    });
    assert_eq!(read(&d, "t.pdf"), pdf);
    // A newer save: it finds the file the last whole pass wrote, and ends as if nothing had been killed.
    doc(&d, &reader("one", ""));
    let o = stable(&d);
    assert!(says(&o, "said=one"), "{}", String::from_utf8_lossy(&o.stdout));
    assert!(String::from_utf8_lossy(&o.stderr).contains("passes=1 result=stable"), "{}", String::from_utf8_lossy(&o.stderr));
    assert_eq!(read(&d, "x.dat"), dat);
    assert_eq!(read(&d, "t.pdf"), pdf);
    nothing_left(&d);
}

#[test]
fn a_kill_of_a_later_pass_leaves_what_the_pass_before_it_wrote() {
    let d = scratch("later");
    // The second pass finds what the first wrote, and stops there.  The file is there at the start: one that a pass only looked for is no reason for a second, as under latexmk.
    fs::write(d.join("x.dat"), "\\relax\n").unwrap();
    doc(&d, &format!("\\immediate\\openin3=x.dat \\ifeof3 \\else\\immediate\\closein3 \\input x.dat \\fi{PAGE}\\immediate\\openout4=x.dat {}\\immediate\\closeout4 \\ifx\\said\\undefined\\else {SPIN}\\fi", long("one")));
    let mut child = command(&d, &d, &["--until-stable"], "t.tex").stdout(Stdio::null()).stderr(Stdio::null()).spawn().unwrap();
    wait_for(&mut child, "the second pass", &|| d.join(MARK).exists());
    child.kill().unwrap();
    child.wait().unwrap();
    assert!(read(&d, "x.dat").len() > 4096);
    fs::remove_file(d.join(MARK)).unwrap();
    let first = long_written("one");
    assert!(kept_in_journal(&d, "t", &first));
    doc(&d, &reader("one", ""));
    let o = stable(&d);
    assert!(says(&o, "said=one"));
    assert_eq!(read(&d, "x.dat"), first);
    nothing_left(&d);
}

/// The bytes are there, whole, in the journal of `job`.
fn kept_in_journal(d: &Path, job: &str, bytes: &[u8]) -> bool {
    read(d, &format!("{job}.rstex-state")).windows(bytes.len()).any(|w| w == bytes)
}

fn long_written(what: &str) -> Vec<u8> {
    (0..300).map(|k| format!("\\def \\said{{{what}}}\\def\\n{{{k}}}\\relax\n")).collect::<String>().into_bytes()
}

#[test]
fn a_file_the_killed_pass_made_goes() {
    let d = scratch("made");
    doc(&d, &format!("{PAGE}\\immediate\\openout4=x.dat {}{SPIN}", long("two")));
    killed_at_the_mark(&d, &d, &["--until-stable"], "t.tex", &|| assert!(read(&d, "x.dat").len() > 1000));
    // A first build, with no pass ever finished: nothing of the kind may look like a file that a pass left.
    doc(&d, &reader("one", ""));
    let o = stable(&d);
    assert!(says(&o, "said=none"), "{}", String::from_utf8_lossy(&o.stdout));
    nothing_left(&d);
    assert!(!d.join("t.aux").exists(), "what a first pass seeds, for a document that writes none");
    // And nothing there if the next save no longer writes it.
    doc(&d, &format!("{PAGE}\\immediate\\openout4=y.dat {}{SPIN}", long("two")));
    killed_at_the_mark(&d, &d, &["--until-stable"], "t.tex", &|| {});
    doc(&d, PAGE);
    stable(&d);
    assert!(!d.join("y.dat").exists());
    nothing_left(&d);
}

/// A first line that says no file, so only the job itself knows its name, and `-jobname`, and a directory seen from elsewhere.
#[test]
fn the_journal_is_found_by_the_job_and_the_directory() {
    let d = scratch("names");
    doc(&d, &reader("one", ""));
    command(&d, &d, &["--until-stable"], "\\input t.tex").output().unwrap();
    let dat = read(&d, "x.dat");
    doc(&d, &format!("\\immediate\\openout4=x.dat {}{SPIN}", long("two")));
    killed_at_the_mark(&d, &d, &["--until-stable"], "\\input t.tex", &|| {});
    assert_ne!(read(&d, "x.dat"), dat);
    doc(&d, &reader("one", ""));
    let o = command(&d, &d, &["--until-stable"], "\\input t.tex").output().unwrap();
    assert!(says(&o, "said=one"), "{}", String::from_utf8_lossy(&o.stdout));
    assert_eq!(read(&d, "x.dat"), dat);
    nothing_left(&d);

    let d = scratch("jobname");
    doc(&d, &format!("{}", reader("one", "")));
    run(&d, &["--until-stable", "-jobname=other"]);
    let dat = read(&d, "x.dat");
    doc(&d, &format!("\\immediate\\openout4=x.dat {}{SPIN}", long("two")));
    killed_at_the_mark(&d, &d, &["--until-stable", "-jobname=other"], "t.tex", &|| assert!(kept_in_journal(&d, "other", &dat)));
    doc(&d, &reader("one", ""));
    let o = run(&d, &["--until-stable", "-jobname=other"]);
    assert!(says(&o, "said=one"));
    assert_eq!(read(&d, "x.dat"), dat);
    nothing_left(&d);

    let d = scratch("elsewhere");
    fs::create_dir_all(d.join("o")).unwrap();
    fs::create_dir_all(d.join("elsewhere")).unwrap();
    doc(&d, &reader("one", ""));
    command(&d, &d, &["--until-stable", "-output-directory=o"], "t.tex").output().unwrap();
    let dat = read(&d.join("o"), "x.dat");
    doc(&d, &format!("\\immediate\\openout4=x.dat {}{SPIN}", long("two")));
    let mark = d.join("o").join(MARK);
    let mut child = command(&d, &d, &["--until-stable", "-output-directory=o"], "t.tex").stdout(Stdio::null()).stderr(Stdio::null()).spawn().unwrap();
    wait_for(&mut child, "the mark", &|| mark.exists());
    child.kill().unwrap();
    child.wait().unwrap();
    doc(&d, &reader("one", ""));
    let o = command(&d, &d.join("elsewhere"), &["--until-stable", "-output-directory=../o"], "../t.tex").output().unwrap();
    assert!(says(&o, "said=one"), "{}", String::from_utf8_lossy(&o.stdout));
    assert_eq!(read(&d.join("o"), "x.dat"), dat);
    nothing_left(&d.join("o"));
}

/// While a pass runs, each name has what a plain run has there.  A file written and read back with its stream open, closed and opened again; its size; a file the pass found and then looks at through another stream: with and without the flag, every byte.
#[test]
fn a_pass_reads_what_a_plain_run_reads() {
    let body = format!(
        "\\immediate\\openin3=y.dat \\ifeof3 \\immediate\\write16{{y absent}}\\else\\immediate\\read3 to\\a \\immediate\\write16{{y: \\a}}\\immediate\\closein3 \\fi\n\
         \\immediate\\openout4=y.dat \\immediate\\write16{{empty: [\\pdffilesize{{y.dat}}]}}\n\
         {}\\immediate\\write16{{part: [\\pdffilesize{{y.dat}}][\\pdfmdfivesum file{{y.dat}}]}}\\immediate\\openin3=y.dat \\immediate\\read3 to\\a \\immediate\\write16{{\\meaning\\a}}\\immediate\\closein3\n\
         \\immediate\\closeout4 \\immediate\\write16{{all: [\\pdffilesize{{y.dat}}][\\pdfmdfivesum file{{y.dat}}]}}\\input y.dat \\immediate\\write16{{said=\\said,\\n}}\n\
         \\immediate\\openout4=y.dat \\immediate\\write4{{\\string\\def\\string\\said{{again}}}}\\immediate\\closeout4 \\input y.dat \\immediate\\write16{{said=\\said}}\n{PAGE}",
        long("written")
    );
    let go = |flags: &[&str], name: &str| {
        let d = scratch(name);
        doc(&d, &body);
        fs::write(d.join("y.dat"), "as it was found\n").unwrap();
        let o = run(&d, flags);
        assert!(!String::from_utf8_lossy(&o.stdout).contains("\n!"), "{}", String::from_utf8_lossy(&o.stdout));
        (listing(&d), o.stdout)
    };
    let (plain, plain_term) = go(&[], "read-plain");
    let (flag, flag_term) = go(&["--until-stable", "--max-passes=1"], "read-flag");
    assert_eq!(String::from_utf8_lossy(&plain_term), String::from_utf8_lossy(&flag_term));
    assert!(plain.iter().any(|e| e.0 == "y.dat"));
    assert_eq!(plain, flag);
}

/// A document that asks about its own PDF after the first page finds what a plain run finds, whichever way it spells the name.
#[test]
#[cfg(unix)] // On Windows there are no inodes to know a second spelling by, and a kill leaves what it leaves.
fn a_pdf_held_for_a_viewer_is_for_nobody_else() {
    for (name, spelling) in [("own-pdf", "t.pdf"), ("own-pdf-dot", "./t.pdf"), ("own-pdf-sub", "sub/../t.pdf")] {
        let go = |flags: &[&str], tag: &str| {
            let d = scratch(&format!("{name}-{tag}"));
            fs::create_dir_all(d.join("sub")).unwrap();
            doc(&d, PAGE);
            stable(&d);
            doc(&d, &format!("{PAGE}\\immediate\\write16{{[\\pdffilesize{{{spelling}}}][\\pdfmdfivesum file{{{spelling}}}]}}"));
            let o = run(&d, flags);
            (o.stdout, read(&d, "t.pdf"))
        };
        let (plain, plain_pdf) = go(&[], "plain");
        // With one pass: a document that asks after its PDF by another name reads a file that changes, and does not settle, which is another matter.
        let (flag, flag_pdf) = go(&["--until-stable", "--max-passes=1"], "flag");
        assert_eq!(String::from_utf8_lossy(&plain), String::from_utf8_lossy(&flag), "{spelling}");
        assert_eq!(plain_pdf, flag_pdf);
        assert!(String::from_utf8_lossy(&flag).contains("[0][D41D8CD98F00B204E9800998ECF8427E]"), "{}", String::from_utf8_lossy(&flag));
    }
}

/// A program that a pass starts sees what it would see under a plain run.
#[cfg(unix)]
#[test]
fn a_program_a_pass_starts_finds_what_a_plain_run_leaves() {
    let go = |flags: &[&str], name: &str| {
        let d = scratch(name);
        doc(&d, PAGE);
        stable(&d);
        doc(&d, &format!("{PAGE}\\immediate\\openout4=x.dat \\immediate\\write4{{written}}\\immediate\\closeout4 \\immediate\\write18{{wc -c < t.pdf > seen; cat x.dat >> seen}}"));
        let mut all = vec!["-shell-escape"];
        all.extend_from_slice(flags);
        let o = run(&d, &all);
        assert_eq!(o.status.code(), Some(0), "{}", String::from_utf8_lossy(&o.stderr));
        read(&d, "seen")
    };
    let plain = go(&[], "program-plain");
    let flag = go(&["--until-stable"], "program-flag");
    assert_eq!(String::from_utf8_lossy(&plain), String::from_utf8_lossy(&flag));
    assert!(String::from_utf8_lossy(&flag).contains("written"));
}

/// What a job leaves that makes no page, or stops part way, is what a plain run leaves, last PDF or not.
#[test]
fn a_job_with_no_whole_pdf_ends_as_a_plain_one() {
    for (name, body) in [
        ("no-page", "\\pdfoutput=1 "),
        ("opened-no-page", "\\pdfoutput=1 \\immediate\\pdfobj{x}"),
        ("fatal-after-a-page", &format!("{PAGE}\\input no-such-file ")[..]),
        ("error-in-a-page", &format!("{PAGE}\\nosuchmacro ")[..]),
    ] {
        let go = |flags: &[&str], tag: &str| {
            let d = scratch(&format!("{name}-{tag}"));
            doc(&d, PAGE);
            stable(&d);
            doc(&d, body);
            let o = run(&d, flags);
            (o.status.code(), listing(&d))
        };
        let (a, plain) = go(&[], "plain");
        let (b, flag) = go(&["--until-stable"], "flag");
        assert_eq!(a, b, "{name}");
        assert_eq!(plain, flag, "{name}");
    }
}

/// Without the flag nothing is set aside, and the PDF is empty while the job runs, as `fopen` leaves it on the oracle.
#[test]
fn a_plain_run_has_none_of_it() {
    let d = scratch("plain");
    doc(&d, &reader("one", ""));
    stable(&d);
    doc(&d, &format!("{PAGE}\\immediate\\openout4=x.dat {}{SPIN}", long("two")));
    killed_at_the_mark(&d, &d, &[], "t.tex", &|| {
        assert_eq!(read(&d, "t.pdf").len(), 0);
        assert!(!read(&d, "t.rstex-state").starts_with(b"rstex-pass"));
    });
    nothing_left(&d);
    // And a journal that some pass left is not its to touch.
    fs::write(d.join("t.rstex-state"), "rstex-pass 1\n/nowhere\n").unwrap();
    doc(&d, PAGE);
    assert_eq!(run(&d, &[]).status.code(), Some(0));
    assert_eq!(read(&d, "t.rstex-state").len(), 22);
}

/// What a pass killed in its first moments leaves of its `.fls`, before the job has its name: one name for a job, which the next pass takes over, and no file more for each kill.
#[test]
fn the_recorder_of_a_pass_has_one_name_until_the_job_has_its_own() {
    let d = scratch("recorder");
    doc(&d, PAGE);
    fs::write(d.join("t.rstex-fls"), "PWD LEFT-BEHIND\n").unwrap();
    let o = run(&d, &["--until-stable", "-recorder"]);
    assert_eq!(o.status.code(), Some(0), "{}", String::from_utf8_lossy(&o.stderr));
    let recorded: Vec<String> = names(&d).into_iter().filter(|n| n.ends_with("fls")).collect();
    assert_eq!(recorded, ["t.fls"]);
    let fls = String::from_utf8(read(&d, "t.fls")).unwrap();
    assert!(fls.starts_with("PWD ") && !fls.contains("LEFT-BEHIND"), "{fls}");
    // Without the flag it is as for pdfTeX, and it goes the same way.
    let d = scratch("recorder-plain");
    doc(&d, PAGE);
    assert_eq!(run(&d, &["-recorder"]).status.code(), Some(0));
    let recorded: Vec<String> = names(&d).into_iter().filter(|n| n.ends_with("fls")).collect();
    assert_eq!(recorded, ["t.fls"]);
}

/// A journal cut anywhere, or not one at all, is no matter: a file it names whole comes back, and no pass stops for it.
#[test]
fn a_journal_cut_short_is_no_matter() {
    let d = scratch("cut-journal");
    doc(&d, PAGE);
    fs::write(d.join("t.rstex-state"), "something else\n").unwrap();
    assert_eq!(run(&d, &["--until-stable"]).status.code(), Some(0));
    let whole = "rstex-pass 1\nK5 5\na.datwas a\nN5 0\nb.dat\nK5 5\nc.datwas c\n".to_string();
    let end = |what: &str| whole.find(what).unwrap() + what.len() + 1;
    let (a, b, c) = (end("a.datwas a"), end("b.dat"), end("c.datwas c"));
    for cut in 0..=whole.len() {
        for (name, text) in [("a.dat", "cut a"), ("b.dat", "made"), ("c.dat", "cut c")] {
            fs::write(d.join(name), text).unwrap();
        }
        fs::write(d.join("t.rstex-state"), &whole[..cut]).unwrap();
        assert_eq!(run(&d, &["--until-stable"]).status.code(), Some(0));
        let of = |name: &str| fs::read_to_string(d.join(name)).ok();
        // An entry counts when it is all there.
        assert_eq!(of("a.dat").as_deref(), Some(if cut >= a { "was a" } else { "cut a" }), "{cut}");
        assert_eq!(of("b.dat"), if cut >= b { None } else { Some("made".to_string()) }, "{cut}");
        assert_eq!(of("c.dat").as_deref(), Some(if cut >= c { "was c" } else { "cut c" }), "{cut}");
    }
    // What lies after the end mark, or after the line of a pass that ended, is what the file held before.
    for (name, text) in [("a.dat", "cut a"), ("b.dat", "made")] {
        fs::write(d.join(name), text).unwrap();
    }
    fs::write(d.join("t.rstex-state"), "rstex-pass 1\nK5 5\na.datwas a\nE\nN5 0\nb.dat\n").unwrap();
    assert_eq!(run(&d, &["--until-stable"]).status.code(), Some(0));
    assert_eq!(read(&d, "a.dat"), b"was a");
    assert_eq!(read(&d, "b.dat"), b"made");
    fs::write(d.join("t.rstex-state"), "rstex-none 1\nK5 5\na.datolder\nE\n").unwrap();
    assert_eq!(run(&d, &["--until-stable"]).status.code(), Some(0));
    assert_eq!(read(&d, "a.dat"), b"was a");
    // The same file twice under two spellings, the second kept when the pass had already written it: it ends as it was first seen.
    fs::write(d.join("t.rstex-state"), "rstex-pass 1\nK5 5\na.datfirst\nK7 6\n./a.datsecond\n").unwrap();
    assert_eq!(run(&d, &["--until-stable"]).status.code(), Some(0));
    assert_eq!(read(&d, "a.dat"), b"first");
}

/// The state file lies among what a document brings along.  Nothing in one may reach further than the document itself could under `openout_any = p`: not outside the directory, and no hidden file, and neither by writing nor by taking away.
#[test]
#[cfg(unix)] // On Windows there are no inodes to know a second spelling by, and a kill leaves what it leaves.
fn a_journal_from_someone_else_reaches_no_further_than_the_document() {
    let top = scratch("planted");
    let d = top.join("doc");
    fs::create_dir_all(d.join(".git/hooks")).unwrap();
    fs::create_dir_all(d.join("sub")).unwrap();
    doc(&d, PAGE);
    let outside = top.join("outside.dat");
    let abs = outside.to_str().unwrap().to_string();
    let targets = [("../outside.dat", outside.clone()), ("sub/../../outside.dat", outside.clone()), (&abs[..], outside.clone()), (".git/hooks/pre-commit", d.join(".git/hooks/pre-commit")), (".hidden", d.join(".hidden")), ("sub/.hidden", d.join("sub/.hidden"))];
    for (name, target) in &targets {
        for kind in ['K', 'N'] {
            fs::write(target, "left alone\n").unwrap();
            let (len, body) = if kind == 'K' { (7, "planted") } else { (0, "") };
            fs::write(d.join("t.rstex-state"), format!("rstex-pass 1\n{kind}{} {len}\n{name}{body}\n", name.len())).unwrap();
            assert_eq!(run(&d, &["--until-stable"]).status.code(), Some(0));
            assert_eq!(fs::read_to_string(target).unwrap(), "left alone\n", "{kind} {name}");
        }
    }
    // What a pass really writes outside its directory, as `openout_any = a` allows, is left to what a kill leaves of it, and nothing of it goes in the journal.
    let outside_doc = format!("\\immediate\\openout4={abs} \\immediate\\write4{{new}}\\immediate\\closeout4 {PAGE}{SPIN}");
    doc(&d, &outside_doc);
    fs::write(&outside, "old\n").unwrap();
    let mark = d.join(MARK);
    let mut child = command(&d, &d, &["--until-stable"], "t.tex").env("openout_any", "a").stdout(Stdio::null()).stderr(Stdio::null()).spawn().unwrap();
    wait_for(&mut child, "the mark", &|| mark.exists());
    child.kill().unwrap();
    child.wait().unwrap();
    assert_eq!(fs::read_to_string(&outside).unwrap(), "new\n");
    assert!(!read(&d, "t.rstex-state").windows(abs.len()).any(|w| w == abs.as_bytes()));
}

#[cfg(unix)]
mod unix {
    use super::*;
    use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};

    /// A file keeps its mode and its inode, as `fopen` leaves them, and a link is written through, and comes back through itself.
    #[test]
    fn a_side_file_stays_the_file_it_was() {
        let d = scratch("mode");
        doc(&d, &format!("\\immediate\\openout4=x.dat \\immediate\\write4{{new}}\\immediate\\closeout4 {PAGE}"));
        fs::write(d.join("x.dat"), "old\n").unwrap();
        fs::set_permissions(d.join("x.dat"), fs::Permissions::from_mode(0o640)).unwrap();
        fs::write(d.join("t.pdf"), "old\n").unwrap();
        fs::set_permissions(d.join("t.pdf"), fs::Permissions::from_mode(0o600)).unwrap();
        let inode = fs::metadata(d.join("x.dat")).unwrap().ino();
        stable(&d);
        assert_eq!(fs::metadata(d.join("x.dat")).unwrap().ino(), inode);
        assert_eq!(fs::metadata(d.join("x.dat")).unwrap().mode() & 0o777, 0o640);
        assert_eq!(fs::metadata(d.join("t.pdf")).unwrap().mode() & 0o777, 0o600);
        nothing_left(&d);

        let d = scratch("links");
        fs::create_dir(d.join("real")).unwrap();
        doc(&d, &format!("\\immediate\\openout4=x.dat \\immediate\\write4{{new}}\\immediate\\closeout4 \\immediate\\openout5=y.dat \\immediate\\write5{{new}}\\immediate\\closeout5 {PAGE}{SPIN}"));
        fs::write(d.join("real/x.dat"), "old\n").unwrap();
        symlink("real/x.dat", d.join("x.dat")).unwrap();
        fs::write(d.join("y.dat"), "old\n").unwrap();
        fs::hard_link(d.join("y.dat"), d.join("real/y.dat")).unwrap();
        killed_at_the_mark(&d, &d, &["--until-stable"], "t.tex", &|| {
            assert_eq!(read(&d, "real/x.dat"), b"new\n");
            assert_eq!(read(&d, "real/y.dat"), b"new\n");
        });
        doc(&d, PAGE);
        stable(&d);
        assert!(fs::symlink_metadata(d.join("x.dat")).unwrap().file_type().is_symlink());
        assert_eq!(read(&d, "real/x.dat"), b"old\n");
        assert_eq!(read(&d, "real/y.dat"), b"old\n");
    }

    /// What cannot be read again is not read for this: a stream on a device goes as under a plain run.
    #[test]
    fn a_device_is_left_alone() {
        let d = scratch("device");
        symlink("/dev/null", d.join("x.dat")).unwrap();
        doc(&d, "\\immediate\\openout4=x.dat \\immediate\\write4{x}\\immediate\\closeout4 ");
        stable(&d);
        nothing_left(&d);
    }

    /// A directory that takes no new name, where `fopen` on a file there still works.
    #[test]
    fn a_directory_that_takes_no_new_name() {
        let d = scratch("no-new-name");
        let o = d.join("o");
        fs::create_dir(&o).unwrap();
        doc(&d, &format!("\\immediate\\openout4=x.dat \\immediate\\write4{{\\pdfelapsedtime}}\\immediate\\closeout4 {PAGE}"));
        fs::write(o.join("x.dat"), "old\n").unwrap();
        fs::write(o.join("t.pdf"), "old\n").unwrap();
        fs::write(o.join("t.log"), "old\n").unwrap();
        fs::set_permissions(&o, fs::Permissions::from_mode(0o555)).unwrap();
        let writable = fs::write(o.join("probe"), "x").is_ok();
        let r = run(&d, &["--until-stable", "--max-passes=1", "-output-directory=o"]);
        let ok = r.status.code();
        let pdf = read(&o, "t.pdf");
        let dat = read(&o, "x.dat");
        fs::set_permissions(&o, fs::Permissions::from_mode(0o755)).unwrap();
        if writable {
            return; // root
        }
        assert_eq!(ok, Some(0), "{}", String::from_utf8_lossy(&r.stderr));
        assert!(pdf.starts_with(b"%PDF"));
        assert_ne!(dat, b"old\n");
    }

    /// A name a stream cannot open under goes to the job as under a plain run, journal or no journal.
    #[test]
    fn a_stream_that_cannot_open_is_as_under_a_plain_run() {
        let go = |flags: &[&str], name: &str| {
            let d = scratch(name);
            doc(&d, "\\immediate\\openout4=no-such-directory/x.dat \\immediate\\write4{x}\\immediate\\closeout4 ");
            let o = run(&d, flags);
            (o.status.code(), listing(&d))
        };
        let plain = go(&[], "no-dir-plain");
        let flag = go(&["--until-stable"], "no-dir-flag");
        assert_eq!(plain, flag);
    }

    /// A DVI file is written as it goes, so it is one of the files a pass sets aside.
    #[test]
    fn a_dvi_file_comes_back() {
        let d = scratch("dvi");
        let page = "\\shipout\\hbox{\\vrule height 1pt width 1pt}";
        doc(&d, page);
        let flags = ["--until-stable", "-output-format=dvi"];
        assert_eq!(run(&d, &flags).status.code(), Some(0));
        let dvi = read(&d, "t.dvi");
        doc(&d, &format!("{page}{SPIN}"));
        killed_at_the_mark(&d, &d, &flags, "t.tex", &|| assert!(kept_in_journal(&d, "t", &dvi)));
        // And that is what the next run has found by the time it reads it.
        doc(&d, &format!("\\immediate\\write16{{[\\pdffilesize{{t.dvi}}]}}{page}"));
        let o = run(&d, &flags);
        assert!(says(&o, &format!("[{}]", dvi.len())), "{}", String::from_utf8_lossy(&o.stdout));
        assert_eq!(read(&d, "t.dvi"), dvi);
        nothing_left(&d);
    }
}
