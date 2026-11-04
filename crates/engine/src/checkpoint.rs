//! The preamble checkpoint, DESIGN.md section 7: the whole image at the moment `\document` is about to run, with the
//! ordered log of what the run asked the outside world, so that a later run can skip the preamble if every answer is
//! still the same.
//!
//! # Where it is taken
//!
//! A restore has no Rust stack to come back to, so it can only resume at a place whose stack it can rebuild:
//! `main_control` at its big switch, inside its `get_x_token`, with `macro_call` next.  That is the usual way to reach
//! `\document`, by the `\csname` in `\begin`.  Any other way, say `\count0=\document`, takes no checkpoint.
//!
//! It is not taken at `\document` itself, but where that macro first comes to `\@input`, for the `.aux` file: the trigger moves on to `\@input`
//! (`checkpoint_late_trigger`).  What `\document` does before that is the same in each run, and it loads l3backend, which is a quarter of what a restored
//! hello had left to do.  Nothing about the position depends on which macro it is, and the log goes on until then, so a hook that does something new meanwhile is seen as in the preamble.
//!
//! The trigger costs the token loop nothing.  **An armed control sequence** has `CK_ARMED` added to its `eq_type`, so that
//! `get_next`'s test `cur_cmd>=outer_call` sends it to `check_outer_validity`, a path that exists anyway, and that sees every
//! fetch of the control sequence from a file or a token list.  There `checkpoint_trap` puts `cur_cmd` right, and nothing
//! after it can tell.  Only what reads `eqtb` without going through `get_next` has to know: `eq_destroy`, `show_eqtb` and
//! `macro_call`'s `long_state`, and `eq_define`, whose test for a self-assignment would miss one and go on to change the word.
//!
//! # The clock
//!
//! Every LaTeX run reads the clock in `\everyjob`, for expl3's `\c_sys_*` constants, and pgfmath reads it for a seed.  Those two
//! sites are what is whitelisted, by the exact sequence of reads and the exact value that lands in each target (`checkpoint_date_read`,
//! `checkpoint_after_prefixed`), and a restore patches the targets.  Whitelisting a read is only sound if nothing else
//! sees the value: so once made, each target is armed, and fetching it means a read of the clock, as `\time` itself
//! does.  A copy made in the preamble therefore costs the checkpoint, and a mention in a macro body or a skipped text does not.
//!
//! # The file
//!
//! ```text
//! header   HEADER_LEN bytes: magic, lengths and hashes, its own hash last
//! meta     config, the log, what the run wrote, the open files, the spans of the image
//! image    the spans: the live range of each POD section, less long runs of zeros, as raw bytes; a long one at a place in the file that it can be mapped from
//! ```
//!
//! A third of a format's bytes are zero, in `font_info`: expl3 reserves `\fontdimen`s by the hundred thousand.  A restore starts from
//! a new reservation, which is zero, so those are neither read nor summed nor written.
//!
//! A restore reads the meta part, answers the log again, and only then touches the image.  This file is part of Host and
//! World, so it may use `std::fs`.
//!
//! # Whether to write one
//!
//! Writing costs a small document more than it will ever get back from being compiled once, and costs a large one next to nothing.  So a run writes at once if its preamble read
//! enough to have taken several times what writing takes, and otherwise only if the same preamble has been seen before, which the table `seen` remembers in a few bytes:
//! see `checkpoint_worth_writing`.  `RSTEX_CHECKPOINT_POLICY` overrides that: `always`, what the test tools use, or `second`.
//!
//! # What keeps the cache small
//!
//! A checkpoint is pruned when another is published, the only moment the directory grows but for a chain of page records, which has its own moment: `cache_prune`.  `seen` has a fixed size, and what lands on an occupied slot replaces it.
#![forbid(unsafe_code)]
#![deny(unreachable_patterns, unused_variables)]
#![allow(non_snake_case, unused_mut, unused_imports, dead_code)]

use crate::access::*;
use crate::consts::*;
use crate::host::*;
use crate::platform::{self, FileStat};
use crate::raw::*;
use crate::state::*;
use crate::files::MONTHS;
use std::io::{Read, Seek, Write};

pub mod pages;

/// Added to an `eq_type` to arm a control sequence.  All commands are below it.
pub const CK_ARMED: i32 = 0x80;
const _: () = assert!(DATA < CK_ARMED);

const FILE_MAGIC: u64 = 0x334b_4358_4554_5352; // "RSTEXCK3"
/// A run of zero pages shorter than this stays in the file: a read per span is not free.
const ZERO_RUN: usize = 16 * 4096;
const HEADER_LEN: usize = 56;
/// What the cache directory may hold, in all.  The oldest files go first.
const CACHE_BOUND: u64 = 1 << 30;
/// A checkpoint not used for this long goes, however much room there is.  A use marks it, once a day at most.
const CACHE_MAX_AGE: u64 = 30 * 86400;
/// A temporary file this old belongs to a run that died.
const STALE_TMP: u64 = 86400;
const CACHE_DIR: &[u8] = b"/checkpoints";
/// The table of what has been seen: [`SEEN_SLOTS`] pairs of words, by the hash of the key.  A slot taken by another document only costs a checkpoint written later, or a sum done again.
const SEEN_FILE: &[u8] = b"/seen";
const SEEN_SLOTS: u64 = 1 << 12;
/// A stat younger than this might not show a change made just after it: as `crates/kpse/src/index.rs`.
const RACY_NS: i64 = 2_000_000_000;
/// A preamble that read this much is worth a checkpoint at once.  Bytes and not the clock, which on a busy machine says ten times what it says on a quiet one, so that a run would decide by its
/// neighbours.  What a preamble costs goes nearly with what it reads, and one that computes a lot from a little is only written a run later.  DESIGN.md section 16 has what that comes to.
const FIRST_SIGHT_BYTES: u64 = 200_000;

const PENDING_DOCUMENT: i32 = 1;
const PENDING_SYS: i32 = 2;
const PENDING_SEED: i32 = 4;
/// The library of tikz-cd is loading: `crate::native`.
pub const PENDING_NATIVE: i32 = 8;

/// The five reads of expl3's `\everyjob`, in order, then `\pdfcreationdate`.
const SYS_NAMES: [&[u8]; 6] =
    [b"c_sys_minute_int", b"c_sys_hour_int", b"c_sys_day_int", b"c_sys_month_int", b"c_sys_year_int", b"c_sys_timestamp_str"];
const SYS_READS: [i32; 5] = [INT_BASE + TIME_CODE, INT_BASE + TIME_CODE, INT_BASE + DAY_CODE, INT_BASE + MONTH_CODE, INT_BASE + YEAR_CODE];
const SYS_DONE: i32 = 6;
/// `31 2026 23:59` and two spaces more: what §536 counts of a date, as the month goes out by `wlog`.
const LONGEST_DATE: i32 = 14;
const SYS_OFF: i32 = -1;
const SEED_FILE: &[u8] = b"/pgfmathfunctions.random.code.tex";
const SEED_NAME: &[u8] = b"pgfmath@rnd@z";
const SEED_DONE: i32 = 3;

/// What the key holds beyond the command line: the variables that `main_body` asks for, whose values are
/// in the image, and those that make a search path or decide what a primitive answers.
const CONFIG_VARS: &[&[u8]] = &[
    b"main_memory", b"extra_mem_top", b"extra_mem_bot", b"pool_size", b"string_vacancies", b"pool_free", b"max_strings",
    b"strings_free", b"font_mem_size", b"font_max", b"trie_size", b"hyph_size", b"buf_size", b"nest_size", b"max_in_open",
    b"param_size", b"save_size", b"stack_size", b"dvi_buf_size", b"error_line", b"half_error_line", b"max_print_line",
    b"hash_extra", b"expand_depth", b"pk_dpi", b"openout_any", b"openin_any", b"TEXMFOUTPUT", b"shell_escape_commands",
    b"texmf_casefold_search",
];
const CONFIG_ENV: &[&[u8]] = &[
    b"TEXINPUTS", b"TEXFONTS", b"TEXFORMATS", b"TFMFONTS", b"VFFONTS", b"T1FONTS", b"ENCFONTS", b"TEXFONTMAPS", b"TEXPSHEADERS",
    b"TEXMFCNF", b"TEXMFHOME", b"TEXMFVAR", b"TEXMFCONFIG", b"TEXMFLOCAL", b"TEXMF", b"TEXMF_OUTPUT_DIRECTORY", b"RSTEX_TEXMF",
    b"FORCE_SOURCE_DATE", b"TZ",
];

crate::pod_struct! {
    /// POD state of the checkpoint, embedded in `image.rs`'s `Vars`.  It comes back with the image.
    pub struct Vars {
        /// `main_control` is in the `get_x_token` of its big switch.
        pub at_big_switch: bool,
        /// A page has gone out, and a record may be taken at the next big switch: `pages`.
        pub page_due: bool,
        /// `scan_toks` of a `\def` is expanding, so a fetch of something armed is a read.
        pub edef: bool,
        /// A restore is about to enter `main_control`: `\everyjob` has run.
        pub resumed: bool,
        /// `\pdfsetrandomseed` ran: a restore keeps the generator.
        pub seed_pinned: bool,
        /// The preamble read the date: the checkpoint is good for this day.
        pub date_read: bool,
        /// Bits, one for each of `\time`, `\day`, `\month`, `\year`, that the preamble assigned.
        pub date_assigned: i32,
        pub pending: i32,
        /// The control sequence whose fetch at the big switch is the moment: `\\document`, and then `\\@input`.
        pub document_cs: i32,
        /// `\\@input`, until the trigger has moved to it.
        pub late_cs: i32,
        pub sys_cs: A<i32, 6>,
        /// The next of the six steps of expl3's `\everyjob`, `SYS_DONE`, or `SYS_OFF`.
        pub sys_state: i32,
        /// What the step in progress must leave in its target.
        pub sys_value: i32,
        pub seed_cs: i32,
        /// 0, or how far pgfmath's seed has got: `\time`, `\year`, the `\xdef`.
        pub seed_state: i32,
        pub seed_value: i32,
        /// Where the date is in the log's first line.
        pub log_date_start: i64,
        pub log_date_end: i64,
    }
}

/// What the run has to remember, outside the image, for a checkpoint it may yet take.
#[derive(Default)]
pub struct HostState {
    /// Where the checkpoint goes, while this run may write one.
    key: Option<Key>,
    /// The worker that writes it.
    worker: Option<std::thread::JoinHandle<()>>,
    /// Bytes the terminal has been sent, while recording.
    pub term_journal: Option<Vec<u8>>,
    /// Say on stderr what the checkpoint did: `RSTEX_CHECKPOINT_TRACE`.
    trace: bool,
    /// Page records.
    pub page: pages::PageHost,
}

/// `RSTEX_CHECKPOINT_POLICY`: see the top of this file.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Policy {
    Auto,
    Always,
    Second,
}

fn checkpoint_policy() -> Policy {
    match platform::env_var(b"RSTEX_CHECKPOINT_POLICY").as_deref() {
        Some(b"always") => Policy::Always,
        Some(b"second") => Policy::Second,
        _ => Policy::Auto,
    }
}

impl HostState {
    /// `uexit`: a checkpoint in progress is published before the process ends.
    pub fn join(&mut self) {
        if let Some(w) = self.worker.take() {
            let _ = w.join();
        }
        self.pages_finish();
    }
}

// The meta part's bytes: all integers as 64 bits, little-endian as the image is, and everything with a length before it.
fn put_u64(v: &mut Vec<u8>, x: u64) {
    v.extend_from_slice(&x.to_le_bytes());
}
fn put_bytes(v: &mut Vec<u8>, b: &[u8]) {
    put_u64(v, b.len() as u64);
    v.extend_from_slice(b);
}
fn put_opt(v: &mut Vec<u8>, b: Option<&[u8]>) {
    match b {
        Some(b) => {
            v.push(1);
            put_bytes(v, b);
        }
        None => v.push(0),
    }
}
fn put_stat(v: &mut Vec<u8>, s: &Option<FileStat>) {
    match s {
        Some(s) => {
            v.push(1);
            put_u64(v, s.size);
            put_u64(v, s.mtime as u64);
            put_u64(v, s.mtime_nsec as u64);
            put_u64(v, s.change as u64);
            put_u64(v, s.inode);
            v.push(s.regular as u8);
        }
        None => v.push(0),
    }
}

/// Bytes being taken apart; `None` past the end.  The meta part's hash has been checked, so this only meets what this
/// build wrote, and a `None` means a bug at worst: a cold run.
struct In<'a> {
    b: &'a [u8],
    p: usize,
}

