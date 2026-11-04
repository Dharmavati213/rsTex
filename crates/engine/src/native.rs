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
//! `native_guards` first: what real tikz-cd reads must be as installed, by the hashes of `native_table.rs`, or be a change the library can be told of as a key list.  Then the
//! tokens are written as bytes for the `commute` library, with `pos` from each byte to its token, and it parses them, asking `DocKeys` what the document's keys are.  What it leaves in
//! cells and labels is then judged by real meanings from eqtb, `native_vet`, those that a style made after `native_made` has read them back.  The library has taken each `\arrow`, `&` and `\\`
//! for what it looks like, so the judgement is of what could make it something else, run by run.  A layout with empty boxes shows what only depends on the structure, such as an arrow to an empty cell.
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
use commute::{AllowAll, BoxDim, BoxRef, Color, Diagram, FontParams, LabelFont, Span, UserKey, UserKeys};
use std::fmt::Write as _;

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

/// The type of the input levels of a cell's lines, but for its last.  They print as `<inserted text>`, and `end_token_list` tells `native_line_ended` of their end.
pub const CD_LINE_TEXT: i32 = WRITE_TEXT + 1;

/// The primitives that real PGF runs in a diagram, by the oracle's `\\tracingcommands=2`, less those LaTeX or a common package gives another meaning, such as `\\par` and `\\everycr`.  What it does with
/// them it does more often than the path, or where the path has nothing like it.
const PRIMITIVES: [&[u8]; 58] = [
    b"expandafter", b"ifx", b"let", b"fi", b"def", b"else", b"csname", b"endcsname", b"futurelet", b"relax", b"global", b"string", b"edef", b"the", b"ifcsname", b"iffalse", b"noexpand", b"begingroup",
    b"endgroup", b"advance", b"ignorespaces", b"ifdim", b"ifcat", b"ifnum", b"xdef", b"afterassignment", b"iftrue", b"if", b"meaning", b"detokenize", b"romannumeral", b"aftergroup", b"pdfliteral", b"setbox",
    b"divide", b"gdef", b"multiply", b"box", b"ifcase", b"or", b"unskip", b"ifodd", b"wd", b"ht", b"dp", b"long", b"hskip", b"unhbox", b"hss", b"raise", b"hbox", b"hfil", b"scriptstyle", b"vbox", b"lower",
    b"halign", b"pdfcolorstack", b"ifmmode",
];

/// What a body may hold, in tokens.  A diagram is far below this.  A read that ends here takes no longer than the reading of those tokens would take in any case.
const MAX_TOKENS: usize = 1 << 16;
/// How many macros one diagram's walk may visit, and how deep.
const WALK_BUDGET: u32 = 1 << 16;
const WALK_DEPTH: u32 = 64;
/// How many tokens one diagram's macros may look through for their arguments, so that a macro repeated before a delimiter far away does not make this quadratic.
const SCAN_BUDGET: u32 = 1 << 18;

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
const N_PROTECTED_TESTOPT: i32 = 21;
const N_TESTOPT: i32 = 22;
const N_PGF_X: i32 = 23;
const N_PGF_Y: i32 = 24;
const N_PICMINX: i32 = 25;
const N_PICMAXX: i32 = 26;
const N_PICMINY: i32 = 27;
const N_PICMAXY: i32 = 28;
const N_PATHMINX: i32 = 29;
const N_PATHMAXX: i32 = 30;
const N_PATHMINY: i32 = 31;
const N_PATHMAXY: i32 = 32;
const N_LINEWIDTH: i32 = 33;
const NAMES: [&[u8]; 34] = [
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
    b"@protected@testopt",
    b"@testopt",
    b"pgf@x",
    b"pgf@y",
    b"pgf@picminx",
    b"pgf@picmaxx",
    b"pgf@picminy",
    b"pgf@picmaxy",
    b"pgf@pathminx",
    b"pgf@pathmaxx",
    b"pgf@pathminy",
    b"pgf@pathmaxy",
    b"pgflinewidth",
];

/// Environments that are alignments of their own, in which `&` and `\\` are theirs, and those that only give `\\` a meaning of their own.  By name, and only as LaTeX and amsmath have them.
const ENV_TABS: [&[u8]; 22] = [
    b"array", b"tabular", b"tabular*", b"tabularx", b"matrix", b"pmatrix", b"bmatrix", b"Bmatrix", b"vmatrix", b"Vmatrix", b"smallmatrix", b"matrix*", b"pmatrix*", b"bmatrix*", b"Bmatrix*", b"vmatrix*", b"Vmatrix*",
    b"cases", b"dcases", b"rcases", b"aligned", b"alignedat",
];
const ENV_LINES: [&[u8]; 5] = [b"subarray", b"split", b"gathered", b"minipage", b"tabbing"];
/// Displays, which are errors in a formula, or which end it.
const ENV_DISPLAY: [&[u8]; 16] = [b"equation", b"equation*", b"displaymath", b"eqnarray", b"eqnarray*", b"align", b"align*", b"flalign", b"flalign*", b"alignat", b"alignat*", b"gather", b"gather*", b"multline", b"multline*", b"math"];

