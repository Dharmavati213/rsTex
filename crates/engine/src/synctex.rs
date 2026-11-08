//! §1895-1929 Part 54: SyncTeX.  WEB only sends messages, and their call sites stay in the modules that make them (`dvi.rs`, `pdfship.rs`, `files.rs`, `main.rs`).  This is what gets
//! them, `refs/synctex/synctex.c` under `synctex-pdftex.h`, whose functions keep their names here, with the same tests in the same order.  `synctexchar` and `synctexnode` are left out: WEB sends neither.
//!
//! **What is pdfTeX's.**  The text of the file, byte for byte.  The names, when the file comes into being, what a run removes, even a run that writes none, which is how a stale
//! file goes away, and the messages, which are `printf`s: they go to the terminal only, past `term_offset`, so the log has no word of SyncTeX.
//!
//! **What is not.**  pdfTeX's file is one gzip stream, written by `gzprintf`, and that is most of what SyncTeX costs it.  This one is gzip members one after the other: one for what comes
//! before the first sheet, one for each sheet, one for what comes after, which `gzread` and so every reader goes through as one stream.
//! - A record is written straight to [`HVars::text`], with no format string, in one bounds check for each number; see [`Rec`].
//! - A member goes to `DeflatePool` if `pdfout.rs` says the workers are up, or is made here if it is small, as hello's are.  Its bytes depend on nothing but its text.
//! - What a member holds is settled only at the end, or when `pdfout_settle` is called: [`Tex::synctex_settle`], which says at what length the file is whole.  The file is written then, in one go.
//!
//! **A checkpoint.**  The context is POD, in `v_image.sync`, and so is in the image.  The text, the name of the file and the root's name are not, and go in the checkpoint's meta part: `synctex_for_checkpoint`
//! and `synctex_restore`, which makes the file again as `\openout`'s are made.  No sheet can lie behind a checkpoint, so all of it is still text.
//!
//! **A page record** holds the same, and the length at which `pdfout_settle` left the file whole: past that length there is only what its text will become.  The members themselves are a stream of the chain, so what a resume gets is
//! what that record's run wrote, and not what the last run left under the file's final name, which may be another run's: `checkpoint/pages.rs`.
//!
//! **Off.**  A run with no `-synctex` pays one load and one test for each message from `ship_out`, and for each file one copy of a name until the first sheet says it is off, as pdfTeX does.
#![forbid(unsafe_code)]
#![deny(unreachable_patterns, unused_variables)]
#![allow(non_snake_case, unused_mut, unused_imports, dead_code)]

use crate::access::*;
use crate::consts::*;
use crate::host::*;
use crate::platform;
use crate::raw::*;
use crate::state::*;
use rstex_pdfio::flate::LEVEL_SYNCTEX;

/// `SYNCTEX_NO_OPTION`: no `-synctex` on the command line.
pub const SYNCTEX_NO_OPTION: i32 = i32::MAX;
/// `SYNCTEX_VERSION`.
const SYNCTEX_VERSION: i32 = 1;
/// What room a record may need: seven numbers of eleven characters, their separators, and the eight bytes that [`Rec::int`] stores at a time.
const MAX_RECORD: usize = 128;
/// What [`HVars::text`] starts with.  Hello's text is a kilobyte.
const TEXT_START: usize = 1 << 13;

crate::pod_struct! {
    /// `synctex_ctxt`, and the static `synctex_tag_counter`.  The `file` of the C is `file_open` here, with what is not POD in [`HVars`].
    pub struct Vars {
        pub tag_counter: u32,
        /// The number of interesting records.
        pub count: i32,
        /// The last synchronized node, must be set before the recorder.
        pub node: i32,
        /// `recorder`: only ever null or `synctex_record_node_kern`.
        pub kern_pending: bool,
        pub option_read: bool,
        pub content_ready: bool,
        /// Definitely turn off synctex.
        pub off: bool,
        pub no_gz: bool,
        /// Whether it really contains synchronization material.
        pub not_void: bool,
        /// One shot warning flag.
        pub warned: bool,
        /// Whether the input file name was quoted by tex or not.
        pub quoted: bool,
        pub file_open: bool,
        pub tag: i32,
        pub line: i32,
        pub curh: i32,
        pub curv: i32,
        pub magnification: i32,
        pub options: i32,
        pub lastv: i32,
        pub form_depth: i32,
        /// Members made so far.
        pub members: i32,
        /// `total_length` is the place in the text now, less this: where the last anchor began.
        pub anchor_at: i64,
        /// The place in the text of `HVars::text[0]`.
        pub text_base: i64,
    }
}

/// Non-POD state private to this module, in `self.host`.
#[derive(Default)]
pub struct HVars {
    /// `SYNCTEX_FILE`, which `v_image.sync.file_open` says of too, in the image.
    file: Option<OutFile>,
    busy_name: Vec<u8>,
    /// `root_name`.  A relative name is made absolute when it is used: the first file a run opens is named in every run, and few ever write it.
    root_name: Option<Vec<u8>>,
    /// The text not yet in a member, in `text[..at]`.  The rest is room for the next records, which is written over.
    text: Vec<u8>,
    at: usize,
    /// In the order of the file, what has no place in it yet: a member made here, or `None` for the next of those the workers have.
    members: Vec<Option<Vec<u8>>>,
    /// How many of those are the workers'.
    with_workers: usize,
    /// Where a member made here is written.  It grows by 32 KiB at a time, and a member is mostly small, so what is kept is a copy, as in `DeflatePool`.
    scratch: Vec<u8>,
    cwd: Option<Vec<u8>>,
    /// A chain of page records is being written: the file keeps what goes to it for the next record, from whenever it comes into being.
    journalling: bool,
}

