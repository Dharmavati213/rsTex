//! The native diagram path, DESIGN.md section 10: the engine takes over one `tikzcd` from real PGF when it can prove the result is
//! the same, and otherwise nothing has happened and the macro runs.
//!
//! # How it is reached
//!
//! Nothing here is on a hot path.  `\tikzcd` and `\endtikzcd` are armed as the checkpoint's control sequences are (`checkpoint.rs`): `CD_ARMED` on their
//! `eq_type` sends every fetch down `get_next`'s existing `cur_cmd>=outer_call` path to `check_outer_validity`, and there `native_trap` puts `cur_cmd` right.  An
//! assignment replaces the word and so disarms them; a group's end brings the armed word back.  They are armed when the file of the library has been opened and
//! `\tikzcd` has come into being, by the same `ck.pending` test at the tail of `prefixed_command` that a checkpoint uses.
//!
//! # Reading ahead without a trace
//!
//! `native_read` reads the options and the body from the input stack as `get_next` would, up to the matching `\end{tikzcd}`, but it changes nothing: not `loc`, not `line`,
//! not `buffer`, not the string pool.  A line not yet read is looked at where the file lies in memory, and a name that the hash table does not hold is a decline: looked up,
//! never created.  So a decline has nothing to undo.  Only `native_commit`, once all is decided, moves the input to the `\end`.
//!
//! # Deciding
//!
//! The tokens are written as bytes for the `commute` library, with `pos` from each byte to its token, and it parses them.  What it leaves in cells and labels is then
//! judged by real meanings from eqtb, `native_vet`.  A layout with empty boxes shows what only depends on the structure, such as an arrow to an empty cell.
//!
//! # Typesetting
//!
//! Cells and labels are typeset by the main loop, in real boxes, in the order real tikz-cd has them.  `CD_STEP`, a location with no name in the part of eqtb that the
//! primitives' table does not reach, is armed for good: it ends each token list that `native_step` starts, and brings control back here with the Rust stack as `main_control`
//! had it.  The boxes wait in an hbox of their own, as `\pgf@matrix@box` and the others do in real PGF, and the box of the whole diagram goes
//! on the list through `box_end`, after a real `\leavevmode`.  `\end{tikzcd}` itself is real, so its hooks and `\@checkend` run; its fetch of `\endtikzcd` comes here and turns to `\relax`.
#![forbid(unsafe_code)]
#![deny(unreachable_patterns, unused_variables)]
#![allow(non_snake_case, unused_mut, unused_imports, dead_code)]

use crate::access::*;
use crate::consts::*;
use crate::host::*;
use crate::raw::*;
use crate::state::*;
use commute::pdf::{self, PdfSink};
use commute::{BoxDim, BoxRef, Color, Diagram, FontParams, LabelFont, Policy, Span, Unsupported};
use std::io::Write as _;

#[cfg(feature = "commute-probe")]
#[path = "native_probe.rs"]
pub mod probe;
#[path = "native_table.rs"]
mod native_table;

/// Added to an `eq_type` to arm a control sequence for this module.  `checkpoint.rs` has `CK_ARMED` below it.
pub const CD_ARMED: i32 = 0x100;
const _: () = assert!(crate::checkpoint::CK_ARMED < CD_ARMED);
/// Both bits: what a reader of `eqtb` that bypasses `get_next` has to mask.
pub const ARMED_BITS: i32 = crate::checkpoint::CK_ARMED | CD_ARMED;

/// The token that ends a list of `native_step`'s.  `prim_lookup`'s first probes stop at `PRIM_PRIME`, and its other entries come down from `PRIM_SIZE`, so this is free: checked below.
const CD_STEP: i32 = PRIM_EQTB_BASE + PRIM_PRIME + 1;

/// What a body may hold, in tokens.  A diagram is far below this.  A read that ends here takes no longer than the reading of those tokens would take in any case.
const MAX_TOKENS: usize = 1 << 16;
/// How many macros one diagram's walk may visit, and how deep.
const WALK_BUDGET: u32 = 1 << 16;
const WALK_DEPTH: u32 = 64;

/// Indexes of `Vars::names`.
const N_BEGIN: i32 = 0;
const N_END: i32 = 1;
const N_BEGINGROUP: i32 = 2;
const N_ENDGROUP: i32 = 3;
const N_AFTERGROUP: i32 = 4;
const N_UNSKIP: i32 = 5;
const N_IGNORESPACES: i32 = 6;
const N_RELAX: i32 = 7;
const N_SELECTFONT: i32 = 8;
const N_LEAVEVMODE: i32 = 9;
const N_EVERYMATH: i32 = 10;
const N_EXPANDAFTER: i32 = 11;
const N_THE: i32 = 12;
const N_SCRIPTSTYLE: i32 = 13;
const N_CURRENT_COLOR: i32 = 14;
const N_SERIAL: i32 = 15;
const N_FIG_COUNT: i32 = 16;
const N_MATRIX_ROW: i32 = 17;
const N_MATRIX_COL: i32 = 18;
const N_MATRIX_NCOLS: i32 = 19;
const N_IFPICTURE: i32 = 20;
const NAMES: [&[u8]; 21] = [
    b"begin",
    b"end",
    b"begingroup",
    b"endgroup",
    b"aftergroup",
    b"unskip",
    b"ignorespaces",
    b"relax",
    b"selectfont",
    b"leavevmode",
    b"everymath",
    b"expandafter",
    b"the",
    b"scriptstyle",
    b"current@color",
    b"pgf@picture@serial@count",
    b"tikz@fig@count",
    b"pgfmatrixcurrentrow",
    b"pgfmatrixcurrentcolumn",
    b"pgf@matrix@numberofcolumns",
    b"ifpgfpicture",
];

crate::pod_struct! {
    /// POD state of the path, embedded in `image.rs`'s `Vars`.  A checkpoint keeps it: `\tikzcd` is armed in the image.
    pub struct Vars {
        /// Not `--no-commute`, `RSTEX_NO_COMMUTE` or `-ini`.
        pub on: bool,
        /// `--commute-report`.
        pub report: bool,
        /// `--commute-report=names`: also the name of a macro that stood in the way.
        pub report_names: bool,
        /// The library's file has been opened and `\tikzcd` is still to come.
        pub watching: bool,
        /// A diagram is on its way through the main loop.
        pub active: bool,
        /// The next fetch of `\endtikzcd` is ours.
        pub pending_end: bool,
        pub tikzcd_cs: i32,
        pub endtikzcd_cs: i32,
        /// Control sequences looked up once, after the first `\tikzcd`: 0 if there is no such name.
        pub names: A<i32, 21>,
        pub names_done: bool,
        /// What `native_table::GUARD` found, three cells for each entry and each one up so that it is no end of a list: the list's place, the control sequence, and its `eq_type` and
        /// `equiv` as last checked.  A macro's block has a reference from here, so that the same place can only mean the same tokens.  In the image: a restored run has no need to check again.
        pub guards: i32,
    }
}

/// What the step token does next.
#[derive(Copy, Clone, PartialEq, Eq, Default)]
enum Stage {
    #[default]
    Idle,
    /// Inside `\begingroup$\relax$`: the fonts are as a cell will find them.
    Fonts,
    /// After that `\endgroup`.
    FirstBox,
    /// A box has just gone onto the list of the waiting hbox.
    AfterBox,
    /// After the `\leavevmode`.
    Finish,
}

/// What a macro's body was found to hold, for a body at the place `ptr`.
#[derive(Copy, Clone, Default)]
struct Memo {
    ptr: i32,
    /// `deps[..]`, the control sequences the body mentions.
    deps: (u32, u32),
    /// Stamp of the diagram it was last walked for, and what it came to then.
    stamp: u32,
    verdict: u8,
    /// The body by itself: 0 is clean, otherwise an index into `REASONS`.
    local: u8,
}

/// Non-POD state of this module.  All of it is a workspace or a memo: a restored run starts with nothing here and fills it again.
#[derive(Default)]
pub struct HVars {
    d: Option<Box<Diagram>>,
    /// The tokens, the line each came from (0 from a list), whether a `&` is on the file's terms.
    tokens: Vec<i32>,
    lines: Vec<i32>,
    bytes: Vec<u8>,
    pos: Vec<u32>,
    /// Where the body ends.
    found: Found,
    /// A read that reached this place in a file and found no end: `(in_open, line, loc)`.  Whatever is read from there on would fail again.
    dead: (i32, i32, i32),
    memo: Vec<Memo>,
    deps: Vec<i32>,
    stamp: u32,
    budget: u32,
    stage: Stage,
    /// The index of the next box, cells then labels; the boxes so far, in `boxes`.
    next: usize,
    boxes: Vec<i32>,
    cell_dims: Vec<BoxDim>,
    label_dims: Vec<BoxDim>,
    /// `save_ptr`, `cur_level` of the waiting hbox.
    level: i32,
    end_line: i32,
    font: Option<FontParams>,
    ops: Vec<u8>,
    list: Vec<i32>,
    scratch: Vec<u8>,
    /// Which entry of `GUARD` was not as installed, for the report.
    detail: Option<usize>,
    /// What each cell starts with, or 0.
    cell_prefix: i32,
    /// The last macro that stopped the walk, and under `--commute-report=names` the way there, and what the library stopped at.
    culprit: i32,
    chain: Vec<i32>,
    at_text: Vec<u8>,
    /// What `\\tikzcdset` has done that the library can be told of, as tokens of a key list, and the bytes of it at the start of `bytes`.
    preset: Vec<i32>,
    preset_bytes: usize,
    library_gap: bool,
    preset_in_tokens: usize,
    /// `>` is not the installed tip.  It matters where something uses it.
    soft_tips: bool,
    styles_seen: u32,
    every_seen: u32,
    /// `hash_high` when `ABSENT` was last looked through, and those of its names that exist.
    absent_at: i32,
    absent_exist: Vec<i32>,
    #[cfg(feature = "commute-probe")]
    pub probe: probe::ProbeState,
}

/// Where the read stopped.
#[derive(Copy, Clone, Default)]
struct Found {
    /// The level of the input stack that holds the `\end`.
    level: i32,
    /// In a token list: where the `\end` is.  A list level's own `loc` if `param` is not 0.
    loc: i32,
    /// The parameter's list it is in, then: where in it.
    param: i32,
    param_loc: i32,
    /// In a file: lines to read, and the offset of the `\end` in the buffer.
    lines: i32,
    is_file: bool,
}

impl HVars {
    /// An offset in the library's buffer as one in `bytes`.
    #[inline]
    fn from_library(&self, at: usize) -> usize {
        if self.library_gap && at > self.preset_bytes { at - 1 } else { at }
    }
}

/// The reader's place on the input stack.
struct Rd {
    level: i32,
    rec: InStateRecord,
    loc: i32,
    /// Inside a parameter: its list and the place in it.
    param: i32,
    param_loc: i32,
    /// The current line of a file, its end-of-line included, and where in it.
    scratch: Vec<u8>,
    i: usize,
    tok_i: usize,
    base: i32,
    state: i32,
    /// Where the file's next line starts, and how many lines were looked at.
    file_pos: usize,
    lines: i32,
    started: bool,
    /// Where the token just read starts.
    at: Found,
}

/// Reasons, as `--commute-report` says them.  Index 0 means no reason.
const R_UNDEFINED: u8 = 1;
const R_STRUCTURE: u8 = 2;
const R_CATCODE: u8 = 3;
const R_CSNAME: u8 = 4;
const R_PRIMITIVE: u8 = 5;
const R_ALIGNMENT: u8 = 6;
const R_PGF: u8 = 7;
const R_OUTER: u8 = 8;
const R_BUDGET: u8 = 9;
const R_ENV: u8 = 10;
const R_POSITION: u8 = 11;
const REASONS: [&str; 12] = [
    "",
    "an undefined control sequence in a cell",
    "a macro in a cell holds a structural token",
    "a macro in a cell changes how text is read",
    "a macro in a cell computes a name",
    "a primitive that cannot stay in a cell",
    "alignment tokens in a cell",
    "PGF's or TikZ's internals in a cell",
    "an outer macro in a cell",
    "the macros of a cell go too deep",
    "an environment in a cell has no meaning that can be checked",
    "something in a cell depends on its place on the page",
];