impl<'a> In<'a> {
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let e = self.p.checked_add(n)?;
        let s = self.b.get(self.p..e)?;
        self.p = e;
        Some(s)
    }
    fn u8(&mut self) -> Option<u8> {
        Some(self.take(1)?[0])
    }
    fn u64(&mut self) -> Option<u64> {
        Some(u64::from_le_bytes(self.take(8)?.try_into().ok()?))
    }
    fn i32(&mut self) -> Option<i32> {
        Some(self.u64()? as i32)
    }
    fn bytes(&mut self) -> Option<&'a [u8]> {
        let n = self.u64()? as usize;
        self.take(n)
    }
    fn vec(&mut self) -> Option<Vec<u8>> {
        Some(self.bytes()?.to_vec())
    }
    /// How many of something follow: each takes a byte at least, so a count that says more is a lie, which must not reach an allocation.
    fn count(&mut self) -> Option<usize> {
        let n = self.u64()? as usize;
        if n <= self.b.len() - self.p { Some(n) } else { None }
    }
    fn opt(&mut self) -> Option<Option<Vec<u8>>> {
        Some(if self.u8()? == 1 { Some(self.vec()?) } else { None })
    }
    fn stat(&mut self) -> Option<Option<FileStat>> {
        Some(if self.u8()? == 1 {
            Some(FileStat {
                size: self.u64()?,
                mtime: self.u64()? as i64,
                mtime_nsec: self.u64()? as i64,
                change: self.u64()? as i64,
                inode: self.u64()?,
                regular: self.u8()? == 1,
            })
        } else {
            None
        })
    }
}

impl Observation {
    fn encode(&self, v: &mut Vec<u8>) {
        match self {
            Observation::Lookup { name, format, must_exist, found } => {
                v.push(1);
                put_bytes(v, name);
                put_u64(v, *format as u64);
                v.push(*must_exist as u8);
                put_opt(v, found.as_deref());
            }
            Observation::Read { path, stat, hash, prefix, kind } => {
                v.push(2);
                put_bytes(v, path);
                put_stat(v, stat);
                put_u64(v, *hash);
                put_u64(v, prefix.map_or(u64::MAX, |p| p));
                v.push(*kind as u8);
            }
            Observation::Stat { path, stat, size_only } => {
                v.push(3);
                put_bytes(v, path);
                put_stat(v, stat);
                v.push(*size_only as u8);
            }
            Observation::Env { name, value, cnf } => {
                v.push(4);
                put_bytes(v, name);
                put_opt(v, value.as_deref());
                v.push(*cnf as u8);
            }
            Observation::Output { path } => {
                v.push(5);
                put_bytes(v, path);
            }
        }
    }

    /// What makes this the same preamble as last time, whatever a stat says: `touch` changes no preamble, and an edit of the body changes the stat of the main file.
    fn identity(&self, v: &mut Vec<u8>) {
        match self {
            Observation::Read { path, hash, prefix, .. } => {
                put_bytes(v, path);
                put_u64(v, *hash);
                put_u64(v, prefix.map_or(u64::MAX, |p| p));
            }
            Observation::Stat { path, .. } => put_bytes(v, path),
            _ => self.encode(v),
        }
    }

    fn decode(r: &mut In) -> Option<Observation> {
        Some(match r.u8()? {
            1 => Observation::Lookup { name: r.vec()?, format: r.i32()?, must_exist: r.u8()? == 1, found: r.opt()? },
            2 => {
                let path = r.vec()?;
                let stat = r.stat()?;
                let hash = r.u64()?;
                let p = r.u64()?;
                let kind = match r.u8()? {
                    0 => ReadKind::Project,
                    1 => ReadKind::Tds,
                    _ => ReadKind::Format,
                };
                Observation::Read { path, stat, hash, prefix: if p == u64::MAX { None } else { Some(p) }, kind }
            }
            3 => Observation::Stat { path: r.vec()?, stat: r.stat()?, size_only: r.u8()? == 1 },
            4 => Observation::Env { name: r.vec()?, value: r.opt()?, cnf: r.u8()? == 1 },
            5 => Observation::Output { path: r.vec()? },
            _ => return None,
        })
    }
}

/// XXH64.  The checkpoint's own integrity and the hashes of the files read: nothing here is against an attacker.
pub fn xxh64(data: &[u8], seed: u64) -> u64 {
    const P1: u64 = 0x9E3779B185EBCA87;
    const P2: u64 = 0xC2B2AE3D27D4EB4F;
    const P3: u64 = 0x165667B19E3779F9;
    const P4: u64 = 0x85EBCA77C2B2AE63;
    const P5: u64 = 0x27D4EB2F165667C5;
    #[inline(always)]
    fn round(acc: u64, w: u64) -> u64 {
        acc.wrapping_add(w.wrapping_mul(P2)).rotate_left(31).wrapping_mul(P1)
    }
    #[inline(always)]
    fn word(b: &[u8]) -> u64 {
        u64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]])
    }
    #[inline(always)]
    fn merge(h: u64, v: u64) -> u64 {
        (h ^ round(0, v)).wrapping_mul(P1).wrapping_add(P4)
    }
    let mut h;
    let mut stripes = data.chunks_exact(32);
    if data.len() >= 32 {
        let (mut a, mut b, mut c, mut d) =
            (seed.wrapping_add(P1).wrapping_add(P2), seed.wrapping_add(P2), seed, seed.wrapping_sub(P1));
        for s in &mut stripes {
            a = round(a, word(&s[0..8]));
            b = round(b, word(&s[8..16]));
            c = round(c, word(&s[16..24]));
            d = round(d, word(&s[24..32]));
        }
        h = a.rotate_left(1).wrapping_add(b.rotate_left(7)).wrapping_add(c.rotate_left(12)).wrapping_add(d.rotate_left(18));
        h = merge(merge(merge(merge(h, a), b), c), d);
    } else {
        h = seed.wrapping_add(P5);
    }
    h = h.wrapping_add(data.len() as u64);
    let rest = stripes.remainder();
    let mut words = rest.chunks_exact(8);
    for w in &mut words {
        h = (h ^ round(0, word(w))).rotate_left(27).wrapping_mul(P1).wrapping_add(P4);
    }
    let rest = words.remainder();
    let mut fours = rest.chunks_exact(4);
    for w in &mut fours {
        let x = u32::from_le_bytes([w[0], w[1], w[2], w[3]]) as u64;
        h = (h ^ x.wrapping_mul(P1)).rotate_left(23).wrapping_mul(P2).wrapping_add(P3);
    }
    for &b in fours.remainder() {
        h = (h ^ (b as u64).wrapping_mul(P5)).rotate_left(11).wrapping_mul(P1);
    }
    h ^= h >> 33;
    h = h.wrapping_mul(P2);
    h ^= h >> 29;
    h = h.wrapping_mul(P3);
    h ^ (h >> 32)
}

/// The image's own check, on every restore: a Fletcher sum in four lanes of 64 bits, which takes a third of XXH64's instructions per byte, and
/// sees a flipped bit, a zeroed page and two pages swapped.  Only ever against accidents.
pub fn image_sum(data: &[u8], seed: u64) -> u64 {
    let mut a = [seed, !seed, seed.rotate_left(21), seed.rotate_left(42)];
    let mut b = [0u64; 4];
    let mut stripes = data.chunks_exact(32);
    for s in &mut stripes {
        for k in 0..4 {
            let w = u64::from_le_bytes([s[8 * k], s[8 * k + 1], s[8 * k + 2], s[8 * k + 3], s[8 * k + 4], s[8 * k + 5], s[8 * k + 6], s[8 * k + 7]]);
            a[k] = a[k].wrapping_add(w);
        }
        for k in 0..4 {
            b[k] = b[k].wrapping_add(a[k]);
        }
    }
    let mut out = [0u8; 72];
    for k in 0..4 {
        out[8 * k..8 * k + 8].copy_from_slice(&a[k].to_le_bytes());
        out[32 + 8 * k..40 + 8 * k].copy_from_slice(&b[k].to_le_bytes());
    }
    out[64..72].copy_from_slice(&xxh64(stripes.remainder(), data.len() as u64).to_le_bytes());
    xxh64(&out, seed)
}

/// A path as the log compares it: `./gen.tex` is `gen.tex`.
fn path_norm(p: &[u8]) -> &[u8] {
    p.strip_prefix(b"./").unwrap_or(p)
}

impl World {
    /// `read_unrecorded`'s entry in the log.  A file is hashed where it is in hand; a format, which is large and written once,
    /// goes by its stat alone.
    pub(crate) fn checkpoint_note_read(&mut self, path: &[u8], stat: Option<FileStat>, data: Option<&[u8]>, is_format: bool) {
        let stat = if data.is_some() { stat } else { None };
        // Asking it again would take from it what the rest of the run has to read.
        if stat.as_ref().is_some_and(|s| !s.regular) {
            self.mark_taint_because(Taint::Hard, "a read of what is not a regular file");
        }
        let n = path_norm(path);
        if self.open_outputs.iter().any(|o| path_norm(o) == n) {
            self.mark_taint_because(Taint::Hard, "a file read while the run writes it");
        }
        let (hash, kind) = match data {
            Some(_) if is_format => (0, ReadKind::Format),
            Some(d) => {
                (xxh64(d, 0), if Self::checkpoint_stat_is_settled(path, &stat) { ReadKind::Tds } else { ReadKind::Project })
            }
            None => (0, ReadKind::Project),
        };
        self.log.push(Observation::Read { path: path.to_vec(), stat, hash, prefix: None, kind });
    }

    /// True for a file of a tree whose stamp is old enough to show a later change: a stamp this young might not, in the same tick of the file system's clock, as `crates/kpse/src/index.rs` has it.
    pub(crate) fn checkpoint_stat_is_settled(path: &[u8], stat: &Option<FileStat>) -> bool {
        let settled = stat.as_ref().is_some_and(|s| ((s.mtime as i128 * 1_000_000_000 + s.mtime_nsec as i128).max(s.change as i128) + 2_000_000_000) < rstex_kpse::now_ns() as i128);
        settled && platform::is_absolute(path) && rstex_kpse::with_global(|k| k.in_database_tree(path))
    }

    /// `\openout`: the journal of the stream is on, and from now on a read of this file would see a part of it.
    pub(crate) fn checkpoint_note_open_out(&mut self, path: &[u8]) {
        self.open_outputs.push(path.to_vec());
        self.written.push(path.to_vec());
    }

    /// The stream is closed, and what it held is for the log: a later read of it is answered by the journal.
    pub(crate) fn checkpoint_note_close_out(&mut self, path: &[u8], bytes: Vec<u8>) {
        if let Some(k) = self.open_outputs.iter().position(|o| o == path) {
            self.open_outputs.remove(k);
        }
        self.log.push(Observation::Output { path: path.to_vec() });
        self.closed_outputs.push((path.to_vec(), bytes));
    }

    /// True if the run has written this file: its stat is its own doing.
    pub(crate) fn checkpoint_is_written(&self, path: &[u8]) -> bool {
        let n = path_norm(path);
        self.written.iter().any(|o| path_norm(o) == n)
    }

    /// Stops recording, and frees what it held.
    fn checkpoint_stop_recording(&mut self) {
        self.recording = false;
        self.journal_recorder = false;
        self.log = Vec::new();
        self.closed_outputs = Vec::new();
        self.open_outputs = Vec::new();
        self.written = Vec::new();
        self.recorder_drop_journal();
    }
}

/// A checkpoint's identity: what a later run has to agree on before it even opens one, and where the file is.
pub struct Key {
    config: Vec<u8>,
    /// Of `config`: the file's name, and the slot in `seen`.
    hash: u64,
    dir: Vec<u8>,
    path: Vec<u8>,
}

impl Key {
    /// What `seen` holds for this key: the identity of the preamble last seen, and the seal of a checkpoint that has passed its sum.  Zeros if nothing, as for a file that is not there.
    fn seen(&self) -> [u64; 2] {
        let mut b = [0u8; 16];
        let path = platform::path_from_bytes(&[&self.dir[..], SEEN_FILE].concat());
        if !rstex_kpse::open_private(&path).is_some_and(|f| platform::read_exact_at(&f, (self.hash % SEEN_SLOTS) * 16, &mut b)) {
            return [0, 0];
        }
        [u64::from_le_bytes(b[..8].try_into().unwrap_or_default()), u64::from_le_bytes(b[8..].try_into().unwrap_or_default())]
    }

