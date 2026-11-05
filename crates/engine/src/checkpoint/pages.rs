//! Page records, DESIGN.md section 7: state at page boundaries, so that a run can resume in the middle of a document, after an edit further on.
//!
//! A chain lives beside the preamble checkpoint `HASH.ck`, in a directory `HASH.pg`, and hangs from it: its head says which checkpoint it belongs to.  Only a run that restored something
//! writes one.  Each record holds what changed since the record before it: the pages of the image that differ, the observations of the outside world since, how long each output stream was then,
//! and how far each open file had been read, with a hash of what was read since.  Output only grows, so the files of a chain are one copy of each stream, and a record is a set of lengths.
//!
//! To resume: the preamble's log is answered as for any restore, and then the log of the body, in order.  A file that changed does not end things as long as a record has it open and has not read as far
//! as the change: the hash of what each record read says so, and no copy of the file is kept.  The latest record left is the one to use.  Its image is the preamble's plus, for each page, the newest version
//! among the records up to it.  A resumed run goes on writing from there, and until it does the chain is as it was: a record past an edit fails the same test the next time, or holds again if the edit is undone.
//!
//! **What a run compares with.**  There is no copy of the image.  The last version of a page is where the run got it, in the checkpoint if it has not changed since the preamble and otherwise in the chain,
//! and both are looked at where they lie, in the system's cache of the file: mapped, or read in once on Windows.  So a run that reaches no record has spent nothing, and one that does touches few pages of its own.
//! The worker has to have finished with a record before the next compares with it, and the buffer that goes between them says that it has.
//!
//! **Damage.**  Every part of a record has a sum, and each page of the image its own, since a resume reads only the newest of each.  What is read is checked; what fails takes the chain away and the run goes cold.
//!
//! **Who may touch a chain.**  One run at a time: a lock file in the directory, which the system lets go of when a process dies.  A run that does not get it restores the preamble, and writes nothing.
//! A process killed at any moment leaves a chain that is good up to its last whole frame in the index, which is written last; the sums see what a power failure may leave.
//!
//! `RSTEX_NO_PAGES` turns all of it off, for the literal path.  `RSTEX_PAGE_CK=N` asks for a record every N pages, `RSTEX_RESUME=k` for no record past the k-th.  `RSTEX_PAGE_READONLY` resumes and writes nothing.
//! `RSTEX_PAGE_HASH` and `RSTEX_PAGE_VERIFY` are the gate's: the first keeps a sum of the state in each record, and the second compares a resumed run's state with those of the records it did not use.  `RSTEX_PAGE_BOUND` and `RSTEX_PAGE_NO_MAP` let it reach
//! a full chain, and the way a system with no mappings goes.
#![allow(clippy::too_many_arguments)]

use super::*;
use std::collections::HashMap;
use std::sync::mpsc::{Receiver, Sender, channel};

const PAGE: usize = 4096;
const F_INDEX: u32 = 0;
const F_IMAGE: u32 = 1;
const F_PDF: u32 = 2;
const F_TERM: u32 = 3;
const F_RECORDER: u32 = 4;
const F_SYNCTEX: u32 = 5;
const F_STREAM: u32 = 100;
const CHAIN_MAGIC: u64 = 0x3147_5052_5845_5453; // "STEXRPG1"
const LOCK_FILE: &[u8] = b"/lock";
/// How many pages a long document had, from a run that had no chain to say so.
const LENGTH_FILE: &[u8] = b"/length";
/// A record every so many pages at least, and for a long document no more than about so many records: one a page took 1231 MB and 3.8 to 8.8% more instructions on a book of a thousand pages, against 0.7 to 1.2% at this spacing.
const MIN_SPACING: i32 = 4;
const MOST_RECORDS: i32 = 64;
/// What one chain may hold.  It stops growing there and stays good.
const CHAIN_BOUND: u64 = CACHE_BOUND / 4;
/// A chain that grew by this much in a run has the cache looked at when the run ends.
const GROWTH_TO_PRUNE: u64 = 16 << 20;

/// What a run has of this.
#[derive(Default)]
pub struct PageHost {
    /// A page going out is of interest: a record may be taken, or a sum compared.
    pub(crate) active: bool,
    /// `RSTEX_PAGE_CK`: a record every so many pages, or 0 for the policy.
    spacing: i32,
    /// `RSTEX_PAGE_HASH`: the records hold a sum of each section, for `verify`.
    hash: bool,
    /// What the chain may come to, in bytes: [`CHAIN_BOUND`], or `RSTEX_PAGE_BOUND`.
    bound: u64,
    /// The page of the last record, and the last page known of the last run that wrote.
    last_page: i32,
    pages_known: i32,
    /// This run was restored from a record: `main_control` is next, with no `macro_call`.
    pub(crate) resumed: bool,
    /// The page of that record.
    pub(crate) resumed_at: i32,
    /// That record was taken where `\\ignorespaces` had just been fetched, at the end of `\\document`, and `main_control` goes on from there.
    at_fetch: bool,
    /// The body of the macro that the preamble's checkpoint was taken in, while `\\ignorespaces` is armed for a record at the end of it: `pages_arm_document_end`.
    document_body: i32,
    /// What it takes to write, until the first record is due.
    start: Option<Box<WriterStart>>,
    writer: Option<Box<PageWriter>>,
    /// A restored run is checking its state against the records it did not use: `RSTEX_PAGE_VERIFY`.
    verify: Option<Verify>,
    /// Held to the end of the process.
    lock: Option<std::fs::File>,
    /// This run wrote a preamble checkpoint: where to say how long the document is, if it is long.
    length_for: Option<Vec<u8>>,
}

#[derive(Default)]
struct Verify {
    records: Vec<(i32, Vec<u64>)>,
    checked: u32,
    differ: u32,
}

/// What the worker is to append, all of it in `buf`: `(file, start, bytes)`.  The frame that says it is there goes last, and the buffer comes back.
struct Job {
    buf: platform::HugeBuf,
    parts: Vec<(u32, usize, usize)>,
    record: Vec<u8>,
}

/// A chain's place, what a writer compares with, and for a resumed run where it goes on from.
struct WriterStart {
    dir: Vec<u8>,
    cache: Vec<u8>,
    tag: u64,
    obs_base: u64,
    /// `(entry of the log, hash of the whole file)` of what was open at the preamble.
    head_open: Vec<(u64, u64)>,
    ck: std::fs::File,
    /// `(start, end, place in the file)` of each span of the checkpoint's image, by `start`.
    spans: Vec<(usize, usize, u64)>,
    /// The union of what each section has been live for, in bytes.
    high: Vec<(usize, usize)>,
    pdf_sent: u64,
    /// `(slot, entry of the log, how far the last hash went)`.
    hashed_to: Vec<(i32, u64, u64)>,
    /// How much of the log the chain already has: the first record comes well after this.
    log_sent: usize,
    go_on: Option<GoOn>,
}

/// A chain that is kept up to a record, and the files as they stand there.
struct GoOn {
    records: u32,
    index_end: u64,
    image_end: u64,
    term_len: u64,
    pdf_stream_len: u64,
    recorder_len: u64,
    synctex_len: u64,
    /// `(id, length)` of every stream that stays, and the next id.
    stream_lens: Vec<(u32, u64)>,
    next_stream: u32,
    /// `(path, id, length)` of those still open.
    open_streams: Vec<(Vec<u8>, u32, u64)>,
    overlay: Vec<OverPage>,
    /// `(entry of the log, hash of the whole file)`: files that have changed beyond what was read of them, for the next frame.
    rebased: Vec<(u64, u64)>,
}

/// A page of the image as a record has it: where it goes, where it is in the chain, its length, and its sum.
#[derive(Clone, Copy)]
struct OverPage {
    img: usize,
    at: u64,
    n: usize,
    sum: u64,
}

/// The last version of each page of the image.
struct Base {
    ck: platform::FileView,
    spans: Vec<(usize, usize, u64)>,
    span_at: usize,
    /// The image file of the chain, at the same places: mapped there, or read in.
    chain_file: std::fs::File,
    chain: platform::HugeBuf,
    chain_len: u64,
    /// The pages that are in the chain, by the place in the image, and those of the walk under way.
    overlay: Vec<OverPage>,
    overlay_at: usize,
    next: Vec<OverPage>,
}

impl Base {
    /// The chain up to `len` bytes, which the worker has written, can be looked at.
    fn see_chain(&mut self, len: u64) -> bool {
        if len <= self.chain_len {
            return true;
        }
        if len > self.chain.as_slice().len() as u64 {
            return false;
        }
        // The system's pages: what lies before `chain_len` on the first of them is the same bytes again.
        let size = platform::page_size() as u64;
        let (a, b) = (self.chain_len & !(size - 1), len);
        let dst = &mut self.chain.as_mut_slice()[a as usize..b as usize];
        let n = dst.len();
        let map = platform::env_var(b"RSTEX_PAGE_NO_MAP").is_none();
        // Over a page's end only the last page of the file may reach: `map_file_private` wants whole pages, and the rest of the last reads as zero and is never looked at.
        let whole = (n as u64).next_multiple_of(size) as usize;
        let room = self.chain.as_slice().len() - a as usize;
        if map && whole <= room {
            let dst = &mut self.chain.as_mut_slice()[a as usize..a as usize + whole];
            let p = dst.as_mut_ptr();
            if platform::map_file_private(&self.chain_file, a, p, whole) {
                platform::populate_read(p, whole);
                self.chain_len = len;
                return true;
            }
        }
        let dst = &mut self.chain.as_mut_slice()[a as usize..b as usize];
        if !platform::commit_range(dst.as_mut_ptr(), n) || !platform::read_exact_at(&self.chain_file, a, dst) {
            return false;
        }
        self.chain_len = len;
        true
    }

    /// A new walk over the image, upward.
    fn rewind(&mut self) {
        self.span_at = 0;
        self.overlay_at = 0;
        self.next.clear();
    }

    /// The last version of `len` bytes at `at`, a page or the last part of one.  Nothing here has been asked for lower down since `rewind`.  `true` if the chain has it.
    fn version<'a>(&'a mut self, at: usize, len: usize, buf: &'a mut [u8; PAGE]) -> (&'a [u8], bool) {
        // What the walk does not come to stays as it is.
        while self.overlay.get(self.overlay_at).is_some_and(|o| o.img < at) {
            self.next.push(self.overlay[self.overlay_at]);
            self.overlay_at += 1;
        }
        if let Some(o) = self.overlay.get(self.overlay_at) {
            if o.img == at {
                let x = o.at as usize;
                return (&self.chain.as_slice()[x..x + len], true);
            }
        }
        while self.spans.get(self.span_at).is_some_and(|s| s.1 <= at) {
            self.span_at += 1;
        }
        match self.spans.get(self.span_at) {
            Some(&(a, b, f)) if a <= at && at + len <= b => {
                let x = f as usize + (at - a);
                (&self.ck.as_slice()[x..x + len], false)
            }
            Some(&(a, _, _)) if a >= at + len => (&ZERO[..len], false),
            None => (&ZERO[..len], false),
            _ => {
                // Parts of several spans, with nothing in between: a page shared by small arrays.
                buf[..len].fill(0);
                for &(a, b, f) in &self.spans[self.span_at..] {
                    if a >= at + len {
                        break;
                    }
                    let (x, y) = (a.max(at), b.min(at + len));
                    let from = f as usize + (x - a);
                    buf[x - at..y - at].copy_from_slice(&self.ck.as_slice()[from..from + (y - x)]);
                }
                (&buf[..len], false)
            }
        }
    }

    /// What became of the page that `version` was asked for: the same as it was, or a new version in the chain.
    fn settle(&mut self, in_chain: bool, new: Option<OverPage>) {
        if in_chain {
            let o = self.overlay[self.overlay_at];
            self.overlay_at += 1;
            self.next.push(new.unwrap_or(o));
        } else if let Some(o) = new {
            self.next.push(o);
        }
    }

    fn end_of_walk(&mut self) {
        self.next.extend_from_slice(&self.overlay[self.overlay_at..]);
        core::mem::swap(&mut self.overlay, &mut self.next);
    }
}

static ZERO: [u8; PAGE] = [0; PAGE];

/// A place for a record's bytes on their way to the worker.  Only what is touched takes room, and on huge pages it takes few faults.
struct Outgoing {
    buf: platform::HugeBuf,
    len: usize,
    /// Something did not fit, and this record cannot be.
    full: bool,
}

impl Outgoing {
    fn push(&mut self, s: &[u8]) {
        let e = self.len + s.len();
        match self.buf.as_mut_slice().get_mut(self.len..e) {
            Some(d) => {
                d.copy_from_slice(s);
                self.len = e;
            }
            None => self.full = true,
        }
    }
}

