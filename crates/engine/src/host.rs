//! Host and World, DESIGN.md section 6: everything that is not POD, and every side effect.
//!
//! - **World** is the only path to the operating system.  Each operation (lookup, read, stat,
//!   clock, env, shell, terminal) is recorded as an [`Observation`] and may mark the run tainted.
//!   Nothing else in the engine touches `std::fs`, `std::env`, `std::process`, `std::io` or
//!   `std::time`; a grep in `tests/` enforces it.
//! - **Input files are memory.**  A file is read whole on open; `input_ln` is memchr plus copy.
//!   Every open input stores its path and logical position, with whole-line semantics.
//! - **Output is a journal.**  Log, `.fls`, `\openout` and SyncTeX streams go through
//!   [`OutFile`], which can keep their bytes for re-materialising on snapshot restore.
//! - **PDF output is a chunk list** with deferred offsets and no seek-back: [`PdfOut`].
//! - **Printing plumbing**: `term_out`, `log_file` and `write_file[j]`, the targets of `selector`,
//!   are reached through `wterm_*`, `wlog_*` and `write_file_*` below.  Part 5's `print_char`,
//!   `print_ln` and friends use only these.
//!
//! Host is the last field of `Tex`, constructed in place, and never snapshotted.  Leaf details
//! (kpathsea search, TCX, the file index) come later; the interface here is what they plug into.
//! What differs by operating system is in `platform/`; there is no `cfg` here.
#![forbid(unsafe_code)]
#![allow(non_snake_case, dead_code)]

use std::io::{Read, Write};

use crate::platform::{self, FileStat, path_from_bytes as path_of};
use crate::state::*;

// ---------------------------------------------------------------------------
// File identities.
// ---------------------------------------------------------------------------

/// WEB's file variables are small integers: `log_file` is `LOG_FILE`, `read_file[n]` is
/// `read_file(n)`, `write_file[j]` is `write_file(j)`, `input_file[k]` is `input_file(k)`,
/// `cur_file` is `self.cur_file()`.
pub type FileId = i32;
pub const LOG_FILE: FileId = 0;
pub const DVI_FILE: FileId = 1;
pub const PDF_FILE: FileId = 2;
pub const TFM_FILE: FileId = 3;
pub const VF_FILE: FileId = 4;
pub const FMT_FILE: FileId = 5;
pub const POOL_FILE: FileId = 6;
pub const AUX_FILE: FileId = 7;
pub const TERM_IN: FileId = 8;
const READ_FILE_BASE: FileId = 16;
const WRITE_FILE_BASE: FileId = 32;
const INPUT_FILE_BASE: FileId = 64;
const NUM_FILES: usize = INPUT_FILE_BASE as usize + MAX_IN_OPEN_CAP;

/// `read_file[n]`, §506.
#[inline(always)]
pub const fn read_file(n: i32) -> FileId {
    READ_FILE_BASE + n
}
/// `write_file[j]`, §1522.
#[inline(always)]
pub const fn write_file(j: i32) -> FileId {
    WRITE_FILE_BASE + j
}
/// `input_file[k]`, §326.
#[inline(always)]
pub const fn input_file(k: i32) -> FileId {
    INPUT_FILE_BASE + k
}

/// Kpathsea format codes used by WEB.
pub const KPSE_TEX_FORMAT: i32 = rstex_kpse::KPSE_TEX_FORMAT;
pub const KPSE_TFM_FORMAT: i32 = rstex_kpse::KPSE_TFM_FORMAT;
pub const KPSE_VF_FORMAT: i32 = rstex_kpse::KPSE_VF_FORMAT;
pub const KPSE_FMT_FORMAT: i32 = rstex_kpse::KPSE_FMT_FORMAT;

/// `version_string` from web2c's generated header, §61.
pub const VERSION_STRING: &[u8] = b" (rstex)";
/// §556.
pub const PROMPT_FILE_NAME_HELP_MSG: &[u8] = b"(Press Enter to retry, or Control-D to exit";

pub use crate::platform::is_dir_sep;

// ---------------------------------------------------------------------------
// World.
// ---------------------------------------------------------------------------

/// How far a run has observed something a checkpoint cannot reproduce.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum Taint {
    #[default]
    None,
    /// A soft read, like the date: valid for a matching key.
    Soft,
    /// No checkpoint past this point: `\write18`, pipe input, terminal `\read`, error state.
    Hard,
}

/// One observation of the outside world.  A checkpoint is valid only if each observation in its
/// ordered log would get the same answer now (DESIGN.md section 7).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Observation {
    Lookup { name: Vec<u8>, format: i32, found: Option<Vec<u8>> },
    Read { path: Vec<u8>, stat: Option<FileStat> },
    Stat { path: Vec<u8>, stat: Option<FileStat> },
    Clock { epoch: i64, micros: i32 },
    Env { name: Vec<u8>, value: Option<Vec<u8>> },
    Shell { command: Vec<u8>, status: i32 },
    Terminal,
}