    /// One of the two words of its slot.  Failure is harmless: the next run writes a checkpoint later, or does a sum again.
    fn set_seen(&self, word: u64, value: u64) {
        let _ = std::fs::create_dir_all(platform::path_from_bytes(&self.dir));
        let path = platform::path_from_bytes(&[&self.dir[..], SEEN_FILE].concat());
        let Some(mut f) = rstex_kpse::open_private_to_update(&path) else { return };
        let mut write = || -> std::io::Result<()> {
            // Whole from the start, and with holes where the system has them, so that any slot can be read.
            if f.metadata()?.len() != SEEN_SLOTS * 16 {
                f.set_len(SEEN_SLOTS * 16)?;
            }
            f.seek(std::io::SeekFrom::Start((self.hash % SEEN_SLOTS) * 16 + word * 8))?;
            f.write_all(&value.to_le_bytes())
        };
        let _ = write();
    }
}

/// What says that a checkpoint, with this stat and this sum, has been read back whole: only a change to the file, which the stat would show, could undo that.
fn seal_of(stat: &Option<FileStat>, image_hash: u64) -> u64 {
    let mut v = Vec::with_capacity(64);
    put_stat(&mut v, stat);
    xxh64(&v, image_hash) | 1
}

/// What came of looking for a checkpoint.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Restored {
    /// No checkpoint, or one that does not hold.
    Cold,
    /// Cold too, but the log was answered, so kpathsea has seen lookups the run has not made yet.
    ColdAfterLookups,
    /// Cold too, but the image was overwritten: it has to be a new one.
    ColdImageDirty,
    /// The state is the one at `\document`.  `checkpoint_resume` goes on from there.
    Yes,
}

/// The meta part.
struct Meta {
    config: Vec<u8>,
    log: Vec<Observation>,
    paths: Vec<(i32, Vec<u8>)>,
    databases: Vec<(Vec<u8>, Option<FileStat>)>,
    /// YYYYMMDD, or 0 if the preamble read no date.
    date_key: i32,
    /// `(slot, path, position)`.
    inputs: Vec<(i32, Vec<u8>, u64)>,
    /// Files the run wrote and closed, in order.
    closed: Vec<(Vec<u8>, Vec<u8>)>,
    /// `(slot, path, bytes)`, the log among them.
    outputs: Vec<(i32, Vec<u8>, Vec<u8>)>,
    term: Vec<u8>,
    recorder: Option<(Vec<u8>, Vec<u8>)>,
    /// The PDF file's name and what it holds so far: its header, and any `\\immediate` object.
    pdf: Option<(Vec<u8>, Vec<u8>)>,
    full_name_of_file: Option<Vec<u8>>,
    /// SyncTeX's file if it is open, by its name and the text so far: no member comes before a checkpoint.
    synctex: Option<(Vec<u8>, Vec<u8>)>,
    /// SyncTeX's name of the first file, while it waits for the file to open.
    synctex_root: Option<Vec<u8>>,
    /// `(start, end)` in the image of each span.  The worker adds these to what the engine encoded.
    spans: Vec<(u64, u64)>,
}

impl Meta {
    fn encode(&self) -> Vec<u8> {
        let mut v = Vec::with_capacity(1 << 16);
        put_bytes(&mut v, &self.config);
        put_u64(&mut v, self.log.len() as u64);
        for o in &self.log {
            o.encode(&mut v);
        }
        put_u64(&mut v, self.paths.len() as u64);
        for (f, p) in &self.paths {
            put_u64(&mut v, *f as u64);
            put_bytes(&mut v, p);
        }
        put_u64(&mut v, self.databases.len() as u64);
        for (p, s) in &self.databases {
            put_bytes(&mut v, p);
            put_stat(&mut v, s);
        }
        put_u64(&mut v, self.date_key as u64);
        put_u64(&mut v, self.inputs.len() as u64);
        for (s, p, pos) in &self.inputs {
            put_u64(&mut v, *s as u64);
            put_bytes(&mut v, p);
            put_u64(&mut v, *pos);
        }
        put_u64(&mut v, self.closed.len() as u64);
        for (p, b) in &self.closed {
            put_bytes(&mut v, p);
            put_bytes(&mut v, b);
        }
        put_u64(&mut v, self.outputs.len() as u64);
        for (s, p, b) in &self.outputs {
            put_u64(&mut v, *s as u64);
            put_bytes(&mut v, p);
            put_bytes(&mut v, b);
        }
        put_bytes(&mut v, &self.term);
        for x in [&self.recorder, &self.pdf, &self.synctex] {
            match x {
                Some((n, b)) => {
                    v.push(1);
                    put_bytes(&mut v, n);
                    put_bytes(&mut v, b);
                }
                None => v.push(0),
            }
        }
        put_opt(&mut v, self.full_name_of_file.as_deref());
        put_opt(&mut v, self.synctex_root.as_deref());
        v
    }

    fn decode(b: &[u8]) -> Option<Meta> {
        let mut r = In { b, p: 0 };
        let config = r.vec()?;
        let n = r.count()?;
        let mut log = Vec::with_capacity(n);
        for _ in 0..n {
            log.push(Observation::decode(&mut r)?);
        }
        let n = r.count()?;
        let mut paths = Vec::with_capacity(n);
        for _ in 0..n {
            paths.push((r.i32()?, r.vec()?));
        }
        let n = r.count()?;
        let mut databases = Vec::with_capacity(n);
        for _ in 0..n {
            databases.push((r.vec()?, r.stat()?));
        }
        let date_key = r.i32()?;
        let n = r.count()?;
        let mut inputs = Vec::with_capacity(n);
        for _ in 0..n {
            inputs.push((r.i32()?, r.vec()?, r.u64()?));
        }
        let n = r.count()?;
        let mut closed = Vec::with_capacity(n);
        for _ in 0..n {
            closed.push((r.vec()?, r.vec()?));
        }
        let n = r.count()?;
        let mut outputs = Vec::with_capacity(n);
        for _ in 0..n {
            outputs.push((r.i32()?, r.vec()?, r.vec()?));
        }
        let term = r.vec()?;
        let recorder = if r.u8()? == 1 { Some((r.vec()?, r.vec()?)) } else { None };
        let pdf = if r.u8()? == 1 { Some((r.vec()?, r.vec()?)) } else { None };
        let synctex = if r.u8()? == 1 { Some((r.vec()?, r.vec()?)) } else { None };
        let full_name_of_file = r.opt()?;
        let synctex_root = r.opt()?;
        let n = r.count()?;
        let mut spans = Vec::with_capacity(n);
        for _ in 0..n {
            spans.push((r.u64()?, r.u64()?));
        }
        Some(Meta { config, log, paths, databases, date_key, inputs, closed, outputs, term, recorder, pdf, full_name_of_file, synctex, synctex_root, spans })
    }
}

/// The sections a checkpoint holds: all of them, less INITEX's scratch arrays that a run from a format never touches.
fn ck_section(s: &PodSection) -> bool {
    !matches!(s.name, "trie_op_hash" | "trie_op_lang" | "trie_op_val")
}

/// `hhmm` and the rest, as §562 prints them.
fn log_date(day: i32, month: i32, year: i32, time: i32) -> Vec<u8> {
    let mut v = format!("{} ", day).into_bytes();
    v.extend_from_slice(&MONTHS[3 * month as usize - 2..=3 * month as usize]);
    v.extend_from_slice(format!(" {} {:02}:{:02}", year, (time / 60) % 100, (time % 60) % 100).as_bytes());
    v
}

/// What is kept of the checkpoints, when one has been published or a chain of page records has grown: a run that died leaves its temporary file for a day, a checkpoint not used for a month goes, and of the
/// others the one that has not been used for longest goes first, down to the bound.  A chain counts with the rest, and has a stamp of its own.  It goes with its checkpoint, and alone if it is the oldest or has no checkpoint;
/// one that a run holds stays.  This reads the directory, and those of the chains, so it is for when a file has just been written, not for every run.
fn cache_prune(dir: &[u8], bound: u64) {
    let Ok(rd) = std::fs::read_dir(platform::path_from_bytes(dir)) else { return };
    let now = std::time::SystemTime::now();
    let mut files = Vec::new();
    let mut total = 0u64;
    let is_old = |t: Option<std::time::SystemTime>, limit: u64| t.and_then(|t| now.duration_since(t).ok()).map_or(0, |d| d.as_secs()) > limit;
    for e in rd.flatten() {
        let name = e.file_name();
        let path = e.path();
        let Ok(m) = e.metadata() else { continue };
        if m.is_dir() {
            if path.extension().is_some_and(|x| x == "pg") {
                let (n, stamp) = pages::chain_size(&path);
                if (is_old(stamp, CACHE_MAX_AGE) || !path.with_extension("ck").exists()) && pages::chain_remove(&path) {
                    continue;
                }
                total += n;
                files.push((stamp, n, path));
            }
            continue;
        }
        let is_tmp = path.extension().is_some_and(|x| x == "tmp");
        if !m.is_file() || name.as_encoded_bytes() == &SEEN_FILE[1..] {
            continue;
        }
        if is_old(m.modified().ok(), if is_tmp { STALE_TMP } else { CACHE_MAX_AGE }) && std::fs::remove_file(&path).is_ok() {
            continue;
        }
        total += m.len();
        files.push((m.modified().ok(), m.len(), path));
    }
    if total <= bound {
        return;
    }
    files.sort_by_key(|f| f.0);
    for k in 0..files.len() {
        if total <= bound {
            break;
        }
        let (n, p) = (files[k].1, files[k].2.clone());
        if p.extension().is_some_and(|x| x == "pg") {
            if p.exists() && pages::chain_remove(&p) {
                total -= n;
            }
        } else if std::fs::remove_file(&p).is_ok() {
            total -= n;
            // Nothing can use its chain now.
            let chain = p.with_extension("pg");
            if let Some(c) = files.iter().find(|f| f.2 == chain) {
                let n = c.1;
                if pages::chain_remove(&chain) {
                    total -= n;
                }
            }
        }
    }
}

/// True if these bytes are all zero.
fn all_zero(b: &[u8]) -> bool {
    b.chunks(64).all(|c| c.iter().fold(0, |x, &y| x | y) == 0)
}

/// `(start, end)` in `b`, which holds a section, of what is left without its long runs of zero pages.
fn nonzero_spans(b: &[u8]) -> Vec<(usize, usize)> {
    let mut spans = Vec::new();
    let mut start = 0;
    let mut k = 0;
    while k + ZERO_RUN <= b.len() {
        if !all_zero(&b[k..k + 4096]) {
            k += 4096;
            continue;
        }
        let mut e = k + 4096;
        while e + 4096 <= b.len() && all_zero(&b[e..e + 4096]) {
            e += 4096;
        }
        if e - k >= ZERO_RUN {
            if k > start {
                spans.push((start, k));
            }
            start = e;
        }
        k = e;
    }
    if b.len() > start {
        spans.push((start, b.len()));
    }
    spans
}

/// A checkpoint on its way to the disk, in the worker.
struct WriteJob {
    dir: Vec<u8>,
    path: Vec<u8>,
    meta: Vec<u8>,
    image: platform::HugeBuf,
    /// `(start in the image, bytes)` of each section, in the order `image` has them.
    sections: Vec<(usize, usize)>,
    bound: u64,
}

