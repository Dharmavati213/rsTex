//! `--until-stable`: passes until the side files a pass reads stop changing, as latexmk has it, and no latexmk.  DESIGN.md section 7, "The driver".
//!
//! A pass is an ordinary run with the same options, the same files and the same checkpoint key.  The engine notes what it does with files outside the tree (`PassRecord` in
//! `crates/engine/src/host.rs`), and at its very end, with everything on the disk, calls [`after_pass`].  That decides: exit, or this program again in place of this process.  So
//! there is no parent process to pay for, a pass that settles at once costs what a plain run costs, and a signal to the process id reaches the pass that is running.
//!
//! **When there is another pass.**  When a file the pass read before writing it now holds something else, or a file it read and did not write has another stat (something else changed
//! it, such as a save during the pass, or a `\write18` program), or a name it looked for and did not find has been made by the pass and the log says it was missing, by the lines latexmk reads
//! for that: `\IfFileExists` alone leaves no such line, and under latexmk no second run.  What a pass reads after writing it is its own doing, and the next pass writes it first as well.  The PDF is
//! no side file: latexmk leaves it out too, for a document that asks its own size.  After `limit` passes it stops.  With no `.aux`, a first pass finds what latexmk leaves there before its first run,
//! and what a one-page document with no references writes, so that such a document needs one pass, and a pass that fails says what it says under latexmk.  If it writes no `.aux`, as a plain TeX
//! document does not, the file does not stay.  With an output directory the environment is what latexmk makes it, by [`search_paths`], or `\input gen` of a `gen.tex` that a document wrote there would
//! work under one and fail under the other.
//!
//! **When a pass ends in an error,** as latexmk's `rdb_make` has it, that is the end, with TeX's status, unless what it missed was a directory for a `.aux`, which has now been made, or a file it read and
//! did not write has changed, in a pass with an earlier state to hold that against.  No program is named.  What is stale stays so and counts as seen ([`after_an_error`]), as under latexmk's
//! `rdb_update_gen_files`, so that only another change brings it back.  A result that is missing is another thing: the next run begins with its program, whatever else has changed, and before any
//! pass ([`before_the_first_pass`]), as latexmk's pre-primary rules do, and the same goes for a `.bib` or `.bst` file changed since.
//!
//! **Bibliography and index programs.**  rstex does not run them.  It says which one must run and why, in a line of its own on stderr and by status 10, and the caller runs it, or
//! latexmk, and comes back.  A line goes out, and passes stop, when the program's result is stale:
//!
//! - `bibtex`, for a `.bbl` the pass read or looked for, whose `.aux` the pass wrote, with a `\bibdata` in it, all of whose `.bib` files exist.  It reads of the `.aux` files only
//!   `\citation`, `\bibdata`, `\bibstyle` and `\@input`, so the digest is of those lines in that order, and a new `\newlabel` is not a reason, where latexmk takes the whole `.aux`.
//! - `biber`, for a `.bcf` the pass wrote: the digest of the whole file, as for latexmk.  One that a pass left cut short goes aside with its `.bbl`, under the names latexmk gives them.
//! - `makeindex`, for a `.idx` the pass wrote and a `.ind` it read or looked for: the digest of the whole `.idx`.  latexmk runs it even with no `.ind` wanted.
//!
//! What the result was made from is kept in `<job>.rstex-tools` beside the outputs, only for a document that has such a program.  A program leaves no trace of what it read, so the record
//! holds the stamp of its output as last seen: if that has moved, the program has run, on what the disk held then, which is what this pass found when it began.  With no record and an output
//! there, nothing says what it came from, and the line goes out once.  latexmk treats that the same, by comparing times when it has no record.
#![cfg_attr(test, allow(dead_code))]

use crate::System;
use rstex_engine::checkpoint::xxh64;
use rstex_engine::host::{PassEnd, PassRecord};
use rstex_engine::platform::{self, FileStat};
use rstex_kpse as kpse;

/// The status when a named program must run.
const STATUS_TOOL: i32 = 10;
/// The status when the limit was reached with files still changing.
const STATUS_LIMIT: i32 = 11;
/// latexmk's `$max_repeat`.
const DEFAULT_LIMIT: u32 = 5;
/// latexmk's `$save_error_suffix`.
const SET_ASIDE: &[u8] = b"-SAVE-ERROR";

/// What the command line says of this.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UntilStable {
    pub on: bool,
    /// This pass's number, which only a pass gives the next.
    pub pass: u32,
    pub limit: u32,
}

impl Default for UntilStable {
    fn default() -> UntilStable {
        UntilStable { on: false, pass: 1, limit: DEFAULT_LIMIT }
    }
}

const FLAG: [&[u8]; 2] = [b"--until-stable", b"-until-stable"];
const LIMIT: [&[u8]; 2] = [b"--max-passes=", b"-max-passes="];
const PASS: &[u8] = b"--until-stable-pass=";

/// True for an option of this file.
pub fn is_option(arg: &[u8]) -> bool {
    FLAG.contains(&arg) || LIMIT.iter().any(|p| arg.starts_with(p)) || arg.starts_with(PASS)
}

/// Takes `arg`, which [`is_option`] said is one of these.
pub fn take_option(arg: &[u8], u: &mut UntilStable) -> Result<(), Vec<u8>> {
    let number = |v: &[u8]| std::str::from_utf8(v).ok().and_then(|s| s.parse::<u32>().ok()).filter(|n| *n > 0);
    if FLAG.contains(&arg) {
        u.on = true;
    } else if let Some(v) = arg.strip_prefix(PASS) {
        u.pass = number(v).ok_or_else(|| b"rstex: --until-stable-pass wants a positive number\n".to_vec())?;
    } else if let Some(v) = LIMIT.iter().find_map(|p| arg.strip_prefix(*p)) {
        u.limit = number(v).ok_or_else(|| b"rstex: --max-passes wants a positive number\n".to_vec())?;
    }
    Ok(())
}