impl HVars {
    #[cold]
    #[inline(never)]
    fn grow(&mut self, need: usize) {
        let n = (self.text.len() * 2).max(self.at + need).max(TEXT_START);
        self.text.resize(n, 0);
    }

    /// One record: all of it goes in by `f`, with no test of the room beyond this one.
    #[inline(always)]
    fn record(&mut self, f: impl FnOnce(&mut Rec<'_>)) {
        if self.text.len() - self.at < MAX_RECORD {
            self.grow(MAX_RECORD);
        }
        let mut r = Rec { b: &mut self.text, at: self.at };
        f(&mut r);
        self.at = r.at;
    }

    fn put(&mut self, s: &[u8]) {
        if self.text.len() - self.at < s.len() {
            self.grow(s.len());
        }
        self.text[self.at..self.at + s.len()].copy_from_slice(s);
        self.at += s.len();
    }
}

/// Where a record is being written.  `int` is the cost of all this, and a loop by digits would miss a branch at the end of nearly every number.
struct Rec<'a> {
    b: &'a mut [u8],
    at: usize,
}

/// The eight decimal digits of `v`, below 10^8, in a word with the first digit in the lowest byte.  Paul Khuong's way: four digits and four in two halves of the word, then two in each,
/// then one in each byte, by multiplications that cannot carry across a field.
#[inline(always)]
fn eight_digits(v: u32) -> u64 {
    let merged = (v / 10000) as u64 | (((v % 10000) as u64) << 32);
    let top = ((merged * 10486) >> 20) & ((0x7F << 32) | 0x7F);
    let hundreds = ((merged - 100 * top) << 16) + top;
    let tens = ((hundreds * 103) >> 10) & ((0xF << 48) | (0xF << 32) | (0xF << 16) | 0xF);
    tens + ((hundreds - 10 * tens) << 8)
}

const ASCII_ZEROS: u64 = 0x3030_3030_3030_3030;

impl Rec<'_> {
    #[inline(always)]
    fn ch(&mut self, c: u8) {
        self.b[self.at] = c;
        self.at += 1;
    }

    /// `%i`.
    #[inline(always)]
    fn int(&mut self, v: i32) {
        if v < 0 {
            self.ch(b'-');
        }
        let u = v.unsigned_abs();
        if u >= 100_000_000 {
            return self.ten_digits(u);
        }
        let w = eight_digits(u);
        // The zero bytes at its low end are the leading zeros of the number, and the last digit stays whatever it is.
        let skip = (w | (1 << 56)).trailing_zeros() / 8;
        self.b[self.at..self.at + 8].copy_from_slice(&((w + ASCII_ZEROS) >> (8 * skip)).to_le_bytes());
        self.at += 8 - skip as usize;
    }

    #[cold]
    #[inline(never)]
    fn ten_digits(&mut self, u: u32) {
        let top = u / 100_000_000;
        if top >= 10 {
            self.ch(b'0' + (top / 10) as u8);
        }
        self.ch(b'0' + (top % 10) as u8);
        self.b[self.at..self.at + 8].copy_from_slice(&(eight_digits(u % 100_000_000) + ASCII_ZEROS).to_le_bytes());
        self.at += 8;
    }

    /// The vertical place of a record: `=` if it is the last one written, under `-synctex`'s bit 8.
    #[inline(always)]
    fn v(&mut self, same: bool, v: i32) {
        if same {
            self.ch(b'=');
        } else {
            self.int(v);
        }
    }
}

impl Tex {
    #[inline(always)]
    fn synctex_with_forms(&self) -> bool {
        self.v_image.sync.options & 4 != 0
    }

    /// `SYNCTEX_SHOULD_COMPRESS_V`, and the `lastv` that goes with writing the number.
    #[inline(always)]
    fn synctex_same_v(&mut self, curv: i32) -> bool {
        let c = &mut self.v_image.sync;
        if c.options & 8 != 0 && c.lastv == c.curv {
            return true;
        }
        c.lastv = curv;
        false
    }

    /// The bytes of a string, `gettexstring`.
    fn synctex_str(&self, s: i32) -> Vec<u8> {
        self.str_pool.range(self.str_start[s], self.str_start[s + 1]).to_vec()
    }

    /// The first half of `generic_synctex_get_current_name`, for a name that is known: an absolute one, or below the working directory, which a run cannot change.
    fn synctex_full_name(&mut self, name: Vec<u8>) -> Vec<u8> {
        if platform::is_absolute(&name) {
            return name;
        }
        let cwd = self.host.hv.synctex.cwd.get_or_insert_with(platform::current_dir_bytes);
        [&cwd[..], b"/", &name].concat()
    }

    /// `_synctex_read_command_line_option`: a one shot.
    fn synctex_read_option(&mut self) {
        if self.v_image.sync.option_read {
            return;
        }
        self.v_image.sync.lastv = -1;
        if self.synctexoption == SYNCTEX_NO_OPTION {
            // No option given from the command line.
            self.set_synctex(0);
        } else if self.synctexoption == 0 {
            // -synctex=0 was given: SyncTeX must be definitely disabled, any subsequent \synctex=N will have no effect at all.
            self.v_image.sync.off = true;
            self.set_synctex(0);
        } else {
            self.v_image.sync.no_gz = self.synctexoption < 0;
            self.v_image.sync.options = self.synctexoption.wrapping_abs();
            self.synctexoption |= 1;
            self.set_synctex(self.synctexoption);
        }
        self.v_image.sync.option_read = true;
    }

    /// `synctexinitcommand`, §1904.
    pub fn synctex_init_command(&mut self) {
        self.synctex_read_option();
    }