impl WriteJob {
    fn run(mut self) {
        let _ = std::fs::create_dir_all(platform::path_from_bytes(&self.dir));
        let img = self.image.as_slice();
        let mut spans: Vec<(usize, usize)> = Vec::with_capacity(self.sections.len());
        let mut in_buf: Vec<(usize, usize)> = Vec::with_capacity(self.sections.len());
        let mut at = 0;
        let mut image_hash = 0;
        for (i, &(start, n)) in self.sections.iter().enumerate() {
            let b = &img[at..at + n];
            // The prefix was not zero when the run began: the driver has been there.
            let parts = if i == 0 { vec![(0, n)] } else { nonzero_spans(b) };
            for (x, y) in parts {
                image_hash = image_sum(&b[x..y], image_hash);
                spans.push((start + x, start + y));
                in_buf.push((at + x, at + y));
            }
            at += n;
        }
        put_u64(&mut self.meta, spans.len() as u64);
        for (x, y) in &spans {
            put_u64(&mut self.meta, *x as u64);
            put_u64(&mut self.meta, *y as u64);
        }
        let meta_hash = xxh64(&self.meta, 0);
        // Where each span goes in the file: the reader works it out in the same way.
        let start = (HEADER_LEN + self.meta.len()) as u64;
        let mut place = start;
        let mut pads = Vec::with_capacity(spans.len());
        for &(x, y) in &spans {
            let pad = crate::image::image_padding(place, x, y - x);
            pads.push(pad as usize);
            place += pad + (y - x) as u64;
        }
        let image_len = place - start;
        if !rstex_kpse::may_write_file_of(place) {
            return;
        }
        let mut h = Vec::with_capacity(HEADER_LEN);
        put_u64(&mut h, FILE_MAGIC);
        put_u64(&mut h, self.meta.len() as u64);
        put_u64(&mut h, image_len);
        put_u64(&mut h, meta_hash);
        put_u64(&mut h, image_hash);
        put_u64(&mut h, core::mem::size_of::<Tex>() as u64);
        let own = xxh64(&h, 0);
        put_u64(&mut h, own);
        let mut tmp = self.path.clone();
        tmp.extend_from_slice(format!(".{}.tmp", std::process::id()).as_bytes());
        let write = || -> std::io::Result<()> {
            let mut f = rstex_kpse::create_private(&platform::path_from_bytes(&tmp))?;
            f.write_all(&h)?;
            f.write_all(&self.meta)?;
            for (&(x, y), &pad) in in_buf.iter().zip(&pads) {
                f.write_all(&[0; 1 << 14][..pad])?;
                f.write_all(&img[x..y])?;
            }
            Ok(())
        };
        if write().is_ok() && platform::publish(&tmp, &self.path).is_ok() {
            cache_prune(&self.dir, self.bound);
        } else {
            let _ = std::fs::remove_file(platform::path_from_bytes(&tmp));
        }
    }
}

impl Tex {
    /// What a checkpoint for this run is called.  `cli` is everything the command line and `texmf.cnf` decided before the engine
    /// started, as the driver sees it.  `None` if there is nowhere to keep one.
    pub fn checkpoint_key(&mut self, cli: &[u8]) -> Option<Key> {
        let dir = [&platform::path_to_bytes(&rstex_kpse::cache_dir()?)[..], CACHE_DIR].concat();
        let mut c = Vec::with_capacity(1 << 12);
        // The build: the program's identity, and so what its code does with the image.
        let exe = std::env::current_exe().ok()?;
        let exe = platform::path_to_bytes(&exe);
        put_bytes(&mut c, &exe);
        put_stat(&mut c, &platform::file_stat(&exe));
        put_bytes(&mut c, crate::image::FMT_BUILD_ID.as_bytes());
        put_u64(&mut c, core::mem::size_of::<Tex>() as u64);
        let cwd = std::env::current_dir().ok()?;
        put_bytes(&mut c, &platform::path_to_bytes(&cwd));
        put_bytes(&mut c, cli);
        for v in CONFIG_ENV {
            put_opt(&mut c, platform::env_var(v).as_deref());
        }
        // Whether it is set decides between the two ways of telling time; its value is the clock, which a restore refreshes.
        c.push(platform::env_var(b"SOURCE_DATE_EPOCH").is_some() as u8);
        for v in CONFIG_VARS {
            put_opt(&mut c, rstex_kpse::var_value(v).as_deref());
        }
        let hash = xxh64(&c, 0);
        let name = format!("/{:016x}.ck", hash);
        let path = [&dir[..], name.as_bytes()].concat();
        self.host.checkpoint.trace = platform::env_var(b"RSTEX_CHECKPOINT_TRACE").is_some();
        Some(Key { config: c, hash, dir, path })
    }

    /// Says why, if asked to.
    fn checkpoint_say(&mut self, what: &str) {
        if self.host.checkpoint.trace {
            let m = format!("rstex: checkpoint: {}\n", what);
            platform::write_stderr(m.as_bytes());
        }
    }

    /// The run may write a checkpoint: from now on what it observes and writes is kept.
    pub fn checkpoint_begin_recording(&mut self, key: Key) {
        self.host.checkpoint.key = Some(key);
        self.host.checkpoint.term_journal = Some(Vec::new());
        self.host.journal_outputs = true;
        self.host.world.journal_recorder = true;
        self.host.world.recording = true;
        self.host.world.in_obs = vec![usize::MAX; self.host.files.len()];
    }

    /// A control sequence by name, or `None`.  `buffer` from `first` on is free, as for `\csname`.
    fn checkpoint_find_cs(&mut self, name: &[u8]) -> Option<i32> {
        let first = self.first;
        if first as usize + name.len() >= self.buf_size as usize {
            return None;
        }
        self.buffer.as_mut_slice()[first as usize..first as usize + name.len()].copy_from_slice(name);
        let p = self.id_lookup(first, name.len() as i32);
        if p == UNDEFINED_CONTROL_SEQUENCE { None } else { Some(p) }
    }

    fn checkpoint_arm_cs(&mut self, cs: i32) {
        let t = self.eq_type(cs);
        if t < CK_ARMED {
            self.set_eq_type(cs, t + CK_ARMED);
        }
    }

    fn checkpoint_disarm_cs(&mut self, cs: i32) {
        let t = self.eq_type(cs);
        if t >= CK_ARMED {
            self.set_eq_type(cs, t - CK_ARMED);
        }
    }

    fn checkpoint_disarm_all(&mut self) {
        let d = self.v_image.ck.document_cs;
        if d != 0 {
            self.checkpoint_disarm_cs(d);
        }
        for k in 0..6 {
            let c = self.v_image.ck.sys_cs[k];
            if c != 0 {
                self.checkpoint_disarm_cs(c);
            }
        }
        let s = self.v_image.ck.seed_cs;
        if s != 0 {
            self.checkpoint_disarm_cs(s);
        }
    }

    /// §1517, after the format: `\document` is armed, and the names of the whitelisted reads looked up.  With no `\document`
    /// this run cannot take a checkpoint, so it stops recording here.
    pub fn checkpoint_arm(&mut self) {
        if !self.host.world.recording {
            return;
        }
        let d = match self.checkpoint_find_cs(b"document") {
            Some(d) if self.eq_type(d) >= CALL && self.eq_type(d) < END_TEMPLATE => d,
            _ => {
                self.checkpoint_abandon("the format has no macro \\document");
                return;
            }
        };
        self.v_image.ck.document_cs = d;
        self.checkpoint_arm_cs(d);
        self.v_image.ck.late_cs = self.checkpoint_find_cs(b"@input").unwrap_or(0);
        self.v_image.ck.sys_state = 0;
        for k in 0..6 {
            match self.checkpoint_find_cs(SYS_NAMES[k]) {
                Some(c) => self.v_image.ck.sys_cs[k as i32] = c,
                None => {
                    self.v_image.ck.sys_state = SYS_OFF;
                    self.v_image.ck.sys_cs = A([0; 6]);
                    break;
                }
            }
        }
    }

    /// No checkpoint for this run: everything it kept goes.
    fn checkpoint_abandon(&mut self, why: &str) {
        if self.host.world.recording {
            let m = format!("no checkpoint: {}", why);
            self.checkpoint_say(&m);
        }
        self.checkpoint_stop();
    }

    fn checkpoint_stop(&mut self) {
        self.checkpoint_disarm_all();
        self.host.world.checkpoint_stop_recording();
        self.host.journal_outputs = false;
        self.host.checkpoint.term_journal = None;
        self.host.checkpoint.key = None;
        for s in self.host.files.iter_mut() {
            if let Slot::Out(o) = s {
                o.journal = None;
            }
        }
    }

    /// True if `\everyjob` is on the input stack.
    fn checkpoint_in_everyjob(&self) -> bool {
        if self.state() == TOKEN_LIST && self.token_type() == EVERY_JOB_TEXT {
            return true;
        }
        for k in 0..self.input_ptr {
            let r = self.input_stack[k];
            if r.state_field == TOKEN_LIST && r.index_field == EVERY_JOB_TEXT {
                return true;
            }
        }
        false
    }

    /// True if the next token comes straight from pgfmath's random number file in a tree: the seed's own site.
    fn checkpoint_in_seed_file(&mut self) -> bool {
        if self.state() == TOKEN_LIST || self.name() <= 17 {
            return false;
        }
        let name = self.make_cstring(self.name());
        name.ends_with(SEED_FILE) && platform::is_absolute(&name) && rstex_kpse::with_global(|k| k.in_database_tree(&name))
    }

    /// A read of `\time`, `\day`, `\month` or `\year`, from `scan_something_internal`.
    pub fn checkpoint_date_read(&mut self, m: i32) {
        if !self.host.world.recording {
            return;
        }
        let state = self.v_image.ck.sys_state;
        if (0..5).contains(&state) && self.checkpoint_in_everyjob() {
            if self.v_image.ck.pending & PENDING_SYS == 0 && SYS_READS[state as usize] == m && !self.checkpoint_tracing() {
                self.v_image.ck.sys_value = match state {
                    0 => self.time() % 60,
                    1 => self.time() / 60,
                    2 => self.day(),
                    3 => self.month(),
                    _ => self.year(),
                };
                self.v_image.ck.pending |= PENDING_SYS;
            } else {
                self.host.world.mark_taint_because(Taint::Hard, "a read of the clock in \\everyjob that is not expl3's");
            }
            return;
        }
        let seed = self.v_image.ck.seed_state;
        if m == INT_BASE + TIME_CODE && seed == 0 && !self.checkpoint_tracing() && self.checkpoint_in_seed_file() {
            self.v_image.ck.seed_state = 1;
            return;
        }
        if seed == 1 {
            if m == INT_BASE + YEAR_CODE && !self.checkpoint_tracing() && self.checkpoint_in_seed_file() {
                let v = self.time().wrapping_mul(self.year());
                self.v_image.ck.seed_value = if v == 0 { 1 } else { v };
                self.v_image.ck.seed_state = 2;
                self.v_image.ck.pending |= PENDING_SEED;
            } else {
                self.host.world.mark_taint_because(Taint::Hard, "pgfmath's seed file read the clock in an unknown way");
            }
            return;
        }
        if m == INT_BASE + TIME_CODE {
            self.host.world.mark_taint_because(Taint::Hard, "\\time was read");
        } else {
            self.v_image.ck.date_read = true;
        }
    }

    /// `\pdfcreationdate`: expl3's last step, or a hard taint.
    pub fn checkpoint_creationdate_read(&mut self) {
        if !self.host.world.recording {
            return;
        }
        if self.v_image.ck.sys_state == 5 && self.v_image.ck.pending & PENDING_SYS == 0 && !self.checkpoint_tracing() && self.checkpoint_in_everyjob() {
            self.v_image.ck.pending |= PENDING_SYS;
        } else {
            self.host.world.mark_taint_because(Taint::Hard, "\\pdfcreationdate was read");
        }
    }

    /// An assignment to the integer parameter or other location `p`, from the two places that can assign `\time` and the like.
    #[inline(always)]
    pub fn checkpoint_date_assigned(&mut self, p: i32) {
        if p >= INT_BASE + TIME_CODE && p <= INT_BASE + YEAR_CODE {
            self.v_image.ck.date_assigned |= 1 << (p - (INT_BASE + TIME_CODE));
        }
    }

    /// `show_eqtb` is about to write the value of the location `n` in the log, which is a read, of the clock if it comes from there: `\\tracingassigns`, `\\tracingrestores`.
    pub fn checkpoint_shown(&mut self, n: i32) {
        if !self.host.world.recording {
            return;
        }
        if n >= INT_BASE + TIME_CODE && n <= INT_BASE + YEAR_CODE {
            self.checkpoint_date_read(n);
        } else if (n < GLUE_BASE || n > EQTB_SIZE) && self.eq_type(n) & CK_ARMED != 0 && n != self.v_image.ck.document_cs {
            self.host.world.mark_taint_because(Taint::Hard, "a value made from the clock was shown in the log");
        }
    }

    /// True if what `\\everyjob` or pgfmath's seed does with the clock would show in the log.  Patching the image does not reach that.
    fn checkpoint_tracing(&self) -> bool {
        self.tracing_macros() > 0 || self.tracing_commands() > 0 || self.tracing_assigns() > 0 || self.tracing_restores() > 0
    }

    /// `\\advance`, `\\multiply` or `\\divide` of the location `p`: what it leaves in `\\day` comes from what the clock put there, with no read that `scan_something_internal` sees.
    #[inline(always)]
    pub fn checkpoint_date_changed(&mut self, p: i32) {
        if p >= INT_BASE + TIME_CODE && p <= INT_BASE + YEAR_CODE {
            self.checkpoint_date_assigned(p);
            self.checkpoint_date_read(p);
        }
    }