/// The only path to the operating system.
#[derive(Default)]
pub struct World {
    pub log: Vec<Observation>,
    pub taint: Taint,
    /// Directories searched before everything else: `-output-directory`.
    pub output_directory: Option<Vec<u8>>,
    /// `SOURCE_DATE_EPOCH`, read once.
    start_time: Option<i64>,
}

impl World {
    pub fn new() -> World {
        World::default()
    }

    /// Marks the run; the taint only ever rises.
    pub fn mark_taint(&mut self, t: Taint) {
        if t > self.taint {
            self.taint = t;
        }
    }

    /// Finds a file through the search path; a negative answer is an observation too.
    pub fn lookup(&mut self, name: &[u8], format: i32, must_exist: bool) -> Option<Vec<u8>> {
        let found = rstex_kpse::find_file(name, format, must_exist);
        self.log.push(Observation::Lookup { name: name.to_vec(), format, found: found.clone() });
        found
    }

    /// True if `path` names a regular file; it does not count as an observation by itself, since
    /// a later `read` or `stat` records the answer.
    fn is_file(&self, path: &[u8]) -> bool {
        std::fs::metadata(path_of(path)).map(|m| m.is_file()).unwrap_or(false)
    }

    /// Reads a whole file into memory.
    pub fn read(&mut self, path: &[u8]) -> Option<Vec<u8>> {
        let data = std::fs::read(path_of(path)).ok();
        let st = if data.is_some() { platform::file_stat(path) } else { None };
        self.log.push(Observation::Read { path: path.to_vec(), stat: st });
        data
    }

    /// `\pdffilesize`, `\pdffilemoddate` and the like.
    pub fn stat(&mut self, path: &[u8]) -> Option<FileStat> {
        let st = platform::file_stat(path);
        self.log.push(Observation::Stat { path: path.to_vec(), stat: st.clone() });
        st
    }

    /// The time now, as seconds since the epoch and microseconds.
    pub fn clock(&mut self) -> (i64, i32) {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| (d.as_secs() as i64, d.subsec_micros() as i32))
            .unwrap_or((0, 0));
        self.log.push(Observation::Clock { epoch: now.0, micros: now.1 });
        now
    }

    /// An environment variable.
    pub fn env(&mut self, name: &[u8]) -> Option<Vec<u8>> {
        let v = platform::env_var(name);
        self.log.push(Observation::Env { name: name.to_vec(), value: v.clone() });
        v
    }

    /// `\write18`: runs a command through the shell.  A hard taint.
    pub fn shell(&mut self, command: &[u8]) -> i32 {
        self.mark_taint(Taint::Hard);
        let status = platform::shell(command);
        self.log.push(Observation::Shell { command: command.to_vec(), status });
        status
    }

    /// A line from the terminal, without its terminator; `None` at end of file.  A hard taint.
    pub fn terminal_read_line(&mut self) -> Option<Vec<u8>> {
        self.mark_taint(Taint::Hard);
        self.log.push(Observation::Terminal);
        let mut line = Vec::new();
        let stdin = std::io::stdin();
        let mut lock = stdin.lock();
        let mut b = [0u8; 1];
        loop {
            match lock.read(&mut b) {
                Ok(0) | Err(_) => {
                    if line.is_empty() {
                        return None;
                    }
                    return Some(line);
                }
                Ok(_) => {
                    if b[0] == b'\n' {
                        return Some(line);
                    }
                    line.push(b[0]);
                }
            }
        }
    }

    /// Writes to the terminal.  Output to the terminal is not an observation.
    pub fn terminal_write(&mut self, bytes: &[u8]) {
        let stdout = std::io::stdout();
        let mut lock = stdout.lock();
        let _ = lock.write_all(bytes);
        let _ = lock.flush();
    }

    /// Ends the process.
    pub fn exit(&mut self, code: i32) -> ! {
        std::process::exit(code)
    }

    /// `SOURCE_DATE_EPOCH`, once; texmfmp.c `init_start_time`.
    fn start_time(&mut self) -> i64 {
        if let Some(t) = self.start_time {
            return t;
        }
        let t = match self.env(b"SOURCE_DATE_EPOCH") {
            Some(v) => std::str::from_utf8(&v).ok().and_then(|s| s.parse::<i64>().ok()).unwrap_or_else(|| self.clock().0),
            None => self.clock().0,
        };
        self.start_time = Some(t);
        t
    }

    /// texmfmp.c `initstarttime`, §1584: fixes the start time at the start of the run, so that a later
    /// `\pdfcreationdate` does not move it.
    pub fn init_start_time(&mut self) {
        let _ = self.start_time();
    }

    /// texmfmp.c `get_date_and_time`: `(minutes, day, month, year)`, honouring `FORCE_SOURCE_DATE`.
    pub fn date_and_time(&mut self) -> (i32, i32, i32, i32) {
        let tm = if self.env(b"FORCE_SOURCE_DATE").as_deref() == Some(b"1") {
            let t = self.start_time();
            platform::broken_time(t, true)
        } else {
            let (t, _) = self.clock();
            platform::broken_time(t, false)
        };
        (tm.hour * 60 + tm.minute, tm.day, tm.month, tm.year)
    }
}

