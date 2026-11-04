//! `--until-stable` through the real program.  None of this needs a TeX tree: `-ini` loads no format, and a document here says what a package would.  tests/until_stable/ has the rest, with LaTeX and the real programs.
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

const CATCODES: &str = "\\catcode`\\{=1 \\catcode`\\}=2 \\catcode`\\#=6 \\catcode`\\^=7 \\batchmode\n";

fn scratch(name: &str) -> PathBuf {
    let d = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("stable-{name}"));
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(&d).unwrap();
    d
}

fn write(d: &Path, name: &str, text: &str) {
    fs::write(d.join(name), text).unwrap();
}

fn doc(d: &Path, body: &str) {
    write(d, "t.tex", &format!("{CATCODES}{body}\n\\end\n"));
}

fn rstex(d: &Path, flags: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rstex")).arg("-ini").args(flags).arg("t.tex").current_dir(d).env("XDG_CACHE_HOME", d.join("cache")).stdin(Stdio::null()).output().unwrap()
}

fn until_stable(d: &Path) -> Output {
    rstex(d, &["--until-stable"])
}

fn err(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).to_string()
}

/// What the last line says, and the status.
fn ended(o: &Output, passes: u32, result: &str, code: i32) {
    let text = err(o);
    let want = format!("rstex: until-stable: passes={passes} result={result}\n");
    assert!(text.ends_with(&want) || text.contains(&format!("rstex: until-stable: passes={passes} result={result} ")), "{text}");
    assert_eq!(o.status.code(), Some(code), "{text}");
}

/// Reads a number from a file if it is there, adds one up to `limit`, and writes it back.
fn counter(limit: u32) -> String {
    format!(
        "\\immediate\\openin3=x.dat \\ifeof3 \\count1=0 \\else \\endlinechar=-1 \\read3 to\\a \\count1=\\a\\relax \\endlinechar=13 \\immediate\\closein3 \\fi\n\
         \\ifnum\\count1<{limit} \\advance\\count1 by 1 \\fi\n\
         \\immediate\\openout4=x.dat \\immediate\\write4{{\\the\\count1}}\\immediate\\closeout4 "
    )
}

#[test]
fn passes_go_on_until_what_a_pass_reads_stops_changing() {
    let d = scratch("settles");
    doc(&d, &counter(2));
    let o = until_stable(&d);
    // Looked for and made, then 1 read and 2 written, then 2 read and 2 written.
    ended(&o, 3, "stable", 0);
    assert!(err(&o).contains("rstex: until-stable: pass=1 changed=1 file=x.dat\n"), "{}", err(&o));
    assert!(err(&o).contains("rstex: until-stable: pass=2 changed=1 file=x.dat\n"), "{}", err(&o));
    assert_eq!(fs::read_to_string(d.join("x.dat")).unwrap().trim(), "2");
    // Nothing to do now: one pass, as a save with no change has.
    ended(&until_stable(&d), 1, "stable", 0);
}

#[test]
fn without_the_flag_there_is_one_pass_and_nothing_said() {
    let d = scratch("plain");
    doc(&d, &counter(2));
    let o = rstex(&d, &[]);
    assert_eq!(o.status.code(), Some(0));
    assert_eq!(err(&o), "");
    assert_eq!(fs::read_to_string(d.join("x.dat")).unwrap().trim(), "1");
    assert!(!d.join("t.rstex-tools").exists());
}

#[test]
fn the_limit_on_passes_has_its_own_status() {
    let d = scratch("limit");
    doc(&d, &counter(1000));
    let o = until_stable(&d);
    ended(&o, 5, "limit", 11);
    assert!(err(&o).contains("file=x.dat"), "{}", err(&o));
    assert_eq!(fs::read_to_string(d.join("x.dat")).unwrap().trim(), "5");
    fs::remove_file(d.join("x.dat")).unwrap();
    let o = rstex(&d, &["--until-stable", "--max-passes=2"]);
    ended(&o, 2, "limit", 11);
    assert_eq!(fs::read_to_string(d.join("x.dat")).unwrap().trim(), "2");
    fs::remove_file(d.join("x.dat")).unwrap();
    let o = rstex(&d, &["--max-passes=1", "-until-stable"]);
    ended(&o, 1, "limit", 11);
}