/// What latexmk's `normalize_aux_out_ETC` does for an output directory: it goes in front of `TEXINPUTS` and `BIBINPUTS` unless it is there, by its `ensure_path`, and into `TEXMFOUTPUT`.
pub fn search_paths(dir: &[u8], sys: &mut dyn System) {
    if dir == b"." || dir == b"./" {
        return;
    }
    let sep = if platform::DOSISH { b';' } else { b':' };
    for var in [&b"BIBINPUTS"[..], b"TEXINPUTS"] {
        let old = sys.getenv(var);
        let there = old.as_deref().is_some_and(|v| v.split(|&c| c == sep).any(|c| c == dir));
        if !there {
            let new = [dir, &[sep][..], old.as_deref().unwrap_or(b"")].concat();
            sys.putenv(var, &new);
        }
    }
    sys.putenv(b"TEXMFOUTPUT", dir);
}

/// Has the engine keep a record of this pass.
pub fn install(world: &mut rstex_engine::host::World, u: UntilStable) {
    if u.on {
        world.pass = Some(PassRecord::new(u.pass, u.limit, after_pass));
    }
}

fn say(parts: &[&[u8]]) {
    let mut line = b"rstex: until-stable: ".to_vec();
    for p in parts {
        line.extend_from_slice(p);
    }
    line.push(b'\n');
    platform::write_stderr(&line);
}

/// The job's name by the first word of the first line, as TeX's `more_name` and `end_name` will have it, if that is a file to read: `./a.tex` is `a`.  Nothing, for a line that begins with something else: what is
/// asked here then waits for a pass that ends well.
fn job_on_the_command_line(line: &[u8]) -> Option<Vec<u8>> {
    let mut l = line.trim_ascii_start();
    if l.first() == Some(&b'&') {
        l = l[l.iter().position(|&c| c == b' ')?..].trim_ascii_start();
    }
    if matches!(l.first(), None | Some(b'\\' | b'*')) {
        return None;
    }
    let (mut name, mut quoted) = (Vec::new(), false);
    for &c in l {
        match c {
            b'"' => quoted = !quoted,
            b' ' if !quoted => break,
            _ => name.push(c),
        }
    }
    let base = base_name(&name);
    let stem = base.iter().rposition(|&c| c == b'.').map_or(base, |dot| &base[..dot]);
    (!stem.is_empty()).then(|| stem.to_vec())
}

/// Before the first pass of a run, what latexmk's pre-primary rules do before its first: a program whose result is missing, or one with a file of the user's changed since it ran.  It runs on what is there now,
/// so that the pass that follows finds what it finds under latexmk.  A document with no such program pays one failed open, and with the flag only.  The status to end with, if a program must run.
pub fn before_the_first_pass(u: UntilStable, job: Option<&[u8]>, command_line: &[u8], dir: Option<&[u8]>) -> Option<i32> {
    if !u.on || u.pass != 1 {
        return None;
    }
    let job = job.map(<[u8]>::to_vec).or_else(|| job_on_the_command_line(command_line))?;
    let state = in_dir(dir, &[&job[..], b".rstex-tools"].concat());
    let mut records = parse_records(&read(&state)?)?;
    let needs: Vec<Need> = records.iter_mut().filter_map(|r| comes_first(r, dir).map(|reason| Need { tool: r.tool_name(), reason, file: r.file.clone() })).collect();
    if needs.is_empty() {
        return None;
    }
    let _ = std::fs::write(platform::path_from_bytes(&state), records_text(&records));
    for n in &needs {
        say(&[b"tool=", n.tool.as_bytes(), b" reason=", n.reason.as_bytes(), b" file=", &n.file]);
    }
    say(&[b"passes=0 result=tool"]);
    Some(STATUS_TOOL)
}

/// Why this program comes before any pass.  It says so once: if nothing comes of it, a pass is next, as latexmk does not try a program again for what it has failed on.
fn comes_first(r: &mut Record, dir: Option<&[u8]>) -> Option<&'static str> {
    let out = r.output()?;
    // Its main source is gone: as latexmk, "a run is pointless".  A `.bib` file gone: as its default `$bibtex_use = 1`, no run.
    platform::file_stat(&r.file)?;
    let (mut digest, mut changed) = (0u64, false);
    if r.paths.len() == r.sources.len() {
        for (path, (name, hash)) in r.paths.iter().zip(&r.sources) {
            // One that was not there when the record was made has no place yet, and may be anywhere a search finds it.
            let found = if path.is_empty() { find_source(dir, name, kpse::KPSE_BIB_FORMAT)? } else { path.clone() };
            let h = xxh64(&read(&found)?, 0);
            changed |= h != *hash;
            digest = xxh64(&[digest.to_le_bytes(), h.to_le_bytes()].concat(), 0);
        }
    }
    let stamp = platform::file_stat(&out);
    if stamp.is_none() {
        if !r.pending && r.stamp.is_none() {
            return None;
        }
        r.pending = false;
        r.stamp = None;
        return Some(REASON_MISSING);
    }
    if changed && r.stamp == stamp && r.named != Some(digest) {
        r.named = Some(digest);
        return Some(REASON_SOURCE);
    }
    None
}