    /// The tokens of a macro with no parameters, `(start, end)` in `toks`: `None` for anything else.
    fn checkpoint_macro_body(&self, cs: i32) -> Option<(i32, i32)> {
        let t = self.eq_type(cs) & !crate::native::ARMED_BITS;
        if t < CALL || t >= END_TEMPLATE {
            return None;
        }
        let p = self.equiv(cs);
        if self.toks[p + 1] != END_MATCH_TOKEN {
            return None;
        }
        let start = p + 2;
        let mut e = start;
        while self.toks[e] != 0 {
            e += 1;
        }
        Some((start, e))
    }

    fn checkpoint_body_is(&self, cs: i32, text: &[u8]) -> bool {
        let Some((a, b)) = self.checkpoint_macro_body(cs) else { return false };
        (b - a) as usize == text.len() && text.iter().enumerate().all(|(k, &c)| self.toks[a + k as i32] == ck_token(c))
    }

    /// The tail of `prefixed_command`, if something is awaited: an assignment may have made a whitelisted read's target,
    /// or redefined `\document`, which arms it again.
    #[cold]
    pub fn checkpoint_after_prefixed(&mut self) {
        if self.v_image.ck.pending & PENDING_NATIVE != 0 {
            self.native_after_prefixed();
        }
        if !self.host.world.recording {
            self.v_image.ck.pending &= PENDING_NATIVE;
            return;
        }
        let pending = self.v_image.ck.pending;
        if pending & PENDING_DOCUMENT != 0 {
            self.v_image.ck.pending &= !PENDING_DOCUMENT;
            let d = self.v_image.ck.document_cs;
            let t = self.eq_type(d);
            if t >= CALL && t < END_TEMPLATE {
                self.checkpoint_arm_cs(d);
            }
        }
        if pending & PENDING_SYS != 0 {
            let k = self.v_image.ck.sys_state;
            let cs = self.v_image.ck.sys_cs[k];
            let made = if k < 5 {
                matches!(self.eq_type(cs), CHAR_GIVEN | MATH_GIVEN) && self.equiv(cs) == self.v_image.ck.sys_value
            } else {
                let (s, n) = self.pdfship_start_time_str();
                self.checkpoint_body_is(cs, &s[..n])
            };
            if made {
                self.checkpoint_arm_cs(cs);
                self.v_image.ck.sys_state = k + 1;
                self.v_image.ck.pending &= !PENDING_SYS;
            }
        }
        if pending & PENDING_SEED != 0 {
            if let Some(cs) = self.checkpoint_find_cs(SEED_NAME) {
                let text = self.v_image.ck.seed_value.to_string();
                if self.checkpoint_body_is(cs, text.as_bytes()) {
                    self.v_image.ck.seed_cs = cs;
                    self.v_image.ck.seed_state = SEED_DONE;
                    self.v_image.ck.pending &= !PENDING_SEED;
                    self.checkpoint_arm_cs(cs);
                }
            }
        }
    }

    /// `check_outer_validity` met an armed control sequence: `get_next` has just fetched it.  `cur_cmd` is put right, and
    /// what comes after sees the command the control sequence really has.
    #[cold]
    #[inline(never)]
    pub fn checkpoint_trap(&mut self) {
        if self.cur_cmd >= crate::native::CD_ARMED {
            self.native_trap();
            return;
        }
        self.cur_cmd -= CK_ARMED;
        let cs = self.cur_cs;
        if !self.host.world.recording {
            // A copy that was on the save stack when the rest were disarmed.
            self.set_eq_type(cs, self.cur_cmd);
            return;
        }
        if cs == self.v_image.ck.document_cs {
            if self.v_image.ck.at_big_switch
                && self.expand_depth_count == 0
                && self.scanner_status == NORMAL
                && self.cur_cmd >= CALL
                && self.cur_cmd < END_TEMPLATE
            {
                if !self.checkpoint_late_trigger() {
                    self.checkpoint_take();
                }
            } else {
                // Perhaps it is being redefined, which would drop the trap.
                self.v_image.ck.pending |= PENDING_DOCUMENT;
            }
            return;
        }
        // Copied, not evaluated: skipped text, a macro's argument, the body of a \def.  Everything else counts as a read.
        let mention = match self.scanner_status {
            SKIPPING | MATCHING => true,
            DEFINING => !self.v_image.ck.edef,
            _ => false,
        };
        if mention {
            return;
        }
        self.set_eq_type(cs, self.cur_cmd);
        let date = (2..5).any(|k| self.v_image.ck.sys_cs[k] == cs);
        if date {
            self.v_image.ck.date_read = true;
        } else {
            self.host.world.mark_taint_because(Taint::Hard, "a value made from the clock was read in the preamble");
        }
    }

    /// `\\document` is about to run.  The moment moves on to where it reads the `.aux` file, if the format has a macro `\\@input`: true if it has moved.
    fn checkpoint_late_trigger(&mut self) -> bool {
        let late = self.v_image.ck.late_cs;
        self.v_image.ck.late_cs = 0;
        if late == 0 || self.eq_type(late) < CALL || self.eq_type(late) >= END_TEMPLATE {
            return false;
        }
        self.checkpoint_disarm_cs(self.v_image.ck.document_cs);
        self.v_image.ck.document_cs = late;
        self.checkpoint_arm_cs(late);
        true
    }

    /// What the log says the preamble read, of the format nothing, and of a file still open what it got to.
    fn checkpoint_bytes_read(&self) -> u64 {
        let of = |o: &Observation| match o {
            Observation::Read { kind: ReadKind::Format, .. } => 0,
            Observation::Read { prefix: Some(n), .. } => *n,
            Observation::Read { stat: Some(s), .. } => s.size,
            _ => 0,
        };
        self.host.world.log.iter().map(of).sum()
    }

    /// True if a checkpoint for this run is worth writing, which it may not be for a small preamble compiled once.  Otherwise this leaves a note of what was seen, for the next run.
    /// Whatever this says the run gives the same bytes: a checkpoint is a question of when.
    fn checkpoint_worth_writing(&mut self, key: &Key) -> bool {
        let policy = checkpoint_policy();
        if policy == Policy::Always {
            return true;
        }
        if policy == Policy::Auto && self.checkpoint_bytes_read() >= FIRST_SIGHT_BYTES {
            return true;
        }
        let mut v = Vec::with_capacity(1 << 12);
        for o in &self.host.world.log {
            o.identity(&mut v);
        }
        let identity = xxh64(&v, key.hash) | 1;
        if key.seen()[0] == identity {
            return true;
        }
        key.set_seen(0, identity);
        false
    }

    /// Why this moment cannot be a checkpoint, if so.
    fn checkpoint_refusal(&mut self) -> Option<String> {
        // What a diagram would check first is kept in the image, if tikz-cd is here, so that a restored run does not do it again.
        if self.v_image.cd.tikzcd_cs != 0 && self.v_image.cd.guards == 0 {
            self.native_guards_build();
        }
        if self.host.world.taint == Taint::Hard {
            return Some(self.host.world.taint_reason.to_string());
        }
        if self.v_image.cd.active {
            return Some("in the middle of a native diagram".to_string());
        }
        if self.history > WARNING_ISSUED {
            return Some("an error was reported".to_string());
        }
        if !self.log_opened {
            return Some("the log is not open yet".to_string());
        }
        if self.help_ptr != 0 || self.interrupt != 0 {
            return Some("in the middle of an error".to_string());
        }
        // A page and all it brings: the fonts' and images' state on the way out, the job id, half a DVI buffer.  A PDF file with
        // only its header and `\\immediate` objects is fine.
        if self.init_pdf_output || self.total_pages != 0 {
            return Some("something was shipped out".to_string());
        }
        if let Some(why) = self.synctex_refusal() {
            return Some(why.to_string());
        }
        if let Some(why) = self.checkpoint_clock_refusal() {
            return Some(why.to_string());
        }
        for (k, s) in self.host.files.iter().enumerate() {
            let k = k as i32;
            let fine = match s {
                Slot::Closed => true,
                Slot::In(_) => (READ_FILE_BASE..WRITE_FILE_BASE).contains(&k) || k >= INPUT_FILE_BASE,
                Slot::Fmt(_) => false,
                // A stream that has gone past its buffer has part of it on the disk, and a restore would have all or none: a read of it could tell.
                Slot::Out(o) => (k == LOG_FILE || ((WRITE_FILE_BASE..INPUT_FILE_BASE).contains(&k) && o.total < OUT_BUF as u64)) && o.journal.is_some() && !o.is_pipe(),
                Slot::Pdf(_) => k == PDF_FILE && !self.pdfout_in_stream(),
            };
            if !fine {
                return Some(format!("file slot {} is open", k));
            }
        }
        let (a, b) = (self.v_image.ck.log_date_start as usize, self.v_image.ck.log_date_end as usize);
        if let Slot::Out(o) = &self.host.files[LOG_FILE as usize] {
            if a >= b || o.journal.as_ref().is_none_or(|j| b > j.len() || j[a..b].contains(&b'\n')) {
                return Some("the date in the log has been wrapped".to_string());
            }
            // The date gets wider with the day of the month, and what fits now may have to wrap on another day: §536 counts from the format's name, the banner going out by `wlog`, and §58 breaks the line as soon as it is full.
            if self.length(self.format_ident) + 2 + LONGEST_DATE >= self.max_print_line {
                return Some("the date in the log may wrap on another day".to_string());
            }
        }
        None
    }

