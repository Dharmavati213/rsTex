//! Host and World, DESIGN.md section 6: everything that is not POD, and every side effect.
//!
//! - **World** is the only path to the operating system.  While a checkpoint may yet be taken, each lookup, read, stat
//!   and variable is recorded as an [`Observation`]; the clock, the shell and the terminal mark the run tainted instead.
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
//! Host is the last field of `Tex`, constructed in place, and never snapshotted.  What differs by
//! operating system is in `platform/`; there is no `cfg` here.
//!
//! # What whoever starts the engine has to do
//!
//! This is what `maininit` and `parse_options` of texmfmp.c leave behind.  Nothing here happens by itself, and all
//! of it comes before the first lookup:
//!
//! - [`World::set_program_name`] with `-progname`, or else the dump name: `TEXINPUTS.pdflatex` depends on it.
//! - [`World::init_output_directory`] with `-output-directory`, or `None`; [`World::cnf_line`] for each `-cnf-line`.
//! - `format_default_length` and `host.tex_format_default`; `shellenabledp` and `restrictedshell` as
//!   `init_shell_escape` leaves them.  The list of allowed commands is read here, at the first command.
//! - `host.command_line`, `host.c_job_name` (`-jobname`), `host.world.recorder_enabled` (`-recorder`),
//!   `host.output_comment`, `host.translate_filename`.
#![forbid(unsafe_code)]
#![allow(non_snake_case, dead_code)]

use std::io::{BufRead, Read, Write};

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
pub(crate) const READ_FILE_BASE: FileId = 16;
pub(crate) const WRITE_FILE_BASE: FileId = 32;
pub(crate) const INPUT_FILE_BASE: FileId = 64;
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
pub const KPSE_PK_FORMAT: i32 = rstex_kpse::KPSE_PK_FORMAT;

/// `version_string` from web2c's generated header, §61.
#[cfg(not(feature = "oracle-banner"))]
pub const VERSION_STRING: &[u8] = b" (rstex)";
#[cfg(feature = "oracle-banner")]
pub const VERSION_STRING: &[u8] = b" (TeX Live 2026/Arch Linux)";
/// kpathsea's `kpathsea_version_string`: the end of `\pdftexbanner` and of the job id.
pub const KPATHSEA_VERSION_STRING: &[u8] = b"kpathsea version 6.4.2";
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
    /// No checkpoint past this point: `\write18`, pipe input, terminal `\read`, error state.
    Hard,
}

/// One observation of the outside world.  A checkpoint is valid only if each observation in its
/// ordered log would get the same answer now (DESIGN.md section 7).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Observation {
    Lookup { name: Vec<u8>, format: i32, must_exist: bool, found: Option<Vec<u8>> },
    /// `stat` is `None` if there was nothing to read.  `prefix` is how much was read, for a file still open at the
    /// checkpoint, and then `hash` is of that much.
    Read { path: Vec<u8>, stat: Option<FileStat>, hash: u64, prefix: Option<u64>, kind: ReadKind },
    /// `size_only`: `\\pdffilesize` looks at no more than that.
    Stat { path: Vec<u8>, stat: Option<FileStat>, size_only: bool },
    /// `cnf`: asked of kpathsea, which looks in the environment and `texmf.cnf`; otherwise of the environment.
    Env { name: Vec<u8>, value: Option<Vec<u8>>, cnf: bool },
    /// The run wrote this file and closed it: a read of it after this is answered by the run's own bytes.
    Output { path: Vec<u8> },
}

/// How a file read is checked again: DESIGN.md section 7.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReadKind {
    /// By its content.
    Project,
    /// In a tree with an `ls-R`: by its stat, then by its content if that differs.
    Tds,
    /// A format: by its stat.
    Format,
}

/// The only path to the operating system.
#[derive(Default)]
pub struct World {
    /// True while a checkpoint may yet be taken: only then is anything kept.
    pub recording: bool,
    pub log: Vec<Observation>,
    pub taint: Taint,
    /// What made the taint hard, for `RSTEX_CHECKPOINT_TRACE`.
    pub taint_reason: &'static str,
    /// The `log` entry of each file slot's `Read`, while recording.
    pub(crate) in_obs: Vec<usize>,
    /// The `\\openout` streams open now, and all those of the run.
    pub(crate) open_outputs: Vec<Vec<u8>>,
    pub(crate) written: Vec<Vec<u8>>,
    /// Files the run wrote and closed, with their bytes, in order.
    pub(crate) closed_outputs: Vec<(Vec<u8>, Vec<u8>)>,
    /// Directories searched before everything else: `-output-directory`.
    pub output_directory: Option<Vec<u8>>,
    /// `SOURCE_DATE_EPOCH`, read once, or the clock when it is not set.
    start_time: Option<i64>,
    /// True if `start_time` came from `SOURCE_DATE_EPOCH`.
    source_date_epoch_set: bool,
    /// `cmdlist` of texmfmp.c: `shell_escape_commands`, under a restricted shell.
    shell_cmdlist: Option<Vec<Vec<u8>>>,
    /// `recorder_enabled` of openclose.c: `-recorder`.
    pub recorder_enabled: bool,
    /// True if the `.fls` file keeps its bytes for a snapshot, as `journal_outputs` for the others.
    pub journal_recorder: bool,
    /// `recorder_name`, `recorder_file`: the `.fls` file, begun at the first thing to record.
    recorder_name: Vec<u8>,
    recorder_file: Option<OutFile>,
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

    /// [`World::mark_taint`], with the reason a checkpoint was refused.
    pub fn mark_taint_because(&mut self, t: Taint, why: &'static str) {
        if t > self.taint {
            self.taint = t;
            self.taint_reason = why;
        }
    }

    // ---- kpathsea ----

    /// Finds a file through the search path; a negative answer is an observation too.
    pub fn lookup(&mut self, name: &[u8], format: i32, must_exist: bool) -> Option<Vec<u8>> {
        let _p = crate::prof_scope!("kpse lookup");
        let found = rstex_kpse::find_file(name, format, must_exist);
        if self.recording {
            self.log.push(Observation::Lookup { name: name.to_vec(), format, must_exist, found: found.clone() });
        }
        self.print_kpse_warnings();
        found
    }

    /// `kpse_find_pk`, which may run `mktexpk`.  Fonts are written after the page that needs them, and a page shipped out refuses a checkpoint, so this is not in
    /// the log, whose lookups a restore makes again: a run that gets here before `\document` is taking no checkpoint.
    pub fn find_pk(&mut self, name: &[u8], dpi: i32, discard_errors: bool) -> Option<(Vec<u8>, rstex_kpse::GlyphFile)> {
        let _p = crate::prof_scope!("kpse lookup");
        rstex_kpse::set_make_tex_discard_errors(discard_errors);
        let found = rstex_kpse::find_glyph(name, dpi as u32, KPSE_PK_FORMAT);
        self.mark_taint_because(Taint::Hard, "a bitmap font was looked for");
        self.print_kpse_warnings();
        found
    }

    /// What C's `WARNING` prints at once; a leaf crate only keeps it.
    fn print_kpse_warnings(&mut self) {
        for w in rstex_kpse::take_warnings() {
            self.stderr_write(&w);
        }
    }

    /// `kpse_var_value`.  Its answer is an observation, as an environment variable's is.
    pub fn var_value(&mut self, var: &[u8]) -> Option<Vec<u8>> {
        let value = rstex_kpse::var_value(var);
        if self.recording {
            self.log.push(Observation::Env { name: var.to_vec(), value: value.clone(), cnf: true });
        }
        self.print_kpse_warnings();
        value
    }

    /// `texmf_yesno`: a boolean from `texmf.cnf` or the environment.
    pub fn yesno(&mut self, var: &[u8]) -> bool {
        matches!(self.var_value(var).as_deref().and_then(|v| v.first()), Some(b't' | b'y' | b'1'))
    }

    /// `setupboundvariable`: §1512's variables, from `texmf.cnf` or the environment, or `dflt`.
    pub fn setup_bound_var(&mut self, var: &[u8], dflt: i32) -> i32 {
        rstex_kpse::setup_bound_var(var, dflt)
    }

    /// `kpse_reset_program_name`: `-progname`, else the dump name.
    pub fn set_program_name(&mut self, name: &[u8]) {
        rstex_kpse::set_program_name(name);
    }

