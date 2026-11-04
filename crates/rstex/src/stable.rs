//! `--until-stable`: passes until the side files a pass reads stop changing, as latexmk has it, and no latexmk.  DESIGN.md section 7, "The driver".
//!
//! A pass is an ordinary run with the same options, the same files and the same checkpoint key.  The engine notes what it does with files outside the tree (`PassRecord` in
//! `crates/engine/src/host.rs`), and at its very end, with everything on the disk, calls [`after_pass`].  That decides: exit, or this program again in place of this process.  So
//! there is no parent process to pay for, a pass that settles at once costs what a plain run costs, and a signal to the process id reaches the pass that is running.
//!
//! **When there is another pass.**  When a file the pass read before writing it now holds something else, or a file it read and did not write has another stat (something else changed
//! it, such as a save during the pass, or a `\write18` program), or a name it looked for and did not find has been made by the pass.  What a pass reads after writing it is its own doing,
//! and the next pass writes it first as well.  The PDF is no side file: latexmk leaves it out too, for a document that asks its own size.  After `limit` passes it stops, and an error
//! stops it at once.  With no `.aux`, a first pass finds what latexmk leaves there before its first run, and what a one-page document with no references writes, so that such a
//! document needs one pass, and a pass that fails says what it says under latexmk.  If it writes no `.aux`, as a plain TeX document does not, the file does not stay.
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

/// The engine's last call.
fn after_pass(rec: &PassRecord, end: &PassEnd<'_>) -> i32 {
    let Some(job) = end.job else {
        return end.code;
    };
    let passes = rec.number.to_string();
    // A pass that failed leaves what it leaves, and another would find the same.  latexmk stops there too.
    if end.code != 0 {
        say(&[b"passes=", passes.as_bytes(), b" result=error"]);
        return end.code;
    }
    let needs = judge_tools(rec, end, job);
    if !needs.is_empty() {
        for n in &needs {
            say(&[b"tool=", n.tool.as_bytes(), b" reason=", n.reason.as_bytes(), b" file=", &n.file]);
        }
        say(&[b"passes=", passes.as_bytes(), b" result=tool"]);
        return STATUS_TOOL;
    }
    if rec.changed.is_empty() {
        say(&[b"passes=", passes.as_bytes(), b" result=stable"]);
        return 0;
    }
    if rec.number >= rec.limit {
        say(&[b"passes=", passes.as_bytes(), b" result=limit file=", &rec.changed[0]]);
        return STATUS_LIMIT;
    }
    say(&[b"pass=", passes.as_bytes(), b" changed=", rec.changed.len().to_string().as_bytes(), b" file=", &rec.changed[0]]);
    let mut args = vec![std::env::args_os().next().unwrap_or_else(|| "rstex".into()), format!("--until-stable-pass={}", rec.number + 1).into()];
    args.extend(std::env::args_os().skip(1).filter(|a| !platform::path_to_bytes(std::path::Path::new(a)).starts_with(PASS)));
    platform::run_again(&args)
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
        let made_from = rule.before.or(Some(rule.now)).filter(|_| out.is_some());
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
