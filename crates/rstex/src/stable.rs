//! `--until-stable`: passes until the side files a pass reads stop changing, as latexmk has it, and no latexmk.  DESIGN.md section 7, "The driver".
//!
//! A pass is an ordinary run with the same options, the same files and the same checkpoint key.  The engine notes what it does with files outside the tree (`PassRecord` in
//! `crates/engine/src/host.rs`), and at its very end, with everything on the disk, calls [`after_pass`].  That decides: exit, or this program again in place of this process.  So
//! there is no parent process to pay for, a pass that settles at once costs what a plain run costs, and a signal to the process id reaches the pass that is running.
//!
//! **A pass killed.**  An editor kills a running build at each newer save.  So the PDF stays as the last run left it until the new one is whole, and what a side file holds goes to a journal before the pass cuts it: `World::pass_keep` and `World::pdf_made` in `crates/engine/src/host.rs`.  The next run under this flag, before anything can read
//! those files, puts them back, [`look`].  A pass that reaches its end has emptied the journal, so a kill leaves what the last whole pass left: the files of an earlier pass of the same build, if one ended.
//!
//! **When there is another pass.**  When a file the pass read before writing it now holds something else, or a file it read and did not write has another stat (something else changed
//! it, such as a save during the pass, or a `\write18` program), or a name it looked for and did not find has been made by the pass.  What a pass reads after writing it is its own doing,
//! and the next pass writes it first as well.  The PDF is no side file: latexmk leaves it out too, for a document that asks its own size.  After `limit` passes it stops, and an error
//! stops it at once, unless the pass found a side file cut in the middle of a line and has written it whole, which is what a killed pass leaves: latexmk would stop there, and only a build after that one would be right.  With no `.aux`, a first pass finds what latexmk leaves there before its first run, and what a one-page document with no references writes, so that such a
//! document needs one pass, and a pass that fails says what it says under latexmk.  If it writes no `.aux`, as a plain TeX document does not, the file does not stay.  With an output
//! directory the environment is what latexmk makes it, by [`search_paths`], or `\input gen` of a `gen.tex` that a document wrote there would work under one and fail under the other.
//!
//! **Bibliography and index programs.**  rstex does not run them.  It says which one must run and why, in a line of its own on stderr and by status 10, and the caller runs it, or
//! latexmk, and comes back.  A line goes out, and passes stop, when the program's result is stale:
//!
//! - `bibtex`, for a `.bbl` the pass read or looked for, whose `.aux` the pass wrote, with a `\bibdata` in it, all of whose `.bib` files exist.  It reads of the `.aux` files only
//!   `\citation`, `\bibdata`, `\bibstyle` and `\@input`, so the digest is of those lines in that order, and a new `\newlabel` is not a reason, where latexmk takes the whole `.aux`.
//! - `biber`, for a `.bcf` the pass wrote: the digest of the whole file, as for latexmk.
//! - `makeindex`, for a `.idx` the pass wrote and a `.ind` it read or looked for: the digest of the whole `.idx`.  latexmk runs it even with no `.ind` wanted.
//!
//! What the result was made from is kept in `<job>.rstex-tools` beside the outputs, only for a document that has such a program.  A program leaves no trace of what it read, so the record
//! holds the stamp of its output as last seen: if that has moved, the program has run, on what the disk held then, which is what this pass found when it began.  With no record and an output
//! there, nothing says what it came from, and the line goes out once.  latexmk treats that the same, by comparing times when it has no record.
#![cfg_attr(test, allow(dead_code))]

use crate::System;
use rstex_engine::checkpoint::xxh64;
use rstex_engine::host::{self, PassEnd, PassRecord};
use rstex_engine::platform::{self, FileStat};
use rstex_kpse as kpse;

/// The status when a named program must run.
const STATUS_TOOL: i32 = 10;
/// The status when the limit was reached with files still changing.
const STATUS_LIMIT: i32 = 11;
/// latexmk's `$max_repeat`.
const DEFAULT_LIMIT: u32 = 5;

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
/// What this program was called, where the next pass cannot be given that as its `argv[0]`: Windows.  It is in the checkpoint's key and in what pdfTeX's warnings print, so a pass with the full path of the program has another of each.
pub const NAME: &[u8] = b"--until-stable-name=";

