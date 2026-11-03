//! Prototype: checkpoints at page boundaries, so that a run can resume in the middle of a document.  Measuring only; nothing here is on unless
//! `RSTEX_PAGE_CK=N` asks for a record every N pages.
//!
//! A chain lives beside the preamble checkpoint, `HASH.ck`, in a directory `HASH.pg`, and hangs from it: its first record says which preamble it belongs to.
//! Each later record holds what changed since the one before: the image pages that differ, found by comparing with a shadow of the last image; the new observations
//! of the outside world; how long each output stream was then; the read position of each open file.  Output is append-only, so the stream files are one copy
//! of each stream, and a record is only a set of lengths.
//!
//! To resume: the preamble's log is answered as for any restore; then the body's log, in order.  A project file that changed does not end things: a record survives as long as
//! it has not read as far as the first byte that differs.  The latest record left is the one to use.  Its image is the preamble's plus, for each page, the newest
//! version among the deltas up to it.
#![allow(clippy::too_many_arguments)]

use super::*;
use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{Sender, channel};

const PAGE: usize = 4096;
const F_INDEX: u32 = 0;
const F_IMAGE: u32 = 1;
const F_PDF: u32 = 2;
const F_TERM: u32 = 3;
const F_COPIES: u32 = 4;
const F_STREAM: u32 = 100;
/// A file read larger than this is not kept for a first difference: a change to it counts as a change at byte 0.
const COPY_MAX: usize = 1 << 26;

/// What a run has of this: the options, and the writer once there is one.
#[derive(Default)]
pub struct PageHost {
    /// `RSTEX_PAGE_CK`: a record every so many pages.  0 for none.
    pub every: i32,
    /// `RSTEX_PAGE_MIN_MINSTR`: not more often than this many millions of the run's instructions, if there is a counter to ask.  0 for no limit.
    pub writer: Option<Box<PageWriter>>,
    /// A restored run is checking its state against the records it did not use: `RSTEX_PAGE_VERIFY`.
    pub verify: Option<Verify>,
    /// `RSTEX_PAGE_HASH`: the records hold a sum of each section, for `verify`.
    pub hash: bool,
    /// This run was restored at a page: `main_control` is next, with no `macro_call`.
    pub resumed_at: i32,
}

#[derive(Default)]
pub struct Verify {
    records: Vec<(i32, Vec<u64>)>,
    checked: u32,
    differ: u32,
}

struct Job {
    parts: Vec<(u32, Vec<u8>)>,
}

/// The image's bytes of a record go to the worker and come back, so that each record does not take new pages.
type Back = std::sync::mpsc::Receiver<Vec<u8>>;

pub struct PageWriter {
    tag: u64,
    shadow: platform::HugeBuf,
    /// The union of what each section has been live for, in bytes.
    high: Vec<(usize, usize)>,
    last_page: i32,
    log_sent: usize,
    /// `(path, stream)` of what has an open stream file.
    streams: Vec<(Vec<u8>, u32, u64)>,
    next_stream: u32,
    closed_sent: usize,
    pdf_sent: u64,
    image_at: u64,
    copies_at: u64,
    term_len: u64,
    records: u32,
    tx: Option<Sender<Job>>,
    back: Back,
    worker: Option<std::thread::JoinHandle<()>>,
    dir: Vec<u8>,
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
                "rstex: checkpoint: pages: {} records, {} pages of the image compared, {} changed, {} extents, image {} bytes\n",
                self.records, self.pages_compared, self.pages_changed, self.extents, self.image_at
            );
            for (k, n) in self.per_section.iter().enumerate() {
                if *n > 0 || self.compared_per_section[k] > 0 {
                    s.push_str(&format!("rstex: checkpoint: pages: section {} changed {} compared {}\n", POD_SECTIONS[k].name, n, self.compared_per_section[k]));
                }
            }
            let _ = std::io::stderr().write_all(s.as_bytes());
        }
    }
}