struct PageWriter {
    cache: Vec<u8>,
    base: Base,
    /// The buffer, when the worker does not have it.
    buf: Option<platform::HugeBuf>,
    high: Vec<(usize, usize)>,
    log_sent: usize,
    /// `(path, stream, length so far)` of what has an open stream file.
    streams: Vec<(Vec<u8>, u32, u64)>,
    next_stream: u32,
    pdf_sent: u64,
    image_at: u64,
    term_len: u64,
    recorder_len: u64,
    synctex_len: u64,
    records: u32,
    hashed_to: Vec<(i32, u64, u64)>,
    rebased: Vec<(u64, u64)>,
    /// What this chain has come to, and what it held when this run began.
    size: u64,
    size_before: u64,
    tx: Option<Sender<Job>>,
    /// The buffer again, and whether what it held got to the disk.
    back: Receiver<(platform::HugeBuf, bool)>,
    worker: Option<std::thread::JoinHandle<()>>,
    /// Totals, said at the end under `RSTEX_CHECKPOINT_TRACE`.
    pages_changed: u64,
    pages_compared: u64,
    extents: u64,
    per_section: Vec<u64>,
    compared_per_section: Vec<u64>,
    /// `RSTEX_PAGE_DUMP=dir:first:last`: the image at these pages, for `tools/prof/resume_state.py`.
    dump: Option<(Vec<u8>, i32, i32)>,
}

impl PageWriter {
    fn finish(&mut self, trace: bool) {
        self.tx = None;
        if let Some(w) = self.worker.take() {
            let _ = w.join();
        }
        if trace {
            let mut s = format!(
                "rstex: checkpoint: pages: {} records, {} pages of the image compared, {} changed, {} extents, image {} bytes, chain {} bytes\n",
                self.records, self.pages_compared, self.pages_changed, self.extents, self.image_at, self.size
            );
            for (k, n) in self.per_section.iter().enumerate() {
                if *n > 0 || self.compared_per_section[k] > 0 {
                    s.push_str(&format!("rstex: checkpoint: pages: section {} changed {} compared {}\n", POD_SECTIONS[k].name, n, self.compared_per_section[k]));
                }
            }
            platform::write_stderr(s.as_bytes());
        }
        if self.size >= self.size_before + GROWTH_TO_PRUNE || self.size_before == 0 {
            cache_prune(&self.cache, CACHE_BOUND);
        }
    }

    /// The stream of this path, gaining `n` bytes, and its length then.
    fn stream_add(&mut self, path: &[u8], n: u64, closing: bool) -> (u32, u64) {
        let k = match self.streams.iter().position(|s| s.0 == path) {
            Some(k) => k,
            None => {
                self.streams.push((path.to_vec(), self.next_stream, 0));
                self.next_stream += 1;
                self.streams.len() - 1
            }
        };
        self.streams[k].2 += n;
        let r = (self.streams[k].1, self.streams[k].2);
        if closing {
            self.streams.remove(k);
        }
        r
    }
}

impl HostState {
    /// What a writer has left goes to the disk, and what checks this run says how it went.
    pub(crate) fn pages_finish(&mut self) {
        let trace = self.trace;
        if let Some(mut w) = self.page.writer.take() {
            w.finish(trace);
        }
        if let Some(v) = self.page.verify.take() {
            if trace {
                let s = format!("rstex: checkpoint: verify: {} pages checked, {} differ\n", v.checked, v.differ);
                platform::write_stderr(s.as_bytes());
            }
        }
    }

    /// Nothing more of this process's goes to the chain, and another run may want it while this one waits for that one.
    pub(crate) fn page_lock_off(&mut self) {
        self.page.lock = None;
    }
}

pub(super) fn page_dir(key: &Key) -> Vec<u8> {
    let mut d = key.path.clone();
    d.truncate(d.len() - 3);
    d.extend_from_slice(b".pg");
    d
}

fn pages_dump_setting() -> Option<(Vec<u8>, i32, i32)> {
    let v = platform::env_var(b"RSTEX_PAGE_DUMP")?;
    let s = String::from_utf8_lossy(&v).to_string();
    let mut parts = s.rsplitn(3, ':');
    let last = parts.next()?.parse().ok()?;
    let first = parts.next()?.parse().ok()?;
    Some((parts.next()?.as_bytes().to_vec(), first, last))
}

fn file_in(dir: &[u8], id: u32) -> std::path::PathBuf {
    platform::path_from_bytes(&[dir, format!("/f{}", id).as_bytes()].concat())
}

/// The lock of a chain, if no other run has it.  `make` for a run that is about to write; a run that only looks has nothing to look at if there is no lock file.
fn chain_lock(dir: &[u8], make: bool) -> Option<std::fs::File> {
    let path = platform::path_from_bytes(&[dir, LOCK_FILE].concat());
    let f = if make {
        let mut b = std::fs::DirBuilder::new();
        platform::dir_builder_private(&mut b);
        b.recursive(true).create(platform::path_from_bytes(dir)).ok()?;
        rstex_kpse::open_private_to_update(&path)?
    } else {
        rstex_kpse::open_private(&path)?
    };
    f.try_lock().ok()?;
    // An eviction that had it just before may have taken the file away.
    std::fs::metadata(&path).ok()?;
    Some(f)
}

/// All of a chain but its lock, which this run holds, and what is known of the document's length, which a chain written from here on goes by.
fn chain_wipe(dir: &[u8]) {
    let Ok(rd) = std::fs::read_dir(platform::path_from_bytes(dir)) else { return };
    for e in rd.flatten() {
        let name = e.file_name();
        if name.as_encoded_bytes() != &LOCK_FILE[1..] && name.as_encoded_bytes() != &LENGTH_FILE[1..] {
            let _ = std::fs::remove_file(e.path());
        }
    }
}

/// What the first chain of a long document is to go by: with the spacing from the page it has got to, a book of a thousand pages has 151 records and fills its chain at page 930, and from its length, 64.
fn chain_note_length(dir: &[u8], pages: i32) {
    let Some(_lock) = chain_lock(dir, true) else { return };
    let path = platform::path_from_bytes(&[dir, LENGTH_FILE].concat());
    if let Some(mut f) = rstex_kpse::open_private_to_update(&path) {
        let _ = f.write_all(&(pages as u64).to_le_bytes());
    }
}

fn chain_length(dir: &[u8]) -> i32 {
    let mut b = [0u8; 8];
    let path = platform::path_from_bytes(&[dir, LENGTH_FILE].concat());
    if rstex_kpse::open_private(&path).is_some_and(|f| platform::read_exact_at(&f, 0, &mut b)) { u64::from_le_bytes(b) as i32 } else { 0 }
}

/// A chain in use is the newest, for the eviction.  Once a day at most, as for a checkpoint.
fn chain_touch(dir: &[u8]) {
    let p = file_in(dir, F_INDEX);
    let Ok(m) = std::fs::metadata(&p) else { return };
    let now = std::time::SystemTime::now();
    if m.modified().ok().and_then(|t| now.duration_since(t).ok()).is_some_and(|d| d.as_secs() > 86400) {
        if let Ok(f) = std::fs::OpenOptions::new().write(true).open(&p) {
            let _ = f.set_modified(now);
        }
    }
}

/// `(bytes, last used)` of a chain, for `cache_prune`.
pub(super) fn chain_size(dir: &std::path::Path) -> (u64, Option<std::time::SystemTime>) {
    let mut total = 0;
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            total += e.metadata().map_or(0, |m| m.len());
        }
    }
    (total, std::fs::metadata(dir.join("f0")).or_else(|_| std::fs::metadata(dir)).ok().and_then(|m| m.modified().ok()))
}

/// Takes a chain away, unless a run holds it.  True if it has gone.
pub(super) fn chain_remove(dir: &std::path::Path) -> bool {
    let bytes = platform::path_to_bytes(dir);
    if !dir.exists() {
        return true;
    }
    let Some(lock) = chain_lock(&bytes, true) else { return false };
    chain_wipe(&bytes);
    drop(lock);
    std::fs::remove_dir_all(dir).is_ok()
}

/// The worker: each job's parts in the order given, with the sums of what it wrote, and the frame last.  Once something fails nothing more goes in the index, so the chain stays good up to there.
fn writer_thread(dir: Vec<u8>, rx: Receiver<Job>, back: Sender<(platform::HugeBuf, bool)>) {
    let mut files: HashMap<u32, (std::fs::File, u64)> = HashMap::new();
    let mut dead = false;
    while let Ok(job) = rx.recv() {
        let mut table = Vec::with_capacity(1 << 10);
        let mut page_sums = Vec::new();
        for &(id, start, n) in &job.parts {
            if dead {
                break;
            }
            let bytes = &job.buf.as_slice()[start..start + n];
            let (at, ok) = writer_append(&dir, &mut files, id, bytes);
            dead = !ok;
            put_u64(&mut table, id as u64);
            put_u64(&mut table, at);
            put_u64(&mut table, n as u64);
            if id == F_IMAGE {
                put_u64(&mut table, 0);
                page_sums.reserve(n / PAGE * 8 + 8);
                for page in bytes.chunks(PAGE) {
                    put_u64(&mut page_sums, image_sum(page, 0));
                }
            } else {
                put_u64(&mut table, image_sum(bytes, 0));
            }
        }
        if !dead {
            let mut framed = Vec::with_capacity(job.record.len() + table.len() + page_sums.len() + 32);
            put_bytes(&mut framed, &job.record);
            put_bytes(&mut framed, &table);
            put_bytes(&mut framed, &page_sums);
            let s = xxh64(&framed, 0);
            put_u64(&mut framed, s);
            let (_, ok) = writer_append(&dir, &mut files, F_INDEX, &framed);
            dead = !ok;
        }
        let _ = back.send((job.buf, !dead));
    }
}

fn writer_append(dir: &[u8], files: &mut HashMap<u32, (std::fs::File, u64)>, id: u32, bytes: &[u8]) -> (u64, bool) {
    if let std::collections::hash_map::Entry::Vacant(v) = files.entry(id) {
        let Some(mut f) = rstex_kpse::open_private_to_update(&file_in(dir, id)) else { return (0, false) };
        let Ok(end) = f.seek(std::io::SeekFrom::End(0)) else { return (0, false) };
        v.insert((f, end));
    }
    let (f, end) = files.get_mut(&id).expect("just made sure");
    let at = *end;
    if !rstex_kpse::may_write_file_of(at + bytes.len() as u64) || f.write_all(bytes).is_err() {
        return (at, false);
    }
    *end += bytes.len() as u64;
    (at, true)
}

/// The byte ranges to compare: every section as far as it has ever been live, in whole pages.
fn merged_ranges(high: &[(usize, usize)], limit: usize) -> Vec<(usize, usize, usize)> {
    let mut v: Vec<(usize, usize, usize)> = Vec::with_capacity(high.len());
    for (k, &(a, b)) in high.iter().enumerate() {
        if a < b {
            v.push((a & !(PAGE - 1), b.next_multiple_of(PAGE).min(limit), k));
        }
    }
    v.sort();
    let mut out: Vec<(usize, usize, usize)> = Vec::with_capacity(v.len());
    for r in v {
        match out.last_mut() {
            Some(l) if r.0 <= l.1 => {
                if r.1 > l.1 {
                    l.1 = r.1;
                }
            }
            _ => out.push(r),
        }
    }
    out
}

impl Tex {
    /// True if this run has anything to do with page records.
    fn pages_wanted(&self) -> bool {
        platform::env_var(b"RSTEX_NO_PAGES").is_none() && platform::env_var(b"RSTEX_PAGE_CK").is_none_or(|v| v != b"0") && !self.session_on()
    }

    fn pages_live_ranges(&self) -> Vec<(usize, usize)> {
        POD_SECTIONS
            .iter()
            .map(|s| {
                let (a, b) = if ck_section(s) { (s.live)(self) } else { (0, 0) };
                (s.offset + a * s.elem_size, s.offset + b.max(a) * s.elem_size)
            })
            .collect()
    }

    /// What a record holds of each section: as far as it is live, and for the stacks as far as they have ever been in this run, since a word up there is still what it was when a resumed run comes to it.
    fn pages_ranges(&self) -> Vec<(usize, usize)> {
        POD_SECTIONS
            .iter()
            .map(|s| {
                let (a, b) = if ck_section(s) { (s.live)(self) } else { (0, 0) };
                let peak = match s.name {
                    "save_stack" => self.max_save_stack + 8,
                    "input_stack" => self.max_in_stack + 1,
                    "nest" => self.max_nest_stack + 1,
                    "param_stack" => self.max_param_stack + 1,
                    "buffer" => self.max_buf_stack + 1,
                    "pdf_os_buf" => self.pdf_os_buf_size,
                    _ => 0,
                } as usize;
                (s.offset + a * s.elem_size, s.offset + b.max(a).max(peak.min(s.cap)) * s.elem_size)
            })
            .collect()
    }