/// True for an option of this file.
pub fn is_option(arg: &[u8]) -> bool {
    FLAG.contains(&arg) || LIMIT.iter().any(|p| arg.starts_with(p)) || arg.starts_with(PASS) || arg.starts_with(NAME)
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

/// The job's name, where the command line says it as TeX would come to it: `-jobname`, or the first word of the first line.  What a first line starting with `\\` or `&` does is for TeX to find out.
pub fn job_from_command_line(job_option: Option<&[u8]>, first: Option<&[u8]>) -> Option<Vec<u8>> {
    if let Some(j) = job_option {
        return Some(j.to_vec());
    }
    let first: Vec<u8> = first?.iter().copied().filter(|&c| c != b'"').collect();
    if matches!(first.first(), None | Some(b'\\' | b'&')) {
        return None;
    }
    let base = base_name(&first);
    let name = match base.iter().rposition(|&c| c == b'.') {
        Some(0) | None => base,
        Some(k) => &base[..k],
    };
    (!name.is_empty()).then(|| name.to_vec())
}

/// What a killed pass of this job set aside comes back, before anything can read it.  The checkpoint's and the chain's own look at what they read comes after this, so they find the files of the run that wrote them.
pub fn recover(dir: Option<&[u8]>, job: Option<&[u8]>) {
    if let Some(job) = job {
        host::pass_recover(&host::pass_state_path(dir, job));
    }
}

/// Has the engine keep a record of this pass.  `job` is the name that [`look`] has seen to, and `seen` what it found of the last pass.
pub fn install(world: &mut rstex_engine::host::World, u: UntilStable, job: Option<Vec<u8>>, seen: Option<(u64, u64)>, config: u64, snapshot: bool, literal: bool) {
    if u.on {
        let mut p = PassRecord::new(u.pass, u.limit, after_pass);
        p.looked_at = job;
        p.config = config;
        p.seen = seen;
        p.may_collect = seen.is_some() && !literal;
        // A pass with no checkpoint to come from has all it reads to see, and a run that has one finds that out when it restores it, or else is on another preamble.
        p.collecting = p.may_collect && !snapshot && shortcut_allowed();
        world.pass = Some(p);
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

/// The engine's last call.
fn after_pass(rec: &PassRecord, end: &PassEnd<'_>) -> i32 {
    let Some(job) = end.job else {
        rec.commit_nothing();
        return end.code;
    };
    let passes = rec.number.to_string();
    // A pass that failed leaves what it leaves, and another would find the same.  latexmk stops there too.  Unless it failed on a side file that a pass killed had cut short, in the middle of a line where a
    // buffer ended, which an editor does on each newer save: then it has just written that file whole, and the error may be nothing of the document's.  One more pass says.
    let repair = end.code != 0 && rec.repaired && rec.number < rec.limit;
    if end.code != 0 && !repair {
        rec.commit_nothing();
        say(&[b"passes=", passes.as_bytes(), b" result=error"]);
        return end.code;
    }
    let needs = if repair { Vec::new() } else { judge_tools(rec, end, job) };
    if !needs.is_empty() {
        rec.commit_nothing();
        for n in &needs {
            say(&[b"tool=", n.tool.as_bytes(), b" reason=", n.reason.as_bytes(), b" file=", &n.file]);
        }
        say(&[b"passes=", passes.as_bytes(), b" result=tool"]);
        return STATUS_TOOL;
    }
    if rec.changed.is_empty() {
        settled(rec, end, job);
        say(&[b"passes=", passes.as_bytes(), b" result=stable"]);
        return 0;
    }
    rec.commit_nothing();
    if rec.number >= rec.limit {
        say(&[b"passes=", passes.as_bytes(), b" result=limit file=", &rec.changed[0]]);
        return STATUS_LIMIT;
    }
    say(&[b"pass=", passes.as_bytes(), b" changed=", rec.changed.len().to_string().as_bytes(), b" file=", &rec.changed[0]]);
    let mut old = std::env::args_os();
    let name = old.next().unwrap_or_else(|| "rstex".into());
    platform::run_again(&next_pass(name, old.collect(), rec.number + 1, platform::DOSISH))
}

/// The next pass's arguments.  The number goes first, and an earlier pass's number is there, as is the name, which stays as the first pass had it.
fn next_pass(name: std::ffi::OsString, old: Vec<std::ffi::OsString>, number: u32, no_argv0: bool) -> Vec<std::ffi::OsString> {
    let mut args = vec![name.clone(), format!("--until-stable-pass={number}").into()];
    if no_argv0 {
        let given = old.iter().find(|a| a.as_encoded_bytes().starts_with(NAME)).cloned();
        args.push(given.unwrap_or_else(|| {
            let mut a = std::ffi::OsString::from("--until-stable-name=");
            a.push(&name);
            a
        }));
    }
    args.extend(old.into_iter().filter(|a| !a.as_encoded_bytes().starts_with(PASS) && !a.as_encoded_bytes().starts_with(NAME)));
    args
}

// ---------------------------------------------------------------------------
// A save that changed nothing.
// ---------------------------------------------------------------------------
//
// An editor saves with nothing changed, and only something that knows what a pass reads can tell: a `.bib` found through `BIBINPUTS` is not in any `.fls`.  So a run answers that itself, with no pass and nothing written, if `<job>.rstex-state` has a record of a pass
// that ended stable and all it says still holds:
//
// - the configuration, which is what a checkpoint's name comes from: the program and its stat, the directory, the command line, what `texmf.cnf` said of the rest.  If the whole command line and the whole environment are what they were, and each path the tables of `texmf.cnf` and `ls-R` came from has its stamp,
//   that follows, and the run has no need of kpathsea, which is most of what is left of it to pay for: [`answered_early`];
// - each file read, by its stat, or if that moved or was too young at the time to show a later change, by its content; a file that was not there is still not there;
// - each file the pass left, by its stat, the PDF, the log, the side files and SyncTeX's among them, so a pass killed or a program that has been at them means a pass;
// - each variable asked of the environment or of `texmf.cnf`, each search path and the stat of each `ls-R`;
// - each lookup, which the checkpoint's validation does by running it again, and which here goes by what the directories a search looks in hold, so that a name can come to be found, or found elsewhere, only by a change of those.  What such a list does not
//   settle runs again: a name with `$` or `~` or `..`, one that must exist and was not found, and all of them if a directory holds anything else now, such as an editor's swap file;
// - what bibtex and biber would be given, found as this file finds it, so a `.bib` on `BIBINPUTS` counts.
//
// What it does not see is what latexmk's `All targets up-to-date` does not see: the clock.  A document with `\\today` gets no new date until it changes.  A pass that ran a program or read the terminal has no record, as nothing here can ask them again.  Windows has no record: its stat does not see every change, `platform::STAT_SEES_CONTENT`.
//
// A record is made for a save that changed nothing, so that an edit does not pay for it: an edit costs a line in the state file, a digest of what the pass read.  A pass that finds its digest there has changed nothing since, and writes the record: so the second save with nothing changed makes it, and the third is answered.
// All of this goes off with `RSTEX_NO_SHORTCUT`, for what measures a pass, and with `--no-snapshot` and `RSTEX_NO_SNAPSHOT`, the literal path.

const SEEN_HEAD: &[u8] = b"rstex-seen 1 ";
const RECORD_HEAD: &[u8] = b"rstex-stable 1\n";
/// A list of a directory larger than this is not kept, and a search path that comes to more directories runs every lookup again.
const MAX_LISTED: usize = 1 << 12;
const MAX_DIRS: usize = 256;
/// As a checkpoint has it: a stamp younger than this at the read may not show the next edit.
const RACY_NS: i64 = 2_000_000_000;

fn shortcut_allowed() -> bool {
    platform::STAT_SEES_CONTENT && platform::env_var(b"RSTEX_NO_SHORTCUT").is_none() && platform::env_var(b"RSTEX_NO_SNAPSHOT").is_none()
}

/// What `<job>.rstex-state` holds, once a journal in it has been seen to.
pub enum State {
    Nothing,
    /// The digests of the last stable pass: all it read, and what a restored pass found of that.
    Seen((u64, u64)),
    /// What [`after_pass`] leaves of a pass on what the last pass read.
    Record(Vec<u8>),
}

/// What a killed pass of this job began to write comes back, before anything can read it, and the checkpoint's and the chain's own look at what they read comes after this, so they find the files of the run that wrote them.
pub fn look(dir: Option<&[u8]>, job: Option<&[u8]>) -> State {
    let Some(job) = job else { return State::Nothing };
    let path = host::pass_state_path(dir, job);
    use std::io::Read;
    let Ok(mut f) = std::fs::File::open(platform::path_from_bytes(&path)) else { return State::Nothing };
    // The first line says, and most often it is all that is wanted: what lies after it may be as long as the longest journal.
    let mut first = [0u8; 64];
    let Ok(n) = f.read(&mut first) else { return State::Nothing };
    let first = &first[..n];
    let mut whole = Vec::new();
    if first.starts_with(host::JOURNAL_HEAD) || first.starts_with(RECORD_HEAD) {
        whole.extend_from_slice(first);
        if f.read_to_end(&mut whole).is_err() {
            return State::Nothing;
        }
    }
    let text = if whole.is_empty() { first } else { &whole[..] };
    if host::pass_recover_from(&path, text) {
        return State::Nothing;
    }
    if let Some(rest) = text.strip_prefix(SEEN_HEAD) {
        let word = |k: usize| rest.get(k..k + 16).and_then(|w| std::str::from_utf8(w).ok()).and_then(|w| u64::from_str_radix(w, 16).ok());
        return match (word(0), word(17)) {
            (Some(all), Some(preamble)) => State::Seen((all, preamble)),
            _ => State::Nothing,
        };
    }
    if text.starts_with(RECORD_HEAD) && shortcut_allowed() {
        return State::Record(whole);
    }
    State::Nothing
}

pub fn say_nothing_changed() {
    say(&[b"passes=0 result=stable"]);
}

/// All that a run is before it looks at anything: its program, its directory, all its arguments and all its environment.
fn identity(args: impl Iterator<Item = Vec<u8>>) -> Option<u64> {
    let mut o = Vec::with_capacity(1 << 14);
    for a in args.filter(|a| !a.starts_with(PASS)) {
        put_bytes(&mut o, &a);
    }
    let mut env: Vec<(Vec<u8>, Vec<u8>)> = std::env::vars_os().map(|(k, v)| (platform::path_to_bytes(std::path::Path::new(&k)), platform::path_to_bytes(std::path::Path::new(&v)))).collect();
    env.sort_unstable();
    for (k, v) in &env {
        put_bytes(&mut o, k);
        put_bytes(&mut o, v);
    }
    let cwd = std::env::current_dir().ok()?;
    put_bytes(&mut o, &platform::path_to_bytes(&cwd));
    let exe = platform::path_to_bytes(&std::env::current_exe().ok()?);
    put_bytes(&mut o, &exe);
    put_stat(&mut o, &platform::file_stat(&exe));
    Some(xxh64(&o, 0))
}

/// What the document read, by name and content: a pass with the same digest as the last has changed nothing since.
fn digest_of_reads(rec: &PassRecord) -> u64 {
    let mut d = rec.config;
    for (path, f) in rec.files() {
        if f.read.is_some() && !f.main_output {
            d = host::pass_digest_step(d, path, f.before.unwrap_or(0));
        }
    }
    d
}

/// A pass has ended stable.
fn settled(rec: &PassRecord, end: &PassEnd<'_>, job: &[u8]) {
    let digest = digest_of_reads(rec);
    if rec.collecting && rec.seen.is_some_and(|s| s.0 == digest) {
        if let Some(record) = make_record(rec, end, job) {
            // Cut, as what it holds is checked to its last byte.  That says the pass is over, as the line does.
            let _ = std::fs::write(platform::path_from_bytes(&host::pass_state_path(end.output_directory, job)), record);
            return;
        }
    }
    rec.commit_anyway(format!("rstex-seen 1 {digest:016x} {:016x}\n", rec.preamble_digest).as_bytes());
}

fn put_u64(o: &mut Vec<u8>, v: u64) {
    o.extend_from_slice(&v.to_le_bytes());
}

fn put_bytes(o: &mut Vec<u8>, b: &[u8]) {
    put_u64(o, b.len() as u64);
    o.extend_from_slice(b);
}

fn put_opt(o: &mut Vec<u8>, b: Option<&[u8]>) {
    o.push(b.is_some() as u8);
    if let Some(b) = b {
        put_bytes(o, b);
    }
}

fn put_stat(o: &mut Vec<u8>, s: &Option<FileStat>) {
    o.push(s.is_some() as u8);
    if let Some(s) = s {
        for v in [s.size, s.mtime as u64, s.mtime_nsec as u64, s.change as u64, s.inode, s.regular as u64] {
            put_u64(o, v);
        }
    }
}

struct In<'a> {
    b: &'a [u8],
}

impl<'a> In<'a> {
    fn u64(&mut self) -> Option<u64> {
        let (v, rest) = self.b.split_first_chunk::<8>()?;
        self.b = rest;
        Some(u64::from_le_bytes(*v))
    }
    fn byte(&mut self) -> Option<u8> {
        let (v, rest) = self.b.split_first()?;
        self.b = rest;
        Some(*v)
    }
    fn bytes(&mut self) -> Option<&'a [u8]> {
        let n = usize::try_from(self.u64()?).ok()?;
        let v = self.b.get(..n)?;
        self.b = &self.b[n..];
        Some(v)
    }
    fn opt(&mut self) -> Option<Option<&'a [u8]>> {
        if self.byte()? != 0 { self.bytes().map(Some) } else { Some(None) }
    }
    fn stat(&mut self) -> Option<Option<FileStat>> {
        if self.byte()? == 0 {
            return Some(None);
        }
        Some(Some(FileStat { size: self.u64()?, mtime: self.u64()? as i64, mtime_nsec: self.u64()? as i64, change: self.u64()? as i64, inode: self.u64()?, regular: self.u64()? != 0 }))
    }
}