    /// What keeps a restored run from setting the clock right, if anything does: this moment and a page record have it in common.
    fn checkpoint_clock_refusal(&self) -> Option<&'static str> {
        let state = self.v_image.ck.sys_state;
        if (1..SYS_DONE).contains(&state) || self.v_image.ck.pending & PENDING_SYS != 0 {
            return Some("expl3's clock constants are not as expected");
        }
        if (1..SEED_DONE).contains(&self.v_image.ck.seed_state) {
            return Some("pgfmath's seed is not as expected");
        }
        // An assignment that left the value as the clock gave it: nothing says which it is on a later day.
        let values = [(self.time(), self.sys_time), (self.day(), self.sys_day), (self.month(), self.sys_month), (self.year(), self.sys_year)];
        for (k, (v, s)) in values.iter().enumerate() {
            if self.v_image.ck.date_assigned & (1 << k) != 0 && v == s {
                return Some("a date parameter was assigned its own value");
            }
        }
        // Assigned in a group that is still open: its old value waits on the save stack, and the end of the group would bring back the clock of that run.
        if self.v_image.ck.date_assigned != 0 {
            for k in 0..self.save_ptr {
                if self.save_type(k) == RESTORE_OLD_VALUE && (INT_BASE + TIME_CODE..=INT_BASE + YEAR_CODE).contains(&self.save_index(k)) {
                    return Some("a date parameter was assigned in a group still open");
                }
            }
        }
        None
    }

    /// True while the run may yet take a preamble checkpoint, as against one going on from a restore.
    pub(crate) fn checkpoint_preamble_awaited(&self) -> bool {
        self.host.checkpoint.key.is_some()
    }

    /// The moment: `\document` was fetched by `main_control`, and `macro_call` comes next.  The cold path copies the image and goes on.
    fn checkpoint_take(&mut self) {
        let _p = crate::prof_scope!("checkpoint");
        self.checkpoint_disarm_all();
        if let Some(why) = self.checkpoint_refusal() {
            self.checkpoint_abandon(&why);
            return;
        }
        let Some(key) = self.host.checkpoint.key.take() else { return };
        // The open files: only what has been read of them counts, whole lines.
        let mut inputs = Vec::new();
        for k in 0..self.host.files.len() {
            if let Slot::In(f) = &self.host.files[k] {
                let idx = self.host.world.in_obs[k];
                if let Some(Observation::Read { hash, prefix, .. }) = self.host.world.log.get_mut(idx) {
                    *hash = xxh64(&f.data[..f.pos], 0);
                    *prefix = Some(f.pos as u64);
                } else {
                    self.checkpoint_abandon("an open file is not in the log");
                    return;
                }
                inputs.push((k as i32, f.path.clone(), f.pos as u64));
            }
        }
        // The range of each section.
        let mut sections = Vec::with_capacity(POD_SECTIONS.len());
        let mut total = 0usize;
        for s in POD_SECTIONS.iter() {
            let (a, b) = if ck_section(s) { (s.live)(self) } else { (0, 0) };
            let (a, b) = (s.offset + a * s.elem_size, s.offset + b.max(a) * s.elem_size);
            sections.push((a, b - a));
            total += b - a;
        }
        let read = self.checkpoint_bytes_read();
        if !self.checkpoint_worth_writing(&key) {
            let m = format!("first sight of this preamble, which read {read} bytes");
            self.checkpoint_abandon(&m);
            return;
        }
        // What the deflate workers hold is not yet in `pdf_gone`, `pdf_offset` and `obj_offset`, and the chunk list has empty places for it.  A preamble with big
        // `\\pdfobj` streams can get that far, and the refusal above has seen to it that no stream is in progress.
        self.pdfout_settle();
        self.host.term_flush();
        self.host.world.recorder_flush_pending();
        let mut outputs = Vec::new();
        let mut pdf = None;
        for k in 0..self.host.files.len() {
            match &mut self.host.files[k] {
                Slot::Out(o) => {
                    // No flush: a body that reads the file sees what it would see in a run with no checkpoint, the journal has the bytes.
                    let bytes = o.journal.take().unwrap_or_default();
                    outputs.push((k as i32, o.path.clone(), bytes));
                }
                Slot::Pdf(p) => pdf = Some((p.path.clone(), p.contents())),
                _ => {}
            }
        }
        let mut paths: Vec<(i32, Vec<u8>)> = Vec::new();
        for o in &self.host.world.log {
            if let Observation::Lookup { format, .. } = o {
                if !paths.iter().any(|p| p.0 == *format) {
                    paths.push((*format, rstex_kpse::format_path(*format).unwrap_or_default()));
                }
            }
        }
        let databases = rstex_kpse::with_global(|k| k.database_files()).into_iter().map(|p| {
            let s = platform::file_stat(&p);
            (p, s)
        });
        let (synctex, synctex_root) = self.synctex_for_checkpoint();
        self.pages_preamble_written(&key);
        let meta = Meta {
            config: key.config,
            log: core::mem::take(&mut self.host.world.log),
            paths,
            databases: databases.collect(),
            date_key: if self.v_image.ck.date_read { self.sys_year * 10000 + self.sys_month * 100 + self.sys_day } else { 0 },
            inputs,
            closed: core::mem::take(&mut self.host.world.closed_outputs),
            outputs,
            term: self.host.checkpoint.term_journal.take().unwrap_or_default(),
            recorder: self.host.world.recorder_take_journal(),
            pdf,
            full_name_of_file: self.host.full_name_of_file.clone(),
            synctex,
            synctex_root,
            spans: Vec::new(),
        };
        let meta = meta.encode();
        // From here on the image is what a restore gets, so what the run does with its own state afterwards no longer matters.
        self.checkpoint_stop();
        let Some(mut image) = platform::HugeBuf::new(total) else { return };
        let mut at = 0;
        {
            let dst = image.as_mut_slice();
            for &(a, n) in &sections {
                dst[at..at + n].copy_from_slice(image_bytes(self, a, n));
                at += n;
            }
        }
        let job = WriteJob { dir: key.dir, path: key.path, meta, image, sections, bound: CACHE_BOUND };
        self.host.checkpoint.worker = std::thread::Builder::new().stack_size(1 << 18).spawn(move || job.run()).ok();
        let m = format!("taken: the preamble read {read} bytes, an image of {} KiB", total >> 10);
        self.checkpoint_say(&m);
    }

    /// Looks for a checkpoint and, if it holds, restores it.
    pub fn checkpoint_try_restore(&mut self, key: &Key) -> Restored {
        // §1584 comes first in a run, and it may end one, for a value of SOURCE_DATE_EPOCH that makes no sense.
        self.init_start_time();
        // One clock for all of it: the date the log prints is the \day and \time that the run reads.
        let now = self.date_and_time();
        let Some(mut f) = rstex_kpse::open_private(&platform::path_from_bytes(&key.path)) else {
            self.checkpoint_say("none");
            return Restored::Cold;
        };
        let Some((meta_len, image_len, meta_hash, image_hash)) = checkpoint_read_header(&mut f) else {
            self.checkpoint_say("a damaged header");
            return Restored::Cold;
        };
        let mut raw = vec![0u8; meta_len];
        if f.read_exact(&mut raw).is_err() || xxh64(&raw, 0) != meta_hash {
            self.checkpoint_say("a damaged meta part");
            return Restored::Cold;
        }
        let Some(meta) = Meta::decode(&raw) else {
            self.checkpoint_say("a meta part that does not decode");
            return Restored::Cold;
        };
        drop(raw);
        if meta.config != key.config {
            self.checkpoint_say("another configuration");
            return Restored::Cold;
        }
        crate::prof_phase!("checkpoint header");
        let mut open_data = Vec::new();
        if let Err(why) = self.checkpoint_validate(&meta, now, &mut open_data) {
            let m = format!("does not hold: {}", why);
            self.checkpoint_say(&m);
            return Restored::ColdAfterLookups;
        }
        crate::prof_phase!("checkpoint validated");
        // A chain of page records hangs from this file and no other with these contents.
        let tag = xxh64(&image_hash.to_le_bytes(), meta_hash);
        let found = self.pages_look(key, &meta, tag, &open_data);
        if matches!(found, pages::Found::Cold) {
            return Restored::ColdAfterLookups;
        }
        // The image: past here a failure needs a new one.  Its sum looks at every byte, which is most of what a restore costs, and at every page, mapped or not.  So it is done until it
        // has passed once, for this file as a stat sees it, and the seal says so: the file cannot change without its stat changing, as for what `checkpoint_read_holds` trusts of a tree.
        let image_at = (HEADER_LEN + meta_len) as u64;
        let seal = seal_of(&platform::file_stat(&key.path), image_hash);
        let sealed = platform::STAT_SEES_CONTENT && key.seen()[1] == seal;
        let ck = if self.host.checkpoint.page.active { f.try_clone().ok() } else { None };
        if !FmtIn::new(f).is_some_and(|f| self.checkpoint_read_image(&f, &meta, image_at, image_len, image_hash, !sealed)) {
            self.checkpoint_say("a damaged image");
            return Restored::ColdImageDirty;
        }
        crate::prof_phase!("checkpoint image read");
        let spans = Self::pages_spans(&meta, image_at);
        match found {
            pages::Found::Records(c) => {
                if !self.pages_overlay(&c) {
                    self.pages_damaged(key);
                    return Restored::ColdImageDirty;
                }
                crate::prof_phase!("pages image read");
                if !self.pages_restore_host(key, tag, meta, now, c, ck, spans) {
                    self.pages_damaged(key);
                    self.checkpoint_say("the files could not be made again");
                    return Restored::ColdImageDirty;
                }
            }
            _ => {
                let mut meta = meta;
                if self.host.world.pass.is_some() {
                    let open: Vec<&[u8]> = meta.outputs.iter().map(|o| &o.1[..]).chain(meta.recorder.iter().map(|r| &r.0[..])).collect();
                    self.host.world.pass_note_restored(meta.log.iter(), &open, meta.pdf.as_ref().map(|p| &p.0[..]));
                }
                let log = core::mem::take(&mut meta.log);
                let inputs = meta.inputs.clone();
                let pdf_sent = meta.pdf.as_ref().map_or(0, |p| p.1.len() as u64);
                if !self.checkpoint_restore_host(meta, now, open_data) {
                    self.checkpoint_say("the files could not be made again");
                    return Restored::ColdImageDirty;
                }
                self.pages_begin(key, tag, ck, spans, log, &inputs, pdf_sent);
            }
        }
        self.checkpoint_refresh_clock(now);
        let _ = rstex_kpse::take_recorded();
        let _ = rstex_kpse::take_warnings();
        if (checkpoint_touch(&key.path) || !sealed) && platform::STAT_SEES_CONTENT {
            let stat = platform::file_stat(&key.path);
            if stat.as_ref().is_some_and(|s| s.change + RACY_NS < rstex_kpse::now_ns() as i64) {
                key.set_seen(1, seal_of(&stat, image_hash));
            }
        }
        crate::prof_phase!("checkpoint restored");
        if !sealed {
            self.checkpoint_say("the image has passed its sum");
        }
        self.checkpoint_say("restored");
        Restored::Yes
    }

    /// Would each observation get the same answer now?  The files stacked at `\document` are kept: they are about to be needed.
    fn checkpoint_validate(&mut self, meta: &Meta, now: (i32, i32, i32, i32), open_data: &mut Vec<(i32, Vec<u8>)>) -> Result<(), String> {
        let slots = self.host.files.len() as i32;
        if meta.inputs.iter().map(|x| x.0).chain(meta.outputs.iter().map(|x| x.0)).any(|k| !(0..slots).contains(&k)) {
            return Err("a file slot that does not exist".to_string());
        }
        if meta.date_key != 0 {
            let (_, d, m, y) = now;
            if y * 10000 + m * 100 + d != meta.date_key {
                return Err("the preamble read the date, and it is another day".to_string());
            }
        }
        for (f, p) in &meta.paths {
            if rstex_kpse::format_path(*f).unwrap_or_default() != *p {
                return Err(format!("the search path of format {}", f));
            }
        }
        for (p, s) in &meta.databases {
            if platform::file_stat(p) != *s {
                return Err(format!("the database {}", String::from_utf8_lossy(p)));
            }
        }
        let mut own: Vec<&[u8]> = Vec::new();
        let mut kept: Vec<(Vec<u8>, Vec<u8>)> = Vec::new();
        // The same question gets the same answer within one validation, and a preamble asks many twice: \IfFileExists, then \input.
        let mut asked: std::collections::HashMap<(&[u8], i32, bool), Option<Vec<u8>>> = std::collections::HashMap::new();
        let mut read: std::collections::HashSet<(&[u8], u64, Option<u64>)> = std::collections::HashSet::new();
        for o in &meta.log {
            match o {
                Observation::Output { path } => own.push(path_norm(path)),
                Observation::Lookup { name, format, must_exist, found } => {
                    let now = asked.entry((&name[..], *format, *must_exist)).or_insert_with(|| rstex_kpse::find_file(name, *format, *must_exist));
                    if *now != *found {
                        return Err(format!("lookup of {}", String::from_utf8_lossy(name)));
                    }
                }
                Observation::Env { name, value, cnf } => {
                    let now = if *cnf { rstex_kpse::var_value(name) } else { platform::env_var(name) };
                    if now != *value {
                        return Err(format!("the variable {}", String::from_utf8_lossy(name)));
                    }
                }
                Observation::Stat { path, stat, size_only } => {
                    let now = platform::file_stat(path);
                    let same = if *size_only {
                        // The run's own file has its own size: nothing to ask.
                        own.contains(&path_norm(path)) || now.as_ref().map(|s| s.size) == stat.as_ref().map(|s| s.size)
                    } else {
                        now == *stat
                    };
                    if !same {
                        return Err(format!("stat of {}", String::from_utf8_lossy(path)));
                    }
                }
                Observation::Read { path, stat, hash, prefix, kind } => {
                    // The run itself will have written it by then, and what it writes is in the checkpoint.
                    if own.contains(&path_norm(path)) || !read.insert((&path[..], *hash, *prefix)) {
                        continue;
                    }
                    if !checkpoint_read_holds(path, stat, *hash, *prefix, *kind, &mut kept) {
                        return Err(format!("read of {}", String::from_utf8_lossy(path)));
                    }
                }
            }
        }
        for (i, (slot, path, _)) in meta.inputs.iter().enumerate() {
            let data = match kept.iter().position(|k| k.0 == *path) {
                // One file can be open on several streams at the same place, and then the log has one read of it.
                Some(k) if meta.inputs[i + 1..].iter().any(|x| x.1 == *path) => kept[k].1.clone(),
                Some(k) => kept.swap_remove(k).1,
                // The run wrote it before it read it.  A cold run writes it first, so what is on the disk now is no matter: it is these bytes that it reads.
                None => match meta.closed.iter().rev().find(|c| path_norm(&c.0) == path_norm(path)) {
                    Some(c) if own.contains(&path_norm(path)) => c.1.clone(),
                    _ => return Err(format!("reopening {}", String::from_utf8_lossy(path))),
                },
            };
            open_data.push((*slot, data));
        }
        Ok(())
    }

    /// The spans, straight into the image, as a format's sections go: read, or mapped where a format's would be.  So what is mapped here is the part of a section that a run
    /// seldom writes, a page it does write gets copied then, and a restore starts from a new reservation, so nothing of this lands on another mapping.  The sum, if `sum`,
    /// looks at every byte, read or mapped: the mapped pages are the file's, which the kernel has at hand.
    fn checkpoint_read_image(&mut self, f: &FmtIn, meta: &Meta, start: u64, image_len: u64, image_hash: u64, sum: bool) -> bool {
        let limit = core::mem::offset_of!(Tex, host) as u64;
        // `hash_tab` too, though a format's is read: most new names are made in the preamble, so what comes after it writes to few of the pages, and looks at the others.
        // Hello takes 15 faults for its 1 MiB, and the thesis a tenth of a percent more time, from 269 faults more.
        let mapped_at = |a: u64| {
            POD_SECTIONS.iter().any(|s| s.offset as u64 <= a && a < (s.offset + s.cap * s.elem_size) as u64 && (crate::image::fmt_mapped(s.name) || s.name == "hash_tab"))
        };
        let mut h = 0;
        let mut at = start;
        for &(a, b) in &meta.spans {
            if a > b || b > limit {
                return false;
            }
            let n = (b - a) as usize;
            at += crate::image::image_padding(at, a as usize, n);
            if self.image_load_bytes(f, mapped_at(a), a as usize, n, at).is_none() {
                return false;
            }
            if sum {
                h = image_sum(image_bytes(self, a as usize, n), h);
            }
            at += n as u64;
        }
        at - start == image_len && (!sum || h == image_hash)
    }

    /// Host's state, field by field: the files come back open and at their places, the terminal and the side files get what the
    /// preamble gave them.  A module's `HVars` that is not here starts empty, by design: `pdffont.rs` and `ext.rs` replay from the image what they need.
    fn checkpoint_restore_host(&mut self, meta: Meta, now: (i32, i32, i32, i32), open_data: Vec<(i32, Vec<u8>)>) -> bool {
        for (path, bytes) in &meta.closed {
            if std::fs::write(platform::path_from_bytes(path), bytes).is_err() {
                return false;
            }
        }
        for ((slot, path, pos), (_, data)) in meta.inputs.into_iter().zip(open_data) {
            self.host.files[slot as usize] = Slot::In(InFile { path, data, pos: pos as usize });
        }
        let (start, end) = (self.v_image.ck.log_date_start as usize, self.v_image.ck.log_date_end as usize);
        let (t, d, m, y) = now;
        for (slot, path, bytes) in meta.outputs {
            let Some(mut o) = OutFile::reopen(&path) else { return false };
            if slot == LOG_FILE {
                if start > end || end > bytes.len() {
                    return false;
                }
                // §562 Print the banner line, including the date and time, of this run.
                o.bytes(&bytes[..start]);
                o.bytes(&log_date(d, m, y, t));
                o.bytes(&bytes[end..]);
            } else {
                o.bytes(&bytes);
            }
            self.host.files[slot as usize] = Slot::Out(o);
        }
        if let Some((name, bytes)) = meta.recorder {
            if !self.host.world.recorder_restore(name, &bytes) {
                return false;
            }
        }
        if let Some((path, bytes)) = meta.pdf {
            self.host.hv.outcache.capture(&path);
            let Some(p) = PdfOut::reopen(&path, &bytes) else { return false };
            self.host.files[PDF_FILE as usize] = Slot::Pdf(p);
        }
        if !self.synctex_restore(meta.synctex, meta.synctex_root) {
            return false;
        }
        self.host.full_name_of_file = meta.full_name_of_file;
        // What `search_string` keeps of the strings of a run: a process that comes here has made none yet, and this says that it does not rest on that.
        self.host.hv.strings = Default::default();
        // Last: up to here a failure means a cold run, which says all this itself.
        self.host.world.terminal_write(&meta.term);
        true
    }

    /// What depends on the clock, as §1517 and §1584 would have set it in this run.  DESIGN section 7.
    fn checkpoint_refresh_clock(&mut self, now: (i32, i32, i32, i32)) {
        let ck = self.v_image.ck;
        let (time, day, month, year) = now;
        self.sys_time = time;
        self.sys_day = day;
        self.sys_month = month;
        self.sys_year = year;
        // A parameter the preamble assigned keeps its value.
        if ck.date_assigned & 1 == 0 {
            self.set_time(time);
        }
        if ck.date_assigned & 2 == 0 {
            self.set_day(day);
        }
        if ck.date_assigned & 4 == 0 {
            self.set_month(month);
        }
        if ck.date_assigned & 8 == 0 {
            self.set_year(year);
        }
        self.seconds_and_micros();
        self.init_start_time();
        self.v_pdfship.start_time_len = 0;
        if !ck.seed_pinned {
            self.random_seed = (self.microseconds * 1000) + (self.epochseconds % 1000000);
            self.init_randoms(self.random_seed);
        }
        // expl3's constants, made in \everyjob.
        if ck.sys_state == SYS_DONE {
            let values = [time % 60, time / 60, day, month, year];
            for k in 0..5 {
                let cs = ck.sys_cs[k as i32];
                // Fetched and assigned something else since, then no one reads them as a constant.
                let t = self.eq_type(cs) & !crate::native::ARMED_BITS;
                if t == CHAR_GIVEN || t == MATH_GIVEN {
                    self.set_equiv(cs, values[k]);
                }
            }
            let (s, n) = self.pdfship_start_time_str();
            self.checkpoint_patch_macro(ck.sys_cs[5], &s[..n]);
        }
        // pgfmath's seed: \time*\year, or 1.
        if ck.seed_state == SEED_DONE {
            let v = self.time().wrapping_mul(self.year());
            let text = (if v == 0 { 1 } else { v }).to_string();
            self.checkpoint_patch_macro(ck.seed_cs, text.as_bytes());
        }
    }

    /// `cs`, a macro with no parameters unless it has been made something else since, gets this text: in place, or in a new block for another length.
    fn checkpoint_patch_macro(&mut self, cs: i32, text: &[u8]) {
        let Some((a, b)) = self.checkpoint_macro_body(cs) else { return };
        if (b - a) as usize == text.len() {
            for (k, &c) in text.iter().enumerate() {
                self.toks[a + k as i32] = ck_token(c);
            }
        } else {
            let m = self.tb_mark();
            self.tb_push(END_MATCH_TOKEN);
            for &c in text {
                self.tb_push(ck_token(c));
            }
            let q = self.tb_commit_ref(m);
            let old = self.equiv(cs);
            self.set_equiv(cs, q);
            self.delete_token_ref(old);
        }
    }

    /// From the restored state to the end of the run: what `get_x_token` was about to do, then `main_control` from its big switch,
    /// and the tail of §1517.
    pub fn checkpoint_resume(&mut self) {
        self.v_image.ck.resumed = true;
        self.pass_job_named();
        self.ext_images_after_restore();
        // At a page the big switch is next.  At the preamble it is `\\document`, which `get_x_token` was about to expand.
        if self.host.checkpoint.page.resumed_at == 0 {
            self.macro_call();
        } else if self.pages_long_way_to_go() {
            self.image_collapse_hot();
        }
        self.main_control();
        crate::prof_phase!("main_control left");
        self.final_cleanup();
        self.close_files_and_terminate();
        self.do_final_end();
    }

    /// An armed word on its way out: `eq_destroy` of what it really is.
    #[cold]
    pub fn checkpoint_eq_destroy(&mut self, w: Word) {
        let t = self.eq_type_field(w) & !crate::native::ARMED_BITS;
        self.eq_destroy(Word::hb(self.equiv_field(w), t, self.eq_level_field(w)));
    }
}

