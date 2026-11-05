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
    write(&d, "x.dat", "0\n");
    doc(&d, &counter(2));
    let o = until_stable(&d);
    // 0 read and 1 written, then 1 read and 2 written, then 2 read and 2 written.
    ended(&o, 3, "stable", 0);
    assert!(err(&o).contains("rstex: until-stable: pass=1 changed=1 file=x.dat\n"), "{}", err(&o));
    assert!(err(&o).contains("rstex: until-stable: pass=2 changed=1 file=x.dat\n"), "{}", err(&o));
    assert_eq!(fs::read_to_string(d.join("x.dat")).unwrap().trim(), "2");
    // Nothing to do now: one pass, as a save with no change has.
    ended(&until_stable(&d), 1, "stable", 0);
}

/// A look with `\openin` for a file the pass then writes leaves no line in the log and nothing in a `.fls`: latexmk does not know of it, and has no second run.  From then on the file is read before it is written.
#[test]
fn a_file_looked_for_and_made_is_no_reason_unless_the_log_says_it_was_missed() {
    let d = scratch("looked-for");
    doc(&d, &counter(2));
    ended(&until_stable(&d), 1, "stable", 0);
    assert_eq!(fs::read_to_string(d.join("x.dat")).unwrap().trim(), "1");
    let o = until_stable(&d);
    ended(&o, 2, "stable", 0);
    assert_eq!(fs::read_to_string(d.join("x.dat")).unwrap().trim(), "2");

    // As `\@input` says it.
    let d = scratch("missed");
    doc(&d, "\\immediate\\openin3=y.dat \\ifeof3 \\immediate\\write-1{No file y.dat.}\\fi \\immediate\\closein3 \\immediate\\openout4=y.dat \\immediate\\write4{made}\\immediate\\closeout4 ");
    let o = until_stable(&d);
    assert!(err(&o).contains("rstex: until-stable: pass=1 changed=1 file=y.dat\n"), "{}", err(&o));
    ended(&o, 2, "stable", 0);

    // Another name in the log is nothing to it, and neither is `y` for a file `y.dat`.
    let d = scratch("missed-other");
    doc(&d, "\\immediate\\openin3=y.dat \\immediate\\write-1{No file z.dat.}\\immediate\\write-1{No file y.}\\immediate\\closein3 \\immediate\\openout4=y.dat \\immediate\\write4{made}\\immediate\\closeout4 ");
    ended(&until_stable(&d), 1, "stable", 0);

    // In an output directory the log has the name with no directory.
    let d = scratch("missed-outdir");
    fs::create_dir(d.join("o")).unwrap();
    doc(&d, "\\immediate\\openin3=y.dat \\ifeof3 \\immediate\\write-1{No file y.dat.}\\fi \\immediate\\closein3 \\immediate\\openout4=y.dat \\immediate\\write4{made}\\immediate\\closeout4 ");
    let o = rstex(&d, &["--until-stable", "-output-directory=o"]);
    assert!(err(&o).contains("rstex: until-stable: pass=1 changed=1 file=o/y.dat\n"), "{}", err(&o));
    ended(&o, 2, "stable", 0);

    // And it counts against the limit as any other.
    let d = scratch("missed-limit");
    doc(&d, "\\immediate\\openin3=y.dat \\ifeof3 \\immediate\\write-1{No file y.dat.}\\fi \\immediate\\closein3 \\immediate\\openout4=y.dat \\immediate\\write4{made}\\immediate\\closeout4 ");
    let o = rstex(&d, &["--until-stable", "--max-passes=1"]);
    ended(&o, 1, "limit", 11);
    assert!(err(&o).contains("file=y.dat"), "{}", err(&o));
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
    write(&d, "x.dat", "0\n");
    doc(&d, &counter(1000));
    let o = until_stable(&d);
    ended(&o, 5, "limit", 11);
    assert!(err(&o).contains("file=x.dat"), "{}", err(&o));
    assert_eq!(fs::read_to_string(d.join("x.dat")).unwrap().trim(), "5");
    write(&d, "x.dat", "0\n");
    let o = rstex(&d, &["--until-stable", "--max-passes=2"]);
    ended(&o, 2, "limit", 11);
    assert_eq!(fs::read_to_string(d.join("x.dat")).unwrap().trim(), "2");
    write(&d, "x.dat", "0\n");
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
    write(&d, "o/x.dat", "0\n");
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

/// The same document with an error in it.
fn with_error(d: &Path, body: &str) {
    doc(d, &format!("{body}\\nosuchmacro "));
}

fn index_doc_with_error(d: &Path, entry: &str) {
    write(d, "n.tex", &format!("\\def\\entry{{{entry}}}\n"));
    with_error(d, "\\input n.tex \\immediate\\openin3=y.ind \\immediate\\closein3 \\immediate\\openout4=y.idx \\immediate\\write4{\\entry}\\immediate\\closeout4 ");
}

fn bib_doc_with_error(d: &Path, aux: &str) {
    write(d, "n.tex", &format!("\\def\\auxlines{{{aux}}}\n"));
    with_error(d, "\\input n.tex \\newlinechar=`\\^^J \\immediate\\openin3=y.bbl \\immediate\\closein3 \\immediate\\openout4=y.aux \\immediate\\write4{\\auxlines}\\immediate\\closeout4 ");
}

/// As latexmk: the pass that fails stops there, and its next run begins with the program whose result is missing, before any pass, on what the failed pass left.
#[test]
fn after_an_error_the_next_run_begins_with_what_is_missing() {
    let d = scratch("error-index");
    index_doc_with_error(&d, "one");
    let o = until_stable(&d);
    ended(&o, 1, "error", 1);
    assert!(!err(&o).contains("tool="), "{}", err(&o));
    assert!(!d.join("y.ind").exists());
    let log = fs::read(d.join("t.log")).unwrap();
    // No pass, so nothing of what the failed one left has moved.
    let o = until_stable(&d);
    assert!(err(&o).contains("rstex: until-stable: tool=makeindex reason=output-missing file=y.idx\n"), "{}", err(&o));
    assert!(err(&o).ends_with("rstex: until-stable: passes=0 result=tool\n"), "{}", err(&o));
    assert_eq!(o.status.code(), Some(10));
    assert_eq!(fs::read(d.join("t.log")).unwrap(), log);
    // What makeindex would do, then a pass, which fails again, with nothing more to name.
    write(&d, "y.ind", "made\n");
    let o = until_stable(&d);
    ended(&o, 1, "error", 1);
    assert!(!err(&o).contains("tool="), "{}", err(&o));
}

/// It says so once.  latexmk does not run a program again for what it has failed on, and without this a caller whose program writes nothing never gets another pass.
#[test]
fn a_program_that_makes_nothing_is_named_once() {
    let d = scratch("error-index-nothing");
    index_doc_with_error(&d, "one");
    ended(&until_stable(&d), 1, "error", 1);
    ended(&until_stable(&d), 0, "tool", 10);
    // The program left nothing.
    ended(&until_stable(&d), 1, "error", 1);
    ended(&until_stable(&d), 1, "error", 1);
    // With the error gone, the pass says what is missing, as it always did.
    index_doc(&d, "one");
    let o = until_stable(&d);
    assert!(err(&o).contains("tool=makeindex reason=output-missing"), "{}", err(&o));
    ended(&o, 1, "tool", 10);
}

/// The error gone by the next run: still the program first, and then the pass that finds what it made, as latexmk's order is.
#[test]
fn with_the_error_gone_the_program_still_comes_first() {
    let d = scratch("error-gone");
    index_doc_with_error(&d, "one");
    ended(&until_stable(&d), 1, "error", 1);
    index_doc(&d, "one");
    ended(&until_stable(&d), 0, "tool", 10);
    write(&d, "y.ind", "made\n");
    ended(&until_stable(&d), 1, "stable", 0);
    ended(&until_stable(&d), 1, "stable", 0);
}

/// As latexmk's `rdb_update_gen_files`: what an error pass wrote counts as seen.  A new entry under an error asks for no makeindex then, nor once the error is gone with the entry the same, and the next change does.
#[test]
fn what_a_failed_pass_wrote_counts_as_seen() {
    let d = scratch("error-stale");
    index_doc(&d, "one");
    ended(&until_stable(&d), 1, "tool", 10);
    write(&d, "y.ind", "made\n");
    ended(&until_stable(&d), 1, "stable", 0);
    index_doc_with_error(&d, "two");
    let o = until_stable(&d);
    ended(&o, 1, "error", 1);
    assert!(!err(&o).contains("tool="), "{}", err(&o));
    index_doc(&d, "two");
    ended(&until_stable(&d), 1, "stable", 0);
    index_doc(&d, "three");
    let o = until_stable(&d);
    assert!(err(&o).contains("tool=makeindex reason=idx-changed"), "{}", err(&o));
    ended(&o, 1, "tool", 10);
}

/// `\jobname` keeps the quotes of a name with a space and the files have none: the seed a first pass finds, the record and what is said of a `.bcf` go by the files.
#[test]
fn a_name_with_a_space_has_its_files_under_that_name() {
    let d = scratch("spaces");
    let body = "\\immediate\\openin3=\"my doc\".bcf \\ifeof3 \\immediate\\write-1{no such}\\else \\immediate\\write-1{found}\\fi \\immediate\\closein3 ";
    write(&d, "my doc.tex", &format!("{CATCODES}\\immediate\\openin2=\"my doc\".aux \\ifeof2 \\immediate\\write-1{{NO AUX}}\\else \\immediate\\write-1{{AUX THERE}}\\fi \\immediate\\closein2 {body}\\immediate\\openout4=\"my doc\".aux \\immediate\\write4{{\\relax}}\\immediate\\closeout4 \\end\n"));
    let o = Command::new(env!("CARGO_BIN_EXE_rstex")).args(["-ini", "--until-stable", "my doc.tex"]).current_dir(&d).env("XDG_CACHE_HOME", d.join("cache")).stdin(Stdio::null()).output().unwrap();
    assert_eq!(o.status.code(), Some(0), "{}", err(&o));
    assert!(fs::read_to_string(d.join("my doc.log")).unwrap().contains("AUX THERE"), "the seed is under another name");
    let names: Vec<String> = fs::read_dir(&d).unwrap().flatten().map(|e| e.file_name().to_string_lossy().to_string()).collect();
    assert!(!names.iter().any(|n| n.contains('"')), "{names:?}");
    // A record under that name, and the name found again before a pass.
    let d = scratch("spaces-record");
    write(&d, "my doc.tex", &format!("{CATCODES}\\immediate\\openin3=y.ind \\immediate\\closein3 \\immediate\\openout4=y.idx \\immediate\\write4{{one}}\\immediate\\closeout4 \\nosuchmacro \\end\n"));
    let run = || Command::new(env!("CARGO_BIN_EXE_rstex")).args(["-ini", "--until-stable", "my doc.tex"]).current_dir(&d).env("XDG_CACHE_HOME", d.join("cache")).stdin(Stdio::null()).output().unwrap();
    ended(&run(), 1, "error", 1);
    assert!(d.join("my doc.rstex-tools").exists());
    ended(&run(), 0, "tool", 10);
}

/// A pass that fails before it asks for the `.ind`: the `.idx` it wrote is enough, as for latexmk, and with the error gone a pass that does ask finds it.
#[test]
fn a_failed_pass_need_not_have_asked_for_the_index() {
    let d = scratch("error-before-asking");
    write(&d, "n.tex", "\\def\\entry{one}\n");
    with_error(&d, "\\input n.tex \\immediate\\openout4=y.idx \\immediate\\write4{\\entry}\\immediate\\closeout4 ");
    ended(&until_stable(&d), 1, "error", 1);
    let o = until_stable(&d);
    assert!(err(&o).contains("rstex: until-stable: tool=makeindex reason=output-missing file=y.idx\n"), "{}", err(&o));
    ended(&o, 0, "tool", 10);
    write(&d, "y.ind", "made\n");
    index_doc(&d, "one");
    ended(&until_stable(&d), 1, "stable", 0);
}

/// A pass that stops before it comes to a program says nothing of it: its record stays.
#[test]
fn a_pass_that_stops_early_leaves_a_record_alone() {
    let d = scratch("error-early");
    index_doc(&d, "one");
    ended(&until_stable(&d), 1, "tool", 10);
    write(&d, "y.ind", "made\n");
    ended(&until_stable(&d), 1, "stable", 0);
    let record = fs::read(d.join("t.rstex-tools")).unwrap();
    with_error(&d, "");
    ended(&until_stable(&d), 1, "error", 1);
    assert_eq!(fs::read(d.join("t.rstex-tools")).unwrap(), record);
}

#[test]
fn an_error_with_no_program_leaves_no_record() {
    let d = scratch("error-none");
    with_error(&d, "");
    ended(&until_stable(&d), 1, "error", 1);
    assert!(!d.join("t.rstex-tools").exists());
    // Nor is anything named before a pass with no record there.
    ended(&until_stable(&d), 1, "error", 1);
}

/// bibtex in the same way, and a `.bib` file changed since it ran comes first whatever the pass is going to do, once.
#[test]
fn a_bib_file_changed_comes_before_the_pass() {
    let d = scratch("error-bib");
    write(&d, "refs.bib", "@misc{a,}\n");
    write(&d, "mine.bst", "ENTRY{}{}{}\n");
    bib_doc_with_error(&d, CITES_A);
    let o = until_stable(&d);
    ended(&o, 1, "error", 1);
    assert!(!err(&o).contains("tool="), "{}", err(&o));
    let o = until_stable(&d);
    assert!(err(&o).contains("rstex: until-stable: tool=bibtex reason=output-missing file=y.aux\n"), "{}", err(&o));
    ended(&o, 0, "tool", 10);
    std::thread::sleep(std::time::Duration::from_millis(20));
    write(&d, "y.bbl", "made\n");
    ended(&until_stable(&d), 1, "error", 1);
    write(&d, "refs.bib", "@misc{a,}\n@misc{b,}\n");
    let log = fs::read(d.join("t.log")).unwrap();
    let o = until_stable(&d);
    assert!(err(&o).contains("rstex: until-stable: tool=bibtex reason=source-changed file=y.aux\n"), "{}", err(&o));
    ended(&o, 0, "tool", 10);
    assert_eq!(fs::read(d.join("t.log")).unwrap(), log);
    // A program that does not write its output has had its turn.
    ended(&until_stable(&d), 1, "error", 1);
    std::thread::sleep(std::time::Duration::from_millis(20));
    write(&d, "y.bbl", "made again\n");
    let o = until_stable(&d);
    ended(&o, 1, "error", 1);
    // On a document with no error it saves a pass as well.
    bib_doc(&d, CITES_A);
    ended(&until_stable(&d), 1, "stable", 0);
    write(&d, "refs.bib", "@misc{a,}\n");
    let o = until_stable(&d);
    assert!(err(&o).contains("reason=source-changed"), "{}", err(&o));
    ended(&o, 0, "tool", 10);
}

/// As latexmk's `$bibtex_use = 1`: no `.bib` file, no bibtex.  It runs when the file comes, as the rule was there all the time.
#[test]
fn no_bib_file_no_bibtex_until_there_is_one() {
    let d = scratch("error-no-bib");
    bib_doc_with_error(&d, CITES_A);
    ended(&until_stable(&d), 1, "error", 1);
    let o = until_stable(&d);
    ended(&o, 1, "error", 1);
    assert!(!err(&o).contains("tool="), "{}", err(&o));
    write(&d, "refs.bib", "@misc{a,}\n");
    let o = until_stable(&d);
    assert!(err(&o).contains("tool=bibtex reason=output-missing"), "{}", err(&o));
    ended(&o, 0, "tool", 10);
    // Gone again before the program ran: nothing to run it on.
    let d = scratch("error-bib-gone");
    write(&d, "refs.bib", "@misc{a,}\n");
    bib_doc_with_error(&d, CITES_A);
    ended(&until_stable(&d), 1, "error", 1);
    fs::remove_file(d.join("refs.bib")).unwrap();
    ended(&until_stable(&d), 1, "error", 1);
}

/// The `.aux` gone, or the `.idx`: as latexmk, a program with no main source does not run.
#[test]
fn a_program_whose_input_is_gone_does_not_come_first() {
    let d = scratch("error-input-gone");
    index_doc_with_error(&d, "one");
    ended(&until_stable(&d), 1, "error", 1);
    fs::remove_file(d.join("y.idx")).unwrap();
    ended(&until_stable(&d), 1, "error", 1);
}

/// biber: a `.bcf` a pass left cut short goes aside with its `.bbl`, under latexmk's names, and a whole one is as `.idx` above.
#[test]
fn a_bcf_cut_short_goes_aside() {
    let d = scratch("error-bcf");
    write(&d, "t.bbl", "from before\n");
    with_error(&d, "\\immediate\\openin3=t.bbl \\immediate\\closein3 \\immediate\\openout4=t.bcf \\immediate\\write4{<bcf:controlfile>}\\immediate\\closeout4 ");
    let o = until_stable(&d);
    ended(&o, 1, "error", 1);
    assert!(!d.join("t.bcf").exists() && !d.join("t.bbl").exists());
    assert_eq!(fs::read_to_string(d.join("t.bbl-SAVE-ERROR")).unwrap(), "from before\n");
    assert!(fs::read_to_string(d.join("t.bcf-SAVE-ERROR")).unwrap().contains("<bcf:controlfile>"));
    // A pass with no error can be cut short too.
    let d = scratch("clean-bcf");
    doc(&d, "\\immediate\\openout4=t.bcf \\immediate\\write4{<bcf:controlfile>}\\immediate\\closeout4 ");
    ended(&until_stable(&d), 1, "stable", 0);
    assert!(!d.join("t.bcf").exists() && d.join("t.bcf-SAVE-ERROR").exists());

    let d = scratch("error-bcf-whole");
    write(&d, "refs.bib", "@misc{a,}\n");
    write(&d, "n.tex", "\\def\\bcflines{<bcf:controlfile>^^J<bcf:datasource type=\"file\" datatype=\"bibtex\" glob=\"false\">refs.bib</bcf:datasource>^^J</bcf:controlfile>}\n");
    with_error(&d, "\\input n.tex \\newlinechar=`\\^^J \\immediate\\openin3=t.bbl \\immediate\\closein3 \\immediate\\openout4=t.bcf \\immediate\\write4{\\bcflines}\\immediate\\closeout4 ");
    ended(&until_stable(&d), 1, "error", 1);
    assert!(d.join("t.bcf").exists());
    let o = until_stable(&d);
    assert!(err(&o).contains("rstex: until-stable: tool=biber reason=output-missing file=t.bcf\n"), "{}", err(&o));
    ended(&o, 0, "tool", 10);
}

/// `\include` under an output directory with no such directory.  latexmk reads `! I can't write on file` in the log, makes the directory and goes on, and that is not the line `-file-line-error` gives.
#[test]
fn a_missing_directory_is_made_if_latexmk_would_see_it() {
    let body = "\\immediate\\openout4=sub/dir/x.aux \\immediate\\write4{\\relax}\\immediate\\closeout4 ";
    let d = scratch("no-dir");
    fs::create_dir(d.join("o")).unwrap();
    doc(&d, body);
    let o = rstex(&d, &["--until-stable", "-output-directory=o"]);
    assert!(err(&o).contains("rstex: until-stable: pass=1 "), "{}", err(&o));
    ended(&o, 2, "stable", 0);
    assert!(d.join("o/sub/dir/x.aux").exists());

    let d = scratch("no-dir-fle");
    fs::create_dir(d.join("o")).unwrap();
    doc(&d, body);
    let o = rstex(&d, &["--until-stable", "-file-line-error", "-output-directory=o"]);
    ended(&o, 1, "error", 1);
    assert!(!d.join("o/sub").exists());

    // With no passes left there is nobody to take the directory further.
    let d = scratch("no-dir-limit");
    fs::create_dir(d.join("o")).unwrap();
    doc(&d, body);
    ended(&rstex(&d, &["--until-stable", "--max-passes=1", "-output-directory=o"]), 1, "error", 1);

    // Nor without an output directory: pdfTeX's error stands.
    let d = scratch("no-dir-here");
    doc(&d, body);
    ended(&until_stable(&d), 1, "error", 1);
    assert!(!d.join("sub").exists());
}

/// A file of the user's that changes during a pass that fails: latexmk goes on, if it knows the file from before, and a first build does not.
#[cfg(unix)]
#[test]
fn a_file_changed_from_outside_goes_on_past_an_error_with_a_history() {
    let d = scratch("error-outside");
    write(&d, "dep.tex", "\\relax\n");
    write(&d, "other.sh", "test -f done || { touch done; printf '\\\\relax \\\\relax\\n' > dep.new; mv dep.new dep.tex; }\n");
    with_error(&d, "\\input dep.tex \\immediate\\write18{sh other.sh}");
    // No `.aux` here: as a first build.
    ended(&rstex(&d, &["--until-stable", "-shell-escape"]), 1, "error", 1);
    // With one, what the pass read is held against what it held before.
    write(&d, "t.aux", "\n");
    fs::remove_file(d.join("done")).unwrap();
    write(&d, "dep.tex", "\\relax\n");
    let o = rstex(&d, &["--until-stable", "-shell-escape"]);
    assert!(err(&o).contains("rstex: until-stable: pass=1 changed=1 file=dep.tex\n"), "{}", err(&o));
    ended(&o, 2, "error", 1);
    // What the pass wrote itself is no such change.
    let d = scratch("error-own");
    write(&d, "t.aux", "\n");
    write(&d, "x.dat", "0\n");
    with_error(&d, &counter(2));
    ended(&until_stable(&d), 1, "error", 1);
    // And at the limit a failed pass ends as a failed pass.
    let d = scratch("error-outside-limit");
    write(&d, "t.aux", "\n");
    write(&d, "dep.tex", "\\relax\n");
    write(&d, "other.sh", "test -f done || { touch done; printf '\\\\relax \\\\relax\\n' > dep.new; mv dep.new dep.tex; }\n");
    with_error(&d, "\\input dep.tex \\immediate\\write18{sh other.sh}");
    ended(&rstex(&d, &["--until-stable", "--max-passes=1", "-shell-escape"]), 1, "error", 1);
}

/// Before a pass, a look at the record, and at nothing when the first line is not a file name.
#[test]
fn the_first_line_says_whose_record_it_is() {
    let d = scratch("error-first-line");
    write(&d, "n.tex", "\\def\\entry{one}\n");
    write(&d, "t.tex", &format!("{CATCODES}\\input n.tex \\immediate\\openin3=y.ind \\immediate\\closein3 \\immediate\\openout4=y.idx \\immediate\\write4{{\\entry}}\\immediate\\closeout4 \\nosuchmacro\n\\end\n"));
    let run = |args: &[&str]| Command::new(env!("CARGO_BIN_EXE_rstex")).arg("-ini").arg("--until-stable").args(args).current_dir(&d).env("XDG_CACHE_HOME", d.join("cache")).stdin(Stdio::null()).output().unwrap();
    ended(&run(&["./t.tex"]), 1, "error", 1);
    ended(&run(&["t"]), 0, "tool", 10);
    // Under another job's name it is another record.
    fs::remove_file(d.join("t.rstex-tools")).unwrap();
    ended(&run(&["-jobname=other", "t.tex"]), 1, "error", 1);
    ended(&run(&["t.tex"]), 1, "error", 1);
    assert!(d.join("other.rstex-tools").exists());
    ended(&run(&["-jobname=other", "t.tex"]), 0, "tool", 10);
    // No file name to go by: a pass, and then the status it ends in.
    fs::remove_file(d.join("other.rstex-tools")).unwrap();
    ended(&run(&["\\input t.tex"]), 1, "error", 1);
    assert!(d.join("t.rstex-tools").exists());
    ended(&run(&["\\input t.tex"]), 1, "error", 1);
}

/// A second run of a document that has no tree: the process stays one, whatever the number of passes, so a signal to it ends the passes.
#[cfg(unix)]
#[test]
fn a_new_pass_is_the_same_process() {
    use std::os::unix::process::ExitStatusExt;
    let d = scratch("signal");
    write(&d, "x.dat", "0\n");
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