/// A hash of the names in a directory, which is enough to tell if one has come or gone.  `None` for no such directory, or one too large to be worth it, which then never matches a record: see [`make_record`].
fn listing(dir: &[u8]) -> Option<u64> {
    let rd = std::fs::read_dir(platform::path_from_bytes(dir)).ok()?;
    let mut names: Vec<Vec<u8>> = Vec::new();
    for e in rd.flatten() {
        names.push(platform::path_to_bytes(std::path::Path::new(&e.file_name())));
        if names.len() > MAX_LISTED {
            return None;
        }
    }
    names.sort_unstable();
    let mut joined = Vec::with_capacity(names.iter().map(|n| n.len() + 1).sum());
    for n in &names {
        joined.extend_from_slice(n);
        joined.push(0);
    }
    Some(xxh64(&joined, 0))
}

/// True if a lookup's answer can come only from what the listed directories hold, the search paths and the databases: not for what kpathsea expands, or a way up, which a list of the directory it names would not settle.
fn name_goes_by_listing(name: &[u8]) -> bool {
    !name.iter().any(|c| matches!(c, b'\\' | b'$' | b'~')) && !name.windows(2).any(|w| w == b"..")
}

/// What a search for this name looks at besides the directories of its format: for a name with a directory in it, the same below each of them, or the directory itself.
fn dirs_of_name(name: &[u8], format_dirs: &[Vec<u8>], dirs: &mut Vec<Vec<u8>>) {
    let Some(k) = name.iter().rposition(|&c| c == b'/') else { return };
    let part = &name[..k + 1];
    let mut add = |d: Vec<u8>| {
        if !dirs.contains(&d) {
            dirs.push(d);
        }
    };
    if platform::is_absolute(name) || name.starts_with(b"./") {
        add(part.to_vec());
    } else {
        for d in format_dirs {
            add([&d[..], part].concat());
        }
    }
}