/// The engine's last call.
fn after_pass(rec: &PassRecord, end: &PassEnd<'_>) -> i32 {
    let Some(job) = end.job else {
        return end.code;
    };
    let passes = rec.number.to_string();
    let log = in_dir(end.output_directory, &[job, b".log"].concat());
    let mut made_dir = None;
    if end.code != 0 {
        // latexmk's main loop on a `$failure`: another pass only for what can change the answer.
        made_dir = missing_directory(end.output_directory, &log);
        if rec.number >= rec.limit || (made_dir.is_none() && !changed_from_outside(rec)) {
            after_an_error(rec, end, job);
            say(&[b"passes=", passes.as_bytes(), b" result=error"]);
            return end.code;
        }
    }
    let needs = judge_tools(rec, end, job);
    if !needs.is_empty() {
        for n in &needs {
            say(&[b"tool=", n.tool.as_bytes(), b" reason=", n.reason.as_bytes(), b" file=", &n.file]);
        }
        say(&[b"passes=", passes.as_bytes(), b" result=tool"]);
        return STATUS_TOOL;
    }
    // The log is read only if nothing else says there is another pass, and a pass has made a file it looked for: a document that has settled makes none.
    let missed = if rec.changed.is_empty() { missed_in_the_log(rec, end.output_directory, &log) } else { Vec::new() };
    let Some(first) = rec.changed.first().or(missed.first()).or(made_dir.as_ref()) else {
        say(&[b"passes=", passes.as_bytes(), b" result=stable"]);
        return 0;
    };
    if rec.number >= rec.limit {
        say(&[b"passes=", passes.as_bytes(), b" result=limit file=", first]);
        return STATUS_LIMIT;
    }
    let count = if rec.changed.is_empty() { missed.len().max(1) } else { rec.changed.len() + rec.made.len() };
    say(&[b"pass=", passes.as_bytes(), b" changed=", count.to_string().as_bytes(), b" file=", first]);
    // The number goes first, and an earlier pass's number is there.
    let mut old = std::env::args_os();
    let mut args = vec![old.next().unwrap_or_else(|| "rstex".into()), format!("--until-stable-pass={}", rec.number + 1).into()];
    args.extend(old.skip(usize::from(rec.number > 1)));
    platform::run_again(&args)
}

/// A file the pass read and did not write holds something else now, which latexmk takes for a change of the user's, and goes on after an error for it.  It sees that by the state it held before the run, and a first
/// run of a document has none: it takes each file's state after the run, and sees nothing.
fn changed_from_outside(rec: &PassRecord) -> bool {
    !rec.no_history && rec.changed.iter().any(|p| rec.file(p).is_some_and(|f| !f.written))
}

// ---------------------------------------------------------------------------
// What the log says, by latexmk's `get_log_file` and `parse_log`.  Only an unusual pass comes here.
// ---------------------------------------------------------------------------

/// The lines of a log as they stand and as TeX meant them.  Of a line of exactly the width nobody can tell if it was cut, so latexmk tries both as well.
fn each_log_line(log: &[u8], act: &mut dyn FnMut(&[u8])) {
    let wrap = kpse::var_value(b"max_print_line").and_then(|v| std::str::from_utf8(&v).ok()?.trim().parse::<usize>().ok()).unwrap_or(79);
    let mut held: Vec<u8> = Vec::new();
    for line in log.split(|&c| c == b'\n') {
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        act(line);
        if line.len() == wrap {
            held.extend_from_slice(line);
        } else if !held.is_empty() {
            held.extend_from_slice(line);
            act(&held);
            held.clear();
        }
    }
}

fn position(line: &[u8], what: &[u8]) -> Option<usize> {
    line.windows(what.len()).position(|w| w == what)
}

/// What stands between the first `open` and the first `close` after it.
fn between<'a>(line: &'a [u8], open: &[u8], close: &[u8]) -> Option<&'a [u8]> {
    let rest = &line[position(line, open)? + open.len()..];
    Some(&rest[..position(rest, close)?])
}

/// The name of what a line says is missing: latexmk's `@file_not_found`.
fn says_missing(line: &[u8]) -> Option<&[u8]> {
    if let Some(rest) = line.strip_prefix(b"No file") {
        let rest = rest.trim_ascii_start();
        return Some(match position(rest, b": Run ") {
            Some(n) => &rest[..n],
            None => rest.strip_suffix(b".").unwrap_or(rest.trim_ascii_end()),
        });
    }
    if line.starts_with(b"! LaTeX Error: File `") || line.starts_with(b"LaTeX Warning: File `") {
        return between(line, b"File `", b"' not found");
    }
    if line.starts_with(b"! I can't find file `") {
        return between(line, b"file `", b"'.");
    }
    if line.starts_with(b"Package ") {
        return between(line, b"ile `", b"' not found").or_else(|| between(line, b" No file `", b"'"));
    }
    between(line, b": LaTeX Error: File `", b"' not found")
        .or_else(|| between(line, b"! Unable to load picture or PDF file '", b"'."))
        .or_else(|| between(line, b"Error: pdflatex (file ", b"): cannot find image file"))
        .or_else(|| {
            let end = line.trim_ascii_end().strip_suffix(b" not found:")?;
            Some(&end[position(end, b": File ")? + b": File ".len()..])
        })
}

/// Of what the pass made after looking for it, what the log says it missed: only that takes latexmk another run, as the `.fls` has no word of a look that found nothing.
fn missed_in_the_log(rec: &PassRecord, dir: Option<&[u8]>, log: &[u8]) -> Vec<Vec<u8>> {
    if rec.made.is_empty() {
        return Vec::new();
    }
    let Some(text) = read(log) else { return Vec::new() };
    let mut missed = Vec::new();
    each_log_line(&text, &mut |line| {
        if let Some(name) = says_missing(line) {
            let inside = in_dir(dir, name);
            let bare = name.strip_prefix(b"./").unwrap_or(name);
            for m in rec.made.iter().filter(|m| m.as_slice() == bare || m.as_slice() == inside.as_slice()) {
                if !missed.contains(m) {
                    missed.push(m.clone());
                }
            }
        }
    });
    missed
}