// ---------------------------------------------------------------------------
// Files.
// ---------------------------------------------------------------------------

/// An input file: the whole contents, its path, and the logical position.
pub struct InFile {
    pub path: Vec<u8>,
    pub data: Vec<u8>,
    pub pos: usize,
}

impl InFile {
    /// One byte, or -1 at end of file: `getc`.
    #[inline(always)]
    pub fn getc(&mut self) -> i32 {
        match self.data.get(self.pos) {
            Some(&b) => {
                self.pos += 1;
                b as i32
            }
            None => -1,
        }
    }
    #[inline(always)]
    pub fn eof(&self) -> bool {
        self.pos >= self.data.len()
    }
}

/// An output stream with its own buffer, and optionally its whole history, the journal.
pub struct OutFile {
    pub path: Vec<u8>,
    file: Option<std::fs::File>,
    buf: Vec<u8>,
    /// Bytes written so far: the logical position.
    pub total: u64,
    /// All bytes, when journalling: re-materialised on snapshot restore.
    pub journal: Option<Vec<u8>>,
}

const OUT_BUF: usize = 1 << 16;

impl OutFile {
    fn create(path: &[u8], journal: bool) -> Option<OutFile> {
        let file = std::fs::File::create(path_of(path)).ok()?;
        Some(OutFile {
            path: path.to_vec(),
            file: Some(file),
            buf: Vec::with_capacity(OUT_BUF),
            total: 0,
            journal: if journal { Some(Vec::new()) } else { None },
        })
    }
    #[inline(always)]
    pub fn byte(&mut self, b: u8) {
        if self.buf.len() >= OUT_BUF {
            self.flush();
        }
        self.buf.push(b);
        self.total += 1;
        if let Some(j) = &mut self.journal {
            j.push(b);
        }
    }
    pub fn bytes(&mut self, s: &[u8]) {
        if self.buf.len() + s.len() > OUT_BUF {
            self.flush();
        }
        if s.len() > OUT_BUF {
            if let Some(f) = &mut self.file {
                let _ = f.write_all(s);
            }
        } else {
            self.buf.extend_from_slice(s);
        }
        self.total += s.len() as u64;
        if let Some(j) = &mut self.journal {
            j.extend_from_slice(s);
        }
    }
    pub fn flush(&mut self) {
        if !self.buf.is_empty() {
            if let Some(f) = &mut self.file {
                let _ = f.write_all(&self.buf);
            }
            self.buf.clear();
        }
    }
}

impl Drop for OutFile {
    fn drop(&mut self) {
        self.flush();
    }
}

/// A placeholder in the PDF chunk list, filled later.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Deferred {
    chunk: usize,
    at: usize,
    len: usize,
}

/// PDF output: a list of chunks with deferred fields and no seek-back.  Bytes are appended; a field
/// whose value is not yet known, such as `/Length`, is reserved with [`PdfOut::defer`] and filled
/// with [`PdfOut::fill`].  Worker threads can later produce chunks in order without changing this.
#[derive(Default)]
pub struct PdfOut {
    pub path: Vec<u8>,
    chunks: Vec<Vec<u8>>,
    /// Bytes in chunks before the last one, plus the last.
    pub total: u64,
}

const PDF_CHUNK: usize = 1 << 20;

impl PdfOut {
    pub fn new(path: &[u8]) -> PdfOut {
        PdfOut { path: path.to_vec(), chunks: Vec::new(), total: 0 }
    }
    /// The offset the next byte will have: `pdf_offset`.
    #[inline(always)]
    pub fn offset(&self) -> i64 {
        self.total as i64
    }
    fn last(&mut self, need: usize) -> &mut Vec<u8> {
        let open = self.chunks.last().map_or(false, |c| c.len() + need <= c.capacity());
        if !open {
            self.chunks.push(Vec::with_capacity(PDF_CHUNK.max(need)));
        }
        self.chunks.last_mut().expect("just pushed")
    }
    /// Appends bytes.
    pub fn write(&mut self, s: &[u8]) {
        self.last(s.len()).extend_from_slice(s);
        self.total += s.len() as u64;
    }
    /// Reserves `len` bytes, filled with spaces until [`PdfOut::fill`].
    pub fn defer(&mut self, len: usize) -> Deferred {
        let c = self.last(len);
        let at = c.len();
        c.resize(at + len, b' ');
        self.total += len as u64;
        Deferred { chunk: self.chunks.len() - 1, at, len }
    }
    /// Fills a deferred field, padded on the right with spaces.
    pub fn fill(&mut self, d: Deferred, s: &[u8]) {
        assert!(s.len() <= d.len, "deferred field too small");
        let c = &mut self.chunks[d.chunk];
        c[d.at..d.at + s.len()].copy_from_slice(s);
    }
    /// Writes every chunk to the file, publishing by temporary and rename.
    pub fn finish(&mut self) -> std::io::Result<()> {
        let mut tmp = self.path.clone();
        tmp.extend_from_slice(b".rstex-tmp");
        {
            let mut f = std::fs::File::create(path_of(&tmp))?;
            for c in &self.chunks {
                f.write_all(c)?;
            }
        }
        platform::publish(&tmp, &self.path)
    }
    /// The whole output, for tests.
    pub fn contents(&self) -> Vec<u8> {
        self.chunks.concat()
    }
}