/// `None` if there is something a later look could not say the same of: a stamp too young, with nothing else to go by.
fn make_record(rec: &PassRecord, end: &PassEnd<'_>, job: &[u8]) -> Option<Vec<u8>> {
    use rstex_engine::host::{Observation, ReadKind};
    use std::collections::HashSet;
    let now = rstex_kpse::now_ns() as i64;
    let racy = |s: &FileStat| s.change + RACY_NS >= now;
    let dir = end.output_directory;
    let wrote = |p: &[u8]| rec.file(p).is_some_and(|f| f.written);
    let mut o = Vec::with_capacity(1 << 16);
    put_u64(&mut o, identity(std::env::args_os().map(|a| platform::path_to_bytes(std::path::Path::new(&a))))?);
    put_u64(&mut o, rec.config);
    put_u64(&mut o, now as u64);
    // What a program would be given comes first, for the lists below, and what it reads counts as read.
    SOUGHT.with(|s| s.borrow_mut().clear());
    let rules = tool_rules(rec, end, job);
    let sought = SOUGHT.with(|s| core::mem::take(&mut *s.borrow_mut()));
    // Each file read, and what it held: a read after the pass wrote it is its own, and what it left is below.
    let mut files = Vec::new();
    let mut seen_file: HashSet<&[u8]> = HashSet::new();
    let mut absent: Vec<&[u8]> = Vec::new();
    let mut sizes: Vec<(&[u8], u64)> = Vec::new();
    let mut lookups: Vec<(&[u8], i32, bool, Option<&[u8]>)> = Vec::new();
    let mut seen_lookup: HashSet<(&[u8], i32, bool)> = HashSet::new();
    let mut envs: Vec<(&[u8], Option<&[u8]>, bool)> = Vec::new();
    let mut seen_env: HashSet<(&[u8], bool)> = HashSet::new();
    let mut formats: Vec<i32> = Vec::new();
    for a in &rec.asked {
        match a {
            Observation::Lookup { name, format, must_exist, found } => {
                if seen_lookup.insert((name, *format, *must_exist)) {
                    lookups.push((name, *format, *must_exist, found.as_deref()));
                }
                if !formats.contains(format) {
                    formats.push(*format);
                }
            }
            Observation::Env { name, value, cnf } => {
                if seen_env.insert((name, *cnf)) {
                    envs.push((name, value.as_deref(), *cnf));
                }
            }
            Observation::Read { path, stat, hash, prefix, kind } => {
                if wrote(path) || !seen_file.insert(path) {
                    continue;
                }
                let Some(stat) = stat else {
                    absent.push(path);
                    continue;
                };
                // What cannot be read twice and say the same.
                if !stat.regular {
                    return None;
                }
                // A file open when a checkpoint was taken has the hash of what had been read of it: the pass has it whole.
                let hash = match (prefix, kind) {
                    (None, ReadKind::Project | ReadKind::Tds) => Some(*hash),
                    (None, ReadKind::Format) => None,
                    (Some(_), _) => rec.file(path).and_then(|f| f.before),
                };
                if hash.is_none() && racy(stat) {
                    return None;
                }
                files.push((path.as_slice(), stat.clone(), hash));
            }
            Observation::Stat { path, stat, size_only } => {
                if wrote(path) {
                    continue;
                }
                if *size_only {
                    match stat {
                        Some(s) => sizes.push((path, s.size)),
                        None => absent.push(path),
                    }
                } else if seen_file.insert(path) {
                    match stat {
                        Some(s) if !racy(s) => files.push((path.as_slice(), s.clone(), None)),
                        Some(_) => return None,
                        None => absent.push(path),
                    }
                }
            }
            Observation::Output { .. } => {}
        }
    }
    let mut tool_files = Vec::new();
    for rule in &rules {
        for s in &rule.sources {
            tool_files.push((s.path.clone(), platform::file_stat(&s.path)?, Some(s.hash)));
        }
    }
    put_u64(&mut o, (files.len() + tool_files.len()) as u64);
    for (path, stat, hash) in files.iter().map(|f| (f.0, &f.1, f.2)).chain(tool_files.iter().map(|f| (&f.0[..], &f.1, f.2))) {
        put_bytes(&mut o, path);
        put_stat(&mut o, &Some(stat.clone()));
        o.push(hash.is_some() as u8);
        put_u64(&mut o, hash.unwrap_or(0));
    }
    put_u64(&mut o, sizes.len() as u64);
    for (path, size) in &sizes {
        put_bytes(&mut o, path);
        put_u64(&mut o, *size);
    }
    // What the pass left.
    let mut outs: Vec<Vec<u8>> = rec.files().filter(|(_, f)| f.written).map(|(p, _)| p.to_vec()).collect();
    for ext in [&b".synctex.gz"[..], b".synctex", b".rstex-tools"] {
        outs.push(in_dir(dir, &[job, ext].concat()));
    }
    put_u64(&mut o, outs.len() as u64);
    for path in &outs {
        put_bytes(&mut o, path);
        put_stat(&mut o, &platform::file_stat(path));
    }
    put_u64(&mut o, envs.len() as u64);
    for (name, value, cnf) in &envs {
        put_bytes(&mut o, name);
        put_opt(&mut o, *value);
        o.push(*cnf as u8);
    }
    // The search paths, of what a program is given too.
    for s in &sought {
        if !formats.contains(&s.1) {
            formats.push(s.1);
        }
    }
    put_u64(&mut o, formats.len() as u64);
    for f in &formats {
        put_u64(&mut o, *f as u64);
        put_opt(&mut o, kpse::format_path(*f).as_deref());
    }
    let stamps = kpse::table_stamps();
    put_u64(&mut o, stamps.len() as u64);
    for (p, st) in &stamps {
        put_bytes(&mut o, p);
        for v in st {
            put_u64(&mut o, *v);
        }
    }
    // Where a search looks on the disk.
    let mut dirs: Vec<Vec<u8>> = Vec::new();
    let mut by_format: Vec<(i32, Vec<Vec<u8>>)> = Vec::new();
    for f in formats.iter().copied() {
        if !by_format.iter().any(|b| b.0 == f) {
            let list = kpse::disk_dirs(f);
            for d in &list {
                if !dirs.contains(d) {
                    dirs.push(d.clone());
                }
            }
            by_format.push((f, list));
        }
    }
    let format_dirs = |f: i32| by_format.iter().find(|b| b.0 == f).map_or(&[][..], |b| &b.1[..]);
    for (name, format, _, _) in &lookups {
        dirs_of_name(name, format_dirs(*format), &mut dirs);
    }
    for (name, format, _) in &sought {
        dirs_of_name(name, format_dirs(*format), &mut dirs);
    }
    // A directory goes by its stamp, as kpathsea's kept lists do, if that is old enough to show a change made just after it: a look at it costs a stat, and the fonts of the system on a font path are thousands of names.  Otherwise, as for the directory the pass has just written to, by a list of its names.
    let usable = dirs.len() <= MAX_DIRS;
    o.push(usable as u8);
    put_u64(&mut o, if usable { dirs.len() as u64 } else { 0 });
    if usable {
        for d in &dirs {
            put_bytes(&mut o, d);
            let st = kpse::file_stamp(d);
            let changed = st[2].max(st[3]);
            let settled = st[0] == 0 || changed + kpse::listed_racy_ns(changed) < now as u64;
            for v in st {
                put_u64(&mut o, v);
            }
            o.push(settled as u8);
            let list = if settled { None } else { listing(d) };
            o.push(list.is_some() as u8);
            put_u64(&mut o, list.unwrap_or(0));
        }
    }
    put_u64(&mut o, absent.len() as u64);
    for p in &absent {
        put_bytes(&mut o, p);
    }
    put_u64(&mut o, lookups.len() as u64);
    for (name, format, must_exist, found) in &lookups {
        put_bytes(&mut o, name);
        put_u64(&mut o, *format as u64);
        o.push(*must_exist as u8);
        put_opt(&mut o, *found);
    }
    put_u64(&mut o, sought.len() as u64);
    for (name, format, found) in &sought {
        put_bytes(&mut o, name);
        put_u64(&mut o, *format as u64);
        put_opt(&mut o, found.as_deref());
    }
    let mut whole = RECORD_HEAD.to_vec();
    put_u64(&mut whole, xxh64(&o, 0));
    whole.extend_from_slice(&o);
    Some(whole)
}