/// `\include` of a file whose directory the output directory does not have.  latexmk makes it and goes on, if the log says so in the form it reads, and with `-file-line-error` it does not: that stays an error.  The
/// directory, if one was made.
fn missing_directory(dir: Option<&[u8]>, log: &[u8]) -> Option<Vec<u8>> {
    let dir = dir?;
    let text = read(log)?;
    let mut wanted = Vec::new();
    each_log_line(&text, &mut |line| {
        let Some(rest) = line.strip_prefix(b"! I can't write on file `") else { return };
        let Some(rest) = rest.trim_ascii_end().strip_suffix(b"'.") else { return };
        let Some(slash) = rest.iter().rposition(|&c| c == b'/') else { return };
        if rest[slash + 1..].ends_with(b".aux") && !rest[slash + 1..].contains(&b'\'') {
            wanted.push(in_dir(Some(dir), &rest[..slash]));
        }
    });
    let mut made = None;
    for w in wanted {
        let path = platform::path_from_bytes(&w);
        if !path.exists() && std::fs::create_dir_all(&path).is_ok() {
            made.get_or_insert(w);
        }
    }
    made
}

// ---------------------------------------------------------------------------
// The programs a document needs.
// ---------------------------------------------------------------------------

const REASON_MISSING: &str = "output-missing";
const REASON_NO_RECORD: &str = "no-record";
const REASON_SOURCE: &str = "source-changed";

/// One program a document needs, as this pass shows it.
struct Rule {
    tool: &'static str,
    /// What the program is given.
    file: Vec<u8>,
    /// What it writes.
    out: Vec<u8>,
    /// Why, if what it reads of what TeX wrote has changed.
    reason: &'static str,
    /// The digest of what it reads of what TeX wrote, now and when this pass began.
    now: u64,
    before: Option<u64>,
    /// What else it reads, which TeX did not write: `.bib` and `.bst`.
    sources: Vec<Source>,
}

impl Rule {
    fn source_digests(&self) -> Vec<(Vec<u8>, u64)> {
        self.sources.iter().map(|s| (s.name.clone(), s.hash)).collect()
    }

    fn source_paths(&self) -> Vec<Vec<u8>> {
        self.sources.iter().map(|s| s.path.clone()).collect()
    }
}

/// A file a program reads that TeX did not write, under the name it asks for it.
struct Source {
    name: Vec<u8>,
    path: Vec<u8>,
    hash: u64,
}

impl Source {
    fn of(name: &[u8], path: Vec<u8>) -> Option<Source> {
        let hash = xxh64(&read(&path)?, 0);
        Some(Source { name: name.to_vec(), path, hash })
    }

    /// A `.bib` file, or nothing without one: as latexmk's default, `$bibtex_use = 1`, a bibliography with no `.bib` file is what came with the document, and its program could not make it.  After an
    /// error that holds as long as the file is not there, and the program comes first once it is, so it stays on record, with no place and no digest.
    fn bib(dir: Option<&[u8]>, name: &[u8], after_an_error: bool) -> Option<Source> {
        let found = find_source(dir, name, kpse::KPSE_BIB_FORMAT).and_then(|p| Source::of(name, p));
        found.or_else(|| after_an_error.then(|| Source { name: name.to_vec(), path: Vec::new(), hash: 0 }))
    }
}

struct Need {
    tool: &'static str,
    reason: &'static str,
    file: Vec<u8>,
}

fn in_dir(dir: Option<&[u8]>, name: &[u8]) -> Vec<u8> {
    match dir {
        Some(d) if !platform::is_absolute(name) => [d, b"/", name].concat(),
        _ => name.to_vec(),
    }
}

fn read(path: &[u8]) -> Option<Vec<u8>> {
    std::fs::read(platform::path_from_bytes(path)).ok()
}

fn base_name(path: &[u8]) -> &[u8] {
    path.rsplit(|&c| c == b'/' || (platform::DOSISH && c == b'\\')).next().unwrap_or(path)
}

/// A file a program reads, as bibtex or biber would find it: beside the outputs, or on its search path.
fn find_source(dir: Option<&[u8]>, name: &[u8], format: i32) -> Option<Vec<u8>> {
    let there = in_dir(dir, name);
    if platform::file_stat(&there).is_some() {
        return Some(there);
    }
    kpse::find_file(name, format, false)
}

/// What bibtex reads of a `.aux` and those it inputs.
#[derive(Default)]
struct AuxLines {
    lines: Vec<u8>,
    bibdata: Vec<Vec<u8>>,
    bibstyle: Option<Vec<u8>>,
}

/// `\name{arg}` at the start of a line, as bibtex looks for it.
fn aux_arg<'a>(line: &'a [u8], name: &[u8]) -> Option<&'a [u8]> {
    let rest = line.strip_prefix(name)?;
    let close = rest.iter().rposition(|&c| c == b'}')?;
    Some(&rest[..close])
}

fn walk_aux(path: &[u8], dir: Option<&[u8]>, get: &dyn Fn(&[u8]) -> Option<Vec<u8>>, out: &mut AuxLines, depth: u32) {
    let Some(data) = get(path) else { return };
    for line in data.split(|&c| c == b'\n') {
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        let input = aux_arg(line, b"\\@input{");
        if line.starts_with(b"\\citation{") || line.starts_with(b"\\bibdata{") || line.starts_with(b"\\bibstyle{") || input.is_some() {
            out.lines.extend_from_slice(line);
            out.lines.push(b'\n');
        }
        if let Some(a) = aux_arg(line, b"\\bibdata{") {
            out.bibdata.extend(a.split(|&c| c == b',').filter(|n| !n.is_empty()).map(<[u8]>::to_vec));
        } else if let Some(a) = aux_arg(line, b"\\bibstyle{") {
            out.bibstyle = Some(a.to_vec());
        } else if let Some(a) = input {
            // bibtex's own limit on how deep this goes is 20.
            if depth < 20 {
                walk_aux(&in_dir(dir, a), dir, get, out, depth + 1);
            }
        }
    }
}

fn digest_of(lines: &[u8]) -> u64 {
    xxh64(lines, 0)
}