    /// `synctexabort`: free all, close and remove the file if any.
    pub fn synctex_abort(&mut self, _log_opened: bool) {
        if self.v_image.sync.file_open {
            if self.host.hv.synctex.with_workers > 0 {
                let _ = self.pdfout_gzip_finish();
            }
            let s = &mut self.host.hv.synctex;
            s.with_workers = 0;
            s.members.clear();
            s.at = 0;
            s.file = None;
            let busy = core::mem::take(&mut s.busy_name);
            let _ = self.host.world.remove_path(&busy);
            self.v_image.sync.file_open = false;
        }
        self.host.hv.synctex.root_name = None;
        self.v_image.sync.off = true;
    }

    /// `synctex_dot_open` ensures that the foo.synctex file is open.  In case of problem, it definitely disables synchronization.
    fn synctex_dot_open(&mut self) -> bool {
        if self.v_image.sync.off || self.synctex() == 0 {
            return false; // synchronization is disabled: do nothing
        }
        if self.v_image.sync.file_open {
            return true; // synchronization is already enabled
        }
        self.synctex_read_option();
        // This is the first time we are asked to open the file: executed only once.
        let tmp = self.synctex_str(self.job_name);
        if tmp.is_empty() {
            self.wterm_s(b"\nSyncTeX information: no synchronization with keyboard input\n");
        } else {
            let mut busy = Vec::with_capacity(tmp.len() + 16);
            if let Some(dir) = &self.host.world.output_directory {
                if !platform::is_absolute(&tmp) {
                    busy.extend_from_slice(dir);
                    busy.push(b'/');
                }
            }
            // The input file name did contain spaces: quotes were added around it, which go here and come back in the terminal's message.
            self.v_image.sync.quoted = tmp[0] == b'"' && tmp[tmp.len() - 1] == b'"';
            if self.v_image.sync.quoted {
                busy.extend_from_slice(if tmp.len() > 1 { &tmp[1..tmp.len() - 1] } else { b"" });
            } else {
                busy.extend_from_slice(&tmp);
            }
            busy.extend_from_slice(b".synctex(busy)");
            if let Some(file) = OutFile::create(&busy, self.host.hv.synctex.journalling) {
                let s = &mut self.host.hv.synctex;
                s.file = Some(file);
                s.busy_name = busy;
                self.v_image.sync.file_open = true;
                // synctex_record_preamble, and the initialization of the context
                let c = &mut self.v_image.sync;
                let version = if c.options > SYNCTEX_VERSION { c.options } else { SYNCTEX_VERSION };
                if c.magnification == 0 {
                    c.magnification = 1000;
                }
                let s = &mut self.host.hv.synctex;
                s.put(b"SyncTeX Version:");
                s.record(|r| {
                    r.int(version);
                    r.ch(b'\n');
                });
                // print the preamble, this is quite an UTF8 file
                if let Some(root) = s.root_name.take() {
                    let root = if root.is_empty() { b"texput".to_vec() } else { self.synctex_full_name(root) };
                    self.synctex_record_input(1, &root);
                }
                self.v_image.sync.count = 0;
                return true;
            }
        }
        // no .synctex file available, so disable synchronization
        self.synctex_abort(false);
        false
    }

    /// `synctex_prepare_content`: `synctex_record_settings` must be called very late, only once there is an opportunity to know whether in pdf or dvi mode.
    fn synctex_prepare_content(&mut self) -> bool {
        if self.v_image.sync.content_ready {
            return self.v_image.sync.file_open;
        }
        if self.synctex_dot_open() {
            self.synctex_record_settings();
            self.host.hv.synctex.put(b"Content:\n");
            self.v_image.sync.content_ready = true;
            return true;
        }
        self.synctex_abort(false);
        false
    }

    /// `synctexstartinput`, §1909: each time TeX opens a file a new tag is created and stored in the current input.  It does so whether or not `\\synctex` is ever set, unless `-synctex=0` turned it off, and that shows without any
    /// SyncTeX: `get_node` writes the tag into the two SyncTeX words of a node, and those words can be read later as old words, under a matrix, as a destination's rectangle.  tests/integration/pdfship/dest_stale_pointer.tex.
    pub fn synctex_start_input(&mut self) {
        self.synctex_read_option();
        if self.v_image.sync.off {
            return;
        }
        // We have reached the limit, subsequent files will be softly ignored.
        if self.v_image.sync.tag_counter == u32::MAX {
            self.set_synctex_tag(0);
            return;
        }
        self.v_image.sync.tag_counter += 1;
        let tag = self.v_image.sync.tag_counter as i32;
        self.set_synctex_tag(tag);
        if tag == 1 {
            // The first file TeX ever opens, in general \jobname.tex: we do not know yet if synchronization will ever be enabled, so we have to store the file name.
            self.host.hv.synctex.root_name = Some(self.host.full_name_of_file.clone().unwrap_or_default());
            return;
        }
        if self.v_image.sync.file_open || self.synctex_dot_open() {
            // Always record the input, even if SYNCTEX_VALUE is 0.  It may happen when \synctex=0 was given temporarily in the source file.
            let name = self.host.full_name_of_file.clone().unwrap_or_default();
            // `generic_synctex_get_current_name`: no name stays none.
            let name = if name.is_empty() { name } else { self.synctex_full_name(name) };
            self.synctex_record_input(tag, &name);
        }
    }

    /// The terminal's half of a `printf`: `term_offset` does not hear of it.
    fn synctex_warn_disabled(&mut self) {
        if self.synctex() != 0 && !self.v_image.sync.warned {
            self.v_image.sync.warned = true;
            self.wterm_s(b"\nSyncTeX warning: Synchronization was disabled from\nthe command line with -synctex=0\nChanging the value of \\synctex has no effect.");
        }
    }