/// How a look at a record ended.
pub enum Answer {
    Unchanged,
    Changed,
    /// And lookups have been made, in an order of their own.
    ChangedAfterLookups,
}

/// True if a file reads as nothing: not there, or a directory, or not to be read.
fn reads_as_nothing(path: &[u8]) -> bool {
    let p = platform::path_from_bytes(path);
    match std::fs::metadata(&p) {
        Err(_) => true,
        Ok(m) => m.is_dir() || std::fs::File::open(&p).is_err(),
    }
}

/// With what a run has before it starts anything: the whole command line and environment against the record's, and no kpathsea.  False if that does not settle it, and then the rest of the run looks by [`unchanged`].
pub fn answered_early(record: &[u8], args: &[Vec<u8>], dir: Option<&[u8]>) -> bool {
    let Some(id) = identity(args.iter().cloned()) else { return false };
    let mut asked = false;
    record_holds(record, Look::Early(id), dir, &mut asked) == Some(true)
}

/// All that the record says still holds, in the order that sees an edit first.
pub fn unchanged(record: &[u8], config: &[u8], dir: Option<&[u8]>) -> Answer {
    let mut asked = false;
    match record_holds(record, Look::Full(config), dir, &mut asked) {
        Some(true) => Answer::Unchanged,
        _ if asked => Answer::ChangedAfterLookups,
        _ => Answer::Changed,
    }
}