/// bibtex for the `.aux` at `aux`, if that has what it takes.
fn bibtex_rule(rec: &PassRecord, dir: Option<&[u8]>, aux: Vec<u8>, bbl: Vec<u8>, after_an_error: bool) -> Option<Rule> {
    let mut now = AuxLines::default();
    walk_aux(&aux, dir, &|p| read(p), &mut now, 0);
    if now.bibdata.is_empty() {
        return None;
    }
    // What the disk held when the pass began: what it has rewritten since is in the record, and what it has not is as it was.
    let mut then = AuxLines::default();
    let before = |p: &[u8]| match rec.file(p) {
        Some(f) if f.written => f.before_bytes.clone(),
        _ => read(p),
    };
    walk_aux(&aux, dir, &before, &mut then, 0);
    let had_aux = rec.file(&aux).is_some_and(|f| f.before_bytes.is_some());
    let mut sources = Vec::new();
    for name in &now.bibdata {
        let with_ext = if name.ends_with(b".bib") { name.clone() } else { [&name[..], b".bib"].concat() };
        sources.push(Source::bib(dir, &with_ext, after_an_error)?);
    }
    if let Some(style) = &now.bibstyle {
        let with_ext = if style.ends_with(b".bst") { style.clone() } else { [&style[..], b".bst"].concat() };
        sources.extend(find_source(dir, &with_ext, kpse::KPSE_BST_FORMAT).and_then(|p| Source::of(&with_ext, p)));
    }
    Some(Rule {
        tool: "bibtex",
        file: aux,
        out: bbl,
        reason: "citations-changed",
        now: digest_of(&now.lines),
        before: had_aux.then(|| digest_of(&then.lines)),
        sources,
    })
}

/// biber for `<job>.bcf`, if the pass wrote it.
fn biber_rule(rec: &PassRecord, dir: Option<&[u8]>, job: &[u8], after_an_error: bool) -> Option<Rule> {
    let bcf = in_dir(dir, &[job, b".bcf"].concat());
    let f = rec.file(&bcf).filter(|f| f.written)?;
    let data = read(&bcf)?;
    let base = &bcf[..bcf.len() - 4];
    let bbl = [base, b".bbl"].concat();
    // A pass that stopped half way: biber could do nothing with it, and latexmk sets it aside, with the `.bbl` in case that is what stopped the pass.
    if !data.is_empty() && !data.trim_ascii_end().ends_with(b"</bcf:controlfile>") {
        for name in [&bcf, &bbl] {
            let _ = std::fs::rename(platform::path_from_bytes(name), platform::path_from_bytes(&[&name[..], SET_ASIDE].concat()));
        }
        return None;
    }
    let mut sources = Vec::new();
    for line in data.split(|&c| c == b'\n') {
        let line = line.trim_ascii_start();
        let has = |what: &[u8]| line.windows(what.len()).any(|w| w == what);
        if !line.starts_with(b"<bcf:datasource type=\"file\"") || !has(b"datatype=\"bibtex\"") {
            continue;
        }
        let Some(open) = line.iter().position(|&c| c == b'>') else { continue };
        let tail = &line[open + 1..];
        let Some(close) = tail.iter().position(|&c| c == b'<') else { continue };
        let name = &tail[..close];
        let remote = [&b"http:"[..], b"https:", b"ftp:", b"ftps:"].iter().any(|p| name.starts_with(p));
        let glob = has(b"glob=\"true\"") && name.iter().any(|c| b"*?[{".contains(c));
        if remote || glob {
            continue;
        }
        sources.push(Source::bib(dir, name, after_an_error)?);
    }
    Some(Rule { tool: "biber", out: bbl, file: bcf.clone(), reason: "bcf-changed", now: xxh64(&data, 0), before: f.before, sources })
}

/// All this pass shows a need of.
fn tool_rules(rec: &PassRecord, end: &PassEnd<'_>, job: &[u8], after_an_error: bool) -> Vec<Rule> {
    let dir = end.output_directory;
    let mut rules = Vec::new();
    // What the pass asked for, whether it was there or not.
    let mut asked: Vec<Vec<u8>> = rec.files().filter(|(_, f)| f.read.is_some()).map(|(p, _)| p.to_vec()).collect();
    asked.extend(rec.probes.iter().map(|p| in_dir(dir, p)));
    // A `.bcf` of this pass means biber, as for latexmk, whatever becomes of its rule.
    let uses_biber = rec.file(&in_dir(dir, &[job, b".bcf"].concat())).is_some_and(|f| f.written);
    rules.extend(biber_rule(rec, dir, job, after_an_error));
    if !uses_biber {
        let mut seen: Vec<Vec<u8>> = Vec::new();
        for path in asked.iter().filter(|p| p.ends_with(b".bbl")) {
            let same = [&path[..path.len() - 4], b".aux"].concat();
            let (aux, bbl) = if rec.file(&same).is_some_and(|f| f.written) {
                (same, path.clone())
            } else {
                // Read from another directory than the outputs': bibtex makes it there, and the next pass finds that first.
                let name = &base_name(path)[..base_name(path).len() - 4];
                (in_dir(dir, &[name, b".aux"].concat()), in_dir(dir, &[name, b".bbl"].concat()))
            };
            if rec.file(&aux).is_some_and(|f| f.written) && !seen.contains(&aux) {
                seen.push(aux.clone());
                rules.extend(bibtex_rule(rec, dir, aux, bbl, after_an_error));
            }
        }
    }
    for (path, f) in rec.files() {
        if !f.written || !path.ends_with(b".idx") {
            continue;
        }
        let ind = [&path[..path.len() - 4], b".ind"].concat();
        if asked.iter().any(|a| base_name(a) == base_name(&ind)) {
            if let Some(data) = read(path) {
                rules.push(Rule { tool: "makeindex", file: path.to_vec(), out: ind, reason: "idx-changed", now: xxh64(&data, 0), before: f.before, sources: Vec::new() });
            }
        }
    }
    rules
}