#[test]
fn an_error_ends_the_passes_with_the_status_of_tex() {
    let d = scratch("error");
    doc(&d, &format!("{}\\nosuchmacro", counter(2)));
    let o = until_stable(&d);
    ended(&o, 1, "error", 1);
    // The log and the file are that pass's.
    assert_eq!(fs::read_to_string(d.join("x.dat")).unwrap().trim(), "1");
    assert!(fs::read_to_string(d.join("t.log")).unwrap().contains("Undefined control sequence"));
}

#[test]
fn what_a_pass_reads_after_writing_it_is_its_own_doing() {
    let d = scratch("own");
    doc(&d, "\\immediate\\openout4=y.dat \\immediate\\write4{\\relax}\\immediate\\closeout4 \\input y.dat ");
    ended(&until_stable(&d), 1, "stable", 0);
}

#[test]
fn a_file_read_and_written_the_same_is_no_reason() {
    let d = scratch("same");
    write(&d, "y.dat", "\\relax\n");
    doc(&d, "\\input y.dat \\immediate\\openout4=y.dat \\immediate\\write4{\\string\\relax}\\immediate\\closeout4 ");
    ended(&until_stable(&d), 1, "stable", 0);
}

/// A program that the pass starts remakes a file it has read, as many do each time: with the same bytes that is nothing, by digests, and with others it is another pass.
#[cfg(unix)]
#[test]
fn a_file_changed_from_outside_goes_by_its_content() {
    let d = scratch("outside");
    write(&d, "dep.tex", "\\relax\n");
    write(&d, "same.sh", "printf '\\\\relax\\n' > dep.new; mv dep.new dep.tex\n");
    doc(&d, "\\input dep.tex \\immediate\\write18{sh same.sh}");
    let o = rstex(&d, &["--until-stable", "-shell-escape"]);
    ended(&o, 1, "stable", 0);
    write(&d, "other.sh", "test -f done || { touch done; printf '\\\\relax \\\\relax\\n' > dep.new; mv dep.new dep.tex; }\n");
    doc(&d, "\\input dep.tex \\immediate\\write18{sh other.sh}");
    let o = rstex(&d, &["--until-stable", "-shell-escape"]);
    assert!(err(&o).contains("rstex: until-stable: pass=1 changed=1 file=dep.tex\n"), "{}", err(&o));
    ended(&o, 2, "stable", 0);
}

#[test]
fn a_document_that_asks_the_size_of_its_own_pdf_settles() {
    let d = scratch("pdf");
    doc(&d, "\\pdfoutput=1 \\pdfpagewidth=100pt \\pdfpageheight=100pt \\edef\\s{\\pdffilesize{t.pdf}}\\shipout\\hbox{\\vrule height 1pt width 1pt}");
    ended(&until_stable(&d), 1, "stable", 0);
    ended(&until_stable(&d), 1, "stable", 0);
    assert!(d.join("t.pdf").exists());
}

#[test]
fn the_output_directory_holds_it_all() {
    let d = scratch("outdir");
    fs::create_dir(d.join("o")).unwrap();
    doc(&d, &counter(2));
    let o = rstex(&d, &["--until-stable", "-output-directory=o"]);
    ended(&o, 3, "stable", 0);
    assert!(err(&o).contains("file=o/x.dat\n"), "{}", err(&o));
    assert!(d.join("o/x.dat").exists() && d.join("o/t.log").exists() && !d.join("x.dat").exists());
}

/// A pass that looks for `y.ind` and writes `y.idx` with what `n.tex` says.
fn index_doc(d: &Path, entry: &str) {
    write(d, "n.tex", &format!("\\def\\entry{{{entry}}}\n"));
    doc(d, "\\input n.tex \\immediate\\openin3=y.ind \\immediate\\closein3 \\immediate\\openout4=y.idx \\immediate\\write4{\\entry}\\immediate\\closeout4 ");
}