/// tikz-cd's own keys, which a style of `/tikz` cannot use: spec 5.1.
const CD_ONLY: [&[u8]; 55] = [
    // These mean something else in `/tikz`, which the library does not tell apart.
    b"dashed", b"shift", b"xshift", b"yshift", b"shorten",
    b"rightarrow", b"leftarrow", b"leftrightarrow", b"Rightarrow", b"Leftarrow", b"Leftrightarrow", b"mapsto", b"mapsfrom", b"Mapsto", b"Mapsfrom", b"hookrightarrow", b"hookleftarrow",
    b"rightharpoonup", b"rightharpoondown", b"leftharpoonup", b"leftharpoondown", b"rightarrowtail", b"leftarrowtail", b"twoheadrightarrow", b"twoheadleftarrow", b"dash", b"equal", b"equals",
    b"dashrightarrow", b"dashleftarrow", b"rightsquigarrow", b"leftsquigarrow", b"leftrightsquigarrow", b"to head", b"no head", b"two heads", b"harpoon", b"harpoon'", b"no tail", b"tail", b"hook", b"hook'", b"maps to",
    b"phantom", b"description", b"marking", b"crossing over", b"shift left", b"shift right", b"start anchor", b"end anchor", b"squiggly", b"double line", b"labels", b"transform nodes",
];

/// Whether the bytes of this token start with a letter.
#[inline]
fn next_is_letter(t: Option<&i32>) -> bool {
    t.is_some_and(|&t| t < CS_TOKEN_FLAG && ((t & 255) as u8).is_ascii_alphabetic())
}

/// The parser's questions are answered by `native_vet` once it is done, with the tokens in hand: so here everything may stay.
struct AfterVet;

impl Policy for AfterVet {
    fn control_sequence(&self, _: &[u8]) -> bool {
        true
    }

    fn environment(&self, _: &[u8]) -> bool {
        true
    }
}

impl Tex {
    // ------------------------------------------------------------------------------------------- arming

    /// `start_input` has opened a file.  Only the library's own says that `\tikzcd` is about to be the real one.
    #[cold]
    pub fn native_input_opened(&mut self) {
        if self.ini_version {
            return;
        }
        let (a, b) = (self.cur_name, self.cur_ext);
        let name = self.str_pool.range(self.str_start[a], self.str_start[a + 1]);
        let ext = self.str_pool.range(self.str_start[b], self.str_start[b + 1]);
        let whole = name.ends_with(b"tikzlibrarycd.code") && ext == b".tex" || name.ends_with(b"tikzlibrarycd.code.tex") && ext.is_empty();
        if whole {
            self.v_image.cd.watching = true;
            self.v_image.ck.pending |= crate::checkpoint::PENDING_NATIVE;
        }
    }

    /// The tail of `prefixed_command` while the library loads.  Takes an assignment or two past the definition of `\tikzcd`.
    #[cold]
    pub fn native_after_prefixed(&mut self) {
        if !self.v_image.cd.on {
            self.v_image.ck.pending &= !crate::checkpoint::PENDING_NATIVE;
            return;
        }
        let cs = self.find_cs(b"tikzcd");
        if cs == UNDEFINED_CONTROL_SEQUENCE {
            return;
        }
        let t = self.eq_type(cs);
        if t != CALL {
            return;
        }
        self.v_image.ck.pending &= !crate::checkpoint::PENDING_NATIVE;
        self.v_image.cd.watching = false;
        if self.prim_text(PRIM_PRIME + 1) != 0 {
            return;
        }
        self.v_image.cd.tikzcd_cs = cs;
        self.set_eq_type(cs, t + CD_ARMED);
        // The step's location: armed for good, with nothing that can reach it but our own lists.
        self.eqtb[CD_STEP] = Word::hb(256, RELAX + CD_ARMED, LEVEL_ONE);
        let relax = self.text(FROZEN_RELAX);
        self.set_text(CD_STEP, relax);
    }

    /// A control sequence armed by `CD_ARMED` was fetched.
    #[cold]
    #[inline(never)]
    pub fn native_trap(&mut self) {
        self.cur_cmd -= CD_ARMED;
        let cs = self.cur_cs;
        if cs == CD_STEP {
            self.native_step();
            return;
        }
        // Under tracing a self-assignment says "reassigning" only if the word is what the assignment would make it.
        if self.tracing_assigns() > 0 || self.tracing_restores() > 0 {
            self.set_eq_type(cs, self.cur_cmd);
        }
        if cs == self.v_image.cd.endtikzcd_cs {
            if self.v_image.cd.pending_end {
                self.v_image.cd.pending_end = false;
                self.cur_cmd = RELAX;
                self.cur_chr = 256;
            }
            return;
        }
        if cs == self.v_image.cd.tikzcd_cs
            && self.cur_cmd == CALL
            && self.v_image.ck.at_big_switch
            && self.expand_depth_count == 0
            && self.scanner_status == NORMAL
            && !self.v_image.cd.active
        {
            self.native_try();
        }
    }

    fn native_try(&mut self) {
        match self.native_decide() {
            Err(reason) => {
                if self.v_image.cd.report {
                    let mut text = reason.as_bytes().to_vec();
                    if let Some(k) = self.host.hv.native.detail.take() {
                        text.extend_from_slice(b": ");
                        text.extend_from_slice(native_table::GUARD[k].0);
                    }
                    // The name in question comes from the document, so only if asked for, and tests/corpus never asks for it on a private one.
                    let at = core::mem::take(&mut self.host.hv.native.at_text);
                    if !at.is_empty() {
                        text.extend_from_slice(b": ");
                        text.extend_from_slice(&at[..at.len().min(60)]);
                    }
                    let cs = core::mem::take(&mut self.host.hv.native.culprit);
                    if cs != 0 && self.v_image.cd.report_names {
                        text.extend_from_slice(b": ");
                        let name = self.native_name(cs).to_vec();
                        text.extend_from_slice(&name);
                        for c in core::mem::take(&mut self.host.hv.native.chain) {
                            text.extend_from_slice(b" < ");
                            let name = self.native_name(c).to_vec();
                            text.extend_from_slice(&name);
                        }
                    }
                    self.native_report(b"[[commute fallback]] ", &text);
                }
            }
            Ok(()) => {
                if self.v_image.cd.report {
                    self.native_report(b"[[commute native]]", b"");
                }
                self.native_commit();
                self.native_start();
            }
        }
    }

    /// One line of the log, if `--commute-report` asked for it.  The rest of the log goes on as if it were not there: if it stands in the middle of one of TeX's lines it ends in ` %`, and a
    /// tool that removes it and the line end before it gets the oracle's line back.
    fn native_report(&mut self, a: &[u8], b: &[u8]) {
        if !self.log_opened {
            return;
        }
        let inside = self.file_offset > 0;
        if inside {
            self.wlog_cr();
        }
        self.wlog_s(a);
        self.wlog_s(b);
        if inside {
            self.wlog_s(b" %");
        }
        self.wlog_cr();
    }

    // ------------------------------------------------------------------------------------------- deciding

    fn native_names(&mut self) {
        if self.v_image.cd.names_done {
            return;
        }
        self.v_image.cd.names_done = true;
        for (k, n) in NAMES.iter().enumerate() {
            let cs = self.find_cs(n);
            self.v_image.cd.names[k as i32] = if cs == UNDEFINED_CONTROL_SEQUENCE { 0 } else { cs };
        }
        let e = self.find_cs(b"endtikzcd");
        if e != UNDEFINED_CONTROL_SEQUENCE {
            let t = self.eq_type(e);
            if t == CALL {
                self.v_image.cd.endtikzcd_cs = e;
                self.set_eq_type(e, t + CD_ARMED);
            }
        }
    }