// ---------------------------------------------------------------------------
// What each result was made from.
// ---------------------------------------------------------------------------

/// What is known of a program's result.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Record {
    tool: String,
    file: Vec<u8>,
    /// The digest of what it read of what TeX wrote, if known.
    made_from: Option<u64>,
    /// The output's stamp as last seen, or `None` if it was not there.
    stamp: Option<FileStat>,
    sources: Vec<(Vec<u8>, u64)>,
    /// Where `sources` were found: before a pass there is nothing to ask of kpathsea, or the `.fls` would show it.
    paths: Vec<Vec<u8>>,
    /// A pass that failed left the result missing: the next run begins with this program.
    pending: bool,
    /// The digest of `sources` a run has already begun with this program for, and is not to say again.
    named: Option<u64>,
}

impl Record {
    fn tool_name(&self) -> &'static str {
        match self.tool.as_str() {
            "bibtex" => "bibtex",
            "biber" => "biber",
            _ => "makeindex",
        }
    }

    /// What its program writes.
    fn output(&self) -> Option<Vec<u8>> {
        let (from, to): (&[u8], &[u8]) = match self.tool.as_str() {
            "bibtex" => (b".aux", b".bbl"),
            "biber" => (b".bcf", b".bbl"),
            "makeindex" => (b".idx", b".ind"),
            _ => return None,
        };
        Some([self.file.strip_suffix(from)?, to].concat())
    }
}

fn hex_or_dash(v: Option<u64>) -> String {
    v.map_or_else(|| "-".to_string(), |v| format!("{v:016x}"))
}

/// The text of the file: a first line that says what it is, a `rule` line for each program with the name of its file last, what is to be said of it next, and for each file it reads a `source` line and where that was found.
fn records_text(records: &[Record]) -> Vec<u8> {
    let mut t = b"rstex-tools 1\n".to_vec();
    for r in records {
        let stamp = match &r.stamp {
            Some(s) => format!("{} {} {} {} {}", s.size, s.mtime, s.mtime_nsec, s.change, s.inode),
            None => "- - - - -".to_string(),
        };
        t.extend_from_slice(format!("rule {} {} {} ", r.tool, hex_or_dash(r.made_from), stamp).as_bytes());
        t.extend_from_slice(&r.file);
        t.push(b'\n');
        if r.pending {
            t.extend_from_slice(b"pending\n");
        }
        if let Some(n) = r.named {
            t.extend_from_slice(format!("named {n:016x}\n").as_bytes());
        }
        for (k, (name, h)) in r.sources.iter().enumerate() {
            t.extend_from_slice(format!("source {h:016x} ").as_bytes());
            t.extend_from_slice(name);
            t.push(b'\n');
            if let Some(p) = r.paths.get(k) {
                t.extend_from_slice(b"from ");
                t.extend_from_slice(p);
                t.push(b'\n');
            }
        }
    }
    t
}

/// The records in a file's text.  Anything else means no records, and each program's line goes out once.
fn parse_records(text: &[u8]) -> Option<Vec<Record>> {
    let mut lines = text.split(|&c| c == b'\n');
    if lines.next()? != b"rstex-tools 1" {
        return None;
    }
    let mut records: Vec<Record> = Vec::new();
    for line in lines.filter(|l| !l.is_empty()) {
        if let Some(rest) = line.strip_prefix(b"rule ") {
            let mut f = rest.splitn(8, |&c| c == b' ');
            let tool = String::from_utf8(f.next()?.to_vec()).ok()?;
            let word = |w: &[u8]| std::str::from_utf8(w).ok().map(str::to_string);
            let made = word(f.next()?)?;
            let made_from = if made == "-" { None } else { Some(u64::from_str_radix(&made, 16).ok()?) };
            let (size, mtime, nsec, change, inode) = (word(f.next()?)?, word(f.next()?)?, word(f.next()?)?, word(f.next()?)?, word(f.next()?)?);
            let stamp = if size == "-" {
                None
            } else {
                Some(FileStat { size: size.parse().ok()?, mtime: mtime.parse().ok()?, mtime_nsec: nsec.parse().ok()?, change: change.parse().ok()?, inode: inode.parse().ok()?, regular: true })
            };
            records.push(Record { tool, file: f.next()?.to_vec(), made_from, stamp, sources: Vec::new(), paths: Vec::new(), pending: false, named: None });
        } else if line == b"pending" {
            records.last_mut()?.pending = true;
        } else if let Some(h) = line.strip_prefix(b"named ") {
            records.last_mut()?.named = Some(u64::from_str_radix(std::str::from_utf8(h).ok()?, 16).ok()?);
        } else if let Some(rest) = line.strip_prefix(b"source ") {
            let (h, name) = (rest.get(..16)?, rest.get(17..)?);
            let h = u64::from_str_radix(std::str::from_utf8(h).ok()?, 16).ok()?;
            records.last_mut()?.sources.push((name.to_vec(), h));
        } else if let Some(path) = line.strip_prefix(b"from ") {
            records.last_mut()?.paths.push(path.to_vec());
        } else {
            return None;
        }
    }
    Some(records)
}

/// What is known of the sources of a result that has been made again since the last look: the program read each as it is now, if that has not been edited since.
fn sources_at_its_run(rule: &Rule, r: &Record, out: &Option<FileStat>) -> Vec<(Vec<u8>, u64)> {
    let edited_after = |s: &Source| match (out, platform::file_stat(&s.path)) {
        (Some(o), Some(f)) => (f.mtime, f.mtime_nsec) > (o.mtime, o.mtime_nsec),
        _ => false,
    };
    let kept = |s: &Source| r.sources.iter().find(|o| o.0 == s.name).map_or(0, |o| o.1);
    rule.sources.iter().map(|s| (s.name.clone(), if edited_after(s) { kept(s) } else { s.hash })).collect()
}