    /// The end of `parse_options`: `-output-directory` goes into `TEXMF_OUTPUT_DIRECTORY`, for
    /// `\write18` children and the output name checks, and without the option that variable is the option.
    pub fn init_output_directory(&mut self, option: Option<&[u8]>) {
        match option {
            Some(d) => {
                rstex_kpse::set_var(b"TEXMF_OUTPUT_DIRECTORY", d);
                self.output_directory = Some(d.to_vec());
            }
            None => self.output_directory = self.env(b"TEXMF_OUTPUT_DIRECTORY"),
        }
    }

    /// `-cnf-line`.
    pub fn cnf_line(&mut self, line: &[u8]) {
        rstex_kpse::cnf_line_env_progname(line);
        self.print_kpse_warnings();
    }

    /// `kpse_init_prog`: `mode` is the `MAKETEX_MODE` given, or `None`.
    pub fn init_prog(&mut self, prefix: &[u8], dpi: i32, mode: Option<&[u8]>) {
        rstex_kpse::init_prog(prefix, dpi as u32, mode);
    }

    /// `mk_shellcmdlist`: `shell_escape_commands = foo,bar,...`, with no spaces around the commas.
    pub fn set_shell_commands(&mut self, v: &[u8]) {
        let mut l: Vec<Vec<u8>> = v.split(|&c| c == b',').map(<[u8]>::to_vec).collect();
        // Nothing after the last comma is no command.
        if l.last().is_some_and(Vec::is_empty) {
            l.pop();
        }
        self.shell_cmdlist = Some(l);
    }

    /// [`shell_cmd_is_allowed`] with the list of `init_shell_escape`, which it makes at the first command:
    /// only under a restricted shell does anyone ask.
    pub fn shell_cmd_is_allowed(&mut self, cmd: &[u8]) -> (i32, Vec<u8>, Vec<u8>) {
        if self.shell_cmdlist.is_none() {
            if let Some(v) = self.var_value(b"shell_escape_commands") {
                self.set_shell_commands(&v);
            }
        }
        shell_cmd_is_allowed(cmd, self.shell_cmdlist.as_deref())
    }

    // ---- the recorder, openclose.c ----

    /// `recorder_start`.  A name with the process id at first, since the log's name is not known yet.
    fn recorder_start(&mut self) {
        let mut name = rstex_kpse::with_global(|k| k.program_name().to_vec());
        name.extend_from_slice(std::process::id().to_string().as_bytes());
        name.extend_from_slice(b".fls");
        if let Some(dir) = &self.output_directory {
            name = [&dir[..], b"/", &name].concat();
        }
        let Some(mut f) = OutFile::create(&name, self.journal_recorder) else {
            let msg = [&rstex_kpse::invocation_name()[..], b": fatal: cannot open ", &name, b" for writing.\n"].concat();
            self.stderr_write(&msg);
            self.exit(1)
        };
        let cwd = std::env::current_dir().map(|d| platform::path_to_bytes(&d)).unwrap_or_default();
        f.bytes(b"PWD ");
        f.bytes(&cwd);
        f.byte(b'\n');
        self.recorder_name = name;
        self.recorder_file = Some(f);
    }

    /// `recorder_change_filename`: the name of the log with `.fls`, once it is known.  `-output-directory` is
    /// added here, since `new_name` does not have it.  In C the file exists by then, since `texmf.cnf` was
    /// recorded at the start of the run.
    pub fn recorder_change_filename(&mut self, new_name: &[u8]) {
        if self.recorder_enabled && self.recorder_file.is_none() {
            self.recorder_start();
        }
        let Some(mut old) = self.recorder_file.take() else {
            return;
        };
        let journal = old.journal.take();
        let total = old.total;
        // An open file cannot be renamed on Windows; everywhere it costs one reopening.
        drop(old);
        let new = match &self.output_directory {
            Some(dir) => [&dir[..], b"/", new_name].concat(),
            None => new_name.to_vec(),
        };
        if platform::publish(&self.recorder_name, &new).is_ok() {
            self.recorder_name = new;
        }
        if let Some(mut f) = OutFile::append(&self.recorder_name) {
            f.journal = journal;
            f.total = total;
            self.recorder_file = Some(f);
        }
    }

    /// `recorder_record_name`.  What kpathsea read since the last call comes first, since C records it as
    /// it reads, and that was before the name given here was found.
    fn recorder_record_name(&mut self, prefix: &[u8], name: &[u8]) {
        if !self.recorder_enabled {
            return;
        }
        if self.recorder_file.is_none() {
            self.recorder_start();
        }
        let read_by_kpse = rstex_kpse::take_recorded();
        if let Some(f) = &mut self.recorder_file {
            for r in &read_by_kpse {
                f.bytes(b"INPUT ");
                f.bytes(r);
                f.byte(b'\n');
            }
            f.bytes(prefix);
            f.byte(b' ');
            f.bytes(name);
            f.byte(b'\n');
            f.flush();
        }
    }

    /// `recorder_record_input`.  [`World::read`] and [`World::stat`] do this themselves.
    pub fn recorder_record_input(&mut self, name: &[u8]) {
        self.recorder_record_name(b"INPUT", name);
    }

    /// `recorder_record_output`.
    pub fn recorder_record_output(&mut self, name: &[u8]) {
        self.recorder_record_name(b"OUTPUT", name);
    }

    /// What kpathsea read since the last file recorded goes in now, where it would have gone: at the head of the next.
    pub(crate) fn recorder_flush_pending(&mut self) {
        if self.recorder_enabled && self.recorder_file.is_some() {
            let read_by_kpse = rstex_kpse::take_recorded();
            if let Some(f) = &mut self.recorder_file {
                for r in &read_by_kpse {
                    f.bytes(b"INPUT ");
                    f.bytes(r);
                    f.byte(b'\n');
                }
                f.flush();
            }
        }
    }

    /// The name of the `.fls` file and its bytes so far.
    pub(crate) fn recorder_take_journal(&mut self) -> Option<(Vec<u8>, Vec<u8>)> {
        let j = self.recorder_file.as_mut()?.journal.take()?;
        Some((self.recorder_name.clone(), j))
    }

    pub(crate) fn recorder_drop_journal(&mut self) {
        if let Some(f) = &mut self.recorder_file {
            f.journal = None;
        }
    }

    /// The `.fls` file again, on a restore.
    pub(crate) fn recorder_restore(&mut self, name: Vec<u8>, bytes: &[u8]) -> bool {
        let Some(mut f) = OutFile::create(&name, false) else { return false };
        f.bytes(bytes);
        f.flush();
        self.recorder_name = name;
        self.recorder_file = Some(f);
        true
    }

    /// At the end of the run, what kpathsea read after the last file recorded.
    fn recorder_finish(&mut self) {
        if self.recorder_enabled && self.recorder_file.is_some() {
            let read_by_kpse = rstex_kpse::take_recorded();
            if let Some(f) = &mut self.recorder_file {
                for r in &read_by_kpse {
                    f.bytes(b"INPUT ");
                    f.bytes(r);
                    f.byte(b'\n');
                }
            }
        }
        self.recorder_file = None;
    }

    // ---- files ----

    /// True if `path` names a regular file; it does not count as an observation by itself, since
    /// a later `read` or `stat` records the answer.
    pub fn is_file(&self, path: &[u8]) -> bool {
        std::fs::metadata(path_of(path)).map(|m| m.is_file()).unwrap_or(false)
    }

    /// `remove(path)`: `remove_pdffile`, on a fatal error.
    pub fn remove_file(&mut self, path: &[u8]) -> bool {
        std::fs::remove_file(path_of(path)).is_ok()
    }

    /// Reads a whole file into memory, and records it for the `.fls` file.
    pub fn read(&mut self, path: &[u8]) -> Option<Vec<u8>> {
        let data = self.read_unrecorded(path);
        if data.is_some() {
            self.recorder_record_input(path);
        }
        data
    }

    /// `open_input` records the name as it shows it, without a `./` the user did not write.
    fn read_unrecorded(&mut self, path: &[u8]) -> Option<Vec<u8>> {
        self.read_kind(path, false)
    }

    fn read_kind(&mut self, path: &[u8], is_format: bool) -> Option<Vec<u8>> {
        let _p = crate::prof_scope!("file read");
        // The stat first: if the file changes in between, it is the stat that is old, and a later run sees a difference.
        let stat = if self.recording { platform::file_stat(path) } else { None };
        let data = std::fs::read(path_of(path)).ok();
        if self.recording {
            self.checkpoint_note_read(path, stat, data.as_deref(), is_format);
        }
        data
    }