    fn native_decide(&mut self) -> Result<(), &'static str> {
        self.native_names();
        self.native_checks()?;
        self.native_guards()?;
        self.native_read()?;
        self.native_bytes()?;
        let font = self.native_font_params();
        let mut d = self.host.hv.native.d.take().unwrap_or_default();
        let r = self.native_parse(&mut d, font);
        self.host.hv.native.d = Some(d);
        r
    }

    fn native_parse(&mut self, d: &mut Diagram, font: FontParams) -> Result<(), &'static str> {
        // A key it does not know may be a style of the document's: it hears of it and starts again.
        let mut tries = 0;
        loop {
            let n = self.host.hv.native.preset_bytes;
            let r = d.parse(&self.host.hv.native.bytes[..n], &self.host.hv.native.bytes[n..], font, &AfterVet);
            let Err(u) = r else { break };
            tries += 1;
            let unknown = matches!(u.reason, "unknown arrow key" | "unknown label key" | "unknown cell key" | "unknown diagram option");
            let key = d.text(u.at).to_vec();
            if self.v_image.cd.report_names {
                self.host.hv.native.at_text = key.clone();
            }
            if !unknown || tries > 16 {
                return Err(u.reason);
            }
            let before = self.host.hv.native.preset.len();
            if !self.native_user_style(&key) {
                return Err(u.reason);
            }
            // The new items go after the old ones, and so before the body.
            let hv = &mut self.host.hv.native;
            let (old, added) = (hv.preset_in_tokens, hv.preset[before..].to_vec());
            let line_zeros = vec![0; added.len()];
            hv.tokens.splice(old..old, added.iter().copied());
            hv.lines.splice(old..old, line_zeros);
            hv.preset_in_tokens += added.len();
            self.native_bytes()?;
        }
        self.native_offsets(d);
        self.native_vet(d)?;
        // What only depends on the structure shows with empty boxes, and it has to show now: after this the cells are typeset.
        let hv = &mut self.host.hv.native;
        hv.cell_dims.clear();
        hv.cell_dims.resize(d.cells().len(), BoxDim::default());
        hv.label_dims.clear();
        hv.label_dims.resize(d.labels().len(), BoxDim::default());
        d.layout(&hv.cell_dims, &hv.label_dims).map_err(|u| u.reason)?;
        if d.opacities().next().is_some() {
            return Err("opacity needs a page resource");
        }
        if self.host.hv.native.soft_tips {
            let b = &self.host.hv.native.bytes;
            if b.windows(2).any(|w| w == b"->" || w == b"<-") || b.windows(11).any(|w| w == b"arrow style") {
                return Err("the tip > is not as installed, and this diagram uses it");
            }
        }
        self.native_absent_numbered(d.rows(), d.cols())
    }

    /// What `.try` looks for by row and column.
    fn native_absent_numbered(&self, rows: usize, cols: usize) -> Result<(), &'static str> {
        let exists = |t: &Tex, name: &[u8]| {
            let cs = t.find_cs(name);
            cs != UNDEFINED_CONTROL_SEQUENCE && !matches!(t.eq_type(cs), UNDEFINED_CS | RELAX)
        };
        const BAD: &str = "a style of a row or a column";
        let mut buf = Vec::with_capacity(64);
        for r in 1..=rows {
            buf.clear();
            let _ = write!(buf, "pgfk@/tikz/row {r}/.@cmd");
            if exists(self, &buf) {
                return Err(BAD);
            }
            for c in 1..=cols {
                buf.clear();
                let _ = write!(buf, "pgfk@/tikz/row {r} column {c}/.@cmd");
                if exists(self, &buf) {
                    return Err(BAD);
                }
            }
        }
        for c in 1..=cols {
            buf.clear();
            let _ = write!(buf, "pgfk@/tikz/column {c}/.@cmd");
            if exists(self, &buf) {
                return Err(BAD);
            }
        }
        buf.clear();
        let _ = write!(buf, "pgfk@/tikz/commutative diagrams/{rows}-row diagram/.@cmd");
        if rows != 1 && exists(self, &buf) {
            return Err(BAD);
        }
        Ok(())
    }

    /// Everything real tikz-cd reads is as installed, as far as `native_table` knows it, or is a change the library can be told of.  Checks by place and kind, which with the reference held
    /// proves the tokens are those that were hashed.
    fn native_guards(&mut self) -> Result<(), &'static str> {
        if self.v_image.cd.guards == 0 {
            self.native_guards_build();
        }
        const CHANGED: &str = "a macro or key of tikz-cd or PGF is not as installed";
        let hv = &mut self.host.hv.native;
        hv.preset.clear();
        hv.soft_tips = false;
        hv.styles_seen = 0;
        hv.every_seen = 0;
        let mut p = self.v_image.cd.guards + 1;
        for k in 0..native_table::GUARD.len() {
            let cs = self.toks[p] - 1;
            let (t, e) = (self.toks[p + 1] - 1, self.toks[p + 2] - 1);
            let w = self.eqtb[cs];
            if cs == 0 || w.b0() & !ARMED_BITS != t || w.rh() != e {
                if !self.native_guard_again(k, p) && !self.native_guard_told(k) {
                    self.host.hv.native.detail = Some(k);
                    return Err(CHANGED);
                }
            }
            p += 3;
        }
        self.native_styles_finish()?;
        // A new name can only come with a new place in the table, and the names that exist are few.
        let hv = &mut self.host.hv.native;
        if hv.absent_at != self.hash_high || hv.absent_at == 0 {
            hv.absent_at = self.hash_high;
            let mut exist = core::mem::take(&mut hv.absent_exist);
            exist.clear();
            for n in native_table::ABSENT {
                let cs = self.find_cs(n);
                if cs != UNDEFINED_CONTROL_SEQUENCE {
                    exist.push(cs);
                }
            }
            self.host.hv.native.absent_exist = exist;
        }
        for k in 0..self.host.hv.native.absent_exist.len() {
            let cs = self.host.hv.native.absent_exist[k];
            if !matches!(self.eq_type(cs), UNDEFINED_CS | RELAX) {
                return Err("a style that tikz-cd looks for is defined");
            }
        }
        Ok(())
    }

    /// An entry of `GUARD` is not as installed.  True if it is something `\\tikzcdset` does that the library takes as a key list before the diagram.
    fn native_guard_told(&mut self, k: usize) -> bool {
        const CD: &[u8] = b"pgfk@/tikz/commutative diagrams/";
        let name = native_table::GUARD[k].0;
        if name == b"pgf@ar@means@>" {
            self.host.hv.native.soft_tips = true;
            return true;
        }
        if name == b"iftikzcd@mathmode" {
            self.native_preset_text(b"math mode=false,");
            return true;
        }
        if let Some(rest) = name.strip_prefix(b"pgf@ar@means@".as_slice()).filter(|r| r.starts_with(b"tikzcd ")) {
            let _ = rest;
            return self.native_style_entry(k);
        }
        let Some(key) = name.strip_prefix(CD) else { return false };
        for (bit, every) in [&b"every arrow"[..], b"every label", b"every cell", b"every matrix", b"every diagram"].iter().enumerate() {
            if key.starts_with(every) && (&key[every.len()..] == b"/.@cmd" || &key[every.len()..] == b"/.@body") {
                if self.host.hv.native.every_seen & 1 << bit != 0 {
                    return true;
                }
                self.host.hv.native.every_seen |= 1 << bit;
                return self.native_every_appended(every);
            }
        }
        for prefix in [&b"row sep/"[..], b"column sep/"] {
            if key.starts_with(prefix) && !key.contains(&b'.') {
                return self.native_value_key(key);
            }
        }
        if key == b"background color" || key == b"crossing over clearance" {
            return self.native_value_key(key);
        }
        false
    }

    /// A key with a value, as the library is told of it: `key={value},`.
    fn native_value_key(&mut self, key: &[u8]) -> bool {
        let full: Vec<u8> = b"pgfk@/tikz/commutative diagrams/".iter().chain(key).copied().collect();
        let cs = self.find_cs(&full);
        if cs == UNDEFINED_CONTROL_SEQUENCE {
            return false;
        }
        let t = self.eq_type(cs) & !ARMED_BITS;
        if t < CALL || t >= END_TEMPLATE || self.toks[self.equiv(cs) + 1] != END_MATCH_TOKEN {
            return false;
        }
        self.native_preset_text(key);
        self.native_preset_text(b"={");
        let mut p = self.equiv(cs) + 2;
        while self.toks[p] != 0 {
            let x = self.toks[p];
            self.host.hv.native.preset.push(x);
            p += 1;
        }
        self.native_preset_text(b"},");
        true
    }

    /// `every arrow/.append style` and the like, or `arrows=`, which is the same: the installed style, then more `\\pgfkeysalso{...}`.
    fn native_every_appended(&mut self, every: &[u8]) -> bool {
        let base: Vec<u8> = b"pgfk@/tikz/commutative diagrams/".iter().chain(every).copied().collect();
        let body_name: Vec<u8> = base.iter().chain(b"/.@body").copied().collect();
        let cmd_name: Vec<u8> = base.iter().chain(b"/.@cmd").copied().collect();
        let (body, cmd) = (self.find_cs(&body_name), self.find_cs(&cmd_name));
        let Some(&(_, stock)) = native_table::GUARD.iter().find(|e| e.0 == body_name.as_slice()) else { return false };
        if body == UNDEFINED_CONTROL_SEQUENCE || cmd == UNDEFINED_CONTROL_SEQUENCE {
            return false;
        }
        if self.eq_type(body) != CALL || self.eq_type(cmd) != LONG_CALL {
            return false;
        }
        let also = self.find_cs(b"pgfkeysalso");
        let start = self.equiv(body) + 1;
        if self.toks[start] != END_MATCH_TOKEN {
            return false;
        }
        // `.@cmd` is that body with the parameter text of a key: `#1\pgfeov`.
        let mut q = self.equiv(cmd) + 1;
        while self.toks[q] != END_MATCH_TOKEN {
            if self.toks[q] == 0 {
                return false;
            }
            q += 1;
        }
        let mut r = start;
        while self.toks[r] != 0 && self.toks[r] == self.toks[q] {
            r += 1;
            q += 1;
        }
        if self.toks[r] != 0 || self.toks[q] != 0 {
            return false;
        }
        // The groups of `\pgfkeysalso`.
        let mut p = start + 1;
        let mut first = true;
        while self.toks[p] != 0 {
            if self.toks[p] != CS_TOKEN_FLAG + also || self.toks[p + 1] >> 8 != LEFT_BRACE || self.toks[p + 1] >= CS_TOKEN_FLAG {
                return false;
            }
            let open = p + 2;
            let mut depth = 1;
            let mut q = open;
            loop {
                let x = self.toks[q];
                if x == 0 {
                    return false;
                }
                if x < CS_TOKEN_FLAG {
                    match x >> 8 {
                        LEFT_BRACE => depth += 1,
                        RIGHT_BRACE => depth -= 1,
                        MAC_PARAM | OUT_PARAM => return false,
                        _ => {}
                    }
                }
                if depth == 0 {
                    break;
                }
                q += 1;
            }
            if first {
                first = false;
                if self.native_macro_hash_to(body, self.equiv(body), q + 1) != stock {
                    return false;
                }
            } else {
                self.native_preset_text(every);
                self.native_preset_text(b"/.append style={");
                for i in open..q {
                    let x = self.toks[i];
                    self.host.hv.native.preset.push(x);
                }
                self.native_preset_text(b"},");
            }
            p = q + 1;
        }
        !first
    }

    /// Text of a key list, as tokens the library reads back as that text.
    fn native_preset_text(&mut self, text: &[u8]) {
        let hv = &mut self.host.hv.native;
        for &c in text {
            hv.preset.push(match c {
                b'{' => LEFT_BRACE_TOKEN + c as i32,
                b'}' => RIGHT_BRACE_TOKEN + c as i32,
                b' ' => SPACE_TOKEN,
                _ if c.is_ascii_alphabetic() => LETTER_TOKEN + c as i32,
                _ => OTHER_TOKEN + c as i32,
            });
        }
    }

    /// A tip of tikz-cd's is another: `arrow style=`, if all of that style's are there.  The style the first such entry is of goes in a bit, and `native_styles_finish` looks at the rest.
    fn native_style_entry(&mut self, k: usize) -> bool {
        let (name, _) = native_table::GUARD[k];
        let cs = self.find_cs(name);
        if cs == UNDEFINED_CONTROL_SEQUENCE {
            return false;
        }
        let Some(h) = self.native_meaning_hash(cs) else { return false };
        for (bit, (_, entries)) in native_table::STYLES.iter().enumerate() {
            if entries.iter().any(|e| e.0 == name && e.1 == h) {
                self.host.hv.native.styles_seen |= 1 << bit;
                return true;
            }
        }
        false
    }

    fn native_styles_finish(&mut self) -> Result<(), &'static str> {
        const BAD: &str = "tikz-cd's tips are set to no known arrow style";
        let seen = self.host.hv.native.styles_seen;
        if seen == 0 {
            return Ok(());
        }
        let Some((bit, (style, entries))) = native_table::STYLES.iter().enumerate().find(|(b, _)| seen == 1 << *b) else { return Err(BAD) };
        let _ = bit;
        for &(name, h) in entries.iter() {
            let cs = self.find_cs(name);
            if cs == UNDEFINED_CONTROL_SEQUENCE || self.native_meaning_hash(cs) != Some(h) {
                return Err(BAD);
            }
        }
        self.native_preset_text(b"arrow style=");
        self.native_preset_text(style);
        self.native_preset_text(b",");
        Ok(())
    }

    /// A key the library does not know may be a style of the document's: `\\tikzcdset{mine/.style={hook}}`, or of `\\tikzset`, which comes second in real tikz-cd's search and may not use tikz-cd's own keys.
    /// True if the library now has it.
    fn native_user_style(&mut self, key: &[u8]) -> bool {
        if key.is_empty() || key.contains(&b'/') {
            return false;
        }
        for (base, own) in [(&b"pgfk@/tikz/commutative diagrams/"[..], true), (b"pgfk@/tikz/", false)] {
            let name: Vec<u8> = base.iter().chain(key).copied().collect();
            let body_name: Vec<u8> = name.iter().chain(b"/.@body").copied().collect();
            let cmd_name: Vec<u8> = name.iter().chain(b"/.@cmd").copied().collect();
            let (body, cmd) = (self.find_cs(&body_name), self.find_cs(&cmd_name));
            if body != UNDEFINED_CONTROL_SEQUENCE && cmd != UNDEFINED_CONTROL_SEQUENCE {
                return self.native_style_items(key, body, cmd, own);
            }
            if cmd != UNDEFINED_CONTROL_SEQUENCE {
                return false;
            }
        }
        false
    }

    fn native_style_items(&mut self, key: &[u8], body: i32, cmd: i32, own: bool) -> bool {
        if self.eq_type(body) != CALL || self.eq_type(cmd) != LONG_CALL {
            return false;
        }
        let also = self.find_cs(b"pgfkeysalso");
        let start = self.equiv(body) + 1;
        let mut q = self.equiv(cmd) + 1;
        while self.toks[q] != END_MATCH_TOKEN {
            if self.toks[q] == 0 {
                return false;
            }
            q += 1;
        }
        let mut r = start;
        while self.toks[r] != 0 && self.toks[r] == self.toks[q] {
            r += 1;
            q += 1;
        }
        if self.toks[r] != 0 || self.toks[q] != 0 || self.toks[start] != END_MATCH_TOKEN {
            return false;
        }
        // Each of its groups, in the order they came.
        let mut p = start + 1;
        let mut items = Vec::new();
        while self.toks[p] != 0 {
            if self.toks[p] != CS_TOKEN_FLAG + also || self.toks[p + 1] >= CS_TOKEN_FLAG || self.toks[p + 1] >> 8 != LEFT_BRACE {
                return false;
            }
            let mut depth = 1;
            let mut i = p + 2;
            while depth > 0 {
                let x = self.toks[i];
                if x == 0 {
                    return false;
                }
                if x < CS_TOKEN_FLAG {
                    match x >> 8 {
                        LEFT_BRACE => depth += 1,
                        RIGHT_BRACE => depth -= 1,
                        MAC_PARAM | OUT_PARAM => return false,
                        _ => {}
                    }
                }
                i += 1;
            }
            items.push((p + 2, i - 1));
            p = i;
        }
        // In `/tikz` real PGF knows no key of tikz-cd's, and says so.
        if !own {
            for &(a, b) in &items {
                let mut i = a;
                let mut word = Vec::new();
                let mut depth = 0;
                while i <= b {
                    let x = if i == b { OTHER_TOKEN + b',' as i32 } else { self.toks[i] };
                    i += 1;
                    if x >= CS_TOKEN_FLAG {
                        continue;
                    }
                    match x >> 8 {
                        LEFT_BRACE => depth += 1,
                        RIGHT_BRACE => depth -= 1,
                        _ if depth == 0 => {
                            let c = (x & 255) as u8;
                            if c == b',' {
                                let w = word.iter().position(|&c| c == b'=').map_or(&word[..], |e| &word[..e]);
                                let t: &[u8] = w.trim_ascii();
                                if CD_ONLY.contains(&t) {
                                    return false;
                                }
                                word.clear();
                            } else {
                                word.push(c);
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
        for (k, &(a, b)) in items.iter().enumerate() {
            self.native_preset_text(key);
            self.native_preset_text(if k == 0 { b"/.style={" } else { b"/.append style={" });
            for i in a..b {
                let x = self.toks[i];
                self.host.hv.native.preset.push(x);
            }
            self.native_preset_text(b"},");
        }
        !items.is_empty()
    }

    /// The list of `Vars::guards`, with what is as installed marked so.
    pub fn native_guards_build(&mut self) {
        let m = self.tb_mark();
        for &(name, h) in native_table::GUARD.iter() {
            let cs = self.find_cs(name);
            let (mut t, mut e) = (-1, -1);
            if cs != UNDEFINED_CONTROL_SEQUENCE {
                let w = self.eqtb[cs];
                let ty = w.b0() & !ARMED_BITS;
                if self.native_meaning_hash(cs) == Some(h) {
                    if ty != IF_TEST {
                        self.add_token_ref(w.rh());
                    }
                    (t, e) = (ty, w.rh());
                }
            }
            // No meaning is a kind that no word has.
            self.tb_push(if cs == UNDEFINED_CONTROL_SEQUENCE { 1 } else { cs + 1 });
            self.tb_push(if t < 0 { 0xffff } else { t + 1 });
            self.tb_push(e.max(0) + 1);
        }
        self.v_image.cd.guards = self.tb_commit(m);
    }

    /// An entry of `GUARD` has another meaning than last time.  Perhaps the same tokens.
    fn native_guard_again(&mut self, k: usize, p: i32) -> bool {
        let (name, h) = native_table::GUARD[k];
        let cs = self.find_cs(name);
        if cs == UNDEFINED_CONTROL_SEQUENCE {
            return false;
        }
        let w = self.eqtb[cs];
        let ty = w.b0() & !ARMED_BITS;
        if self.native_meaning_hash(cs) != Some(h) {
            return false;
        }
        if ty != IF_TEST {
            self.add_token_ref(w.rh());
        }
        self.toks[p] = cs + 1;
        self.toks[p + 1] = ty + 1;
        self.toks[p + 2] = w.rh() + 1;
        true
    }

    /// What decides for every diagram at this place, before a token is read.
    fn native_checks(&mut self) -> Result<(), &'static str> {
        if self.pdf_output() <= 0 {
            return Err("not PDF output");
        }
        if self.pausing() > 0 {
            return Err("\\pausing");
        }
        if self.every_hbox() != NULL {
            return Err("\\everyhbox is not empty");
        }
        if self.tracing_macros() > 0
            || self.tracing_commands() > 0
            || self.tracing_assigns() > 0
            || self.tracing_restores() > 0
            || self.tracing_groups() > 0
            || self.tracing_ifs() > 0
            || self.tracing_scan_tokens() > 0
            || self.tracing_nesting() > 0
            || self.tracing_output() > 0
        {
            return Err("tracing is on");
        }
        for k in [N_BEGINGROUP, N_ENDGROUP, N_AFTERGROUP, N_UNSKIP, N_IGNORESPACES, N_RELAX, N_SELECTFONT, N_LEAVEVMODE, N_EVERYMATH, N_EXPANDAFTER, N_THE, N_SCRIPTSTYLE] {
            if self.v_image.cd.names[k] == 0 {
                return Err("not LaTeX");
            }
        }
        let p = self.v_image.cd.names[N_IFPICTURE];
        if p == 0 || self.eq_type(p) != IF_TEST || self.equiv(p) != IF_FALSE_CODE {
            return Err("inside a picture");
        }
        // Real PGF's `$\\relax$` run many times, on the picture's own list.
        if self.math_surround() != 0 {
            return Err("\\mathsurround is not zero");
        }
        // `\\normalbaselines` in each cell, as `\\pgf@matrix@startcell` has it, if it would change anything.
        let normal = self.find_cs(b"normalbaselines");
        self.host.hv.native.cell_prefix = 0;
        if !self.native_baselines_normal() {
            if normal == UNDEFINED_CONTROL_SEQUENCE {
                return Err("no \\normalbaselines");
            }
            self.host.hv.native.cell_prefix = CS_TOKEN_FLAG + normal;
        }
        if !self.native_everymath_harmless() {
            return Err("\\everymath does more than choose a style");
        }
        if self.native_ambient_color().is_none() {
            return Err("the current colour is not in a form that can be read");
        }
        let e = self.v_image.cd.endtikzcd_cs;
        if e == 0 || self.eq_type(e) < CD_ARMED {
            return Err("\\endtikzcd was changed");
        }
        Ok(())
    }

    #[inline]
    fn glue_is_set(&self, code: i32) -> bool {
        self.equiv(GLUE_BASE + code) != ZERO_GLUE
    }

    /// `\\normalbaselines`, which a cell's `\\pgf@matrix@startcell` runs and the path does not.  Equal values make it nothing.
    fn native_baselines_normal(&self) -> bool {
        let skip = |t: &Tex, name: &[u8]| -> Option<i32> {
            let cs = t.find_cs(name);
            if cs == UNDEFINED_CONTROL_SEQUENCE {
                return None;
            }
            match t.eq_type(cs) {
                ASSIGN_GLUE => Some(t.equiv(cs)),
                _ => None,
            }
        };
        let (Some(b), Some(l)) = (skip(self, b"normalbaselineskip"), skip(self, b"normallineskip")) else { return false };
        let limit = self.find_cs(b"normallineskiplimit");
        if limit == UNDEFINED_CONTROL_SEQUENCE || self.eq_type(limit) != ASSIGN_DIMEN {
            return false;
        }
        let same = |t: &Tex, a: i32, b: i32| {
            a == b || (t.width(a) == t.width(b) && t.stretch(a) == t.stretch(b) && t.shrink(a) == t.shrink(b) && t.stretch_order(a) == t.stretch_order(b) && t.shrink_order(a) == t.shrink_order(b))
        };
        same(self, self.equiv(GLUE_BASE + BASELINE_SKIP_CODE), self.equiv(b))
            && same(self, self.equiv(GLUE_BASE + LINE_SKIP_CODE), self.equiv(l))
            && self.line_skip_limit() == self.eqtb[self.equiv(limit)].sc()
    }

    /// LaTeX's `\\everymath`, which real PGF's `$\\relax$` run many times: only what cannot be counted.
    fn native_everymath_harmless(&self) -> bool {
        let cs = self.v_image.cd.names[N_EVERYMATH];
        let p = match self.eq_type(cs) {
            ASSIGN_TOKS => self.equiv(self.equiv(cs)),
            _ => return false,
        };
        if p == NULL {
            return true;
        }
        let mut q = p + 1;
        while self.toks[q] != 0 {
            let t = self.toks[q];
            q += 1;
            if t < CS_TOKEN_FLAG || !matches!(self.eq_type(t - CS_TOKEN_FLAG), MATH_STYLE | RELAX) {
                return false;
            }
        }
        true
    }

    fn native_font_params(&self) -> FontParams {
        let f = self.cur_font();
        let dim = |k: i32, f: i32| if self.font_params[f] >= k { f64::from(self.param(k, f)) / 65536.0 } else { 0.0 };
        FontParams {
            ex: dim(X_HEIGHT_CODE, f),
            em: dim(QUAD_CODE, f),
            rt: dim(8, self.fam_fnt(3 + TEXT_SIZE)),
            ax: dim(22, self.fam_fnt(2 + TEXT_SIZE)),
        }
    }

    // ------------------------------------------------------------------------------------------- reading ahead

    /// The stack's level `k`, `input_ptr` being `cur_input`.
    #[inline]
    fn native_level(&self, k: i32) -> InStateRecord {
        if k == self.input_ptr { self.cur_input } else { self.input_stack[k] }
    }

    /// The next token, or `Ok(None)` at the end of what can be read.
    fn native_next(&mut self, rd: &mut Rd) -> Result<i32, &'static str> {
        loop {
            if rd.rec.state_field == TOKEN_LIST {
                if rd.param_loc != 0 {
                    let t = self.toks[rd.param_loc];
                    if t != 0 {
                        rd.at = Found { level: rd.level, loc: rd.loc, param: rd.param, param_loc: rd.param_loc, lines: 0, is_file: false };
                        rd.param_loc += 1;
                        return Ok(t);
                    }
                    rd.param_loc = 0;
                    continue;
                }
                let t = self.toks[rd.loc];
                if t == 0 {
                    match rd.rec.index_field {
                        U_TEMPLATE | V_TEMPLATE | OUTPUT_TEXT | MARK_TEXT | WRITE_TEXT => return Err("the end of an alignment's template or the like"),
                        _ => {}
                    }
                    if rd.level == 0 {
                        return Err("no \\end{tikzcd} within reach");
                    }
                    rd.level -= 1;
                    rd.rec = self.native_level(rd.level);
                    rd.loc = rd.rec.loc_field;
                    rd.started = false;
                    continue;
                }
                rd.loc += 1;
                if t >> 8 == OUT_PARAM {
                    let p = self.param_stack[rd.rec.limit_field + (t & 255) - 1];
                    rd.param = p;
                    rd.param_loc = p + 1;
                    continue;
                }
                rd.at = Found { level: rd.level, loc: rd.loc - 1, param: 0, param_loc: 0, lines: 0, is_file: false };
                return Ok(t);
            }
            // A file.
            if !rd.started {
                rd.started = true;
                // Once more from here is the same again.
                if self.host.hv.native.dead == (self.in_open, self.line, rd.rec.loc_field) {
                    return Err("no \\end{tikzcd} within reach");
                }
                rd.scratch.clear();
                let (a, b) = (rd.rec.loc_field, rd.rec.limit_field);
                if a <= b {
                    rd.scratch.extend_from_slice(self.buffer.range(a, b + 1));
                }
                rd.i = 0;
                rd.base = a;
                rd.state = rd.rec.state_field;
            }
            let t = self.native_file_token(rd)?;
            if t != 0 {
                rd.at = Found { level: rd.level, loc: rd.base + rd.tok_i as i32, param: 0, param_loc: 0, lines: rd.lines, is_file: true };
                return Ok(t);
            }
            self.native_next_line(rd)?;
        }
    }

    /// Looks at the next line of the file of the level the reader is in, as `input_ln` and §360 would have it.
    fn native_next_line(&mut self, rd: &mut Rd) -> Result<(), &'static str> {
        const END: &str = "no \\end{tikzcd} within reach";
        if rd.rec.name_field < 20 || self.force_eof {
            return Err(END);
        }
        if self.end_line_char() != 13 || self.cat_code(13) != CAR_RET {
            return Err("\\endlinechar is not the usual one");
        }
        let f = input_file(rd.rec.index_field);
        let Slot::In(i) = &self.host.files[f as usize] else { return Err(END) };
        if rd.lines == 0 {
            rd.file_pos = i.pos;
        }
        let Some(data) = i.data.get(rd.file_pos..) else { return Err(END) };
        if data.is_empty() {
            return Err(END);
        }
        let n = host_line_end(data);
        rd.scratch.clear();
        rd.scratch.extend(data[..n].iter().map(|&c| self.xord[c as i32]));
        while rd.scratch.last() == Some(&b' ') {
            rd.scratch.pop();
        }
        rd.scratch.push(13);
        let mut next = rd.file_pos + n;
        if let Some(&term) = i.data.get(next) {
            next += 1;
            if term == b'\r' && i.data.get(next) == Some(&b'\n') {
                next += 1;
            }
        }
        rd.file_pos = next;
        rd.lines += 1;
        rd.i = 0;
        rd.base = rd.rec.start_field;
        rd.state = NEW_LINE;
        Ok(())
    }

    /// §363's tokens of a line, with no side effect.  0 at the end of the line.
    fn native_file_token(&mut self, rd: &mut Rd) -> Result<i32, &'static str> {
        const REDUCED: &str = "^^ notation";
        loop {
            let Some(&c) = rd.scratch.get(rd.i) else { return Ok(0) };
            rd.tok_i = rd.i;
            rd.i += 1;
            let cat = self.cat_code(c as i32);
            match cat {
                IGNORE => {}
                SPACER => {
                    if rd.state == MID_LINE {
                        rd.state = SKIP_BLANKS;
                        return Ok(SPACE_TOKEN);
                    }
                }
                ESCAPE => {
                    let Some(&c2) = rd.scratch.get(rd.i) else { return Err("a backslash at the end of a line") };
                    let cat2 = self.cat_code(c2 as i32);
                    let cs;
                    if cat2 == LETTER {
                        let start = rd.i;
                        let mut k = start + 1;
                        while k < rd.scratch.len() && self.cat_code(rd.scratch[k] as i32) == LETTER {
                            k += 1;
                        }
                        if k + 1 < rd.scratch.len() && self.cat_code(rd.scratch[k] as i32) == SUP_MARK && rd.scratch[k] == rd.scratch[k + 1] {
                            return Err(REDUCED);
                        }
                        cs = if k == start + 1 { SINGLE_BASE + c2 as i32 } else { self.find_cs(&rd.scratch[start..k]) };
                        if cs == UNDEFINED_CONTROL_SEQUENCE {
                            return Err(REASONS[R_UNDEFINED as usize]);
                        }
                        rd.i = k;
                        rd.state = SKIP_BLANKS;
                    } else {
                        if cat2 == SUP_MARK && rd.scratch.get(rd.i + 1) == Some(&c2) {
                            return Err(REDUCED);
                        }
                        cs = SINGLE_BASE + c2 as i32;
                        rd.i += 1;
                        rd.state = if cat2 == SPACER { SKIP_BLANKS } else { MID_LINE };
                    }
                    return Ok(CS_TOKEN_FLAG + cs);
                }
                ACTIVE_CHAR => {
                    rd.state = MID_LINE;
                    return Ok(CS_TOKEN_FLAG + ACTIVE_BASE + c as i32);
                }
                SUP_MARK => {
                    if rd.scratch.get(rd.i) == Some(&c) {
                        return Err(REDUCED);
                    }
                    rd.state = MID_LINE;
                    return Ok(cat * 256 + c as i32);
                }
                INVALID_CHAR => return Err("an invalid character"),
                COMMENT => rd.i = rd.scratch.len(),
                CAR_RET => {
                    rd.i = rd.scratch.len();
                    if rd.state == MID_LINE {
                        return Ok(SPACE_TOKEN);
                    } else if rd.state == NEW_LINE {
                        return Ok(CS_TOKEN_FLAG + self.par_loc);
                    }
                }
                _ => {
                    rd.state = MID_LINE;
                    return Ok(cat * 256 + c as i32);
                }
            }
        }
    }

    /// Reads to the matching `\end{tikzcd}` or `\endtikzcd`: tokens, lines and where it stopped.  Nothing else changes.
    fn native_read(&mut self) -> Result<(), &'static str> {
        const END: &str = "no \\end{tikzcd} within reach";
        let mut tokens = core::mem::take(&mut self.host.hv.native.tokens);
        let mut lines = core::mem::take(&mut self.host.hv.native.lines);
        let scratch = core::mem::take(&mut self.host.hv.native.scratch);
        tokens.clear();
        lines.clear();
        tokens.extend_from_slice(&self.host.hv.native.preset);
        lines.resize(tokens.len(), 0);
        self.host.hv.native.preset_in_tokens = tokens.len();
        let rec = self.cur_input;
        let mut rd = Rd { level: self.input_ptr, rec, loc: rec.loc_field, param: 0, param_loc: 0, scratch, i: 0, tok_i: 0, base: 0, state: 0, file_pos: 0, lines: 0, started: false, at: Found::default() };
        let r = self.native_scan(&mut rd, &mut tokens, &mut lines);
        // A failed read that got into a file ends the same way from there, as long as nothing has moved.
        if let Err(why) = r {
            if why == END && rd.rec.state_field != TOKEN_LIST {
                self.host.hv.native.dead = (self.in_open, self.line, rd.rec.loc_field);
            }
        }
        let hv = &mut self.host.hv.native;
        hv.tokens = tokens;
        hv.lines = lines;
        hv.scratch = core::mem::take(&mut rd.scratch);
        r
    }

    fn native_scan(&mut self, rd: &mut Rd, tokens: &mut Vec<i32>, lines: &mut Vec<i32>) -> Result<(), &'static str> {
        let begin = CS_TOKEN_FLAG + self.v_image.cd.names[N_BEGIN];
        let end = CS_TOKEN_FLAG + self.v_image.cd.names[N_END];
        let endtikzcd = CS_TOKEN_FLAG + self.v_image.cd.endtikzcd_cs;
        let tikzcd = CS_TOKEN_FLAG + self.v_image.cd.tikzcd_cs;
        let par = CS_TOKEN_FLAG + self.par_loc;
        let mut depth = 0i32;
        let mut envs: Vec<(usize, usize)> = Vec::new();
        loop {
            let t = self.native_next(rd)?;
            let at = rd.at;
            if tokens.len() >= MAX_TOKENS {
                return Err("no \\end{tikzcd} within reach");
            }
            if t == par {
                return Err("a blank line or \\par in the body");
            }
            if t == tikzcd {
                return Err("nested tikzcd");
            }
            if t == CS_TOKEN_FLAG + ACTIVE_BASE + b'&' as i32 {
                return Err("& is active before the matrix makes it so");
            }
            if t == endtikzcd {
                if depth != 0 || !envs.is_empty() {
                    return Err("\\end{tikzcd} inside a group");
                }
                self.host.hv.native.found = at;
                return Ok(());
            }
            if t == end || t == begin {
                let start = tokens.len();
                tokens.push(t);
                lines.push(self.native_line_of(rd));
                let t2 = self.native_next(rd)?;
                if t2 >= CS_TOKEN_FLAG || t2 >> 8 != LEFT_BRACE {
                    return Err("\\begin or \\end without a name");
                }
                tokens.push(t2);
                lines.push(0);
                let name_at = tokens.len();
                loop {
                    let t3 = self.native_next(rd)?;
                    if t3 >= CS_TOKEN_FLAG || (t3 >> 8 != LETTER && t3 >> 8 != OTHER_CHAR && t3 >> 8 != RIGHT_BRACE) {
                        return Err("\\begin or \\end with a computed name");
                    }
                    if t3 >> 8 == RIGHT_BRACE {
                        break;
                    }
                    tokens.push(t3);
                    lines.push(0);
                }
                let name_end = tokens.len();
                let is_tikzcd = name_end - name_at == 6 && b"tikzcd".iter().enumerate().all(|(k, &c)| tokens[name_at + k] & 255 == c as i32);
                if t == begin {
                    if is_tikzcd {
                        return Err("nested tikzcd");
                    }
                    envs.push((name_at, name_end));
                } else if let Some(e) = envs.pop() {
                    let same = e.1 - e.0 == name_end - name_at && (0..name_end - name_at).all(|k| tokens[e.0 + k] & 255 == tokens[name_at + k] & 255);
                    if !same {
                        return Err("\\end of another environment in the body");
                    }
                } else {
                    if !is_tikzcd {
                        return Err("no \\end{tikzcd} within reach");
                    }
                    if depth != 0 {
                        return Err("\\end{tikzcd} inside a group");
                    }
                    // The \end and its name stay in the input.
                    tokens.truncate(start);
                    lines.truncate(start);
                    self.host.hv.native.found = at;
                    return Ok(());
                }
                tokens.push(RIGHT_BRACE_TOKEN + b'}' as i32);
                lines.push(0);
                continue;
            }
            if t < CS_TOKEN_FLAG {
                match t >> 8 {
                    LEFT_BRACE => depth += 1,
                    RIGHT_BRACE => {
                        depth -= 1;
                        if depth < 0 {
                            return Err("an extra } in the body");
                        }
                    }
                    TAB_MARK if !at.is_file => return Err("& of a list's own: the oracle stops there"),
                    _ => {}
                }
            }
            tokens.push(t);
            lines.push(self.native_line_of(rd));
        }
    }

    /// The line a token just read from a file is on, 0 if it came from a list.
    #[inline]
    fn native_line_of(&self, rd: &Rd) -> i32 {
        if rd.rec.state_field == TOKEN_LIST { 0 } else { self.line + rd.lines }
    }

    /// The tokens as the library's bytes.  `pos[i]` is the token a byte came from.
    fn native_bytes(&mut self) -> Result<(), &'static str> {
        let tokens = core::mem::take(&mut self.host.hv.native.tokens);
        let mut bytes = core::mem::take(&mut self.host.hv.native.bytes);
        let mut pos = core::mem::take(&mut self.host.hv.native.pos);
        bytes.clear();
        pos.clear();
        let mut r = Ok(());
        let preset_tokens = self.host.hv.native.preset_in_tokens;
        self.host.hv.native.preset_bytes = 0;
        for (k, &t) in tokens.iter().enumerate() {
            let at = bytes.len();
            if k == preset_tokens {
                self.host.hv.native.preset_bytes = at;
            }
            if t >= CS_TOKEN_FLAG {
                let cs = t - CS_TOKEN_FLAG;
                bytes.push(b'\\');
                if cs >= SINGLE_BASE && cs < NULL_CS {
                    let c = (cs - SINGLE_BASE) as u8;
                    bytes.push(c);
                    if c.is_ascii_alphabetic() && next_is_letter(tokens.get(k + 1)) {
                        bytes.push(b' ');
                    }
                } else {
                    let s = if cs >= HASH_BASE { self.text(cs) } else { 0 };
                    let name = if s > 0 && s < self.str_ptr { self.str_pool.range(self.str_start[s], self.str_start[s + 1]) } else { &[] };
                    if !name.is_empty() && name.iter().all(|c| c.is_ascii_alphabetic()) && cs >= HASH_BASE {
                        bytes.extend_from_slice(name);
                    } else {
                        // A control word the library takes for something opaque.
                        bytes.extend_from_slice(b"QqQ");
                        let mut n = cs as usize;
                        loop {
                            bytes.push(b'a' + (n % 26) as u8);
                            n /= 26;
                            if n == 0 {
                                break;
                            }
                        }
                    }
                    // Only what would run into the name: the library takes `\\begin{name}` as it stands.
                    if next_is_letter(tokens.get(k + 1)) {
                        bytes.push(b' ');
                    }
                }
            } else {
                let (cmd, c) = (t >> 8, (t & 255) as u8);
                let expected = match cmd {
                    LEFT_BRACE => c == b'{',
                    RIGHT_BRACE => c == b'}',
                    MATH_SHIFT => c == b'$',
                    TAB_MARK => c == b'&',
                    MAC_PARAM => c == b'#',
                    SPACER => c == b' ',
                    SUP_MARK => c == b'^',
                    SUB_MARK => c == b'_',
                    LETTER | OTHER_CHAR => !matches!(c, b'\\' | b'{' | b'}' | b'%' | b'&' | b'#' | b'$' | b'\r' | b'\n' | b'\t'),
                    _ => false,
                };
                if !expected {
                    r = Err("a character with an unusual category");
                    break;
                }
                // The library makes one of two blanks, and offsets must stay the same.
                if c != b' ' || bytes.last() != Some(&b' ') {
                    bytes.push(c);
                }
            }
            pos.resize(bytes.len(), k as u32);
            let _ = at;
        }
        let hv = &mut self.host.hv.native;
        if preset_tokens >= tokens.len() {
            hv.preset_bytes = bytes.len();
        }
        hv.tokens = tokens;
        hv.bytes = bytes;
        hv.pos = pos;
        r
    }

    /// The token range of a span of the library's bytes.  `None` if it has no place in them, as text a style made has not.
    #[inline]
    fn native_span(&self, s: Span) -> Option<(usize, usize)> {
        let hv = &self.host.hv.native;
        if s.is_empty() {
            return Some((0, 0));
        }
        let (a, b) = (hv.from_library(s.start as usize), hv.from_library(s.end as usize - 1));
        if b >= hv.pos.len() {
            return None;
        }
        Some((hv.pos[a] as usize, hv.pos[b] as usize + 1))
    }

    /// Where the library's buffer has more than our bytes: it may set the diagram apart from the key list before it.
    fn native_offsets(&mut self, d: &Diagram) {
        let hv = &mut self.host.hv.native;
        let n = hv.preset_bytes as u32;
        hv.library_gap = d.text(Span { start: n, end: n + 1 }) == b"," && d.text(Span { start: n + 1, end: hv.bytes.len() as u32 + 1 }) == &hv.bytes[n as usize..];
    }

    // ------------------------------------------------------------------------------------------- real meanings

    /// What may not stay, in everything the library has left in cells and labels.
    fn native_vet(&mut self, d: &Diagram) -> Result<(), &'static str> {
        let hv = &mut self.host.hv.native;
        hv.stamp = hv.stamp.wrapping_add(1).max(1);
        hv.budget = WALK_BUDGET;
        for i in 0..d.cells().len() {
            let c = d.cells()[i];
            if !c.has_node {
                continue;
            }
            let f = c.font();
            self.native_vet_span(f, false, false)?;
            let runs = d.cell_runs(i);
            for k in 0..runs.len() {
                self.native_vet_span(runs[k], k + 1 == runs.len(), d.math_mode())?;
            }
        }
        for l in d.labels() {
            if let LabelFont::Command(f) = l.font {
                self.native_vet_span(f, false, false)?;
            }
            self.native_vet_span(l.text, true, d.math_mode())?;
        }
        Ok(())
    }

    fn native_vet_span(&mut self, s: Span, last: bool, math: bool) -> Result<(), &'static str> {
        let Some((a, b)) = self.native_span(s) else { return Err("the text of a cell or label is made by a style") };
        let mut envs = 0;
        let mut braces = 0;
        // How deep the argument of a macro in which `\\\\` is its own business starts, or 0.
        let mut lines_arg = 0;
        let mut k = a;
        while k < b {
            let t = self.host.hv.native.tokens[k];
            k += 1;
            if t < CS_TOKEN_FLAG {
                let c = t & 255;
                match t >> 8 {
                    OTHER_CHAR | LETTER => {
                        if self.math_code(c) == 0x8000 {
                            self.native_walk_cs(ACTIVE_BASE + c, 0).map_err(|r| REASONS[r as usize])?;
                        }
                    }
                    TAB_MARK if envs <= 0 => return Err(REASONS[R_ALIGNMENT as usize]),
                    LEFT_BRACE => braces += 1,
                    RIGHT_BRACE => {
                        if braces == lines_arg {
                            lines_arg = 0;
                        }
                        braces -= 1;
                    }
                    MATH_SHIFT if braces == 0 && math => return Err("$ in a math cell"),
                    _ => {}
                }
                continue;
            }
            let cs = t - CS_TOKEN_FLAG;
            if cs == self.v_image.cd.names[N_BEGIN] {
                // The library has seen to it that a name follows.
                let name = self.native_env_name(k);
                let (n, m) = self.native_env_css(&name);
                self.native_walk_cs(n, 0).map_err(|r| REASONS[r as usize])?;
                self.native_walk_cs(m, 0).map_err(|r| REASONS[r as usize])?;
                envs += 1;
                continue;
            }
            if cs == self.v_image.cd.names[N_END] {
                envs -= 1;
                continue;
            }
            if cs == SINGLE_BASE + b'\\' as i32 {
                if envs <= 0 && lines_arg == 0 {
                    return Err(REASONS[R_STRUCTURE as usize]);
                }
                continue;
            }
            // `\\substack{a\\b}`: these take their own `\\\\`, whatever else is in their argument.
            if k < b && lines_arg == 0 && matches!(self.native_name(cs), b"substack" | b"shortstack" | b"subarray") {
                let next = self.host.hv.native.tokens[k];
                if next < CS_TOKEN_FLAG && next >> 8 == LEFT_BRACE {
                    lines_arg = braces + 1;
                }
            }
            self.native_walk_cs(cs, 0).map_err(|r| REASONS[r as usize])?;
            if last && k == b && self.native_takes_arguments(cs) {
                return Err("a macro at the end of a cell looks for its argument beyond it");
            }
        }
        Ok(())
    }

    fn native_takes_arguments(&self, cs: i32) -> bool {
        let t = self.eq_type(cs) & !ARMED_BITS;
        if t < CALL || t >= END_TEMPLATE {
            return false;
        }
        let p = self.equiv(cs) + 1;
        let first = if self.toks[p] == PROTECTED_TOKEN { self.toks[p + 1] } else { self.toks[p] };
        first != END_MATCH_TOKEN
    }

    fn native_env_name(&self, k: usize) -> Vec<u8> {
        let tokens = &self.host.hv.native.tokens;
        let mut v = Vec::new();
        let mut i = k + 1;
        while i < tokens.len() && tokens[i] < CS_TOKEN_FLAG && tokens[i] >> 8 != RIGHT_BRACE {
            v.push((tokens[i] & 255) as u8);
            i += 1;
        }
        v
    }

    fn native_env_css(&self, name: &[u8]) -> (i32, i32) {
        let end: Vec<u8> = b"end".iter().chain(name).copied().collect();
        (self.find_cs(name), self.find_cs(&end))
    }

    /// The verdict on a control sequence's meaning, with its body and what that mentions, as far as this diagram has not looked yet.
    fn native_walk_cs(&mut self, cs: i32, depth: u32) -> Result<(), u8> {
        if cs == UNDEFINED_CONTROL_SEQUENCE {
            return Err(R_UNDEFINED);
        }
        // A picture makes the active `!` an ordinary one, and unlike `;`, `:` and `|` it stays so in a node.
        if cs == ACTIVE_BASE + b'!' as i32 {
            return Err(R_PGF);
        }
        let w = self.eqtb[cs];
        let t = w.b0() & !ARMED_BITS;
        if t >= CALL && t < END_TEMPLATE {
            if t >= OUTER_CALL {
                return Err(R_OUTER);
            }
            return self.native_walk_macro(cs, w.rh(), depth);
        }
        match self.native_meaning_of(cs, t, w.rh()) {
            0 => Ok(()),
            r => Err(r),
        }
    }

    /// The first bytes of a control sequence's name tell PGF's and TikZ's own.
    fn native_name(&self, cs: i32) -> &[u8] {
        if cs < HASH_BASE || cs > EQTB_SIZE && cs > self.eqtb_top {
            return &[];
        }
        let s = self.text(cs);
        if s <= 0 || s >= self.str_ptr {
            return &[];
        }
        self.str_pool.range(self.str_start[s], self.str_start[s + 1])
    }

    /// A meaning that is no macro.
    fn native_meaning_of(&self, cs: i32, t: i32, chr: i32) -> u8 {
        if cs >= HASH_BASE {
            let name = self.native_name(cs);
            if name.starts_with(b"pgf") || name.starts_with(b"tikz") {
                return R_PGF;
            }
            if matches!(name, b"arrow" | b"ar" | b"rar" | b"lar" | b"dar" | b"uar" | b"urar" | b"ular" | b"drar" | b"dlar") {
                return R_STRUCTURE;
            }
        }
        match t {
            UNDEFINED_CS => R_UNDEFINED,
            XRAY | STOP | INPUT | READ_TO_CS | IN_STREAM => R_PRIMITIVE,
            LEADER_SHIP if chr == A_LEADERS - 1 => R_PRIMITIVE,
            DEF_CODE if chr == CAT_CODE_BASE => R_CATCODE,
            ASSIGN_INT if chr == INT_BASE + END_LINE_CHAR_CODE => R_CATCODE,
            EXTENSION => match chr {
                OPEN_NODE | WRITE_NODE | CLOSE_NODE | SPECIAL_NODE | IMMEDIATE_CODE | PDF_LITERAL_NODE | PDF_COLORSTACK_NODE | SET_LANGUAGE_CODE | PDF_REFXIMAGE_NODE | PDF_REFXFORM_NODE
                | PDF_SAVE_NODE | PDF_RESTORE_NODE | PDF_SETMATRIX_NODE => 0,
                _ => R_POSITION,
            },
            _ => 0,
        }
    }

    fn native_walk_macro(&mut self, cs: i32, ptr: i32, depth: u32) -> Result<(), u8> {
        if depth > WALK_DEPTH {
            return Err(R_BUDGET);
        }
        let hv = &mut self.host.hv.native;
        if hv.budget == 0 {
            return Err(R_BUDGET);
        }
        hv.budget -= 1;
        // Control sequences made at run time are numbered in order, so this stays small next to `eqtb_top`.
        if hv.memo.len() <= cs as usize {
            hv.memo.resize(cs as usize + 4096, Memo::default());
        }
        let stamp = hv.stamp;
        let m = hv.memo[cs as usize];
        if m.ptr == ptr && m.stamp == stamp {
            return if m.verdict == 0 { Ok(()) } else { Err(m.verdict) };
        }
        let m = if m.ptr == ptr { m } else { self.native_analyse(cs, ptr) };
        let hv = &mut self.host.hv.native;
        if m.local != 0 {
            hv.culprit = cs;
        }
        // Cycles: a macro under walk counts as clean; its other mentions decide.
        let mut entry = m;
        entry.stamp = stamp;
        entry.verdict = m.local;
        hv.memo[cs as usize] = entry;
        if m.local != 0 {
            return Err(m.local);
        }
        let (a, b) = m.deps;
        for k in a..b {
            let dep = self.host.hv.native.deps[k as usize];
            if let Err(r) = self.native_walk_cs(dep, depth + 1) {
                self.host.hv.native.memo[cs as usize].verdict = r;
                if self.v_image.cd.report_names {
                    self.host.hv.native.chain.push(cs);
                }
                return Err(r);
            }
        }
        Ok(())
    }

    /// A macro's body: `Memo::local` and what it mentions.  A body the table knows is clean, and what it mentions is its own business.
    fn native_analyse(&mut self, cs: i32, ptr: i32) -> Memo {
        if native_table::is_safe(self.native_macro_hash(cs, ptr)) {
            return Memo { ptr, ..Memo::default() };
        }
        self.native_analyse_body(cs, ptr)
    }

    /// For the table's generator: a meaning that no macro holding it can be called safe with.
    #[cfg(feature = "commute-probe")]
    pub fn native_severe(&self, cs: i32, t: i32, chr: i32) -> bool {
        matches!(self.native_meaning_of(cs, t, chr), R_CATCODE | R_PRIMITIVE | R_POSITION)
    }

    fn native_analyse_body(&mut self, cs: i32, ptr: i32) -> Memo {
        let mut m = Memo { ptr, ..Memo::default() };
        let name = self.native_name(cs);
        if name.starts_with(b"pgf") || name.starts_with(b"tikz") {
            m.local = R_PGF;
            return m;
        }
        if matches!(name, b"arrow" | b"ar" | b"rar" | b"lar" | b"dar" | b"uar" | b"urar" | b"ular" | b"drar" | b"dlar") {
            m.local = R_STRUCTURE;
            return m;
        }
        let start = self.host.hv.native.deps.len() as u32;
        let begin = self.v_image.cd.names[N_BEGIN];
        let mut aligned = false;
        // The parameter text only delimits.
        let mut p = ptr + 1;
        while self.toks[p] != END_MATCH_TOKEN && self.toks[p] != 0 {
            p += 1;
        }
        let backslash = SINGLE_BASE + b'\\' as i32;
        let mut prev = 0;
        // How many of the next tokens are only named, not used: the two of `\\ifx`, what `\\let` and `\\def` assign to.
        let mut named = 0;
        loop {
            let t = self.toks[p];
            p += 1;
            if t == 0 {
                break;
            }
            if named > 0 && t != END_MATCH_TOKEN {
                named -= 1;
                prev = t;
                continue;
            }
            if t < CS_TOKEN_FLAG {
                if t >> 8 == TAB_MARK && !aligned {
                    m.local = R_ALIGNMENT;
                    break;
                }
                // A number by a character's code: the control sequence after it is only that character.
                if t == OTHER_TOKEN + b'`' as i32 {
                    named = 1;
                }
                prev = t;
                continue;
            }
            let d = t - CS_TOKEN_FLAG;
            let w = self.eqtb[d];
            let ty = w.b0() & !ARMED_BITS;
            if ty == CS_NAME && w.rh() == 0 {
                // \csname with a name that can be read here is one more mention, and any other is beyond proof.
                let mut q = p;
                let mut name = Vec::new();
                let mut fixed = true;
                while self.toks[q] != 0 {
                    let x = self.toks[q];
                    if x >= CS_TOKEN_FLAG {
                        if self.eqtb[x - CS_TOKEN_FLAG].b0() & !ARMED_BITS == END_CS_NAME {
                            break;
                        }
                        fixed = false;
                        break;
                    }
                    if x >> 8 == OUT_PARAM {
                        fixed = false;
                        break;
                    }
                    name.push((x & 255) as u8);
                    q += 1;
                }
                if !fixed {
                    m.local = R_CSNAME;
                    break;
                }
                let found = self.find_cs(&name);
                if found != UNDEFINED_CONTROL_SEQUENCE {
                    self.host.hv.native.deps.push(found);
                }
                p = q;
                continue;
            }
            if d == begin {
                // `\begin{matrix}`, with a name that can be read here.  Any other is beyond proof.
                let mut name = Vec::new();
                let mut q = p;
                if self.toks[q] >> 8 != LEFT_BRACE || self.toks[q] >= CS_TOKEN_FLAG {
                    m.local = R_ENV;
                    break;
                }
                q += 1;
                let mut fixed = true;
                while self.toks[q] != 0 && self.toks[q] >> 8 != RIGHT_BRACE {
                    let x = self.toks[q];
                    if x >= CS_TOKEN_FLAG || x >> 8 == OUT_PARAM {
                        fixed = false;
                        break;
                    }
                    name.push((x & 255) as u8);
                    q += 1;
                }
                if !fixed || name == b"tikzcd" {
                    m.local = R_ENV;
                    break;
                }
                let (n, e) = self.native_env_css(&name);
                if n == UNDEFINED_CONTROL_SEQUENCE || e == UNDEFINED_CONTROL_SEQUENCE {
                    m.local = R_ENV;
                    break;
                }
                self.host.hv.native.deps.push(n);
                self.host.hv.native.deps.push(e);
                aligned = true;
                p = q;
                continue;
            }
            if d == backslash {
                let assigned = prev >= CS_TOKEN_FLAG && matches!(self.eqtb[prev - CS_TOKEN_FLAG].b0() & !ARMED_BITS, LET | DEF);
                if !assigned && !aligned {
                    m.local = R_STRUCTURE;
                    break;
                }
                prev = t;
                continue;
            }
            match ty {
                HALIGN | VALIGN => aligned = true,
                TAB_MARK | CAR_RET | OMIT | NO_ALIGN if !aligned => {
                    m.local = R_ALIGNMENT;
                    break;
                }
                IF_TEST if w.rh() == IFX_CODE => named = 2,
                LET | DEF => named = 1,
                _ => {}
            }
            if ty == UNDEFINED_CS && self.native_undefined_is_fine(d) {
                prev = t;
                continue;
            }
            if self.native_is_expl3(d) {
                // expl3's own are its business, but for a verbatim argument of `\\NewDocumentCommand`, whose tokens are not to be read ahead.
                if self.native_name(d) == b"__cmd_start:nNNnnn" && self.native_group_has_v(p) {
                    m.local = R_CATCODE;
                    break;
                }
                prev = t;
                continue;
            }
            if ty >= CALL && ty < END_TEMPLATE {
                if ty >= OUTER_CALL {
                    m.local = R_OUTER;
                    break;
                }
                self.host.hv.native.deps.push(d);
            } else {
                let r = self.native_meaning_of(d, ty, w.rh());
                if r != 0 {
                    m.local = r;
                    if self.v_image.cd.report_names {
                        self.host.hv.native.chain.push(d);
                    }
                    break;
                }
            }
            prev = t;
        }
        m.deps = (start, self.host.hv.native.deps.len() as u32);
        m
    }

    /// In a macro's body, as against a cell: what has an @ in its name is defined when it is needed or is a delimiter, like graphics' `\\Gscale@x` and `\\@nil`, and these are read by `\\ifthenelse`.
    /// A misspelt name in a macro of the document has neither, and goes to real PGF for its error message.
    fn native_undefined_is_fine(&self, cs: i32) -> bool {
        // An active character without a meaning, in a body: `\\lowercase` makes others of it, as `ifthen` does.
        if cs < SINGLE_BASE {
            return true;
        }
        let name = self.native_name(cs);
        name.contains(&b'@') || matches!(name, b"equal" | b"isodd" | b"lengthtest" | b"boolean" | b"isundefined" | b"AND" | b"OR" | b"NOT")
    }

    fn native_is_expl3(&self, cs: i32) -> bool {
        let name = self.native_name(cs);
        name.contains(&b':') || name.starts_with(b"__") || name.starts_with(b"l__") || name.starts_with(b"g__") || name.starts_with(b"c__")
    }

    /// Whether the brace group at `p` holds a `v` or `+v`: the argument specification of a command of `\\NewDocumentCommand`.
    fn native_group_has_v(&self, mut p: i32) -> bool {
        if self.toks[p] >= CS_TOKEN_FLAG || self.toks[p] >> 8 != LEFT_BRACE {
            return false;
        }
        let mut depth = 1;
        p += 1;
        while depth > 0 && self.toks[p] != 0 {
            let t = self.toks[p];
            if t < CS_TOKEN_FLAG {
                match t >> 8 {
                    LEFT_BRACE => depth += 1,
                    RIGHT_BRACE => depth -= 1,
                    _ => {
                        if t & 255 == b'v' as i32 {
                            return true;
                        }
                    }
                }
            }
            p += 1;
        }
        false
    }

    /// A hash of a macro: its name, its kind and its tokens, by what they are and not by where they lie.
    pub fn native_macro_hash(&self, cs: i32, ptr: i32) -> u64 {
        self.native_macro_hash_to(cs, ptr, i32::MAX)
    }

    /// The same as if the body ended before `end`.  A style appended to is the installed style and more.
    fn native_macro_hash_to(&self, cs: i32, ptr: i32, end: i32) -> u64 {
        const PRIME: u64 = 0x0000_0100_0000_01b3;
        let mut h = 0xcbf2_9ce4_8422_2325u64;
        h = self.native_hash_name(h, cs);
        h = (h ^ (0x100 + (self.eq_type(cs) & !ARMED_BITS) as u64)).wrapping_mul(PRIME);
        let mut p = ptr + 1;
        loop {
            let t = self.toks[p];
            if t == 0 || p >= end {
                return h;
            }
            p += 1;
            if t >= CS_TOKEN_FLAG {
                h = (h ^ 0x200).wrapping_mul(PRIME);
                h = self.native_hash_name(h, t - CS_TOKEN_FLAG);
            } else {
                h = (h ^ (0x400 + t as u64)).wrapping_mul(PRIME);
            }
        }
    }

    /// What a guarded name has for a hash: a macro's as above, and for a switch such as `\\iftikz@foo` a number that says which way it stands.
    pub fn native_meaning_hash(&self, cs: i32) -> Option<u64> {
        let w = self.eqtb[cs];
        let t = w.b0() & !ARMED_BITS;
        if t >= CALL && t < END_TEMPLATE {
            Some(self.native_macro_hash(cs, w.rh()))
        } else if t == IF_TEST {
            Some(0x5357_0000 + w.rh() as u64)
        } else {
            None
        }
    }

    #[inline]
    fn native_hash_name(&self, mut h: u64, cs: i32) -> u64 {
        const PRIME: u64 = 0x0000_0100_0000_01b3;
        if cs < NULL_CS {
            return (h ^ (0x1000 + cs as u64)).wrapping_mul(PRIME);
        }
        for &c in self.native_name(cs) {
            h = (h ^ c as u64).wrapping_mul(PRIME);
        }
        h
    }

    // ------------------------------------------------------------------------------------------- committing

    /// Moves the input to the `\end`, past what the path has taken.
    fn native_commit(&mut self) {
        let f = self.host.hv.native.found;
        while self.input_ptr > f.level {
            self.end_token_list();
        }
        if f.is_file {
            for _ in 0..f.lines {
                self.line += 1;
                self.first = self.start();
                let file = self.cur_file();
                let ok = self.input_ln(file, true);
                debug_assert!(ok);
                self.firm_up_the_line();
                if self.end_line_char_inactive() {
                    self.set_limit(self.limit() - 1);
                } else {
                    self.buffer.put(self.limit(), self.end_line_char());
                }
                self.first = self.limit() + 1;
            }
            self.set_loc(f.loc);
            self.set_state(MID_LINE);
        } else if f.param != 0 {
            self.set_loc(f.loc);
            self.begin_parameter_list(f.param);
            self.set_loc(f.param_loc);
        } else {
            self.set_loc(f.loc);
        }
    }

    // ------------------------------------------------------------------------------------------- typesetting

    /// An hbox with no `to` and no `\everyhbox`, §1257 and §645 with the scanning left out.  A `}` closes it.
    fn native_open_hbox(&mut self) {
        self.set_saved(0, 0);
        self.save_ptr += 1;
        self.set_saved(0, ADDITIONAL);
        self.set_saved(1, 0);
        self.save_ptr += 2;
        self.new_save_level(HBOX_GROUP);
        self.normal_paragraph();
        self.push_nest();
        self.set_mode(-HMODE);
        self.set_space_factor(1000);
    }

    /// The parameter is `0pt` as far as this group goes.
    fn native_zero_glue(&mut self, code: i32) {
        if self.equiv(GLUE_BASE + code) != ZERO_GLUE {
            self.add_glue_ref(ZERO_GLUE);
            self.eq_define(GLUE_BASE + code, GLUE_REF, ZERO_GLUE);
        }
    }

    /// A list of `self.host.hv.native.list`'s tokens, a step at its end.
    fn native_run_list(&mut self) {
        let list = core::mem::take(&mut self.host.hv.native.list);
        let m = self.tb_mark();
        for &t in &list {
            self.tb_push(t);
        }
        self.tb_push(CS_TOKEN_FLAG + CD_STEP);
        let p = self.tb_commit(m);
        self.host.hv.native.list = list;
        // The list before has only its step behind it: its level goes first, or a diagram's levels pile up.
        if self.state() == TOKEN_LIST && self.token_type() == INSERTED && self.at_list_end() {
            self.end_token_list();
        }
        self.begin_token_list(p, INSERTED);
    }

    fn native_cs_token(&self, k: i32) -> i32 {
        CS_TOKEN_FLAG + self.v_image.cd.names[k]
    }

    /// The input is at the `\end`.  From here on the diagram is ours.
    fn native_start(&mut self) {
        self.v_image.cd.active = true;
        self.align_state += 1;
        self.host.hv.native.end_line = self.line;
        self.native_global_counts();
        self.native_open_hbox();
        // `\\pgf@picture`'s, which holds for cells and labels alike.
        self.native_zero_glue(SPACE_SKIP_CODE);
        self.native_zero_glue(XSPACE_SKIP_CODE);
        self.host.hv.native.level = self.cur_level;
        self.host.hv.native.boxes.clear();
        self.host.hv.native.next = 0;
        self.host.hv.native.stage = Stage::Fonts;
        // Each real cell selects its font; here once, as the group holds it for all of them.  The fonts of `\begingroup$\relax$` are what `rule_thickness` reads.
        let list = [
            self.native_cs_token(N_SELECTFONT),
            self.native_cs_token(N_BEGINGROUP),
            MATH_SHIFT_TOKEN + b'$' as i32,
            self.native_cs_token(N_RELAX),
            MATH_SHIFT_TOKEN + b'$' as i32,
        ];
        self.host.hv.native.list.clear();
        self.host.hv.native.list.extend_from_slice(&list);
        self.native_run_list();
        self.cur_cmd = RELAX;
        self.cur_chr = 256;
    }

    /// What a real diagram leaves behind in the registers: DESIGN.md section 10 says how that was established.
    fn native_global_counts(&mut self) {
        self.native_global_add(N_SERIAL, 1);
        self.native_global_add(N_FIG_COUNT, 1);
        // `\\global\\let\\tikzcdmatrixname\\tikzlastnode`: `tikz@f@N`.
        let cs = self.find_cs(b"tikzcdmatrixname");
        let Some(n) = self.native_count_read(N_FIG_COUNT) else { return };
        if cs == UNDEFINED_CONTROL_SEQUENCE {
            return;
        }
        let m = self.tb_mark();
        self.tb_push(END_MATCH_TOKEN);
        for c in b"tikz@f@" {
            self.tb_push(LETTER_TOKEN + *c as i32);
        }
        for c in n.to_string().bytes() {
            self.tb_push(OTHER_TOKEN + c as i32);
        }
        let p = self.tb_commit(m);
        self.geq_define(cs, CALL, p);
    }

    /// The matrix's counters as the last row leaves them.
    fn native_matrix_counts(&mut self, d: &Diagram) {
        let rows = d.rows() as i32;
        let last = d.cells().iter().filter(|c| c.row as i32 == rows - 1).count() as i32;
        self.native_global_set(N_MATRIX_ROW, rows);
        self.native_global_set(N_MATRIX_COL, last);
        self.native_global_set(N_MATRIX_NCOLS, d.cols() as i32);
    }

    fn native_count_read(&mut self, k: i32) -> Option<i32> {
        let cs = self.v_image.cd.names[k];
        if cs == 0 {
            return None;
        }
        match self.eq_type(cs) {
            ASSIGN_INT => Some(self.eqtb[self.equiv(cs)].int()),
            REGISTER => {
                let p = self.equiv(cs);
                if p < MEM_BOT || p > LO_MEM_STAT_MAX { Some(self.sa_int(p)) } else { None }
            }
            _ => None,
        }
    }

    fn native_global_set(&mut self, k: i32, v: i32) {
        let cs = self.v_image.cd.names[k];
        match self.eq_type(cs) {
            ASSIGN_INT => {
                let p = self.equiv(cs);
                self.geq_word_define(p, v);
            }
            REGISTER => {
                let p = self.equiv(cs);
                self.gsa_w_def(p, v);
            }
            _ => {}
        }
    }

    fn native_global_add(&mut self, k: i32, by: i32) {
        if let Some(v) = self.native_count_read(k) {
            self.native_global_set(k, v + by);
        }
    }

    /// The step token was fetched.
    fn native_step(&mut self) {
        // In the wrong place only after an error in a cell, whose log is not the oracle's anyway.
        if !self.v_image.ck.at_big_switch || self.expand_depth_count != 0 || self.scanner_status != NORMAL || !self.v_image.cd.active {
            return;
        }
        let stage = self.host.hv.native.stage;
        match stage {
            Stage::Idle => {}
            Stage::Fonts => {
                self.host.hv.native.stage = Stage::FirstBox;
                let font = self.native_font_params();
                let mut d = self.host.hv.native.d.take().unwrap_or_default();
                if d.font() != font {
                    let bytes = core::mem::take(&mut self.host.hv.native.bytes);
                    let n = self.host.hv.native.preset_bytes;
                    let r = d.parse(&bytes[..n], &bytes[n..], font, &AfterVet);
                    self.host.hv.native.bytes = bytes;
                    if r.is_err() {
                        self.native_failed("the second parse differs");
                    }
                }
                self.host.hv.native.d = Some(d);
                let e = self.native_cs_token(N_ENDGROUP);
                self.host.hv.native.list.clear();
                self.host.hv.native.list.push(e);
                self.native_run_list();
            }
            Stage::FirstBox => self.native_next_box(),
            Stage::AfterBox => {
                if !self.native_take_box() {
                    self.native_failed("a cell was not left as it began");
                    return;
                }
                self.native_next_box();
            }
            Stage::Finish => self.native_finish(),
        }
    }

    /// Takes the box that has just gone onto the waiting hbox's list.
    fn native_take_box(&mut self) -> bool {
        if self.cur_level != self.host.hv.native.level || self.mode() != -HMODE {
            return false;
        }
        let b = self.tail();
        if b == self.head() || self.is_char_node(b) || self.ty(b) != HLIST_NODE {
            return false;
        }
        let mut p = self.head();
        while self.link(p) != b {
            p = self.link(p);
        }
        self.set_link(p, NULL);
        self.set_tail(p);
        self.host.hv.native.boxes.push(b);
        true
    }

    /// Opens the next cell or label, or if there is none closes the waiting hbox.
    fn native_next_box(&mut self) {
        let d = self.host.hv.native.d.take().unwrap_or_default();
        let ncells = d.cells().len();
        let math = d.math_mode();
        let mut k = self.host.hv.native.next;
        while k < ncells && !d.cells()[k].has_node {
            k += 1;
        }
        if k < ncells {
            self.host.hv.native.next = k + 1;
            let c = d.cells()[k];
            let runs = d.cell_runs(k).to_vec();
            let first = runs.first().and_then(|&s| self.native_span(s)).map(|r| r.0);
            self.line = match first {
                Some(a) if self.host.hv.native.lines.get(a).is_some_and(|&l| l != 0) => self.host.hv.native.lines[a],
                _ => self.host.hv.native.end_line,
            };
            let f = c.font();
            self.native_box_tokens(&[f], &runs, math, false, true, &d);
            // `\\pgfmatrix`'s, for a matrix in the cell: `\\everycr{}` and `\\tabskip=0pt`.
            self.native_zero_glue(TAB_SKIP_CODE);
            if self.every_cr() != NULL {
                self.eq_define(EVERY_CR_LOC, ASSIGN_TOKS, NULL);
            }
            self.host.hv.native.d = Some(d);
            return;
        }
        let l = k - ncells;
        if l < d.labels().len() {
            self.host.hv.native.next = k + 1;
            self.line = self.host.hv.native.end_line;
            let spec = d.labels()[l];
            let font = match spec.font {
                LabelFont::Script => None,
                LabelFont::Text => Some(Span::EMPTY),
                LabelFont::Command(f) => Some(f),
            };
            let runs = [spec.text];
            match font {
                Some(f) => self.native_box_tokens(&[f], &runs, math, false, false, &d),
                None => self.native_box_tokens(&[], &runs, math, true, false, &d),
            }
            self.host.hv.native.d = Some(d);
            return;
        }
        self.host.hv.native.d = Some(d);
        self.native_close_waiting();
    }

    /// `\begingroup <font> $ { \aftergroup\unskip \ignorespaces <text> } $ \endgroup }`, what `\tikz@do@fig` and `\tikz@fig@boxdone` leave of a node's text.
    fn native_box_tokens(&mut self, fonts: &[Span], runs: &[Span], math: bool, script: bool, cell: bool, d: &Diagram) {
        let replacement = d.has_ampersand_replacement();
        let mut list = core::mem::take(&mut self.host.hv.native.list);
        list.clear();
        let prefix = self.host.hv.native.cell_prefix;
        if cell && prefix != 0 {
            list.push(prefix);
        }
        list.push(self.native_cs_token(N_BEGINGROUP));
        if script {
            // `every label`'s font.
            list.push(self.native_cs_token(N_EVERYMATH));
            list.push(self.native_cs_token(N_EXPANDAFTER));
            list.push(LEFT_BRACE_TOKEN + b'{' as i32);
            list.push(self.native_cs_token(N_THE));
            list.push(self.native_cs_token(N_EVERYMATH));
            list.push(self.native_cs_token(N_SCRIPTSTYLE));
            list.push(RIGHT_BRACE_TOKEN + b'}' as i32);
        }
        for &f in fonts {
            let (a, b) = self.native_span(f).unwrap_or((0, 0));
            for k in a..b {
                list.push(self.host.hv.native.tokens[k]);
            }
        }
        if math {
            list.push(MATH_SHIFT_TOKEN + b'$' as i32);
        }
        list.push(LEFT_BRACE_TOKEN + b'{' as i32);
        list.push(self.native_cs_token(N_AFTERGROUP));
        list.push(self.native_cs_token(N_UNSKIP));
        list.push(self.native_cs_token(N_IGNORESPACES));
        let relax = self.native_cs_token(N_RELAX);
        for &s in runs {
            let (a, b) = self.native_span(s).unwrap_or((0, 0));
            for k in a..b {
                let t = self.host.hv.native.tokens[k];
                if t >> 8 == TAB_MARK && t < CS_TOKEN_FLAG && !replacement {
                    // In a matrix of a cell, `&` is active and its meaning is two tabs: spec 3.1.
                    list.push(t);
                    list.push(relax);
                }
                list.push(t);
            }
        }
        list.push(RIGHT_BRACE_TOKEN + b'}' as i32);
        if math {
            list.push(MATH_SHIFT_TOKEN + b'$' as i32);
        }
        list.push(self.native_cs_token(N_ENDGROUP));
        list.push(RIGHT_BRACE_TOKEN + b'}' as i32);
        self.host.hv.native.list = list;
        self.native_open_hbox();
        self.align_state += 1;
        self.host.hv.native.stage = Stage::AfterBox;
        self.native_run_list();
    }

    /// All boxes are there.  What is left on the waiting hbox's list goes, and then a real `\leavevmode`, as `\pgfsys@typesetpicturebox` has it.
    fn native_close_waiting(&mut self) {
        self.unsave();
        self.save_ptr -= 3;
        self.flush_node_list(self.link(self.head()));
        self.pop_nest();
        self.line = self.host.hv.native.end_line;
        self.host.hv.native.stage = Stage::Finish;
        let e = self.native_cs_token(N_LEAVEVMODE);
        self.host.hv.native.list.clear();
        self.host.hv.native.list.push(e);
        self.native_run_list();
    }

    /// An error in a cell has left things as no diagram leaves them.  What is open stays open, and real TeX has its say about that.
    fn native_failed(&mut self, why: &str) {
        self.native_report(b"[[commute failure]] ", why.as_bytes());
        self.v_image.cd.active = false;
        self.host.hv.native.stage = Stage::Idle;
    }

    /// The colour of the picture's start, `\pgfsetcolor{.}`, from xcolor's `\current@color`.
    fn native_ambient_color(&self) -> Option<Color> {
        let cs = self.v_image.cd.names[N_CURRENT_COLOR];
        if cs == 0 {
            return Some(Color::Gray(0.0));
        }
        let t = self.eq_type(cs) & !ARMED_BITS;
        if t < CALL || t >= END_TEMPLATE {
            return None;
        }
        let mut s = Vec::new();
        let mut p = self.equiv(cs) + 1;
        if self.toks[p] != END_MATCH_TOKEN {
            return None;
        }
        p += 1;
        while self.toks[p] != 0 {
            let x = self.toks[p];
            if x >= CS_TOKEN_FLAG {
                return None;
            }
            s.push((x & 255) as u8);
            p += 1;
        }
        let text = core::str::from_utf8(&s).ok()?;
        let w: Vec<&str> = text.split_ascii_whitespace().collect();
        let num = |x: &str| x.parse::<f64>().ok();
        match w.as_slice() {
            [g, "g", ..] => Some(Color::Gray(num(g)?)),
            [r, g, b, "rg", ..] => Some(Color::Rgb(num(r)?, num(g)?, num(b)?)),
            [c, m, y, k, "k", ..] => Some(Color::Cmyk(num(c)?, num(m)?, num(y)?, num(k)?)),
            _ => None,
        }
    }

    /// The real `\leavevmode` has run.  Now the geometry, and the picture's box, as `\pgfsys@typesetpicturebox` shapes it.
    fn native_finish(&mut self) {
        let mut d = self.host.hv.native.d.take().unwrap_or_default();
        let boxes = core::mem::take(&mut self.host.hv.native.boxes);
        let mut cell_dims = core::mem::take(&mut self.host.hv.native.cell_dims);
        let mut label_dims = core::mem::take(&mut self.host.hv.native.label_dims);
        cell_dims.clear();
        label_dims.clear();
        let dim = |t: &Tex, b: i32| BoxDim { w: f64::from(t.width(b)) / 65536.0, h: f64::from(t.height(b)) / 65536.0, d: f64::from(t.depth(b)) / 65536.0 };
        let mut at = 0;
        let mut boxes_of_cells = Vec::with_capacity(d.cells().len());
        for c in d.cells() {
            if c.has_node {
                cell_dims.push(dim(self, boxes[at]));
                boxes_of_cells.push(boxes[at]);
                at += 1;
            } else {
                cell_dims.push(BoxDim::default());
                boxes_of_cells.push(NULL);
            }
        }
        let boxes_of_labels: Vec<i32> = boxes[at..].to_vec();
        for &b in &boxes_of_labels {
            label_dims.push(dim(self, b));
        }
        let ambient = self.native_ambient_color().unwrap_or(Color::Gray(0.0));
        let layout = d.layout(&cell_dims, &label_dims).map(|l| *l);
        self.native_matrix_counts(&d);
        self.v_image.cd.active = false;
        self.v_image.cd.pending_end = true;
        self.host.hv.native.stage = Stage::Idle;
        self.align_state -= 1;
        match layout {
            Err(u) => {
                // Not reached by anything the empty boxes let through; the boxes are gone and an empty place is left.
                self.native_failed(u.reason);
                for &b in &boxes {
                    self.flush_node_list(b);
                }
                self.cur_box = self.new_null_box();
                self.box_end(0);
            }
            Ok(l) => {
                let mut scratch = core::mem::take(&mut self.host.hv.native.ops);
                let mut sink = Sink::new(&mut *self, &boxes_of_cells, &boxes_of_labels);
                pdf::emit(&d, ambient, &mut scratch, &mut sink);
                let first = sink.first;
                self.host.hv.native.ops = scratch;
                let inner = self.new_null_box();
                self.set_list_ptr(inner, first);
                let sp = |v: f64| (v * 65536.0).round() as i32;
                self.set_shift_amount(inner, sp(l.miny));
                let k = self.new_kern(sp(-l.minx));
                self.set_link(k, inner);
                let outer = self.new_null_box();
                self.set_list_ptr(outer, k);
                self.set_width(outer, sp(l.maxx) - sp(l.minx));
                self.set_height(outer, sp(l.maxy) - sp(l.miny));
                self.set_depth(outer, 0);
                self.cur_box = outer;
                self.box_end(sp(l.base) - sp(l.miny));
            }
        }
        self.cur_cmd = RELAX;
        self.cur_chr = 256;
        let hv = &mut self.host.hv.native;
        hv.boxes = boxes;
        hv.cell_dims = cell_dims;
        hv.label_dims = label_dims;
        hv.d = Some(d);
    }
}