/// The record of a rule after this pass, and why its program must run, if it must.
fn judge(rule: &Rule, old: Option<Record>) -> (Record, Option<&'static str>) {
    let out = platform::file_stat(&rule.out);
    let sources = rule.source_digests();
    let Some(mut r) = old else {
        let reason = if out.is_some() { REASON_NO_RECORD } else { REASON_MISSING };
        // The line goes out once: from here on, what is there counts as made from what TeX has just written, unless a program writes it again.
        let made_from = Some(rule.now).filter(|_| out.is_some());
        return (Record { tool: rule.tool.to_string(), file: rule.file.clone(), made_from, stamp: out, sources, paths: rule.source_paths(), pending: false, named: None }, Some(reason));
    };
    r.pending = false;
    r.paths = rule.source_paths();
    if r.stamp != out {
        // The program has run since the last look, on what TeX had left then, and on a source as it is now if that has not been edited since.
        r.sources = sources_at_its_run(rule, &r, &out);
        r.made_from = rule.before.or(r.made_from);
        r.stamp = out.clone();
        r.named = None;
    }
    let reason = if out.is_none() {
        Some(REASON_MISSING)
    } else if r.made_from != Some(rule.now) {
        Some(rule.reason)
    } else if r.sources != sources {
        Some(REASON_SOURCE)
    } else {
        None
    };
    (r, reason)
}

/// Judges each program this pass shows a need of, and keeps what that found.  A document with none has no file.
fn judge_tools(rec: &PassRecord, end: &PassEnd<'_>, job: &[u8]) -> Vec<Need> {
    let state = in_dir(end.output_directory, &[job, b".rstex-tools"].concat());
    let rules = tool_rules(rec, end, job, false);
    let loaded = read(&state);
    if rules.is_empty() {
        if loaded.is_some() {
            let _ = std::fs::remove_file(platform::path_from_bytes(&state));
        }
        return Vec::new();
    }
    let old = loaded.as_deref().and_then(parse_records).unwrap_or_default();
    let mut needs = Vec::new();
    let mut records = Vec::new();
    for rule in &rules {
        let prior = old.iter().find(|r| r.tool == rule.tool && r.file == rule.file).cloned();
        let (r, why) = judge(rule, prior);
        if let Some(reason) = why {
            needs.push(Need { tool: rule.tool, reason, file: rule.file.clone() });
        }
        records.push(r);
    }
    let text = records_text(&records);
    if loaded.as_deref() != Some(&text[..]) {
        let _ = std::fs::write(platform::path_from_bytes(&state), &text);
    }
    needs
}

/// The record of a rule after a pass that failed: as latexmk's `rdb_update_gen_files`, what it reads of what TeX wrote counts as seen and no program is named, or the error would bring reruns that repair nothing.  A file of
/// the user's stays as it was known.  A result that is missing is for the next run to begin with, unless its program was named for that already, and nothing came of it, which latexmk does not try again.
fn seen_under_an_error(rule: &Rule, old: Option<Record>) -> Record {
    let out = platform::file_stat(&rule.out);
    let Some(mut r) = old else {
        let pending = out.is_none();
        return Record { tool: rule.tool.to_string(), file: rule.file.clone(), made_from: Some(rule.now), stamp: out, sources: rule.source_digests(), paths: rule.source_paths(), pending, named: None };
    };
    let tried = r.stamp.is_none() && !r.pending;
    if r.stamp != out {
        r.sources = sources_at_its_run(rule, &r, &out);
        r.named = None;
    }
    r.pending = out.is_none() && !tried;
    r.made_from = Some(rule.now);
    r.stamp = out;
    r.paths = rule.source_paths();
    r
}