    /// `synctexterminate`, §1912.  If synchronization was active, the working file is moved to the final name.  If it was not, or there is no output, the file is removed if any.
    /// That way we can be sure that any synctex file is in sync with a tex run.
    pub fn synctex_terminate(&mut self, log_opened: bool) {
        if log_opened {
            let mut real = self.synctex_str(self.log_name);
            // now remove the last path extension which is in general log
            if let Some(k) = real.iter().rposition(|&c| c == b'.') {
                real.truncate(k);
            }
            real.extend_from_slice(b".synctex");
            let no_gz = self.v_image.sync.no_gz;
            let no_gz_aux_name = no_gz || self.v_image.sync.options & 2 != 0;
            if !no_gz {
                // Remove any uncompressed synctex file, from a previous build.
                let _ = self.host.world.remove_path(&real);
                if !no_gz_aux_name {
                    real.extend_from_slice(b".gz");
                }
            }
            // allways remove the synctex output file before renaming it, windows requires it.
            if self.host.world.remove_path_denied(&real) {
                let msg = [&b"SyncTeX: Can't remove "[..], &real, b" (file is open or read only)\n"].concat();
                self.host.world.stderr_write(&msg);
            }
            if self.v_image.sync.file_open {
                if self.v_image.sync.not_void {
                    self.synctex_record_postamble();
                    self.synctex_cut();
                    self.synctex_settle();
                    let s = &mut self.host.hv.synctex;
                    let failed = s.file.as_mut().is_none_or(|f| f.has_failed());
                    s.file = None; // close the synctex file
                    let busy = core::mem::take(&mut s.busy_name);
                    self.v_image.sync.file_open = false;
                    if failed {
                        // As `gzprintf` failing: `synctexabort`.
                        let _ = self.host.world.remove_path(&busy);
                    } else if self.host.world.rename_file(&busy, &real) {
                        // renamed the working synctex file
                        if self.interaction > 0 {
                            let quoted = self.v_image.sync.quoted;
                            let msg = if quoted { [&b"\nSyncTeX written on \""[..], &real, b"\""].concat() } else { [&b"\nSyncTeX written on "[..], &real, b"."].concat() };
                            self.wterm_s(&msg);
                        }
                    } else {
                        let msg = [&b"SyncTeX: Can't rename "[..], &busy, b" to ", &real, b"\n"].concat();
                        self.host.world.stderr_write(&msg);
                        let _ = self.host.world.remove_path(&busy);
                    }
                } else {
                    // close and remove the synctex file because there are no pages of output
                    self.synctex_abort(false);
                }
            }
            if no_gz_aux_name {
                // Remove any compressed synctex file, from a previous build.
                real.extend_from_slice(b".gz");
                let _ = self.host.world.remove_path(&real);
            }
        } else {
            // There was a problem with the output.  We just try to remove existing synctex output files including the busy one.
            let tmp = self.synctex_str(self.job_name);
            let mut real = if tmp.len() > 1 && tmp[0] == b'"' && tmp[tmp.len() - 1] == b'"' {
                tmp[1..tmp.len() - 1].to_vec()
            } else if !tmp.is_empty() && tmp[0] == b'"' && tmp[tmp.len() - 1] == b'"' {
                Vec::new()
            } else {
                tmp
            };
            real.extend_from_slice(b".synctex");
            let _ = self.host.world.remove_path(&real);
            real.extend_from_slice(b".gz");
            let _ = self.host.world.remove_path(&real);
        }
        self.synctex_abort(false);
    }

    /// `synctexsheet`, §1914: the "{..." line, at the very beginning of `ship_out`.
    pub fn synctex_sheet(&mut self, mag: i32) {
        if self.v_image.sync.off {
            self.synctex_warn_disabled();
            return;
        }
        if self.total_pages == 0 {
            // Now it is time to properly set up the scale factor.
            if mag > 0 {
                self.v_image.sync.magnification = mag;
            }
            // What came before the first sheet is a member of its own, the same in every run while the preamble stands.
            self.synctex_cut();
        }
        if self.synctex_prepare_content() {
            // synctex_record_sheet
            self.synctex_record_anchor();
            let sheet = self.total_pages + 1;
            self.host.hv.synctex.record(|r| {
                r.ch(b'{');
                r.int(sheet);
                r.ch(b'\n');
            });
            self.v_image.sync.count += 1;
        }
    }

    /// `synctexteehs`, §1915: the "}..." line, at the very end of `ship_out`.  So the member of a sheet ends here, and with it a place where the file is whole.
    pub fn synctex_teehs(&mut self) {
        if self.v_image.sync.off || !self.v_image.sync.file_open {
            return;
        }
        // synctex_record_teehs
        self.synctex_record_anchor();
        let sheet = self.total_pages;
        self.host.hv.synctex.record(|r| {
            r.ch(b'}');
            r.int(sheet);
            r.ch(b'\n');
        });
        self.v_image.sync.count += 1;
        self.synctex_cut();
    }

    /// `synctexpdfxform`: a form will ship out.
    pub fn synctex_pdfxform(&mut self, _p: i32) {
        if self.v_image.sync.off {
            self.synctex_warn_disabled();
            return;
        }
        if self.synctex_prepare_content() {
            // synctex_record_pdfxform
            if !self.synctex_ignore() {
                self.v_image.sync.form_depth += 1;
                if self.synctex_with_forms() {
                    let form = self.pdf_cur_form;
                    self.host.hv.synctex.record(|r| {
                        r.ch(b'<');
                        r.int(form);
                        r.ch(b'\n');
                    });
                    self.v_image.sync.count += 1;
                }
            }
        }
    }