    /// A sum of each section, as far as it is live.  The fields that a run has from the clock, and the one that says it was restored, count as they do in a cold run.
    fn pages_state_sums(&mut self) -> Vec<u64> {
        let resumed = self.v_image.ck.resumed;
        self.v_image.ck.resumed = false;
        let saved = self.pages_mask_volatile();
        let v = self.pages_live_ranges().iter().map(|&(a, b)| image_sum(image_bytes(self, a, b - a), 0)).collect();
        self.pages_unmask_volatile(saved);
        self.v_image.ck.resumed = resumed;
        v
    }

    fn pages_mask_volatile(&mut self) -> (i32, i32, i32, i32, A<i32, 55>) {
        let s = (self.microseconds, self.random_seed, self.epochseconds, self.j_random, self.randoms);
        self.microseconds = 0;
        self.random_seed = 0;
        self.epochseconds = 0;
        self.j_random = 0;
        self.randoms = A([0; 55]);
        s
    }

    fn pages_unmask_volatile(&mut self, s: (i32, i32, i32, i32, A<i32, 55>)) {
        self.microseconds = s.0;
        self.random_seed = s.1;
        self.epochseconds = s.2;
        self.j_random = s.3;
        self.randoms = s.4;
    }

    /// `(start, end, place in the file)` of the checkpoint's spans, as `checkpoint_read_image` goes through them, by `start`.
    pub(super) fn pages_spans(meta: &Meta, image_at: u64) -> Vec<(usize, usize, u64)> {
        let mut at = image_at;
        let mut v = Vec::with_capacity(meta.spans.len());
        for &(a, b) in &meta.spans {
            let n = (b - a) as usize;
            at += crate::image::image_padding(at, a as usize, n);
            v.push((a as usize, b as usize, at));
            at += n as u64;
        }
        v.sort();
        v
    }

    /// The run goes on recording from where it was restored, as the run that wrote the checkpoint did before it.
    fn pages_recording_on(&mut self, log: Vec<Observation>, in_obs: Vec<(i32, usize)>, written: Vec<Vec<u8>>, open: Vec<Vec<u8>>) {
        let recorder = self.host.world.recorder_enabled;
        let w = &mut self.host.world;
        w.recording = true;
        w.log = log;
        w.in_obs = vec![usize::MAX; self.host.files.len()];
        for (slot, i) in in_obs {
            w.in_obs[slot as usize] = i;
        }
        w.written = written;
        w.open_outputs = open;
        w.closed_outputs = Vec::new();
        w.journal_recorder = recorder;
        w.recorder_journal_on();
        self.host.journal_outputs = true;
        self.synctex_journal_on();
        self.host.checkpoint.term_journal = Some(Vec::new());
        for s in self.host.files.iter_mut() {
            if let Slot::Out(o) = s {
                o.journal = Some(Vec::new());
            }
        }
        // What the clock made in the preamble is a read of the clock from here on too.
        let ck = self.v_image.ck;
        for cs in ck.sys_cs.0.iter().copied().chain([ck.seed_cs]) {
            if cs != 0 {
                self.checkpoint_arm_cs(cs);
            }
        }
    }

    /// Why this page cannot be a record, or `None`.  `Err` for the rest of the run, `Ok` for this page.
    fn pages_refusal(&mut self) -> Option<Result<String, String>> {
        if self.host.world.taint == Taint::Hard {
            return Some(Err(self.host.world.taint_reason.to_string()));
        }
        if self.pdf_output() <= 0 {
            return Some(Err("DVI".to_string()));
        }
        // As for a stream below: only this process knows of it, and SyncTeX's file goes when it closes.
        if self.synctex_write_failed() {
            return Some(Err("a write failed".to_string()));
        }
        // The prefix made from the job id, which comes from the clock, is in the pages' bytes.
        if self.pdf_unique_resname() > 0 {
            return Some(Err("\\pdfuniqueresname".to_string()));
        }
        if self.v_image.cd.active {
            return Some(Ok("in the middle of a native diagram".to_string()));
        }
        // All of what an error leaves is POD: `history`, `error_count`, what it printed.  Only one that needs the terminal cannot be gone on from, and a read of the terminal is a hard taint.
        if self.help_ptr != 0 || self.interrupt != 0 {
            return Some(Ok("in the middle of an error".to_string()));
        }
        if self.ext_host_state_open() {
            return Some(Ok("a PDF or JBIG2 file is open".to_string()));
        }
        if let Some(why) = self.checkpoint_clock_refusal() {
            return Some(Ok(why.to_string()));
        }
        for (k, s) in self.host.files.iter().enumerate() {
            let k = k as i32;
            let fine = match s {
                Slot::Closed => true,
                // A file the run wrote is the journal's by the time a record comes back to it, and which of its versions was read is not kept.
                Slot::In(f) => {
                    ((READ_FILE_BASE..WRITE_FILE_BASE).contains(&k) || k >= INPUT_FILE_BASE)
                        && self.host.world.in_obs[k as usize] != usize::MAX
                        && !self.host.world.checkpoint_is_written(&f.path)
                }
                Slot::Fmt(_) => false,
                Slot::Out(o) => {
                    // A write that failed is said when the stream closes, and only this process knows of it.
                    if o.has_failed_already() {
                        return Some(Err("a write failed".to_string()));
                    }
                    (k == LOG_FILE || (WRITE_FILE_BASE..INPUT_FILE_BASE).contains(&k)) && o.journal.is_some() && !o.is_pipe()
                }
                Slot::Pdf(_) => k == PDF_FILE && !self.pdfout_in_stream(),
            };
            if !fine {
                return Some(Ok(format!("file slot {} is open", k)));
            }
        }
        None
    }

    /// The first record is due: the files of the chain, and what to compare with.
    fn pages_start_writer(&mut self, s: Box<WriterStart>) -> Option<Box<PageWriter>> {
        let s = *s;
        if self.host.checkpoint.page.lock.is_none() {
            self.host.checkpoint.page.lock = Some(chain_lock(&s.dir, true)?);
        }
        let (mut image_at, mut term_len, mut recorder_len, mut synctex_len, mut records, mut next_stream, mut size_before) = (0, 0, 0, 0, 0, 0, 0);
        let mut streams = Vec::new();
        let mut rebased = Vec::new();
        let mut overlay = Vec::new();
        match s.go_on {
            None => {
                chain_wipe(&s.dir);
                let mut head = Vec::new();
                put_u64(&mut head, CHAIN_MAGIC);
                put_u64(&mut head, s.tag);
                put_u64(&mut head, s.obs_base);
                put_u64(&mut head, s.head_open.len() as u64);
                for &(i, h) in &s.head_open {
                    put_u64(&mut head, i);
                    put_u64(&mut head, h);
                }
                let mut framed = Vec::with_capacity(head.len() + 16);
                put_bytes(&mut framed, &head);
                put_u64(&mut framed, xxh64(&head, 0));
                let mut f = rstex_kpse::open_private_to_update(&file_in(&s.dir, F_INDEX))?;
                f.write_all(&framed).ok()?;
            }
            Some(g) => {
                // The index first: a process killed in the middle of this has a chain that ends at the record, and files longer than it says.
                let mut cuts = vec![(F_INDEX, g.index_end), (F_IMAGE, g.image_end), (F_PDF, g.pdf_stream_len), (F_TERM, g.term_len), (F_RECORDER, g.recorder_len), (F_SYNCTEX, g.synctex_len)];
                cuts.extend(g.stream_lens.iter().map(|&(id, n)| (F_STREAM + id, n)));
                for (id, len) in cuts {
                    if let Some(f) = rstex_kpse::open_private_to_update(&file_in(&s.dir, id)) {
                        f.set_len(len).ok()?;
                        size_before += len;
                    }
                }
                if let Ok(rd) = std::fs::read_dir(platform::path_from_bytes(&s.dir)) {
                    for e in rd.flatten() {
                        let name = e.file_name().to_string_lossy().to_string();
                        if name.strip_prefix('f').and_then(|x| x.parse::<u32>().ok()).is_some_and(|id| id >= F_STREAM + g.next_stream) {
                            let _ = std::fs::remove_file(e.path());
                        }
                    }
                }
                image_at = g.image_end;
                term_len = g.term_len;
                recorder_len = g.recorder_len;
                synctex_len = g.synctex_len;
                records = g.records;
                next_stream = g.next_stream;
                streams = g.open_streams;
                rebased = g.rebased;
                overlay = g.overlay;
            }
        }
        let ck_len = s.ck.metadata().ok()?.len();
        let ck = platform::FileView::new(&s.ck, ck_len as usize)?;
        let chain_file = rstex_kpse::open_private_to_update(&file_in(&s.dir, F_IMAGE))?;
        let bound = self.host.checkpoint.page.bound;
        let chain = platform::HugeBuf::new_small(bound as usize + (2 << 20))?;
        let mut base = Base { ck, spans: s.spans, span_at: 0, chain_file, chain, chain_len: 0, overlay, overlay_at: 0, next: Vec::new() };
        if !base.see_chain(image_at) {
            return None;
        }
        let limit = core::mem::offset_of!(Tex, host);
        // What the compare is about to read, in one go, so that it is no fault each.
        for &(a, b, _) in &merged_ranges(&s.high, limit) {
            let r = image_bytes(self, a, b - a);
            platform::populate_read(r.as_ptr().cast_mut(), r.len());
        }
        let buf = platform::HugeBuf::new_small(bound as usize)?;
        platform::advise_huge(buf.as_slice().as_ptr().cast_mut(), buf.as_slice().len());
        let (tx, rx) = channel();
        let (back_tx, back) = channel();
        let d = s.dir;
        let worker = Some(std::thread::Builder::new().stack_size(1 << 18).spawn(move || writer_thread(d, rx, back_tx)).ok()?);
        Some(Box::new(PageWriter {
            cache: s.cache,
            base,
            buf: Some(buf),
            high: s.high,
            log_sent: s.log_sent,
            streams,
            next_stream,
            pdf_sent: s.pdf_sent,
            image_at,
            term_len,
            recorder_len,
            synctex_len,
            records,
            hashed_to: s.hashed_to,
            rebased,
            size: size_before,
            size_before,
            tx: Some(tx),
            back,
            worker,
            pages_changed: 0,
            pages_compared: 0,
            extents: 0,
            per_section: vec![0; POD_SECTIONS.len()],
            compared_per_section: vec![0; POD_SECTIONS.len()],
            dump: pages_dump_setting(),
        }))
    }

    /// Stops writing for the rest of the run: what is in the chain stays good.
    fn pages_stop(&mut self, why: &str) {
        let m = format!("pages: stop at page {}: {}", self.total_pages, why);
        self.checkpoint_say(&m);
        self.host.checkpoint.page.start = None;
        self.host.checkpoint.page.active = false;
        self.host.checkpoint.pages_finish();
        self.checkpoint_stop();
    }

    /// The big switch, just after a page went out, where a restore can come back to: the record.
    #[cold]
    #[inline(never)]
    pub fn pages_take(&mut self) {
        let _p = crate::prof_scope!("page checkpoint");
        self.v_image.ck.page_due = false;
        if self.host.checkpoint.page.verify.is_some() {
            self.pages_verify();
            return;
        }
        // A first page before `\\document` has ended: a hook of its own that ships one.  Too much has happened for that record to be what it is for.
        if self.host.checkpoint.page.document_body != 0 {
            self.host.checkpoint.page.document_body = 0;
            self.checkpoint_disarm_cs(self.v_image.ck.ignore_cs);
        }
        let page = &self.host.checkpoint.page;
        let every = if page.spacing > 0 { page.spacing } else { MIN_SPACING.max(self.total_pages.max(page.pages_known) / MOST_RECORDS) };
        if self.total_pages < page.last_page + every {
            return;
        }
        self.pages_record(false);
    }

    /// A run that will write a chain, restored at `\\@input` in the body of `\\document`: its end is worth a record of its own, which holds as long as the `.aux` stands and needs no first page.  It is `\\ignorespaces`, the last thing
    /// there as LaTeX has it, so that is armed: this and hooks fetch it a few times, and the token machine pays nothing.  A class that wraps `\\document` is still in that body, and one that takes the word away only has no such record.
    fn pages_arm_document_end(&mut self) {
        if self.state() != TOKEN_LIST || self.token_type() != MACRO || platform::env_var(b"RSTEX_PAGE_NO_DOCUMENT_END").is_some() {
            return;
        }
        let Some(cs) = self.checkpoint_find_cs(b"ignorespaces") else { return };
        if self.eq_type(cs) != IGNORE_SPACES {
            return;
        }
        self.host.checkpoint.page.document_body = self.start();
        self.v_image.ck.ignore_cs = cs;
        self.checkpoint_arm_cs(cs);
    }