/// What a pass that failed leaves of the records.  A pass may stop before it comes to a program, and that says nothing of it, so what it did not show stays as it is.
fn after_an_error(rec: &PassRecord, end: &PassEnd<'_>, job: &[u8]) {
    let state = in_dir(end.output_directory, &[job, b".rstex-tools"].concat());
    let rules = tool_rules(rec, end, job, true);
    let loaded = read(&state);
    if rules.is_empty() {
        return;
    }
    let old = loaded.as_deref().and_then(parse_records).unwrap_or_default();
    let mut records: Vec<Record> = rules.iter().map(|rule| seen_under_an_error(rule, old.iter().find(|r| r.tool == rule.tool && r.file == rule.file).cloned())).collect();
    records.extend(old.iter().filter(|r| !rules.iter().any(|x| x.tool == r.tool && x.file == r.file)).cloned());
    let text = records_text(&records);
    if loaded.as_deref() != Some(&text[..]) {
        let _ = std::fs::write(platform::path_from_bytes(&state), &text);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn options_and_their_values() {
        let mut u = UntilStable::default();
        assert!(!u.on);
        assert_eq!(u.limit, 5);
        for a in [&b"--until-stable"[..], b"-until-stable", b"--max-passes=3", b"-max-passes=3", b"--until-stable-pass=2"] {
            assert!(is_option(a));
            take_option(a, &mut u).unwrap();
        }
        assert_eq!(u, UntilStable { on: true, pass: 2, limit: 3 });
        assert!(take_option(b"--max-passes=0", &mut u).is_err());
        assert!(take_option(b"--max-passes=x", &mut u).is_err());
        assert!(!is_option(b"--until"));
    }

    #[test]
    fn records_read_back() {
        let stat = FileStat { size: 12, mtime: 34, mtime_nsec: 56, change: 78, inode: 90, regular: true };
        let records = vec![
            Record {
                tool: "bibtex".into(),
                file: b".build/my doc.aux".to_vec(),
                made_from: Some(0xabc),
                stamp: Some(stat),
                sources: vec![(b"a b.bib".to_vec(), 1), (b"plain.bst".to_vec(), u64::MAX)],
                paths: vec![b"a b.bib".to_vec(), b"/tree/plain.bst".to_vec()],
                pending: false,
                named: Some(7),
            },
            Record { tool: "makeindex".into(), file: b"x.idx".to_vec(), made_from: None, stamp: None, sources: Vec::new(), paths: Vec::new(), pending: true, named: None },
        ];
        assert_eq!(parse_records(&records_text(&records)).unwrap(), records);
        assert!(parse_records(b"something else\n").is_none());
        assert!(parse_records(b"rstex-tools 1\nrule bibtex\n").is_none());
        assert!(parse_records(b"rstex-tools 1\npending\n").is_none());
        // What an earlier version wrote still reads, with nothing known of where a source was.
        let old = parse_records(b"rstex-tools 1\nrule bibtex - - - - - - y.aux\nsource 0000000000000001 refs.bib\n").unwrap();
        assert_eq!(old[0].sources.len(), 1);
        assert!(old[0].paths.is_empty() && !old[0].pending);
    }

    #[test]
    fn what_each_program_writes() {
        let r = |tool: &str, file: &[u8]| Record { tool: tool.into(), file: file.to_vec(), made_from: None, stamp: None, sources: Vec::new(), paths: Vec::new(), pending: false, named: None };
        assert_eq!(r("makeindex", b"o/a b.idx").output().unwrap(), b"o/a b.ind");
        assert_eq!(r("bibtex", b"o/a.aux").output().unwrap(), b"o/a.bbl");
        assert_eq!(r("biber", b"a.bcf").output().unwrap(), b"a.bbl");
        assert!(r("glossaries", b"a.glo").output().is_none());
        assert!(r("bibtex", b"a.bcf").output().is_none());
    }

    #[test]
    fn the_job_by_the_first_line() {
        let job = |l: &[u8]| job_on_the_command_line(l);
        assert_eq!(job(b"./a.tex"), Some(b"a".to_vec()));
        assert_eq!(job(b"a"), Some(b"a".to_vec()));
        assert_eq!(job(b"sub/dir/x.y.tex"), Some(b"x.y".to_vec()));
        assert_eq!(job(b"\"my doc.tex\""), Some(b"my doc".to_vec()));
        assert_eq!(job(b"&fmt ./a.tex"), Some(b"a".to_vec()));
        assert_eq!(job(b"a.tex more words"), Some(b"a".to_vec()));
        assert_eq!(job(b"\\input a"), None);
        assert_eq!(job(b"&fmt \\input a"), None);
        assert_eq!(job(b"*a"), None);
        assert_eq!(job(b""), None);
        assert_eq!(job(b"&fmt"), None);
    }

    /// latexmk's `@file_not_found`, each form of it, and what comes close.
    #[test]
    fn what_a_log_says_is_missing() {
        let m = |l: &[u8]| says_missing(l).map(<[u8]>::to_vec);
        assert_eq!(m(b"No file a.toc."), Some(b"a.toc".to_vec()));
        assert_eq!(m(b"No file a.bbl: Run bibtex"), Some(b"a.bbl".to_vec()));
        assert_eq!(m(b"No file a b.ind"), Some(b"a b.ind".to_vec()));
        assert_eq!(m(b"! LaTeX Error: File `x.sty' not found."), Some(b"x.sty".to_vec()));
        assert_eq!(m(b"! I can't find file `y.tex'."), Some(b"y.tex".to_vec()));
        assert_eq!(m(b"./a.tex:3: LaTeX Error: File `z.cls' not found."), Some(b"z.cls".to_vec()));
        assert_eq!(m(b"LaTeX Warning: File `p.png' not found on input line 3."), Some(b"p.png".to_vec()));
        assert_eq!(m(b"Package graphics Warning: File `q.pdf' not found"), Some(b"q.pdf".to_vec()));
        assert_eq!(m(b"Package foo Warning: No file `r.dat'"), Some(b"r.dat".to_vec()));
        assert_eq!(m(b"! Unable to load picture or PDF file 's.png'."), Some(b"s.png".to_vec()));
        assert_eq!(m(b"Error: pdflatex (file t.png): cannot find image file"), Some(b"t.png".to_vec()));
        assert_eq!(m(b"Something: File u.dat not found: "), Some(b"u.dat".to_vec()));
        assert_eq!(m(b"There is no file here."), None);
        assert_eq!(m(b"Package hyperref Info: all is well"), None);
        assert_eq!(m(b""), None);
    }

    #[test]
    fn a_cut_line_is_seen_whole_and_as_it_stands() {
        let long = vec![b'x'; 79];
        let mut log = long.clone();
        log.extend_from_slice(b"\nrest\nother\n");
        let mut lines = Vec::new();
        each_log_line(&log, &mut |l| lines.push(l.to_vec()));
        assert!(lines.contains(&long));
        assert!(lines.contains(&[&long[..], b"rest"].concat()));
        assert!(lines.contains(&b"other".to_vec()));
    }

    #[test]
    fn what_bibtex_reads_of_an_aux() {
        let main = b"\\relax\n\\citation{b}\n\\newlabel{x}{{1}{1}}\n\\bibstyle{plain}\n\\@input{ch.aux}\n\\bibdata{p,q}\n\\bibcite{b}{1}\n".to_vec();
        let ch = b"\\relax\n\\citation{a}\n".to_vec();
        let get = |p: &[u8]| match p {
            b"o/main.aux" => Some(main.clone()),
            b"o/ch.aux" => Some(ch.clone()),
            _ => None,
        };
        let mut l = AuxLines::default();
        walk_aux(b"o/main.aux", Some(b"o"), &get, &mut l, 0);
        assert_eq!(l.lines, b"\\citation{b}\n\\bibstyle{plain}\n\\@input{ch.aux}\n\\citation{a}\n\\bibdata{p,q}\n");
        assert_eq!(l.bibdata, [b"p".to_vec(), b"q".to_vec()]);
        assert_eq!(l.bibstyle.as_deref(), Some(&b"plain"[..]));
    }
}