#[test]
fn makeindex_is_named_when_its_result_is_missing_or_stale() {
    let d = scratch("index");
    index_doc(&d, "one");
    let o = until_stable(&d);
    assert!(err(&o).contains("rstex: until-stable: tool=makeindex reason=output-missing file=y.idx\n"), "{}", err(&o));
    ended(&o, 1, "tool", 10);
    // What makeindex would do, and then it holds until the .idx says something else.
    write(&d, "y.ind", "made\n");
    ended(&until_stable(&d), 1, "stable", 0);
    ended(&until_stable(&d), 1, "stable", 0);
    index_doc(&d, "two");
    let o = until_stable(&d);
    assert!(err(&o).contains("tool=makeindex reason=idx-changed file=y.idx\n"), "{}", err(&o));
    ended(&o, 1, "tool", 10);
    // It stays stale until the program has run.
    ended(&until_stable(&d), 1, "tool", 10);
    write(&d, "y.ind", "made again\n");
    ended(&until_stable(&d), 1, "stable", 0);
    fs::remove_file(d.join("y.ind")).unwrap();
    let o = until_stable(&d);
    assert!(err(&o).contains("reason=output-missing"), "{}", err(&o));
}

/// A pass that looks for `y.bbl` and writes `y.aux` with what `n.tex` says.
fn bib_doc(d: &Path, aux: &str) {
    write(d, "n.tex", &format!("\\def\\auxlines{{{aux}}}\n"));
    doc(d, "\\input n.tex \\newlinechar=`\\^^J \\immediate\\openin3=y.bbl \\immediate\\closein3 \\immediate\\openout4=y.aux \\immediate\\write4{\\auxlines}\\immediate\\closeout4 ");
}

const CITES_A: &str = "\\string\\citation{a}^^J\\string\\bibstyle{mine}^^J\\string\\bibdata{refs}";

#[test]
fn bibtex_goes_by_what_it_reads_of_the_aux() {
    let d = scratch("bibtex");
    write(&d, "refs.bib", "@misc{a,}\n");
    write(&d, "mine.bst", "ENTRY{}{}{}\n");
    bib_doc(&d, CITES_A);
    let o = until_stable(&d);
    assert!(err(&o).contains("rstex: until-stable: tool=bibtex reason=output-missing file=y.aux\n"), "{}", err(&o));
    ended(&o, 1, "tool", 10);
    write(&d, "y.bbl", "made\n");
    // The .aux was the same before this pass as it was after: nothing else to do.
    let o = until_stable(&d);
    ended(&o, 1, "stable", 0);
    // Another line in the .aux that bibtex does not read is no reason.
    bib_doc(&d, &format!("{CITES_A}^^J\\string\\newlabel{{x}}{{{{1}}{{1}}}}"));
    let o = until_stable(&d);
    assert!(!err(&o).contains("tool="), "{}", err(&o));
    assert_eq!(o.status.code(), Some(0));
    // A citation is.
    bib_doc(&d, &format!("\\string\\citation{{b}}^^J{CITES_A}"));
    let o = until_stable(&d);
    assert!(err(&o).contains("tool=bibtex reason=citations-changed file=y.aux\n"), "{}", err(&o));
    ended(&o, 1, "tool", 10);
    // Taking it out again, before bibtex has run, leaves the .bbl right.
    bib_doc(&d, CITES_A);
    assert_eq!(until_stable(&d).status.code(), Some(0));
    // The program runs, and that is seen by its output, also when it writes the same.
    bib_doc(&d, &format!("\\string\\citation{{b}}^^J{CITES_A}"));
    assert_eq!(until_stable(&d).status.code(), Some(10));
    std::thread::sleep(std::time::Duration::from_millis(20));
    write(&d, "y.bbl", "made\n");
    let o = until_stable(&d);
    assert!(!err(&o).contains("tool="), "{}", err(&o));
    // A .bib file is what it reads too.
    write(&d, "refs.bib", "@misc{a,}\n@misc{b,}\n");
    let o = until_stable(&d);
    assert!(err(&o).contains("tool=bibtex reason=source-changed file=y.aux\n"), "{}", err(&o));
    std::thread::sleep(std::time::Duration::from_millis(20));
    write(&d, "y.bbl", "made\n");
    assert!(!err(&until_stable(&d)).contains("tool="));
    write(&d, "mine.bst", "ENTRY{}{}{}\nREAD\n");
    assert!(err(&until_stable(&d)).contains("tool=bibtex reason=source-changed"));
}

#[test]
fn a_bbl_with_no_bib_file_is_what_came_with_the_document() {
    let d = scratch("shipped");
    bib_doc(&d, CITES_A);
    ended(&until_stable(&d), 1, "stable", 0);
    write(&d, "y.bbl", "shipped\n");
    let o = until_stable(&d);
    assert!(!err(&o).contains("tool="), "{}", err(&o));
    assert!(!d.join("t.rstex-tools").exists());
}