    /// `synctexmrofxfdp`: a form did ship out.
    pub fn synctex_mrofxfdp(&mut self) {
        if self.v_image.sync.file_open {
            self.synctex_record_anchor();
            self.v_image.sync.form_depth -= 1;
            if self.synctex_with_forms() {
                self.host.hv.synctex.put(b">\n");
                self.v_image.sync.count += 1;
            }
        }
    }

    /// `synctexpdfrefxform`, from `out_form`: a form ref will ship out.
    pub fn synctexpdfrefxform(&mut self, objnum: i32) {
        if self.v_image.sync.file_open {
            self.synctex_record_node_pdfrefxform(objnum);
        }
    }

    /// The first `SYNCTEX_IGNORE`.
    #[inline(always)]
    fn synctex_ignore(&self) -> bool {
        let c = &self.v_image.sync;
        c.off || self.synctex() == 0 || !c.file_open || (c.form_depth > 0 && c.options & 4 == 0)
    }

    /// The second `SYNCTEX_IGNORE`, without its test of `\synctex`, which is the caller's.  `q` is the end of the node.
    #[inline(always)]
    fn synctex_ignore_node(&self, q: i32) -> bool {
        self.v_image.sync.off || self.sync_tag(q) <= 0 || self.sync_line(q) <= 0
    }

    /// `SYNCTEX_CONTEXT_DID_CHANGE`.
    #[inline(always)]
    fn synctex_context_did_change(&self, q: i32) -> bool {
        let c = &self.v_image.sync;
        c.node == 0 || self.sync_tag(q) != c.tag || self.sync_line(q) != c.line
    }

    /// `(*synctex_ctxt.recorder)(synctex_ctxt.node)`, if there is one.
    #[inline(always)]
    fn synctex_run_recorder(&mut self) {
        if self.v_image.sync.kern_pending {
            let node = self.v_image.sync.node;
            self.synctex_record_node_kern(node);
        }
    }

    /// What all but the kern say of the node that is to be written: the context is now that node, with the current point, and nothing is pending.
    #[inline(always)]
    fn synctex_context_is(&mut self, node: i32, q: i32) {
        let (tag, line, curh, curv) = (self.sync_tag(q), self.sync_line(q), self.cur_h, self.cur_v);
        let c = &mut self.v_image.sync;
        c.node = node;
        c.tag = tag;
        c.line = line;
        c.curh = curh;
        c.curv = curv;
        c.kern_pending = false;
    }

    /// `synctexvlist`, §1916.
    #[inline(always)]
    pub fn synctex_vlist(&mut self, this_box: i32) {
        if self.synctex() != 0 {
            self.synctex_box_begins(this_box, b'[');
        }
    }

    /// `synctextsilv`, §1917.
    #[inline(always)]
    pub fn synctex_tsilv(&mut self, this_box: i32) {
        if self.synctex() != 0 {
            self.synctex_box_ends(this_box, b']');
        }
    }

    /// `synctexvoidvlist`, §1920.
    #[inline(always)]
    pub fn synctex_void_vlist(&mut self, p: i32, _this_box: i32) {
        if self.synctex() != 0 {
            self.synctex_void_vlist_on(p);
        }
    }

    /// `synctexhlist`, §1918.
    #[inline(always)]
    pub fn synctex_hlist(&mut self, this_box: i32) {
        if self.synctex() != 0 {
            self.synctex_box_begins(this_box, b'(');
        }
    }

    /// `synctextsilh`, §1919.
    #[inline(always)]
    pub fn synctex_tsilh(&mut self, this_box: i32) {
        if self.synctex() != 0 {
            self.synctex_box_ends(this_box, b')');
        }
    }

    /// `synctexvoidhlist`, §1920.
    #[inline(always)]
    pub fn synctex_void_hlist(&mut self, p: i32, _this_box: i32) {
        if self.synctex() != 0 {
            self.synctex_void_hlist_on(p);
        }
    }

    /// `synctexmath`, §1924.
    #[inline(always)]
    pub fn synctex_math(&mut self, p: i32, _this_box: i32) {
        if self.synctex() != 0 {
            self.synctex_math_on(p);
        }
    }

    /// `synctexhorizontalruleorglue`, §1922.
    #[inline(always)]
    pub fn synctex_horizontal_rule_or_glue(&mut self, p: i32, _this_box: i32) {
        if self.synctex() != 0 {
            self.synctex_rule_or_glue_on(p);
        }
    }

    /// `synctexkern`, §1923.
    #[inline(always)]
    pub fn synctex_kern(&mut self, p: i32, this_box: i32) {
        if self.synctex() != 0 {
            self.synctex_kern_on(p, this_box);
        }
    }

    /// `synctexcurrent`, §1921.
    #[inline(always)]
    pub fn synctex_current(&mut self) {
        if self.synctex() != 0 {
            self.synctex_current_on();
        }
    }

    #[inline(never)]
    fn synctex_box_begins(&mut self, this_box: i32, c: u8) {
        if !self.synctex_ignore() {
            self.synctex_context_is(this_box, this_box + BOX_NODE_SIZE);
            self.v_image.sync.not_void = true;
            self.synctex_record_box(c, this_box);
        }
    }

    #[inline(never)]
    fn synctex_box_ends(&mut self, this_box: i32, c: u8) {
        if !self.synctex_ignore() {
            // Ignoring any pending info to be recorded.
            self.synctex_context_is(this_box, this_box + BOX_NODE_SIZE);
            self.host.hv.synctex.record(|r| {
                r.ch(c);
                r.ch(b'\n');
            });
            self.v_image.sync.count += 1;
        }
    }