    /// `check_outer_validity` met that word, with `cur_cmd` put right.  Only `get_next`'s return and `get_x_token`'s are left of this fetch, and they do nothing but what `pages_fetch_done` does: a place to come back to, like the fetch of
    /// `\\@input` a preamble's checkpoint is taken at.
    pub(super) fn pages_ignorespaces_fetched(&mut self) {
        let body = self.host.checkpoint.page.document_body;
        if body == 0 {
            // A copy that was on the save stack, in this run or one before it.
            self.set_eq_type(self.cur_cs, self.cur_cmd);
            return;
        }
        if self.v_image.ck.at_big_switch && self.expand_depth_count == 0 && self.scanner_status == NORMAL && self.cur_cmd == IGNORE_SPACES && self.state() == TOKEN_LIST && self.token_type() == MACRO && self.start() == body {
            self.host.checkpoint.page.document_body = 0;
            self.set_eq_type(self.cur_cs, self.cur_cmd);
            let _p = crate::prof_scope!("page checkpoint");
            self.pages_record(true);
        }
    }

    /// `main_control` is about to start: true if what it has to do is what comes after that fetch.
    pub(crate) fn pages_fetch_done(&mut self) -> bool {
        if !self.host.checkpoint.page.at_fetch {
            return false;
        }
        self.host.checkpoint.page.at_fetch = false;
        self.cur_tok = CS_TOKEN_FLAG + self.cur_cs;
        self.v_image.ck.at_big_switch = false;
        true
    }

    /// A record, if nothing stands against one here.
    fn pages_record(&mut self, at_fetch: bool) {
        match self.pages_refusal() {
            Some(Err(why)) => {
                self.pages_stop(&why);
                return;
            }
            Some(Ok(why)) => {
                let m = format!("pages: skip page {}: {}", self.total_pages, why);
                self.checkpoint_say(&m);
                return;
            }
            None => {}
        }
        let mut w = match self.host.checkpoint.page.writer.take() {
            Some(w) => w,
            None => {
                let Some(s) = self.host.checkpoint.page.start.take() else { return };
                match self.pages_start_writer(s) {
                    Some(w) => w,
                    None => {
                        self.pages_stop("the chain cannot be written");
                        return;
                    }
                }
            }
        };
        // The worker is done with the last record, which can be looked at from now on, if it got to the disk.  With no room left there, on a full disk or under a limit on a file's size, what that record's
        // pages should be read from is not there, and a mapping of it would end the run with SIGBUS, for a document that has nothing wrong with it.
        let buf = match w.buf.take() {
            Some(b) => b,
            None => match w.back.recv() {
                Ok((b, true)) => b,
                _ => {
                    self.host.checkpoint.page.writer = Some(w);
                    self.pages_stop("the chain cannot be written");
                    return;
                }
            },
        };
        let (true, true) = (w.size < self.host.checkpoint.page.bound, w.base.see_chain(w.image_at)) else {
            self.host.checkpoint.page.writer = Some(w);
            self.pages_stop("the chain is full");
            return;
        };
        let mut out = Outgoing { buf, len: 0, full: false };
        self.host.checkpoint.page.last_page = self.total_pages;
        self.pdfout_settle();
        self.host.term_flush();
        self.host.world.recorder_flush_pending();
        let mut parts: Vec<(u32, usize, usize)> = Vec::with_capacity(8);
        let mut r = Vec::with_capacity(1 << 12);
        put_u64(&mut r, self.total_pages as u64);
        put_u64(&mut r, at_fetch as u64);
        // What was observed since.
        let log_len = self.host.world.log.len();
        put_u64(&mut r, log_len as u64);
        put_u64(&mut r, (log_len - w.log_sent) as u64);
        for o in &self.host.world.log[w.log_sent..] {
            o.encode(&mut r);
        }
        w.log_sent = log_len;
        put_u64(&mut r, if self.v_image.ck.date_read { (self.sys_year * 10000 + self.sys_month * 100 + self.sys_day) as u64 } else { 0 });
        // The open files, how far each has been read, and a hash of what was read since the last record.
        let mut count = 0;
        let mut inputs = Vec::with_capacity(1 << 8);
        for k in 0..self.host.files.len() {
            if let Slot::In(f) = &self.host.files[k] {
                let (slot, obs) = (k as i32, self.host.world.in_obs[k] as u64);
                let from = match w.hashed_to.iter_mut().find(|h| h.0 == slot && h.1 == obs) {
                    Some(h) => core::mem::replace(&mut h.2, f.pos as u64),
                    None => {
                        w.hashed_to.push((slot, obs, f.pos as u64));
                        0
                    }
                };
                put_u64(&mut inputs, k as u64);
                put_bytes(&mut inputs, &f.path);
                put_u64(&mut inputs, f.pos as u64);
                put_u64(&mut inputs, obs);
                put_u64(&mut inputs, xxh64(&f.data[from as usize..f.pos], 0));
                count += 1;
            }
        }
        if self.host.checkpoint.trace && platform::env_var(b"RSTEX_PAGE_INPUTS").is_some() {
            let top = (0..self.host.files.len()).rev().find_map(|k| match &self.host.files[k] {
                Slot::In(f) if k as i32 >= INPUT_FILE_BASE => Some((f.pos, f.path.clone())),
                _ => None,
            });
            if let Some((pos, path)) = top {
                let m = format!("pages: rec {} page {} at {} {}", w.records + 1, self.total_pages, pos, String::from_utf8_lossy(&path));
                self.checkpoint_say(&m);
            }
        }
        put_u64(&mut r, count);
        r.extend_from_slice(&inputs);
        // The streams closed since, whole.
        let closed = core::mem::take(&mut self.host.world.closed_outputs);
        put_u64(&mut r, closed.len() as u64);
        for (path, bytes) in closed {
            let (id, len) = w.stream_add(path_norm(&path), bytes.len() as u64, true);
            put_bytes(&mut r, &path);
            put_u64(&mut r, id as u64);
            put_u64(&mut r, len);
            parts.push((F_STREAM + id, out.len, bytes.len()));
            out.push(&bytes);
        }
        // The open ones.
        let mut outs: Vec<(u64, Vec<u8>, u32, u64, u64)> = Vec::new();
        for k in 0..self.host.files.len() {
            if let Slot::Out(o) = &mut self.host.files[k] {
                let bytes = o.journal.as_mut().expect("the refusal looked");
                let (id, len) = w.stream_add(path_norm(&o.path), bytes.len() as u64, false);
                if !bytes.is_empty() {
                    parts.push((F_STREAM + id, out.len, bytes.len()));
                    out.push(bytes);
                    bytes.clear();
                }
                outs.push((k as u64, o.path.clone(), id, len, o.waiting() as u64));
            }
        }
        put_u64(&mut r, outs.len() as u64);
        for (k, p, id, len, waiting) in &outs {
            put_u64(&mut r, *k);
            put_bytes(&mut r, p);
            put_u64(&mut r, *id as u64);
            put_u64(&mut r, *len);
            put_u64(&mut r, *waiting);
        }
        // The terminal.
        let t = self.host.checkpoint.term_journal.as_mut().expect("while recording");
        w.term_len += t.len() as u64;
        put_u64(&mut r, w.term_len);
        if !t.is_empty() {
            parts.push((F_TERM, out.len, t.len()));
            out.push(t);
            t.clear();
        }
        // The recorder's file.
        if let Some(b) = self.host.world.recorder_drain_journal() {
            w.recorder_len += b.len() as u64;
            if !b.is_empty() {
                parts.push((F_RECORDER, out.len, b.len()));
                out.push(&b);
            }
        }
        put_u64(&mut r, w.recorder_len);
        // SyncTeX's file: the members since the last record, which `pdfout_settle` has just put there, so that it is whole at this length, and what has no member yet.
        let gained = self.synctex_drain_journal();
        if !gained.is_empty() {
            w.synctex_len += gained.len() as u64;
            parts.push((F_SYNCTEX, out.len, gained.len()));
            out.push(&gained);
        }
        put_u64(&mut r, w.synctex_len);
        let (sync_file, sync_root) = self.synctex_for_checkpoint();
        put_opt(&mut r, sync_file.as_ref().map(|f| &f.0[..]));
        put_opt(&mut r, sync_file.as_ref().map(|f| &f.1[..]));
        put_opt(&mut r, sync_root.as_deref());
        // The PDF, what the chunk list gained.
        let mut pdf_path = None;
        if let Slot::Pdf(p) = &self.host.files[PDF_FILE as usize] {
            let total = p.offset() as u64;
            if total > w.pdf_sent {
                parts.push((F_PDF, out.len, (total - w.pdf_sent) as usize));
                p.for_each_from(w.pdf_sent, |b| out.push(b));
            }
            w.pdf_sent = total;
            pdf_path = Some(p.path.clone());
        }
        put_opt(&mut r, pdf_path.as_deref());
        put_u64(&mut r, w.pdf_sent);
        // Files that have changed further on, since the run that read them.
        put_u64(&mut r, w.rebased.len() as u64);
        for &(i, h) in &w.rebased {
            put_u64(&mut r, i);
            put_u64(&mut r, h);
        }
        w.rebased = Vec::new();
        // The image: pages that differ from their last version.
        let limit = core::mem::offset_of!(Tex, host);
        let live = self.pages_ranges();
        for (h, l) in w.high.iter_mut().zip(&live) {
            if l.0 < l.1 {
                *h = if h.0 < h.1 { (h.0.min(l.0), h.1.max(l.1)) } else { *l };
            }
        }
        let mut extents: Vec<(u64, u64, u64)> = Vec::new();
        let trace = self.host.checkpoint.trace;
        let mut page_buf = [0u8; PAGE];
        let image_start = out.len;
        w.base.rewind();
        for &(a, b, _) in &merged_ranges(&w.high, limit) {
            let mut at = a;
            while at < b {
                let e = (at + PAGE).min(b);
                let now = image_bytes(self, at, e - at);
                let (was, in_chain) = w.base.version(at, e - at, &mut page_buf);
                let same = now == was;
                w.pages_compared += 1;
                if trace {
                    if let Some(k) = POD_SECTIONS.iter().rposition(|s| s.offset <= at) {
                        w.compared_per_section[k] += 1;
                    }
                }
                if same {
                    w.base.settle(in_chain, None);
                } else {
                    let place = w.image_at + (out.len - image_start) as u64;
                    w.base.settle(in_chain, Some(OverPage { img: at, at: place, n: e - at, sum: 0 }));
                    match extents.last_mut() {
                        Some(l) if l.0 + l.2 == at as u64 => l.2 += (e - at) as u64,
                        _ => extents.push((at as u64, place, (e - at) as u64)),
                    }
                    out.push(now);
                    w.pages_changed += 1;
                    if trace {
                        if let Some(k) = POD_SECTIONS.iter().rposition(|s| s.offset <= at) {
                            w.per_section[k] += 1;
                        }
                    }
                }
                at = e;
            }
        }
        w.base.end_of_walk();
        let image_n = out.len - image_start;
        parts.push((F_IMAGE, image_start, image_n));
        w.image_at += image_n as u64;
        w.extents += extents.len() as u64;
        put_u64(&mut r, extents.len() as u64);
        for &(x, y, z) in &extents {
            put_u64(&mut r, x);
            put_u64(&mut r, y);
            put_u64(&mut r, z);
        }
        if let Some((dir, first, last)) = &w.dump {
            if (*first..=*last).contains(&self.total_pages) {
                self.pages_dump(dir);
            }
        }
        put_opt(&mut r, self.host.full_name_of_file.as_deref());
        let sums = if self.host.checkpoint.page.hash { self.pages_state_sums() } else { Vec::new() };
        put_u64(&mut r, sums.len() as u64);
        for s in &sums {
            put_u64(&mut r, *s);
        }
        let held = self.native_held();
        put_opt(&mut r, held.as_deref());
        if out.full {
            // Nothing of this record went anywhere, and the streams have lost what they had: no more records.
            self.host.checkpoint.page.writer = Some(w);
            self.pages_stop("a record does not fit");
            return;
        }
        w.size += out.len as u64 + r.len() as u64;
        if let Some(tx) = &w.tx {
            let _ = tx.send(Job { buf: out.buf, parts, record: r });
        }
        w.records += 1;
        self.host.checkpoint.page.writer = Some(w);
    }

    /// The live sections, for `tools/prof/resume_state.py`.
    fn pages_dump(&self, dir: &[u8]) {
        let mut img = Vec::new();
        let mut index = String::new();
        for (k, &(a, b)) in self.pages_live_ranges().iter().enumerate() {
            if a < b {
                index.push_str(&format!("{} {} {} {}\n", POD_SECTIONS[k].name, a, b - a, img.len()));
                img.extend_from_slice(image_bytes(self, a, b - a));
            }
        }
        // Where the fields are that a comparison of two runs may ask about.
        for (name, at) in field_offsets() {
            index.push_str(&format!("field {} {}\n", name, at));
        }
        let base = String::from_utf8_lossy(dir).to_string();
        let _ = std::fs::write(format!("{}/p{}.img", base, self.total_pages), &img);
        let _ = std::fs::write(format!("{}/p{}.idx", base, self.total_pages), index);
    }