/// A character as `str_toks` makes it: a space is a space, and everything else other.
fn ck_token(c: u8) -> i32 {
    if c == b' ' { SPACE_TOKEN } else { OTHER_TOKEN + c as i32 }
}

/// `(meta length, image length, meta hash, image hash)`, if the header is whole and fits the file's length.
fn checkpoint_read_header(f: &mut std::fs::File) -> Option<(usize, u64, u64, u64)> {
    let mut h = [0u8; HEADER_LEN];
    f.read_exact(&mut h).ok()?;
    let mut r = In { b: &h, p: 0 };
    let (magic, meta_len, image_len, meta_hash, image_hash, size, own) = (r.u64()?, r.u64()?, r.u64()?, r.u64()?, r.u64()?, r.u64()?, r.u64()?);
    if magic != FILE_MAGIC || own != xxh64(&h[..HEADER_LEN - 8], 0) || size != core::mem::size_of::<Tex>() as u64 {
        return None;
    }
    let total = f.metadata().ok()?.len();
    if total != (HEADER_LEN as u64).checked_add(meta_len)?.checked_add(image_len)? {
        return None;
    }
    Some((meta_len as usize, image_len, meta_hash, image_hash))
}

/// Would this read get the same answer?  A file in a tree goes by its stat where that sees every change, and a project file by its
/// content; a file that was only partly read when the checkpoint was taken, by what was read of it.
fn checkpoint_read_holds(path: &[u8], stat: &Option<FileStat>, hash: u64, prefix: Option<u64>, kind: ReadKind, kept: &mut Vec<(Vec<u8>, Vec<u8>)>) -> bool {
    let p = platform::path_from_bytes(path);
    let Some(_) = stat else {
        // Absent, or not readable, or a directory, which reads as nothing too.
        return match std::fs::metadata(&p) {
            Err(_) => true,
            Ok(m) => m.is_dir() || std::fs::File::open(&p).is_err(),
        };
    };
    let now = platform::file_stat(path);
    if prefix.is_none() {
        match kind {
            ReadKind::Format => return now == *stat,
            ReadKind::Tds if platform::STAT_SEES_CONTENT && now == *stat => return true,
            _ => {}
        }
    }
    let Ok(data) = std::fs::read(&p) else { return false };
    match prefix {
        Some(n) => {
            // It is about to be the open file, and these are the bytes that were hashed.
            let holds = data.len() as u64 >= n && xxh64(&data[..n as usize], 0) == hash && prefix_ends_a_line(&data, n as usize);
            if holds {
                kept.push((path.to_vec(), data));
            }
            holds
        }
        None => xxh64(&data, 0) == hash,
    }
}

/// True if a line still ends after the first `n` bytes of `data`, where the file was read up to when the checkpoint was taken.  With no terminator the end of the file ended
/// that line, and with more bytes after it the line is another one.  A CR ends a line unless an LF comes next, and then the line ends one byte later.
fn prefix_ends_a_line(data: &[u8], n: usize) -> bool {
    match n.checked_sub(1).map(|k| data[k]) {
        None | Some(b'\n') => true,
        Some(b'\r') => data.get(n) != Some(&b'\n'),
        Some(_) => data.len() == n,
    }
}