    #[inline(never)]
    fn synctex_void_vlist_on(&mut self, p: i32) {
        if !self.synctex_ignore() {
            self.synctex_context_is(p, p + BOX_NODE_SIZE);
            self.synctex_record_box(b'v', p);
        }
    }

    #[inline(never)]
    fn synctex_void_hlist_on(&mut self, p: i32) {
        if !self.synctex_ignore() {
            // the sync context has changed but was not yet recorded
            self.synctex_run_recorder();
            self.synctex_context_is(p, p + BOX_NODE_SIZE);
            self.synctex_record_box(b'h', p);
        }
    }

    #[inline(never)]
    fn synctex_math_on(&mut self, p: i32) {
        if !self.synctex_ignore() {
            if self.synctex_context_did_change(p + MEDIUM_NODE_SIZE) {
                self.synctex_run_recorder();
            }
            self.synctex_context_is(p, p + MEDIUM_NODE_SIZE);
            self.synctex_record_point(b'$', p + MEDIUM_NODE_SIZE); // always record synchronously
        }
    }

    #[inline(never)]
    fn synctex_rule_or_glue_on(&mut self, p: i32) {
        let ty = self.ty(p);
        let q = p + if ty == RULE_NODE { RULE_NODE_SIZE } else { MEDIUM_NODE_SIZE };
        match ty {
            RULE_NODE | GLUE_NODE | KERN_NODE => {
                if self.synctex_ignore_node(q) {
                    return;
                }
                self.synctex_context_is(p, q);
            }
            _ => {
                self.synctex_unknown_node(ty);
                let (curh, curv) = (self.cur_h, self.cur_v);
                let c = &mut self.v_image.sync;
                c.node = p;
                c.curh = curh;
                c.curv = curv;
                c.kern_pending = false;
                self.synctex_unknown_node(ty);
                return;
            }
        }
        // always record synchronously: maybe some text is outside the box
        match ty {
            RULE_NODE => self.synctex_record_node_rule(p),
            GLUE_NODE => self.synctex_record_point(b'g', q),
            _ => self.synctex_record_node_kern(p),
        }
    }

    #[cold]
    fn synctex_unknown_node(&mut self, ty: i32) {
        let msg = format!("\nSynchronize ERROR: unknown node type {ty}\n");
        self.wterm_s(msg.as_bytes());
    }

    #[inline(never)]
    fn synctex_kern_on(&mut self, p: i32, this_box: i32) {
        let q = p + MEDIUM_NODE_SIZE;
        if self.synctex_ignore_node(q) {
            return;
        }
        let (tag, line) = (self.sync_tag(q), self.sync_line(q));
        if self.synctex_context_did_change(q) {
            // the sync context has changed but was not yet recorded
            self.synctex_run_recorder();
            let first = self.v_image.sync.node == this_box;
            let c = &mut self.v_image.sync;
            c.node = p;
            c.tag = tag;
            c.line = line;
            if first {
                // first node in the list
                c.kern_pending = true;
            } else {
                // always record when the context has just changed and when not the first node
                c.kern_pending = false;
                self.synctex_record_node_kern(p);
            }
        } else {
            // just update the geometry and type (for future improvements)
            let c = &mut self.v_image.sync;
            c.node = p;
            c.tag = tag;
            c.line = line;
            c.kern_pending = true;
        }
    }

    #[inline(never)]
    fn synctex_current_on(&mut self) {
        if self.v_image.sync.off || !self.v_image.sync.file_open {
            return;
        }
        let (tag, line, curh, curv) = (self.v_image.sync.tag, self.v_image.sync.line, self.cur_h, self.cur_v);
        let same = self.synctex_same_v(curv);
        self.host.hv.synctex.record(|r| {
            r.ch(b'x');
            r.int(tag);
            r.ch(b',');
            r.int(line);
            r.ch(b':');
            r.int(curh);
            r.ch(b',');
            r.v(same, curv);
            r.ch(b'\n');
        });
    }

    /// `synctex_record_input`, a "Input:..." line.
    fn synctex_record_input(&mut self, tag: i32, name: &[u8]) {
        let name = &*platform::name_for_file_apis(name);
        let s = &mut self.host.hv.synctex;
        s.put(b"Input:");
        s.record(|r| {
            r.int(tag);
            r.ch(b':');
        });
        s.put(name);
        s.put(b"\n");
    }

    /// `synctex_record_settings`.
    fn synctex_record_settings(&mut self) {
        let (pdf, magnification) = (self.pdf_output_value > 0, self.v_image.sync.magnification);
        let s = &mut self.host.hv.synctex;
        s.put(if pdf { b"Output:pdf\nMagnification:" } else { b"Output:dvi\nMagnification:" });
        s.record(|r| {
            r.int(magnification);
            r.ch(b'\n');
        });
        // The unit is 1, SYNCTEX_UNIT_FACTOR, in all that follows.
        s.put(if pdf { b"Unit:1\nX Offset:0\nY Offset:0\n" } else { b"Unit:1\nX Offset:4736287\nY Offset:4736287\n" });
    }

    /// `synctex_record_anchor`, a "!..." line: how many bytes since the last one began.
    fn synctex_record_anchor(&mut self) {
        let at = self.v_image.sync.text_base + self.host.hv.synctex.at as i64;
        let total = (at - self.v_image.sync.anchor_at) as i32;
        self.v_image.sync.anchor_at = at;
        self.host.hv.synctex.record(|r| {
            r.ch(b'!');
            r.int(total);
            r.ch(b'\n');
        });
        self.v_image.sync.count += 1;
    }

    /// `synctex_record_postamble`, and in it `synctex_record_count`.
    fn synctex_record_postamble(&mut self) {
        self.synctex_record_anchor();
        let count = self.v_image.sync.count;
        let s = &mut self.host.hv.synctex;
        s.put(b"Postamble:\nCount:");
        s.record(|r| {
            r.int(count);
            r.ch(b'\n');
        });
        self.synctex_record_anchor();
        self.host.hv.synctex.put(b"Post scriptum:\n");
    }