#[derive(Clone, Copy)]
enum Look<'a> {
    /// The hash of the run's identity: nothing here may ask kpathsea.
    Early(u64),
    /// What the checkpoint's key is made from.
    Full(&'a [u8]),
}

fn record_holds(record: &[u8], look: Look<'_>, dir: Option<&[u8]>, asked: &mut bool) -> Option<bool> {
    let body = record.strip_prefix(RECORD_HEAD)?;
    let (sum, body) = body.split_first_chunk::<8>()?;
    if u64::from_le_bytes(*sum) != xxh64(body, 0) {
        return None;
    }
    let mut r = In { b: body };
    let (id, config) = (r.u64()?, r.u64()?);
    let early = match look {
        Look::Early(now) if now != id => return Some(false),
        Look::Early(_) => true,
        Look::Full(c) if xxh64(c, 0) != config => return Some(false),
        Look::Full(_) => false,
    };
    let written = r.u64()? as i64;
    for _ in 0..r.u64()? {
        let path = r.bytes()?;
        let stat = r.stat()??;
        let has_hash = r.byte()? != 0;
        let hash = r.u64()?;
        let now = platform::file_stat(path);
        let same = now.as_ref() == Some(&stat) && stat.change + RACY_NS < written;
        if !same {
            // A stamp too young at the time, or a save of the same bytes.
            if !has_hash || std::fs::read(platform::path_from_bytes(path)).ok().map(|d| xxh64(&d, 0)) != Some(hash) {
                return Some(false);
            }
        }
    }
    for _ in 0..r.u64()? {
        let path = r.bytes()?;
        let size = r.u64()?;
        if platform::file_stat(path).map(|s| s.size) != Some(size) {
            return Some(false);
        }
    }
    for _ in 0..r.u64()? {
        let path = r.bytes()?;
        if platform::file_stat(path) != r.stat()? {
            return Some(false);
        }
    }
    for _ in 0..r.u64()? {
        let name = r.bytes()?;
        let value = r.opt()?;
        let cnf = r.byte()? != 0;
        if !early {
            let now = if cnf { kpse::var_value(name) } else { platform::env_var(name) };
            if now.as_deref() != value {
                return Some(false);
            }
        }
    }
    for _ in 0..r.u64()? {
        let f = r.u64()? as i32;
        let path = r.opt()?;
        if !early && kpse::format_path(f).as_deref() != path {
            return Some(false);
        }
    }
    // What the tables came from.  A run that has them has seen to this, by making them again.
    for _ in 0..r.u64()? {
        let path = r.bytes()?;
        let mut st: kpse::Stamp = [0; 5];
        for x in st.iter_mut() {
            *x = r.u64()?;
        }
        if early && kpse::file_stamp(path) != st {
            return Some(false);
        }
    }
    let mut lists_hold = r.byte()? != 0;
    let mut listed: Vec<&[u8]> = Vec::new();
    for _ in 0..r.u64()? {
        let d = r.bytes()?;
        let mut st: kpse::Stamp = [0; 5];
        for x in st.iter_mut() {
            *x = r.u64()?;
        }
        let settled = r.byte()? != 0;
        let has_list = r.byte()? != 0;
        let list = r.u64()?;
        if lists_hold {
            lists_hold = (settled && kpse::file_stamp(d) == st) || (has_list && listing(d) == Some(list));
            listed.push(d);
        }
    }
    for _ in 0..r.u64()? {
        let path = r.bytes()?;
        // A name in a directory whose names are as they were is still not there.
        let settled = lists_hold && path.iter().rposition(|&c| c == b'/').is_some_and(|k| listed.contains(&&path[..k + 1]));
        if !settled && !reads_as_nothing(path) {
            return Some(false);
        }
    }
    for _ in 0..r.u64()? {
        let name = r.bytes()?;
        let format = r.u64()? as i32;
        let must_exist = r.byte()? != 0;
        let found = r.opt()?;
        // `must_exist` takes a search no further than the directories on the disk that are listed here, and a `!!` element it does not take there at all.
        if lists_hold && name_goes_by_listing(name) {
            continue;
        }
        if early {
            return Some(false);
        }
        *asked = true;
        if kpse::find_file(name, format, must_exist).as_deref() != found {
            return Some(false);
        }
    }
    for _ in 0..r.u64()? {
        let name = r.bytes()?;
        let format = r.u64()? as i32;
        let found = r.opt()?;
        if lists_hold && name_goes_by_listing(name) {
            continue;
        }
        if early {
            return Some(false);
        }
        *asked = true;
        if find_source(dir, name, format).as_deref() != found {
            return Some(false);
        }
    }
    Some(r.b.is_empty())
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

thread_local! {
    /// What [`find_source`] has been asked, and what it said on the search path: a `.bib` that was not found then may be there now.
    static SOUGHT: std::cell::RefCell<Vec<(Vec<u8>, i32, Option<Vec<u8>>)>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// A file a program reads, as bibtex or biber would find it: beside the outputs, or on its search path.
fn find_source(dir: Option<&[u8]>, name: &[u8], format: i32) -> Option<Vec<u8>> {
    let there = in_dir(dir, name);
    let found = if platform::file_stat(&there).is_some() { Some(there) } else { kpse::find_file(name, format, false) };
    SOUGHT.with(|s| s.borrow_mut().push((name.to_vec(), format, found.clone())));
    found
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
fn bibtex_rule(rec: &PassRecord, dir: Option<&[u8]>, aux: Vec<u8>, bbl: Vec<u8>) -> Option<Rule> {
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
        // As latexmk's default, $bibtex_use = 1: a bibliography with no .bib file is what came with the document, and bibtex could not make it.
        let path = find_source(dir, &with_ext, kpse::KPSE_BIB_FORMAT)?;
        sources.push(Source::of(&with_ext, path)?);
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
fn biber_rule(rec: &PassRecord, dir: Option<&[u8]>, job: &[u8]) -> Option<Rule> {
    let bcf = in_dir(dir, &[job, b".bcf"].concat());
    let f = rec.file(&bcf).filter(|f| f.written)?;
    let data = read(&bcf)?;
    // A pass that stopped half way: latexmk sets such a file aside, and biber could do nothing with it.
    if !data.trim_ascii_end().ends_with(b"</bcf:controlfile>") {
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
        let path = find_source(dir, name, kpse::KPSE_BIB_FORMAT)?;
        sources.push(Source::of(name, path)?);
    }
    let base = &bcf[..bcf.len() - 4];
    Some(Rule { tool: "biber", out: [base, b".bbl"].concat(), file: bcf.clone(), reason: "bcf-changed", now: xxh64(&data, 0), before: f.before, sources })
}

/// All this pass shows a need of.
fn tool_rules(rec: &PassRecord, end: &PassEnd<'_>, job: &[u8]) -> Vec<Rule> {
    let dir = end.output_directory;
    let mut rules = Vec::new();
    // What the pass asked for, whether it was there or not.
    let mut asked: Vec<Vec<u8>> = rec.files().filter(|(_, f)| f.read.is_some()).map(|(p, _)| p.to_vec()).collect();
    asked.extend(rec.probes.iter().map(|p| in_dir(dir, p)));
    let biber = biber_rule(rec, dir, job);
    let uses_biber = biber.is_some();
    rules.extend(biber);
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
                rules.extend(bibtex_rule(rec, dir, aux, bbl));
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
}

fn hex_or_dash(v: Option<u64>) -> String {
    v.map_or_else(|| "-".to_string(), |v| format!("{v:016x}"))
}

/// The text of the file: a first line that says what it is, a `rule` line for each program with the name of its file last, and a `source` line for each file it reads.
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
        for (name, h) in &r.sources {
            t.extend_from_slice(format!("source {h:016x} ").as_bytes());
            t.extend_from_slice(name);
            t.push(b'\n');
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
            records.push(Record { tool, file: f.next()?.to_vec(), made_from, stamp, sources: Vec::new() });
        } else if let Some(rest) = line.strip_prefix(b"source ") {
            let (h, name) = (rest.get(..16)?, rest.get(17..)?);
            let h = u64::from_str_radix(std::str::from_utf8(h).ok()?, 16).ok()?;
            records.last_mut()?.sources.push((name.to_vec(), h));
        } else {
            return None;
        }
    }
    Some(records)
}

/// The record of a rule after this pass, and why its program must run, if it must.
fn judge(rule: &Rule, old: Option<Record>) -> (Record, Option<&'static str>) {
    let out = platform::file_stat(&rule.out);
    let sources: Vec<(Vec<u8>, u64)> = rule.sources.iter().map(|s| (s.name.clone(), s.hash)).collect();
    let Some(mut r) = old else {
        let reason = if out.is_some() { REASON_NO_RECORD } else { REASON_MISSING };
        // The line goes out once: from here on, what is there counts as made from what TeX has just written, unless a program writes it again.
        let made_from = Some(rule.now).filter(|_| out.is_some());
        return (Record { tool: rule.tool.to_string(), file: rule.file.clone(), made_from, stamp: out, sources }, Some(reason));
    };
    if r.stamp != out {
        // The program has run since the last look, on what TeX had left then, and on a source as it is now if that has not been edited since.
        let edited_after = |s: &Source| match (&out, platform::file_stat(&s.path)) {
            (Some(o), Some(f)) => (f.mtime, f.mtime_nsec) > (o.mtime, o.mtime_nsec),
            _ => false,
        };
        let kept = |s: &Source| r.sources.iter().find(|o| o.0 == s.name).map_or(0, |o| o.1);
        let now_sources = rule.sources.iter().map(|s| (s.name.clone(), if edited_after(s) { kept(s) } else { s.hash })).collect();
        r.sources = now_sources;
        r.made_from = rule.before.or(r.made_from);
        r.stamp = out.clone();
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
    let rules = tool_rules(rec, end, job);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_job_as_the_command_line_says_it() {
        let j = |o: Option<&str>, f: Option<&str>| job_from_command_line(o.map(str::as_bytes), f.map(str::as_bytes)).map(|v| String::from_utf8(v).unwrap());
        assert_eq!(j(None, Some("./a.tex")).as_deref(), Some("a"));
        assert_eq!(j(None, Some("a")).as_deref(), Some("a"));
        assert_eq!(j(None, Some("sub/a.b.tex")).as_deref(), Some("a.b"));
        assert_eq!(j(None, Some("\"my doc.tex\"")).as_deref(), Some("my doc"));
        assert_eq!(j(Some("x"), Some("./a.tex")).as_deref(), Some("x"));
        assert_eq!(j(Some("x"), None).as_deref(), Some("x"));
        assert_eq!(j(None, Some("\\input a")), None);
        assert_eq!(j(None, Some("&fmt a")), None);
        assert_eq!(j(None, None), None);
    }

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
    fn the_next_pass_has_the_first_ones_name() {
        let os = |v: &[&str]| v.iter().map(std::ffi::OsString::from).collect::<Vec<_>>();
        // Where the name can go as argv[0].
        assert_eq!(next_pass("rstex".into(), os(&["--until-stable", "a.tex"]), 2, false), os(&["rstex", "--until-stable-pass=2", "--until-stable", "a.tex"]));
        assert_eq!(next_pass("rstex".into(), os(&["--until-stable-pass=2", "--until-stable", "a.tex"]), 3, false), os(&["rstex", "--until-stable-pass=3", "--until-stable", "a.tex"]));
        // Where it cannot: the second pass is called by its full path, and so is the third, and they say what the first was called.
        assert_eq!(next_pass("rstex".into(), os(&["--until-stable", "a.tex"]), 2, true), os(&["rstex", "--until-stable-pass=2", "--until-stable-name=rstex", "--until-stable", "a.tex"]));
        assert_eq!(
            next_pass("C:/bin/rstex.exe".into(), os(&["--until-stable-pass=2", "--until-stable-name=rstex", "--until-stable", "a.tex"]), 3, true),
            os(&["C:/bin/rstex.exe", "--until-stable-pass=3", "--until-stable-name=rstex", "--until-stable", "a.tex"])
        );
    }

    #[test]
    fn records_read_back() {
        let stat = FileStat { size: 12, mtime: 34, mtime_nsec: 56, change: 78, inode: 90, regular: true };
        let records = vec![
            Record { tool: "bibtex".into(), file: b".build/my doc.aux".to_vec(), made_from: Some(0xabc), stamp: Some(stat), sources: vec![(b"a b.bib".to_vec(), 1), (b"plain.bst".to_vec(), u64::MAX)] },
            Record { tool: "makeindex".into(), file: b"x.idx".to_vec(), made_from: None, stamp: None, sources: Vec::new() },
        ];
        assert_eq!(parse_records(&records_text(&records)).unwrap(), records);
        assert!(parse_records(b"something else\n").is_none());
        assert!(parse_records(b"rstex-tools 1\nrule bibtex\n").is_none());
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