/// One slot of the file table.
#[derive(Default)]
pub enum Slot {
    #[default]
    Closed,
    In(InFile),
    Out(OutFile),
    Pdf(PdfOut),
}

// ---------------------------------------------------------------------------
// Host.
// ---------------------------------------------------------------------------

/// Everything that is not POD.
pub struct Host {
    pub world: World,
    /// §79: help lines are static strings.  `help_ptr` stays in the image.
    pub help_line: [&'static [u8]; 6],
    pub files: Vec<Slot>,
    /// Text for the terminal not yet written.
    pub term_buf: Vec<u8>,
    /// The first line given on the command line, read by `t_open_in`.
    pub command_line: Vec<u8>,
    /// `-output-comment`, §1869.
    pub output_comment: Option<Vec<u8>>,
    /// `TEX_format_default`, §546: the name of the format file.
    pub tex_format_default: Vec<u8>,
    /// `translate_filename`: printed in the banner.  The table itself is the hardcoded cp227 one.
    pub translate_filename: Option<Vec<u8>>,
    /// `fullnameoffile` of openclose.c.
    pub full_name_of_file: Option<Vec<u8>>,
    /// True if output streams keep their bytes.
    pub journal_outputs: bool,
    pub hv: HostVars,
}

/// Per-module non-POD state.  Each `HVars` is declared in its module's own file.
#[derive(Default)]
pub struct HostVars {
    pub print: crate::print::HVars,
    pub strings: crate::strings::HVars,
    pub errors: crate::errors::HVars,
    pub arith: crate::arith::HVars,
    pub mem: crate::mem::HVars,
    pub nodes: crate::nodes::HVars,
    pub show: crate::show::HVars,
    pub copy: crate::copy::HVars,
    pub eqtb: crate::eqtb::HVars,
    pub nest: crate::nest::HVars,
    pub hash: crate::hash::HVars,
    pub save: crate::save::HVars,
    pub toks: crate::toks::HVars,
    pub input: crate::input::HVars,
    pub getnext: crate::getnext::HVars,
    pub expand: crate::expand::HVars,
    pub macrocall: crate::macrocall::HVars,
    pub scan: crate::scan::HVars,
    pub buildtoks: crate::buildtoks::HVars,
    pub cond: crate::cond::HVars,
    pub files: crate::files::HVars,
    pub tfm: crate::tfm::HVars,
    pub dvi: crate::dvi::HVars,
    pub pdfout: crate::pdfout::HVars,
    pub pdfobj: crate::pdfobj::HVars,
    pub pdffont: crate::pdffont::HVars,
    pub pdfship: crate::pdfship::HVars,
    pub pack: crate::pack::HVars,
    pub mathdata: crate::mathdata::HVars,
    pub mlist: crate::mlist::HVars,
    pub align: crate::align::HVars,
    pub linebreak: crate::linebreak::HVars,
    pub hyph: crate::hyph::HVars,
    pub page: crate::page::HVars,
    pub control: crate::control::HVars,
    pub build: crate::build::HVars,
    pub mathbuild: crate::mathbuild::HVars,
    pub prefixed: crate::prefixed::HVars,
    pub image: crate::image::HVars,
    pub main: crate::main::HVars,
    pub ext: crate::ext::HVars,
    pub etex: crate::etex::HVars,
    pub web2c: crate::web2c::HVars,
}

impl Host {
    pub fn new() -> Host {
        let mut files = Vec::with_capacity(NUM_FILES);
        files.resize_with(NUM_FILES, Slot::default);
        Host {
            world: World::new(),
            help_line: [b""; 6],
            files,
            term_buf: Vec::with_capacity(1 << 12),
            command_line: Vec::new(),
            output_comment: None,
            tex_format_default: Vec::new(),
            translate_filename: None,
            full_name_of_file: None,
            journal_outputs: false,
            hv: HostVars::default(),
        }
    }

    /// `update_terminal`, §34.
    pub fn term_flush(&mut self) {
        if !self.term_buf.is_empty() {
            let b = core::mem::take(&mut self.term_buf);
            self.world.terminal_write(&b);
            self.term_buf = b;
            self.term_buf.clear();
        }
    }