crate::pod_struct! {
    /// POD state of the path, embedded in `image.rs`'s `Vars`.  A checkpoint keeps it: `\tikzcd` is armed in the image.
    pub struct Vars {
        /// Not `--no-commute`, `RSTEX_NO_COMMUTE` or `-ini`.
        pub on: bool,
        /// `--commute-report`.
        pub report: bool,
        /// `--commute-report=names`: also the name of a macro that stood in the way.
        pub report_names: bool,
        /// A diagram is on its way through the main loop.
        pub active: bool,
        /// A diagram of this run has been typeset here.
        pub ran: bool,
        /// The next fetch of `\endtikzcd` is ours.
        pub pending_end: bool,
        pub tikzcd_cs: i32,
        pub endtikzcd_cs: i32,
        /// Control sequences looked up once, after the first `\tikzcd`: 0 if there is no such name.
        pub names: A<i32, 34>,
        pub names_done: bool,
        /// What `native_table::GUARD` found, three cells for each entry and each one up so that it is no end of a list: the list's place, the control sequence, and its `eq_type` and
        /// `equiv` as last checked.  A macro's block has a reference from here, so that the same place can only mean the same tokens.  In the image: a restored run has no need to check again.
        pub guards: i32,
        /// `session.rs`: `\commutesession`, or 0, and the number of its box.
        pub session_cs: i32,
        pub session_box: i32,
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
    /// All boxes are there, and the fonts are as the end of a real diagram leaves them.
    LastFonts,
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
    /// The body is one of `SAFE`, so `deps` is empty and `native_closure` looks at what it mentions.
    safe: bool,
    /// For `native_closure`: `all[..]`, what the body mentions, if `closure_ptr` is the body, whether `KNOWN` has it, and the diagram that has looked.
    all: (u32, u32),
    closure_ptr: i32,
    known: bool,
    /// Not known, and something of the structure in it: an index into `REASONS`.
    closure_local: u8,
    closure_stamp: u32,
}

/// What has to balance within a run of a cell or a macro's body, if what was read of the structure is to hold whichever way TeX goes: conditionals, with the braces of each branch, groups by name, `\left` and `\right`, `$`.
#[derive(Default)]
struct Balance {
    conds: [i32; 16],
    nconds: usize,
    braces: i32,
    groups: i32,
    bgroups: i32,
    delims: i32,
    dollars: bool,
}

impl Balance {
    /// A control sequence of this meaning goes by.  False if that breaks the balance.
    fn meaning(&mut self, t: i32, chr: i32) -> bool {
        match t {
            IF_TEST => {
                if self.nconds == self.conds.len() {
                    return false;
                }
                self.conds[self.nconds] = self.braces;
                self.nconds += 1;
            }
            FI_OR_ELSE => {
                if self.nconds == 0 || self.conds[self.nconds - 1] != self.braces {
                    return false;
                }
                if chr == FI_CODE {
                    self.nconds -= 1;
                }
            }
            BEGIN_GROUP => self.groups += 1,
            END_GROUP => {
                self.groups -= 1;
                return self.groups >= 0;
            }
            LEFT_BRACE => self.bgroups += 1,
            RIGHT_BRACE => {
                self.bgroups -= 1;
                return self.bgroups >= 0;
            }
            MATH_SHIFT => self.dollars = !self.dollars,
            LEFT_RIGHT if chr == LEFT_NOAD => self.delims += 1,
            LEFT_RIGHT if chr == RIGHT_NOAD => {
                self.delims -= 1;
                return self.delims >= 0;
            }
            _ => {}
        }
        true
    }

    fn closed(&self) -> bool {
        self.nconds == 0 && self.braces == 0 && self.groups == 0 && self.bgroups == 0 && self.delims == 0 && !self.dollars
    }

    /// Whether what stands here is in a group of the same run or body.
    fn in_group(&self) -> bool {
        self.braces > 0 || self.groups > 0 || self.bgroups > 0
    }
}

/// How the start of a cell's text looks to `align_peek`: `native_solid`.
#[derive(Copy, Clone, PartialEq, Eq)]
enum Solidity {
    /// Something TeX will not expand: there is a cell there, with this in it.
    Solid,
    /// Nothing is left of it.
    Vanishes,
    /// A parameter, or what cannot be told without running it.
    Unknown,
}

/// `Memo`s by control sequence.  A diagram looks at a few hundred of tens of thousands, so the table is in pages, and a short run does not pay for zeroing it.
#[derive(Default)]
struct MemoTable {
    pages: Vec<Option<Box<[Memo; 256]>>>,
}

impl MemoTable {
    #[inline]
    fn at(&mut self, cs: i32) -> &mut Memo {
        let page = cs as usize >> 8;
        if self.pages.len() <= page {
            self.pages.resize_with(page + 64, || None);
        }
        &mut self.pages[page].get_or_insert_with(|| Box::new([Memo::default(); 256]))[cs as usize & 255]
    }
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
    memo: MemoTable,
    deps: Vec<i32>,
    stamp: u32,
    budget: u32,
    /// How many tokens `native_match` may still look at for this diagram.
    scan_budget: core::cell::Cell<u32>,
    /// The macros of `SAFE` that this diagram has reached, for `native_closure`.
    safe_seen: Vec<i32>,
    /// All that `native_closure` has looked at, with where each body stood, and for what generation of that list.
    flat: Vec<(i32, i32)>,
    flat_gen: u32,
    /// The primitives by the control sequence that has their name, and what it means as a primitive.
    prims: Vec<(i32, i32, i32)>,
    stage: Stage,
    /// The index of the next box, cells then labels; the boxes so far, in `boxes`.
    next: usize,
    boxes: Vec<i32>,
    cell_dims: Vec<BoxDim>,
    label_dims: Vec<BoxDim>,
    /// `save_ptr`, `cur_level` of the waiting hbox.
    level: i32,
    end_line: i32,
    math_groups_left: i32,
    font: Option<FontParams>,
    ops: Vec<u8>,
    list: Vec<i32>,
    scratch: Vec<u8>,
    /// Which entry of `GUARD` was not as installed, for the report.
    detail: Option<usize>,
    /// The same of `GUARD_ANY`, and where each was last found to be one that `KNOWN` has.  Not a reference: a body that went and came back is looked at again.
    detail_any: Option<usize>,
    any_at: Vec<i32>,
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
    /// What `\\tikz@fig@count` was for this diagram's matrix.
    matrix_number: i32,
    /// `native_primitives_stock`: each one's place in eqtb and its meaning.
    stock: Vec<(i32, i32, i32)>,
    /// Where `native_math_stock` last found what it wants.
    math_stock: (i32, i32),
    /// `native_layers` of this diagram, and `native_color_stack`.
    /// How to start the job again, from the driver, and why this run is the second if it is.
    pub restart: Option<Box<dyn FnOnce(u64, &'static str)>>,
    pub restarted: Option<&'static str>,
    /// The lines of the pieces of a cell still to come, the next one last.
    line_queue: Vec<i32>,
    /// Where in `list` a new line starts, and which.
    breaks: Vec<(usize, i32)>,
    layers: (u32, u32),
    color_stack: i32,
    /// `every loop` is not as installed.  It matters where something uses it.
    every_loop_changed: bool,
    /// Where the library's own text starts, what a style made: with the tokens of it, by its span.
    made: Vec<(Span, (usize, usize))>,
    /// A letter of another category or the like stands in `bytes`, which only `pos` tells apart.
    bytes_inexact: bool,
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
const R_GROUP: u8 = 12;
const R_FORMULA: u8 = 13;
const R_PEEK: u8 = 14;
const R_PLACE: u8 = 15;
const R_THE: u8 = 16;
const R_LEAK: u8 = 17;
const R_TRACING: u8 = 18;
const R_CELL_START: u8 = 19;
const R_AFTER_GROUP: u8 = 20;
const REASONS: [&str; 21] = [
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
    "a macro in a cell leaves a group or a conditional open",
    "a primitive that has no place in a formula",
    "a macro in a cell looks at what comes next",
    "a value in a cell that is another in real PGF's cell",
    "\\the of what can hold anything",
    "a cell changes what the path has already read for the whole diagram",
    "a macro in a cell sets what real PGF would run differently under",
    "what starts a cell is expanded before the cell begins",
    "\\aftergroup at the top of a cell, which real PGF's parser then finds",
];

/// Whether a code of an integer parameter is one of the `\\tracing...`, those that would show the path's tokens or real PGF's.
fn native_is_tracing(code: i32) -> bool {
    matches!(
        code,
        TRACING_MACROS_CODE | TRACING_OUTPUT_CODE | TRACING_COMMANDS_CODE | TRACING_RESTORES_CODE | TRACING_ASSIGNS_CODE | TRACING_GROUPS_CODE | TRACING_IFS_CODE | TRACING_SCAN_TOKENS_CODE | TRACING_NESTING_CODE
    )
}

/// Macros of LaTeX in `SAFE`, so not looked into, that end in `\\aftergroup\\glb@settings`.  Only `\\color` has an `\\aftergroup` that real PGF expects.
fn native_aftergroup_of_latex(name: &[u8]) -> bool {
    matches!(name, b"mathversion" | b"mathversion " | b"boldmath" | b"unboldmath")
}

/// What `native_write_cs` and `native_style_text` start an opaque word with: a control sequence by its number, a character token by its value.
const OPAQUE_CS: &[u8] = b"QqQ";
const OPAQUE_CHAR: &[u8] = b"QqZ";

fn native_encode(mut n: i32, out: &mut Vec<u8>) {
    loop {
        out.push(b'a' + (n % 26) as u8);
        n /= 26;
        if n == 0 {
            break;
        }
    }
}

fn native_decode(code: &[u8]) -> Option<i32> {
    let mut n = 0i32;
    for &c in code.iter().rev() {
        if !c.is_ascii_lowercase() {
            return None;
        }
        n = n.checked_mul(26)?.checked_add((c - b'a') as i32)?;
    }
    if code.is_empty() { None } else { Some(n) }
}

/// The token a byte stands for where nothing says otherwise.
fn native_plain_char(c: u8) -> Option<i32> {
    Some(match c {
        b'{' => LEFT_BRACE_TOKEN + c as i32,
        b'}' => RIGHT_BRACE_TOKEN + c as i32,
        b'$' => MATH_SHIFT_TOKEN + c as i32,
        b'&' => TAB_TOKEN + c as i32,
        b'#' => MAC_PARAM * 256 + c as i32,
        b' ' => SPACE_TOKEN,
        b'^' => SUP_MARK * 256 + c as i32,
        b'_' => SUB_MARK * 256 + c as i32,
        b'\\' | b'%' | b'\r' | b'\n' | b'\t' => return None,
        _ if c.is_ascii_alphabetic() => LETTER_TOKEN + c as i32,
        _ => OTHER_TOKEN + c as i32,
    })
}

/// Whether a character token is the one `native_plain_char` makes of its byte.
#[inline]
fn native_char_is_plain(t: i32) -> bool {
    native_plain_char((t & 255) as u8) == Some(t)
}

/// Whether the bytes of this token start with a letter.
#[inline]
fn next_is_letter(t: Option<&i32>) -> bool {
    t.is_some_and(|&t| t < CS_TOKEN_FLAG && ((t & 255) as u8).is_ascii_alphabetic())
}

/// Scratch of LaTeX's and of amsmath's, which holds what it last held and is set before it is used.
fn native_scratch_name(name: &[u8]) -> bool {
    // The last of these is what a command of xparse's last read, and the next one sets it before it reads it.
    [&b"@temp"[..], b"@gtemp", b"reserved@", b"@x@", b"@next", b"next"].iter().any(|p| name.starts_with(p)) || name.ends_with(b"next") || name.ends_with(b"next@") || name == b"l__cmd_args_tl"
}

/// Names real tikz-cd gives a meaning of its own, whatever the document's is.
fn native_structural_name(name: &[u8]) -> bool {
    matches!(name, b"arrow" | b"ar" | b"rar" | b"lar" | b"dar" | b"uar" | b"urar" | b"ular" | b"drar" | b"dlar" | b"endtikzcd" | b"tikzcd" | b"pgfmatrixnextcell" | b"pgfmatrixendrow")
}

/// What a `\\global` in a cell makes last for the cells after it, and which real PGF sets again at each or the path has read for all.
fn native_global_param(ty: i32) -> bool {
    matches!(ty, ASSIGN_INT | ASSIGN_DIMEN | ASSIGN_GLUE | ASSIGN_MU_GLUE | DEF_FAMILY | DEF_CODE)
}

/// Integer parameters that say how much is logged or how numbers are written, whatever a cell has them for.
fn native_param_is_real_pgfs(code: i32) -> bool {
    matches!(
        code,
        TRACING_ONLINE_CODE | TRACING_MACROS_CODE | TRACING_STATS_CODE | TRACING_PARAGRAPHS_CODE | TRACING_PAGES_CODE | TRACING_OUTPUT_CODE | TRACING_LOST_CHARS_CODE | TRACING_COMMANDS_CODE | TRACING_RESTORES_CODE | TRACING_ASSIGNS_CODE | TRACING_GROUPS_CODE | TRACING_IFS_CODE
            | TRACING_SCAN_TOKENS_CODE | TRACING_NESTING_CODE | SHOW_BOX_BREADTH_CODE | SHOW_BOX_DEPTH_CODE | PAUSING_CODE | GLOBAL_DEFS_CODE
    ) || (PDF_OUTPUT_CODE..PDF_INT_PARS).contains(&code)
}

/// What may not stand directly in a cell, as the first thing that decides how what follows it is read: the library has already taken what follows for what it looks like.
fn native_direct_denied(ty: i32, chr: i32) -> Option<&'static str> {
    const NEXT: &str = "a prefix in a cell that decides how the next token is read";
    match ty {
        LET | DEF | SHORTHAND_DEF | READ_TO_CS => Some("a control sequence assigned in a cell"),
        EXPAND_AFTER | NO_EXPAND | AFTER_ASSIGNMENT => Some(NEXT),
        CONVERT if chr == STRING_CODE || chr == MEANING_CODE => Some(NEXT),
        HALIGN | VALIGN | MATH_SHIFT | MAC_PARAM => Some("an alignment or a formula begun by a name in a cell"),
        _ => None,
    }
}

/// What LaTeX or a package has for text in a box, whose last argument is no longer in the formula: there `$` starts one of its own, and anywhere else in a cell it ends the cell's.  How many arguments it has in braces, or 0.
fn native_takes_text(name: &[u8]) -> i32 {
    match name {
        b"text" | b"mbox" | b"textrm" | b"textbf" | b"textit" | b"textsf" | b"texttt" | b"textsc" | b"textnormal" | b"textup" | b"textsl" | b"textmd" | b"emph" | b"fbox" | b"framebox" | b"makebox" | b"reflectbox" => 1,
        b"parbox" | b"raisebox" | b"scalebox" | b"rotatebox" | b"colorbox" => 2,
        b"resizebox" | b"fcolorbox" => 3,
        _ => 0,
    }
}

/// What LaTeX has that is as much at home in text as in a formula.
fn native_keeps_text(name: &[u8]) -> bool {
    matches!(
        name,
        b"tiny" | b"scriptsize" | b"footnotesize" | b"small" | b"normalsize" | b"large" | b"Large" | b"LARGE" | b"huge" | b"Huge" | b"bfseries" | b"mdseries" | b"itshape" | b"slshape" | b"upshape" | b"scshape" | b"rmfamily" | b"sffamily" | b"ttfamily" | b"normalfont" | b"em" | b"color" | b"textcolor" | b"strut" | b"leavevmode" | b"hspace" | b"quad" | b"qquad" | b"enspace" | b"thinspace" | b"LaTeX" | b"TeX"
    )
}

/// The first box of a span or a body that is open: how deep it opens and how many of its arguments are to come.
#[derive(Copy, Clone)]
struct OpenBox {
    depth: i32,
    args: i32,
}

/// A token that goes on with the arguments of such a macro.
fn native_opens_argument(t: i32) -> bool {
    t < CS_TOKEN_FLAG && t >> 8 == LEFT_BRACE || t == OTHER_TOKEN + b'[' as i32
}

/// Whether what stands at this depth is out of the formula.
fn native_in_box(b: Option<OpenBox>, braces: i32) -> bool {
    b.is_some_and(|b| b.args == 1 && braces > b.depth)
}

/// A `}` has brought the braces to this.
fn native_close_box(b: &mut Option<OpenBox>, braces: i32, next_opens: bool) {
    if let Some(o) = *b {
        if braces <= o.depth {
            *b = if o.args > 1 && next_opens { Some(OpenBox { depth: o.depth, args: o.args - 1 }) } else { None };
        }
    }
}

/// `\\(`, `\\)`, `\\[` and `\\]`, which in math end the formula that holds them.
fn native_is_math_delimiter(cs: i32) -> bool {
    [b'(', b')', b'[', b']'].iter().any(|&c| cs == SINGLE_BASE + c as i32)
}

/// Past the `}` that closes a group whose `{` is before `k`, in a run that ends at `b`.
fn native_group_end(tokens: &[i32], mut k: usize, b: usize) -> Option<usize> {
    let mut depth = 1;
    while k < b {
        let t = tokens[k];
        k += 1;
        if t < CS_TOKEN_FLAG {
            match t >> 8 {
                LEFT_BRACE => depth += 1,
                RIGHT_BRACE => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(k);
                    }
                }
                _ => {}
            }
        }
    }
    None
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
        if cs == self.v_image.cd.session_cs {
            self.session_step();
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
                if self.session_library_only() {
                    let mut at = core::mem::take(&mut self.host.hv.native.at_text);
                    let culprit = core::mem::take(&mut self.host.hv.native.culprit);
                    if at.is_empty() && culprit != 0 {
                        at = [b"\\", self.native_name(culprit)].concat();
                    }
                    let why = if at.is_empty() { reason.to_string() } else { format!("{reason}: {}", String::from_utf8_lossy(&at)) };
                    self.session_declined(why);
                    self.cur_cmd = RELAX;
                    self.cur_chr = 256;
                    return;
                }
                if self.v_image.cd.report {
                    let mut text = reason.as_bytes().to_vec();
                    if let Some(k) = self.host.hv.native.detail.take() {
                        text.extend_from_slice(b": ");
                        text.extend_from_slice(native_table::GUARD[k].0);
                    }
                    if let Some(k) = self.host.hv.native.detail_any.take() {
                        text.extend_from_slice(b": ");
                        text.extend_from_slice(native_table::GUARD_ANY[k]);
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
        let offset = self.file_offset;
        if inside {
            self.wlog_cr();
        }
        self.wlog_s(a);
        self.wlog_s(b);
        if inside {
            self.wlog_s(b" %");
        }
        self.wlog_cr();
        self.file_offset = offset;
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
        if self.session_library_only() {
            // There is no real tikz-cd to be as installed.
            self.native_session_preset()?;
        } else {
            self.native_guards()?;
        }
        self.native_read()?;
        self.native_bytes()?;
        let font = self.native_font_params();
        let mut d = self.host.hv.native.d.take().unwrap_or_default();
        let r = self.native_parse(&mut d, font);
        self.host.hv.native.d = Some(d);
        r
    }

    /// `commute --preset`, once: tokens for `native_read`.
    fn native_session_preset(&mut self) -> Result<(), &'static str> {
        if !self.host.hv.native.preset.is_empty() {
            return Ok(());
        }
        let Some(mut line) = self.host.hv.session.config.as_ref().map(|c| c.preset.clone()) else { return Ok(()) };
        if line.is_empty() {
            return Ok(());
        }
        line.iter_mut().for_each(|c| {
            if *c == b'\n' || *c == b'\r' {
                *c = b' ';
            }
        });
        line.push(b',');
        line.push(13);
        let mut rd = Rd { level: 0, rec: self.cur_input, loc: 0, param: 0, param_loc: 0, scratch: line, i: 0, tok_i: 0, base: 0, state: MID_LINE, file_pos: 0, lines: 0, started: true, at: Found::default() };
        let mut tokens = Vec::new();
        loop {
            match self.native_file_token(&mut rd)? {
                0 => break,
                t if t == CS_TOKEN_FLAG + self.par_loc => {}
                t => tokens.push(t),
            }
        }
        self.host.hv.native.preset = tokens;
        Ok(())
    }

    /// `session.rs` starts another diagram.
    pub(crate) fn native_session_reset(&mut self) {
        self.host.hv.native.dead = (0, 0, 0);
    }

    /// What `native_finish` would have given `box_end`, for the last diagram.
    pub(crate) fn native_session_shift(&self) -> i32 {
        self.host.hv.native.d.as_ref().map_or(0, |d| {
            let l = d.result();
            (l.base * 65536.0).round() as i32 - (l.miny * 65536.0).round() as i32
        })
    }

    pub(crate) fn native_session_color_stack(&self) -> i32 {
        self.host.hv.native.color_stack
    }

    fn native_parse(&mut self, d: &mut Diagram, font: FontParams) -> Result<(), &'static str> {
        // What may stay in a cell is `native_vet`'s to say, with the tokens in hand, once the library has found the cells.  What is the document's it asks of `DocKeys`.
        let n = self.host.hv.native.preset_bytes;
        let r = d.parse_with(&self.host.hv.native.bytes[..n], &self.host.hv.native.bytes[n..], font, &AllowAll, &DocKeys(&*self));
        if let Err(u) = r {
            if self.v_image.cd.report_names {
                self.host.hv.native.at_text = d.text(u.at).to_vec();
            }
            return Err(u.reason);
        }
        self.native_offsets(d);
        self.native_made(d)?;
        self.native_vet(d)?;
        let left = self.host.hv.native.math_groups_left;
        if left != i32::MAX && !self.native_math_groups_enough(d, left) {
            return Err("this diagram may use up the math groups");
        }
        // What only depends on the structure shows with empty boxes, and it has to show now: after this the cells are typeset.
        let hv = &mut self.host.hv.native;
        hv.cell_dims.clear();
        hv.cell_dims.resize(d.cells().len(), BoxDim::default());
        hv.label_dims.clear();
        hv.label_dims.resize(d.labels().len(), BoxDim::default());
        d.layout(&hv.cell_dims, &hv.label_dims).map_err(|u| u.reason)?;
        // With the library only there is no log to compare and no real PGF to have the last word: it draws what real PGF draws.
        if d.has_arrow_to_itself() && !self.session_library_only() {
            return Err("real PGF warns of an arrow from a node to itself");
        }
        if d.opacities().next().is_some() && !self.session_library_only() {
            return Err("opacity needs a page resource");
        }
        // `squiggly` of tikz-cd has the error for it.
        if d.has_squiggly() && !self.session_library_only() && !self.native_exists(b"tikz@library@decorations.pathmorphing@loaded") {
            return Err("a squiggly arrow without its library");
        }
        if self.host.hv.native.every_loop_changed && d.has_loop() {
            return Err("every loop is not as installed, and this diagram has a loop");
        }
        // Asked for at each cell there is, which may be far fewer than rows times columns.
        let cells: Vec<(u32, u32)> = d.cells().iter().map(|c| (c.row, c.col)).collect();
        self.native_absent_numbered(d.rows(), d.cols(), &cells)
    }

    /// Whether a name has a meaning.  `\\csname` makes it `\\relax` when something only asks.
    fn native_exists(&self, name: &[u8]) -> bool {
        let cs = self.find_cs(name);
        cs != UNDEFINED_CONTROL_SEQUENCE && !matches!(self.eq_type(cs) & !ARMED_BITS, UNDEFINED_CS | RELAX)
    }

    /// What `.try` looks for by row and column.
    fn native_absent_numbered(&self, rows: usize, cols: usize, cells: &[(u32, u32)]) -> Result<(), &'static str> {
        let exists = |t: &Tex, name: &str| t.native_exists(name.as_bytes());
        const BAD: &str = "a style of a row or a column";
        let mut buf = String::with_capacity(64);
        for r in 1..=rows {
            buf.clear();
            let _ = write!(buf, "pgfk@/tikz/row {r}/.@cmd");
            if exists(self, &buf) {
                return Err(BAD);
            }
        }
        for &(r, c) in cells {
            buf.clear();
            let _ = write!(buf, "pgfk@/tikz/row {} column {}/.@cmd", r + 1, c + 1);
            if exists(self, &buf) {
                return Err(BAD);
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
        hv.every_loop_changed = false;
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
        if !self.native_every_scope_stock() {
            self.host.hv.native.at_text = b"every scope".to_vec();
            return Err(CHANGED);
        }
        // These differ from one preamble to another, as libraries define them.  Any of those.
        let hv = &mut self.host.hv.native;
        if hv.any_at.len() != native_table::GUARD_ANY.len() {
            hv.any_at = vec![-1; native_table::GUARD_ANY.len()];
        }
        for k in 0..native_table::GUARD_ANY.len() {
            let cs = self.find_cs(native_table::GUARD_ANY[k]);
            if cs == UNDEFINED_CONTROL_SEQUENCE || !(CALL..END_TEMPLATE).contains(&(self.eq_type(cs) & !ARMED_BITS)) {
                continue;
            }
            let ptr = self.equiv(cs);
            if self.host.hv.native.any_at[k] != ptr {
                if !native_table::is_known(self.native_macro_hash(cs, ptr)) {
                    self.host.hv.native.detail_any = Some(k);
                    return Err(CHANGED);
                }
                self.host.hv.native.any_at[k] = ptr;
            }
        }
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
        self.native_registers()
    }

    /// `every scope` is run for each arrow's scope.  Empty as TikZ has it, or with what the library `intersections` and `spath3` append: `\\pgfkeysalso` groups, the others empty.
    fn native_every_scope_stock(&self) -> bool {
        let cs = self.find_cs(b"pgfk@/tikz/every scope/.@cmd");
        if cs == UNDEFINED_CONTROL_SEQUENCE {
            return true;
        }
        if !(CALL..END_TEMPLATE).contains(&(self.eq_type(cs) & !ARMED_BITS)) {
            return false;
        }
        let also = self.find_cs(b"pgfkeysalso");
        let saved = self.find_cs(b"tikz@intersect@namedpaths@outside");
        let mut p = self.equiv(cs) + 1;
        while self.toks[p] != END_MATCH_TOKEN {
            if self.toks[p] == 0 {
                return false;
            }
            p += 1;
        }
        p += 1;
        while self.toks[p] != 0 {
            if self.toks[p] != CS_TOKEN_FLAG + also || self.toks[p + 1] >= CS_TOKEN_FLAG || self.toks[p + 1] >> 8 != LEFT_BRACE {
                return false;
            }
            p += 2;
            let open = p;
            let mut depth = 1;
            let mut library = false;
            while depth > 0 {
                let t = self.toks[p];
                if t == 0 {
                    return false;
                }
                if t < CS_TOKEN_FLAG {
                    match t >> 8 {
                        LEFT_BRACE => depth += 1,
                        RIGHT_BRACE => depth -= 1,
                        _ => {}
                    }
                } else if saved != UNDEFINED_CONTROL_SEQUENCE && t == CS_TOKEN_FLAG + saved {
                    library = true;
                } else if self.native_name(t - CS_TOKEN_FLAG).starts_with(b"l__tikzspath_") {
                    library = true;
                }
                p += 1;
            }
            if p - 1 != open && !library {
                return false;
            }
        }
        true
    }

    /// What a key of TikZ leaves in a register outside a picture, such as `shorten >` in `\\pgf@shorten@end@additional`.
    fn native_registers(&mut self) -> Result<(), &'static str> {
        for &(n, installed) in native_table::REGISTERS.iter() {
            let cs = self.find_cs(n);
            if cs == UNDEFINED_CONTROL_SEQUENCE {
                continue;
            }
            let v = match self.eq_type(cs) {
                ASSIGN_INT | ASSIGN_DIMEN => self.eqtb[self.equiv(cs)].int(),
                REGISTER => match self.equiv(cs) {
                    p if p < MEM_BOT || p > LO_MEM_STAT_MAX => self.sa_int(p),
                    _ => 0,
                },
                _ => 0,
            };
            if v != installed {
                self.host.hv.native.at_text = n.to_vec();
                return Err("a register that a key of TikZ writes is not as installed");
            }
        }
        Ok(())
    }

    /// An entry of `GUARD` is not as installed.  True if it is something `\\tikzcdset` does that the library takes as a key list before the diagram.
    fn native_guard_told(&mut self, k: usize) -> bool {
        const CD: &[u8] = b"pgfk@/tikz/commutative diagrams/";
        let name = native_table::GUARD[k].0;
        if name == b"pgf@ar@means@>" {
            return self.native_gt_told();
        }
        if name == b"pgfk@/tikz/every loop/.@cmd" || name == b"pgfk@/tikz/every loop/.@body" {
            self.host.hv.native.every_loop_changed = true;
            return true;
        }
        if name == b"iftikzcd@mathmode" {
            self.native_preset_text(b"math mode=false,");
            return true;
        }
        if let Some(tip) = name.strip_prefix(b"pgf@ar@means@").filter(|t| t.starts_with(b"tikzcd ")) {
            return self.native_style_entry(k, tip);
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

    /// `>=`: the library knows these by name, and declines an arrow that draws any other.
    fn native_gt_told(&mut self) -> bool {
        let cs = self.find_cs(b"pgf@ar@means@>");
        if cs == UNDEFINED_CONTROL_SEQUENCE || !(CALL..END_TEMPLATE).contains(&(self.eq_type(cs) & !ARMED_BITS)) {
            return false;
        }
        let mut text = Vec::new();
        let mut p = self.equiv(cs) + 2;
        while self.toks[p] != 0 {
            let t = self.toks[p];
            if t < CS_TOKEN_FLAG {
                text.push((t & 255) as u8);
            }
            p += 1;
        }
        let word: &[u8] = match text.as_slice() {
            // PGF's own, unless the document made the name mean another with `.tip`.
            b"{stealth}{}" if !self.native_exists(b"pgf@ar@means@stealth") => b"stealth",
            b"{latex}{}" if !self.native_exists(b"pgf@ar@means@latex") => b"latex",
            _ => b"other",
        };
        self.native_preset_text(b">=");
        self.native_preset_text(word);
        self.native_preset_text(b",");
        true
    }

    /// A tip of tikz-cd's is another: `arrow style=`, if all of that style's are there.  The style the first such entry is of goes in a bit, and `native_styles_finish` looks at the rest.
    /// Any other is the document's own, and the library declines an arrow that draws it.
    fn native_style_entry(&mut self, k: usize, tip: &[u8]) -> bool {
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
        self.native_preset_text(tip);
        self.native_preset_text(b"/.tip=x,");
        true
    }

    fn native_styles_finish(&mut self) -> Result<(), &'static str> {
        const BAD: &str = "tikz-cd's tips are set to no known arrow style";
        let seen = self.host.hv.native.styles_seen;
        if seen == 0 {
            return Ok(());
        }
        let Some((_, (style, entries))) = native_table::STYLES.iter().enumerate().find(|(b, _)| seen == 1 << *b) else { return Err(BAD) };
        self.native_preset_text(b"arrow style=");
        self.native_preset_text(style);
        self.native_preset_text(b",");
        // The document may have changed one of them since: the arrows that draw that one are not for the library.
        for &(name, h) in entries.iter() {
            let cs = self.find_cs(name);
            if cs == UNDEFINED_CONTROL_SEQUENCE {
                return Err(BAD);
            }
            if self.native_meaning_hash(cs) != Some(h) {
                self.native_preset_text(name.strip_prefix(b"pgf@ar@means@").unwrap_or(name));
                self.native_preset_text(b"/.tip=x,");
            }
        }
        Ok(())
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
                    if (CALL..END_TEMPLATE).contains(&ty) {
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
        if (CALL..END_TEMPLATE).contains(&ty) {
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
        // Real PGF has a `\\vbox` of its matrix, and all that a picture assigns would stay.
        if self.every_vbox() != NULL {
            return Err("\\everyvbox is not empty");
        }
        if self.global_defs() != 0 {
            return Err("\\globaldefs is not zero");
        }
        // Real PGF has `\\string` in its names, and its matrix is a `\\vbox`.
        if self.escape_char() != b'\\' as i32 {
            return Err("\\escapechar is not a backslash");
        }
        if self.box_max_depth() != MAX_DIMEN {
            return Err("\\boxmaxdepth has a limit");
        }
        // Real PGF loads a font's description where a letter of another category has it typeset in a cell, and writes its numbers with what the document changed.
        for c in (b'a'..=b'z').chain(b'A'..=b'Z') {
            if self.cat_code(c as i32) != LETTER {
                return Err("a letter of another category");
            }
        }
        if !self.native_primitives_stock() {
            return Err("a primitive of real PGF's has another meaning");
        }
        if !self.native_math_stock() {
            return Err("what each $ runs is not LaTeX's");
        }
        self.native_selectfont_stock()?;
        // Real PGF has many more of these than the path, and a document's own has them run so many times.
        for name in [&b"__hook selectfont"[..], b"__hook_toplevel selectfont", b"__hook_next selectfont", b"color"] {
            let cs = self.find_cs(name);
            if cs != UNDEFINED_CONTROL_SEQUENCE && (CALL..END_TEMPLATE).contains(&(self.eq_type(cs) & !ARMED_BITS)) && !native_table::is_known(self.native_macro_hash(cs, self.equiv(cs))) {
                let mut budget = 4096;
                if !(name.starts_with(b"__") && self.native_quiet(cs, &mut budget)) {
                    self.host.hv.native.at_text = name.to_vec();
                    return Err("a macro that real PGF runs by itself is the document's");
                }
            }
        }
        // `\\end{tikzcd}` runs it before `\\endtikzcd`, so in a real diagram it is in the last cell, before all labels.
        if !self.native_hook_empty(b"env/tikzcd/end") {
            return Err("a hook at the end of the environment");
        }
        // The order of what is written then depends on how the boxes nest, and a picture here has other boxes.
        if self.TeXXeT_en() {
            return Err("\\TeXXeTstate is not zero");
        }
        if self.native_tracing_on() {
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
        let Some(left) = self.native_math_groups_left() else { return Err("a math version has used up its math groups") };
        self.host.hv.native.math_groups_left = left;
        let Some(ambient) = self.native_ambient_color() else { return Err("the current colour is not in a form that can be read") };
        // `\\pgfsetcolor{.}` is in xcolor's target model, which `\\current@color` may have been written before.
        let same = match self.native_color_model().as_deref() {
            Some(b"") => true,
            Some(b"gray") => matches!(ambient, Color::Gray(..)),
            Some(b"rgb") => matches!(ambient, Color::Rgb(..)),
            Some(b"cmyk") => matches!(ambient, Color::Cmyk(..)),
            _ => false,
        };
        if !same {
            return Err("the current colour is not in xcolor's target model");
        }
        let e = self.v_image.cd.endtikzcd_cs;
        if e == 0 || self.eq_type(e) < CD_ARMED {
            return Err("\\endtikzcd was changed");
        }
        let Some(layers) = self.native_layers() else { return Err("the layers of \\pgfsetlayers have no main") };
        self.host.hv.native.layers = layers;
        let Some(stack) = self.native_color_stack() else { return Err("no colour stack in \\@pdfcolorstack") };
        self.host.hv.native.color_stack = stack;
        Ok(())
    }

    /// Whether it makes no difference how many times a macro runs, as far as can be seen: nothing global, nothing written, nothing shown.  `url` and `ragged2e` have such code on `\\selectfont`'s hook.
    fn native_quiet(&self, cs: i32, budget: &mut u32) -> bool {
        if *budget == 0 {
            return false;
        }
        *budget -= 1;
        let mut p = self.equiv(cs) + 1;
        while self.toks[p] != 0 {
            let t = self.toks[p];
            p += 1;
            if t < CS_TOKEN_FLAG {
                continue;
            }
            let d = t - CS_TOKEN_FLAG;
            let w = self.eqtb[d];
            let ty = w.b0() & !ARMED_BITS;
            match ty {
                PREFIX | MESSAGE | XRAY | EXTENSION | ADVANCE | MULTIPLY | DIVIDE => return false,
                CALL | LONG_CALL if d != cs && !self.native_quiet(d, budget) => return false,
                _ => {}
            }
        }
        true
    }

    /// Whether `PRIMITIVES` still mean what `primitive` made them.  Those places are compared while they stay, and looked up again when one has moved.
    fn native_primitives_stock(&mut self) -> bool {
        let same = |t: &Tex, e: &(i32, i32, i32)| {
            let w = t.eqtb[e.0];
            w.b0() & !ARMED_BITS == e.1 && w.rh() == e.2
        };
        let list = core::mem::take(&mut self.host.hv.native.stock);
        if !list.is_empty() && list.iter().all(|e| same(self, e)) {
            self.host.hv.native.stock = list;
            return true;
        }
        let mut list = list;
        list.clear();
        for name in PRIMITIVES {
            let cs = self.find_cs(name);
            if cs == UNDEFINED_CONTROL_SEQUENCE {
                return false;
            }
            let w = self.eqtb[cs];
            let p = self.prim_lookup(self.text(cs));
            if p == UNDEFINED_PRIMITIVE || self.prim_eq_type(p) != w.b0() & !ARMED_BITS || self.prim_equiv(p) != w.rh() {
                return false;
            }
            list.push((cs, w.b0() & !ARMED_BITS, w.rh()));
        }
        self.host.hv.native.stock = list;
        true
    }

    /// Whether the tokens from `p` are these, by name, `{` and `}` being themselves.  What stands behind them is for the caller, at the place that this returns.
    fn native_tokens_are(&self, mut p: i32, names: &[&[u8]]) -> Option<i32> {
        for name in names {
            let t = self.toks[p];
            let same = match *name {
                b"{" => t == LEFT_BRACE_TOKEN + b'{' as i32,
                b"}" => t == RIGHT_BRACE_TOKEN + b'}' as i32,
                _ => t >= CS_TOKEN_FLAG && self.native_name(t - CS_TOKEN_FLAG) == *name,
            };
            if !same {
                return None;
            }
            p += 1;
        }
        Some(p)
    }

    /// `\\frozen@everymath` and `\\check@mathfonts`, which real PGF's many `$\\relax$` run more often than the path, as LaTeX has them but for the versions' resets, which `native_math_groups_left` looks at.
    fn native_math_stock(&mut self) -> bool {
        let every = self.every_math();
        let check = self.find_cs(b"check@mathfonts");
        if check == UNDEFINED_CONTROL_SEQUENCE || self.eq_type(check) != CALL {
            return false;
        }
        let body = self.equiv(check);
        if self.host.hv.native.math_stock == (every, body) {
            return true;
        }
        // A package's addition may stay if it leaves no trace, such as a `\\src@spec` that says nothing.
        let names: [&[u8]; 8] = [b"aftergroup", b"@ignorefalse", b"advance", b"@math@level", b"@ne", b"check@mathfonts", b"the", b"everymath"];
        if every == NULL {
            return false;
        }
        let (mut q, mut budget) = (every + 1, 4096);
        for name in names {
            while self.toks[q] != 0 && self.native_tokens_are(q, &[name]).is_none() {
                let t = self.toks[q];
                if t < CS_TOKEN_FLAG || !self.native_no_trace(t - CS_TOKEN_FLAG, &mut budget) {
                    return false;
                }
                q += 1;
            }
            if self.toks[q] == 0 {
                return false;
            }
            q += 1;
        }
        if self.toks[q] != 0 {
            return false;
        }
        let names: [&[u8]; 16] = [
            b"ifx", b"glb@currsize", b"f@size", b"else", b"glb@settings", b"init@restore@glb@settings", b"fi", b"let", b"curr@math@size", b"f@size", b"def", b"init@restore@glb@settings", b"{", b"aftergroup",
            b"restglb@settings", b"}",
        ];
        if self.toks[body + 1] != END_MATCH_TOKEN {
            return false;
        }
        let Some(mut q) = self.native_tokens_are(body + 2, &names) else { return false };
        while self.toks[q] != 0 {
            let Some(next) = self.native_tokens_are(q, &[b"aftergroup"]) else { return false };
            let r = self.toks[next];
            if r < CS_TOKEN_FLAG || !self.native_name(r - CS_TOKEN_FLAG).starts_with(b"mv@") {
                return false;
            }
            q = next + 1;
        }
        // A reference each, as for the guards: the same place can then only mean the same tokens.
        self.add_token_ref(every);
        self.add_token_ref(body);
        self.host.hv.native.math_stock = (every, body);
        true
    }

    /// Real PGF has a `\\selectfont` for each node, and the path fewer, and the same goes for what a cell or a matrix sets up.  That only shows if it leaves a trace: what it does in a group, or again to the same
    /// effect, is no different.  A class such as scrartcl has `\\par@update` in the hook.
    fn native_selectfont_stock(&self) -> Result<(), &'static str> {
        for (name, why) in [
            (&b"selectfont"[..], "\\selectfont is not LaTeX's"),
            (b"selectfont ", "\\selectfont is not LaTeX's"),
            (b"normalbaselines", "\\normalbaselines is not LaTeX's"),
            (b"offinterlineskip", "\\offinterlineskip is not LaTeX's"),
        ] {
            let cs = self.find_cs(name);
            if cs != UNDEFINED_CONTROL_SEQUENCE {
                let t = self.eq_type(cs) & !ARMED_BITS;
                if !(CALL..END_TEMPLATE).contains(&t) {
                    return Err(why);
                }
                if !native_table::is_safe(self.native_macro_hash(cs, self.equiv(cs))) && !self.native_no_trace(cs, &mut 4096) {
                    return Err(why);
                }
            }
        }
        if self.native_hook_cs(b"__hook selectfont") {
            Ok(())
        } else {
            Err("a hook of \\selectfont leaves a trace")
        }
    }

    fn native_hook_cs(&self, name: &[u8]) -> bool {
        let cs = self.find_cs(name);
        cs == UNDEFINED_CONTROL_SEQUENCE || self.native_no_trace(cs, &mut 4096)
    }

    /// Whether running this, any number of times, in a group, can do nothing to the document that shows: nothing global, nothing written or said, no text.  As far as a walk through its macros can tell.
    fn native_no_trace(&self, cs: i32, budget: &mut i32) -> bool {
        let w = self.eqtb[cs];
        let t = w.b0() & !ARMED_BITS;
        if !(CALL..END_TEMPLATE).contains(&t) {
            // What cannot run here without being seen; `\\let` and `\\def` are only so with a `\\global`, which counts by itself.
            return !matches!(t, PREFIX | MESSAGE | EXTENSION | LETTER | OTHER_CHAR | CHAR_NUM | CHAR_GIVEN | LEADER_SHIP | UN_HBOX | UN_VBOX | UNDEFINED_CS | STOP | INPUT | XRAY);
        }
        let mut p = w.rh() + 1;
        while self.toks[p] != END_MATCH_TOKEN {
            if self.toks[p] == 0 {
                return false;
            }
            p += 1;
        }
        loop {
            p += 1;
            let x = self.toks[p];
            if x == 0 {
                return true;
            }
            *budget -= 1;
            if *budget < 0 {
                return false;
            }
            if x < CS_TOKEN_FLAG {
                continue;
            }
            let d = x - CS_TOKEN_FLAG;
            if d == cs {
                continue;
            }
            let y = self.eqtb[d].b0() & !ARMED_BITS;
            // `\\gdef` and `\\xdef` are 2 and 3 of `def`.
            if y == DEF && self.eqtb[d].rh() >= 2 {
                return false;
            }
            if !self.native_no_trace(d, budget) {
                return false;
            }
        }
    }

    /// Whether a hook of LaTeX has no code: `\\__hook NAME` is empty or as `\\UseHook` makes it, with nothing in those two it calls.
    fn native_hook_empty(&self, hook: &[u8]) -> bool {
        let named = |head: &[u8]| -> Vec<u8> { head.iter().chain(hook).copied().collect() };
        let cs = self.find_cs(&named(b"__hook "));
        if cs == UNDEFINED_CONTROL_SEQUENCE || matches!(self.eq_type(cs) & !ARMED_BITS, UNDEFINED_CS | RELAX) {
            return true;
        }
        if !(CALL..END_TEMPLATE).contains(&(self.eq_type(cs) & !ARMED_BITS)) {
            return false;
        }
        let p = self.equiv(cs) + 1;
        if self.toks[p] != END_MATCH_TOKEN {
            return false;
        }
        if self.toks[p + 1] == 0 {
            return true;
        }
        let (top, next) = (named(b"__hook_toplevel "), named(b"__hook_next "));
        self.toks[p + 1] == CS_TOKEN_FLAG + self.find_cs(&top)
            && self.toks[p + 2] == CS_TOKEN_FLAG + self.find_cs(&next)
            && self.toks[p + 3] == 0
            && self.native_value_is(&top, b"")
            && self.native_value_is(&next, b"")
    }

    fn native_tracing_on(&self) -> bool {
        self.tracing_macros() > 0
            || self.tracing_commands() > 0
            || self.tracing_assigns() > 0
            || self.tracing_restores() > 0
            || self.tracing_groups() > 0
            || self.tracing_ifs() > 0
            || self.tracing_scan_tokens() > 0
            || self.tracing_nesting() > 0
            || self.tracing_output() > 0
    }

    /// `\@pdfcolorstack`, pdftex.def's macro that holds `\main@pdfcolorstack`, or whatever a package has made it.
    fn native_color_stack(&self) -> Option<i32> {
        let mut cs = self.find_cs(b"@pdfcolorstack");
        if cs == UNDEFINED_CONTROL_SEQUENCE {
            return None;
        }
        if self.eq_type(cs) == CALL {
            let p = self.equiv(cs) + 1;
            if self.toks[p] != END_MATCH_TOKEN || self.toks[p + 1] < CS_TOKEN_FLAG || self.toks[p + 2] != 0 {
                return None;
            }
            cs = self.toks[p + 1] - CS_TOKEN_FLAG;
        }
        // xy has made it `\\z@`: `\\pdfcolorstack` reads a number there, and a dimension is one.
        match self.eq_type(cs) {
            CHAR_GIVEN => Some(self.equiv(cs)),
            ASSIGN_DIMEN => Some(self.eqtb[self.equiv(cs)].sc()),
            _ => self.native_count_of(cs),
        }
    }

    /// `\pgf@layerlist`: how many layers come before `main` and after it.
    fn native_layers(&self) -> Option<(u32, u32)> {
        let cs = self.find_cs(b"pgf@layerlist");
        if cs == UNDEFINED_CONTROL_SEQUENCE || self.eq_type(cs) != CALL {
            return None;
        }
        let (mut before, mut after, mut main, mut word) = (0, 0, false, 0usize);
        let mut p = self.equiv(cs) + 1;
        if self.toks[p] != END_MATCH_TOKEN {
            return None;
        }
        let mut matched = true;
        loop {
            p += 1;
            let t = self.toks[p];
            if t != 0 && (t >= CS_TOKEN_FLAG || t & 255 != b',' as i32) {
                matched &= t < CS_TOKEN_FLAG && b"main".get(word) == Some(&((t & 255) as u8));
                word += 1;
                continue;
            }
            if word == 4 && matched {
                main = true;
            } else if main {
                after += 1;
            } else {
                before += 1;
            }
            (word, matched) = (0, true);
            if t == 0 {
                break;
            }
        }
        main.then_some((before, after))
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

    /// `\\check@mathfonts` runs each math version's `\\mv@...@reset` after every formula.  They are empty until a version uses up its math groups, and from then on each writes to the log, as many
    /// times as real PGF has formulas, which is more than the path has.  How many groups are left, `i32::MAX` if there are enough for what a diagram could want: `native_math_groups_enough`.
    fn native_math_groups_left(&self) -> Option<i32> {
        const MORE_THAN_A_DIAGRAM_USES: i32 = 4;
        let is_macro = |t: &Tex, cs: i32| cs != UNDEFINED_CONTROL_SEQUENCE && (CALL..=LONG_OUTER_CALL).contains(&(t.eq_type(cs) & !ARMED_BITS));
        let cs = self.find_cs(b"check@mathfonts");
        if !is_macro(self, cs) {
            return None;
        }
        let mut q = self.equiv(cs) + 1;
        while self.toks[q] != 0 {
            let t = self.toks[q];
            q += 1;
            if t >= CS_TOKEN_FLAG {
                let name = self.native_name(t - CS_TOKEN_FLAG);
                if name.starts_with(b"mv@") && name.ends_with(b"@reset") {
                    let r = t - CS_TOKEN_FLAG;
                    if !is_macro(self, r) || self.toks[self.equiv(r) + 1] != END_MATCH_TOKEN || self.toks[self.equiv(r) + 2] != 0 {
                        return None;
                    }
                }
            }
        }
        let version = self.find_cs(b"math@version");
        if !is_macro(self, version) || self.toks[self.equiv(version) + 1] != END_MATCH_TOKEN {
            return None;
        }
        let mut name = b"c@mv@".to_vec();
        let mut q = self.equiv(version) + 2;
        while self.toks[q] != 0 {
            if self.toks[q] >= CS_TOKEN_FLAG {
                return None;
            }
            name.push(self.toks[q] as u8);
            q += 1;
        }
        let (used, top, local) = (self.find_cs(&name), self.find_cs(b"e@mathgroup@top"), self.find_cs(b"c@localmathalphabets"));
        if used == UNDEFINED_CONTROL_SEQUENCE || top == UNDEFINED_CONTROL_SEQUENCE || local == UNDEFINED_CONTROL_SEQUENCE || self.eq_type(top) != CHAR_GIVEN {
            return None;
        }
        let (used, local) = (self.native_count_of(used)?, self.native_count_of(local)?);
        let left = self.equiv(top) - local - used;
        if left > MORE_THAN_A_DIAGRAM_USES {
            return Some(i32::MAX);
        }
        // All those not yet used, which `\\select@group` shows in the version's list, may fit.
        name.drain(..2);
        let list = self.find_cs(&name);
        if !is_macro(self, list) {
            return None;
        }
        let group = self.find_cs(b"select@group");
        let mut unused = 0;
        let mut q = self.equiv(list) + 1;
        while self.toks[q] != 0 {
            if self.toks[q] == CS_TOKEN_FLAG + group {
                unused += 1;
            }
            q += 1;
        }
        Some(if unused <= left { i32::MAX } else { left })
    }

    /// Whether the math alphabets not yet used that the cells and labels reach, macros that start with `\\select@group`, fit in what is left.  As in `native_walk_macro`, a macro of `SAFE` is not looked into.
    fn native_math_groups_enough(&self, d: &Diagram, left: i32) -> bool {
        let (group, protect) = (self.find_cs(b"select@group"), self.find_cs(b"protect"));
        let tokens = &self.host.hv.native.tokens;
        let mut todo = Vec::new();
        let spans = (0..d.cells().len()).flat_map(|k| d.cell_runs(k).iter().copied()).chain(d.labels().iter().map(|l| l.text));
        for s in spans {
            if let Some((a, b)) = self.native_span(s) {
                todo.extend(tokens[a..b].iter().filter(|&&t| t >= CS_TOKEN_FLAG).map(|&t| t - CS_TOKEN_FLAG));
            }
        }
        let mut seen = std::collections::HashSet::new();
        let mut budget = WALK_BUDGET;
        let mut first_used = 0;
        while let Some(cs) = todo.pop() {
            if !seen.insert(cs) || !(CALL..=LONG_OUTER_CALL).contains(&(self.eq_type(cs) & !ARMED_BITS)) {
                continue;
            }
            let ptr = self.equiv(cs);
            let mut q = ptr + 1;
            while self.toks[q] != END_MATCH_TOKEN {
                q += 1;
            }
            q += 1;
            if self.toks[q] == CS_TOKEN_FLAG + group {
                first_used += 1;
                continue;
            }
            if native_table::is_safe(self.native_macro_hash(cs, ptr)) {
                // `\\DeclareRobustCommand`'s two levels: it is the second that starts so.
                if self.toks[q] == CS_TOKEN_FLAG + protect && self.toks[q + 1] >= CS_TOKEN_FLAG {
                    todo.push(self.toks[q + 1] - CS_TOKEN_FLAG);
                }
                continue;
            }
            while self.toks[q] != 0 {
                if budget == 0 {
                    return false;
                }
                budget -= 1;
                if self.toks[q] >= CS_TOKEN_FLAG {
                    todo.push(self.toks[q] - CS_TOKEN_FLAG);
                }
                q += 1;
            }
        }
        first_used <= left
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
                            if self.v_image.cd.report_names {
                                self.host.hv.native.at_text = rd.scratch[start - 1..k].to_vec();
                            }
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
        self.host.hv.native.bytes_inexact = false;
        for (k, &t) in tokens.iter().enumerate() {
            let at = bytes.len();
            if k == preset_tokens {
                self.host.hv.native.preset_bytes = at;
            }
            if t >= CS_TOKEN_FLAG {
                self.native_write_cs(t - CS_TOKEN_FLAG, next_is_letter(tokens.get(k + 1)), &mut bytes);
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
                if !native_char_is_plain(t) {
                    self.host.hv.native.bytes_inexact = true;
                }
                // The library makes one of two blanks, and offsets must stay the same.
                if c != b' ' || bytes.last() != Some(&b' ') {
                    bytes.push(c);
                }
            }
            pos.resize(bytes.len(), k as u32);
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

    /// The token range of a span of the library's bytes, or of what it holds of a style's text: `native_made`.
    #[inline]
    fn native_span(&self, s: Span) -> Option<(usize, usize)> {
        let hv = &self.host.hv.native;
        if s.is_empty() {
            return Some((0, 0));
        }
        let (a, b) = (hv.from_library(s.start as usize), hv.from_library(s.end as usize - 1));
        if b >= hv.pos.len() {
            return hv.made.iter().find(|m| m.0 == s).map(|m| m.1);
        }
        Some((hv.pos[a] as usize, hv.pos[b] as usize + 1))
    }

    /// Text a style made is in the library's own buffer, past what we gave it, and no token of ours lies behind it.  Each such span is read back into tokens, after the diagram's own: the bytes
    /// of `native_style_text` and `native_bytes` say which they were.
    fn native_made(&mut self, d: &Diagram) -> Result<(), &'static str> {
        self.host.hv.native.made.clear();
        let limit = self.host.hv.native.pos.len();
        let mut spans: Vec<Span> = Vec::new();
        for (k, c) in d.cells().iter().enumerate() {
            spans.push(c.font());
            spans.extend_from_slice(d.cell_runs(k));
        }
        for l in d.labels() {
            spans.push(l.text);
            if let LabelFont::Command(f) = l.font {
                spans.push(f);
            }
        }
        for s in spans {
            let hv = &self.host.hv.native;
            if s.is_empty() || hv.from_library(s.end as usize - 1) < limit || hv.made.iter().any(|m| m.0 == s) {
                continue;
            }
            if hv.bytes_inexact {
                return Err("the text of a style's value holds a character of an unusual category");
            }
            let mut tokens = core::mem::take(&mut self.host.hv.native.tokens);
            let a = tokens.len();
            let ok = self.native_read_back(d.text(s), &mut tokens);
            let b = tokens.len();
            let hv = &mut self.host.hv.native;
            hv.lines.resize(b, 0);
            hv.tokens = tokens;
            if !ok {
                return Err("the text of a style cannot be read back");
            }
            hv.made.push((s, (a, b)));
        }
        Ok(())
    }

    /// Tokens from the bytes of `native_style_text`.  False if a name is not there, or the text is not as that writes it.
    fn native_read_back(&self, text: &[u8], out: &mut Vec<i32>) -> bool {
        let mut i = 0;
        while i < text.len() {
            let c = text[i];
            i += 1;
            if c != b'\\' {
                match native_plain_char(c) {
                    Some(t) => out.push(t),
                    None => return false,
                }
                continue;
            }
            let Some(&n) = text.get(i) else { return false };
            if !n.is_ascii_alphabetic() {
                out.push(CS_TOKEN_FLAG + SINGLE_BASE + n as i32);
                i += 1;
                continue;
            }
            let start = i;
            while i < text.len() && text[i].is_ascii_alphabetic() {
                i += 1;
            }
            let name = &text[start..i];
            if let Some(code) = name.strip_prefix(OPAQUE_CS) {
                let Some(cs) = native_decode(code) else { return false };
                out.push(CS_TOKEN_FLAG + cs);
            } else if let Some(code) = name.strip_prefix(OPAQUE_CHAR) {
                let Some(t) = native_decode(code) else { return false };
                out.push(t);
            } else {
                let cs = self.find_cs(name);
                if cs == UNDEFINED_CONTROL_SEQUENCE {
                    return false;
                }
                out.push(CS_TOKEN_FLAG + cs);
            }
            // What keeps the name from running into the next word.
            if text.get(i) == Some(&b' ') {
                i += 1;
            }
        }
        true
    }

    /// A macro's replacement text as the library reads it, `#1` for the first parameter.  Bytes that `native_read_back` returns to the same tokens: a character in a category it would not
    /// guess is written as a control word the library takes for something opaque, as `native_bytes` writes a name that is more than letters.
    fn native_style_text(&self, mut p: i32, out: &mut Vec<u8>) {
        loop {
            let t = self.toks[p];
            p += 1;
            if t == 0 {
                return;
            }
            if t >= CS_TOKEN_FLAG {
                self.native_write_cs(t - CS_TOKEN_FLAG, next_is_letter(Some(&self.toks[p])), out);
            } else if t >> 8 == OUT_PARAM {
                out.push(b'#');
                out.push(b'0' + (t & 255) as u8);
            } else if t >> 8 == MAC_PARAM {
                out.extend_from_slice(b"##");
            } else if native_char_is_plain(t) {
                out.push((t & 255) as u8);
            } else {
                out.push(b'\\');
                out.extend_from_slice(OPAQUE_CHAR);
                native_encode(t, out);
                if next_is_letter(Some(&self.toks[p])) {
                    out.push(b' ');
                }
            }
        }
    }

    /// A control sequence as bytes for the library: a word if it is one, or else something it takes as a word and that says which control sequence it is.
    fn native_write_cs(&self, cs: i32, before_letter: bool, out: &mut Vec<u8>) {
        out.push(b'\\');
        if cs >= SINGLE_BASE && cs < NULL_CS {
            let c = (cs - SINGLE_BASE) as u8;
            out.push(c);
            if c.is_ascii_alphabetic() && before_letter {
                out.push(b' ');
            }
            return;
        }
        let s = if cs >= HASH_BASE { self.text(cs) } else { 0 };
        let name = if s > 0 && s < self.str_ptr { self.str_pool.range(self.str_start[s], self.str_start[s + 1]) } else { &[] };
        if !name.is_empty() && name.iter().all(|c| c.is_ascii_alphabetic()) && cs >= HASH_BASE {
            out.extend_from_slice(name);
        } else {
            out.extend_from_slice(OPAQUE_CS);
            native_encode(cs, out);
        }
        // Only what would run into the name: the library takes `\\begin{name}` as it stands.
        if before_letter {
            out.push(b' ');
        }
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
        hv.scan_budget.set(SCAN_BUDGET);
        hv.safe_seen.clear();
        self.native_vet_keys(d)?;
        // Real PGF comes to no formula, and to none of what that loads, without a node.  Without math mode it comes there when it comes there, and only with the fonts of this size in place has that nothing to say.
        if !d.cells().iter().any(|c| c.has_node) {
            return Err("a diagram with no node");
        }
        if !d.math_mode() && !self.native_math_fonts_in_place() {
            return Err("text in cells, and the math fonts of this size not yet set");
        }
        for i in 0..d.cells().len() {
            let c = d.cells()[i];
            if !c.has_node {
                continue;
            }
            let f = c.font();
            self.native_vet_span(f, false, false)?;
            let runs = d.cell_runs(i);
            if let Some((a, b)) = runs.first().and_then(|&s| self.native_span(s)).filter(|&(a, _)| !self.native_after_options(a)) {
                if let Err(why) = self.native_cell_start(a, b) {
                    let t = self.host.hv.native.tokens[a];
                    if t >= CS_TOKEN_FLAG {
                        self.host.hv.native.culprit = t - CS_TOKEN_FLAG;
                    }
                    return Err(why);
                }
            }
            // Words on both sides of an arrow, and the blanks between them and it, which a formula has no use for.
            if runs.len() > 1 && !d.math_mode() {
                return Err("text on both sides of an arrow");
            }
            for k in 0..runs.len() {
                self.native_vet_text(runs[k], d.math_mode(), false, !d.math_mode())?;
                if k + 1 < runs.len() && self.native_ends_in_cs(runs[k]) && !self.native_blank(runs[k + 1]) {
                    return Err("a macro before an arrow may look beyond it");
                }
            }
            if let Some(&first) = runs.first() {
                self.native_vet_start(first).map_err(|r| REASONS[r as usize])?;
            }
        }
        for l in d.labels() {
            if let LabelFont::Command(f) = l.font {
                self.native_vet_span(f, false, false)?;
            }
            self.native_vet_text(l.text, d.math_mode(), !d.has_ampersand_replacement(), !d.math_mode())?;
        }
        self.native_math_active().map_err(|r| REASONS[r as usize])?;
        self.native_primitives().map_err(|r| REASONS[r as usize])?;
        self.native_closure().map_err(|r| REASONS[r as usize])
    }

    /// `\\check@mathfonts` has nothing to do.
    fn native_math_fonts_in_place(&self) -> bool {
        let (a, b) = (self.find_cs(b"glb@currsize"), self.find_cs(b"f@size"));
        if a == UNDEFINED_CONTROL_SEQUENCE || b == UNDEFINED_CONTROL_SEQUENCE || !(CALL..END_TEMPLATE).contains(&self.eq_type(a)) || !(CALL..END_TEMPLATE).contains(&self.eq_type(b)) {
            return false;
        }
        let (mut p, mut q) = (self.equiv(a) + 1, self.equiv(b) + 1);
        while self.toks[p] == self.toks[q] {
            if self.toks[p] == 0 {
                return true;
            }
            p += 1;
            q += 1;
        }
        false
    }

    /// A character the library took by its byte, of another category than that byte has where nothing says otherwise: in a cell's text TeX reads it as it is, but outside it, in the keys, real PGF compares it by what it is.
    fn native_vet_keys(&mut self, d: &Diagram) -> Result<(), &'static str> {
        if !self.host.hv.native.bytes_inexact {
            return Ok(());
        }
        let from = self.host.hv.native.preset_in_tokens;
        let n = self.host.hv.native.tokens.len();
        let mut text = vec![false; n];
        let mut spans: Vec<Span> = Vec::new();
        for (k, c) in d.cells().iter().enumerate() {
            spans.push(c.font());
            spans.extend_from_slice(d.cell_runs(k));
        }
        for l in d.labels() {
            spans.push(l.text);
            if let LabelFont::Command(f) = l.font {
                spans.push(f);
            }
        }
        for s in spans {
            if let Some((a, b)) = self.native_span(s) {
                text[a.min(n)..b.min(n)].fill(true);
            }
        }
        for k in from..n {
            let t = self.host.hv.native.tokens[k];
            if t < CS_TOKEN_FLAG && !text[k] && !native_char_is_plain(t) {
                return Err("a character of an unusual category in a key");
            }
        }
        Ok(())
    }

    fn native_vet_span(&mut self, s: Span, math: bool, label: bool) -> Result<(), &'static str> {
        self.native_vet_text(s, math, label, false)
    }

    /// `text`: this is a cell or a label of a diagram with no math mode.
    fn native_vet_text(&mut self, s: Span, math: bool, label: bool, text: bool) -> Result<(), &'static str> {
        const UNBALANCED: &str = REASONS[R_GROUP as usize];
        let Some((a, b)) = self.native_span(s) else { return Err("the text of a cell or label is made by a style") };
        // Real TeX has the blank that ends this, which its `\\unskip` takes.  With another before what leaves nothing, the path's `\\unskip` would take that one.
        if text {
            let tokens = &self.host.hv.native.tokens;
            if b > a && b < tokens.len() && tokens[b] == SPACE_TOKEN && (tokens[b - 1] >= CS_TOKEN_FLAG || !matches!(tokens[b - 1] >> 8, LETTER | OTHER_CHAR | MATH_SHIFT)) {
                return Err("a blank after what leaves nothing, in text");
            }
        }
        // The environments open here, and whether each has `&` and `\\` to itself, `\\` alone, or neither.
        let mut envs = [0u8; 8];
        let mut nenvs = 0;
        let mut bal = Balance::default();
        // `\\raise` and its like have had their number, and the box must follow.
        let mut wants_box = false;
        // For `--commute-report=names`.
        let mut open_env = Vec::new();
        // How deep the argument of a macro in which `\\\\` is its own business starts, or 0.
        let mut lines_arg = 0;
        let mut boxed: Option<OpenBox> = None;
        let mut k = a;
        while k < b {
            let t = self.host.hv.native.tokens[k];
            k += 1;
            if t < CS_TOKEN_FLAG {
                match t >> 8 {
                    TAB_MARK if label || nenvs == 0 || envs[nenvs - 1] != 2 => return Err(REASONS[R_ALIGNMENT as usize]),
                    LEFT_BRACE => bal.braces += 1,
                    RIGHT_BRACE => {
                        if bal.braces == lines_arg {
                            lines_arg = 0;
                        }
                        bal.braces -= 1;
                        let next = if k < b { self.host.hv.native.tokens[k] } else { 0 };
                        native_close_box(&mut boxed, bal.braces, native_opens_argument(next));
                    }
                    MATH_SHIFT => {
                        if math && !native_in_box(boxed, bal.braces) {
                            return Err("$ in a math cell");
                        }
                        bal.dollars = !bal.dollars;
                    }
                    SUP_MARK | SUB_MARK if text && !bal.dollars && !native_in_box(boxed, bal.braces) => return Err(REASONS[R_FORMULA as usize]),
                    // What takes the next token as its own.  The run ends here, and the real next token is another.
                    SUP_MARK | SUB_MARK if k == b => return Err("a superscript or subscript at the end of a run"),
                    OTHER_CHAR if k == b && t & 255 == b'`' as i32 => return Err("a character constant at the end of a run"),
                    _ => {}
                }
                continue;
            }
            let cs = t - CS_TOKEN_FLAG;
            // These have an `\\aftergroup` of their own, and it does no harm in a group of the cell.
            if bal.in_group() && native_aftergroup_of_latex(self.native_name(cs)) {
                continue;
            }
            if cs == self.v_image.cd.names[N_BEGIN] {
                // The library has seen to it that a name follows.
                let name = self.native_env_name(k);
                if ENV_DISPLAY.contains(&name.as_slice()) {
                    return Err(REASONS[R_FORMULA as usize]);
                }
                let (n, m) = self.native_env_css(&name);
                self.native_walk_cs(n, 0).map_err(|r| REASONS[r as usize])?;
                self.native_walk_cs(m, 0).map_err(|r| REASONS[r as usize])?;
                if nenvs == envs.len() {
                    return Err(UNBALANCED);
                }
                envs[nenvs] = self.native_env_kind(&name, n);
                nenvs += 1;
                if self.v_image.cd.report_names {
                    open_env = name;
                }
                continue;
            }
            if cs == self.v_image.cd.names[N_END] {
                if nenvs == 0 {
                    return Err(UNBALANCED);
                }
                nenvs -= 1;
                continue;
            }
            if cs == SINGLE_BASE + b'\\' as i32 {
                if (nenvs == 0 || envs[nenvs - 1] == 0) && lines_arg == 0 {
                    if self.v_image.cd.report_names && nenvs > 0 {
                        self.host.hv.native.at_text = open_env.clone();
                    }
                    return Err(REASONS[R_STRUCTURE as usize]);
                }
                continue;
            }
            // `\\substack{a\\b}`: these take their own `\\\\`, whatever else is in their argument.
            if k < b && lines_arg == 0 && matches!(self.native_name(cs), b"substack" | b"shortstack") && self.native_is_stock(cs) {
                let next = self.host.hv.native.tokens[k];
                if next < CS_TOKEN_FLAG && next >> 8 == LEFT_BRACE {
                    lines_arg = bal.braces + 1;
                }
            }
            // The library says so of a cell, and does not read a label.
            if native_is_math_delimiter(cs) {
                return Err("math shift inside the diagram");
            }
            let w = self.eqtb[cs];
            let (ty, chr) = (w.b0() & !ARMED_BITS, w.rh());
            if let Some(why) = native_direct_denied(ty, chr) {
                return Err(why);
            }
            if !bal.meaning(ty, chr) {
                return Err(UNBALANCED);
            }
            if k >= a + 2 && native_global_param(ty) {
                let before = self.host.hv.native.tokens[k - 2];
                if before >= CS_TOKEN_FLAG && self.eqtb[before - CS_TOKEN_FLAG].b0() & !ARMED_BITS == PREFIX {
                    return Err(REASONS[R_LEAK as usize]);
                }
            }
            if boxed.is_none() {
                let next = if k < b { self.host.hv.native.tokens[k] } else { 0 };
                let args = if matches!(ty, MAKE_BOX | VCENTER) {
                    1
                } else if native_opens_argument(next) && self.native_is_stock(cs) {
                    native_takes_text(self.native_name(cs))
                } else {
                    0
                };
                if args > 0 {
                    boxed = Some(OpenBox { depth: bal.braces, args });
                }
            }
            // Real tikz-cd has its own `\\errmessage` around what a label does, which adds to the message.
            if ty == MESSAGE && chr == 1 {
                return Err(REASONS[R_FORMULA as usize]);
            }
            // What a macro comes to in text is not read here, and if it is a formula's that is the same error.  Those that are at home in text are enough for what such a diagram holds.
            if text && !bal.dollars && !native_in_box(boxed, bal.braces) && (CALL..END_TEMPLATE).contains(&ty) {
                let at_home = native_takes_text(self.native_name(cs)) > 0 || native_keeps_text(self.native_name(cs));
                if !at_home || !self.native_is_stock(cs) {
                    self.host.hv.native.culprit = cs;
                    return Err("a macro in a cell of text");
                }
            }
            // In text what only a formula has starts one, by an error that TeX recovers from by the groups it finds there, and those are not real PGF's.
            if text && !bal.dollars && !native_in_box(boxed, bal.braces) && matches!(ty, MATH_CHAR_NUM | MATH_GIVEN | MATH_COMP | DELIM_NUM | LEFT_RIGHT | ABOVE | RADICAL | MATH_STYLE | MATH_CHOICE | NON_SCRIPT | MKERN | LIMIT_SWITCH | MSKIP | MATH_ACCENT) {
                return Err(REASONS[R_FORMULA as usize]);
            }
            match ty {
                HMOVE | VMOVE => wants_box = true,
                MAKE_BOX | VCENTER => wants_box = false,
                AFTER_GROUP if !bal.in_group() => return Err(REASONS[R_AFTER_GROUP as usize]),
                THE if chr == 0 && (k == b || !self.native_the_takes(self.host.hv.native.tokens[k])) => return Err(REASONS[R_THE as usize]),
                CS_NAME | IF_TEST if ty == CS_NAME || chr == IF_CS_CODE => {
                    k = self.native_vet_name(k, b, ty == CS_NAME)?;
                    continue;
                }
                _ => {}
            }
            self.native_walk_cs(cs, 0).map_err(|r| REASONS[r as usize])?;
            let takes = (CALL..END_TEMPLATE).contains(&ty);
            if takes {
                if !self.native_args_fit(cs, k, b, 0) {
                    self.host.hv.native.culprit = cs;
                    return Err("a macro in a cell looks for its argument beyond the run");
                }
                // What it finds there is an `&` or an arrow, or the end of the path's box.
                if k == b {
                    let mut budget = 4096;
                    if self.native_peeks(cs, &mut budget) {
                        return Err(REASONS[R_PEEK as usize]);
                    }
                }
            } else if k == b && !self.native_complete(ty, chr, k) {
                return Err("a primitive at the end of a run looks for what follows");
            }
        }
        if !bal.closed() || nenvs != 0 {
            return Err(UNBALANCED);
        }
        if wants_box {
            return Err("a primitive at the end of a run looks for what follows");
        }
        Ok(())
    }

    /// Whether the last token of a run, spaces aside, is a control sequence.
    fn native_ends_in_cs(&self, s: Span) -> bool {
        let Some((a, b)) = self.native_span(s) else { return true };
        let tokens = &self.host.hv.native.tokens;
        tokens[a..b].iter().rev().find(|&&t| t != SPACE_TOKEN).is_some_and(|&t| t >= CS_TOKEN_FLAG)
    }

    fn native_blank(&self, s: Span) -> bool {
        let Some((a, b)) = self.native_span(s) else { return false };
        self.host.hv.native.tokens[a..b].iter().all(|&t| t == SPACE_TOKEN)
    }

    /// Whether a cell's options stand before this token: the `|` of those ended what §785 expands at a cell's start.
    fn native_after_options(&self, k: usize) -> bool {
        self.host.hv.native.tokens[..k].iter().rev().find(|&&t| t != SPACE_TOKEN).is_some_and(|&t| t < CS_TOKEN_FLAG && t & 255 == b'|' as i32)
    }

    /// `\\halign` looks for `\\omit` at the start of a cell, and expands what it finds there.  So in real PGF the first tokens of a cell are expanded before the cell has begun: not yet in math mode, with
    /// `\\nullfont` in force.  That only shows in what asks.  `|` ends that search, so what follows a cell's options is not concerned.
    fn native_vet_start(&mut self, s: Span) -> Result<(), u8> {
        let Some((a, b)) = self.native_span(s) else { return Ok(()) };
        let tokens = &self.host.hv.native.tokens;
        let Some(k) = (a..b).find(|&k| tokens[k] != SPACE_TOKEN) else { return Ok(()) };
        if self.native_after_options(k) {
            return Ok(());
        }
        let mut budget = 4096;
        for &t in &tokens[k..b] {
            if t != SPACE_TOKEN && !self.native_start_token(t, &mut budget)? {
                break;
            }
        }
        Ok(())
    }

    /// One token of those: whether the search goes on after it.
    fn native_start_token(&self, t: i32, budget: &mut i32) -> Result<bool, u8> {
        if t < CS_TOKEN_FLAG {
            return Ok(false);
        }
        let w = self.eqtb[t - CS_TOKEN_FLAG];
        let ty = w.b0() & !ARMED_BITS;
        // These come to a `\\let` or a `\\futurelet` in a document's text, whatever else they have, and there the search stops: a macro with an optional argument starts so.
        if matches!(self.native_name(t - CS_TOKEN_FLAG), b"@protected@testopt" | b"@testopt" | b"@ifnextchar" | b"kernel@ifnextchar" | b"@ifstar" | b"@ifstar " | b"@dblarg" | b"@protected@testopt ") {
            return Ok(false);
        }
        match ty {
            CALL..END_TEMPLATE if self.native_is_expl3(t - CS_TOKEN_FLAG) => Ok(false),
            CALL..END_TEMPLATE => self.native_start_body(w.rh(), budget),
            IF_TEST if matches!(w.rh() % UNLESS_CODE, IF_VMODE_CODE | IF_HMODE_CODE | IF_MMODE_CODE | IF_INNER_CODE | IF_FONT_CHAR_CODE) => Err(R_CELL_START),
            CONVERT if w.rh() == FONT_NAME_CODE || w.rh() == PDF_FONT_NAME_CODE => Err(R_CELL_START),
            // After a `\\the`: the font is not yet the cell's, and the mode and the list not.
            DEF_FONT | ASSIGN_FONT_DIMEN | ASSIGN_FONT_INT | SET_AUX | SET_PREV_GRAF | LAST_ITEM => Err(R_CELL_START),
            IF_TEST | FI_OR_ELSE | EXPAND_AFTER | THE | CONVERT | CS_NAME => Ok(true),
            _ => Ok(false),
        }
    }

    /// A macro's body as the start of a cell: to what stops the search, and as far as a condition holds it open, since either branch may be the one.
    fn native_start_body(&self, ptr: i32, budget: &mut i32) -> Result<bool, u8> {
        let mut p = ptr + 1;
        if self.toks[p] == PROTECTED_TOKEN {
            return Ok(false);
        }
        while self.toks[p] != END_MATCH_TOKEN {
            if self.toks[p] == 0 {
                return Ok(false);
            }
            p += 1;
        }
        let mut open = 0;
        // The next tokens that a condition only reads.
        let mut named = 0;
        loop {
            p += 1;
            let t = self.toks[p];
            if t == 0 {
                return Ok(true);
            }
            *budget -= 1;
            if *budget < 0 {
                return Err(R_CELL_START);
            }
            if named > 0 {
                named -= 1;
                continue;
            }
            if t >= CS_TOKEN_FLAG {
                let w = self.eqtb[t - CS_TOKEN_FLAG];
                let ty = w.b0() & !ARMED_BITS;
                if ty == IF_TEST {
                    open += 1;
                    if w.rh() % UNLESS_CODE == IFX_CODE {
                        named = 2;
                    }
                } else if ty == FI_OR_ELSE && w.rh() == FI_CODE && open > 0 {
                    open -= 1;
                    continue;
                }
                if !self.native_start_token(t, budget)? && open == 0 {
                    return Ok(false);
                }
            } else if t >> 8 == OUT_PARAM {
                // An argument may start with anything.  What a document's macro does with one is not this search's to prove, and it stops at the first unexpandable token of it.
                if open == 0 {
                    return Ok(false);
                }
            } else if open == 0 {
                return Ok(false);
            }
        }
    }

    /// `\\csname` or `\\ifcsname` in a cell, with `k` past it.  A name that can be read is one more mention, of what it comes to.  Any other can only be tested for.
    fn native_vet_name(&mut self, k: usize, b: usize, used: bool) -> Result<usize, &'static str> {
        let mut name = Vec::new();
        let mut i = k;
        while i < b {
            let x = self.host.hv.native.tokens[i];
            i += 1;
            if x >= CS_TOKEN_FLAG {
                let d = x - CS_TOKEN_FLAG;
                if self.eqtb[d].b0() & !ARMED_BITS != END_CS_NAME {
                    return Err(REASONS[R_CSNAME as usize]);
                }
                if used {
                    let found = self.find_cs(&name);
                    if found != UNDEFINED_CONTROL_SEQUENCE {
                        self.native_walk_cs(found, 0).map_err(|r| REASONS[r as usize])?;
                        let t = self.eq_type(found) & !ARMED_BITS;
                        if (CALL..END_TEMPLATE).contains(&t) && !self.native_args_fit(found, i, b, 0) {
                            return Err("a macro in a cell looks for its argument beyond the run");
                        }
                    }
                }
                return Ok(i);
            }
            name.push((x & 255) as u8);
        }
        Err(REASONS[R_CSNAME as usize])
    }

    /// `native_vet_span`'s `\\the`: what it makes of a number is a number, and of a token register whatever that holds.
    fn native_the_takes(&self, next: i32) -> bool {
        next >= CS_TOKEN_FLAG
            && matches!(
                self.eqtb[next - CS_TOKEN_FLAG].b0() & !ARMED_BITS,
                ASSIGN_INT | ASSIGN_DIMEN | ASSIGN_GLUE | ASSIGN_MU_GLUE | ASSIGN_FONT_DIMEN | ASSIGN_FONT_INT | SET_AUX | SET_PREV_GRAF | SET_PAGE_DIMEN | SET_PAGE_INT | SET_BOX_DIMEN | SET_SHAPE | DEF_CODE | DEF_FAMILY | SET_FONT | DEF_FONT | REGISTER | LAST_ITEM | CHAR_GIVEN | MATH_GIVEN
            )
    }

    /// Whether a macro that is not one of `KNOWN`, whose peeks only ever look for what a cell's end is not, comes to a `\\futurelet`.
    fn native_peeks(&self, cs: i32, budget: &mut u32) -> bool {
        let ptr = self.equiv(cs);
        if *budget == 0 {
            return true;
        }
        *budget -= 1;
        if native_table::is_known(self.native_macro_hash(cs, ptr)) {
            return false;
        }
        let mut p = ptr + 1;
        while self.toks[p] != 0 {
            let t = self.toks[p];
            p += 1;
            if t < CS_TOKEN_FLAG {
                continue;
            }
            let d = t - CS_TOKEN_FLAG;
            let w = self.eqtb[d];
            let ty = w.b0() & !ARMED_BITS;
            if ty == LET && w.rh() != NORMAL {
                return true;
            }
            if (CALL..END_TEMPLATE).contains(&ty) && d != cs && self.native_peeks(d, budget) {
                return true;
            }
        }
        false
    }

    /// Whether a primitive that ends a run wants nothing more.  A register, only after what reads it.
    fn native_complete(&self, ty: i32, chr: i32, k: usize) -> bool {
        match ty {
            RELAX | SPACER | LETTER | OTHER_CHAR | CHAR_GIVEN | MATH_GIVEN | EX_SPACE | NO_BOUNDARY | ITAL_CORR | MATH_STYLE | LIMIT_SWITCH | NON_SCRIPT | SET_FONT | ABOVE | REMOVE_ITEM | IGNORE_SPACES | START_PAR | BEGIN_GROUP | END_GROUP
            | RIGHT_BRACE | LEFT_BRACE | VRULE | FI_OR_ELSE | MATH_SHIFT | MESSAGE => true,
            HSKIP => chr < SKIP_CODE,
            DISCRETIONARY => chr == 1,
            ASSIGN_INT | ASSIGN_DIMEN | ASSIGN_GLUE | ASSIGN_MU_GLUE | REGISTER | LAST_ITEM | SET_BOX_DIMEN | ASSIGN_FONT_DIMEN | ASSIGN_FONT_INT => {
                let before = if k >= 2 { self.host.hv.native.tokens[k - 2] } else { 0 };
                before >= CS_TOKEN_FLAG && matches!(self.eqtb[before - CS_TOKEN_FLAG].b0() & !ARMED_BITS, THE | CONVERT)
            }
            _ => false,
        }
    }

    /// Whether what a macro at `k`, the token after it, takes is there before the run ends at `b`, and whatever its body ends in.  In real TeX it would take a `&`, a `\\\\`, an `\\arrow` or what follows them.
    fn native_args_fit(&self, cs: i32, k: usize, b: usize, depth: u32) -> bool {
        let t = self.eq_type(cs) & !ARMED_BITS;
        if !(CALL..END_TEMPLATE).contains(&t) {
            return true;
        }
        if depth > 16 {
            return false;
        }
        let mut p = self.equiv(cs) + 1;
        if self.toks[p] == PROTECTED_TOKEN {
            p += 1;
        }
        let Some((k, p)) = self.native_match(p, k, b) else { return false };
        // The tail of the body.
        let body = p + 1;
        let first = self.toks[body];
        let (protected_testopt, testopt) = (self.v_image.cd.names[N_PROTECTED_TESTOPT], self.v_image.cd.names[N_TESTOPT]);
        let target = if first == CS_TOKEN_FLAG + protected_testopt {
            self.toks[body + 2]
        } else if first == CS_TOKEN_FLAG + testopt {
            self.toks[body + 1]
        } else {
            0
        };
        if target >= CS_TOKEN_FLAG {
            // `\\newcommand` with an optional argument: `[#1]`, which may not be there, then the others.
            let cs = target - CS_TOKEN_FLAG;
            if !(CALL..END_TEMPLATE).contains(&(self.eq_type(cs) & !ARMED_BITS)) {
                return true;
            }
            let mut q = self.equiv(cs) + 1;
            if k < b && self.host.hv.native.tokens[k] == OTHER_TOKEN + b'[' as i32 {
                return self.native_match(q, k, b).is_some();
            }
            while self.toks[q] != 0 && self.toks[q] != END_MATCH_TOKEN && self.toks[q] != OTHER_TOKEN + b']' as i32 {
                q += 1;
            }
            return self.toks[q] != OTHER_TOKEN + b']' as i32 || self.native_match(q + 1, k, b).is_some();
        }
        // What the body holds may take more than the body has: `\\use@mathgroup` takes the letter after `\\mathrm`.
        if !self.native_body_fits(body, cs, k, b) {
            return false;
        }
        let mut last = 0;
        let mut q = body;
        while self.toks[q] != 0 {
            last = self.toks[q];
            q += 1;
        }
        last < CS_TOKEN_FLAG || last - CS_TOKEN_FLAG == cs || native_scratch_name(self.native_name(last - CS_TOKEN_FLAG)) || self.native_args_fit(last - CS_TOKEN_FLAG, k, b, depth + 1)
    }

    /// Each macro in a body from `q`, left to right, with what it takes by undelimited parameters from what follows it there.  What the body has not got, the run must have.
    fn native_body_fits(&self, mut q: i32, cs: i32, k: usize, b: usize) -> bool {
        let mut wanted = 0;
        while self.toks[q] != 0 {
            let t = self.toks[q];
            q += 1;
            if wanted > 0 {
                if t == SPACE_TOKEN {
                    continue;
                }
                if t < CS_TOKEN_FLAG && t >> 8 == LEFT_BRACE {
                    let mut depth = 1;
                    while depth > 0 && self.toks[q] != 0 {
                        let x = self.toks[q];
                        if x < CS_TOKEN_FLAG {
                            match x >> 8 {
                                LEFT_BRACE => depth += 1,
                                RIGHT_BRACE => depth -= 1,
                                _ => {}
                            }
                        }
                        q += 1;
                    }
                }
                wanted -= 1;
                continue;
            }
            if t < CS_TOKEN_FLAG || t - CS_TOKEN_FLAG == cs {
                continue;
            }
            let d = t - CS_TOKEN_FLAG;
            let w = self.eqtb[d];
            let ty = w.b0() & !ARMED_BITS;
            if ty == IF_TEST && w.rh() == IFX_CODE {
                // Only named.
                q += 2;
                continue;
            }
            if (CALL..END_TEMPLATE).contains(&ty) && !native_scratch_name(self.native_name(d)) {
                wanted = self.native_undelimited(d);
            }
        }
        let tokens = &self.host.hv.native.tokens;
        let mut k = k;
        while wanted > 0 {
            while k < b && tokens[k] == SPACE_TOKEN {
                k += 1;
            }
            if k >= b {
                return false;
            }
            let t = tokens[k];
            k += 1;
            if t < CS_TOKEN_FLAG && t >> 8 == RIGHT_BRACE {
                return false;
            }
            if t < CS_TOKEN_FLAG && t >> 8 == LEFT_BRACE {
                match native_group_end(tokens, k, b) {
                    Some(e) => k = e,
                    None => return false,
                }
            }
            wanted -= 1;
        }
        true
    }

    /// How many things a macro takes for sure, if it takes them all undelimited; 0 for one with a delimiter, which is not looked for.  `\\use@mathgroup` has an `\\math@egroup` behind a conditional, which takes one more.
    fn native_undelimited(&self, cs: i32) -> u32 {
        let mut p = self.equiv(cs) + 1;
        if self.toks[p] == PROTECTED_TOKEN {
            p += 1;
        }
        let mut n = u32::from(self.native_name(cs) == b"use@mathgroup");
        while self.toks[p] != END_MATCH_TOKEN {
            let x = self.toks[p];
            if x == 0 || x >= CS_TOKEN_FLAG || x >> 8 != MATCH {
                return 0;
            }
            n += 1;
            p += 1;
        }
        n
    }

    /// A macro's parameter text from `p` against the tokens from `k`, as §397 would match them: where they end and where the parameter text does.  None if the run ends first.
    fn native_match(&self, p: i32, k: usize, b: usize) -> Option<(usize, i32)> {
        let budget = &self.host.hv.native.scan_budget;
        if budget.get() == 0 {
            return None;
        }
        let r = self.native_match_tokens(p, k, b);
        let seen = (r.map_or(b, |x| x.0).saturating_sub(k) + 1) as u32;
        budget.set(budget.get().saturating_sub(seen));
        r
    }

    fn native_match_tokens(&self, mut p: i32, mut k: usize, b: usize) -> Option<(usize, i32)> {
        let tokens = &self.host.hv.native.tokens;
        loop {
            let x = self.toks[p];
            if x == END_MATCH_TOKEN || x == 0 {
                return Some((k, p));
            }
            if x >= CS_TOKEN_FLAG || x >> 8 != MATCH {
                // A token that the macro has to find here.
                if k >= b || tokens[k] != x {
                    return None;
                }
                k += 1;
                p += 1;
                continue;
            }
            let next = self.toks[p + 1];
            p += 1;
            if next == END_MATCH_TOKEN || (next < CS_TOKEN_FLAG && next >> 8 == MATCH) {
                // Undelimited.
                while k < b && tokens[k] == SPACE_TOKEN {
                    k += 1;
                }
                if k >= b {
                    return None;
                }
                let t = tokens[k];
                k += 1;
                if t < CS_TOKEN_FLAG && t >> 8 == RIGHT_BRACE {
                    return None;
                }
                if t < CS_TOKEN_FLAG && t >> 8 == LEFT_BRACE {
                    k = native_group_end(tokens, k, b)?;
                }
                continue;
            }
            // Delimited: up to the delimiter, outside braces.
            let mut q = p;
            while self.toks[q] != END_MATCH_TOKEN && self.toks[q] != 0 && (self.toks[q] >= CS_TOKEN_FLAG || self.toks[q] >> 8 != MATCH) {
                q += 1;
            }
            let delim = q - p;
            loop {
                if k >= b {
                    return None;
                }
                if (0..delim).all(|i| k + (i as usize) < b && tokens[k + i as usize] == self.toks[p + i]) {
                    k += delim as usize;
                    break;
                }
                let t = tokens[k];
                k += 1;
                if t < CS_TOKEN_FLAG && t >> 8 == RIGHT_BRACE {
                    return None;
                }
                if t < CS_TOKEN_FLAG && t >> 8 == LEFT_BRACE {
                    k = native_group_end(tokens, k, b)?;
                }
            }
            p = q;
        }
    }

    /// What real TeX does first at each cell of an `\\halign`, §785: it expands what the cell starts with, before the template.  So a macro that comes to nothing there leaves an empty cell, with no node; `|` from a macro is the start of a node's options; and what is expanded is expanded outside the cell's group and its math, where other things hold.  The first token after all that must be something TeX will not expand.
    fn native_cell_start(&self, a: usize, b: usize) -> Result<(), &'static str> {
        const WHY: &str = "what a cell starts with is expanded outside it";
        let mut budget = 256;
        for k in a..b {
            match self.native_solid(self.host.hv.native.tokens[k], false, 0, &mut budget) {
                Solidity::Solid => {
                    // A macro that starts with its argument: that is what is expanded first.
                    let t = self.host.hv.native.tokens[k];
                    if t >= CS_TOKEN_FLAG && self.native_starts_with_argument(t - CS_TOKEN_FLAG) {
                        let tokens = &self.host.hv.native.tokens;
                        let mut i = k + 1;
                        while i < b && tokens[i] == SPACE_TOKEN {
                            i += 1;
                        }
                        if i < b && tokens[i] < CS_TOKEN_FLAG && tokens[i] >> 8 == LEFT_BRACE {
                            i += 1;
                        }
                        if i < b && !(tokens[i] < CS_TOKEN_FLAG && tokens[i] >> 8 == RIGHT_BRACE) && self.native_solid(tokens[i], false, 0, &mut budget) != Solidity::Solid {
                            return Err(WHY);
                        }
                    }
                    return Ok(());
                }
                Solidity::Vanishes => {
                    // Its arguments would come next, as they would not if the body had looked at them.
                    let t = self.host.hv.native.tokens[k];
                    if t >= CS_TOKEN_FLAG && self.native_takes_any(t - CS_TOKEN_FLAG) {
                        return Err(WHY);
                    }
                }
                Solidity::Unknown => return Err(WHY),
            }
        }
        Err(WHY)
    }

    fn native_starts_with_argument(&self, cs: i32) -> bool {
        if !(CALL..END_TEMPLATE).contains(&(self.eq_type(cs) & !ARMED_BITS)) {
            return false;
        }
        let mut p = self.equiv(cs) + 1;
        while self.toks[p] != END_MATCH_TOKEN {
            p += 1;
        }
        self.toks[p + 1] < CS_TOKEN_FLAG && self.toks[p + 1] >> 8 == OUT_PARAM
    }

    fn native_takes_any(&self, cs: i32) -> bool {
        if !(CALL..END_TEMPLATE).contains(&(self.eq_type(cs) & !ARMED_BITS)) {
            return false;
        }
        let p = self.equiv(cs) + 1;
        let first = if self.toks[p] == PROTECTED_TOKEN { self.toks[p + 1] } else { self.toks[p] };
        first != END_MATCH_TOKEN
    }

    /// What `native_cell_start` needs to know of one token.
    fn native_solid(&self, t: i32, from_macro: bool, depth: u32, budget: &mut u32) -> Solidity {
        if t < CS_TOKEN_FLAG {
            return match t >> 8 {
                SPACER => Solidity::Vanishes,
                MATCH => Solidity::Unknown,
                OTHER_CHAR if from_macro && t & 255 == b'|' as i32 => Solidity::Unknown,
                _ => Solidity::Solid,
            };
        }
        let cs = t - CS_TOKEN_FLAG;
        // A picture has an active `|` mean the character, and so does real PGF's test for a node's options.
        if cs == ACTIVE_BASE + b'|' as i32 {
            return Solidity::Unknown;
        }
        let w = self.eqtb[cs];
        let ty = w.b0() & !ARMED_BITS;
        if !(CALL..END_TEMPLATE).contains(&ty) {
            return match ty {
                EXPAND_AFTER | NO_EXPAND | INPUT | IF_TEST | FI_OR_ELSE | CS_NAME | CONVERT | THE | TOP_BOT_MARK | UNDEFINED_CS | OMIT | NO_ALIGN | CAR_RET | TAB_MARK => Solidity::Unknown,
                _ => Solidity::Solid,
            };
        }
        // These come to `\\futurelet` or to an assignment, whatever way their test goes, and it does not depend on where they stand.
        if cs == self.v_image.cd.names[N_PROTECTED_TESTOPT] || cs == self.v_image.cd.names[N_TESTOPT] || matches!(self.native_name(cs), b"@ifundefined" | b"@ifnextchar" | b"@ifstar") {
            return Solidity::Solid;
        }
        if depth > 32 || *budget == 0 {
            return Solidity::Unknown;
        }
        *budget -= 1;
        let mut p = w.rh() + 1;
        if self.toks[p] == PROTECTED_TOKEN {
            return Solidity::Solid;
        }
        while self.toks[p] != END_MATCH_TOKEN {
            p += 1;
        }
        p += 1;
        let mut parameter = false;
        while self.toks[p] != 0 {
            let t = self.toks[p];
            p += 1;
            if t < CS_TOKEN_FLAG && t >> 8 == OUT_PARAM {
                parameter = true;
                continue;
            }
            match self.native_solid(t, true, depth + 1, budget) {
                Solidity::Vanishes => {}
                s => return s,
            }
        }
        if parameter { Solidity::Unknown } else { Solidity::Vanishes }
    }

    /// A math-active character means what its active form means wherever it comes to math, in the body of a macro of `SAFE`, by `\\char` or by `\\chardef`.
    fn native_math_active(&mut self) -> Result<(), u8> {
        for c in 0..256 {
            if self.math_code(c) == 0x8000 {
                match self.native_walk_cs(ACTIVE_BASE + c, 0) {
                    Err(R_UNDEFINED) | Ok(()) => {}
                    Err(r) => return Err(r),
                }
            }
        }
        Ok(())
    }

    /// A primitive's name: whoever has made it mean something else has done so for all that mention it, most of which are not looked into.  As LaTeX and the packages of `KNOWN` have it, or what it now holds is judged as a cell's macro would be.
    fn native_primitives(&mut self) -> Result<(), u8> {
        self.native_primitive_list();
        let prims = core::mem::take(&mut self.host.hv.native.prims);
        let mut r = Ok(());
        for &(cs, cmd, chr) in &prims {
            let w = self.eqtb[cs];
            let t = w.b0() & !ARMED_BITS;
            if t == cmd && w.rh() == chr {
                continue;
            }
            if t == UNDEFINED_CS {
                r = Err(R_UNDEFINED);
                break;
            }
            if (CALL..END_TEMPLATE).contains(&t) {
                self.native_closure_known(cs, w.rh());
                let m = *self.host.hv.native.memo.at(cs);
                if m.closure_local != 0 {
                    r = Err(m.closure_local);
                    self.host.hv.native.culprit = cs;
                    break;
                }
                if !m.known {
                    self.host.hv.native.safe_seen.push(cs);
                }
            }
        }
        self.host.hv.native.prims = prims;
        r
    }

    fn native_primitive_list(&mut self) {
        if !self.host.hv.native.prims.is_empty() {
            return;
        }
        let mut prims = Vec::new();
        for p in 1..=PRIM_SIZE {
            let s = self.prim_text(p) - 1;
            if s < 256 || s >= self.str_ptr {
                continue;
            }
            let name = self.str_pool.range(self.str_start[s], self.str_start[s + 1]).to_vec();
            let cs = self.find_cs(&name);
            if cs != UNDEFINED_CONTROL_SEQUENCE {
                prims.push((cs, self.prim_eq_type(p), self.prim_equiv(p)));
            }
        }
        self.host.hv.native.prims = prims;
    }

    fn native_is_primitive(&mut self, cs: i32) -> bool {
        self.native_primitive_list();
        self.host.hv.native.prims.iter().any(|p| p.0 == cs)
    }

    /// Whether `KNOWN` has this body of this macro, and in any case what it mentions, for `native_closure`.  One it has not got is the document's, and what `native_closure` asks of it is only this: that it holds nothing of the structure.  A macro of `SAFE` that calls it is LaTeX's, and has all else in hand.
    fn native_closure_known(&mut self, cs: i32, ptr: i32) -> bool {
        let m = *self.host.hv.native.memo.at(cs);
        if m.closure_ptr == ptr {
            return m.known;
        }
        // With a reference, as `native_walk_macro`'s: the same place can only mean the same tokens.
        if m.closure_ptr != 0 {
            self.delete_token_ref(m.closure_ptr);
        }
        self.add_token_ref(ptr);
        let known = native_table::is_known(self.native_macro_hash(cs, ptr));
        let start = self.host.hv.native.deps.len() as u32;
        let mut local = 0;
        let mut p = ptr + 1;
        // Only named: a character's code, as in `\\catcode`\\\\`, what `\\let` and `\\def` assign to, and the operands of `\\ifx`.
        let mut named = 0;
        while self.toks[p] != 0 {
            let t = self.toks[p];
            p += 1;
            let only_named = named > 0;
            if only_named {
                named -= 1;
            }
            if t < CS_TOKEN_FLAG {
                if !known && t >> 8 == TAB_MARK && !only_named {
                    local = R_ALIGNMENT;
                }
                if t == OTHER_TOKEN + b'`' as i32 {
                    named = 1;
                }
                continue;
            }
            let d = t - CS_TOKEN_FLAG;
            if !known {
                let w = self.eqtb[d];
                let ty = w.b0() & !ARMED_BITS;
                if !only_named && (d == ACTIVE_BASE + b'&' as i32 || matches!(ty, TAB_MARK | CAR_RET | OMIT | NO_ALIGN)) {
                    local = R_ALIGNMENT;
                } else if !only_named && (d == SINGLE_BASE + b'\\' as i32 || native_structural_name(self.native_name(d))) {
                    local = R_STRUCTURE;
                }
                match ty {
                    LET | DEF => named = 1,
                    IF_TEST if w.rh() == IFX_CODE => named = 2,
                    _ => {}
                }
            }
            if d == SINGLE_BASE + b'\\' as i32 {
                continue;
            }
            let name = self.native_name(d);
            // Scratch holds what it last held, these are what a list or a cell makes them, and PGF's are the guards' to look at.
            let skip = native_scratch_name(name) || [&b"pgf"[..], b"tikz", b"endpgf", b"endtikz"].iter().any(|q| name.starts_with(q)) || matches!(name, b"par" | b"@par" | b"@@par");
            if !skip {
                self.host.hv.native.deps.push(d);
            }
        }
        let end = self.host.hv.native.deps.len() as u32;
        let m = self.host.hv.native.memo.at(cs);
        m.closure_ptr = ptr;
        m.known = known;
        m.closure_local = local;
        m.all = (start, end);
        known
    }

    /// `SAFE` holds a macro by its own text.  What it says by name is as installed too, or it is not what the hash vouched for: what a macro of `SAFE` mentions is looked at to the end, by `native_closure_known`.
    /// All of that stays so as long as each body stands where it did, which costs a comparison for each and no walk.  What has moved is looked at again, with what it mentions now: real `\\selectfont` writes `\\font@name` and its like for every diagram.
    fn native_closure(&mut self) -> Result<(), u8> {
        let r = self.native_closure_walk();
        if r.is_err() {
            // What was left to do is lost, and with it what those mention.
            let hv = &mut self.host.hv.native;
            hv.flat.clear();
            hv.flat_gen += 1;
        }
        r
    }

    fn native_closure_walk(&mut self) -> Result<(), u8> {
        let hv = &mut self.host.hv.native;
        hv.flat_gen = hv.flat_gen.max(1);
        let generation = hv.flat_gen;
        let mut flat = core::mem::take(&mut hv.flat);
        let mut todo = Vec::new();
        flat.retain(|&(cs, ptr)| {
            let same = self.eqtb[cs].rh() == ptr;
            if !same {
                todo.push(cs);
            }
            same
        });
        let hv = &mut self.host.hv.native;
        hv.flat = flat;
        for &cs in &todo {
            hv.memo.at(cs).closure_stamp = 0;
        }
        loop {
            let hv = &mut self.host.hv.native;
            let Some(cs) = todo.pop().or_else(|| hv.safe_seen.pop()) else { break };
            let w = self.eqtb[cs];
            if !(CALL..END_TEMPLATE).contains(&(w.b0() & !ARMED_BITS)) {
                // It can become a macro, and what mentions it stays where it is.
                let m = hv.memo.at(cs);
                if m.closure_stamp != generation {
                    m.closure_stamp = generation;
                    hv.flat.push((cs, w.rh()));
                }
                continue;
            }
            if hv.budget == 0 {
                return Err(R_BUDGET);
            }
            hv.budget -= 1;
            self.native_closure_known(cs, w.rh());
            let hv = &mut self.host.hv.native;
            let m = hv.memo.at(cs);
            if m.closure_local != 0 {
                let r = m.closure_local;
                hv.culprit = cs;
                return Err(r);
            }
            if m.closure_stamp == generation {
                continue;
            }
            m.closure_stamp = generation;
            let (a, b) = m.all;
            hv.flat.push((cs, w.rh()));
            todo.extend_from_slice(&hv.deps[a as usize..b as usize]);
        }
        Ok(())
    }

    /// Whether `\\` and `&` are this environment's: 2 for both, 1 for `\\`, 0 for neither.  As LaTeX and amsmath have it, or not.
    fn native_env_kind(&self, name: &[u8], cs: i32) -> u8 {
        let kind = if ENV_TABS.contains(&name) {
            2
        } else if ENV_LINES.contains(&name) {
            1
        } else {
            0
        };
        if kind != 0 && self.native_is_stock(cs) { kind } else { 0 }
    }

    /// A macro as LaTeX or a package of `KNOWN` has it.
    fn native_is_stock(&self, cs: i32) -> bool {
        cs != UNDEFINED_CONTROL_SEQUENCE && (CALL..END_TEMPLATE).contains(&(self.eq_type(cs) & !ARMED_BITS)) && native_table::is_known(self.native_macro_hash(cs, self.equiv(cs)))
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
        // The matrix gives it the meaning of a cell's end.
        if cs == ACTIVE_BASE + b'&' as i32 {
            return Err(R_ALIGNMENT);
        }
        let w = self.eqtb[cs];
        let t = w.b0() & !ARMED_BITS;
        if t >= CALL && t < END_TEMPLATE {
            if t >= OUTER_CALL {
                return Err(R_OUTER);
            }
            // LaTeX's `\\par` is a macro, and has other names.
            if w.rh() == self.equiv(self.par_loc) {
                return Err(R_FORMULA);
            }
            // The library has read the colours that keys name.
            if matches!(self.native_name(cs), b"definecolor" | b"colorlet" | b"providecolor" | b"xglobal" | b"xdefinecolor" | b"definecolors" | b"providecolors" | b"definecolorset" | b"providecolorset" | b"selectcolormodel" | b"definecolorseries" | b"tracingall" | b"tracingnone" | b"loggingall" | b"showoutput") {
                return Err(R_LEAK);
            }
            // amsmath makes macros of these.
            if matches!(self.native_name(cs), b"eqno" | b"leqno") {
                return Err(R_FORMULA);
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

    /// A token that a macro only names, by `\\ifx`, by `\\let` or as a character's code, and that real PGF has its way with: a macro that compares it with what comes next sees something else there than on the path.
    fn native_is_structural(&self, t: i32) -> bool {
        if t < CS_TOKEN_FLAG {
            return t >> 8 == TAB_MARK;
        }
        let cs = t - CS_TOKEN_FLAG;
        if cs == SINGLE_BASE + b'\\' as i32 || cs == ACTIVE_BASE + b'&' as i32 || cs == self.v_image.cd.names[N_END] {
            return true;
        }
        native_structural_name(self.native_name(cs))
    }

    /// A meaning that is no macro.
    fn native_meaning_of(&self, cs: i32, t: i32, chr: i32) -> u8 {
        if cs >= HASH_BASE {
            let name = self.native_name(cs);
            if name.starts_with(b"pgf") || name.starts_with(b"tikz") {
                return R_PGF;
            }
            if native_structural_name(name) {
                return R_STRUCTURE;
            }
        }
        match t {
            UNDEFINED_CS => R_UNDEFINED,
            XRAY | STOP | INPUT | READ_TO_CS | IN_STREAM | PARTOKEN_NAME => R_PRIMITIVE,
            LEADER_SHIP if chr == A_LEADERS - 1 => R_PRIMITIVE,
            DEF_CODE if chr == CAT_CODE_BASE => R_CATCODE,
            ASSIGN_INT if chr == INT_BASE + END_LINE_CHAR_CODE => R_CATCODE,
            // Real PGF's tokens and its `\\hbox`es would run under these, and the path's are others.
            ASSIGN_INT if chr == INT_BASE + GLOBAL_DEFS_CODE || native_is_tracing(chr - INT_BASE) => R_TRACING,
            ASSIGN_TOKS if matches!(chr, EVERY_HBOX_LOC | EVERY_VBOX_LOC | EVERY_MATH_LOC) => R_TRACING,
            // Vertical material and the like: an error in a formula, which a cell of real PGF's does not report as this one does.
            PAR_END | VSKIP | HRULE | UN_HBOX | UN_VBOX | EQ_NO | END_CS_NAME => R_FORMULA,
            // Marks hold anything, and a case change can make any active character.
            TOP_BOT_MARK | CASE_SHIFT => R_CSNAME,
            // Font parameters are global whatever the group, and real PGF reads them for what comes after this cell.  The `\\every`s run in real PGF's own boxes and formulas.  The others log or write what real PGF does differently.
            ASSIGN_FONT_DIMEN | ASSIGN_FONT_INT => R_LEAK,
            // These read a file's name up to a blank, or on through the `}` of a label, whose last blank the library has taken off.
            DEF_FONT => R_LEAK,
            EXTENSION if chr == OPEN_NODE => R_LEAK,
            DEF_CODE if chr == MATH_CODE_BASE => R_LEAK,
            ASSIGN_TOKS | TOKS_REGISTER if chr < TOKS_BASE || cs >= HASH_BASE && matches!(self.native_name(cs), b"everymath" | b"everydisplay") => R_LEAK,
            ASSIGN_INT if native_param_is_real_pgfs(chr - INT_BASE) => R_LEAK,
            TAB_MARK | CAR_RET | OMIT | NO_ALIGN => R_ALIGNMENT,
            LAST_ITEM if matches!(chr, LAST_NODE_TYPE_CODE | INPUT_LINE_NO_CODE | BADNESS_CODE | PDF_LAST_OBJ_CODE | PDF_LAST_XFORM_CODE | PDF_LAST_XIMAGE_CODE | PDF_LAST_XIMAGE_PAGES_CODE | PDF_LAST_ANNOT_CODE | PDF_LAST_X_POS_CODE | PDF_LAST_Y_POS_CODE | PDF_LAST_XIMAGE_COLORDEPTH_CODE | PDF_LAST_LINK_CODE | CURRENT_GROUP_LEVEL_CODE | CURRENT_GROUP_TYPE_CODE | CURRENT_IF_LEVEL_CODE | CURRENT_IF_TYPE_CODE | CURRENT_IF_BRANCH_CODE) => R_PLACE,
            EXTENSION => match chr {
                WRITE_NODE | CLOSE_NODE | SPECIAL_NODE | IMMEDIATE_CODE | PDF_LITERAL_NODE | PDF_COLORSTACK_NODE | SET_LANGUAGE_CODE | PDF_REFXIMAGE_NODE | PDF_REFXFORM_NODE
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
        let stamp = hv.stamp;
        let m = *hv.memo.at(cs);
        if m.ptr == ptr && m.stamp == stamp {
            return if m.verdict == 0 { Ok(()) } else { Err(m.verdict) };
        }
        let m = if m.ptr == ptr {
            m
        } else {
            // With a reference, the same place can only mean the same tokens: otherwise a redefinition can land on a block that this memo still names.
            if m.ptr != 0 {
                self.delete_token_ref(m.ptr);
            }
            self.add_token_ref(ptr);
            self.native_analyse(cs, ptr, m)
        };
        let hv = &mut self.host.hv.native;
        if m.local != 0 {
            hv.culprit = cs;
        }
        if m.safe {
            hv.safe_seen.push(cs);
        }
        // Cycles: a macro under walk counts as clean; its other mentions decide.
        let mut entry = m;
        entry.stamp = stamp;
        entry.verdict = m.local;
        *hv.memo.at(cs) = entry;
        if m.local != 0 {
            return Err(m.local);
        }
        let (a, b) = m.deps;
        for k in a..b {
            let dep = self.host.hv.native.deps[k as usize];
            if let Err(r) = self.native_walk_cs(dep, depth + 1) {
                self.host.hv.native.memo.at(cs).verdict = r;
                if self.v_image.cd.report_names {
                    self.host.hv.native.chain.push(cs);
                }
                return Err(r);
            }
        }
        Ok(())
    }

    /// A macro's body: `Memo::local` and what it mentions.  A body the table knows is clean, and what it mentions is `native_closure`'s business.
    fn native_analyse(&mut self, cs: i32, ptr: i32, old: Memo) -> Memo {
        let mut m = if native_aftergroup_of_latex(self.native_name(cs)) {
            Memo { ptr, local: R_AFTER_GROUP, ..Memo::default() }
        } else if native_table::is_safe(self.native_macro_hash(cs, ptr)) {
            Memo { ptr, safe: true, ..Memo::default() }
        } else {
            self.native_analyse_body(cs, ptr)
        };
        m.closure_ptr = old.closure_ptr;
        m.known = old.known;
        m.closure_local = old.closure_local;
        m.all = old.all;
        m.closure_stamp = old.closure_stamp;
        m
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
        if native_structural_name(name) {
            m.local = R_STRUCTURE;
            return m;
        }
        // A macro as LaTeX or a package has it, that `SAFE` does not hold because of its name: it is looked at for the structure, and what is only ill-mannered in a macro of the document is its own business.
        let stock = native_table::is_known(self.native_macro_hash(cs, ptr));
        let start = self.host.hv.native.deps.len() as u32;
        let begin = self.v_image.cd.names[N_BEGIN];
        let end = self.v_image.cd.names[N_END];
        // An `\\halign` of its own, for the rest of the body, and the environments open here with whether each has `&` and `\\`.
        let mut aligned = false;
        let mut boxed: Option<OpenBox> = None;
        // `\\ifmmode` has come to its `\\else`, which a formula does not read, and how many conditionals that takes: `$` is no matter there.
        let mut mmode: usize = 0;
        let mut mmode_else = false;
        let mut envs = [0u8; 8];
        let mut nenvs = 0;
        let mut bal = Balance::default();
        // The parameter text only delimits.
        let mut p = ptr + 1;
        while self.toks[p] != END_MATCH_TOKEN && self.toks[p] != 0 {
            p += 1;
        }
        let backslash = SINGLE_BASE + b'\\' as i32;
        let mut prev = 0;
        // How many of the next tokens are only named, not used: the two of `\\ifx`, what `\\let` and `\\def` assign to.  An assignment has a target, which must not be PGF's or real tikz-cd's.
        let mut named = 0;
        let mut target = false;
        // A `\\let` has had its target: what follows, past `=`, is only its value, and opens no conditional.
        let mut value = false;
        // Past a character's code: a brace there is no brace, but it is how the master counter of an alignment is reached.
        let mut code = false;
        loop {
            let t = self.toks[p];
            p += 1;
            if t == 0 {
                break;
            }
            if named > 0 && t != END_MATCH_TOKEN {
                named -= 1;
                let name_is_ours = target && t >= CS_TOKEN_FLAG && {
                    let n = self.native_name(t - CS_TOKEN_FLAG);
                    n.starts_with(b"pgf") || n.starts_with(b"tikz")
                };
                if !stock && (name_is_ours || self.native_is_structural(t)) {
                    m.local = if name_is_ours { R_PGF } else { R_STRUCTURE };
                    break;
                }
                // A primitive given another meaning in a cell: real PGF goes on using it.
                if !stock && target && t >= CS_TOKEN_FLAG && self.native_is_primitive(t - CS_TOKEN_FLAG) {
                    m.local = R_LEAK;
                    break;
                }
                if !stock && code && t < CS_TOKEN_FLAG && matches!(t >> 8, LEFT_BRACE | RIGHT_BRACE) {
                    m.local = R_GROUP;
                    break;
                }
                code = false;
                target = false;
                prev = t;
                continue;
            }
            let is_value = value && t != OTHER_TOKEN + b'=' as i32 && t != SPACE_TOKEN;
            if t < CS_TOKEN_FLAG {
                if is_value {
                    value = false;
                }
                match t >> 8 {
                    TAB_MARK if !aligned && (nenvs == 0 || envs[nenvs - 1] != 2) => {
                        m.local = R_ALIGNMENT;
                        break;
                    }
                    LEFT_BRACE if !is_value => bal.braces += 1,
                    RIGHT_BRACE if !is_value => {
                        bal.braces -= 1;
                        native_close_box(&mut boxed, bal.braces, native_opens_argument(self.toks[p]));
                    }
                    MATH_SHIFT if !is_value => {
                        if !stock && !mmode_else && !native_in_box(boxed, bal.braces) {
                            m.local = R_FORMULA;
                            break;
                        }
                        bal.dollars = !bal.dollars;
                    }
                    _ => {}
                }
                // A number by a character's code: the control sequence after it is only that character.
                if t == OTHER_TOKEN + b'`' as i32 {
                    named = 1;
                    code = true;
                }
                prev = t;
                continue;
            }
            let d = t - CS_TOKEN_FLAG;
            if d == end {
                // Its name is what `\\begin`'s was, and `\\end` itself is guarded.
                let q = self.native_env_name_in_body(p);
                let Some((name, after)) = q else {
                    m.local = R_ENV;
                    break;
                };
                if name == b"tikzcd" || (nenvs == 0 && !stock) {
                    m.local = R_ENV;
                    break;
                }
                nenvs = nenvs.saturating_sub(1);
                p = after;
                prev = t;
                continue;
            }
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
                if !fixed || self.toks[q] == 0 {
                    m.local = R_CSNAME;
                    break;
                }
                let found = self.find_cs(&name);
                if found != UNDEFINED_CONTROL_SEQUENCE {
                    self.host.hv.native.deps.push(found);
                }
                // Past `\\endcsname`, which here has its place.
                p = q + 1;
                continue;
            }
            if d == begin {
                // `\begin{matrix}`, with a name that can be read here.  Any other is beyond proof.
                let Some((name, after)) = self.native_env_name_in_body(p) else {
                    m.local = R_ENV;
                    break;
                };
                if ENV_DISPLAY.contains(&name.as_slice()) {
                    m.local = R_FORMULA;
                    break;
                }
                if name == b"tikzcd" || nenvs == envs.len() {
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
                envs[nenvs] = if stock { 2 } else { self.native_env_kind(&name, n) };
                nenvs += 1;
                p = after;
                continue;
            }
            if d == backslash {
                let assigned = prev >= CS_TOKEN_FLAG && matches!(self.eqtb[prev - CS_TOKEN_FLAG].b0() & !ARMED_BITS, LET | DEF);
                if !assigned && !aligned && (nenvs == 0 || envs[nenvs - 1] == 0) {
                    m.local = R_STRUCTURE;
                    break;
                }
                prev = t;
                continue;
            }
            // A stock macro is counted too, for its `\\aftergroup`s, and may leave open what its other half closes.
            if !is_value && !bal.meaning(ty, w.rh()) && !stock {
                m.local = R_GROUP;
                break;
            }
            value = false;
            if !stock && native_global_param(ty) && prev >= CS_TOKEN_FLAG && self.eqtb[prev - CS_TOKEN_FLAG].b0() & !ARMED_BITS == PREFIX {
                m.local = R_LEAK;
                break;
            }
            if !stock && native_is_math_delimiter(d) {
                m.local = R_FORMULA;
                break;
            }
            match ty {
                // In a formula this is an error that TeX recovers from by the groups it finds there, and those are not real PGF's.
                HALIGN | VALIGN if !stock && !native_in_box(boxed, bal.braces) => {
                    m.local = R_FORMULA;
                    break;
                }
                HALIGN | VALIGN => aligned = true,
                MAKE_BOX | VCENTER if boxed.is_none() => boxed = Some(OpenBox { depth: bal.braces, args: 1 }),
                CALL..END_TEMPLATE if boxed.is_none() && native_opens_argument(self.toks[p]) && native_takes_text(self.native_name(d)) > 0 && self.native_is_stock(d) => boxed = Some(OpenBox { depth: bal.braces, args: native_takes_text(self.native_name(d)) }),
                IF_TEST if w.rh() == IF_MMODE_CODE && !mmode_else => mmode = bal.nconds,
                FI_OR_ELSE if w.rh() == ELSE_CODE && mmode != 0 && mmode == bal.nconds => mmode_else = true,
                FI_OR_ELSE if w.rh() == FI_CODE && mmode > bal.nconds => {
                    mmode = 0;
                    mmode_else = false;
                }
                TAB_MARK | CAR_RET | OMIT | NO_ALIGN if !aligned && (nenvs == 0 || envs[nenvs - 1] != 2) => {
                    m.local = R_ALIGNMENT;
                    break;
                }
                TAB_MARK | CAR_RET | OMIT | NO_ALIGN => {
                    prev = t;
                    continue;
                }
                IF_TEST if w.rh() == IFX_CODE => named = 2,
                IF_TEST if w.rh() == IF_CS_CODE => {
                    // `\\ifcsname`: only asks.
                    let mut q = p;
                    while self.toks[q] != 0 && (self.toks[q] < CS_TOKEN_FLAG || self.eqtb[self.toks[q] - CS_TOKEN_FLAG].b0() & !ARMED_BITS != END_CS_NAME) {
                        q += 1;
                    }
                    if self.toks[q] == 0 {
                        m.local = R_GROUP;
                        break;
                    }
                    p = q + 1;
                    continue;
                }
                IF_TEST if w.rh() == IF_DEF_CODE => named = 1,
                LET if w.rh() == NORMAL => {
                    named = 1;
                    target = true;
                    value = true;
                }
                DEF => {
                    named = 1;
                    target = true;
                }
                AFTER_GROUP if !bal.in_group() => {
                    m.local = R_AFTER_GROUP;
                    break;
                }
                THE if !stock && w.rh() == 0 && !self.native_the_takes(self.toks[p]) => {
                    m.local = R_THE;
                    break;
                }
                _ => {}
            }
            if ty == UNDEFINED_CS && self.native_undefined_is_fine(d) {
                prev = t;
                continue;
            }
            if self.native_is_expl3(d) {
                // expl3's own are its business, but for a verbatim argument of `\\NewDocumentCommand`, whose tokens are not to be read ahead.  What the document or a package made with such a name is walked.
                if self.native_name(d) == b"__cmd_start:nNNnnn" && self.native_group_has_v(p) {
                    m.local = R_CATCODE;
                    break;
                }
                let stock = !(CALL..END_TEMPLATE).contains(&ty) || native_table::is_known(self.native_macro_hash(d, w.rh()));
                if stock {
                    prev = t;
                    continue;
                }
            }
            if ty >= CALL && ty < END_TEMPLATE {
                if ty >= OUTER_CALL {
                    m.local = R_OUTER;
                    break;
                }
                self.host.hv.native.deps.push(d);
            } else {
                let r = self.native_meaning_of(d, ty, w.rh());
                if r != 0 && !(stock && matches!(r, R_FORMULA | R_PEEK | R_PLACE | R_LEAK)) {
                    m.local = r;
                    if self.v_image.cd.report_names {
                        self.host.hv.native.chain.push(d);
                    }
                    break;
                }
                // That is what it means now, and a `\\let` can change it with this body as it is.
                if r == 0 {
                    self.host.hv.native.deps.push(d);
                }
            }
            prev = t;
        }
        if m.local == 0 && !stock && (!bal.closed() || nenvs != 0) {
            m.local = R_GROUP;
        }
        m.deps = (start, self.host.hv.native.deps.len() as u32);
        m
    }

    /// `{name}` at `p` in a macro's body, if it can be read there, with where it ends.
    fn native_env_name_in_body(&self, p: i32) -> Option<(Vec<u8>, i32)> {
        if self.toks[p] >> 8 != LEFT_BRACE || self.toks[p] >= CS_TOKEN_FLAG {
            return None;
        }
        let mut name = Vec::new();
        let mut q = p + 1;
        while self.toks[q] >> 8 != RIGHT_BRACE {
            let x = self.toks[q];
            if x == 0 || x >= CS_TOKEN_FLAG || x >> 8 == OUT_PARAM {
                return None;
            }
            name.push((x & 255) as u8);
            q += 1;
        }
        Some((name, q + 1))
    }

    /// In a macro's body, as against a cell: what has an @ in its name is defined when it is needed or is a delimiter, like graphics' `\\Gscale@x` and `\\@nil`, and these are read by `\\ifthenelse`.
    /// A misspelt name in a macro of the document has neither, and goes to real PGF for its error message.
    fn native_undefined_is_fine(&self, cs: i32) -> bool {
        // An active character without a meaning, in a body: `\\lowercase` makes others of it, as `ifthen` does.
        if cs < SINGLE_BASE {
            return cs != ACTIVE_BASE + b'&' as i32;
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
            // A primitive, by what it does.
            Some(0x5052_0000_0000_0000 + ((t as u64) << 32) + w.rh() as u32 as u64)
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

    /// A list of `self.host.hv.native.list`'s tokens, a step at its end.  Where `breaks` says that the next line of the source starts, a new level starts, whose end `native_line_ended` hears of, since
    /// the place and the macros reading across it must not see any token there: `\\inputlineno` and all that TeX says of a line then goes as with the file.
    fn native_run_list(&mut self) {
        let list = core::mem::take(&mut self.host.hv.native.list);
        let breaks = core::mem::take(&mut self.host.hv.native.breaks);
        // The list before has only its step behind it: its level goes first, or a diagram's levels pile up.
        if self.state() == TOKEN_LIST && self.token_type() == INSERTED && self.at_list_end() {
            self.end_token_list();
        }
        let mut end = list.len();
        for k in (0..=breaks.len()).rev() {
            let start = if k == 0 { 0 } else { breaks[k - 1].0 };
            let m = self.tb_mark();
            for &t in &list[start..end] {
                self.tb_push(t);
            }
            if k == breaks.len() {
                self.tb_push(CS_TOKEN_FLAG + CD_STEP);
            }
            let p = self.tb_commit(m);
            if k == breaks.len() {
                self.begin_token_list(p, INSERTED);
            } else {
                self.begin_token_list(p, CD_LINE_TEXT);
                self.delete_token_ref(p);
            }
            end = start;
        }
        let hv = &mut self.host.hv.native;
        hv.line_queue.clear();
        hv.line_queue.extend(breaks.iter().rev().map(|b| b.1));
        hv.breaks = breaks;
        hv.breaks.clear();
        hv.list = list;
    }

    fn native_cs_token(&self, k: i32) -> i32 {
        CS_TOKEN_FLAG + self.v_image.cd.names[k]
    }

    /// The input is at the `\end`.  From here on the diagram is ours.
    fn native_start(&mut self) {
        self.v_image.cd.active = true;
        self.align_state += 1;
        self.host.hv.native.end_line = self.line;
        self.line = self.native_first_cell_line();
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
        self.host.hv.native.matrix_number = self.native_count_read(N_FIG_COUNT).unwrap_or(0);
    }

    /// What the end of a real diagram leaves in the registers, as far as it does not depend on the last path drawn.  All of it `\\global`.
    fn native_left_behind(&mut self, d: &Diagram, minx: i32, maxx: i32, miny: i32, maxy: i32) {
        self.native_matrix_counts(d);
        // `\\global\\let\\tikzcdmatrixname\\tikzlastnode`, in the `\\pgfextra` after the matrix: `tikz@f@N`.
        let cs = self.find_cs(b"tikzcdmatrixname");
        let n = self.host.hv.native.matrix_number;
        if cs != UNDEFINED_CONTROL_SEQUENCE {
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
        const SIXTEEN_THOUSAND: i32 = 16000 * 65536;
        // `\\pgf@process` of the origin; 0.4pt of `\\pgfpicture`, each path's own width being in its scope; `\\pgf@resetpathsizes`.
        for (k, v) in [
            (N_PGF_X, 0),
            (N_PGF_Y, 0),
            (N_PICMINX, minx),
            (N_PICMAXX, maxx),
            (N_PICMINY, miny),
            (N_PICMAXY, maxy),
            (N_PATHMINX, SIXTEEN_THOUSAND),
            (N_PATHMAXX, -SIXTEEN_THOUSAND),
            (N_PATHMINY, SIXTEEN_THOUSAND),
            (N_PATHMAXY, -SIXTEEN_THOUSAND),
            (N_LINEWIDTH, 26214),
        ] {
            self.native_global_set(k, v);
        }
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
        self.native_count_of(cs)
    }

    fn native_count_of(&self, cs: i32) -> Option<i32> {
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
            ASSIGN_INT | ASSIGN_DIMEN => {
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
                    let r = d.parse_with(&bytes[..n], &bytes[n..], font, &AllowAll, &DocKeys(&*self));
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
            Stage::LastFonts => self.native_close_waiting(),
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

    /// The line of the first cell with something in it: real PGF's first `$` is there, since `\\halign` has looked at its first token, and a font first needed there says so.
    fn native_first_cell_line(&mut self) -> i32 {
        let d = self.host.hv.native.d.take().unwrap_or_default();
        let first = (0..d.cells().len()).find_map(|k| d.cell_runs(k).first().copied());
        let line = first.and_then(|s| self.native_span(s)).map(|r| self.host.hv.native.lines.get(r.0).copied().unwrap_or(0));
        self.host.hv.native.d = Some(d);
        match line {
            Some(l) if l != 0 => l,
            _ => self.host.hv.native.end_line,
        }
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
        // The end of a real diagram has the document's font selected and its math fonts in order again, whatever cells and labels did.  `\\font@name` and `\\tf@size` and its like are global.
        self.line = self.host.hv.native.end_line;
        self.host.hv.native.stage = Stage::LastFonts;
        let list = [MATH_SHIFT_TOKEN + b'$' as i32, self.native_cs_token(N_RELAX), MATH_SHIFT_TOKEN + b'$' as i32, self.native_cs_token(N_SELECTFONT)];
        self.host.hv.native.list.clear();
        self.host.hv.native.list.extend_from_slice(&list);
        self.native_run_list();
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
        // A label's tokens wait in real PGF until the diagram's end, so only a cell goes by lines.
        let mut breaks = core::mem::take(&mut self.host.hv.native.breaks);
        breaks.clear();
        let mut at_line = self.line;
        for (n, &s) in runs.iter().enumerate() {
            // An arrow stood here.  What came before it cannot run on into what came after it: `\\relax` stops a number and a keyword as the arrow's own macros do, and ends a word.
            if n > 0 {
                list.push(relax);
            }
            let (a, b) = self.native_span(s).unwrap_or((0, 0));
            for k in a..b {
                let t = self.host.hv.native.tokens[k];
                if cell {
                    let l = self.host.hv.native.lines.get(k).copied().unwrap_or(0);
                    if l != 0 && l != at_line {
                        breaks.push((list.len(), l));
                        at_line = l;
                    }
                }
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
        self.host.hv.native.breaks = breaks;
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
        // With the library only the box goes nowhere but to `session.rs`, and nothing is there to leave vertical mode for.
        if !self.session_library_only() {
            self.host.hv.native.list.push(e);
        }
        self.native_run_list();
    }

    /// Something has shown that this run cannot be what real PGF gives.  The driver starts the job again with the path off, in this process, and that does not return.  A run that cannot be
    /// done over stays as it is: one that has run a command or read from the terminal.  The files it has overwritten come back first, since the second run reads them.
    #[cold]
    pub fn native_restart(&mut self, why: &'static str) {
        if self.host.world.done_once {
            return;
        }
        let Some(again) = self.host.hv.native.restart.take() else { return };
        self.host.checkpoint.join();
        self.host.world.restore_kept_outputs();
        let written = self.host.world.term_written;
        again(written, why);
    }

    /// §82: an error on its way to the user.  In a cell, a real run would say it from elsewhere, with other context, and go on otherwise.
    #[cold]
    pub fn native_on_error(&mut self) {
        if self.v_image.cd.active {
            self.native_restart("an error in a cell");
        }
    }

    /// §245: something is about to be traced in a cell, by what a macro of `SAFE` has set there, such as `\\tracingall`.  What would show is not what real PGF shows.
    #[cold]
    pub fn native_on_diagnostic(&mut self) {
        if self.native_tracing_on() {
            self.native_restart("tracing in a cell");
        }
    }

    /// §1283, `\\errmessage`, after a diagram went by here.  PGF's error for a name that no node has: the name may be that of a node of such a diagram, whose table only real PGF fills.
    #[cold]
    pub fn native_on_errmessage(&mut self, s: i32) {
        let text = self.str_pool.range(self.str_start[s], self.str_start[s + 1]);
        if text.windows(14).any(|w| w == b"No shape named") {
            self.native_restart("a picture asks for a node that may be of a diagram of this path");
        }
    }

    /// §216 `show_box`: `\\showbox`, `\\showlists`, `\\tracingoutput` or an overfull box is about to show what is inside a box, and a diagram of this run may be there with other nodes than real PGF's.
    #[cold]
    pub fn native_on_show(&mut self) {
        self.native_restart("the inside of a box is shown");
    }

    /// With `--commute-report`, that this run is the second one.
    #[cold]
    pub fn native_report_restart(&mut self) {
        if let Some(why) = self.host.hv.native.restarted.take() {
            if self.v_image.cd.report {
                self.native_report(b"[[commute restart]] ", why.as_bytes());
            }
        }
    }

    /// `end_token_list` of a list of `CD_LINE_TEXT`: what follows is from the next line.
    #[cold]
    pub fn native_line_ended(&mut self) {
        if let Some(l) = self.host.hv.native.line_queue.pop() {
            self.line = l;
        }
    }

    /// What should have been the end of a diagram went otherwise.
    fn native_failed(&mut self, why: &'static str) {
        self.native_restart(why);
        self.native_report(b"[[commute failure]] ", why.as_bytes());
        self.v_image.cd.active = false;
        self.host.hv.native.stage = Stage::Idle;
    }

    /// xcolor's target model: `\\XC@tgt@mod`, `#1->#1` for none, and it is empty then, or `#1->rgb` and the like.
    fn native_color_model(&self) -> Option<Vec<u8>> {
        let m = self.find_cs(b"XC@tgt@mod");
        if m == UNDEFINED_CONTROL_SEQUENCE || self.eq_type(m) != CALL {
            return None;
        }
        let mut p = self.equiv(m) + 1;
        if self.toks[p] >> 8 != MATCH || self.toks[p + 1] != END_MATCH_TOKEN {
            return None;
        }
        p += 2;
        if self.toks[p] >> 8 == OUT_PARAM && self.toks[p + 1] == 0 {
            return Some(Vec::new());
        }
        let mut name = Vec::with_capacity(4);
        while self.toks[p] != 0 {
            if self.toks[p] >= CS_TOKEN_FLAG {
                return None;
            }
            name.push((self.toks[p] & 255) as u8);
            p += 1;
        }
        Some(name)
    }

    /// Whether a macro with no parameters holds just this text.
    fn native_value_is(&self, name: &[u8], text: &[u8]) -> bool {
        let cs = self.find_cs(name);
        if cs == UNDEFINED_CONTROL_SEQUENCE || !(CALL..END_TEMPLATE).contains(&(self.eq_type(cs) & !ARMED_BITS)) {
            return false;
        }
        let mut p = self.equiv(cs) + 1;
        if self.toks[p] != END_MATCH_TOKEN {
            return false;
        }
        for &c in text {
            p += 1;
            if self.toks[p] >= CS_TOKEN_FLAG || self.toks[p] & 255 != c as i32 {
                return false;
            }
        }
        self.toks[p + 1] == 0
    }

    /// What `\definecolor` or xcolor itself has made of a name: `\xcolor@{}{<operators>}{<model>}{<values>}` in `\\color@<name>`.
    fn native_named_color(&self, name: &[u8]) -> Option<Color> {
        let mut buf = [0u8; 256];
        let full = native_key_name(&mut buf, b"\\color@", name, b"")?;
        let cs = self.find_cs(full);
        if cs == UNDEFINED_CONTROL_SEQUENCE || !(CALL..END_TEMPLATE).contains(&(self.eq_type(cs) & !ARMED_BITS)) {
            return None;
        }
        let mut p = self.equiv(cs) + 1;
        let head = self.find_cs(b"xcolor@");
        if self.toks[p] != END_MATCH_TOKEN || self.toks[p + 1] != CS_TOKEN_FLAG + head {
            return None;
        }
        p += 2;
        // `{}{`
        for _ in 0..2 {
            if self.toks[p] >> 8 != LEFT_BRACE || self.toks[p] >= CS_TOKEN_FLAG || self.toks[p + 1] >> 8 != RIGHT_BRACE {
                break;
            }
            p += 2;
        }
        if self.toks[p] >> 8 != LEFT_BRACE || self.toks[p] >= CS_TOKEN_FLAG {
            return None;
        }
        p += 1;
        let mut s = Vec::with_capacity(48);
        while self.toks[p] != 0 && self.toks[p] >> 8 != RIGHT_BRACE {
            if self.toks[p] >= CS_TOKEN_FLAG {
                return None;
            }
            s.push((self.toks[p] & 255) as u8);
            p += 1;
        }
        native_color_of_operators(&s)
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
        native_color_of_operators(&s)
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
                // Real PGF adds up what is far beyond a page, and may come to `Dimension too large`, where the path has no error.
                const FAR: f64 = 4000.0;
                if l.maxx - l.minx > FAR || l.maxy - l.miny > FAR {
                    self.native_restart("a diagram of over 4000pt");
                }
                if self.session_on() {
                    self.session_svg(&d, ambient, &boxes_of_cells, &boxes_of_labels);
                }
                if self.session_library_only() {
                    self.session_page_resources(&d);
                }
                let mut scratch = core::mem::take(&mut self.host.hv.native.ops);
                let mut sink = Sink::new(&mut *self, &boxes_of_cells, &boxes_of_labels);
                pdf::emit(&d, ambient, &mut scratch, &mut sink);
                // `\\pgfpicture`'s `\\hss`.  `find_protchar_right` stops at a glue, and would otherwise find the last character of the diagram, since it passes empty boxes and whatsits.
                let (first, seg) = (sink.first, sink.seg);
                self.host.hv.native.ops = scratch;
                let hss = self.new_glue(SS_GLUE);
                self.set_link(seg, hss);
                let inner = self.new_null_box();
                self.set_list_ptr(inner, first);
                let sp = |v: f64| (v * 65536.0).round() as i32;
                self.native_left_behind(&d, sp(l.minx), sp(l.maxx), sp(l.miny), sp(l.maxy));
                self.v_image.cd.ran = true;
                self.set_shift_amount(inner, sp(l.miny));
                // A glue, as `\\pgfsys@typesetpicturebox`'s `\\hskip` is, and not a kern: `find_protchar_left` passes a kern and goes on into the box, to the first character of a cell.
                let spec = self.new_spec(ZERO_GLUE);
                self.set_width(spec, sp(-l.minx));
                let k = self.new_glue(spec);
                self.set_link(k, inner);
                let outer = self.new_null_box();
                self.set_list_ptr(outer, k);
                self.set_width(outer, sp(l.maxx) - sp(l.minx));
                self.set_height(outer, sp(l.maxy) - sp(l.miny));
                self.set_depth(outer, 0);
                if self.session_library_only() {
                    // What `box_end` would raise it by is for a line it stands in, and here the page is the box.
                    self.set_box_reg(self.v_image.cd.session_box, outer);
                } else {
                    self.cur_box = outer;
                    self.box_end(sp(l.base) - sp(l.miny));
                }
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

/// `<r> <g> <b> rg ...` and its like, as xcolor writes them.
pub(crate) fn native_color_of_operators(s: &[u8]) -> Option<Color> {
    let text = core::str::from_utf8(s).ok()?;
    let w: Vec<&str> = text.split_ascii_whitespace().collect();
    let num = |x: &str| x.parse::<f64>().ok();
    match w.as_slice() {
        [g, "g", ..] => Some(Color::Gray(num(g)?)),
        [r, g, b, "rg", ..] => Some(Color::Rgb(num(r)?, num(g)?, num(b)?)),
        [c, m, y, k, "k", ..] => Some(Color::Cmyk(num(c)?, num(m)?, num(y)?, num(k)?)),
        _ => None,
    }
}

/// What the library asks of the document's keys, spec 5.1.  Anything that is not as TikZ and tikz-cd define it, by the hashes of `native_table::KEYS`, is the document's: the library gets its text.
struct DocKeys<'a>(&'a Tex);

/// A name for `find_cs`, on the stack: a diagram asks this for each of its keys.
fn native_key_name<'b>(buf: &'b mut [u8; 256], dir: &[u8], key: &[u8], tail: &[u8]) -> Option<&'b [u8]> {
    let n = dir.len() + key.len() + tail.len();
    if n > buf.len() {
        return None;
    }
    buf[..dir.len()].copy_from_slice(dir);
    buf[dir.len()..dir.len() + key.len()].copy_from_slice(key);
    buf[dir.len() + key.len()..n].copy_from_slice(tail);
    Some(&buf[..n])
}

const KEY_DIRS: [(&[u8], UserKey); 2] = [(b"pgfk@/tikz/commutative diagrams/", UserKey::Cd), (b"pgfk@/tikz/", UserKey::Tikz)];

impl UserKeys for DocKeys<'_> {
    fn user_key(&self, name: &[u8], out: &mut Vec<u8>) -> UserKey {
        let t = self.0;
        let mut buf = [0u8; 256];
        // tikz-cd's `.unknown` takes these for a direction before it looks in `/tikz`.
        let direction = !name.is_empty() && name.iter().all(|c| matches!(c, b'r' | b'd' | b'l' | b'u'));
        for (dir, kind) in &KEY_DIRS[..if direction { 1 } else { 2 }] {
            let kind = *kind;
            let Some(full) = native_key_name(&mut buf, dir, name, b"/.@cmd") else { return UserKey::Standard };
            let cs = t.find_cs(full);
            if cs == UNDEFINED_CONTROL_SEQUENCE {
                continue;
            }
            let ty = t.eq_type(cs) & !ARMED_BITS;
            if !(CALL..END_TEMPLATE).contains(&ty) {
                continue;
            }
            let ptr = t.equiv(cs);
            if native_table::is_stock_key(t.native_macro_hash(cs, ptr)) {
                return UserKey::Standard;
            }
            // quiver's `curve` is `settings` and a path that reads these, which the library takes as they are when the package loads.
            if name == b"settings" && !(t.native_value_is(b"pgfk@/tikz/quiver/pos", b"0.35") && t.native_value_is(b"pgfk@/tikz/quiver/height", b"0")) {
                out.extend_from_slice(b"\\relax");
                return kind;
            }
            // `#1\pgfeov`, or the code of a form that takes its value otherwise, which the library won't recognise.
            let pgfeov = t.find_cs(b"pgfeov");
            if t.toks[ptr + 1] >> 8 == MATCH && t.toks[ptr + 2] == CS_TOKEN_FLAG + pgfeov && t.toks[ptr + 3] == END_MATCH_TOKEN {
                t.native_style_text(ptr + 4, out);
            } else {
                out.extend_from_slice(b"\\relax");
            }
            return kind;
        }
        UserKey::Standard
    }

    fn default_value(&self, name: &[u8], out: &mut Vec<u8>) -> bool {
        let t = self.0;
        let mut buf = [0u8; 256];
        for (dir, _) in KEY_DIRS {
            let Some(full) = native_key_name(&mut buf, dir, name, b"/.@def") else { return false };
            let cs = t.find_cs(full);
            if cs == UNDEFINED_CONTROL_SEQUENCE {
                continue;
            }
            if !(CALL..END_TEMPLATE).contains(&(t.eq_type(cs) & !ARMED_BITS)) || t.toks[t.equiv(cs) + 1] != END_MATCH_TOKEN {
                return false;
            }
            t.native_style_text(t.equiv(cs) + 2, out);
            return true;
        }
        false
    }

    fn color(&self, name: &[u8]) -> Option<Color> {
        // xcolor makes a colour its target model's when it is used.
        let c = self.0.native_named_color(name)?;
        match (self.0.native_color_model()?.as_slice(), c) {
            (b"", _) | (b"gray", Color::Gray(..)) | (b"rgb", Color::Rgb(..)) | (b"cmyk", Color::Cmyk(..)) => Some(c),
            _ => None,
        }
    }

    fn base_colors(&self) -> bool {
        let t = self.0;
        t.native_color_model().is_some_and(|m| m.is_empty()) && commute::base_colors().all(|(name, c)| t.native_named_color(name) == Some(c))
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
    /// `PdfSink::box_color`'s tokens for the next box.
    box_color: i32,
}

impl<'a> Sink<'a> {
    fn new(t: &'a mut Tex, cells: &'a [i32], labels: &'a [i32]) -> Sink<'a> {
        let mut s = Sink { t, first: NULL, seg: NULL, tail: NULL, cells, labels, box_color: NULL };
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

    fn literal_tokens(&mut self, ops: &[u8]) -> i32 {
        let t = &mut *self.t;
        let m = t.tb_mark();
        for &c in ops {
            t.tb_push(if c == b' ' { SPACE_TOKEN } else { OTHER_TOKEN + c as i32 });
        }
        t.tb_commit(m)
    }

    /// §1539: the node of a `\\pdfcolorstack`.
    fn stack_node(&mut self, cmd: i32, data: i32) -> i32 {
        let t = &mut *self.t;
        let n = t.get_node(if cmd == COLORSTACK_POP { PDF_COLORSTACK_GETTER_NODE_SIZE } else { PDF_COLORSTACK_SETTER_NODE_SIZE });
        t.set_ty(n, WHATSIT_NODE);
        t.set_subtype(n, PDF_COLORSTACK_NODE);
        t.set_link(n, NULL);
        let stack = t.host.hv.native.color_stack;
        t.set_pdf_colorstack_stack(n, stack);
        t.set_pdf_colorstack_cmd(n, cmd);
        if cmd != COLORSTACK_POP {
            t.set_pdf_colorstack_data(n, data);
        }
        n
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
        // pdfTeX ends a literal with a newline of its own.
        let ops = ops.strip_suffix(b"\n").unwrap_or(ops);
        if ops.is_empty() {
            return;
        }
        let p = self.literal_tokens(ops);
        let t = &mut *self.t;
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
        if self.box_color != NULL {
            // A real node's `\\color` is inside its box, around what TeX has put there.
            let (push, pop) = (self.stack_node(COLORSTACK_PUSH, self.box_color), self.stack_node(COLORSTACK_POP, NULL));
            self.box_color = NULL;
            let first = self.t.list_ptr(b);
            self.t.set_list_ptr(b, push);
            self.t.set_link(push, first);
            let mut last = push;
            while self.t.link(last) != NULL {
                last = self.t.link(last);
            }
            self.t.set_link(last, pop);
        }
        self.t.set_link(b, NULL);
        self.push(b);
        let k = self.t.new_kern(-w);
        self.push(k);
    }

    fn color_push(&mut self, ops: &[u8]) {
        let p = self.literal_tokens(ops);
        let n = self.stack_node(COLORSTACK_PUSH, p);
        self.push(n);
    }

    fn color_pop(&mut self) {
        let n = self.stack_node(COLORSTACK_POP, NULL);
        self.push(n);
    }

    fn box_color(&mut self, ops: &[u8]) {
        self.box_color = self.literal_tokens(ops);
    }

    fn tex_positions(&self) -> bool {
        true
    }

    fn layers(&self) -> (u32, u32) {
        self.t.host.hv.native.layers
    }

    fn position(&mut self, x: i64, y: i64) {
        self.segment(x as i32, y as i32);
    }
}