    /// A restored run, at a page it did not need: its state against what the writer's was.
    fn pages_verify(&mut self) {
        let page = self.total_pages;
        self.pdfout_settle();
        if let Some((dir, first, last)) = pages_dump_setting() {
            if (first..=last).contains(&page) {
                self.pages_dump(&dir);
            }
        }
        let sums = self.pages_state_sums();
        let mut v = self.host.checkpoint.page.verify.take().expect("the caller looked");
        if let Some((_, want)) = v.records.iter().find(|r| r.0 == page) {
            v.checked += 1;
            if *want != sums {
                v.differ += 1;
                let names: Vec<&str> = (0..sums.len()).filter(|&k| want.get(k) != sums.get(k)).map(|k| POD_SECTIONS[k].name).collect();
                let m = format!("verify: page {} differs in {:?}", page, names);
                self.checkpoint_say(&m);
            }
        }
        self.host.checkpoint.page.verify = Some(v);
    }

    /// A run that comes back past the page where a long document goes to huge pages, and has as much again to do, has not been there: `pdf_ship_out`.
    pub(super) fn pages_long_way_to_go(&self) -> bool {
        let p = &self.host.checkpoint.page;
        p.resumed_at >= 64 && p.pages_known - p.resumed_at >= 64
    }

    /// A preamble checkpoint is on its way, for a key whose chain would be here.
    pub(super) fn pages_preamble_written(&mut self, key: &Key) {
        if self.pages_wanted() {
            self.host.checkpoint.page.length_for = Some(page_dir(key));
        }
    }

    /// `final_cleanup` is next: no later record, so nothing more to keep.
    pub fn pages_end_of_body(&mut self) {
        self.checkpoint_closure_at_end();
        let long = self.total_pages >= MOST_RECORDS * MIN_SPACING;
        let p = &mut self.host.checkpoint.page;
        if let (Some(dir), true) = (p.length_for.take(), long) {
            chain_note_length(&dir, self.total_pages);
        }
        let p = &mut self.host.checkpoint.page;
        if p.active {
            p.active = false;
            let verifying = p.verify.is_some();
            p.start = None;
            self.host.checkpoint.pages_finish();
            if !verifying {
                self.checkpoint_stop();
            }
        }
    }
}

// ------------------------------------------------------------------------------------------------ reading a chain
struct Input {
    slot: i32,
    path: Vec<u8>,
    pos: u64,
    obs: u64,
    /// Of what was read since the last record.
    hash: u64,
}

pub(super) struct Rec {
    index_end: u64,
    page: u64,
    /// Taken at a fetch, and not at the big switch: `pages_ignorespaces_fetched`.
    at_fetch: bool,
    log_len: u64,
    obs: Vec<Observation>,
    date_key: i32,
    inputs: Vec<Input>,
    closed: Vec<(Vec<u8>, u32, u64)>,
    /// `(slot, path, id, length, what waits in the stream's buffer)`.
    outs: Vec<(i32, Vec<u8>, u32, u64, u64)>,
    term_len: u64,
    recorder_len: u64,
    /// How long SyncTeX's file is whole for, and its name, its text so far and the root's name, as a checkpoint's meta part has them.
    synctex_len: u64,
    synctex_name: Option<Vec<u8>>,
    synctex_text: Option<Vec<u8>>,
    synctex_root: Option<Vec<u8>>,
    pdf: Option<Vec<u8>>,
    pdf_len: u64,
    rebased: Vec<(u64, u64)>,
    extents: Vec<(u64, u64, u64)>,
    full_name: Option<Vec<u8>>,
    sums: Vec<u64>,
    /// `(file, place, bytes, sum)`.
    parts: Vec<(u32, u64, u64, u64)>,
    /// One for each page of `extents`, in order.
    page_sums: Vec<u64>,
    /// `native_held`, if it changed since the record before.
    held: Option<Vec<u8>>,
}

fn rec_decode(b: &[u8], table: &[u8], page_sums: &[u8]) -> Option<Rec> {
    let mut r = In { b, p: 0 };
    let page = r.u64()?;
    let at_fetch = r.u64()? != 0;
    let log_len = r.u64()?;
    let n = r.count()?;
    let mut obs = Vec::with_capacity(n);
    for _ in 0..n {
        obs.push(Observation::decode(&mut r)?);
    }
    let date_key = r.i32()?;
    let n = r.count()?;
    let mut inputs = Vec::with_capacity(n);
    for _ in 0..n {
        inputs.push(Input { slot: r.i32()?, path: r.vec()?, pos: r.u64()?, obs: r.u64()?, hash: r.u64()? });
    }
    let n = r.count()?;
    let mut closed = Vec::with_capacity(n);
    for _ in 0..n {
        closed.push((r.vec()?, r.u64()? as u32, r.u64()?));
    }
    let n = r.count()?;
    let mut outs = Vec::with_capacity(n);
    for _ in 0..n {
        outs.push((r.i32()?, r.vec()?, r.u64()? as u32, r.u64()?, r.u64()?));
    }
    let term_len = r.u64()?;
    let recorder_len = r.u64()?;
    let synctex_len = r.u64()?;
    let synctex_name = r.opt()?;
    let synctex_text = r.opt()?;
    let synctex_root = r.opt()?;
    let pdf = r.opt()?;
    let pdf_len = r.u64()?;
    let n = r.count()?;
    let mut rebased = Vec::with_capacity(n);
    for _ in 0..n {
        rebased.push((r.u64()?, r.u64()?));
    }
    let n = r.count()?;
    let mut extents = Vec::with_capacity(n);
    for _ in 0..n {
        extents.push((r.u64()?, r.u64()?, r.u64()?));
    }
    let full_name = r.opt()?;
    let n = r.count()?;
    let mut sums = Vec::with_capacity(n);
    for _ in 0..n {
        sums.push(r.u64()?);
    }
    let held = r.opt()?;
    let mut t = In { b: table, p: 0 };
    let mut parts = Vec::with_capacity(table.len() / 32);
    while t.p < table.len() {
        parts.push((t.u64()? as u32, t.u64()?, t.u64()?, t.u64()?));
    }
    let mut s = In { b: page_sums, p: 0 };
    let mut sum_list = Vec::with_capacity(page_sums.len() / 8);
    while s.p < page_sums.len() {
        sum_list.push(s.u64()?);
    }
    Some(Rec { index_end: 0, page, at_fetch, log_len, obs, date_key, inputs, closed, outs, term_len, recorder_len, synctex_len, synctex_name, synctex_text, synctex_root, pdf, pdf_len, rebased, extents, full_name, sums, parts, page_sums: sum_list, held })
}

/// The sums are against accidents, and what a frame says is checked as a checkpoint's meta part is, before it is used as an index.
fn rec_in_bounds(x: &Rec, before: Option<&Rec>, obs_base: u64) -> bool {
    let limit = core::mem::offset_of!(Tex, host) as u64;
    let slots = 0..(INPUT_FILE_BASE as usize + MAX_IN_OPEN_CAP) as i32;
    x.log_len == before.map_or(obs_base, |b| b.log_len) + x.obs.len() as u64
        && before.is_none_or(|b| x.pdf_len >= b.pdf_len && x.synctex_len >= b.synctex_len)
        && x.inputs.iter().all(|i| slots.contains(&i.slot) && i.obs < x.log_len)
        && x.outs.iter().all(|o| slots.contains(&o.0))
        && x.extents.iter().all(|e| e.0.checked_add(e.2).is_some_and(|end| end <= limit))
}

/// A chain as its index has it, up to the last whole frame.  What a later record says of a file that had changed goes into the entry of the log it is about.
struct Parsed {
    obs_base: u64,
    head_open: Vec<(u64, u64)>,
    recs: Vec<Rec>,
}

fn chain_parse(raw: &[u8], tag: u64) -> Option<Parsed> {
    let mut r = In { b: raw, p: 0 };
    let head = r.bytes()?;
    if r.u64()? != xxh64(head, 0) {
        return None;
    }
    let mut h = In { b: head, p: 0 };
    if h.u64()? != CHAIN_MAGIC || h.u64()? != tag {
        return None;
    }
    let obs_base = h.u64()?;
    let n = h.count()?;
    let mut head_open = Vec::with_capacity(n);
    for _ in 0..n {
        head_open.push((h.u64()?, h.u64()?));
    }
    let mut recs: Vec<Rec> = Vec::new();
    while r.p < raw.len() {
        let start = r.p;
        let Some(b) = r.bytes() else { break };
        let Some(table) = r.bytes() else { break };
        let Some(sums) = r.bytes() else { break };
        let end = r.p;
        let Some(s) = r.u64() else { break };
        if s != xxh64(&raw[start..end], 0) {
            break;
        }
        let Some(mut x) = rec_decode(b, table, sums) else { break };
        if !rec_in_bounds(&x, recs.last(), obs_base) {
            break;
        }
        x.index_end = r.p as u64;
        let rebased = core::mem::take(&mut x.rebased);
        recs.push(x);
        for (i, whole) in rebased {
            if i < obs_base {
                if let Some(e) = head_open.iter_mut().find(|e| e.0 == i) {
                    e.1 = whole;
                }
                continue;
            }
            for rec in recs.iter_mut().rev() {
                let first = rec.log_len - rec.obs.len() as u64;
                if i >= first {
                    if let Some(Observation::Read { hash, kind, .. }) = rec.obs.get_mut((i - first) as usize) {
                        *hash = whole;
                        *kind = ReadKind::Project;
                    }
                    break;
                }
            }
        }
    }
    Some(Parsed { obs_base, head_open, recs })
}

/// A chain, read, and the records that hold.
pub(super) struct Chain {
    dir: Vec<u8>,
    recs: Vec<Rec>,
    obs_base: u64,
    head_open: Vec<(u64, u64)>,
    /// `(slot, the file as it is now)` of what the last record has open.
    kept: Vec<(i32, Vec<u8>)>,
    /// The newest version of each page, by the place in the image.
    overlay: Vec<OverPage>,
    /// `(page, sums)` of the records past the last that holds, for `RSTEX_PAGE_VERIFY`.
    recs_after: Vec<(i32, Vec<u64>)>,
}

/// A file the run wrote and closed: from the preamble, in the checkpoint, or from the chain, with what the checkpoint holds of a stream that was open then.
enum OwnFile<'a> {
    Preamble(usize),
    Stream { prefix: &'a [u8], rec: usize, id: u32, len: u64 },
}

impl OwnFile<'_> {
    fn len(&self, meta: &Meta) -> u64 {
        match self {
            OwnFile::Preamble(j) => meta.closed[*j].1.len() as u64,
            OwnFile::Stream { prefix, len, .. } => prefix.len() as u64 + len,
        }
    }

    fn hash(&self, meta: &Meta, dir: &[u8], recs: &[Rec]) -> Option<u64> {
        match self {
            OwnFile::Preamble(j) => Some(xxh64(&meta.closed[*j].1, 0)),
            OwnFile::Stream { prefix, rec, id, len } => {
                let mut v = prefix.to_vec();
                v.extend_from_slice(&chain_stream(dir, &recs[..=*rec], F_STREAM + *id, *len)?);
                Some(xxh64(&v, 0))
            }
        }
    }
}

/// What came of looking for a chain.
pub(super) enum Found {
    /// None to use.  A chain may be written.
    Nothing,
    /// A chain to resume from.
    Records(Chain),
    /// Asking the log's questions has told kpathsea something, of a part of the log that does not hold, and the recorder would miss it: `checkpoint_try_restore`.
    Cold,
}

/// Would this read get the same answer?  As `checkpoint_read_holds` for a whole file, with the bytes if it took them.
fn read_answer(path: &[u8], stat: &Option<FileStat>, hash: u64, kind: ReadKind) -> (bool, Option<Vec<u8>>) {
    let p = platform::path_from_bytes(path);
    if stat.is_none() {
        return (checkpoint_read_holds(path, stat, hash, None, kind, &mut Vec::new()), None);
    }
    let now = platform::file_stat(path);
    match kind {
        ReadKind::Format => return (now == *stat, None),
        ReadKind::Tds if platform::STAT_SEES_CONTENT && now == *stat => return (true, None),
        _ => {}
    }
    match std::fs::read(&p) {
        Ok(data) => (xxh64(&data, 0) == hash, Some(data)),
        Err(_) => (false, None),
    }
}

/// True if what this record had read of the file is still what the file holds, going on from how far the last record's hash went.
fn read_part_holds(x: &Input, data: &[u8], hashed_to: &mut HashMap<(i32, u64), usize>) -> bool {
    let from = hashed_to.get(&(x.slot, x.obs)).copied().unwrap_or(0);
    let pos = x.pos as usize;
    if pos > data.len() || from > pos || xxh64(&data[from..pos], 0) != x.hash || !prefix_ends_a_line(data, pos) {
        return false;
    }
    hashed_to.insert((x.slot, x.obs), pos);
    true
}