    #[inline(always)]
    fn out(&mut self, f: FileId) -> Option<&mut OutFile> {
        match &mut self.files[f as usize] {
            Slot::Out(o) => Some(o),
            _ => None,
        }
    }

    /// The input file in a slot.  Reading a closed file is `getc` on a null `FILE*` in the oracle:
    /// a crash; here it is an end of file.
    #[inline(always)]
    pub fn in_file(&mut self, f: FileId) -> Option<&mut InFile> {
        match &mut self.files[f as usize] {
            Slot::In(i) => Some(i),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// The engine's use of it: Part 3's file operations and the print targets.
// ---------------------------------------------------------------------------

impl Tex {
    /// `cur_file`, §326: `input_file[index]`.
    #[inline(always)]
    pub fn cur_file(&self) -> FileId {
        input_file(self.cur_input.index_field)
    }

    /// The current `name_of_file[1..=name_length]`.
    pub fn name_of_file_bytes(&self) -> Vec<u8> {
        self.name_of_file.range(1, self.name_length + 1).to_vec()
    }

    /// Replaces `name_of_file`, as `open_input` does when a search finds a longer path.
    pub fn set_name_of_file(&mut self, s: &[u8]) {
        let n = s.len().min(FILE_NAME_CAP - 2);
        self.name_of_file.as_mut_slice()[1..1 + n].copy_from_slice(&s[..n]);
        self.name_of_file[n as i32 + 1] = 0;
        self.name_length = n as i32;
    }

    // ---- print targets ----

    /// `wterm(c)` for one character.
    #[inline(always)]
    pub fn wterm_char(&mut self, c: i32) {
        self.host.term_buf.push(c as u8);
        if self.host.term_buf.len() >= 1 << 12 {
            self.host.term_flush();
        }
    }
    /// `wterm('abc')`.
    pub fn wterm_s(&mut self, s: &[u8]) {
        self.host.term_buf.extend_from_slice(s);
        if self.host.term_buf.len() >= 1 << 12 {
            self.host.term_flush();
        }
    }
    /// `wterm_ln('abc')`.
    pub fn wterm_ln_s(&mut self, s: &[u8]) {
        self.wterm_s(s);
        self.wterm_cr();
    }
    /// `wterm_cr`.
    #[inline]
    pub fn wterm_cr(&mut self) {
        self.wterm_char(b'\n' as i32);
    }
    /// `wlog(c)` for one character.
    #[inline(always)]
    pub fn wlog_char(&mut self, c: i32) {
        if let Some(o) = self.host.out(LOG_FILE) {
            o.byte(c as u8);
        }
    }
    /// `wlog('abc')`.
    pub fn wlog_s(&mut self, s: &[u8]) {
        if let Some(o) = self.host.out(LOG_FILE) {
            o.bytes(s);
        }
    }
    /// `wlog_ln('abc')`.
    pub fn wlog_ln_s(&mut self, s: &[u8]) {
        self.wlog_s(s);
        self.wlog_cr();
    }
    /// `wlog_cr`.
    #[inline]
    pub fn wlog_cr(&mut self) {
        self.wlog_char(b'\n' as i32);
    }
    /// `write(write_file[j], c)`: an `\openout` stream.
    #[inline(always)]
    pub fn write_file_char(&mut self, j: i32, c: i32) {
        if let Some(o) = self.host.out(write_file(j)) {
            o.byte(c as u8);
        }
    }
    /// `write_ln(write_file[j])`.
    #[inline]
    pub fn write_file_ln(&mut self, j: i32) {
        self.write_file_char(j, b'\n' as i32);
    }

    // ---- opening and closing ----

    fn open_in(&mut self, f: FileId, format: i32) -> bool {
        let name = self.name_of_file_bytes();
        // Look in -output-directory first, if the name is not absolute: lib/openclose.c.
        let mut found: Option<Vec<u8>> = None;
        if let Some(dir) = self.host.world.output_directory.clone() {
            if !platform::is_absolute(&name) {
                let mut p = dir;
                p.push(b'/');
                p.extend_from_slice(&name);
                if self.host.world.is_file(&p) {
                    found = Some(p);
                }
            }
        }
        let path = match found {
            Some(p) => p,
            None => {
                let must_exist = (format != KPSE_TEX_FORMAT || self.tex_input_type != 0) && format != KPSE_VF_FORMAT;
                match self.host.world.lookup(&name, format, must_exist) {
                    Some(p) => p,
                    None => return false,
                }
            }
        };
        let Some(data) = self.host.world.read(&path) else {
            return false;
        };
        self.host.full_name_of_file = Some(path.clone());
        // Drop a leading ./ unless the user said it: lib/openclose.c.
        let mut shown: &[u8] = &path;
        if shown.starts_with(b"./") && !name.starts_with(b"./") {
            shown = &shown[2..];
        }
        let shown = shown.to_vec();
        self.set_name_of_file(&shown);
        self.host.files[f as usize] = Slot::In(InFile { path, data, pos: 0 });
        true
    }

    fn open_out(&mut self, f: FileId) -> bool {
        let name = self.name_of_file_bytes();
        let mut path = name.clone();
        if let Some(dir) = &self.host.world.output_directory {
            if !platform::is_absolute(&name) {
                path = dir.clone();
                path.push(b'/');
                path.extend_from_slice(&name);
            }
        }
        if f == PDF_FILE {
            // The chunk list publishes by rename at the end; check now that it can be created.
            if OutFile::create(&path, false).is_none() {
                return false;
            }
            self.host.files[f as usize] = Slot::Pdf(PdfOut::new(&path));
        } else {
            let journal = self.host.journal_outputs;
            match OutFile::create(&path, journal) {
                Some(o) => self.host.files[f as usize] = Slot::Out(o),
                None => return false,
            }
        }
        if path != name {
            self.set_name_of_file(&path);
        }
        true
    }

    /// `a_open_in(f, format)`, texmfmp.h `open_in_or_pipe`.  Pipes are not supported yet.
    pub fn a_open_in(&mut self, f: FileId, format: i32) -> bool {
        self.open_in(f, format)
    }
    /// `a_open_out(f)`.
    pub fn a_open_out(&mut self, f: FileId) -> bool {
        self.open_out(f)
    }
    /// `b_open_in(f)`: a TFM file.
    pub fn b_open_in(&mut self, f: FileId) -> bool {
        self.open_in(f, KPSE_TFM_FORMAT)
    }
    /// `b_open_out(f)`: a DVI or PDF file.
    pub fn b_open_out(&mut self, f: FileId) -> bool {
        self.open_out(f)
    }
    /// `w_open_in(f)`: a format file.
    pub fn w_open_in(&mut self, f: FileId) -> bool {
        self.open_in(f, KPSE_FMT_FORMAT)
    }
    /// `w_open_out(f)`.
    pub fn w_open_out(&mut self, f: FileId) -> bool {
        self.open_out(f)
    }
    /// `tex_b_openin(f)`: a binary file found on the TeX path.
    pub fn tex_b_openin(&mut self, f: FileId) -> bool {
        self.open_in(f, KPSE_TEX_FORMAT)
    }
    /// `vf_b_open_in(f)`.
    pub fn vf_b_open_in(&mut self, f: FileId) -> bool {
        self.open_in(f, KPSE_VF_FORMAT)
    }
    /// `a_close(f)`; closing a closed file is harmless.
    pub fn a_close(&mut self, f: FileId) {
        self.host.files[f as usize] = Slot::Closed;
    }
    /// `b_close(f)`.  The PDF chunk list is finished by its owner before closing.
    pub fn b_close(&mut self, f: FileId) {
        self.host.files[f as usize] = Slot::Closed;
    }
    /// `w_close(f)`.
    pub fn w_close(&mut self, f: FileId) {
        self.host.files[f as usize] = Slot::Closed;
    }
    /// `kpse_in_name_ok(name_of_file)`.
    pub fn kpse_in_name_ok(&mut self) -> bool {
        let n = self.name_of_file_bytes();
        rstex_kpse::in_name_ok(&n)
    }
    /// `kpse_out_name_ok(name_of_file)`.
    pub fn kpse_out_name_ok(&mut self) -> bool {
        let n = self.name_of_file_bytes();
        rstex_kpse::out_name_ok(&n)
    }
    /// `texmf_yesno(var)`: a boolean from `texmf.cnf`.
    pub fn texmf_yesno(&mut self, var: &[u8]) -> bool {
        rstex_kpse::yesno(var)
    }

    // ---- reading ----

    /// `getc(f)`: one byte, -1 at end of file.
    #[inline(always)]
    pub fn getc(&mut self, f: FileId) -> i32 {
        match self.host.in_file(f) {
            Some(i) => i.getc(),
            None => -1,
        }
    }
    /// `feof(f)`, `eof(f)`.
    pub fn feof(&mut self, f: FileId) -> bool {
        match self.host.in_file(f) {
            Some(i) => i.eof(),
            None => true,
        }
    }

    /// §31 `input_ln`, as texmfmp.c `input_line`: reads a line into `buffer[first..]`, sets `last`,
    /// removes trailing spaces (only spaces), recognises LF, CR and CRLF, and applies `xord`.
    pub fn input_ln(&mut self, f: FileId, _bypass_eoln: bool) -> bool {
        let first = self.first as usize;
        let size = self.buf_size as usize;
        self.last = self.first;
        let mut last = first;
        if f == TERM_IN {
            self.host.term_flush();
            let Some(line) = self.host.world.terminal_read_line() else {
                return false;
            };
            let line = match line.iter().position(|&c| c == b'\r') {
                Some(k) => &line[..k],
                None => &line[..],
            };
            let n = line.len().min(size.saturating_sub(first));
            self.buffer.as_mut_slice()[first..first + n].copy_from_slice(&line[..n]);
            last += n;
            if n < line.len() {
                self.line_too_long();
            }
        } else {
            let Some(i) = self.host.in_file(f) else {
                return false;
            };
            let data = &i.data[i.pos..];
            if data.is_empty() {
                return false;
            }
            let n = data.iter().position(|&c| c == b'\n' || c == b'\r').unwrap_or(data.len());
            if n > size.saturating_sub(first) {
                self.line_too_long();
            }
            let (term, mut next) = (data.get(n).copied(), i.pos + n);
            self.buffer.as_mut_slice()[first..first + n].copy_from_slice(&data[..n]);
            last += n;
            if term.is_some() {
                next += 1;
                if term == Some(b'\r') && i.data.get(next) == Some(&b'\n') {
                    next += 1;
                }
            }
            i.pos = next;
        }
        self.buffer[last as i32] = b' ';
        if last as i32 >= self.max_buf_stack {
            self.max_buf_stack = last as i32;
        }
        while last > first && self.buffer[last as i32 - 1] == b' ' {
            last -= 1;
        }
        for k in first..=last {
            let c = self.buffer.at(k as i32);
            let x = self.xord[c];
            self.buffer[k as i32] = x;
        }
        self.last = last as i32;
        true
    }

    #[cold]
    fn line_too_long(&mut self) -> ! {
        self.host.term_flush();
        let msg = format!(
            "! Unable to read an entire line---bufsize={}.\nPlease increase buf_size in texmf.cnf.\n",
            self.buf_size
        );
        self.host.world.terminal_write(msg.as_bytes());
        self.uexit(1)
    }

    // ---- writing ----

    /// `put_byte(b, f)`, §1870, and every other byte written to a binary stream.
    #[inline]
    pub fn put_byte(&mut self, b: i32, f: FileId) {
        match &mut self.host.files[f as usize] {
            Slot::Out(o) => o.byte(b as u8),
            Slot::Pdf(p) => p.write(&[b as u8]),
            _ => {}
        }
    }

    /// Bytes to a stream, in one piece.
    pub fn put_bytes(&mut self, f: FileId, s: &[u8]) {
        match &mut self.host.files[f as usize] {
            Slot::Out(o) => o.bytes(s),
            Slot::Pdf(p) => p.write(s),
            _ => {}
        }
    }

    /// The PDF chunk list, while the PDF file is open.
    pub fn pdf_out_file(&mut self) -> Option<&mut PdfOut> {
        match &mut self.host.files[PDF_FILE as usize] {
            Slot::Pdf(p) => Some(p),
            _ => None,
        }
    }

    // ---- the rest of the world ----

    /// `date_and_time(time,day,month,year)`, §241: returns `(time, day, month, year)`.
    pub fn date_and_time(&mut self) -> (i32, i32, i32, i32) {
        self.host.world.date_and_time()
    }
    /// `seconds_and_micros(epochseconds,microseconds)`, §1584.
    pub fn seconds_and_micros(&mut self) {
        let (s, u) = self.host.world.clock();
        self.epochseconds = s as i32;
        self.microseconds = u;
    }
    /// `runsystem(cmd)`, §1617.
    pub fn runsystem(&mut self, cmd: &[u8]) -> i32 {
        self.host.term_flush();
        self.host.world.shell(cmd)
    }
    /// `uexit(code)`, §81.
    pub fn uexit(&mut self, code: i32) -> ! {
        self.host.term_flush();
        // Dropping the slots flushes every output stream.
        for s in self.host.files.iter_mut() {
            *s = Slot::Closed;
        }
        self.host.world.exit(code)
    }
    /// `t_open_in`, §33: the first line comes from the command line.
    pub fn t_open_in(&mut self) {
        let line = core::mem::take(&mut self.host.command_line);
        self.buffer[self.first] = 0;
        let first = self.first as usize;
        let n = line.len().min(self.buf_size as usize - first);
        self.buffer.as_mut_slice()[first..first + n].copy_from_slice(&line[..n]);
        let mut last = first + n;
        while last > first && self.buffer[last as i32 - 1] == b' ' {
            last -= 1;
        }
        self.last = last as i32;
        for k in first..last {
            let c = self.buffer.at(k as i32);
            let x = self.xord[c];
            self.buffer[k as i32] = x;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch() -> std::path::PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!("rstex-host-test-{}-{:?}", std::process::id(), std::thread::current().id()));
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    fn bytes(p: &std::path::Path) -> Vec<u8> {
        platform::path_to_bytes(p)
    }

    #[test]
    fn world_records_observations() {
        let mut w = World::new();
        assert_eq!(w.taint, Taint::None);
        let _ = w.env(b"RSTEX_SURELY_UNSET_VAR");
        assert!(matches!(w.log.last(), Some(Observation::Env { value: None, .. })));
        let _ = w.clock();
        assert!(matches!(w.log.last(), Some(Observation::Clock { .. })));
        assert_eq!(w.lookup(b"/rstex/surely/not/here.tex", KPSE_TEX_FORMAT, true), None);
        assert!(matches!(w.log.last(), Some(Observation::Lookup { found: None, .. })));
        w.mark_taint(Taint::Soft);
        w.mark_taint(Taint::None);
        assert_eq!(w.taint, Taint::Soft, "taint only rises");
        let st = w.shell(b"exit 3");
        assert_eq!(st, 3);
        assert_eq!(w.taint, Taint::Hard);
    }

    #[test]
    fn pdf_chunks_defer_without_seeking() {
        let mut p = PdfOut::new(b"/tmp/unused.pdf");
        p.write(b"abc ");
        assert_eq!(p.offset(), 4);
        let d = p.defer(6);
        p.write(b" def");
        assert_eq!(p.offset(), 14);
        p.fill(d, b"123");
        assert_eq!(p.contents(), b"abc 123    def");
    }

    #[test]
    fn input_ln_handles_terminators_and_trailing_spaces() {
        let t = Tex::alloc();
        for i in 0..256 {
            t.xord[i] = i as u8;
        }
        t.first = 1;
        t.buf_size = 1000;
        let dir = scratch();
        let f = dir.join("a.tex");
        std::fs::write(&f, b"one  \r\ntwo\rthree\n\nlast").unwrap();
        t.set_name_of_file(&bytes(&f));
        assert!(t.a_open_in(input_file(1), KPSE_TEX_FORMAT));
        let mut lines = Vec::new();
        while t.input_ln(input_file(1), true) {
            lines.push(t.buffer.range(t.first, t.last).to_vec());
        }
        assert_eq!(lines, vec![&b"one"[..], b"two", b"three", b"", b"last"]);
        assert!(t.feof(input_file(1)));
        std::fs::remove_dir_all(&dir).unwrap();
        Tex::dealloc(t);
    }

    #[test]
    fn output_routes_through_host() {
        let t = Tex::alloc();
        let dir = scratch();
        let f = dir.join("o.log");
        t.set_name_of_file(&bytes(&f));
        t.host.journal_outputs = true;
        assert!(t.a_open_out(LOG_FILE));
        t.wlog_s(b"abc");
        t.wlog_char(b'd' as i32);
        t.wlog_cr();
        match &t.host.files[LOG_FILE as usize] {
            Slot::Out(o) => {
                assert_eq!(o.total, 5);
                assert_eq!(o.journal.as_deref(), Some(&b"abcd\n"[..]));
            }
            _ => panic!("log not open"),
        }
        t.a_close(LOG_FILE);
        assert_eq!(std::fs::read(&f).unwrap(), b"abcd\n");
        std::fs::remove_dir_all(&dir).unwrap();
        Tex::dealloc(t);
    }

    #[test]
    fn section_table_is_consistent() {
        let mut v: Vec<&PodSection> = POD_SECTIONS.iter().collect();
        assert_eq!(v[0].name, "prefix");
        v.sort_by_key(|s| s.offset);
        let host_at = core::mem::offset_of!(Tex, host);
        let mut end = 0usize;
        for s in v.iter() {
            // mem appears twice, as mem_lo and mem_hi over one array.
            if s.name == "mem_hi" {
                continue;
            }
            assert!(s.offset >= end, "{} overlaps its predecessor", s.name);
            assert!(s.offset - end < 4096, "gap before {}", s.name);
            end = s.offset + s.elem_size * s.cap;
        }
        assert!(end <= host_at);
        assert!(host_at - end < 4096, "the table misses the end of the image");
        let t = Tex::alloc();
        t.toks_init();
        for s in POD_SECTIONS {
            let (a, b) = (s.live)(t);
            assert!(a <= b || s.name == "mem_hi", "{}: live range {}..{}", s.name, a, b);
            assert!(b <= s.cap, "{}: live range ends past capacity: {} > {}", s.name, b, s.cap);
        }
        Tex::dealloc(t);
    }

    /// A format load must not touch what §1517 has already set up: docs/PORTING.md, image.rs.
    #[test]
    fn format_sections_leave_the_live_state_alone() {
        let in_format = |name: &str| POD_SECTIONS.iter().find(|s| s.name == name).unwrap().in_format;
        for live in ["prefix", "buffer", "input_stack", "nest", "save_stack", "param_stack", "tb", "tx", "name_of_file", "dvi_buf", "pdf_os_buf", "dest_names", "xeq_level", "font_used", "pdf_font_type", "pdf_char_used"] {
            assert!(!in_format(live), "{} is live during §1517 or set up by it, so a format must not hold it", live);
        }
        for dumped in ["eqtb", "toks", "hash", "hash_tab", "mem_lo", "mem_hi", "str_pool", "str_start", "font_info", "font_glue", "hyf_next", "hyph_list", "trie_trc", "pdf_mem", "obj_tab", "arena"] {
            assert!(in_format(dumped), "{} is dumped by §1485-1507, or replaces what is", dumped);
        }
    }
}