#[test]
fn a_bbl_nothing_is_known_of_gets_the_line_once() {
    let d = scratch("first-sight");
    write(&d, "refs.bib", "@misc{a,}\n");
    write(&d, "y.bbl", "from before\n");
    bib_doc(&d, CITES_A);
    let o = until_stable(&d);
    assert!(err(&o).contains("tool=bibtex reason=no-record file=y.aux\n"), "{}", err(&o));
    ended(&o, 1, "tool", 10);
    ended(&until_stable(&d), 1, "stable", 0);
}

#[test]
fn a_damaged_record_counts_as_none() {
    let d = scratch("damaged");
    write(&d, "refs.bib", "@misc{a,}\n");
    write(&d, "y.bbl", "from before\n");
    bib_doc(&d, CITES_A);
    write(&d, "t.rstex-tools", "not this\n");
    assert!(err(&until_stable(&d)).contains("reason=no-record"));
    assert!(fs::read_to_string(d.join("t.rstex-tools")).unwrap().starts_with("rstex-tools 1\n"));
}

#[test]
fn biber_goes_by_the_whole_bcf_and_its_data_sources() {
    let d = scratch("biber");
    write(&d, "refs.bib", "@misc{a,}\n");
    let bcf = |cite: &str| {
        write(&d, "n.tex", &format!("\\def\\bcflines{{<bcf:controlfile>^^J<bcf:datasource type=\"file\" datatype=\"bibtex\" glob=\"false\">refs.bib</bcf:datasource>^^J<bcf:citekey>{cite}</bcf:citekey>^^J</bcf:controlfile>}}\n"));
    };
    bcf("a");
    doc(&d, "\\input n.tex \\newlinechar=`\\^^J \\immediate\\openin3=t.bbl \\immediate\\closein3 \\immediate\\openout4=t.bcf \\immediate\\write4{\\bcflines}\\immediate\\closeout4 ");
    let o = until_stable(&d);
    assert!(err(&o).contains("tool=biber reason=output-missing file=t.bcf\n"), "{}", err(&o));
    write(&d, "t.bbl", "made\n");
    ended(&until_stable(&d), 1, "stable", 0);
    bcf("b");
    assert!(err(&until_stable(&d)).contains("tool=biber reason=bcf-changed file=t.bcf\n"));
    std::thread::sleep(std::time::Duration::from_millis(20));
    write(&d, "t.bbl", "made\n");
    ended(&until_stable(&d), 1, "stable", 0);
    write(&d, "refs.bib", "@misc{b,}\n");
    assert!(err(&until_stable(&d)).contains("tool=biber reason=source-changed"));
}

/// A second run of a document that has no tree: the process stays one, whatever the number of passes, so a signal to it ends the passes.
#[cfg(unix)]
#[test]
fn a_new_pass_is_the_same_process() {
    use std::os::unix::process::ExitStatusExt;
    let d = scratch("signal");
    // Each pass takes as long as it takes to count, and the file says which pass it is.
    doc(&d, &format!("{}\\count2=0 \\loop\\advance\\count2 by 1 \\ifnum\\count2<200000 \\repeat ", counter(1_000_000)).replace("\\loop", "\\def\\loop#1\\repeat{\\def\\body{#1}\\iterate}\\def\\iterate{\\body\\expandafter\\iterate\\fi}\\loop"));
    let mut child = Command::new(env!("CARGO_BIN_EXE_rstex"))
        .args(["-ini", "--until-stable", "--max-passes=1000000", "t.tex"])
        .current_dir(&d)
        .env("XDG_CACHE_HOME", d.join("cache"))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    // Until the second pass has written the file.
    let mut waited = 0;
    while fs::read_to_string(d.join("x.dat")).map_or(true, |t| t.trim().parse::<u32>().unwrap_or(0) < 2) && waited < 12000 {
        std::thread::sleep(std::time::Duration::from_millis(5));
        waited += 1;
    }
    Command::new("kill").args(["-KILL", &child.id().to_string()]).status().unwrap();
    let st = child.wait().unwrap();
    assert_eq!(st.signal(), Some(9), "{st:?}");
}