/// A file that has changed since the chain was written: it has to be open at this record, and what the record read of it the same as what it holds now.
fn change_holds(rec: &Rec, i: u64, data: &HashMap<u64, Vec<u8>>, hashed_to: &mut HashMap<(i32, u64), usize>) -> bool {
    let mut open = rec.inputs.iter().filter(|x| x.obs == i).peekable();
    if open.peek().is_none() {
        return false;
    }
    let Some(bytes) = data.get(&i) else { return false };
    open.all(|x| read_part_holds(x, bytes, hashed_to))
}

/// `n` bytes of this file of the chain, if the sums of the records say they are what was written.
fn chain_stream(dir: &[u8], recs: &[Rec], id: u32, n: u64) -> Option<Vec<u8>> {
    if n == 0 {
        return Some(Vec::new());
    }
    let f = rstex_kpse::open_private(&file_in(dir, id))?;
    let mut v = vec![0u8; n as usize];
    if !platform::read_exact_at(&f, 0, &mut v) {
        return None;
    }
    let mut covered = 0;
    for rec in recs {
        for &(file, at, len, sum) in &rec.parts {
            if file == id {
                if at + len > n || image_sum(&v[at as usize..(at + len) as usize], 0) != sum {
                    return None;
                }
                covered += len;
            }
        }
    }
    if covered == n { Some(v) } else { None }
}

impl Tex {
    /// §1517's driver, at a restore that holds: what is asked of this run, and the latest record whose every observation would get the same answer now.
    pub(super) fn pages_look(&mut self, key: &Key, meta: &Meta, tag: u64, open_data: &[(i32, Vec<u8>)]) -> Found {
        if !self.pages_wanted() {
            return Found::Nothing;
        }
        let page = &mut self.host.checkpoint.page;
        page.active = true;
        page.spacing = platform::env_var(b"RSTEX_PAGE_CK").and_then(|v| String::from_utf8_lossy(&v).parse().ok()).unwrap_or(0);
        page.hash = platform::env_var(b"RSTEX_PAGE_HASH").is_some();
        page.bound = platform::env_var(b"RSTEX_PAGE_BOUND").and_then(|v| String::from_utf8_lossy(&v).parse().ok()).unwrap_or(CHAIN_BOUND);
        let most: usize = match platform::env_var(b"RSTEX_RESUME") {
            Some(v) => String::from_utf8_lossy(&v).parse().unwrap_or(usize::MAX),
            None => usize::MAX,
        };
        let dir = page_dir(key);
        let Some(lock) = chain_lock(&dir, false) else { return Found::Nothing };
        self.host.checkpoint.page.lock = Some(lock);
        self.host.checkpoint.page.pages_known = chain_length(&dir);
        let Some(raw) = rstex_kpse::open_private(&file_in(&dir, F_INDEX)).and_then(|mut f| {
            let mut v = Vec::new();
            f.read_to_end(&mut v).ok().map(|_| v)
        }) else {
            return Found::Nothing;
        };
        let Some(parsed) = chain_parse(&raw, tag) else {
            self.checkpoint_say("pages: another preamble's");
            return Found::Nothing;
        };
        drop(raw);
        chain_touch(&dir);
        crate::prof_phase!("pages index read");
        let Parsed { obs_base, head_open, mut recs } = parsed;
        self.host.checkpoint.page.pages_known = recs.last().map_or(0, |r| r.page as i32);
        let limit = recs.len().min(most);
        let today = {
            let (_, d, m, y) = self.date_and_time();
            y * 10000 + m * 100 + d
        };
        let force = platform::env_var(b"RSTEX_RESUME_FORCE").is_some();
        // The bytes of a file, as far as anything has read them, by its entry in the log.
        let mut data: HashMap<u64, Vec<u8>> = HashMap::new();
        let mut changed: Vec<u64> = Vec::new();
        for (slot_data, input) in open_data.iter().zip(&meta.inputs) {
            let Some(i) = pages_read_of_input(&meta.log, input) else { return Found::Nothing };
            let Some(e) = head_open.iter().find(|e| e.0 == i as u64) else { return Found::Nothing };
            if xxh64(&slot_data.1, 0) != e.1 && !changed.contains(&(i as u64)) {
                changed.push(i as u64);
            }
            data.entry(i as u64).or_insert_with(|| slot_data.1.clone());
        }
        // What the run wrote and closed, by the name: a later read of it gets those bytes again, and has to have got them in the first place.  A run beside it, in the same directory, can see to it that it did not.
        let mut own: Vec<(&[u8], OwnFile)> = meta.closed.iter().enumerate().map(|(j, c)| (path_norm(&c.0), OwnFile::Preamble(j))).collect();
        let mut open_at_preamble: Vec<(&[u8], &[u8])> = meta.outputs.iter().map(|o| (path_norm(&o.1), &o.2[..])).collect();
        let mut asked: HashMap<(Vec<u8>, i32, bool), Option<Vec<u8>>> = HashMap::new();
        let mut read: HashMap<(Vec<u8>, u64), bool> = HashMap::new();
        let mut hashed_to: HashMap<(i32, u64), usize> = HashMap::new();
        let mut good = 0;
        let mut late_recording = false;
        // What kpathsea has read until now, its cnf files, is in the `.fls` the checkpoint holds.
        let _ = rstex_kpse::take_recorded();
        'records: for r in 0..limit {
            if !force && recs[r].date_key != 0 && recs[r].date_key != today {
                break;
            }
            let first = recs[r].log_len - recs[r].obs.len() as u64;
            // What is known to have changed comes first, since it asks nothing of anyone: a record past an edit fails with no lookup answered, and a read of the font map for a lookup of a run that will not be made is
            // what `-recorder` has to take back.  An `.aux` that changed fails the first record at once in the same way, at its read, which comes before the fonts that `\\document` looks up.
            let mut holds = changed.iter().all(|&i| change_holds(&recs[r], i, &data, &mut hashed_to));
            let mut closed_at = 0;
            for (k, o) in recs[r].obs.iter().enumerate() {
                if !holds {
                    break;
                }
                match o {
                    Observation::Output { path } => {
                        let n = path_norm(path);
                        let Some(c) = recs[r].closed.get(closed_at).filter(|c| path_norm(&c.0) == n) else { break 'records };
                        closed_at += 1;
                        let prefix = open_at_preamble.iter().position(|o| o.0 == n).map_or(&[][..], |j| open_at_preamble.swap_remove(j).1);
                        own.push((n, OwnFile::Stream { prefix, rec: r, id: c.1, len: c.2 }));
                    }
                    Observation::Lookup { name, format, must_exist, found } => {
                        let now = asked.entry((name.clone(), *format, *must_exist)).or_insert_with(|| rstex_kpse::find_file(name, *format, *must_exist));
                        holds = *now == *found;
                    }
                    Observation::Env { name, value, cnf } => {
                        let now = if *cnf { rstex_kpse::var_value(name) } else { platform::env_var(name) };
                        holds = now == *value;
                    }
                    Observation::Stat { path, stat, size_only } => {
                        let mine = own.iter().rev().find(|e| e.0 == path_norm(path));
                        holds = match mine {
                            Some(e) if *size_only => stat.as_ref().map(|s| s.size) == Some(e.1.len(meta)),
                            _ => {
                                let now = platform::file_stat(path);
                                if *size_only { now.as_ref().map(|s| s.size) == stat.as_ref().map(|s| s.size) } else { now == *stat }
                            }
                        };
                    }
                    Observation::Read { path, stat, hash, kind, .. } => {
                        if let Some(e) = own.iter().rev().find(|e| e.0 == path_norm(path)) {
                            holds = e.1.hash(meta, &dir, &recs) == Some(*hash);
                        } else {
                            let i = first + k as u64;
                            let same = match read.get(&(path.clone(), *hash)) {
                                Some(&v) if v => true,
                                _ => {
                                    let (same, bytes) = read_answer(path, stat, *hash, *kind);
                                    read.insert((path.clone(), *hash), same);
                                    if let Some(b) = bytes {
                                        data.insert(i, b);
                                    }
                                    same
                                }
                            };
                            if !same {
                                changed.push(i);
                                holds = change_holds(&recs[r], i, &data, &mut hashed_to);
                            }
                        }
                    }
                }
            }
            let recorded = rstex_kpse::take_recorded();
            if !holds {
                // Lookups before the place where it failed may have made kpathsea read its font map, which a run from an earlier record has yet to read, and name in its `.fls` where it does.
                late_recording = !recorded.is_empty() && !rstex_kpse::forget_recorded(&recorded);
                break 'records;
            }
            good = r + 1;
        }
        if force {
            good = limit;
        }
        crate::prof_phase!("pages log answered");
        let m = format!("pages: {} records, using {}{}", recs.len(), good, if good > 0 { format!(", page {}", recs[good - 1].page) } else { String::new() });
        self.checkpoint_say(&m);
        if late_recording && self.host.world.recorder_enabled {
            self.checkpoint_say("pages: kpathsea has read something the recorder would not then show");
            chain_wipe(&dir);
            self.host.checkpoint.page.active = false;
            return Found::Cold;
        }
        if good == 0 {
            return Found::Nothing;
        }
        let recs_after: Vec<(i32, Vec<u64>)> = recs[good..].iter().map(|r| (r.page as i32, r.sums.clone())).collect();
        recs.truncate(good);
        // What the last record has open comes back as it is now.
        let mut kept = Vec::new();
        for x in &recs[good - 1].inputs {
            let bytes = match data.get(&x.obs) {
                Some(b) => b.clone(),
                None => match std::fs::read(platform::path_from_bytes(&x.path)) {
                    Ok(b) => b,
                    Err(_) => return Found::Nothing,
                },
            };
            if x.pos as usize > bytes.len() {
                return Found::Nothing;
            }
            kept.push((x.slot, bytes));
        }
        // The newest version of each page.
        let limit = core::mem::offset_of!(Tex, host);
        let mut seen = vec![0u64; limit / PAGE / 64 + 1];
        let mut overlay: Vec<OverPage> = Vec::new();
        for rec in recs.iter().rev() {
            let mut sums = rec.page_sums.iter();
            for &(img, at, n) in &rec.extents {
                let mut x = 0;
                while x < n {
                    let page = ((img + x) as usize) / PAGE;
                    let e = (PAGE as u64).min(n - x);
                    let Some(&sum) = sums.next() else { return Found::Nothing };
                    if seen[page / 64] & 1 << (page % 64) == 0 {
                        seen[page / 64] |= 1 << (page % 64);
                        overlay.push(OverPage { img: (img + x) as usize, at: at + x, n: e as usize, sum });
                    }
                    x += e;
                }
            }
        }
        overlay.sort_by_key(|o| o.img);
        Found::Records(Chain { dir, recs, obs_base, head_open, kept, overlay, recs_after })
    }

    /// The newest version of each page up to the record, over the preamble's image.  Each page's sum is looked at.
    pub(super) fn pages_overlay(&mut self, c: &Chain) -> bool {
        let Some(f) = rstex_kpse::open_private(&file_in(&c.dir, F_IMAGE)) else { return false };
        let (mut reads, mut bytes) = (0u64, 0u64);
        let mut k = 0;
        // One read for each run that is together both in the file and in the image.
        while k < c.overlay.len() {
            let OverPage { img, at, mut n, .. } = c.overlay[k];
            let start = k;
            k += 1;
            while k < c.overlay.len() && c.overlay[k].img == img + n && c.overlay[k].at == at + n as u64 {
                n += c.overlay[k].n;
                k += 1;
            }
            let dst = image_bytes_mut(self, img, n);
            if !platform::commit_range(dst.as_mut_ptr(), n) || !platform::read_exact_at(&f, at, dst) {
                return false;
            }
            reads += 1;
            bytes += n as u64;
            for o in &c.overlay[start..k] {
                if image_sum(image_bytes(self, o.img, o.n), 0) != o.sum {
                    return false;
                }
            }
        }
        let m = format!("pages: overlay {} reads, {} bytes", reads, bytes);
        self.checkpoint_say(&m);
        true
    }

    /// A chain that failed its sums goes, so that the next run does not come here again, and this run goes cold.
    pub(super) fn pages_damaged(&mut self, key: &Key) {
        chain_wipe(&page_dir(key));
        self.checkpoint_say("pages: a damaged chain");
        let p = &mut self.host.checkpoint.page;
        p.active = false;
        p.start = None;
        p.resumed = false;
        p.at_fetch = false;
        p.resumed_at = 0;
        p.verify = None;
    }