/// A used checkpoint is the newest, for the eviction.  Once a day at most, since it is one more system call, and it undoes the seal.  True if it did.
fn checkpoint_touch(path: &[u8]) -> bool {
    let p = platform::path_from_bytes(path);
    let Ok(m) = std::fs::metadata(&p) else { return false };
    let now = std::time::SystemTime::now();
    if m.modified().ok().and_then(|t| now.duration_since(t).ok()).is_some_and(|d| d.as_secs() > 86400) {
        if let Ok(f) = std::fs::OpenOptions::new().write(true).open(&p) {
            return f.set_modified(now).is_ok();
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xxh64_is_xxh64() {
        assert_eq!(xxh64(b"", 0), 0xef46db3751d8e999);
        assert_eq!(xxh64(b"a", 0), 0xd24ec4f1a98c6e5b);
        assert_eq!(xxh64(b"abc", 0), 0x44bc2cf5ad770999);
        assert_eq!(xxh64(b"Nobody inspects the spammish repetition", 0), 0xfbcea83c8a378bf1);
    }

    #[test]
    fn image_sum_sees_what_goes_wrong() {
        let good: Vec<u8> = (0..1 << 16).map(|k: u32| (k.wrapping_mul(2654435761) >> 24) as u8).collect();
        let sum = image_sum(&good, 7);
        assert_eq!(sum, image_sum(&good, 7));
        assert_ne!(sum, image_sum(&good, 8));
        // What the cache group of run_snapshot.py does to a file: the same mask at every page.
        let mut x = good.clone();
        for k in (0..x.len()).step_by(4096) {
            x[k] ^= 0x55;
        }
        assert_ne!(sum, image_sum(&x, 7));
        let mut x = good.clone();
        x[12345] ^= 1;
        assert_ne!(sum, image_sum(&x, 7));
        let mut x = good.clone();
        x[8192..12288].fill(0);
        assert_ne!(sum, image_sum(&x, 7));
        let mut x = good.clone();
        x[..8192].rotate_left(4096);
        assert_ne!(sum, image_sum(&x, 7));
        assert_ne!(sum, image_sum(&good[..good.len() - 1], 7));
    }

    #[test]
    fn long_runs_of_zero_pages_are_left_out() {
        let mut b = vec![1u8; 200 * 4096];
        assert_eq!(nonzero_spans(&b), vec![(0, b.len())]);
        b[4096..4096 * 21].fill(0);
        assert_eq!(nonzero_spans(&b), vec![(0, 4096), (4096 * 21, b.len())]);
        // A short run stays.
        b[4096 * 40..4096 * 45].fill(0);
        assert_eq!(nonzero_spans(&b), vec![(0, 4096), (4096 * 21, b.len())]);
        // At the end, and the whole of it.
        b[4096 * 150..].fill(0);
        assert_eq!(nonzero_spans(&b), vec![(0, 4096), (4096 * 21, 4096 * 150)]);
        assert_eq!(nonzero_spans(&vec![0u8; 100 * 4096]), vec![]);
        assert_eq!(nonzero_spans(&[]), vec![]);
        assert_eq!(nonzero_spans(&[0u8; 100]), vec![(0, 100)]);
    }

    #[test]
    fn a_prefix_ends_where_a_line_ends() {
        assert!(prefix_ends_a_line(b"", 0));
        assert!(prefix_ends_a_line(b"more", 0));
        assert!(prefix_ends_a_line(b"one\n", 4));
        assert!(prefix_ends_a_line(b"one\nmore", 4));
        assert!(prefix_ends_a_line(b"one", 3), "the end of the file, as it was");
        assert!(!prefix_ends_a_line(b"one more", 3), "the line went on");
        assert!(prefix_ends_a_line(b"one\r", 4));
        assert!(prefix_ends_a_line(b"one\rmore", 4));
        assert!(!prefix_ends_a_line(b"one\r\nmore", 4), "that line ended a byte later");
    }

    #[test]
    fn the_date_is_what_section_562_prints() {
        assert_eq!(log_date(3, 11, 2026, 2 * 60 + 5), b"3 NOV 2026 02:05");
        assert_eq!(log_date(14, 1, 2023, 23 * 60 + 59), b"14 JAN 2023 23:59");
    }

    #[test]
    fn a_path_is_compared_without_its_dot() {
        assert_eq!(path_norm(b"./a.tex"), b"a.tex");
        assert_eq!(path_norm(b"a.tex"), b"a.tex");
        assert_eq!(path_norm(b"../a.tex"), b"../a.tex");
    }

    #[test]
    fn the_log_goes_through_the_file_format() {
        let log = vec![
            Observation::Lookup { name: b"a.sty".to_vec(), format: 26, must_exist: true, found: None },
            Observation::Lookup { name: b"b.sty".to_vec(), format: 26, must_exist: false, found: Some(b"./b.sty".to_vec()) },
            Observation::Read {
                path: b"b.sty".to_vec(),
                stat: Some(FileStat { size: 1, mtime: 2, mtime_nsec: 3, change: 4, inode: 5, regular: true }),
                hash: 6,
                prefix: Some(7),
                kind: ReadKind::Tds,
            },
            Observation::Read { path: b"c".to_vec(), stat: None, hash: 0, prefix: None, kind: ReadKind::Project },
            Observation::Stat { path: b"d".to_vec(), stat: None, size_only: true },
            Observation::Env { name: b"E".to_vec(), value: Some(b"1".to_vec()), cnf: true },
            Observation::Output { path: b"f".to_vec() },
        ];
        let m = Meta {
            config: b"cfg".to_vec(),
            log: log.clone(),
            paths: vec![(26, b".:/x".to_vec())],
            databases: vec![(b"/x/ls-R".to_vec(), None)],
            date_key: 20261103,
            inputs: vec![(65, b"main.tex".to_vec(), 40)],
            closed: vec![(b"note".to_vec(), b"text".to_vec())],
            outputs: vec![(0, b"main.log".to_vec(), b"log".to_vec())],
            term: b"term".to_vec(),
            recorder: Some((b"main.fls".to_vec(), b"PWD".to_vec())),
            pdf: None,
            full_name_of_file: None,
            synctex: Some((b"main.synctex(busy)".to_vec(), b"SyncTeX".to_vec())),
            synctex_root: Some(b"main.tex".to_vec()),
            spans: Vec::new(),
        };
        let mut encoded = m.encode();
        put_u64(&mut encoded, 1); // what the worker adds
        put_u64(&mut encoded, 1);
        put_u64(&mut encoded, 2);
        let back = Meta::decode(&encoded).unwrap();
        assert_eq!(back.log, log);
        assert_eq!(back.config, m.config);
        assert_eq!(back.paths, m.paths);
        assert_eq!(back.databases, m.databases);
        assert_eq!(back.date_key, 20261103);
        assert_eq!(back.inputs, m.inputs);
        assert_eq!(back.closed, m.closed);
        assert_eq!(back.outputs, m.outputs);
        assert_eq!(back.term, m.term);
        assert_eq!(back.recorder, m.recorder);
        assert_eq!(back.synctex, m.synctex);
        assert_eq!(back.synctex_root, m.synctex_root);
        assert_eq!(back.spans, vec![(1, 2)]);
    }

    /// A file of `n` bytes that was last used `age` seconds ago.
    fn cache_file(d: &std::path::Path, name: &str, n: usize, age: u64) {
        let p = d.join(name);
        std::fs::write(&p, vec![0u8; n]).unwrap();
        let t = std::time::SystemTime::now() - std::time::Duration::from_secs(age);
        std::fs::OpenOptions::new().write(true).open(&p).unwrap().set_modified(t).unwrap();
    }

    #[test]
    fn eviction_keeps_the_newest_under_the_bound() {
        let d = std::env::temp_dir().join(format!("rstex-ck-evict-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        for (k, name) in ["old.ck", "mid.ck", "new.ck"].iter().enumerate() {
            cache_file(&d, name, 1000, 3000 - k as u64 * 1000);
        }
        cache_prune(&platform::path_to_bytes(&d), 2000);
        assert!(!d.join("old.ck").exists());
        assert!(d.join("mid.ck").exists() && d.join("new.ck").exists());
        cache_prune(&platform::path_to_bytes(&d), 2000);
        assert!(d.join("mid.ck").exists(), "under the bound nothing goes");
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn age_goes_whatever_the_room_and_the_table_stays() {
        let d = std::env::temp_dir().join(format!("rstex-ck-prune-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        cache_file(&d, "stale.ck", 10, CACHE_MAX_AGE + 100);
        cache_file(&d, "used.ck", 10, CACHE_MAX_AGE - 100);
        cache_file(&d, "dead.ck.77.tmp", 10, STALE_TMP + 100);
        cache_file(&d, "live.ck.78.tmp", 10, 100);
        cache_file(&d, "seen", 10, CACHE_MAX_AGE * 2);
        cache_prune(&platform::path_to_bytes(&d), 1 << 30);
        assert!(!d.join("stale.ck").exists(), "not used for too long");
        assert!(!d.join("dead.ck.77.tmp").exists(), "a run that died");
        assert!(d.join("used.ck").exists() && d.join("live.ck.78.tmp").exists());
        assert!(d.join("seen").exists(), "it has a fixed size, and holds what was seen long ago");
        // The bound does not count it, and it is never the oldest that goes.
        cache_prune(&platform::path_to_bytes(&d), 0);
        assert!(d.join("seen").exists());
        assert!(!d.join("used.ck").exists());
        std::fs::remove_dir_all(&d).unwrap();
    }

    /// A chain of `n` bytes, last used `age` seconds ago.
    fn chain_dir(d: &std::path::Path, name: &str, n: usize, age: u64) {
        let c = d.join(name);
        std::fs::create_dir_all(&c).unwrap();
        cache_file(&c, "f1", n, 0);
        cache_file(&c, "lock", 0, 0);
        cache_file(&c, "f0", 16, age);
    }

    #[test]
    fn a_chain_goes_with_its_checkpoint_and_counts_with_the_rest() {
        let d = std::env::temp_dir().join(format!("rstex-ck-chain-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        // The oldest checkpoint takes its chain along, though that was used later.
        cache_file(&d, "a.ck", 1000, 5000);
        chain_dir(&d, "a.pg", 1000, 100);
        cache_file(&d, "b.ck", 1000, 200);
        chain_dir(&d, "b.pg", 1000, 300);
        // Nothing to hang from.
        chain_dir(&d, "orphan.pg", 10, 1);
        cache_prune(&platform::path_to_bytes(&d), 1 << 30);
        assert!(!d.join("orphan.pg").exists());
        assert!(d.join("a.pg").exists(), "under the bound nothing else goes");
        cache_prune(&platform::path_to_bytes(&d), 2100);
        assert!(!d.join("a.ck").exists() && !d.join("a.pg").exists());
        assert!(d.join("b.ck").exists() && d.join("b.pg").exists());
        // A chain goes on its own, if it is the oldest, and leaves its checkpoint.
        cache_prune(&platform::path_to_bytes(&d), 1100);
        assert!(d.join("b.ck").exists() && !d.join("b.pg").exists());
        // One not used for a month goes, whatever the room.
        chain_dir(&d, "b.pg", 10, CACHE_MAX_AGE + 100);
        cache_prune(&platform::path_to_bytes(&d), 1 << 30);
        assert!(!d.join("b.pg").exists());
        std::fs::remove_dir_all(&d).unwrap();
    }

    fn test_key(d: &std::path::Path, hash: u64) -> Key {
        Key { config: Vec::new(), hash, dir: platform::path_to_bytes(d), path: Vec::new() }
    }

    #[test]
    fn the_table_of_what_was_seen() {
        let d = std::env::temp_dir().join(format!("rstex-ck-seen-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        let a = test_key(&d, 5);
        assert_eq!(a.seen(), [0, 0], "no directory yet");
        a.set_seen(0, 11);
        assert_eq!(a.seen(), [11, 0]);
        a.set_seen(1, 22);
        assert_eq!(a.seen(), [11, 22]);
        a.set_seen(0, 33);
        assert_eq!(a.seen(), [33, 22], "a word at a time");
        let b = test_key(&d, 6);
        assert_eq!(b.seen(), [0, 0], "a slot of its own");
        // At the end of the table, and the file is never larger than that.
        let c = test_key(&d, SEEN_SLOTS - 1);
        c.set_seen(1, 44);
        assert_eq!(c.seen(), [0, 44]);
        let same_slot = test_key(&d, SEEN_SLOTS + 5);
        assert_eq!(same_slot.seen(), [33, 22], "another document on a taken slot only costs it a later checkpoint or a sum");
        assert_eq!(std::fs::metadata(d.join("seen")).unwrap().len(), SEEN_SLOTS * 16);
        // A damaged table says nothing false: a short file reads as empty.
        std::fs::write(d.join("seen"), [1, 2, 3]).unwrap();
        assert_eq!(a.seen(), [0, 0]);
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_preamble_is_the_same_whatever_a_stat_says() {
        let identity = |stat: u64, hash: u64| {
            let mut v = Vec::new();
            let st = Some(FileStat { size: 1, mtime: stat as i64, mtime_nsec: 3, change: 4, inode: 5, regular: true });
            Observation::Read { path: b"main.tex".to_vec(), stat: st.clone(), hash, prefix: Some(40), kind: ReadKind::Project }.identity(&mut v);
            Observation::Stat { path: b"a.png".to_vec(), stat: st, size_only: false }.identity(&mut v);
            v
        };
        assert_eq!(identity(1, 7), identity(2, 7));
        assert_ne!(identity(1, 7), identity(1, 8));
    }

    #[test]
    fn what_a_preamble_read() {
        let t = crate::toks::testkit::machine();
        let st = |size| Some(FileStat { size, mtime: 2, mtime_nsec: 3, change: 4, inode: 5, regular: true });
        let read = |stat, prefix, kind| Observation::Read { path: b"x".to_vec(), stat, hash: 0, prefix, kind };
        t.host.world.log = vec![
            read(st(100), None, ReadKind::Tds),
            read(st(20), None, ReadKind::Project),
            read(st(1000), Some(30), ReadKind::Project), // still open, at 30
            read(st(5_000_000), None, ReadKind::Format),
            read(None, None, ReadKind::Project), // not there
            Observation::Lookup { name: b"y".to_vec(), format: 26, must_exist: true, found: None },
        ];
        assert_eq!(t.checkpoint_bytes_read(), 150);
    }

    #[test]
    fn a_seal_goes_with_the_stat_and_the_sum() {
        let st = |change: i64| Some(FileStat { size: 1, mtime: 2, mtime_nsec: 3, change, inode: 5, regular: true });
        assert_eq!(seal_of(&st(4), 9), seal_of(&st(4), 9));
        assert_ne!(seal_of(&st(4), 9), seal_of(&st(8), 9));
        assert_ne!(seal_of(&st(4), 9), seal_of(&st(4), 10));
        assert_ne!(seal_of(&None, 0), 0, "an empty slot seals nothing");
    }
}