    /// The "[...", "(...", "v..." and "h..." lines, which only differ in their first byte.
    fn synctex_record_box(&mut self, c: u8, p: i32) {
        let q = p + BOX_NODE_SIZE;
        let (tag, line, curh, curv) = (self.sync_tag(q), self.sync_line(q), self.v_image.sync.curh, self.v_image.sync.curv);
        let (wd, ht, dp) = (self.width(p), self.height(p), self.depth(p));
        let same = self.synctex_same_v(curv);
        self.host.hv.synctex.record(|r| {
            r.ch(c);
            r.int(tag);
            r.ch(b',');
            r.int(line);
            r.ch(b':');
            r.int(curh);
            r.ch(b',');
            r.v(same, curv);
            r.ch(b':');
            r.int(wd);
            r.ch(b',');
            r.int(ht);
            r.ch(b',');
            r.int(dp);
            r.ch(b'\n');
        });
        self.v_image.sync.count += 1;
    }

    /// The "g..." and "$..." lines.  `q` is the end of the node.
    fn synctex_record_point(&mut self, c: u8, q: i32) {
        let (tag, line, curh, curv) = (self.sync_tag(q), self.sync_line(q), self.v_image.sync.curh, self.v_image.sync.curv);
        let same = self.synctex_same_v(curv);
        self.host.hv.synctex.record(|r| {
            r.ch(c);
            r.int(tag);
            r.ch(b',');
            r.int(line);
            r.ch(b':');
            r.int(curh);
            r.ch(b',');
            r.v(same, curv);
            r.ch(b'\n');
        });
        self.v_image.sync.count += 1;
    }

    /// `synctex_record_node_kern`, a "k..." line.
    fn synctex_record_node_kern(&mut self, p: i32) {
        let q = p + MEDIUM_NODE_SIZE;
        let (tag, line, curh, curv, wd) = (self.sync_tag(q), self.sync_line(q), self.v_image.sync.curh, self.v_image.sync.curv, self.width(p));
        let same = self.synctex_same_v(curv);
        self.host.hv.synctex.record(|r| {
            r.ch(b'k');
            r.int(tag);
            r.ch(b',');
            r.int(line);
            r.ch(b':');
            r.int(curh);
            r.ch(b',');
            r.v(same, curv);
            r.ch(b':');
            r.int(wd);
            r.ch(b'\n');
        });
        self.v_image.sync.count += 1;
    }

    /// `synctex_record_node_rule`, a "r..." line.
    fn synctex_record_node_rule(&mut self, p: i32) {
        let q = p + RULE_NODE_SIZE;
        let (tag, line, curh, curv) = (self.sync_tag(q), self.sync_line(q), self.v_image.sync.curh, self.v_image.sync.curv);
        let (wd, ht, dp) = (self.rule_wd, self.rule_ht, self.rule_dp);
        let same = self.synctex_same_v(curv);
        self.host.hv.synctex.record(|r| {
            r.ch(b'r');
            r.int(tag);
            r.ch(b',');
            r.int(line);
            r.ch(b':');
            r.int(curh);
            r.ch(b',');
            r.v(same, curv);
            r.ch(b':');
            r.int(wd);
            r.ch(b',');
            r.int(ht);
            r.ch(b',');
            r.int(dp);
            r.ch(b'\n');
        });
        self.v_image.sync.count += 1;
    }

    /// `synctex_record_node_pdfrefxform`, a "f..." line.
    fn synctex_record_node_pdfrefxform(&mut self, objnum: i32) {
        let (curh, curv) = (self.cur_h, self.cur_v);
        self.v_image.sync.curh = curh;
        self.v_image.sync.curv = curv;
        if self.synctex_ignore() {
            return;
        }
        let same = self.synctex_same_v(curv);
        self.host.hv.synctex.record(|r| {
            r.ch(b'f');
            r.int(objnum);
            r.ch(b':');
            r.int(curh);
            r.ch(b',');
            r.v(same, curv);
            r.ch(b'\n');
        });
        self.v_image.sync.count += 1;
    }

    /// The text so far becomes a member of the file, if there is any: or plain text in the file, for `-synctex` below 0, whose text is not held back.
    pub(crate) fn synctex_cut(&mut self) {
        if !self.v_image.sync.file_open || self.host.hv.synctex.at == 0 {
            return;
        }
        let at = self.host.hv.synctex.at;
        self.v_image.sync.text_base += at as i64;
        self.v_image.sync.members += 1;
        self.host.hv.synctex.at = 0;
        if self.v_image.sync.no_gz {
            let s = &mut self.host.hv.synctex;
            if let Some(f) = &mut s.file {
                f.bytes(&s.text[..at]);
            }
            return;
        }
        let mut text = core::mem::take(&mut self.host.hv.synctex.text);
        let room = text.len();
        text.truncate(at);
        if self.pdfout_workers_take(&mut text, LEVEL_SYNCTEX) {
            let s = &mut self.host.hv.synctex;
            s.members.push(None);
            s.with_workers += 1;
        } else {
            let mut scratch = core::mem::take(&mut self.host.hv.synctex.scratch);
            scratch.clear();
            self.pdfout_gzip_here(&text, LEVEL_SYNCTEX, &mut scratch);
            let s = &mut self.host.hv.synctex;
            s.members.push(Some(scratch.as_slice().to_vec()));
            s.scratch = scratch;
        }
        // The workers' spare one if they gave one, with the room this one had, since the next page is likely to need it.
        text.resize(room, 0);
        self.host.hv.synctex.text = text;
    }