    /// A run restored at the preamble, or one whose chain holds nothing: it may make a new chain, on the files as they now are.  Recording goes on from the preamble's log.
    pub(super) fn pages_begin(&mut self, key: &Key, tag: u64, ck: Option<std::fs::File>, spans: Vec<(usize, usize, u64)>, log: Vec<Observation>, inputs: &[(i32, Vec<u8>, u64)], pdf_sent: u64) {
        let page = &mut self.host.checkpoint.page;
        let (true, Some(ck)) = (page.active && platform::env_var(b"RSTEX_PAGE_READONLY").is_none(), ck) else {
            page.active = false;
            return;
        };
        let mut in_obs = Vec::with_capacity(inputs.len());
        let mut head_open = Vec::with_capacity(inputs.len());
        for input in inputs {
            let Some(i) = pages_read_of_input(&log, input) else {
                self.host.checkpoint.page.active = false;
                return;
            };
            in_obs.push((input.0, i));
            if let Slot::In(f) = &self.host.files[input.0 as usize] {
                if !head_open.iter().any(|e: &(u64, u64)| e.0 == i as u64) {
                    head_open.push((i as u64, xxh64(&f.data, 0)));
                }
            }
        }
        let obs_base = log.len() as u64;
        let open: Vec<Vec<u8>> = self.host.files.iter().enumerate().filter_map(|(k, s)| if let Slot::Out(o) = s { if k as i32 != LOG_FILE { Some(o.path.clone()) } else { None } } else { None }).collect();
        let mut written: Vec<Vec<u8>> = log.iter().filter_map(|o| if let Observation::Output { path } = o { Some(path.clone()) } else { None }).collect();
        written.extend(open.iter().cloned());
        self.pages_recording_on(log, in_obs, written, open);
        self.pages_arm_document_end();
        let high = self.pages_ranges();
        self.host.checkpoint.page.start = Some(Box::new(WriterStart {
            dir: page_dir(key),
            cache: key.dir.clone(),
            tag,
            obs_base,
            head_open,
            ck,
            spans,
            high,
            pdf_sent,
            hashed_to: Vec::new(),
            log_sent: obs_base as usize,
            go_on: None,
        }));
    }

    /// As `checkpoint_restore_host`, for a record: the terminal last, since up to there a failure means a cold run, which says it all itself.
    pub(super) fn pages_restore_host(&mut self, key: &Key, tag: u64, mut meta: Meta, now: (i32, i32, i32, i32), mut c: Chain, ck: Option<std::fs::File>, spans: Vec<(usize, usize, u64)>) -> bool {
        let n = c.recs.len();
        let last = n - 1;
        // A pass of `--until-stable` has read and written all that the logs up to this record say, and has to know what those files held before any is made again.
        if self.host.world.pass.is_some() {
            let open: Vec<&[u8]> = c.recs.iter().flat_map(|r| r.closed.iter().map(|x| &x.0[..])).chain(c.recs[last].outs.iter().map(|o| &o.1[..])).chain(meta.recorder.iter().map(|r| &r.0[..])).collect();
            self.host.world.pass_note_restored(meta.log.iter().chain(c.recs.iter().flat_map(|r| r.obs.iter())), &open, c.recs[last].pdf.as_deref());
        }
        // All that comes from the chain, first, and checked: a run that finds something wrong goes cold, and the disk has to be as it was.
        let Some(term) = chain_stream(&c.dir, &c.recs, F_TERM, c.recs[last].term_len) else { return false };
        let mut base: HashMap<Vec<u8>, Vec<u8>> = HashMap::new();
        for (_, path, bytes) in core::mem::take(&mut meta.outputs) {
            base.insert(path_norm(&path).to_vec(), bytes);
        }
        let mut stream_lens: Vec<(u32, u64)> = Vec::new();
        let mut next_stream = 0;
        let mut written: Vec<Vec<u8>> = meta.log.iter().filter_map(|o| if let Observation::Output { path } = o { Some(path.clone()) } else { None }).collect();
        let mut closed_files: Vec<(&[u8], Vec<u8>)> = Vec::new();
        for k in 0..n {
            for (path, id, len) in &c.recs[k].closed {
                let mut whole = base.remove(path_norm(path)).unwrap_or_default();
                let Some(tail) = chain_stream(&c.dir, &c.recs[..=k], F_STREAM + *id, *len) else { return false };
                whole.extend_from_slice(&tail);
                closed_files.push((path, whole));
                stream_lens.push((*id, *len));
                next_stream = next_stream.max(*id + 1);
                written.push(path.clone());
            }
        }
        let (start, end) = (self.v_image.ck.log_date_start as usize, self.v_image.ck.log_date_end as usize);
        let (t, d, m, y) = now;
        let mut open_streams = Vec::new();
        let mut open_paths = Vec::new();
        let mut open_files: Vec<(i32, &[u8], Vec<u8>, usize)> = Vec::new();
        for (slot, path, id, len, waiting) in &c.recs[last].outs {
            let mut bytes = base.remove(path_norm(path)).unwrap_or_default();
            let Some(tail) = chain_stream(&c.dir, &c.recs, F_STREAM + *id, *len) else { return false };
            bytes.extend_from_slice(&tail);
            stream_lens.push((*id, *len));
            next_stream = next_stream.max(*id + 1);
            open_streams.push((path_norm(path).to_vec(), *id, *len));
            if *slot == LOG_FILE {
                if start > end || end > bytes.len() {
                    return false;
                }
                bytes = [&bytes[..start], &log_date(d, m, y, t), &bytes[end..]].concat();
            } else {
                open_paths.push(path.clone());
                written.push(path.clone());
            }
            open_files.push((*slot, path, bytes, *waiting as usize));
        }
        let recorder_len = c.recs[last].recorder_len;
        let recorder = match meta.recorder.take() {
            Some((name, mut bytes)) => {
                let Some(tail) = chain_stream(&c.dir, &c.recs, F_RECORDER, recorder_len) else { return false };
                bytes.extend_from_slice(&tail);
                Some((name, bytes))
            }
            None => None,
        };
        let synctex_len = c.recs[last].synctex_len;
        let Some(synctex_bytes) = chain_stream(&c.dir, &c.recs, F_SYNCTEX, synctex_len) else { return false };
        let preamble_pdf = meta.pdf.as_ref().map_or(0, |p| p.1.len() as u64);
        let Some(pdf_stream_len) = c.recs[last].pdf_len.checked_sub(preamble_pdf) else { return false };
        let Some(pdf_tail) = chain_stream(&c.dir, &c.recs, F_PDF, pdf_stream_len) else { return false };
        if let Some(held) = c.recs[..=last].iter().rev().find_map(|r| r.held.as_deref()) {
            if !self.native_held_restore(held) {
                return false;
            }
        }
        // Then the disk.  What these files hold now is what the job found, and what `native_restart` has to give back to a run from the start: the partial `.aux` of this record would be another document.
        if self.v_image.cd.tikzcd_cs != 0 {
            let others = open_files.iter().filter(|f| f.0 != LOG_FILE).map(|f| f.1);
            for path in meta.closed.iter().map(|c| &c.0[..]).chain(closed_files.iter().map(|c| c.0)).chain(others) {
                self.host.world.keep_output(path);
            }
        }
        for (path, bytes) in &meta.closed {
            if std::fs::write(platform::path_from_bytes(path), bytes).is_err() {
                return false;
            }
        }
        for (path, whole) in &closed_files {
            if std::fs::write(platform::path_from_bytes(path), whole).is_err() {
                return false;
            }
        }
        // The files, each at its place.
        let mut hashed_to = Vec::with_capacity(c.recs[last].inputs.len());
        for ((slot, data), x) in core::mem::take(&mut c.kept).into_iter().zip(&c.recs[last].inputs) {
            hashed_to.push((x.slot, x.obs, x.pos));
            self.host.files[slot as usize] = Slot::In(InFile { path: x.path.clone(), data, pos: x.pos as usize });
        }
        for (slot, path, bytes, waiting) in &open_files {
            let Some(o) = OutFile::restored(path, bytes, *waiting) else { return false };
            self.host.files[*slot as usize] = Slot::Out(o);
        }
        if let Some((name, bytes)) = recorder {
            if !self.host.world.recorder_restore(name, &bytes) {
                return false;
            }
        }
        // SyncTeX's file, cut where the sheets of this record end.  `meta` had it as it stood at the preamble, and this is what became of it.
        let rec = &c.recs[last];
        let sync_file = rec.synctex_name.clone().map(|name| (name, rec.synctex_text.clone().unwrap_or_default()));
        if !self.synctex_restore(sync_file, rec.synctex_root.clone(), &synctex_bytes) {
            return false;
        }
        drop(synctex_bytes);
        let rec = &c.recs[last];
        if let Some(path) = &rec.pdf {
            let head = meta.pdf.take().map(|p| p.1).unwrap_or_default();
            // The last PDF goes now.  What the output cache wants of it is what comes after this record.
            self.host.hv.outcache.capture_resumed(path, rec.pdf_len);
            let Some(mut p) = PdfOut::reopen(path, &head) else { return false };
            p.push_chunk(pdf_tail);
            self.host.files[PDF_FILE as usize] = Slot::Pdf(p);
        }
        self.host.full_name_of_file = rec.full_name.clone();
        self.host.hv.strings = Default::default();
        // As the first page did, in `pdf_ship_out`.
        if self.init_pdf_output {
            let mode = if self.pdf_pk_mode() != NULL {
                let s = self.tokens_to_string(self.pdf_pk_mode());
                let m = self.make_cstring(s);
                self.flush_string();
                m
            } else {
                Vec::new()
            };
            self.kpse_init_prog(b"PDFTEX", self.fixed_pk_resolution, &mode, b"");
            self.kpse_set_program_enabled(rstex_kpse::KPSE_PK_FORMAT, true, crate::pdfship::KPSE_SRC_COMPILE);
        }
        self.host.checkpoint.page.resumed = true;
        self.host.checkpoint.page.at_fetch = rec.at_fetch;
        self.host.checkpoint.page.resumed_at = rec.page as i32;
        self.host.checkpoint.page.last_page = rec.page as i32;
        self.host.world.terminal_write(&meta.term);
        self.host.world.terminal_write(&term);
        if platform::env_var(b"RSTEX_PAGE_VERIFY").is_some() {
            let records = core::mem::take(&mut c.recs_after).into_iter().filter(|r| !r.1.is_empty()).collect();
            self.host.checkpoint.page.verify = Some(Verify { records, ..Default::default() });
            return true;
        }
        let (Some(ck), None) = (ck, platform::env_var(b"RSTEX_PAGE_READONLY")) else {
            self.host.checkpoint.page.active = false;
            return true;
        };
        // It goes on from here.  What has changed in the files it has open, past where it has read them, is for the next frame to say.
        let mut log = core::mem::take(&mut meta.log);
        for r in &c.recs {
            log.extend(r.obs.iter().cloned());
        }
        let mut rebased: Vec<(u64, u64)> = Vec::new();
        for x in &c.recs[last].inputs {
            let Slot::In(f) = &self.host.files[x.slot as usize] else { continue };
            let whole = xxh64(&f.data, 0);
            let known = if x.obs < c.obs_base {
                c.head_open.iter().find(|e| e.0 == x.obs).map(|e| e.1)
            } else if let Some(Observation::Read { hash, .. }) = log.get(x.obs as usize) {
                Some(*hash)
            } else {
                None
            };
            if known != Some(whole) && !rebased.iter().any(|e| e.0 == x.obs) {
                rebased.push((x.obs, whole));
            }
        }
        for &(i, whole) in &rebased {
            if let Some(Observation::Read { hash, kind, prefix: None, .. }) = log.get_mut(i as usize) {
                *hash = whole;
                *kind = ReadKind::Project;
            }
        }
        let rec = &c.recs[last];
        let in_obs: Vec<(i32, usize)> = rec.inputs.iter().map(|x| (x.slot, x.obs as usize)).collect();
        let (pdf_sent, index_end, term_len) = (rec.pdf_len, rec.index_end, rec.term_len);
        let log_sent = log.len();
        self.pages_recording_on(log, in_obs, written, open_paths);
        let image_end = c.recs.iter().flat_map(|r| r.extents.iter()).map(|e| e.1 + e.2).max().unwrap_or(0);
        let high = self.pages_ranges();
        self.host.checkpoint.page.start = Some(Box::new(WriterStart {
            dir: c.dir,
            cache: key.dir.clone(),
            tag,
            obs_base: c.obs_base,
            head_open: Vec::new(),
            ck,
            spans,
            high,
            pdf_sent,
            hashed_to,
            log_sent,
            go_on: Some(GoOn {
                records: n as u32,
                index_end,
                image_end,
                term_len,
                pdf_stream_len,
                recorder_len,
                synctex_len,
                stream_lens,
                next_stream,
                open_streams,
                overlay: c.overlay,
                rebased,
            }),
        }));
        true
    }
}

/// The entry of the log for a file open at the preamble.
fn pages_read_of_input(log: &[Observation], input: &(i32, Vec<u8>, u64)) -> Option<usize> {
    log.iter().position(|o| matches!(o, Observation::Read { path: p, prefix: Some(x), .. } if *p == input.1 && *x == input.2))
}