/// Builds the picture's list.  Each position the library asks for is an hbox of no width at that place, as real PGF's boxes are in their own way, so that pdfTeX reaches the place as it does for real PGF and writes
/// the same `cm`, with the same rounding from the same page position.  A box goes in with a kern back.
struct Sink<'a> {
    t: &'a mut Tex,
    /// The first of the hboxes, and the last, with its list's end.
    first: i32,
    seg: i32,
    tail: i32,
    cells: &'a [i32],
    labels: &'a [i32],
}

impl<'a> Sink<'a> {
    fn new(t: &'a mut Tex, cells: &'a [i32], labels: &'a [i32]) -> Sink<'a> {
        let mut s = Sink { t, first: NULL, seg: NULL, tail: NULL, cells, labels };
        s.segment(0, 0);
        s
    }

    fn segment(&mut self, x: i32, y: i32) {
        let b = self.t.new_null_box();
        self.t.set_shift_amount(b, -y);
        if self.seg == NULL {
            self.first = b;
        } else {
            self.t.set_link(self.seg, b);
        }
        self.seg = b;
        self.tail = NULL;
        if x != 0 {
            let k = self.t.new_kern(x);
            self.push(k);
        }
    }

    fn push(&mut self, n: i32) {
        if self.tail == NULL {
            self.t.set_list_ptr(self.seg, n);
        } else {
            self.t.set_link(self.tail, n);
        }
        self.tail = n;
    }
}

impl PdfSink for Sink<'_> {
    fn literal(&mut self, ops: &[u8]) {
        if ops.is_empty() {
            return;
        }
        let t = &mut *self.t;
        let m = t.tb_mark();
        for &c in ops {
            t.tb_push(if c == b' ' { SPACE_TOKEN } else { OTHER_TOKEN + c as i32 });
        }
        let p = t.tb_commit(m);
        let n = t.get_node(WRITE_NODE_SIZE);
        t.set_ty(n, WHATSIT_NODE);
        t.set_subtype(n, PDF_LITERAL_NODE);
        t.set_pdf_literal_mode(n, SET_ORIGIN);
        t.set_pdf_literal_data(n, p);
        t.set_link(n, NULL);
        self.push(n);
    }

    fn place_box(&mut self, which: BoxRef) {
        let b = match which {
            BoxRef::Cell(i) => self.cells[i as usize],
            BoxRef::Label(i) => self.labels[i as usize],
        };
        if b == NULL {
            return;
        }
        let w = self.t.width(b);
        self.t.set_shift_amount(b, 0);
        self.t.set_link(b, NULL);
        self.push(b);
        let k = self.t.new_kern(-w);
        self.push(k);
    }

    fn tex_positions(&self) -> bool {
        true
    }

    fn position(&mut self, x: i64, y: i64) {
        self.segment(x as i32, y as i32);
    }
}