    /// Waits for the workers and writes what they have to the file, and what was made here.  The file is then whole up to the length this returns: all but the text since the last member.
    /// Called from `pdfout_settle`, so a checkpoint or a page record that settles the PDF has this as well.
    pub fn synctex_settle(&mut self) -> u64 {
        if !self.v_image.sync.file_open {
            return 0;
        }
        if !self.host.hv.synctex.members.is_empty() {
            let _p = crate::prof_scope!("synctex write");
            let made = if self.host.hv.synctex.with_workers > 0 { self.pdfout_gzip_finish() } else { Vec::new() };
            let mut made = made.into_iter();
            let s = &mut self.host.hv.synctex;
            s.with_workers = 0;
            let parts: Vec<Vec<u8>> = s.members.drain(..).map(|m| m.or_else(|| made.next()).expect("one for each the workers had")).collect();
            if let Some(f) = &mut s.file {
                f.parts(&parts);
            }
        }
        let s = &mut self.host.hv.synctex;
        match &mut s.file {
            Some(f) => {
                f.flush();
                f.total
            }
            None => 0,
        }
    }

    /// Why a checkpoint cannot be taken now, if so.  A sheet that came to nothing can leave a member, which a run with an error shows anyway.
    pub(crate) fn synctex_refusal(&self) -> Option<&'static str> {
        if self.v_image.sync.members != 0 { Some("SyncTeX's file holds a sheet") } else { None }
    }

    /// What a checkpoint keeps of this that is not in the image: the name of the file and its text so far, if it is open, and the root's name if that still waits.
    pub(crate) fn synctex_for_checkpoint(&self) -> (Option<(Vec<u8>, Vec<u8>)>, Option<Vec<u8>>) {
        let s = &self.host.hv.synctex;
        let file = if self.v_image.sync.file_open { Some((s.busy_name.clone(), s.text[..s.at].to_vec())) } else { None };
        (file, s.root_name.clone())
    }

    /// A chain is being written from here on: what goes to the file is kept for the next record.
    pub(crate) fn synctex_journal_on(&mut self) {
        let s = &mut self.host.hv.synctex;
        s.journalling = true;
        if let Some(f) = &mut s.file {
            f.journal = Some(Vec::new());
        }
    }

    pub(crate) fn synctex_journal_off(&mut self) {
        let s = &mut self.host.hv.synctex;
        s.journalling = false;
        if let Some(f) = &mut s.file {
            f.journal = None;
        }
    }

    /// What the file gained since the last time, all of it settled.
    pub(crate) fn synctex_drain_journal(&mut self) -> Vec<u8> {
        self.host.hv.synctex.file.as_mut().and_then(|f| f.journal.as_mut()).map_or(Vec::new(), core::mem::take)
    }

    /// A write to the file has failed, which only this process knows of.
    pub(crate) fn synctex_write_failed(&self) -> bool {
        self.host.hv.synctex.file.as_ref().is_some_and(|f| f.has_failed_already())
    }

    /// A restore: the file comes into being as in the run it stands for, with `on_disk` in it: nothing at a preamble, and for a page record the members of the sheets behind it.  False if it could not.
    pub(crate) fn synctex_restore(&mut self, file: Option<(Vec<u8>, Vec<u8>)>, root: Option<Vec<u8>>, on_disk: &[u8]) -> bool {
        let s = &mut self.host.hv.synctex;
        s.root_name = root;
        if let Some((busy, text)) = file {
            let Some(f) = OutFile::restored(&busy, on_disk, 0) else { return false };
            s.file = Some(f);
            s.busy_name = busy;
            s.put(&text);
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn written(v: i32) -> Vec<u8> {
        let mut b = [0u8; MAX_RECORD];
        let mut r = Rec { b: &mut b, at: 0 };
        r.int(v);
        let n = r.at;
        b[..n].to_vec()
    }

    #[test]
    fn a_number_is_what_percent_i_gives() {
        let mut vals = vec![0, 1, 9, 10, 99, 100, 999, 1000, 9999, 10000, 99999, 100000, 999999, 1000000, 9999999, 10000000, 99999999, 100000000, 100000001, 999999999, 1000000000, 2147483647, -1, -10, -99999999, -100000000, -2147483647, i32::MIN];
        let mut x = 0x1234_5678_9ABC_DEF1u64;
        for _ in 0..200_000 {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            // All lengths, not only the long ones that most numbers are.
            vals.push(((x as u32) >> ((x >> 40) as u32 % 32)) as i32);
        }
        for v in vals {
            assert_eq!(written(v), v.to_string().into_bytes(), "{v}");
        }
    }

    #[test]
    fn a_record_has_the_room_it_may_need() {
        let mut s = HVars::default();
        for _ in 0..10_000 {
            s.record(|r| {
                r.ch(b'(');
                for _ in 0..7 {
                    r.int(i32::MIN);
                    r.ch(b',');
                }
                r.ch(b'\n');
            });
            assert!(s.at <= s.text.len());
        }
        assert_eq!(s.at, 10_000 * (1 + 7 * 12 + 1));
    }

    #[test]
    fn what_goes_in_by_pieces_comes_out_in_order() {
        let mut s = HVars::default();
        s.put(b"Input:");
        s.record(|r| {
            r.int(3);
            r.ch(b':');
        });
        s.put(&vec![b'a'; 3 * TEXT_START]);
        s.put(b"\n");
        assert!(s.text[..s.at].starts_with(b"Input:3:aaa"));
        assert!(s.text[..s.at].ends_with(b"aaa\n"));
        assert_eq!(s.at, 8 + 3 * TEXT_START + 1);
    }
}