/// Enough of a map of the prefix to name what differs.
fn field_offsets() -> Vec<(&'static str, usize)> {
    use core::mem::offset_of as o;
    vec![
        ("v_image", o!(Tex, v_image)),
        ("v_pdfship", o!(Tex, v_pdfship)),
        ("v_pdffont", o!(Tex, v_pdffont)),
        ("v_ext", o!(Tex, v_ext)),
        ("v_web2c", o!(Tex, v_web2c)),
        ("cd", o!(Tex, v_image) + o!(crate::image::Vars, cd)),
        ("ck", o!(Tex, v_image) + o!(crate::image::Vars, ck)),
        ("randoms", o!(Tex, randoms)),
        ("pdf_gone", o!(Tex, pdf_gone)),
        ("pdf_ptr", o!(Tex, pdf_ptr)),
        ("pdf_save_offset", o!(Tex, pdf_save_offset)),
        ("pdf_os_ptr", o!(Tex, pdf_os_ptr)),
        ("start_time_str", o!(Tex, v_pdfship) + o!(crate::pdfship::Vars, start_time_str)),
        ("start_time_len", o!(Tex, v_pdfship) + o!(crate::pdfship::Vars, start_time_len)),
        ("sys_obj_ptr", o!(Tex, sys_obj_ptr)),
        ("selector", o!(Tex, selector)),
        ("term_offset", o!(Tex, term_offset)),
        ("file_offset", o!(Tex, file_offset)),
        ("history", o!(Tex, history)),
        ("name_length", o!(Tex, name_length)),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(rec: &[u8], table: &[u8], sums: &[u8]) -> Vec<u8> {
        let mut f = Vec::new();
        put_bytes(&mut f, rec);
        put_bytes(&mut f, table);
        put_bytes(&mut f, sums);
        let s = xxh64(&f, 0);
        put_u64(&mut f, s);
        f
    }

    fn head(tag: u64, obs_base: u64, open: &[(u64, u64)]) -> Vec<u8> {
        let mut h = Vec::new();
        put_u64(&mut h, CHAIN_MAGIC);
        put_u64(&mut h, tag);
        put_u64(&mut h, obs_base);
        put_u64(&mut h, open.len() as u64);
        for &(i, w) in open {
            put_u64(&mut h, i);
            put_u64(&mut h, w);
        }
        let mut f = Vec::new();
        put_bytes(&mut f, &h);
        put_u64(&mut f, xxh64(&h, 0));
        f
    }

    /// A record as `pages_take` lays it out, with one read since the last and these changed files.
    fn record(page: u64, log_len: u64, read: Option<(&[u8], u64)>, rebased: &[(u64, u64)]) -> Vec<u8> {
        let mut r = Vec::new();
        put_u64(&mut r, page);
        put_u64(&mut r, 0); // at the big switch
        put_u64(&mut r, log_len);
        put_u64(&mut r, read.is_some() as u64);
        if let Some((path, hash)) = read {
            Observation::Read { path: path.to_vec(), stat: None, hash, prefix: None, kind: ReadKind::Tds }.encode(&mut r);
        }
        put_u64(&mut r, 0); // date
        put_u64(&mut r, 0); // inputs
        put_u64(&mut r, 0); // closed
        put_u64(&mut r, 0); // outs
        put_u64(&mut r, 7); // terminal
        put_u64(&mut r, 8); // recorder
        put_u64(&mut r, 0); // SyncTeX's length, its name, its text and the root
        put_opt(&mut r, None);
        put_opt(&mut r, None);
        put_opt(&mut r, None);
        put_opt(&mut r, None);
        put_u64(&mut r, 9); // PDF
        put_u64(&mut r, rebased.len() as u64);
        for &(i, h) in rebased {
            put_u64(&mut r, i);
            put_u64(&mut r, h);
        }
        put_u64(&mut r, 0); // extents
        put_opt(&mut r, None);
        put_u64(&mut r, 0); // sums
        put_opt(&mut r, None); // what the native path's memos hold
        r
    }

    #[test]
    fn a_chain_ends_at_its_last_whole_frame() {
        let mut raw = head(5, 3, &[(1, 11)]);
        raw.extend(frame(&record(4, 3, None, &[]), &[], &[]));
        let whole = raw.len();
        raw.extend(frame(&record(8, 4, Some((b"a.tex", 22)), &[]), &[], &[]));
        let p = chain_parse(&raw, 5).unwrap();
        assert_eq!(p.recs.len(), 2);
        assert_eq!(p.recs[0].index_end as usize, whole);
        assert_eq!((p.recs[1].page, p.recs[1].term_len, p.recs[1].recorder_len, p.recs[1].pdf_len), (8, 7, 8, 9));
        assert!(chain_parse(&raw, 6).is_none(), "another preamble's");
        // Cut anywhere in the last frame, or with a byte of it flipped.
        for cut in [raw.len() - 1, whole + 9, whole + 1] {
            assert_eq!(chain_parse(&raw[..cut], 5).unwrap().recs.len(), 1, "cut at {cut}");
        }
        let mut flipped = raw.clone();
        flipped[whole + 20] ^= 4;
        assert_eq!(chain_parse(&flipped, 5).unwrap().recs.len(), 1);
    }

    #[test]
    fn a_later_frame_says_what_a_file_has_become() {
        let mut raw = head(5, 3, &[(1, 11)]);
        raw.extend(frame(&record(4, 4, Some((b"a.tex", 22)), &[]), &[], &[]));
        raw.extend(frame(&record(8, 4, None, &[(1, 111), (3, 222)]), &[], &[]));
        let p = chain_parse(&raw, 5).unwrap();
        assert_eq!(p.head_open, vec![(1, 111)]);
        let Observation::Read { hash, kind, .. } = &p.recs[0].obs[0] else { panic!() };
        assert_eq!((*hash, *kind), (222, ReadKind::Project));
    }

    #[test]
    fn what_a_record_read_is_judged_part_by_part() {
        let data = b"one\ntwo\nthree\n".to_vec();
        let input = |pos: u64, from: usize| Input { slot: 65, path: Vec::new(), pos, obs: 9, hash: xxh64(&data[from..pos as usize], 0) };
        let mut to = HashMap::new();
        assert!(read_part_holds(&input(4, 0), &data, &mut to));
        assert!(read_part_holds(&input(8, 4), &data, &mut to));
        // The file as it is now differs at the third line: the first two records still hold.
        let mut now = data.clone();
        now[9] = b'X';
        let mut to = HashMap::new();
        assert!(read_part_holds(&input(4, 0), &now, &mut to));
        assert!(read_part_holds(&input(8, 4), &now, &mut to));
        assert!(!read_part_holds(&input(14, 8), &now, &mut to));
        // Shorter than what was read, and a line that has gone on past where it ended.
        let mut to = HashMap::new();
        assert!(!read_part_holds(&input(14, 0), &data[..10], &mut to));
        let last = b"no end";
        let x = Input { slot: 65, path: Vec::new(), pos: 6, obs: 9, hash: xxh64(last, 0) };
        assert!(read_part_holds(&x, last, &mut HashMap::new()));
        assert!(!read_part_holds(&x, b"no end, and more", &mut HashMap::new()));
    }

    fn test_file(name: &str, data: &[u8]) -> std::fs::File {
        let p = std::env::temp_dir().join(format!("rstex-pages-{}-{}", std::process::id(), name));
        std::fs::write(&p, data).unwrap();
        let f = std::fs::File::open(&p).unwrap();
        let _ = std::fs::remove_file(&p);
        f
    }

    #[test]
    fn the_last_version_of_a_page() {
        // In the file: bytes from 100 on, for a span over image pages 2 and 3, and two small spans that share page 5.
        let mut ck = vec![0u8; 100];
        ck.extend(std::iter::repeat_n(1u8, 2 * PAGE));
        let small = ck.len() as u64;
        ck.extend(std::iter::repeat_n(2u8, 300));
        ck.extend(std::iter::repeat_n(3u8, 200));
        let spans = vec![(2 * PAGE, 4 * PAGE, 100), (5 * PAGE + 100, 5 * PAGE + 400, small), (5 * PAGE + 900, 5 * PAGE + 1100, small + 300)];
        let chain_bytes: Vec<u8> = std::iter::repeat_n(9u8, PAGE).collect();
        let mut chain = platform::HugeBuf::new_small(4 * PAGE).unwrap();
        chain.as_mut_slice()[..PAGE].copy_from_slice(&chain_bytes);
        let mut base = Base {
            ck: platform::FileView::new(&test_file("ck", &ck), ck.len()).unwrap(),
            spans,
            span_at: 0,
            chain_file: test_file("chain", &chain_bytes),
            chain,
            chain_len: PAGE as u64,
            overlay: vec![OverPage { img: 3 * PAGE, at: 0, n: PAGE, sum: 0 }, OverPage { img: 9 * PAGE, at: 0, n: PAGE, sum: 0 }],
            overlay_at: 0,
            next: Vec::new(),
        };
        let mut buf = [0u8; PAGE];
        base.rewind();
        // Below everything: zero.
        let (v, c) = base.version(PAGE, PAGE, &mut buf);
        assert!(!c && v.iter().all(|&b| b == 0));
        base.settle(c, None);
        let (v, c) = base.version(2 * PAGE, PAGE, &mut buf);
        assert!(!c && v.iter().all(|&b| b == 1));
        base.settle(c, None);
        // The chain has this one.
        let (v, c) = base.version(3 * PAGE, PAGE, &mut buf);
        assert!(c && v.iter().all(|&b| b == 9));
        base.settle(c, Some(OverPage { img: 3 * PAGE, at: 77, n: PAGE, sum: 0 }));
        // Between spans: zero.  Pieces of two: put together.
        let (v, c) = base.version(4 * PAGE, PAGE, &mut buf);
        assert!(!c && v.iter().all(|&b| b == 0));
        base.settle(c, None);
        let (v, c) = base.version(5 * PAGE, PAGE, &mut buf);
        assert!(!c);
        assert_eq!((v[99], v[100], v[399], v[400], v[899], v[900], v[1099], v[1100]), (0, 2, 2, 0, 0, 3, 3, 0));
        base.settle(c, Some(OverPage { img: 5 * PAGE, at: 88, n: PAGE, sum: 0 }));
        // Page 9 is not looked at: it stays.
        base.end_of_walk();
        let list: Vec<(usize, u64)> = base.overlay.iter().map(|o| (o.img, o.at)).collect();
        assert_eq!(list, vec![(3 * PAGE, 77), (5 * PAGE, 88), (9 * PAGE, 0)]);
    }

    #[test]
    fn the_chain_can_be_seen_as_it_grows() {
        let p = std::env::temp_dir().join(format!("rstex-pages-grow-{}", std::process::id()));
        let size = platform::page_size();
        std::fs::write(&p, vec![5u8; size + 100]).unwrap();
        let f = std::fs::OpenOptions::new().read(true).write(true).open(&p).unwrap();
        let mut base = Base {
            ck: platform::FileView::new(&test_file("empty", b""), 0).unwrap(),
            spans: Vec::new(),
            span_at: 0,
            chain_file: f,
            chain: platform::HugeBuf::new_small(16 * size).unwrap(),
            chain_len: 0,
            overlay: Vec::new(),
            overlay_at: 0,
            next: Vec::new(),
        };
        assert!(base.see_chain(size as u64 + 100));
        assert!(base.chain.as_slice()[..size + 100].iter().all(|&b| b == 5));
        let mut g = std::fs::OpenOptions::new().append(true).open(&p).unwrap();
        g.write_all(&vec![6u8; 2 * size]).unwrap();
        assert!(base.see_chain(3 * size as u64 + 100));
        assert!(base.chain.as_slice()[..size + 100].iter().all(|&b| b == 5), "what was there stays");
        assert!(base.chain.as_slice()[size + 100..3 * size + 100].iter().all(|&b| b == 6));
        assert!(!base.see_chain(1 << 40), "more than the room");
        std::fs::remove_file(&p).unwrap();
    }

    #[test]
    fn a_chain_held_by_a_run_stays() {
        let d = std::env::temp_dir().join(format!("rstex-pages-lock-{}.pg", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        let bytes = platform::path_to_bytes(&d);
        assert!(chain_lock(&bytes, false).is_none(), "no chain, nothing to look at");
        let held = chain_lock(&bytes, true).unwrap();
        assert!(chain_lock(&bytes, false).is_none(), "one run at a time");
        std::fs::write(d.join("f1"), b"bytes").unwrap();
        assert!(!chain_remove(&d));
        assert!(d.join("f1").exists());
        drop(held);
        assert!(chain_lock(&bytes, false).is_some(), "the end of a run lets go of it");
        assert!(chain_remove(&d));
        assert!(!d.exists());
        assert!(chain_remove(&d), "gone already");
    }
}