    /// A file whose contents are in hand some other way counts as read, for the `.fls` file and a checkpoint, which goes by content and is only then worth the read.
    pub fn record_read(&mut self, path: &[u8]) {
        if self.recording {
            let stat = platform::file_stat(path);
            let data = std::fs::read(path_of(path)).ok();
            self.checkpoint_note_read(path, stat, data.as_deref(), false);
        }
        self.recorder_record_input(path);
    }

    /// The contents of a file that has already counted as read.
    pub fn read_quietly(&mut self, path: &[u8]) -> Option<Vec<u8>> {
        std::fs::read(path_of(path)).ok()
    }

    /// What `open_in` puts in the file table for `path`: a format is opened, for `image.rs` to take what it wants, and anything else is read whole.
    fn open_slot(&mut self, path: &[u8], format: i32) -> Option<Slot> {
        if format == KPSE_FMT_FORMAT {
            return self.open_fmt(path).map(Slot::Fmt);
        }
        let data = self.read_unrecorded(path)?;
        Some(Slot::In(InFile { path: path.to_vec(), data, pos: 0 }))
    }

    /// As `read_unrecorded`, for a format, with what it observes: a checkpoint takes a format by its stat, and needs none of its bytes.  A directory opens on Unix, but
    /// reads as nothing.
    fn open_fmt(&mut self, path: &[u8]) -> Option<FmtIn> {
        let _p = crate::prof_scope!("file read");
        let stat = if self.recording { platform::file_stat(path) } else { None };
        let file = std::fs::File::open(path_of(path)).ok();
        let md = file.as_ref().and_then(|f| f.metadata().ok()).filter(|m| !m.is_dir());
        if self.recording {
            self.checkpoint_note_read(path, stat, md.is_some().then_some(&[][..]), true);
        }
        Some(FmtIn { size: md?.len(), file: file? })
    }

    /// `\pdffilemoddate` and the like, which texmfmp.c records too.
    pub fn stat(&mut self, path: &[u8]) -> Option<FileStat> {
        self.stat_kind(path, false)
    }

    /// `\pdffilesize`: of a file the run wrote, the answer is the run's own too.
    pub fn stat_size(&mut self, path: &[u8]) -> Option<FileStat> {
        self.stat_kind(path, true)
    }

    fn stat_kind(&mut self, path: &[u8], size_only: bool) -> Option<FileStat> {
        let st = platform::file_stat(path);
        if self.recording {
            if !size_only && self.checkpoint_is_written(path) {
                self.mark_taint_because(Taint::Hard, "the modification date of a file the run wrote");
            }
            self.log.push(Observation::Stat { path: path.to_vec(), stat: st.clone(), size_only });
        }
        if st.is_some() {
            self.recorder_record_input(path);
        }
        st
    }