impl HostState {
    pub(crate) fn pages_finish(&mut self) {
        let trace = self.trace;
        if let Some(mut w) = self.page.writer.take() {
            w.finish(trace);
        }
        if let Some(v) = self.page.verify.take() {
            if trace {
                let s = format!("rstex: checkpoint: verify: {} pages checked, {} differ\n", v.checked, v.differ);
                let _ = std::io::stderr().write_all(s.as_bytes());
            }
        }
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

fn file_in(dir: &[u8], id: u32) -> Vec<u8> {
    [dir, format!("/f{}", id).as_bytes()].concat()
}

fn writer_thread(dir: Vec<u8>, rx: std::sync::mpsc::Receiver<Job>, fresh: bool, back: Sender<Vec<u8>>) {
    if fresh {
        let _ = std::fs::remove_dir_all(platform::path_from_bytes(&dir));
        let _ = std::fs::create_dir_all(platform::path_from_bytes(&dir));
    }
    let mut files: HashMap<u32, std::fs::File> = HashMap::new();
    while let Ok(job) = rx.recv() {
        for (id, bytes) in job.parts {
            let f = files.entry(id).or_insert_with(|| {
                std::fs::OpenOptions::new().create(true).append(true).open(platform::path_from_bytes(&file_in(&dir, id))).expect("a file in the chain's directory")
            });
            let _ = f.write_all(&bytes);
            if id == F_IMAGE {
                let mut b = bytes;
                b.clear();
                let _ = back.send(b);
            }
        }
    }
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
    /// §1517's driver, from the options: this run may write a chain.
    pub fn pages_configure(&mut self) {
        self.host.checkpoint.page.every = platform::env_var(b"RSTEX_PAGE_CK").and_then(|v| String::from_utf8_lossy(&v).parse().ok()).unwrap_or(0);
        self.host.checkpoint.page.hash = platform::env_var(b"RSTEX_PAGE_HASH").is_some();
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

    /// `checkpoint_take` has copied the preamble's image, which is what the first delta is against.  `tag` ties the chain to that checkpoint.
    pub(super) fn pages_begin(&mut self, key_dir: Vec<u8>, tag: u64, open_copies: Vec<(u64, Vec<u8>)>, armed: Vec<i32>, pdf_sent: u64, obs_base: u64) {
        let limit = core::mem::offset_of!(Tex, host);
        let Some(mut shadow) = platform::HugeBuf::new_small(limit) else { return };
        let high = self.pages_live_ranges();
        for &(a, b, _) in &merged_ranges(&high, limit) {
            shadow.as_mut_slice()[a..b].copy_from_slice(image_bytes(self, a, b - a));
        }
        for cs in armed {
            self.checkpoint_arm_cs(cs);
        }
        let (tx, rx) = channel();
        let (back_tx, back) = channel();
        let d = key_dir.clone();
        let worker = std::thread::Builder::new().stack_size(1 << 18).spawn(move || writer_thread(d, rx, true, back_tx)).ok();
        // The first record: the tag, and what was open at the preamble that a change could still be past.
        let mut head = Vec::new();
        put_u64(&mut head, tag);
        put_u64(&mut head, obs_base);
        put_u64(&mut head, open_copies.len() as u64);
        let mut parts = Vec::new();
        let mut copies_at = 0;
        for (idx, bytes) in open_copies {
            put_u64(&mut head, idx);
            put_u64(&mut head, copies_at);
            put_u64(&mut head, bytes.len() as u64);
            copies_at += bytes.len() as u64;
            parts.push((F_COPIES, bytes));
        }
        let mut framed = Vec::with_capacity(head.len() + 16);
        put_bytes(&mut framed, &head);
        put_u64(&mut framed, xxh64(&head, 0));
        parts.push((F_INDEX, framed));
        let _ = tx.send(Job { parts });
        self.host.world.page_copies = Some(Vec::new());
        self.host.checkpoint.page.writer = Some(Box::new(PageWriter {
            tag,
            shadow,
            high,
            last_page: 0,
            log_sent: obs_base as usize,
            streams: Vec::new(),
            next_stream: 0,
            closed_sent: self.host.world.closed_outputs.len(),
            pdf_sent,
            image_at: 0,
            copies_at,
            term_len: 0,
            records: 0,
            tx: Some(tx),
            back,
            worker,
            dir: key_dir,
            pages_changed: 0,
            pages_compared: 0,
            extents: 0,
            per_section: vec![0; POD_SECTIONS.len()],
            compared_per_section: vec![0; POD_SECTIONS.len()],
            dump: pages_dump_setting(),
        }));
    }

    /// Why this page cannot be a record, or `None`.  `Err` for the rest of the run, `Ok` for this page.
    fn pages_refusal(&mut self) -> Option<Result<String, String>> {
        if self.host.world.taint == Taint::Hard {
            return Some(Err(self.host.world.taint_reason.to_string()));
        }
        // All of what an error leaves is POD: `history`, `error_count`, what it printed.  Only one that needs the terminal cannot be gone on from, and a read of the terminal is a hard taint.
        if self.history > WARNING_ISSUED && platform::env_var(b"RSTEX_PAGE_ERRORS").is_none() {
            return Some(Err("an error was reported".to_string()));
        }
        if self.host.world.recorder_enabled {
            return Some(Err("the recorder is on".to_string()));
        }
        if self.pdf_output() <= 0 {
            return Some(Err("DVI".to_string()));
        }
        // The prefix made from the job id, which comes from the clock, is in the pages' bytes.
        if self.pdf_unique_resname() > 0 {
            return Some(Err("\\pdfuniqueresname".to_string()));
        }
        if self.v_image.cd.active {
            return Some(Ok("in the middle of a native diagram".to_string()));
        }
        if self.help_ptr != 0 || self.interrupt != 0 {
            return Some(Ok("in the middle of an error".to_string()));
        }
        if self.ext_host_state_open() {
            return Some(Ok("a PDF or JBIG2 file is open".to_string()));
        }
        for (k, s) in self.host.files.iter().enumerate() {
            let k = k as i32;
            let fine = match s {
                Slot::Closed => true,
                Slot::In(_) => (READ_FILE_BASE..WRITE_FILE_BASE).contains(&k) || k >= INPUT_FILE_BASE,
                Slot::Fmt(_) => false,
                Slot::Out(o) => (k == LOG_FILE || (WRITE_FILE_BASE..INPUT_FILE_BASE).contains(&k)) && o.journal.is_some() && !o.is_pipe(),
                Slot::Pdf(_) => k == PDF_FILE && !self.pdfout_in_stream(),
            };
            if !fine {
                return Some(Ok(format!("file slot {} is open", k)));
            }
        }
        None
    }

    /// The big switch, just after a page went out, where a restore can come back to: the record.
    #[cold]
    #[inline(never)]
    pub fn pages_take(&mut self) {
        let _p = crate::prof_scope!("page checkpoint");
        self.v_image.ck.page_due = false;
        let every = self.host.checkpoint.page.every;
        if self.host.checkpoint.page.verify.is_some() {
            self.pages_verify();
            return;
        }
        let Some(mut w) = self.host.checkpoint.page.writer.take() else { return };
        if self.total_pages < w.last_page + every {
            self.host.checkpoint.page.writer = Some(w);
            return;
        }
        match self.pages_refusal() {
            Some(Err(why)) => {
                self.checkpoint_say(&format!("pages: stop at page {}: {}", self.total_pages, why));
                let trace = self.host.checkpoint.trace;
                w.finish(trace);
                self.checkpoint_stop();
                return;
            }
            Some(Ok(why)) => {
                self.checkpoint_say(&format!("pages: skip page {}: {}", self.total_pages, why));
                self.host.checkpoint.page.writer = Some(w);
                return;
            }
            None => {}
        }
        self.pdfout_settle();
        self.host.term_flush();
        w.last_page = self.total_pages;
        let mut parts: Vec<(u32, Vec<u8>)> = Vec::new();
        let mut r = Vec::with_capacity(1 << 12);
        put_u64(&mut r, self.total_pages as u64);
        // What was observed since.
        let log_len = self.host.world.log.len();
        put_u64(&mut r, log_len as u64);
        put_u64(&mut r, (log_len - w.log_sent) as u64);
        for o in &self.host.world.log[w.log_sent..] {
            o.encode(&mut r);
        }
        w.log_sent = log_len;
        put_u64(&mut r, if self.v_image.ck.date_read { (self.sys_year * 10000 + self.sys_month * 100 + self.sys_day) as u64 } else { 0 });
        // The open files.
        let mut inputs = Vec::new();
        for k in 0..self.host.files.len() {
            if let Slot::In(f) = &self.host.files[k] {
                inputs.push((k as u64, f.path.clone(), f.pos as u64, self.host.world.in_obs[k] as u64));
            }
        }
        if platform::env_var(b"RSTEX_PAGE_INPUTS").is_some() {
            if let Some((_, p, pos, _)) = inputs.iter().rev().find(|x| x.0 >= INPUT_FILE_BASE as u64) {
                self.checkpoint_say(&format!("pages: rec {} page {} at {} {}", w.records + 1, self.total_pages, pos, String::from_utf8_lossy(p)));
            }
        }
        put_u64(&mut r, inputs.len() as u64);
        for (k, p, pos, i) in &inputs {
            put_u64(&mut r, *k);
            put_bytes(&mut r, p);
            put_u64(&mut r, *pos);
            put_u64(&mut r, *i);
        }
        // The streams closed since, whole.
        let closed: Vec<(Vec<u8>, Vec<u8>)> = self.host.world.closed_outputs[w.closed_sent..].to_vec();
        w.closed_sent = self.host.world.closed_outputs.len();
        put_u64(&mut r, closed.len() as u64);
        for (path, bytes) in closed {
            let (id, len) = w.stream_add(path_norm(&path), bytes.len() as u64, true);
            put_bytes(&mut r, &path);
            put_u64(&mut r, id as u64);
            put_u64(&mut r, len);
            parts.push((F_STREAM + id, bytes));
        }
        // The open ones.
        let mut outs: Vec<(u64, Vec<u8>, u32, u64)> = Vec::new();
        for k in 0..self.host.files.len() {
            if let Slot::Out(o) = &mut self.host.files[k] {
                let bytes = core::mem::replace(o.journal.as_mut().expect("the refusal looked"), Vec::new());
                let (id, len) = w.stream_add(path_norm(&o.path), bytes.len() as u64, false);
                outs.push((k as u64, o.path.clone(), id, len));
                if !bytes.is_empty() {
                    parts.push((F_STREAM + id, bytes));
                }
            }
        }
        put_u64(&mut r, outs.len() as u64);
        for (k, p, id, len) in &outs {
            put_u64(&mut r, *k);
            put_bytes(&mut r, p);
            put_u64(&mut r, *id as u64);
            put_u64(&mut r, *len);
        }
        // The terminal.
        let t = core::mem::replace(self.host.checkpoint.term_journal.as_mut().expect("while recording"), Vec::new());
        w.term_len += t.len() as u64;
        put_u64(&mut r, w.term_len);
        parts.push((F_TERM, t));
        // The PDF, what the chunk list gained.
        let mut pdf_path = None;
        if let Slot::Pdf(p) = &self.host.files[PDF_FILE as usize] {
            let total = p.offset() as u64;
            parts.push((F_PDF, p.contents_from(w.pdf_sent)));
            w.pdf_sent = total;
            pdf_path = Some(p.path.clone());
        }
        put_opt(&mut r, pdf_path.as_deref());
        put_u64(&mut r, w.pdf_sent);
        // Project files read since, for a later first difference.
        let copies = self.host.world.page_copies.as_mut().map(core::mem::take).unwrap_or_default();
        put_u64(&mut r, copies.len() as u64);
        for (idx, bytes) in copies {
            put_u64(&mut r, idx as u64);
            put_u64(&mut r, w.copies_at);
            put_u64(&mut r, bytes.len() as u64);
            w.copies_at += bytes.len() as u64;
            parts.push((F_COPIES, bytes));
        }
        // The image: pages that differ from the last record's.
        let limit = core::mem::offset_of!(Tex, host);
        let live = self.pages_live_ranges();
        for (h, l) in w.high.iter_mut().zip(&live) {
            if l.0 < l.1 {
                *h = if h.0 < h.1 { (h.0.min(l.0), h.1.max(l.1)) } else { *l };
            }
        }
        let mut extents: Vec<(u64, u64, u64)> = Vec::new();
        let mut data: Vec<u8> = w.back.try_recv().unwrap_or_else(|_| Vec::with_capacity(1 << 20));
        for &(a, b, _) in &merged_ranges(&w.high, limit) {
            let mut at = a;
            while at < b {
                let e = (at + PAGE).min(b);
                let now = image_bytes(self, at, e - at);
                let was = &mut w.shadow.as_mut_slice()[at..e];
                w.pages_compared += 1;
                if self.host.checkpoint.trace {
                    if let Some(k) = POD_SECTIONS.iter().rposition(|s| s.offset <= at) {
                        w.compared_per_section[k] += 1;
                    }
                }
                if now != &*was {
                    was.copy_from_slice(now);
                    match extents.last_mut() {
                        Some(l) if l.0 + l.2 == at as u64 => l.2 += (e - at) as u64,
                        _ => extents.push((at as u64, w.image_at + data.len() as u64, (e - at) as u64)),
                    }
                    data.extend_from_slice(now);
                    w.pages_changed += 1;
                    if self.host.checkpoint.trace {
                        // Which section it is, for the totals.
                        if let Some(k) = POD_SECTIONS.iter().rposition(|s| s.offset <= at) {
                            w.per_section[k] += 1;
                        }
                    }
                }
                at = e;
            }
        }
        w.image_at += data.len() as u64;
        w.extents += extents.len() as u64;
        put_u64(&mut r, extents.len() as u64);
        for &(x, y, z) in &extents {
            put_u64(&mut r, x);
            put_u64(&mut r, y);
            put_u64(&mut r, z);
        }
        parts.push((F_IMAGE, data));
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
        let mut framed = Vec::with_capacity(r.len() + 16);
        put_bytes(&mut framed, &r);
        put_u64(&mut framed, xxh64(&r, 0));
        parts.push((F_INDEX, framed));
        if let Some(tx) = &w.tx {
            let _ = tx.send(Job { parts });
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
        for (name, at) in crate::checkpoint::pages::field_offsets() {
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
                self.checkpoint_say(&format!("verify: page {} differs in {:?}", page, names));
            }
        }
        self.host.checkpoint.page.verify = Some(v);
    }

    /// `final_cleanup` is next: no later record, so nothing more to keep.
    pub fn pages_end_of_body(&mut self) {
        if self.host.checkpoint.page.writer.is_some() {
            let trace = self.host.checkpoint.trace;
            let mut w = self.host.checkpoint.page.writer.take().expect("just looked");
            w.finish(trace);
            self.checkpoint_stop();
        }
        self.host.checkpoint.pages_finish();
    }
}

impl PageWriter {
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

// ------------------------------------------------------------------------------------------------ reading a chain
pub(super) struct Rec {
    index_end: u64,
    page: u64,
    log_len: u64,
    obs: Vec<Observation>,
    date_key: i32,
    inputs: Vec<(i32, Vec<u8>, u64, u64)>,
    closed: Vec<(Vec<u8>, u32, u64)>,
    outs: Vec<(i32, Vec<u8>, u32, u64)>,
    term_len: u64,
    pdf: Option<Vec<u8>>,
    pdf_len: u64,
    copies: Vec<(u64, u64, u64)>,
    extents: Vec<(u64, u64, u64)>,
    full_name: Option<Vec<u8>>,
    sums: Vec<u64>,
}

fn rec_decode(b: &[u8]) -> Option<Rec> {
    let mut r = In { b, p: 0 };
    let page = r.u64()?;
    let log_len = r.u64()?;
    let n = r.u64()? as usize;
    let mut obs = Vec::with_capacity(n);
    for _ in 0..n {
        obs.push(Observation::decode(&mut r)?);
    }
    let date_key = r.i32()?;
    let n = r.u64()? as usize;
    let mut inputs = Vec::with_capacity(n);
    for _ in 0..n {
        inputs.push((r.i32()?, r.vec()?, r.u64()?, r.u64()?));
    }
    let n = r.u64()? as usize;
    let mut closed = Vec::with_capacity(n);
    for _ in 0..n {
        closed.push((r.vec()?, r.u64()? as u32, r.u64()?));
    }
    let n = r.u64()? as usize;
    let mut outs = Vec::with_capacity(n);
    for _ in 0..n {
        outs.push((r.i32()?, r.vec()?, r.u64()? as u32, r.u64()?));
    }
    let term_len = r.u64()?;
    let pdf = r.opt()?;
    let pdf_len = r.u64()?;
    let n = r.u64()? as usize;
    let mut copies = Vec::with_capacity(n);
    for _ in 0..n {
        copies.push((r.u64()?, r.u64()?, r.u64()?));
    }
    let n = r.u64()? as usize;
    let mut extents = Vec::with_capacity(n);
    for _ in 0..n {
        extents.push((r.u64()?, r.u64()?, r.u64()?));
    }
    let full_name = r.opt()?;
    let n = r.u64()? as usize;
    let mut sums = Vec::with_capacity(n);
    for _ in 0..n {
        sums.push(r.u64()?);
    }
    Some(Rec { index_end: 0, page, log_len, obs, date_key, inputs, closed, outs, term_len, pdf, pdf_len, copies, extents, full_name, sums })
}

/// A chain, read, and the record chosen.
pub(super) struct Chain {
    dir: Vec<u8>,
    recs: Vec<Rec>,
    use_count: usize,
    kept: Vec<(Vec<u8>, Vec<u8>)>,
    head_copies_end: u64,
}

fn first_diff(a: &[u8], b: &[u8]) -> u64 {
    let n = a.len().min(b.len());
    let mut k = 0;
    while k + 64 <= n && a[k..k + 64] == b[k..k + 64] {
        k += 64;
    }
    while k < n && a[k] == b[k] {
        k += 1;
    }
    k as u64
}

fn read_range(f: &std::fs::File, at: u64, n: u64) -> Option<Vec<u8>> {
    let mut v = vec![0u8; n as usize];
    if platform::read_exact_at(f, at, &mut v) { Some(v) } else { None }
}

impl Tex {
    /// The latest record whose every observation would get the same answer now.
    pub(super) fn pages_select(&mut self, key: &Key, meta: &Meta) -> Option<Chain> {
        let setting = platform::env_var(b"RSTEX_RESUME")?;
        let setting = String::from_utf8_lossy(&setting).to_string();
        let most: usize = if setting == "latest" { usize::MAX } else { setting.parse().ok()? };
        let dir = page_dir(key);
        let raw = std::fs::read(platform::path_from_bytes(&file_in(&dir, F_INDEX))).ok()?;
        // Records.
        let mut r = In { b: &raw, p: 0 };
        let head = r.vec()?;
        if r.u64()? != xxh64(&head, 0) {
            return None;
        }
        let mut h = In { b: &head, p: 0 };
        if h.u64()? != meta.page_tag {
            self.checkpoint_say("pages: another preamble's");
            return None;
        }
        let obs_base = h.u64()? as usize;
        let n = h.u64()? as usize;
        let mut open_copies = Vec::with_capacity(n);
        for _ in 0..n {
            open_copies.push((h.u64()?, h.u64()?, h.u64()?));
        }
        let mut recs = Vec::new();
        while r.p < raw.len() {
            let Some(b) = r.vec() else { break };
            let Some(s) = r.u64() else { break };
            if s != xxh64(&b, 0) {
                break;
            }
            let Some(mut x) = rec_decode(&b) else { break };
            x.index_end = r.p as u64;
            recs.push(x);
        }
        drop(raw);
        crate::prof_phase!("pages index read");
        let copies = std::fs::File::open(platform::path_from_bytes(&file_in(&dir, F_COPIES))).ok();
        let mut limit = recs.len().min(most);
        let today = {
            let (_, d, m, y) = self.date_and_time();
            y * 10000 + m * 100 + d
        };
        for k in 0..limit {
            if recs[k].date_key != 0 && recs[k].date_key != today {
                limit = k;
                break;
            }
        }
        // `(entry of the log, first byte that differs)`.
        let mut changed: Vec<(u64, u64)> = Vec::new();
        let mut files_now: HashMap<Vec<u8>, Option<Vec<u8>>> = HashMap::new();
        let holds = |rec: &Rec, changed: &[(u64, u64)]| changed.iter().all(|&(i, d)| i >= rec.log_len || rec.inputs.iter().any(|x| x.3 == i && x.2 <= d));
        // What was open at the preamble.
        for &(idx, at, len) in &open_copies {
            let Some(Observation::Read { path, .. }) = meta.log.get(idx as usize) else { return None };
            let now = std::fs::read(platform::path_from_bytes(path)).ok();
            let old = copies.as_ref().and_then(|f| read_range(f, at, len))?;
            if now.as_deref() != Some(&old[..]) {
                changed.push((idx, now.as_deref().map_or(0, |n| first_diff(&old, n))));
            }
            files_now.insert(path.clone(), now);
        }
        while limit > 0 && !holds(&recs[limit - 1], &changed) {
            limit -= 1;
        }
        // The body's log, in order, as far as the records that are left reach.
        let mut own: Vec<Vec<u8>> = meta.log.iter().filter_map(|o| if let Observation::Output { path } = o { Some(path_norm(path).to_vec()) } else { None }).collect();
        let mut asked: HashMap<(Vec<u8>, i32, bool), Option<Vec<u8>>> = HashMap::new();
        let mut read: HashMap<(Vec<u8>, u64), Option<u64>> = HashMap::new();
        let mut i = obs_base as u64;
        'walk: for k in 0..recs.len() {
            for o in &recs[k].obs {
                if limit == 0 || i >= recs[limit - 1].log_len {
                    break 'walk;
                }
                let mut bad = false;
                match o {
                    Observation::Output { path } => own.push(path_norm(path).to_vec()),
                    Observation::Lookup { name, format, must_exist, found } => {
                        let now = asked.entry((name.clone(), *format, *must_exist)).or_insert_with(|| rstex_kpse::find_file(name, *format, *must_exist));
                        bad = *now != *found;
                    }
                    Observation::Env { name, value, cnf } => {
                        let now = if *cnf { rstex_kpse::var_value(name) } else { platform::env_var(name) };
                        bad = now != *value;
                    }
                    Observation::Stat { path, stat, size_only } => {
                        let now = platform::file_stat(path);
                        bad = if *size_only { !own.iter().any(|p| &p[..] == path_norm(path)) && now.as_ref().map(|s| s.size) != stat.as_ref().map(|s| s.size) } else { now != *stat };
                    }
                    Observation::Read { path, stat, hash, prefix, kind } => {
                        if !own.iter().any(|p| &p[..] == path_norm(path)) {
                            let verdict = match read.get(&(path.clone(), *hash)) {
                                Some(v) => *v,
                                None => {
                                    let mut kept = Vec::new();
                                    let v = if checkpoint_read_holds(path, stat, *hash, *prefix, *kind, &mut kept) {
                                        None
                                    } else {
                                        let now = std::fs::read(platform::path_from_bytes(path)).ok();
                                        let old = recs[k].copies.iter().find(|c| c.0 == i).and_then(|c| copies.as_ref().and_then(|f| read_range(f, c.1, c.2)));
                                        Some(match (&old, &now) {
                                            (Some(o), Some(n)) => first_diff(o, n),
                                            _ => 0,
                                        })
                                    };
                                    read.insert((path.clone(), *hash), v);
                                    v
                                }
                            };
                            if let Some(d) = verdict {
                                changed.push((i, d));
                                while limit > 0 && !holds(&recs[limit - 1], &changed) {
                                    limit -= 1;
                                }
                            }
                        }
                    }
                }
                if bad {
                    while limit > 0 && recs[limit - 1].log_len > i {
                        limit -= 1;
                    }
                }
                i += 1;
            }
        }
        // A negative control for the gate: the record asked for, whatever the log said.
        if platform::env_var(b"RSTEX_RESUME_FORCE").is_some() {
            limit = recs.len().min(most);
        }
        crate::prof_phase!("pages log answered");
        self.checkpoint_say(&format!("pages: {} records, using {}{}", recs.len(), limit, if limit > 0 { format!(", page {}", recs[limit - 1].page) } else { String::new() }));
        if limit == 0 {
            return None;
        }
        // The file slots of the record come back with what the disk has now.
        let mut kept = Vec::new();
        for x in &recs[limit - 1].inputs {
            let d = match files_now.remove(&x.1) {
                Some(Some(d)) => d,
                _ => std::fs::read(platform::path_from_bytes(&x.1)).ok()?,
            };
            kept.push((x.1.clone(), d));
        }
        let head_copies_end = open_copies.iter().map(|c: &(u64, u64, u64)| c.1 + c.2).max().unwrap_or(0);
        Some(Chain { dir, recs, use_count: limit, kept, head_copies_end })
    }

    /// The newest version of each page up to the record, over the preamble's image.
    pub(super) fn pages_overlay(&mut self, c: &Chain) -> bool {
        let Ok(f) = std::fs::File::open(platform::path_from_bytes(&file_in(&c.dir, F_IMAGE))) else { return false };
        let limit = core::mem::offset_of!(Tex, host);
        let mut seen = vec![false; limit / PAGE + 1];
        let mut list: Vec<(u64, u64, u64)> = Vec::new();
        for rec in c.recs[..c.use_count].iter().rev() {
            for &(img, at, n) in &rec.extents {
                let mut x = 0;
                while x < n {
                    let page = ((img + x) as usize) / PAGE;
                    let e = (PAGE as u64).min(n - x);
                    if !seen[page] {
                        seen[page] = true;
                        list.push((img + x, at + x, e));
                    }
                    x += e;
                }
            }
        }
        list.sort_by_key(|l| l.1);
        // One read for each run that is together both in the file and in the image.
        let (mut reads, mut bytes) = (0u64, 0u64);
        let mut k = 0;
        while k < list.len() {
            let (img, at, mut n) = list[k];
            k += 1;
            while k < list.len() && list[k].0 == img + n && list[k].1 == at + n {
                n += list[k].2;
                k += 1;
            }
            if !platform::read_exact_at(&f, at, image_bytes_mut(self, img as usize, n as usize)) {
                return false;
            }
            reads += 1;
            bytes += n;
        }
        self.checkpoint_say(&format!("pages: overlay {} reads, {} bytes", reads, bytes));
        true
    }

    /// A restored run goes on recording, as the run that wrote it did: `RSTEX_PAGE_CK`.
    fn pages_recording_on(&mut self, log: Vec<Observation>, in_obs: Vec<(i32, usize)>, written: Vec<Vec<u8>>, open: Vec<Vec<u8>>) {
        self.pages_configure();
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
        w.page_copies = Some(Vec::new());
        self.host.journal_outputs = true;
        self.host.checkpoint.term_journal = Some(Vec::new());
        for s in self.host.files.iter_mut() {
            if let Slot::Out(o) = s {
                o.journal = Some(Vec::new());
            }
        }
    }

    /// The chain up to the record stays, and what came after it goes.
    fn pages_go_on(&mut self, c: &Chain, mut meta_log: Vec<Observation>, preamble_pdf: u64, pos: Vec<Vec<u8>>) -> bool {
        let n = c.use_count;
        let rec = &c.recs[n - 1];
        let image_end = c.recs[..n].iter().flat_map(|r| r.extents.iter()).map(|e| e.1 + e.2).max().unwrap_or(0);
        let mut copies_end = c.head_copies_end;
        for r in &c.recs[..n] {
            for e in &r.copies {
                copies_end = copies_end.max(e.1 + e.2);
            }
        }
        let mut next_stream = 0;
        for r in &c.recs[..n] {
            for x in &r.closed {
                next_stream = next_stream.max(x.1 + 1);
            }
            for x in &r.outs {
                next_stream = next_stream.max(x.2 + 1);
            }
        }
        let cut = |id: u32, len: u64| {
            if let Ok(f) = std::fs::OpenOptions::new().write(true).open(platform::path_from_bytes(&file_in(&c.dir, id))) {
                let _ = f.set_len(len);
            }
        };
        cut(F_INDEX, rec.index_end);
        cut(F_IMAGE, image_end);
        cut(F_COPIES, copies_end);
        cut(F_TERM, rec.term_len);
        cut(F_PDF, rec.pdf_len - preamble_pdf);
        for x in &rec.outs {
            cut(F_STREAM + x.2, x.3);
        }
        if let Ok(rd) = std::fs::read_dir(platform::path_from_bytes(&c.dir)) {
            for e in rd.flatten() {
                let name = e.file_name().to_string_lossy().to_string();
                if let Some(id) = name.strip_prefix('f').and_then(|x| x.parse::<u32>().ok()) {
                    if id >= F_STREAM + next_stream {
                        let _ = std::fs::remove_file(e.path());
                    }
                }
            }
        }
        for r in &c.recs[..n] {
            meta_log.extend(r.obs.iter().cloned());
        }
        let log_len = meta_log.len();
        let in_obs: Vec<(i32, usize)> = rec.inputs.iter().map(|x| (x.0, x.3 as usize)).collect();
        let open: Vec<Vec<u8>> = rec.outs.iter().filter(|x| x.0 != LOG_FILE).map(|x| x.1.clone()).collect();
        let mut written = pos;
        for r in &c.recs[..n] {
            written.extend(r.closed.iter().map(|x| x.0.clone()));
        }
        written.extend(open.iter().cloned());
        self.pages_recording_on(meta_log, in_obs, written, open);
        let limit = core::mem::offset_of!(Tex, host);
        let Some(mut shadow) = platform::HugeBuf::new_small(limit) else { return false };
        let high = self.pages_live_ranges();
        for &(a, b, _) in &merged_ranges(&high, limit) {
            shadow.as_mut_slice()[a..b].copy_from_slice(image_bytes(self, a, b - a));
        }
        let (tx, rx) = channel();
        let (back_tx, back) = channel();
        let d = c.dir.clone();
        let worker = std::thread::Builder::new().stack_size(1 << 18).spawn(move || writer_thread(d, rx, false, back_tx)).ok();
        self.host.checkpoint.page.writer = Some(Box::new(PageWriter {
            tag: 0,
            shadow,
            high,
            last_page: rec.page as i32,
            log_sent: log_len,
            streams: rec.outs.iter().map(|x| (path_norm(&x.1).to_vec(), x.2, x.3)).collect(),
            next_stream,
            closed_sent: 0,
            pdf_sent: rec.pdf_len,
            image_at: image_end,
            copies_at: copies_end,
            term_len: rec.term_len,
            records: n as u32,
            tx: Some(tx),
            back,
            worker,
            dir: c.dir.clone(),
            pages_changed: 0,
            pages_compared: 0,
            extents: 0,
            per_section: vec![0; POD_SECTIONS.len()],
            compared_per_section: vec![0; POD_SECTIONS.len()],
            dump: pages_dump_setting(),
        }));
        true
    }

    /// A run restored at the preamble makes a new chain, on the files as they now are.
    pub(super) fn pages_begin_warm(&mut self, key: &Key, log: Vec<Observation>, tag: u64, inputs: &[(i32, Vec<u8>, u64)], pdf_sent: u64, written: Vec<Vec<u8>>) {
        if platform::env_var(b"RSTEX_PAGE_CK").is_none() || tag == 0 {
            return;
        }
        let mut in_obs = Vec::new();
        let mut open_copies = Vec::new();
        for (slot, path, pos) in inputs {
            let i = log.iter().position(|o| matches!(o, Observation::Read { path: p, prefix: Some(x), .. } if p == path && x == pos));
            let Some(i) = i else { return };
            in_obs.push((*slot, i));
            if let (Observation::Read { kind: ReadKind::Project, .. }, Slot::In(f)) = (&log[i], &self.host.files[*slot as usize]) {
                open_copies.push((i as u64, f.data.clone()));
            }
        }
        let obs_base = log.len() as u64;
        let open: Vec<Vec<u8>> = self.host.files.iter().enumerate().filter_map(|(k, s)| if let Slot::Out(o) = s { if k as i32 != LOG_FILE { Some(o.path.clone()) } else { None } } else { None }).collect();
        self.pages_recording_on(log, in_obs, written, open);
        let ck = self.v_image.ck;
        let armed: Vec<i32> = ck.sys_cs.0.iter().copied().chain([ck.seed_cs]).filter(|&c| c != 0).collect();
        self.pages_begin(page_dir(key), tag, open_copies, armed, pdf_sent, obs_base);
    }

    /// As `checkpoint_restore_host`, for a record.
    pub(super) fn pages_restore_host(&mut self, mut meta: Meta, now: (i32, i32, i32, i32), mut c: Chain) -> bool {
        let n = c.use_count;
        let last = n - 1;
        let go_on = platform::env_var(b"RSTEX_PAGE_CK").is_some().then(|| {
            let own: Vec<Vec<u8>> = meta.log.iter().filter_map(|o| if let Observation::Output { path } = o { Some(path.clone()) } else { None }).collect();
            (meta.log.clone(), meta.pdf.as_ref().map_or(0, |p| p.1.len() as u64), own)
        });
        let term = {
            let Ok(f) = std::fs::File::open(platform::path_from_bytes(&file_in(&c.dir, F_TERM))) else { return false };
            let Some(t) = read_range(&f, 0, c.recs[last].term_len) else { return false };
            t
        };
        self.host.world.terminal_write(&meta.term);
        self.host.world.terminal_write(&term);
        for (path, bytes) in &meta.closed {
            if std::fs::write(platform::path_from_bytes(path), bytes).is_err() {
                return false;
            }
        }
        let mut base: HashMap<Vec<u8>, Vec<u8>> = HashMap::new();
        for (_, path, bytes) in core::mem::take(&mut meta.outputs) {
            base.insert(path_norm(&path).to_vec(), bytes);
        }
        let dir = c.dir.clone();
        let whole = |base: &mut HashMap<Vec<u8>, Vec<u8>>, path: &[u8], id: u32, len: u64| -> Option<Vec<u8>> {
            let mut v = base.remove(path_norm(path)).unwrap_or_default();
            if len > 0 {
                let f = std::fs::File::open(platform::path_from_bytes(&file_in(&dir, F_STREAM + id))).ok()?;
                v.extend_from_slice(&read_range(&f, 0, len)?);
            }
            Some(v)
        };
        for rec in &c.recs[..n] {
            for (path, id, len) in &rec.closed {
                let Some(b) = whole(&mut base, path, *id, *len) else { return false };
                if std::fs::write(platform::path_from_bytes(path), &b).is_err() {
                    return false;
                }
            }
        }
        for (slot, path, _pos, _) in &c.recs[last].inputs {
            let k = c.kept.iter().position(|k| k.0 == *path).expect("read for this");
            let data = c.kept.swap_remove(k).1;
            let pos = c.recs[last].inputs.iter().find(|x| x.0 == *slot).map(|x| x.2).unwrap_or(0);
            self.host.files[*slot as usize] = Slot::In(InFile { path: path.clone(), data, pos: pos as usize });
        }
        let (start, end) = (self.v_image.ck.log_date_start as usize, self.v_image.ck.log_date_end as usize);
        let (t, d, m, y) = now;
        for (slot, path, id, len) in &c.recs[last].outs {
            let Some(bytes) = whole(&mut base, path, *id, *len) else { return false };
            let Some(mut o) = OutFile::reopen(path) else { return false };
            if *slot == LOG_FILE {
                o.bytes(&bytes[..start]);
                o.bytes(&log_date(d, m, y, t));
                o.bytes(&bytes[end..]);
            } else {
                o.bytes(&bytes);
            }
            self.host.files[*slot as usize] = Slot::Out(o);
        }
        let rec = &c.recs[last];
        if let Some(path) = &rec.pdf {
            let mut bytes = meta.pdf.take().map(|p| p.1).unwrap_or_default();
            let at = bytes.len() as u64;
            if rec.pdf_len > at {
                let Ok(f) = std::fs::File::open(platform::path_from_bytes(&file_in(&c.dir, F_PDF))) else { return false };
                let Some(b) = read_range(&f, 0, rec.pdf_len - at) else { return false };
                bytes.extend_from_slice(&b);
            }
            let Some(p) = PdfOut::reopen(path, &bytes) else { return false };
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
        self.host.checkpoint.page.resumed_at = rec.page as i32;
        if platform::env_var(b"RSTEX_PAGE_VERIFY").is_some() {
            let records = c.recs[n..].iter().filter(|r| !r.sums.is_empty()).map(|r| (r.page as i32, r.sums.clone())).collect();
            self.host.checkpoint.page.verify = Some(Verify { records, ..Default::default() });
        } else if let Some((log, pdf, own)) = go_on {
            return self.pages_go_on(&c, log, pdf, own);
        }
        true
    }
}

/// Enough of a map of the prefix to name what differs.
pub(super) fn field_offsets() -> Vec<(&'static str, usize)> {
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