    /// The time now, as seconds since the epoch and microseconds.
    pub fn clock(&mut self) -> (i64, i32) {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| (d.as_secs() as i64, d.subsec_micros() as i32))
            .unwrap_or((0, 0));
        now
    }

    /// An environment variable.
    pub fn env(&mut self, name: &[u8]) -> Option<Vec<u8>> {
        let v = platform::env_var(name);
        // The clock's own variables are not observations: a restore refreshes what comes from them, and the key has the rest.
        if self.recording && name != b"SOURCE_DATE_EPOCH" && name != b"FORCE_SOURCE_DATE" {
            self.log.push(Observation::Env { name: name.to_vec(), value: v.clone(), cnf: false });
        }
        v
    }

    /// `\write18`: runs a command through the shell.  A hard taint.  The child gets what C has put in
    /// the environment: `SELFAUTO*`, `progname`, `engine`, `TEXMF_OUTPUT_DIRECTORY`.
    pub fn shell(&mut self, command: &[u8]) -> i32 {
        self.mark_taint_because(Taint::Hard, "\\write18");
        platform::shell(command, &rstex_kpse::child_env())
    }

    /// `popen(command, "r")`, read to its end.  A hard taint.
    pub fn popen_read(&mut self, command: &[u8]) -> Option<Vec<u8>> {
        self.mark_taint_because(Taint::Hard, "pipe input");
        platform::popen_read(command, &rstex_kpse::child_env())
    }

    /// `popen(command, "w")`.  A hard taint.
    fn popen_write(&mut self, command: &[u8]) -> Option<std::process::Child> {
        self.mark_taint_because(Taint::Hard, "pipe output");
        platform::popen_write(command, &rstex_kpse::child_env())
    }

    /// A line from the terminal, without its terminator; `None` at end of file.  A hard taint.  Like
    /// `input_line`, it stops at LF, CR or CRLF, and what follows stays for the next line.
    pub fn terminal_read_line(&mut self) -> Option<Vec<u8>> {
        self.mark_taint_because(Taint::Hard, "a read from the terminal");
        let mut line = Vec::new();
        let stdin = std::io::stdin();
        let mut lock = stdin.lock();
        let mut b = [0u8; 1];
        loop {
            match lock.read(&mut b) {
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
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
                    if b[0] == b'\r' {
                        if lock.fill_buf().is_ok_and(|rest| rest.first() == Some(&b'\n')) {
                            lock.consume(1);
                        }
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

    /// Writes to standard error: what C says with `fprintf(stderr, ...)`, and kpathsea's messages.
    pub fn stderr_write(&mut self, bytes: &[u8]) {
        // There is no journal of it, and a restored run would say nothing.
        self.mark_taint_because(Taint::Hard, "a message on stderr");
        let _ = std::io::stderr().write_all(bytes);
    }

    /// Ends the process.  In this crate's unit tests, which share one, it panics, so that a test can see the run stop.
    pub fn exit(&mut self, code: i32) -> ! {
        #[cfg(test)]
        panic!("exit({code})");
        #[cfg(not(test))]
        {
            crate::prof_finish!();
            std::process::exit(code)
        }
    }

    /// `start_time` of texmfmp.c, set once from `SOURCE_DATE_EPOCH` or the clock.
    fn start_time(&mut self) -> i64 {
        if let Some(t) = self.start_time {
            return t;
        }
        let t = match self.env(b"SOURCE_DATE_EPOCH") {
            Some(v) => match strtoull_i64(&v) {
                Some(t) => {
                    self.source_date_epoch_set = true;
                    t
                }
                None => {
                    let msg = [
                        &rstex_kpse::invocation_name()[..],
                        b": fatal: invalid epoch-seconds-timezone value for environment variable $SOURCE_DATE_EPOCH: ",
                        &v,
                        b".\n",
                    ]
                    .concat();
                    self.stderr_write(&msg);
                    self.exit(1)
                }
            },
            None => self.clock().0,
        };
        self.start_time = Some(t);
        t
    }

    /// texmfmp.c `init_start_time`, for ext.rs's `init_start_time` (utils.c `initstarttime`): the start time,
    /// and whether `SOURCE_DATE_EPOCH` gave it, which is what makes `makepdftime` use UTC.
    pub fn init_start_time(&mut self) -> (i64, bool) {
        let t = self.start_time();
        (t, self.source_date_epoch_set)
    }

    /// texmfmp.c `get_date_and_time`: `(minutes, day, month, year)`, honouring `FORCE_SOURCE_DATE`.
    pub fn date_and_time(&mut self) -> (i32, i32, i32, i32) {
        let force = self.env(b"FORCE_SOURCE_DATE");
        let tm = if force.as_deref() == Some(b"1") {
            let t = self.start_time();
            platform::broken_time(t, true)
        } else {
            let (t, _) = self.clock();
            // Whether the variable was not set, the usual case, or invalid: the current time.
            if let Some(v) = force.filter(|v| !v.is_empty() && v != b"0") {
                let msg = [
                    &b"warning: invalid value (expected 0 or 1) for environment variable $FORCE_SOURCE_DATE: "[..],
                    &v,
                    b".\n",
                ]
                .concat();
                self.stderr_write(&msg);
            }
            platform::broken_time(t, false)
        };
        (tm.hour * 60 + tm.minute, tm.day, tm.month, tm.year)
    }
}

/// `strtoull(s, &endptr, 10)`, as `init_start_time` tests it: `None` if something follows the number or
/// it is out of range.  An empty string is 0 and a negative number wraps, as in C.
fn strtoull_i64(s: &[u8]) -> Option<i64> {
    let mut p = 0;
    while s.get(p).is_some_and(|c| matches!(c, b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r')) {
        p += 1;
    }
    let neg = s.get(p) == Some(&b'-');
    if neg || s.get(p) == Some(&b'+') {
        p += 1;
    }
    let digits = s.get(p).is_some_and(u8::is_ascii_digit);
    if !digits {
        // No conversion: endptr is the start of the string.
        return if s.is_empty() { Some(0) } else { None };
    }
    let mut n: u64 = 0;
    while let Some(c) = s.get(p).filter(|c| c.is_ascii_digit()) {
        n = n.checked_mul(10)?.checked_add((c - b'0') as u64)?;
        p += 1;
    }
    if p != s.len() {
        return None;
    }
    Some(if neg { n.wrapping_neg() as i64 } else { n as i64 })
}

/// `Isspace` of texmfmp.c.
fn shell_isspace(c: u8) -> bool {
    c == b' ' || c == b'\t'
}

/// `shell_cmd_is_allowed` of texmfmp.c: `(allow, safecmd, cmdname)`.  `allow` is -1 for an invalid quotation
/// of an argument, 0 if the command is not allowed, and 2 if it is allowed under a restricted shell.  `safecmd` has
/// each argument quoted, so that no embedded command runs; `cmdname` is the first word, checked against `cmdlist`.
pub fn shell_cmd_is_allowed(cmd: &[u8], cmdlist: Option<&[Vec<u8>]>) -> (i32, Vec<u8>, Vec<u8>) {
    let quote = platform::SHELL_QUOTE;
    let at = |i: usize| cmd.get(i).copied().unwrap_or(0);
    let mut s = 0;
    while shell_isspace(at(s)) {
        s += 1;
    }
    let start = s;
    while !shell_isspace(at(s)) && at(s) != 0 {
        s += 1;
    }
    let cmdname = cmd[start..s].to_vec();
    if !cmdlist.is_some_and(|l| l.iter().any(|c| c == &cmdname)) {
        return (0, Vec::new(), cmdname);
    }
    let mut d = cmdname.clone();
    // pre: the previous character is a white space.
    let mut pre = true;
    while at(s) != 0 {
        // Quotation given by a user: " should always be used, and ' is an error at once.
        if at(s) == b'\'' {
            return (-1, d, cmdname);
        }
        if at(s) == b'"' {
            // All arguments are quoted, so a closing quote is needed if the previous character is not a
            // white space: --format="other text files" becomes '--format=''other text files', and
            // "--format"="other text files" on Windows.
            if !pre {
                if platform::DOSISH && at(s - 1) == b'=' {
                    let n = d.len();
                    d[n - 1] = quote;
                    d.push(b'=');
                } else {
                    d.push(quote);
                }
            }
            pre = false;
            d.push(quote);
            s += 1;
            while at(s) != b'"' {
                // An illegal ', or the closing quotation mark is missing.
                if at(s) == b'\'' || at(s) == 0 {
                    return (-1, d, cmdname);
                }
                d.push(at(s));
                s += 1;
            }
            // The closing quotation mark is written afterwards.
            s += 1;
            // What follows it must be a white space or the end.
            if !shell_isspace(at(s)) && at(s) != 0 {
                return (-1, d, cmdname);
            }
        } else if pre && !shell_isspace(at(s)) {
            // The beginning of a usual argument.
            pre = false;
            d.push(quote);
            d.push(at(s));
            s += 1;
        } else if !pre && shell_isspace(at(s)) {
            // The end of a usual argument.
            pre = true;
            d.push(quote);
            d.push(at(s));
            s += 1;
        } else {
            d.push(at(s));
            s += 1;
        }
    }
    if !pre {
        d.push(quote);
    }
    (2, d, cmdname)
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

/// Where an output stream's bytes go.
enum Sink {
    File(std::fs::File),
    /// `\openout` to a pipe: the child, and its standard input until the stream closes.
    Pipe(std::process::Child, Option<std::process::ChildStdin>),
}

impl Sink {
    fn write_all(&mut self, s: &[u8]) {
        match self {
            Sink::File(f) => {
                let _ = f.write_all(s);
            }
            Sink::Pipe(_, Some(stdin)) => {
                let _ = stdin.write_all(s);
            }
            Sink::Pipe(_, None) => {}
        }
    }
}

/// An output stream with its own buffer, and optionally its whole history, the journal.
pub struct OutFile {
    pub path: Vec<u8>,
    sink: Sink,
    buf: Vec<u8>,
    /// The bytes that may wait in `buf`: 1 on a pipe, which C leaves unbuffered.
    limit: usize,
    /// Bytes written so far: the logical position.
    pub total: u64,
    /// All bytes, when journalling: re-materialised on snapshot restore.
    pub journal: Option<Vec<u8>>,
    /// The name this file takes when it closes: a format is published by rename.
    publish_as: Option<Vec<u8>>,
}

const OUT_BUF: usize = 1 << 16;

impl OutFile {
    fn new(path: &[u8], sink: Sink, limit: usize, journal: bool) -> OutFile {
        OutFile {
            path: path.to_vec(),
            sink,
            buf: Vec::with_capacity(limit),
            limit,
            total: 0,
            journal: if journal { Some(Vec::new()) } else { None },
            publish_as: None,
        }
    }
    fn create(path: &[u8], journal: bool) -> Option<OutFile> {
        let file = std::fs::File::create(path_of(path)).ok()?;
        Some(OutFile::new(path, Sink::File(file), OUT_BUF, journal))
    }
    /// The stream a checkpoint had open, on a restore: its bytes so far are written by the caller.
    pub(crate) fn reopen(path: &[u8]) -> Option<OutFile> {
        OutFile::create(path, false)
    }
    pub(crate) fn is_pipe(&self) -> bool {
        matches!(self.sink, Sink::Pipe(..))
    }
    /// Opens an existing file to go on writing: the recorder after its rename.
    fn append(path: &[u8]) -> Option<OutFile> {
        let file = std::fs::OpenOptions::new().append(true).create(true).open(path_of(path)).ok()?;
        Some(OutFile::new(path, Sink::File(file), OUT_BUF, false))
    }
    fn pipe(path: &[u8], mut child: std::process::Child, journal: bool) -> OutFile {
        let stdin = child.stdin.take();
        OutFile::new(path, Sink::Pipe(child, stdin), 1, journal)
    }
    #[inline(always)]
    pub fn byte(&mut self, b: u8) {
        self.buf.push(b);
        self.total += 1;
        if self.buf.len() >= self.limit {
            self.flush();
        }
        if let Some(j) = &mut self.journal {
            j.push(b);
        }
    }
    pub fn bytes(&mut self, s: &[u8]) {
        if self.buf.len() + s.len() > self.limit {
            self.flush();
        }
        if s.len() >= self.limit {
            self.sink.write_all(s);
        } else {
            self.buf.extend_from_slice(s);
            if self.buf.len() >= self.limit {
                self.flush();
            }
        }
        self.total += s.len() as u64;
        if let Some(j) = &mut self.journal {
            j.extend_from_slice(s);
        }
    }
    pub fn flush(&mut self) {
        if !self.buf.is_empty() {
            let _p = crate::prof_scope!("log and aux write");
            self.sink.write_all(&self.buf);
            self.buf.clear();
        }
    }
}

impl Drop for OutFile {
    fn drop(&mut self) {
        self.flush();
        // `pclose`: the end of its input lets the child finish, and we wait for that.
        if let Sink::Pipe(child, stdin) = &mut self.sink {
            drop(stdin.take());
            let _ = child.wait();
        }
        if let Some(dest) = self.publish_as.take() {
            let _ = platform::publish(&self.path, &dest);
        }
    }
}

pub use rstex_pdfio::Deferred;

/// PDF output: `rstex_pdfio::ChunkWriter`, a list of chunks with deferred fields and no seek-back, and
/// the name it is published under.  Bytes are appended; a field whose value is not yet known, such as
/// `/Length`, is reserved with [`PdfOut::defer`] and filled with [`PdfOut::fill`], or overwritten by
/// its absolute offset with [`PdfOut::patch`], as §685's `write_stream_length` does.
#[derive(Default)]
pub struct PdfOut {
    pub path: Vec<u8>,
    w: rstex_pdfio::ChunkWriter,
}

impl PdfOut {
    pub fn new(path: &[u8]) -> PdfOut {
        PdfOut { path: path.to_vec(), w: rstex_pdfio::ChunkWriter::new() }
    }
    /// The file as `b_open_out` left it, with what has been written so far back in the list: a restore.
    pub(crate) fn reopen(path: &[u8], bytes: &[u8]) -> Option<PdfOut> {
        OutFile::create(path, false)?;
        let mut p = PdfOut::new(path);
        p.write(bytes);
        Some(p)
    }
    /// The offset the next byte will have: `pdf_offset`.
    #[inline(always)]
    pub fn offset(&self) -> i64 {
        self.w.offset()
    }
    /// Appends bytes.
    #[inline]
    pub fn write(&mut self, s: &[u8]) {
        self.w.write(s);
    }
    /// Appends a chunk that is ready, without copying it: what a worker deflated.
    pub fn push_chunk(&mut self, chunk: Vec<u8>) {
        self.w.push_chunk(chunk);
    }
    /// Keeps a place for what a worker is deflating.
    pub fn reserve_chunk(&mut self) -> usize {
        self.w.reserve_chunk()
    }
    /// Puts what the workers made in its places.
    pub fn fill_reserved(&mut self, filled: impl IntoIterator<Item = (usize, Vec<u8>)>) {
        self.w.fill_reserved(filled);
    }
    /// Reserves `len` bytes, filled with spaces until [`PdfOut::fill`].
    pub fn defer(&mut self, len: usize) -> Deferred {
        self.w.defer(len)
    }
    /// Fills a deferred field, padded on the right with spaces.
    pub fn fill(&mut self, d: Deferred, s: &[u8]) {
        self.w.fill(d, s);
    }
    /// Overwrites bytes already written, by offset, also across chunks.
    pub fn patch(&mut self, offset: u64, s: &[u8]) {
        self.w.patch(offset, s);
    }
    /// Writes every chunk to the file, publishing by temporary and rename.
    pub fn finish(&mut self) -> std::io::Result<()> {
        let _p = crate::prof_scope!("PDF file write");
        let mut tmp = self.path.clone();
        tmp.extend_from_slice(b".rstex-tmp");
        {
            let mut f = std::fs::File::create(path_of(&tmp))?;
            self.w.write_to(&mut f)?;
        }
        platform::publish(&tmp, &self.path)
    }
    /// The whole output, for tests.
    pub fn contents(&self) -> Vec<u8> {
        self.w.contents()
    }
}

/// A format or a checkpoint being loaded.  Its sections go straight to their places in the image, read or mapped, with no copy of the file in between.
pub struct FmtIn {
    file: std::fs::File,
    pub size: u64,
}

impl FmtIn {
    /// `file`, with its size; `None` for what has none.
    pub fn new(file: std::fs::File) -> Option<FmtIn> {
        let size = file.metadata().ok().filter(|m| !m.is_dir())?.len();
        Some(FmtIn { file, size })
    }
    /// `buf.len()` bytes at `offset`; false past the end.
    pub fn read_at(&self, offset: u64, buf: &mut [u8]) -> bool {
        platform::read_exact_at(&self.file, offset, buf)
    }
    /// Maps `dst`, whole pages of the image holding nothing yet, onto the file at `offset`; false if that cannot be done, and then nothing changed.
    pub fn map_at(&self, offset: u64, dst: &mut [u8]) -> bool {
        platform::map_file_private(&self.file, offset, dst.as_mut_ptr(), dst.len())
    }
}

/// True if the file begins with these bytes.
pub fn file_starts_with(path: &std::path::Path, start: &[u8]) -> bool {
    let mut buf = vec![0; start.len()];
    std::fs::File::open(path).and_then(|mut f| f.read_exact(&mut buf)).is_ok() && buf == start
}

/// One slot of the file table.
#[derive(Default)]
pub enum Slot {
    #[default]
    Closed,
    In(InFile),
    Fmt(FmtIn),
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
    /// `TEX_format_default`, §546: as C's `DUMP_VAR`, the name of the format file with its `.fmt` after a blank,
    /// "adjust array for Pascal", so WEB's `TEX_format_default[j]` is index `j`.
    pub tex_format_default: Vec<u8>,
    /// `translate_filename`: printed in the banner.  The table itself is the hardcoded cp227 one.
    pub translate_filename: Option<Vec<u8>>,
    /// `fullnameoffile` of openclose.c.
    pub full_name_of_file: Option<Vec<u8>>,
    /// `c_job_name` of texmfmp.c: `-jobname`.
    pub c_job_name: Option<Vec<u8>>,
    /// True if output streams keep their bytes.
    pub journal_outputs: bool,
    pub checkpoint: crate::checkpoint::HostState,
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
            c_job_name: None,
            journal_outputs: false,
            checkpoint: Default::default(),
            hv: HostVars::default(),
        }
    }

    /// `update_terminal`, §34.
    pub fn term_flush(&mut self) {
        // In this crate's unit tests the terminal is the buffer: `toks::testkit::printed` takes what is in it.
        if cfg!(not(test)) && !self.term_buf.is_empty() {
            let b = core::mem::take(&mut self.term_buf);
            self.world.terminal_write(&b);
            if let Some(j) = &mut self.checkpoint.term_journal {
                j.extend_from_slice(&b);
            }
            self.term_buf = b;
            self.term_buf.clear();
        }
    }

    /// A new stream on `path`, for the slot `f`.  The PDF is a chunk list that publishes by rename at the end; this
    /// creates the file now, empty, as the oracle's `fopen` does, so that the job sees whether it can.  If the job ends
    /// with no pages, that empty file is what stays: until a page is finished the oracle has written nothing either.
    fn out_slot(&mut self, f: FileId, path: &[u8]) -> Option<Slot> {
        if f == PDF_FILE {
            OutFile::create(path, false)?;
            Some(Slot::Pdf(PdfOut::new(path)))
        } else if f == FMT_FILE {
            // A running job may have this file mapped, so it is replaced, never changed in place.
            let tmp = [path, b".rstex-tmp"].concat();
            let mut o = OutFile::create(&tmp, false)?;
            o.publish_as = Some(path.to_vec());
            Some(Slot::Out(o))
        } else {
            Some(Slot::Out(OutFile::create(path, self.journal_outputs)?))
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

/// Where the line at the start of `data` ends: the first `\n` or `\r`, or the end.  Eight bytes at a time, with the
/// usual test for a zero byte in a word; a line is 40 bytes or so, and this is the only look at most of them.
#[inline]
fn host_line_end(data: &[u8]) -> usize {
    const ONES: u64 = 0x0101_0101_0101_0101;
    const HIGHS: u64 = 0x8080_8080_8080_8080;
    let mut chunks = data.chunks_exact(8);
    let mut at = 0;
    for w in &mut chunks {
        let w = u64::from_le_bytes(w.try_into().unwrap());
        let (a, b) = (w ^ (ONES * b'\n' as u64), w ^ (ONES * b'\r' as u64));
        let z = (a.wrapping_sub(ONES) & !a | b.wrapping_sub(ONES) & !b) & HIGHS;
        if z != 0 {
            return at + (z.trailing_zeros() / 8) as usize;
        }
        at += 8;
    }
    at + chunks.remainder().iter().position(|&c| c == b'\n' || c == b'\r').unwrap_or(chunks.remainder().len())
}

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
    /// Bytes written to the log so far.
    pub(crate) fn host_log_total(&self) -> i64 {
        match &self.host.files[LOG_FILE as usize] {
            Slot::Out(o) => o.total as i64,
            _ => 0,
        }
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

    /// `open_input` of openclose.c.  `name_of_file` becomes the full name opened, `full_name_of_file` what was
    /// found, and for a TFM file `tfm_temp` its first byte, "to pretend we're Pascal".
    fn open_in(&mut self, f: FileId, format: i32) -> bool {
        let name = self.name_of_file_bytes();
        self.host.full_name_of_file = None;
        // Look in -output-directory first, if the name is not absolute, since .aux and such get written
        // there.  Only the name as it stands.  A directory reads as nothing.
        let mut opened: Option<Slot> = None;
        if let Some(dir) = self.host.world.output_directory.clone() {
            if !platform::is_absolute(&name) {
                let fname = [&dir[..], b"/", &name].concat();
                if let Some(s) = self.host.world.open_slot(&fname, format) {
                    self.set_name_of_file(&fname);
                    self.host.full_name_of_file = Some(fname);
                    opened = Some(s);
                }
            }
        }
        // No file means do the normal search.
        if opened.is_none() {
            // Only \openin need not exist, and .vf files: pounding the disk for those is overkill.
            let must_exist = (format != KPSE_TEX_FORMAT || self.tex_input_type != 0) && format != KPSE_VF_FORMAT;
            let Some(path) = self.host.world.lookup(&name, format, must_exist) else {
                return false;
            };
            let Some(s) = self.host.world.open_slot(&path, format) else {
                return false;
            };
            self.host.full_name_of_file = Some(path.clone());
            // Found in the current directory: no ./ in front, since `(./foo.tex ...)` looks dumb, unless the user
            // said ./foo.
            let c0 = path.first().copied().unwrap_or(0);
            let c1 = path.get(1).copied().unwrap_or(0) as i32;
            if c0 == b'.' && is_dir_sep(c1) && (self.name_of_file.at(1) != b'.' as i32 || !is_dir_sep(self.name_of_file.at(2))) {
                self.set_name_of_file(&path[2..]);
            } else {
                self.set_name_of_file(&path);
            }
            opened = Some(s);
        }
        let Some(slot) = opened else {
            return false;
        };
        let shown = self.name_of_file_bytes();
        self.host.world.recorder_record_input(&shown);
        if self.host.world.recording {
            self.host.world.in_obs[f as usize] = self.host.world.log.len() - 1;
        }
        self.host.files[f as usize] = slot;
        // An empty file gives -1 here, and read_font_info then finds the file bad.
        if format == KPSE_TFM_FORMAT {
            self.tfm_temp = self.getc(f);
        }
        true
    }

    /// `open_output` of openclose.c: in `-output-directory`, and if the name will not open, in `TEXMFOUTPUT`.
    fn open_out(&mut self, f: FileId) -> bool {
        let name = self.name_of_file_bytes();
        let absolute = platform::is_absolute(&name);
        let mut path = name.clone();
        if let Some(dir) = &self.host.world.output_directory {
            if !absolute {
                path = [&dir[..], b"/", &name].concat();
            }
        }
        let mut slot = self.host.out_slot(f, &path);
        if slot.is_none() && !absolute {
            if let Some(dir) = self.host.world.var_value(b"TEXMFOUTPUT").filter(|d| !d.is_empty()) {
                path = [&dir[..], b"/", &name].concat();
                slot = self.host.out_slot(f, &path);
            }
        }
        let Some(slot) = slot else {
            return false;
        };
        self.host.files[f as usize] = slot;
        if path != name {
            self.set_name_of_file(&path);
        }
        if self.host.world.recording && f != LOG_FILE {
            self.host.world.checkpoint_note_open_out(&path);
        }
        self.host.world.recorder_record_output(&path);
        true
    }

    /// The command `runpopen` runs for `cmd`, or `None` with a message, if it may not run.
    fn runpopen(&mut self, cmd: &[u8]) -> Option<Vec<u8>> {
        let mut cmd = cmd.to_vec();
        if platform::DOSISH {
            for c in cmd.iter_mut() {
                if *c == b'\'' {
                    *c = b'"';
                }
            }
        }
        if self.restrictedshell == 0 {
            return Some(cmd);
        }
        match self.host.world.shell_cmd_is_allowed(&cmd) {
            (2, safecmd, _) => Some(safecmd),
            (-1, _, _) => {
                let msg = [&b"\nrunpopen quotation error in command line: "[..], &cmd, b"\n"].concat();
                self.host.world.stderr_write(&msg);
                None
            }
            (_, _, cmdname) => {
                let msg = [&b"\nrunpopen command not allowed: "[..], &cmdname, b"\n"].concat();
                self.host.world.stderr_write(&msg);
                None
            }
        }
    }

    /// `a_open_in(f, format)`, texmfmp.c `open_in_or_pipe`: `\input|"cmd"` and `\openin` of one, if shell
    /// escapes are enabled at all.  Its output is read to the end here.
    pub fn a_open_in(&mut self, f: FileId, format: i32) -> bool {
        if self.shellenabledp != 0 && self.name_of_file.at(1) == b'|' as i32 {
            let fname = self.name_of_file_bytes();
            self.host.full_name_of_file = Some(fname.clone());
            self.host.world.recorder_record_input(&fname[1..]);
            let Some(cmd) = self.runpopen(&fname[1..]) else {
                return false;
            };
            let Some(data) = self.host.world.popen_read(&cmd) else {
                return false;
            };
            self.host.files[f as usize] = Slot::In(InFile { path: fname, data, pos: 0 });
            return true;
        }
        self.open_in(f, format)
    }
    /// `a_open_out(f)`, texmfmp.c `open_out_or_pipe`: `\openout` of `|cmd`.  TeX may have appended `.tex`;
    /// it goes away only if the command is a bare word.
    pub fn a_open_out(&mut self, f: FileId) -> bool {
        if self.shellenabledp != 0 && self.name_of_file.at(1) == b'|' as i32 {
            let fname = self.name_of_file_bytes();
            let mut cmd = &fname[1..];
            if !cmd.contains(&b' ') && !cmd.contains(&b'>') && fname.ends_with(b".tex") {
                cmd = &cmd[..cmd.len() - 4];
            }
            let child = match self.runpopen(cmd) {
                Some(c) => self.host.world.popen_write(&c),
                None => None,
            };
            self.host.world.recorder_record_output(&fname[1..]);
            let Some(child) = child else {
                return false;
            };
            let journal = self.host.journal_outputs;
            self.host.files[f as usize] = Slot::Out(OutFile::pipe(&fname, child, journal));
            return true;
        }
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
        if let Slot::Out(o) = &mut self.host.files[f as usize] {
            if let Some(j) = o.journal.take() {
                let path = o.path.clone();
                self.host.world.checkpoint_note_close_out(&path, j);
            }
        }
        self.host.files[f as usize] = Slot::Closed;
    }
    /// `b_close(f)`.  The PDF chunk list is finished by its owner before closing; if it was not, the file stays as
    /// `b_open_out` made it.  Only `remove_pdffile` removes it, through [`Tex::pdf_close_and_remove`].
    pub fn b_close(&mut self, f: FileId) {
        self.host.files[f as usize] = Slot::Closed;
    }
    /// What utils.c `removepdffile` does when its test passes: `xfclose(pdffile)`, then `remove`.  Nothing written by
    /// then has reached the file, since the chunk list goes with the slot.
    pub fn pdf_close_and_remove(&mut self) {
        if let Slot::Pdf(p) = core::mem::take(&mut self.host.files[PDF_FILE as usize]) {
            self.host.world.remove_file(&p.path);
        }
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
        let ok = rstex_kpse::out_name_ok(&n);
        if !ok {
            self.host.world.mark_taint_because(Taint::Hard, "a message on stderr"); // kpathsea's own, about the name
        }
        ok
    }
    /// `texmf_yesno(var)`: a boolean from `texmf.cnf`.
    pub fn texmf_yesno(&mut self, var: &[u8]) -> bool {
        self.host.world.yesno(var)
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
    /// web2c's `eof(f)`: nothing more to read.  Not C's `feof`, which is true only after a read has failed: §602,
    /// the one place WEB says that, has `tfm_temp` for it.
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
        // Its loop does not read when `last<bufsize` fails at once, and then `i` is still EOF: the longest line of a file can leave `first` at buf_size, and the terminal has ended for an error's prompt.
        if first >= size {
            return false;
        }
        if f == TERM_IN {
            self.host.term_flush();
            let Some(line) = self.host.world.terminal_read_line() else {
                return false;
            };
            let n = line.len();
            if n > 0 && first + n >= size {
                self.line_too_long();
            }
            self.buffer.as_mut_slice()[first..first + n].copy_from_slice(&line[..n]);
            last += n;
        } else {
            let Some(i) = self.host.in_file(f) else {
                return false;
            };
            let data = &i.data[i.pos..];
            if data.is_empty() {
                return false;
            }
            let n = host_line_end(data);
            // `last<bufsize` stops the loop of input_line one short of the terminator, so a line
            // that just reaches buf_size does not fit either.
            if n > 0 && first + n >= size {
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
        self.host.world.stderr_write(msg.as_bytes());
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
    /// `runsystem(cmd)`, §1617, of texmfmp.c: -1 for a quotation error, 0 if the command may not run, 1 if any
    /// command may, 2 if this one may under a restricted shell.  Not its exit status, which is only
    /// reported on stderr.  `cmd` ends at its first null byte, like the C string §1617 builds.
    pub fn runsystem(&mut self, cmd: &[u8]) -> i32 {
        self.host.term_flush();
        if self.shellenabledp <= 0 {
            return 0;
        }
        let cmd = cmd.split(|&c| c == 0).next().unwrap_or(b"");
        let mut status = 0;
        let allow;
        if self.restrictedshell == 0 {
            allow = 1;
            status = self.host.world.shell(cmd);
        } else {
            let (a, safecmd, _) = self.host.world.shell_cmd_is_allowed(cmd);
            allow = a;
            if allow == 2 {
                // A command including a '|' is not allowed in restricted mode, for security.
                if safecmd.contains(&b'|') {
                    return 0;
                }
                status = self.host.world.shell(&safecmd);
            }
        }
        if status != 0 {
            let msg = format!("system returned with code {}\n", platform::system_status(status));
            self.host.world.stderr_write(msg.as_bytes());
        }
        allow
    }
    /// `uexit(code)`, §81.
    pub fn uexit(&mut self, code: i32) -> ! {
        self.host.checkpoint.join();
        self.host.term_flush();
        self.host.world.recorder_finish();
        self.pdfout_write_unfinished();
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
        // One past the last character that is not a space or a line terminator.  Tabs stay.
        while last > first && matches!(self.buffer[last as i32 - 1], b' ' | b'\r' | b'\n') {
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

/// For the unit tests of other modules, which are not allowed `std::fs`: files in a directory of their own.
#[cfg(test)]
pub(crate) mod testfs {
    use super::platform;

    /// A new directory, as the bytes of an area: with its last separator.
    pub fn scratch(name: &str) -> Vec<u8> {
        let d = std::env::temp_dir().join(format!("rstex-test-{}-{}", std::process::id(), name));
        std::fs::create_dir_all(&d).unwrap();
        let mut b = platform::path_to_bytes(&d);
        b.push(b'/');
        b
    }

    /// A file of that directory.
    pub fn write(dir: &[u8], name: &str, data: &[u8]) {
        std::fs::write(platform::path_from_bytes(&[dir, name.as_bytes()].concat()), data).unwrap();
    }

    /// The whole directory goes.
    pub fn remove(dir: &[u8]) {
        std::fs::remove_dir_all(platform::path_from_bytes(dir)).unwrap();
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
        assert!(w.log.is_empty(), "nothing is kept unless a checkpoint may be taken");
        w.recording = true;
        let _ = w.env(b"RSTEX_SURELY_UNSET_VAR");
        assert!(matches!(w.log.last(), Some(Observation::Env { value: None, .. })));
        assert_eq!(w.lookup(b"/rstex/surely/not/here.tex", KPSE_TEX_FORMAT, true), None);
        assert!(matches!(w.log.last(), Some(Observation::Lookup { found: None, .. })));
        let st = w.shell(b"exit 3");
        assert_eq!(st, 3);
        assert_eq!(w.taint, Taint::Hard);
        w.mark_taint(Taint::None);
        assert_eq!(w.taint, Taint::Hard, "taint only rises");
    }

    /// A child sees what C's xputenv would have put in the environment.
    #[test]
    fn shell_passes_the_kpathsea_environment() {
        if platform::DOSISH {
            return; // these commands are for sh, and its quote
        }
        rstex_kpse::set_var(b"RSTEX_HOST_TEST_VAR", b"from-kpse");
        let mut w = World::new();
        assert_eq!(w.shell(b"test \"$RSTEX_HOST_TEST_VAR\" = from-kpse"), 0);
        assert_eq!(w.popen_read(b"echo \"$RSTEX_HOST_TEST_VAR\"").unwrap(), b"from-kpse\n");
        assert_eq!(w.shell(b"test -n \"$SELFAUTOLOC\" && test -n \"$progname\" && test -n \"$engine\""), 0);
    }

    fn cmdlist(l: &[&str]) -> Vec<Vec<u8>> {
        l.iter().map(|c| c.as_bytes().to_vec()).collect()
    }

    /// The cases of the comments in texmfmp.c, with QUOTE as on this platform: the tests run on Unix.
    #[test]
    fn restricted_shell_quotes_every_argument() {
        if platform::DOSISH {
            return; // these commands are for sh, and its quote
        }
        let l = cmdlist(&["kpsewhich", "repstopdf"]);
        let check = |cmd: &str, allow: i32, safe: &str, name: &str| {
            let (a, s, n) = shell_cmd_is_allowed(cmd.as_bytes(), Some(&l));
            assert_eq!((a, n.as_slice()), (allow, name.as_bytes()), "{}", cmd);
            if allow == 2 {
                assert_eq!(String::from_utf8_lossy(&s), safe, "{}", cmd);
            }
        };
        check("kpsewhich", 2, "kpsewhich", "kpsewhich");
        check("  kpsewhich  a", 2, "kpsewhich  'a'", "kpsewhich");
        check("kpsewhich a b", 2, "kpsewhich 'a' 'b'", "kpsewhich");
        check("kpsewhich\ta", 2, "kpsewhich\t'a'", "kpsewhich");
        check("kpsewhich a ", 2, "kpsewhich 'a' ", "kpsewhich");
        check(
            "kpsewhich --progname=dvipdfm --format=\"other text files\" config",
            2,
            "kpsewhich '--progname=dvipdfm' '--format=''other text files' 'config'",
            "kpsewhich",
        );
        check("kpsewhich \"a b\"", 2, "kpsewhich 'a b'", "kpsewhich");
        check("kpsewhich a;rm", 2, "kpsewhich 'a;rm'", "kpsewhich");
        check("kpsewhich 'a'", -1, "", "kpsewhich");
        check("kpsewhich \"a", -1, "", "kpsewhich");
        check("kpsewhich \"a'b\"", -1, "", "kpsewhich");
        check("kpsewhich \"a\"b", -1, "", "kpsewhich");
        check("ls -l", 0, "", "ls");
        check("kpsewhic", 0, "", "kpsewhic");
        check("", 0, "", "");
        assert_eq!(shell_cmd_is_allowed(b"ls", None).0, 0);
    }

    #[test]
    fn set_shell_commands_is_mk_shellcmdlist() {
        let mut w = World::new();
        w.set_shell_commands(b"bibtex,kpsewhich,");
        assert_eq!(w.shell_cmdlist.as_ref().map(Vec::len), Some(2));
        assert_eq!(w.shell_cmdlist.as_deref().unwrap(), &cmdlist(&["bibtex", "kpsewhich"])[..]);
        assert_eq!(w.shell_cmd_is_allowed(b"kpsewhich x").0, 2);
        assert_eq!(w.shell_cmd_is_allowed(b"rm x").0, 0);
    }

    #[test]
    fn strtoull_as_init_start_time_uses_it() {
        assert_eq!(strtoull_i64(b"1700000000"), Some(1700000000));
        assert_eq!(strtoull_i64(b""), Some(0));
        assert_eq!(strtoull_i64(b"  12"), Some(12));
        assert_eq!(strtoull_i64(b"+12"), Some(12));
        assert_eq!(strtoull_i64(b"-1"), Some(-1));
        assert_eq!(strtoull_i64(b"12x"), None);
        assert_eq!(strtoull_i64(b" "), None);
        assert_eq!(strtoull_i64(b"x"), None);
        assert_eq!(strtoull_i64(b"99999999999999999999"), None);
    }

    /// `runsystem` answers what §1617 prints, not the exit status.
    #[test]
    fn runsystem_returns_allow() {
        if platform::DOSISH {
            return; // these commands are for sh, and its quote
        }
        let t = Tex::alloc();
        assert_eq!(t.runsystem(b"exit 3\0"), 0, "shell escapes are off");
        t.shellenabledp = 1;
        assert_eq!(t.runsystem(b"exit 3\0"), 1, "any command");
        assert_eq!(t.runsystem(b"true"), 1);
        t.restrictedshell = 1;
        assert_eq!(t.runsystem(b"true"), 0, "not in texmf.cnf's list of commands");
        assert!(t.host.world.shell_cmdlist.is_some(), "it was read at the first command");
        t.host.world.set_shell_commands(b"true,false");
        assert_eq!(t.runsystem(b"true a b\0"), 2);
        assert_eq!(t.runsystem(b"false"), 2, "its status is only reported");
        assert_eq!(t.runsystem(b"true 'a'"), -1);
        assert_eq!(t.runsystem(b"true a|b"), 0, "a pipe character is not allowed");
        assert_eq!(t.runsystem(b"rm x"), 0);
        Tex::dealloc(t);
    }

    #[test]
    fn pipes_read_and_write() {
        if platform::DOSISH {
            return; // these commands are for sh, and its quote
        }
        let t = Tex::alloc();
        t.first = 1;
        t.buf_size = 1000;
        for i in 0..256 {
            t.xord[i] = i as u8;
            t.xchr[i] = i as u8;
        }
        let dir = scratch();
        // Off: the name is a name, and no such file exists.
        t.set_name_of_file(b"|echo one");
        assert!(!t.a_open_in(input_file(1), KPSE_TEX_FORMAT));
        t.shellenabledp = 1;
        t.set_name_of_file(b"|echo one");
        assert!(t.a_open_in(input_file(1), KPSE_TEX_FORMAT));
        assert_eq!(t.name_of_file_bytes(), b"|echo one", "the name stays");
        assert_eq!(t.host.full_name_of_file.as_deref(), Some(&b"|echo one"[..]));
        assert!(t.input_ln(input_file(1), true));
        assert_eq!(t.buffer.range(t.first, t.last), b"one");
        assert!(!t.input_ln(input_file(1), true));
        t.a_close(input_file(1));
        // A write pipe.  .tex comes off a bare word only, and the second command is not one.
        let out = dir.join("pipe-out");
        let name = format!("|cat > '{}'", out.display());
        t.set_name_of_file(name.as_bytes());
        assert!(t.a_open_out(write_file(0)));
        t.write_file_char(0, b'x' as i32);
        t.write_file_ln(0);
        t.a_close(write_file(0));
        assert_eq!(std::fs::read(&out).unwrap(), b"x\n");
        // Restricted, with no list: nothing runs.
        t.restrictedshell = 1;
        t.set_name_of_file(b"|echo one");
        assert!(!t.a_open_in(input_file(1), KPSE_TEX_FORMAT));
        std::fs::remove_dir_all(&dir).unwrap();
        Tex::dealloc(t);
    }

    /// The first byte is read at the open, and C's feof is what `tfm_temp` says: read_font_info relies on both.
    #[test]
    fn a_tfm_open_reads_the_first_byte() {
        let t = Tex::alloc();
        let dir = scratch();
        let f = dir.join("x.tfm");
        std::fs::write(&f, b"\x12\x34").unwrap();
        t.set_name_of_file(&bytes(&f));
        assert!(t.b_open_in(TFM_FILE));
        assert_eq!(t.tfm_temp, 0x12);
        assert_eq!(t.getc(TFM_FILE), 0x34);
        assert_eq!(t.getc(TFM_FILE), -1);
        t.b_close(TFM_FILE);
        let e = dir.join("empty.tfm");
        std::fs::write(&e, b"").unwrap();
        t.set_name_of_file(&bytes(&e));
        assert!(t.b_open_in(TFM_FILE));
        assert_eq!(t.tfm_temp, -1);
        t.b_close(TFM_FILE);
        // Other binary opens read nothing.
        t.tfm_temp = 7;
        let v = dir.join("x.vf");
        std::fs::write(&v, b"\x12\x34").unwrap();
        t.set_name_of_file(&bytes(&v));
        assert!(t.vf_b_open_in(VF_FILE));
        assert_eq!(t.tfm_temp, 7);
        assert_eq!(t.getc(VF_FILE), 0x12);
        t.b_close(VF_FILE);
        std::fs::remove_dir_all(&dir).unwrap();
        Tex::dealloc(t);
    }

    #[test]
    fn output_directory_comes_first_for_input_and_output() {
        let t = Tex::alloc();
        let dir = scratch();
        let d = bytes(&dir);
        t.host.world.output_directory = Some(d.clone());
        t.set_name_of_file(b"made.tex");
        assert!(t.a_open_out(write_file(1)));
        let full = [&d[..], b"/made.tex"].concat();
        assert_eq!(t.name_of_file_bytes(), full, "the name becomes what was opened");
        t.write_file_char(1, b'z' as i32);
        t.write_file_ln(1);
        t.a_close(write_file(1));
        t.set_name_of_file(b"made.tex");
        t.tex_input_type = 1;
        assert!(t.a_open_in(input_file(2), KPSE_TEX_FORMAT));
        assert_eq!(t.name_of_file_bytes(), full);
        assert_eq!(t.host.full_name_of_file.as_deref(), Some(&full[..]));
        t.a_close(input_file(2));
        // A directory there is not a file, and the search then finds nothing.
        std::fs::create_dir_all(dir.join("adir")).unwrap();
        t.set_name_of_file(b"adir");
        assert!(!t.a_open_in(input_file(2), KPSE_TEX_FORMAT));
        assert_eq!(t.host.full_name_of_file, None);
        std::fs::remove_dir_all(&dir).unwrap();
        Tex::dealloc(t);
    }

    #[test]
    fn the_recorder_follows_openclose() {
        let t = Tex::alloc();
        let dir = scratch();
        let d = bytes(&dir);
        t.host.world.output_directory = Some(d.clone());
        t.host.world.recorder_enabled = true;
        // What kpathsea read earlier does not matter here: only the lines of this test are looked at.
        t.set_name_of_file(b"rec.log");
        assert!(t.a_open_out(LOG_FILE));
        t.a_close(LOG_FILE);
        t.host.world.recorder_change_filename(b"rec.fls");
        t.host.world.recorder_record_input(b"some/input.tex");
        t.host.world.recorder_finish();
        let fls = String::from_utf8(std::fs::read(dir.join("rec.fls")).unwrap()).unwrap();
        let mut lines = fls.lines();
        assert!(lines.next().unwrap().starts_with("PWD "));
        let rest: Vec<&str> = lines.filter(|l| l.ends_with("rec.log") || l.ends_with("input.tex")).collect();
        assert_eq!(rest, vec![format!("OUTPUT {}/rec.log", dir.display()).as_str(), "INPUT some/input.tex"]);
        std::fs::remove_dir_all(&dir).unwrap();
        Tex::dealloc(t);
    }

    #[test]
    fn a_full_line_does_not_fit_and_a_shorter_one_does() {
        // The line that does not fit ends the process, so only the one that does is tried here.
        let t = Tex::alloc();
        for i in 0..256 {
            t.xord[i] = i as u8;
        }
        t.first = 1;
        t.buf_size = 6;
        let dir = scratch();
        let f = dir.join("line.tex");
        std::fs::write(&f, b"abcd\n").unwrap();
        t.set_name_of_file(&bytes(&f));
        assert!(t.a_open_in(input_file(1), KPSE_TEX_FORMAT));
        assert!(t.input_ln(input_file(1), true));
        assert_eq!(t.last, 5);
        t.a_close(input_file(1));
        std::fs::remove_dir_all(&dir).unwrap();
        Tex::dealloc(t);
    }

    #[test]
    fn first_line_loses_only_trailing_blanks_and_terminators() {
        let t = Tex::alloc();
        for i in 0..256 {
            t.xord[i] = i as u8;
        }
        t.first = 1;
        t.buf_size = 100;
        t.host.command_line = b"a\tb\t \r\n  ".to_vec();
        t.t_open_in();
        assert_eq!(t.buffer.range(t.first, t.last), b"a\tb\t");
        // Only once.
        t.t_open_in();
        assert_eq!(t.last, t.first);
        Tex::dealloc(t);
    }

    /// A close of what was never finished leaves the empty file `b_open_out` made, as on the oracle, and the bytes in
    /// the chunk list do not reach it.  Only `remove_pdffile` makes the file go.
    #[test]
    fn an_unfinished_pdf_stays_empty_until_removed() {
        let t = Tex::alloc();
        let dir = scratch();
        let f = dir.join("o.pdf");
        t.set_name_of_file(&bytes(&f));
        assert!(t.b_open_out(PDF_FILE));
        assert!(f.exists());
        t.pdf_out_file().unwrap().write(b"never published");
        t.b_close(PDF_FILE);
        assert_eq!(std::fs::read(&f).unwrap(), b"");
        t.set_name_of_file(&bytes(&f));
        assert!(t.b_open_out(PDF_FILE));
        t.pdf_out_file().unwrap().write(b"never published");
        t.pdf_close_and_remove();
        assert!(!f.exists());
        // Finished, it stays through the close.
        t.set_name_of_file(&bytes(&f));
        assert!(t.b_open_out(PDF_FILE));
        t.pdf_out_file().unwrap().write(b"published");
        t.pdf_out_file().unwrap().finish().unwrap();
        t.b_close(PDF_FILE);
        assert_eq!(std::fs::read(&f).unwrap(), b"published");
        std::fs::remove_dir_all(&dir).unwrap();
        Tex::dealloc(t);
    }

    #[test]
    fn pdf_patches_by_offset_and_publishes() {
        let dir = scratch();
        let f = dir.join("o.pdf");
        let mut p = PdfOut::new(&bytes(&f));
        p.write(b"0123456789");
        let at = p.offset();
        p.write(b"          ");
        p.patch(at as u64, b"42");
        p.finish().unwrap();
        assert_eq!(std::fs::read(&f).unwrap(), b"012345678942        ");
        std::fs::remove_dir_all(&dir).unwrap();
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
