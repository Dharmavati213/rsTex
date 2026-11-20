//! A memo of expansions.  An `\expanded` that stands in a macro's body, §496's `expanded_code` with a brace after it, expands what its braces
//! hold and nothing else, and what it reads of the engine can be listed.  So such a call is recorded, and a later call from the same place with
//! the same arguments is replayed, when the replay can prove that the real path would do what it did.  It began as a memo of l3text's case
//! change, whence the name: `\MakeUppercase` comes to `\exp_not:e`, which is `\unexpanded\expanded{{#1}}`, and runs l3text's pipeline token by
//! token, some 0.4 M instructions for each character of the result, on every page for the same running head.  The other large case is
//! lthooks' `\exp_args:Neee`, which doubles the hashes of a hook's code token by token each time the code is added, as hyperref does at each theorem.
//!
//! What the real path reads, and so what has to be the same for a replay:
//!
//! - **The input.**  The key is the place and the arguments: the body the call stands in, the offset of the brace in it, and every argument
//!   of that macro, compared token by token.  An entry keeps a reference to the body, so that the same place means the same tokens.  What the
//!   call reads of the body, past its own closing brace too, where an `\expandafter` before that brace reaches, is then the same, and the
//!   body counts among those that ran, below.  A read below that level stops a recording: the hook where a macro's level ends sees the level
//!   the call stands in go.  That hook has no test of its own: while a call records, `interrupt` holds `CASE_MEMO_INTERRUPT`, which sends
//!   every level that ends through `end_token_list`'s cold path, where an interrupt goes.
//! - **Every meaning it can read.**  Every control sequence in the arguments, in the body of each macro that ran (the hook where its level
//!   ends: `macro_call` is the one place that begins a macro's level, and `end_token_list` the one that ends it while a call records), in the
//!   list of a `\toks` that `\the` gave, and each `\csname` and `\ifcsname` found: its `eq_type` and `equiv`.  A macro's body has a reference
//!   held from here, so that the same place means the same tokens, as `native_font.rs` holds its own; a macro defined again with the tokens
//!   it had is the macro it was, by a comparison with that body.  Every token the call reads comes from one of those lists, or is a character
//!   of catcode 10 or 12 that `str_toks` made, the identifier of a font that `\the` gave, or one of the frozen control sequences that §405
//!   and §369 put in.
//! - **Less of a macro that did not run.**  A macro whose body is none of those that ran is read for its `eq_type`, which the scanner and
//!   `\noexpand` look at, and for whether it is protected, §504's test of the first token of its body; so the closure holds those two of it
//!   and no body, and it may be defined as anything else of that kind.  Two things read a body without running it.  `\ifx` of two macros,
//!   §534, reads whether their bodies are the same tokens: that answer is a read, of both names, looked up again for a replay.  `\meaning`
//!   shows the body: its macro is held to its body as one that ran.  A macro that shares its body with one that ran, as after a `\let`, is
//!   held to it too, which is more than is needed.
//! - **Every value it can read.**  A register or parameter named by such a control sequence (`assign_int`, `assign_dimen`), and
//!   what `scan_internal_others` reads (`\catcode` and its kin, `\count`, `\dimen`, `\toks`, `\fontdimen`, `\hyphenchar`, `\skewchar`), the font of `\font` and
//!   `\textfont` and of the units `em` and `ex`, `\pdfpxdimen` for `px`, `\escapechar` for `print_esc`, the `\catcode` that `print_cs`
//!   tests for a name of one character, the name of a font's identifier that `print_cs` and `sprint_cs` print, which a later `\font`
//!   of the same font and size changes, the mode for `\ifvmode` and its kin, and `par_token`, which a macro that is not `\long`
//!   tests each token of its arguments against.  Names `\ifcsname` did not find are looked up again once a name has been made since.
//! - **Where it stands in an alignment.**  An `&`, `\span` or `\cr` it reads is a token unless `align_state` is 0 there, and the capture
//!   loops of `capture.rs` pass one with no test where it cannot be 0 from where they start, which they do not tell.  So a call whose
//!   lists hold such a token, a character of catcode 4 or a control sequence that means one, as l3text's `\__text_char_catcode:N` holds
//!   `\c_alignment_token`, is replayed only where `align_state` is what it was when the call was recorded, and then every token it reads
//!   has the `align_state` it had.  At another `align_state` the key has another entry.
//! - **Nothing else.**  Anything outside this list stops a recording, which then leaves no entry and changes nothing: marks,
//!   `\input` and `\scantokens`, `\pdfprimitive`, a glue, a box, `\ifeof`, `\iffontchar`, `\jobname`, `true` units, any other
//!   convert command but `\number`, `\romannumeral`, `\string`, `\meaning`, `\expanded`, `\pdfstrcmp` and the four escapes, any other
//!   internal quantity, the clock and the line number among them, an outer or armed name, a name made, an error, an `&` or `\cr` where
//!   `align_state` is 0, and a `\fi` or `\else` read while a condition that began before the call is evaluated, which reads its `if_limit`.
//!
//! What it does, and how a replay does the same:
//!
//! - **Condition and expression nodes** come from `get_node` and go to `free_node` in the order recorded, with the link of each
//!   written as the real path writes it, and the two words of an expression node that outlive it (its `t` and `n`, halves `rh`).
//!   The rest of each node's words are what `free_node` writes over before anything reads them.  A pop calls `if_warning` where the
//!   real `cond_pop` would.  Pushes are hooked in `conditional` and `scan_expr`, pops in `scan_expr`, and a condition's pops are
//!   counted from the depth of the stack at the next event.
//! - **The stacks' marks** rise by what the call needed above where it began: `max_in_stack`, `max_param_stack` and
//!   `max_buf_stack` are lowered to where the call stands while it records, and the expansion depth is watched by its limit, so
//!   each costs nothing outside a recording; the pool and the strings are watched where `str_toks` and `make_string` take them,
//!   and main memory where a macro's level ends, by what its arguments and those below hold.  A replay that would go past a
//!   capacity runs the real path, which stops there, and so does one where `webmem.rs`'s coarse bound has less room than twice what
//!   the call held at once.
//! - **The result** is the list `scan_toks` builds, in a block of its own; `cur_cmd`, `cur_chr`, `cur_cs` and `cur_tok` as the
//!   last `}` left them; `align_state` moved as the braces move it; `loc` where the call left it; `font_info[fmem_ptr]` cleared if a
//!   `\fontdimen` was read.  `cur_val`, `cur_val_level`, `radix` and `cur_order` are put back by `expand_saving`, §388, around `conv_toks`.
//! - **Scratch** that the real path writes and nothing reads before writing it again: `long_state`, `tb_match_mark`, `pstack`,
//!   `skip_line`, `tally`, `old_setting`, `last_tokens_string`, `base_ptr`, and what lies above the tops of `buffer`, `str_pool`,
//!   `str_start`, the input and parameter stacks, the builder and the transient stack.
//!
//! An entry whose reads no longer hold stays, for where they hold again, as a switch that a macro sets and clears does, and the key may get
//! another beside it, up to `VARIANTS_MAX`.  When the entries or their cells run out, all go.
//!
//! What is worth it.  A recording costs about a fifth of the call more than the call, and making its entry some 15,000 instructions; a
//! replay costs a fiftieth of a large call, and for a call of a few macros its checks are no clear gain.  Most calls of a document come once, and most are
//! small, a preamble's above all.  So:
//!
//! - **Credit.**  A recording is paid for from a credit that each page shipped adds two recordings to, and each replay what it saved, in
//!   macros run.  With no credit and no entry a call is left alone before its key is looked at, so a document of one page, or one whose
//!   calls never come again, pays for a test and no more.
//! - **The second call.**  A key is recorded at its second call, which `seen`, the hashes of keys called once, tells, or at its first
//!   while the replays at its place have paid for the recordings there, `RECORDINGS_PER_REPLAY` of them for each, as they have at a
//!   place with no recording yet.
//! - **Small calls.**  A call that ran fewer than `WORK_MIN` macros gets no entry, and its key is in `never`, with those whose recording
//!   stopped; a place whose calls were `SMALL_MAX` times too small, with few replays against that, is left alone.
//!
//! All of that is in the image, so a run from a checkpoint or a page record decides as the run that wrote it would have.
//!
//! None of it runs with tracing that would print, with an interrupt waiting, inside `\csname`, with `webmem.rs` in its fine mode or with
//! less room than the call took, or under `-ini`, so a format holds nothing of it.  The token heap comes out with other blocks free than the
//! real path would have left, the case of docs/PORTING.md section 15 that `native_font.rs` has too.
//!
//! `RSTEX_NO_CASE_MEMO` runs the real path, and is in a checkpoint's key.  `RSTEX_CASE_MEMO_CHECK` is a test of it, also in the key: it
//! records every first call, and where the memo would replay, the real path runs under the recorder, and the entry it would make has to
//! be the one there, but where that was, the bound on main memory, which depends on the heap, and a macro held to its body in one and to
//! nothing in the other; a difference is said on the terminal's error stream.  `RSTEX_CASE_MEMO_EAGER`, in the key too, asks no credit of
//! a recording, so that a test of one page has replays.  `RSTEX_PF_STATS` says at the end what became of the calls.
#![forbid(unsafe_code)]
#![allow(non_snake_case, unused_mut, unused_imports, dead_code)]

use crate::access::*;
use crate::consts::*;
use crate::host::*;
use crate::raw::*;
use crate::state::*;

/// What `interrupt` holds while a call records, so that each level that ends goes through `end_token_list`'s cold path and its
/// hook, with no test of its own on the hot ones.  A real interrupt is 1, `platform::catch_interrupt`'s.
pub const CASE_MEMO_INTERRUPT: i32 = -1;

/// `Vars::mode`: off, on, and `RSTEX_CASE_MEMO_CHECK`.
pub const MODE_OFF: i32 = 0;
pub const MODE_ON: i32 = 1;
pub const MODE_CHECK: i32 = 2;
/// `RSTEX_CASE_MEMO_EAGER`, for the tests: on, with no credit asked of a recording, so that one page of calls shows replays.
pub const MODE_EAGER: i32 = 3;

/// How many keys, and the cells they share.
const ENTRIES: usize = 128;
const ARENA: usize = crate::state::CM_ARENA_CAP;
const OPS: usize = crate::state::CM_OPS_CAP;
/// The longest key and result kept.
const KEY_CAP: i32 = 4096;
/// The deepest nesting of expressions kept.
const EXPR_CAP: usize = 64;
/// The keys called once that `seen` remembers, direct mapped.
const SEEN: usize = 256;
/// How many recordings a replay pays for: a recording takes about a fifth of a call more than the call, and a replay saves nearly
/// all of one.
const RECORDINGS_PER_REPLAY: i32 = 4;
/// The bodies a recording has seen lately, direct mapped, so that the closure's list of them stays short.
const BODY_CACHE: usize = 1024;
/// The keys that are not to be recorded again, direct mapped: a recording of one stopped, or the call was too small to gain by.
const NEVER: usize = 1024;
/// The places an `\\expanded` stands at, direct mapped, each with what became of its calls.
const SITES: usize = 64;
/// The fewest macros a call has to run for its entry to be kept: below that a replay's checks are no clear gain on the call.
const WORK_MIN: i32 = 12;
/// A place whose calls were this often too small, with few replays against it, is left alone.
const SMALL_MAX: i32 = 16;
/// What a recording costs of the credit, in macros run, beside a quarter of those the call runs; what each page shipped gives, which
/// is two recordings; and the most that is kept.
const RECORD_COST: i32 = 64;
const PAGE_CREDIT: i32 = 128;
const CREDIT_MAX: i32 = 1 << 20;
/// How many entries a key may have, each for another state of what the call reads.
const VARIANTS_MAX: i32 = 4;

/// In place of a macro's body in the closure: any body will do, of a macro that is not protected, and of one that is.
const ANY_BODY: i32 = -1;
const ANY_PROTECTED: i32 = -2;

const OP_COND_PUSH: u8 = 0;
const OP_COND_POP: u8 = 1;
/// An expression node whose link is null, the first of its `scan_expr`.
const OP_EXPR_PUSH: u8 = 2;
/// One whose link is the node below it.
const OP_EXPR_PUSH_LINKED: u8 = 3;
const OP_EXPR_POP: u8 = 4;

/// What a read reads: `eqtb[loc].rh`, `mem[loc].rh`, `font_info[loc].sc`, those two for a token list with a reference held here, the
/// mode, the `\hyphenchar` and `\skewchar` of font `loc`, which expl3's integer arrays keep their sizes in, `par_token`, and the text
/// of control sequence `loc`, a font's identifier.
pub const RD_EQTB: i32 = 0;
pub const RD_MEM: i32 = 1;
pub const RD_FONT: i32 = 2;
pub const RD_EQTB_LIST: i32 = 3;
pub const RD_MEM_LIST: i32 = 4;
pub const RD_MODE: i32 = 5;
pub const RD_HYPHEN_CHAR: i32 = 6;
pub const RD_SKEW_CHAR: i32 = 7;
pub const RD_PAR_TOKEN: i32 = 8;
pub const RD_TEXT: i32 = 9;
/// Whether the bodies of two macros are the same tokens, which is what `\\ifx` reads of them: `loc` is the one, and the value twice the
/// other, and one more if they were.
pub const RD_IFX: i32 = 10;

/// An entry's header in the arena; its parts follow in this order: the key, the result, the closure as triples of control
/// sequence, `eq_type` and `equiv`, the reads as triples, the names not found, each its length and then its bytes four to a cell, the
/// expression nodes' words as pairs.  The operations are in `ops`.
const H_KEY: i32 = 0;
const H_RES: i32 = 1;
const H_CL: i32 = 2;
const H_RD: i32 = 3;
const H_NM: i32 = 4;
const H_NM_CELLS: i32 = 5;
const H_EX: i32 = 6;
const H_OP_AT: i32 = 7;
const H_OP_N: i32 = 8;
const H_LOC: i32 = 9;
const H_ALIGN: i32 = 10;
const H_CMD: i32 = 11;
const H_CHR: i32 = 12;
const H_CS: i32 = 13;
const H_TOK: i32 = 14;
/// How far above where the call began it took the input stack (-1 for not at all), the parameter stack, the buffer (-1 for not at
/// all), the expansion depth, the string pool and the strings; and a bound on the cells of main memory it had in use at once.
const H_IN: i32 = 15;
const H_PARAM: i32 = 16;
const H_BUF: i32 = 17;
const H_EXP: i32 = 18;
const H_POOL: i32 = 19;
const H_STR: i32 = 20;
const H_MEM: i32 = 21;
/// Whether a `\fontdimen` was read; `hash_high` when the names not found were last known not to be there.
const H_FD: i32 = 22;
const H_HH: i32 = 23;
/// Whether the call can read an `&`, `\span` or `\cr`, and if so `align_state` where it began, which a replay then needs, and otherwise 0.
const H_TABS: i32 = 24;
const H_ALIGN0: i32 = 25;
/// The macros the call ran: what a replay saves, and so what it adds to the credit.
const H_WORK: i32 = 26;
const H_SIZE: i32 = 27;

crate::pod_struct! {
    /// In the image, under `v_image`, so that a checkpoint and a page record carry it with the references it holds.
    pub struct Vars {
        /// `MODE_ON`, `MODE_OFF` for `RSTEX_NO_CASE_MEMO` or `-ini`, `MODE_CHECK`.  In a checkpoint's key, as the driver's `Options` is.
        pub mode: i32,
        /// Not 0 while a call records: what each hook tests.
        pub rec: i32,
        /// Calls of the shape, replayed, recorded, stopped, not tried for the context, an entry that did not hold, arena emptied, no room
        /// for a replay, first calls left to the real path, checked, check failed.  Before the arena, with `seen`, so that what a call
        /// changes of the image is one page of a page record.
        pub stats: A<i32, 16>,
        /// What recordings may still cost: pages shipped and replays add to it, recordings take from it, so that a document whose calls
        /// do not come again pays for few.  `pages` is `total_pages` when it was last added to, and `live` the entries there are.
        pub credit: i32,
        pub pages: i32,
        pub live: i32,
        /// The entries: a hash of the key, 0 for none, and where the header is.
        pub e_hash: A<i32, ENTRIES>,
        pub e_at: A<i32, ENTRIES>,
        /// The hashes of keys called once and not recorded, 0 for none.
        pub seen: A<i32, SEEN>,
        /// The hashes of keys not to be recorded again, 0 for none.
        pub never: A<i32, NEVER>,
        /// The places: a hash of the body and the offset in it, and how many of its calls were recorded, replayed and too small.
        pub site_h: A<i32, SITES>,
        pub site_rec: A<i32, SITES>,
        pub site_rep: A<i32, SITES>,
        pub site_small: A<i32, SITES>,
        /// What `Tex::cm_arena` and `Tex::cm_ops` hold so far, where the entries are: arrays of their own, so that an image has of them
        /// what they hold and no more.
        pub top: i32,
        pub ops_top: i32,
    }
}

/// What a recording keeps until it ends: nothing of it outlives the call, which is inside one `expand`, so no image holds it.
#[derive(Default)]
pub struct Rec {
    pub abort: Option<&'static str>,
    /// Under `MODE_CHECK`, the slot of the entry this recording is held against.
    pub check: Option<i32>,
    /// The key, and the arguments it is made of, for the closure.
    pub key_cells: Vec<i32>,
    pub keys: Vec<i32>,
    pub key_hash: i32,
    pub site: i32,
    /// The macros whose level ended in the call: how much work it was.
    pub levels: i32,
    pub block: i32,
    pub loc0: i32,
    pub in0: i32,
    pub floor: i32,
    pub param0: i32,
    pub cond0: i32,
    pub var0: i32,
    pub errors0: i32,
    pub history0: i32,
    pub interaction0: i32,
    pub first0: i32,
    pub pool0: i32,
    pub str0: i32,
    pub tb0: i32,
    pub tx0: i32,
    pub hash0: i32,
    pub fmem0: i32,
    pub save0: i32,
    pub nest0: i32,
    pub level0: i32,
    pub align0: i32,
    pub obs0: usize,
    pub term0: u64,
    pub offsets0: (i32, i32),
    pub cur_ptr0: i32,
    pub exp0: i32,
    pub saved_in: i32,
    pub saved_param: i32,
    pub saved_buf: i32,
    pub real_expand_depth: i32,
    pub peak_exp: i32,
    pub peak_pool: i32,
    pub peak_str: i32,
    pub peak_mem: i64,
    pub bodies: Vec<i32>,
    /// What `bodies` has had lately, by a hash of the block.
    pub body_seen: Vec<i32>,
    /// What the arguments above the floor hold, summed from the floor up: one for each that has been seen on the parameter stack, which
    /// only a level's end takes from, here.
    pub held: Vec<i64>,
    pub extra_cs: Vec<i32>,
    /// Macros whose body was read though they did not run: `\\ifx` of two macros, and `\\meaning`.
    pub strict: Vec<i32>,
    pub lists: Vec<i32>,
    pub reads: Vec<(i32, i32, i32)>,
    pub names: Vec<Vec<u8>>,
    pub ops: Vec<u8>,
    pub expr: Vec<(i32, i32)>,
    pub expr_stack: Vec<i32>,
    pub cond_depth: i32,
    pub fd_read: bool,
    /// Whether §364 has seen an `&`, `\span` or `\cr`.
    pub tab: bool,
}

/// Non-POD state, in `self.host`.
#[derive(Default)]
pub struct HVars {
    pub r: Rec,
    /// The key of the call at hand: the body, the offset of the brace in it, the number of arguments, and each argument as its length and its tokens.
    pub key: Vec<i32>,
    /// For `RSTEX_PF_STATS`: why recordings stopped, in this process.
    pub reasons: Vec<(&'static str, u32)>,
}

/// An entry made of a recording, before it goes into the arena: its cells with `H_OP_AT` 0, its operations, and the blocks it holds.
struct Entry {
    cells: Vec<i32>,
    ops: Vec<u8>,
    refs: Vec<i32>,
}

#[inline]
fn key_hash(toks: &[i32]) -> i32 {
    let mut h = 0x811c_9dc5u32;
    for &t in toks {
        h = (h ^ t as u32).wrapping_mul(0x0100_0193);
    }
    (h | 1) as i32
}

impl Tex {
    /// `\expanded` in a macro's body, at the brace: true if this has done what `scan_pdf_ext_toks` would, by a replay or by
    /// running it under the recorder.  The key is the place, which is the body and the offset in it, and the macro's arguments.
    #[inline(never)]
    pub fn case_memo_expanded(&mut self) -> bool {
        if self.state() != TOKEN_LIST || self.token_type() != MACRO {
            return false;
        }
        let b = self.start();
        let loc = self.loc();
        let t = self.toks[loc];
        if t < LEFT_BRACE_TOKEN || t >= LEFT_BRACE_LIMIT {
            return false;
        }
        let ps = self.param_start();
        let np = self.param_ptr - ps;
        if !(0..=9).contains(&np) {
            return false;
        }
        self.v_image.cm.stats[0] += 1;
        {
            let pages = self.total_pages;
            let v = &mut self.v_image.cm;
            if pages != v.pages {
                v.credit = (v.credit as i64 + (pages - v.pages).max(0) as i64 * PAGE_CREDIT as i64).min(CREDIT_MAX as i64) as i32;
                v.pages = pages;
            }
            if v.mode == MODE_ON && v.live == 0 && v.credit < RECORD_COST {
                // Nothing to replay and nothing to record with.
                v.stats[15] += 1;
                return false;
            }
        }
        // The place: one whose calls are too small is left alone.
        let sh = ((b as u32).wrapping_mul(0x9e37_79b9) ^ ((loc - b) as u32).wrapping_mul(0x85eb_ca6b)) | 1;
        let site = (sh >> 1) as i32 % SITES as i32;
        {
            let v = &mut self.v_image.cm;
            if v.site_h[site] != sh as i32 {
                v.site_h[site] = sh as i32;
                v.site_rec[site] = 0;
                v.site_rep[site] = 0;
                v.site_small[site] = 0;
            } else if v.mode != MODE_CHECK && v.site_small[site] >= SMALL_MAX && v.site_rep[site] * 4 < v.site_small[site] {
                v.stats[14] += 1;
                return false;
            }
        }
        if !self.cm_context() {
            self.v_image.cm.stats[4] += 1;
            return false;
        }
        let mut key = std::mem::take(&mut self.host.hv.case_memo.key);
        key.clear();
        key.extend_from_slice(&[b, loc - b, np]);
        // Every argument whole, or no key: a key cut short would be the key of another call too.
        let mut whole = true;
        for k in 0..np {
            let p = self.param_stack[ps + k];
            if p == NULL {
                key.push(0);
            } else {
                let n = self.tok_len(p);
                if key.len() as i32 + 1 + n > KEY_CAP {
                    whole = false;
                    break;
                }
                key.push(n);
                key.extend_from_slice(self.toks.range(p + 1, p + 1 + n));
            }
        }
        let done = whole && {
            let h = key_hash(&key);
            self.cm_look(&key, h, site)
        };
        self.host.hv.case_memo.key = key;
        done
    }

    /// The call whose key is `key`: replayed from its entry, or recorded, or left to the real path, which is false.
    fn cm_look(&mut self, key: &[i32], h: i32, site: i32) -> bool {
        let n = key.len() as i32;
        let never = (h as u32 >> 1) as i32 % NEVER as i32;
        if self.v_image.cm.never[never] == h {
            self.v_image.cm.stats[11] += 1;
            return false;
        }
        let mut variants = 0;
        for k in 0..ENTRIES as i32 {
            if self.v_image.cm.e_hash[k] == h {
                let at = self.v_image.cm.e_at[k];
                if self.cm_arena[at + H_KEY] == n && self.cm_arena.range(at + H_SIZE, at + H_SIZE + n) == key {
                    if self.cm_arena[at + H_TABS] != 0 && self.cm_arena[at + H_ALIGN0] != self.align_state {
                        // Recorded where the alignment stood elsewhere: an entry of its own is for here.
                        continue;
                    }
                    if self.cm_holds(at) {
                        if !self.cm_room(at) {
                            self.v_image.cm.stats[7] += 1;
                            return false;
                        }
                        if self.v_image.cm.mode == MODE_CHECK {
                            self.cm_record(key, h, site, Some(k));
                        } else {
                            self.cm_replay(at);
                            let work = self.cm_arena[at + H_WORK];
                            let v = &mut self.v_image.cm;
                            v.stats[1] += 1;
                            v.site_rep[site] += 1;
                            v.credit = (v.credit as i64 + work as i64).min(CREDIT_MAX as i64) as i32;
                        }
                        return true;
                    }
                    // Something it read is not what it was.  The entry stays, for where that is as it was again, as a switch that
                    // a macro sets and clears is; the call may get another entry beside it, and a key with as many as it may have is left alone.
                    self.v_image.cm.stats[5] += 1;
                    variants += 1;
                }
            }
        }
        if variants >= VARIANTS_MAX {
            self.v_image.cm.stats[13] += 1;
            return false;
        }
        if !self.cm_worth_recording(h, site) {
            self.v_image.cm.stats[8] += 1;
            return false;
        }
        self.cm_record(key, h, site, None);
        true
    }

    /// Whether to record a key that is not in the memo: at its second call, or at its first while the replays at its place have
    /// paid for what was recorded there.  If not, `seen` keeps its hash.  `MODE_CHECK` records every first call, so that every second one is checked.
    fn cm_worth_recording(&mut self, h: i32, site: i32) -> bool {
        let v = &mut self.v_image.cm;
        if v.mode == MODE_CHECK {
            return true;
        }
        let slot = (h as u32 >> 1) as i32 % SEEN as i32;
        if v.credit < RECORD_COST && v.mode == MODE_ON {
            v.seen[slot] = h;
            return false;
        }
        if v.seen[slot] == h {
            v.seen[slot] = 0;
            return true;
        }
        if v.site_rep[site] as i64 * RECORDINGS_PER_REPLAY as i64 >= v.site_rec[site] as i64 {
            return true;
        }
        v.seen[slot] = h;
        false
    }

    /// The key at hand is not to be recorded again.
    fn cm_never(&mut self, h: i32) {
        let never = (h as u32 >> 1) as i32 % NEVER as i32;
        self.v_image.cm.never[never] = h;
    }

    /// What decides at this place for a recording and a replay alike: nothing would be printed, nothing is waiting, the stops of
    /// main memory are far.
    #[inline]
    fn cm_context(&mut self) -> bool {
        self.tracing_macros() <= 0
            && self.tracing_commands() <= 1
            && self.tracing_ifs() <= 0
            && self.tracing_nesting() <= 0
            && self.interrupt == 0
            && !self.arith_error
            && self.csname_depth == 0
            && self.v_webmem.fine == 0
            && self.max_buf_stack < self.buf_size
            && self.first < self.buf_size
    }

    // -----------------------------------------------------------------------------------------------------------------------------
    // A replay.
    // -----------------------------------------------------------------------------------------------------------------------------

    #[inline]
    fn cm_parts(&self, at: i32) -> (i32, i32, i32, i32, i32) {
        let a = &self.cm_arena;
        let res = at + H_SIZE + a[at + H_KEY];
        let cl = res + a[at + H_RES];
        let rd = cl + 3 * a[at + H_CL];
        let nm = rd + 3 * a[at + H_RD];
        let ex = nm + a[at + H_NM_CELLS];
        (res, cl, rd, nm, ex)
    }

    /// The value a read would have now.
    #[inline]
    fn cm_value(&self, kind: i32, loc: i32) -> i32 {
        match kind {
            RD_EQTB | RD_EQTB_LIST => self.eqtb[loc].rh(),
            RD_MEM | RD_MEM_LIST => self.mem[loc].rh(),
            RD_FONT => self.font_info[loc].sc(),
            RD_HYPHEN_CHAR => self.hyphen_char[loc],
            RD_SKEW_CHAR => self.skew_char[loc],
            RD_PAR_TOKEN => self.par_token,
            RD_TEXT => self.text(loc),
            _ => self.mode(),
        }
    }

    /// A macro defined again with the tokens it had is the macro it was: `old` is the body the entry holds a reference to.
    #[inline(never)]
    fn cm_same_body(&self, eq_type: i32, new: i32, old: i32) -> bool {
        let t = eq_type & !crate::native::ARMED_BITS;
        if t < CALL || t >= END_TEMPLATE || new == NULL || old == NULL {
            return false;
        }
        self.cm_tok_eq(new, old)
    }

    /// Whether two lists are the same tokens.
    fn cm_tok_eq(&self, new: i32, old: i32) -> bool {
        let mut i = 1;
        loop {
            let x = self.toks[new + i];
            if x != self.toks[old + i] {
                return false;
            }
            if x == 0 {
                return true;
            }
            i += 1;
        }
    }

    /// Whether all the entry read is as it was.
    fn cm_holds(&mut self, at: i32) -> bool {
        let (_, cl, rd, nm, _) = self.cm_parts(at);
        let a = &self.cm_arena;
        for i in 0..a[at + H_CL] {
            let p = cl + 3 * i;
            let w = self.eqtb[a[p]];
            let want = a[p + 2];
            let same = if want <= ANY_BODY {
                w.rh() != NULL && (self.toks[w.rh() + 1] == PROTECTED_TOKEN) == (want == ANY_PROTECTED)
            } else {
                w.rh() == want || self.cm_same_body(w.b0(), w.rh(), want)
            };
            if w.b0() != a[p + 1] || !same {
                return false;
            }
        }
        for i in 0..a[at + H_RD] {
            let p = rd + 3 * i;
            if a[p] == RD_IFX {
                let (x, y) = (self.eqtb[a[p + 1]].rh(), self.eqtb[a[p + 2] >> 1].rh());
                let same = x == y || x != NULL && y != NULL && self.cm_tok_eq(x, y);
                if same as i32 != a[p + 2] & 1 {
                    return false;
                }
                continue;
            }
            if self.cm_value(a[p], a[p + 1]) != a[p + 2] {
                return false;
            }
        }
        // `\ifcsname` makes no name, and any other new name moves `hash_high`.
        if a[at + H_NM] > 0 && a[at + H_HH] != self.hash_high {
            let mut p = nm;
            for _ in 0..a[at + H_NM] {
                let len = a[p];
                let mut name = [0u8; 256];
                for j in 0..len {
                    name[j as usize] = (a[p + 1 + j / 4] >> (8 * (j % 4))) as u8;
                }
                if self.find_cs(&name[..len as usize]) != UNDEFINED_CONTROL_SEQUENCE {
                    return false;
                }
                p += 1 + (len + 3) / 4;
            }
            let hh = self.hash_high;
            self.cm_arena[at + H_HH] = hh;
        }
        true
    }

    /// Whether the real path would have stayed inside every capacity, and `webmem.rs` far from its stops.
    fn cm_room(&self, at: i32) -> bool {
        let a = &self.cm_arena;
        let up_in = a[at + H_IN];
        let up_buf = a[at + H_BUF];
        (up_in < 0 || self.input_ptr + up_in < self.stack_size)
            && self.param_ptr + a[at + H_PARAM] <= self.param_size
            && (up_buf < 0 || self.first + up_buf < self.buf_size)
            && self.expand_depth_count + a[at + H_EXP] < self.expand_depth
            && self.pool_ptr as i64 + a[at + H_POOL] as i64 + 2 <= self.pool_size as i64
            && self.str_ptr + a[at + H_STR] < self.max_strings
            && self.web_room_for(a[at + H_MEM] as i64)
    }

    /// What the real path would have done, from the entry at `at`.
    fn cm_replay(&mut self, at: i32) {
        let (res, _, _, _, ex) = self.cm_parts(at);
        let (op_at, op_n) = (self.cm_arena[at + H_OP_AT], self.cm_arena[at + H_OP_N]);
        let mut stack = [0i32; EXPR_CAP];
        let mut d = 0;
        let mut x = ex;
        for k in op_at..op_at + op_n {
            match self.cm_ops[k] {
                OP_COND_PUSH => {
                    // §521; the rest of its words are written over by `free_node` before anything reads them.
                    let p = self.get_node_leaf(IF_NODE_SIZE);
                    self.set_link(p, self.cond_ptr);
                    self.cond_ptr = p;
                }
                OP_COND_POP => {
                    // §522
                    if self.if_stack[self.in_open] == self.cond_ptr {
                        self.if_warning();
                    }
                    let p = self.cond_ptr;
                    self.cond_ptr = self.link(p);
                    self.free_node(p, IF_NODE_SIZE);
                }
                OP_EXPR_PUSH | OP_EXPR_PUSH_LINKED => {
                    // §1788; `t` and `n` outlive the node.
                    let q = self.get_node(EXPR_NODE_SIZE);
                    let link = if self.cm_ops[k] == OP_EXPR_PUSH { NULL } else { stack[d - 1] };
                    self.set_link(q, link);
                    let (t, n) = (self.cm_arena[x], self.cm_arena[x + 1]);
                    self.set_expr_t_field(q, t);
                    self.set_expr_n_field(q, n);
                    x += 2;
                    stack[d] = q;
                    d += 1;
                }
                _ => {
                    // §1789
                    d -= 1;
                    self.free_node(stack[d], EXPR_NODE_SIZE);
                }
            }
        }
        let a = &self.cm_arena;
        let (up_in, up_param, up_buf) = (a[at + H_IN], a[at + H_PARAM], a[at + H_BUF]);
        if up_in >= 0 {
            self.max_in_stack = self.max_in_stack.max(self.input_ptr + up_in);
        }
        self.max_param_stack = self.max_param_stack.max(self.param_ptr + up_param);
        if up_buf >= 0 {
            self.max_buf_stack = self.max_buf_stack.max(self.first + up_buf);
        }
        if a[at + H_FD] != 0 {
            // §451's `font_info[fmem_ptr].sc:=0`.
            let k = self.fmem_ptr;
            self.font_info[k].set_sc(0);
        }
        // §473's list, as `scan_toks` makes it.
        self.web_head();
        let m = self.tb_mark();
        let n = self.cm_arena[at + H_RES];
        for i in 0..n {
            let t = self.cm_arena[res + i];
            self.tb_push(t);
        }
        self.def_ref = self.tb_commit_ref(m);
        let a = &self.cm_arena;
        let (cmd, chr, cs, tok, align, loc) = (a[at + H_CMD], a[at + H_CHR], a[at + H_CS], a[at + H_TOK], a[at + H_ALIGN], a[at + H_LOC]);
        self.cur_cmd = cmd;
        self.cur_chr = chr;
        self.cur_cs = cs;
        self.cur_tok = tok;
        self.align_state += align;
        self.set_loc(self.loc() + loc);
    }

    // -----------------------------------------------------------------------------------------------------------------------------
    // A recording.
    // -----------------------------------------------------------------------------------------------------------------------------

    /// The real path, recorded: into a new entry, or under `MODE_CHECK` held against the entry in slot `check`.
    #[cold]
    #[inline(never)]
    fn cm_record(&mut self, key: &[i32], h: i32, site: i32, check: Option<i32>) {
        let b = self.start();
        let ps = self.param_start();
        let keys: Vec<i32> = (ps..self.param_ptr).map(|k| self.param_stack[k]).filter(|&p| p != NULL).collect();
        self.v_image.cm.site_rec[site] += 1;
        let r = Rec {
            check,
            key_cells: key.to_vec(),
            keys,
            key_hash: h,
            site,
            // The body the call stands in is read from too: past the brace that ends it, an `\\expandafter` can reach.
            bodies: vec![b],
            block: b,
            loc0: self.loc(),
            in0: self.input_ptr,
            floor: self.param_start(),
            param0: self.param_ptr,
            cond0: self.cond_ptr,
            var0: self.var_used,
            errors0: self.error_count,
            history0: self.history,
            interaction0: self.interaction,
            first0: self.first,
            pool0: self.pool_ptr,
            str0: self.str_ptr,
            tb0: self.tb_ptr,
            tx0: self.tx_ptr,
            hash0: self.hash_high,
            fmem0: self.fmem_ptr,
            save0: self.save_ptr,
            nest0: self.nest_ptr,
            level0: self.cur_level,
            align0: self.align_state,
            obs0: self.host.world.log.len(),
            term0: self.host.world.term_written,
            offsets0: (self.file_offset, self.term_offset),
            cur_ptr0: self.cur_ptr,
            exp0: self.expand_depth_count,
            saved_in: self.max_in_stack,
            saved_param: self.max_param_stack,
            saved_buf: self.max_buf_stack,
            real_expand_depth: self.expand_depth,
            body_seen: vec![0; BODY_CACHE],
            ..Rec::default()
        };
        self.host.hv.case_memo.r = r;
        // The marks start where the call stands, so that they say how far it went; §321 and §390 still stop at the capacities.
        self.max_in_stack = self.input_ptr - 1;
        self.max_param_stack = self.param_ptr;
        self.max_buf_stack = self.first;
        self.expand_depth = self.expand_depth_count + 1;
        self.v_image.cm.rec = 1;
        self.interrupt = CASE_MEMO_INTERRUPT;
        self.scan_pdf_ext_toks();
        if self.v_image.cm.rec == 0 {
            // An error has ended it.
            if let Some(k) = check {
                self.cm_check_failed(k, "an error");
            }
            return;
        }
        self.cm_event(false, 0);
        let up = self.cm_unlower();
        let why = self.cm_end_check();
        if let Some(why) = why {
            self.cm_abort(why);
        }
        let r = std::mem::take(&mut self.host.hv.case_memo.r);
        if check.is_none() {
            let v = &mut self.v_image.cm;
            v.credit = (v.credit - RECORD_COST - r.levels / 4).max(0);
        }
        if r.abort.is_none() && check.is_none() && r.levels < WORK_MIN && self.v_image.cm.mode != MODE_CHECK {
            // Too small to gain by.
            self.v_image.cm.stats[12] += 1;
            self.v_image.cm.site_small[r.site] += 1;
            self.cm_never(r.key_hash);
            self.host.hv.case_memo.r = r;
            return;
        }
        let made = match r.abort {
            Some(why) => Err(why),
            None => self.cm_entry(&r, up),
        };
        match (made, check) {
            (Err(why), Some(k)) => self.cm_check_failed(k, why),
            (Err(why), None) => {
                self.cm_note_reason(why);
                self.v_image.cm.stats[3] += 1;
                self.cm_never(r.key_hash);
            }
            (Ok(e), Some(k)) => match self.cm_compare(k, &e) {
                Some(why) => self.cm_check_failed(k, why),
                None => self.v_image.cm.stats[9] += 1,
            },
            (Ok(e), None) => {
                self.cm_store(e, r.key_hash);
                self.v_image.cm.stats[2] += 1;
            }
        }
    }

    /// The marks back to what they would be, as the larger of where they were and where the call took them; the rises for the entry.
    fn cm_unlower(&mut self) -> (i32, i32, i32) {
        self.v_image.cm.rec = 0;
        if self.interrupt == CASE_MEMO_INTERRUPT {
            self.interrupt = 0;
        }
        let r = &self.host.hv.case_memo.r;
        let up_in = if self.max_in_stack >= r.in0 { self.max_in_stack - r.in0 } else { -1 };
        let up_param = self.max_param_stack - r.param0;
        let up_buf = if self.max_buf_stack > r.first0 { self.max_buf_stack - r.first0 } else { -1 };
        self.max_in_stack = self.max_in_stack.max(r.saved_in);
        self.max_param_stack = self.max_param_stack.max(r.saved_param);
        self.max_buf_stack = if up_buf >= 0 { self.max_buf_stack.max(r.saved_buf) } else { r.saved_buf };
        self.expand_depth = r.real_expand_depth;
        (up_in, up_param, up_buf)
    }

    /// An error, a fatal one too, while recording: the marks as they would be before anything prints them, and no entry.
    #[cold]
    #[inline(never)]
    pub fn case_memo_on_error(&mut self) {
        if self.v_image.cm.rec != 0 {
            self.cm_unlower();
            if self.host.hv.case_memo.r.check.is_none() {
                self.cm_note_reason("an error");
                self.v_image.cm.stats[3] += 1;
                let h = self.host.hv.case_memo.r.key_hash;
                self.cm_never(h);
            }
        }
    }

    /// Whether all is as the call found it, but what it was meant to change.
    fn cm_end_check(&self) -> Option<&'static str> {
        let r = &self.host.hv.case_memo.r;
        if self.input_ptr != r.in0 || self.state() != TOKEN_LIST || self.start() != r.block || self.param_start() != r.floor || self.param_ptr != r.param0 {
            return Some("the input");
        }
        if self.cond_ptr != r.cond0 || r.cond_depth != 0 || !r.expr_stack.is_empty() || self.var_used != r.var0 {
            return Some("the nodes");
        }
        if self.error_count != r.errors0 || self.history != r.history0 || self.interaction != r.interaction0 || self.interrupt != 0 {
            return Some("an error");
        }
        if self.host.world.log.len() != r.obs0 || self.host.world.term_written != r.term0 || (self.file_offset, self.term_offset) != r.offsets0 {
            return Some("printing or the world");
        }
        if self.hash_high != r.hash0 {
            return Some("a name made");
        }
        if self.pool_ptr != r.pool0 || self.str_ptr != r.str0 || self.fmem_ptr != r.fmem0 || self.cur_ptr != r.cur_ptr0 {
            return Some("strings or fonts");
        }
        if self.tb_ptr != r.tb0 || self.tx_ptr != r.tx0 || self.csname_depth != 0 || self.save_ptr != r.save0 || self.nest_ptr != r.nest0 || self.cur_level != r.level0 {
            return Some("the stacks");
        }
        if self.v_webmem.fine != 0 || self.arith_error {
            return Some("main memory");
        }
        None
    }

    #[cold]
    pub fn cm_abort(&mut self, why: &'static str) {
        let r = &mut self.host.hv.case_memo.r;
        if r.abort.is_none() {
            r.abort = Some(why);
        }
    }

    fn cm_note_reason(&mut self, why: &'static str) {
        let v = &mut self.host.hv.case_memo.reasons;
        match v.iter_mut().find(|e| e.0 == why) {
            Some(e) => e.1 += 1,
            None => v.push((why, 1)),
        }
    }

    /// `MODE_CHECK`: the real path did not do what the entry in slot `k` says.  Said on the error stream, and the entry goes.
    #[cold]
    fn cm_check_failed(&mut self, k: i32, why: &'static str) {
        self.v_image.cm.stats[10] += 1;
        let h = self.v_image.cm.e_hash[k];
        crate::platform::write_stderr(format!("case memo check: key {h:08x} differs in {why}\n").as_bytes());
        self.cm_drop(k);
    }

    /// Takes an entry away, and the references it holds.
    fn cm_drop(&mut self, k: i32) {
        let at = self.v_image.cm.e_at[k];
        self.v_image.cm.e_hash[k] = 0;
        self.v_image.cm.live -= 1;
        let mut held = Vec::new();
        self.cm_entry_blocks(at, &mut held);
        for p in held {
            self.delete_token_ref(p);
        }
    }

    /// The blocks the entry at `at` holds a reference to.
    fn cm_entry_blocks(&self, at: i32, held: &mut Vec<i32>) {
        let (_, cl, rd, _, _) = self.cm_parts(at);
        let a = &self.cm_arena;
        // The body the call stood in, the first cell of its key.
        held.push(a[at + H_SIZE]);
        for i in 0..a[at + H_CL] {
            let p = cl + 3 * i;
            let t = a[p + 1] & !crate::native::ARMED_BITS;
            if t >= CALL && t < END_TEMPLATE && a[p + 2] > ANY_BODY {
                held.push(a[p + 2]);
            }
        }
        for i in 0..a[at + H_RD] {
            let p = rd + 3 * i;
            if (a[p] == RD_EQTB_LIST || a[p] == RD_MEM_LIST) && a[p + 2] != NULL {
                held.push(a[p + 2]);
            }
        }
    }

    /// For `webmem.rs`: the blocks of all entries, and `\exp_not:e`'s.
    pub fn cm_held_blocks(&self, held: &mut Vec<i32>) {
        let v = &self.v_image.cm;
        for k in 0..ENTRIES as i32 {
            if v.e_hash[k] != 0 {
                self.cm_entry_blocks(v.e_at[k], held);
            }
        }
    }

    /// The entry for the recording `r` that has just ended well, or why there is none.
    fn cm_entry(&mut self, r: &Rec, up: (i32, i32, i32)) -> Result<Entry, &'static str> {
        // The closure: every control sequence in a list the call read from.  A character of catcode 4 in one is an `&` it can read.
        let mut cs: Vec<i32> = Vec::new();
        let mut tab = r.tab;
        let add_list = |t: &Tex, p: i32, cs: &mut Vec<i32>, tab: &mut bool| {
            let mut i = p + 1;
            loop {
                let x = t.toks[i];
                if x == 0 {
                    break;
                }
                if x >= CS_TOKEN_FLAG {
                    cs.push(x - CS_TOKEN_FLAG);
                } else if x >> 8 == TAB_MARK {
                    *tab = true;
                }
                i += 1;
            }
        };
        for &p in &r.keys {
            add_list(self, p, &mut cs, &mut tab);
        }
        let mut bodies = r.bodies.clone();
        bodies.sort_unstable();
        bodies.dedup();
        for &p in &bodies {
            add_list(self, p, &mut cs, &mut tab);
        }
        for &p in &r.lists {
            if p != NULL {
                add_list(self, p, &mut cs, &mut tab);
            }
        }
        cs.extend_from_slice(&r.extra_cs);
        cs.sort_unstable();
        cs.dedup();
        let mut reads = r.reads.clone();
        reads.push((RD_EQTB, INT_BASE + ESCAPE_CHAR_CODE, self.escape_char()));
        reads.push((RD_PAR_TOKEN, 0, self.par_token));
        let mut closure: Vec<(i32, i32, i32)> = Vec::with_capacity(cs.len());
        let mut strict = r.strict.clone();
        strict.sort_unstable();
        let mut refs: Vec<i32> = vec![r.block];
        for &c in &cs {
            let w = self.eqtb[c];
            let t = w.b0() & !crate::native::ARMED_BITS;
            // A macro that did not run, and that neither `\\ifx` nor `\\meaning` looked into, was read for its `eq_type` and for whether it
            // is protected, §504's test, and no more: whatever it is defined as then, the call does what it did.
            let any_body = t >= CALL && t < END_TEMPLATE && w.rh() != NULL && bodies.binary_search(&w.rh()).is_err() && strict.binary_search(&c).is_err();
            if any_body {
                closure.push((c, w.b0(), if self.toks[w.rh() + 1] == PROTECTED_TOKEN { ANY_PROTECTED } else { ANY_BODY }));
                continue;
            }
            closure.push((c, w.b0(), w.rh()));
            // A register or parameter it names: what `scan_something_internal` reads of it without a hook.
            if t == ASSIGN_INT || t == ASSIGN_DIMEN {
                reads.push((RD_EQTB, w.rh(), self.eqtb[w.rh()].rh()));
            }
            // A name that means an `&`, `\span` or `\cr`.
            if t == TAB_MARK || t == CAR_RET {
                tab = true;
            }
            if t >= CALL && t < END_TEMPLATE {
                refs.push(w.rh());
            }
        }
        reads.sort_unstable();
        reads.dedup();
        for w in reads.windows(2) {
            if w[0].0 == w[1].0 && w[0].1 == w[1].1 && w[0].0 != RD_IFX {
                // Two values of one place: something wrote it.
                return Err("a place read twice");
            }
        }
        for &(kind, _, val) in &reads {
            if (kind == RD_EQTB_LIST || kind == RD_MEM_LIST) && val != NULL {
                refs.push(val);
            }
        }
        let mut names = r.names.clone();
        names.sort_unstable();
        names.dedup();
        if names.iter().any(|n| n.len() > 255) {
            return Err("a long name");
        }
        let res_n = self.tok_len(self.def_ref);
        let key_n = r.key_cells.len() as i32;
        if res_n > KEY_CAP {
            return Err("a long result");
        }
        let nm_cells: i32 = names.iter().map(|n| 1 + (n.len() as i32 + 3) / 4).sum();
        let size = H_SIZE + key_n + res_n + 3 * closure.len() as i32 + 3 * reads.len() as i32 + nm_cells + 2 * r.expr.len() as i32;
        if size as usize > ARENA / 2 || r.ops.len() > OPS / 2 {
            return Err("too large");
        }
        let mut cells: Vec<i32> = Vec::with_capacity(size as usize);
        cells.extend_from_slice(&[
            key_n,
            res_n,
            closure.len() as i32,
            reads.len() as i32,
            names.len() as i32,
            nm_cells,
            r.expr.len() as i32,
            0,
            r.ops.len() as i32,
            self.loc() - r.loc0,
            self.align_state - r.align0,
            self.cur_cmd,
            self.cur_chr,
            self.cur_cs,
            self.cur_tok,
            up.0,
            up.1,
            up.2,
            r.peak_exp,
            r.peak_pool,
            r.peak_str,
            (r.peak_mem.max(res_n as i64) * 2 + 4096).min(i32::MAX as i64) as i32,
            r.fd_read as i32,
            self.hash_high,
            tab as i32,
            if tab { r.align0 } else { 0 },
            r.levels,
        ]);
        debug_assert_eq!(cells.len() as i32, H_SIZE);
        cells.extend_from_slice(&r.key_cells);
        cells.extend_from_slice(self.toks.range(self.def_ref + 1, self.def_ref + 1 + res_n));
        for &(c, b0, rh) in &closure {
            cells.extend_from_slice(&[c, b0, rh]);
        }
        for &(kind, loc, val) in &reads {
            cells.extend_from_slice(&[kind, loc, val]);
        }
        for n in &names {
            cells.push(n.len() as i32);
            for c in n.chunks(4) {
                let mut v = 0u32;
                for (j, &b) in c.iter().enumerate() {
                    v |= (b as u32) << (8 * j);
                }
                cells.push(v as i32);
            }
        }
        for &(t, n) in &r.expr {
            cells.extend_from_slice(&[t, n]);
        }
        debug_assert_eq!(cells.len() as i32, size);
        Ok(Entry { cells, ops: r.ops.clone(), refs })
    }

    /// `e` into the memo, under the hash `h`.
    fn cm_store(&mut self, e: Entry, h: i32) {
        let size = e.cells.len() as i32;
        let ops_n = e.ops.len() as i32;
        let mut slot = (0..ENTRIES as i32).find(|&k| self.v_image.cm.e_hash[k] == 0);
        if slot.is_none() || self.v_image.cm.top + size > ARENA as i32 || self.v_image.cm.ops_top + ops_n > OPS as i32 {
            // Full: all go.
            for k in 0..ENTRIES as i32 {
                if self.v_image.cm.e_hash[k] != 0 {
                    self.cm_drop(k);
                }
            }
            self.v_image.cm.top = 0;
            self.v_image.cm.ops_top = 0;
            self.v_image.cm.stats[6] += 1;
            slot = Some(0);
        }
        let k = slot.unwrap_or(0);
        // References: each macro of the closure and each list read.
        for &p in &e.refs {
            self.add_token_ref(p);
        }
        let (at, op_at) = (self.v_image.cm.top, self.v_image.cm.ops_top);
        for (i, &c) in e.cells.iter().enumerate() {
            self.cm_arena[at + i as i32] = c;
        }
        self.cm_arena[at + H_OP_AT] = op_at;
        for (i, &o) in e.ops.iter().enumerate() {
            self.cm_ops[op_at + i as i32] = o;
        }
        let v = &mut self.v_image.cm;
        v.top = at + size;
        v.ops_top = op_at + ops_n;
        v.e_at[k] = at;
        v.e_hash[k] = h;
        v.live += 1;
    }

    /// `MODE_CHECK`: what in `e`, the entry the real path has just made, is not in the entry in slot `k`.  Where the operations are,
    /// `hash_high` at the last look for the names not found, and the bound on main memory, which depends on the blocks of the heap, may differ.
    fn cm_compare(&self, k: i32, e: &Entry) -> Option<&'static str> {
        let at = self.v_image.cm.e_at[k];
        let a = &self.cm_arena;
        let (res, cl, rd, nm, ex) = self.cm_parts(at);
        let size = ex + 2 * a[at + H_EX] - at;
        let parts = [(res, "the key"), (cl, "the result"), (rd, "the closure"), (nm, "the reads"), (ex, "the names"), (at + size, "the expressions")];
        for i in 0..H_SIZE {
            if i != H_OP_AT && i != H_HH && i != H_MEM && a[at + i] != e.cells[i as usize] {
                return Some(match i {
                    H_KEY | H_RES | H_CL | H_RD | H_NM | H_NM_CELLS | H_EX | H_OP_N => "the sizes",
                    H_LOC | H_ALIGN | H_CMD | H_CHR | H_CS | H_TOK => "where it ended",
                    H_TABS | H_ALIGN0 => "the alignment",
                    H_FD => "\\fontdimen",
                    _ => "the stacks' marks",
                });
            }
        }
        if size != e.cells.len() as i32 {
            return Some("the sizes");
        }
        for i in H_SIZE..size {
            if a[at + i] != e.cells[i as usize] {
                // A macro of the closure defined again with the tokens it had: `cm_holds` takes it for the same, and so does this.  And
                // one that did not run counts as having run where it shares its body with one that did, as after a `\\let`, so the same
                // macro may be held to its body in one recording and to nothing in the other.
                let (old, new) = (a[at + i], e.cells[i as usize]);
                if at + i >= cl && at + i < rd && (at + i - cl) % 3 == 2 && (old <= ANY_BODY || new <= ANY_BODY || self.cm_same_body(a[at + i - 1], new, old)) {
                    continue;
                }
                return Some(parts.iter().find(|&&(end, _)| at + i < end).map_or("the expressions", |&(_, what)| what));
            }
        }
        let op_at = a[at + H_OP_AT];
        if (0..a[at + H_OP_N]).any(|i| self.cm_ops[op_at + i] != e.ops[i as usize]) {
            return Some("the nodes");
        }
        None
    }

    // -----------------------------------------------------------------------------------------------------------------------------
    // The hooks, each behind `rec`.
    // -----------------------------------------------------------------------------------------------------------------------------

    /// A level ends, in `end_token_list`, with its list still there: a macro's says what the call read.
    #[cold]
    #[inline(never)]
    pub fn case_memo_level_ends(&mut self) {
        if self.token_type() != MACRO {
            return;
        }
        let ps = self.param_start();
        let (in0, base) = (self.host.hv.case_memo.r.in0, self.host.hv.case_memo.r.param0);
        if self.input_ptr <= in0 || ps < base {
            // The level the call stands in, or one below: the call reads past its input.
            self.cm_abort("the input ended");
            return;
        }
        self.host.hv.case_memo.r.levels += 1;
        let b = self.start();
        let (pp, tb, tx) = (self.param_ptr, self.tb_ptr, self.tx_ptr);
        let r = &mut self.host.hv.case_memo.r;
        let slot = ((b as u32).wrapping_mul(0x9e37_79b9) >> 22) as usize % BODY_CACHE;
        match r.body_seen.get_mut(slot) {
            Some(s) if *s == b => {}
            Some(s) => {
                *s = b;
                r.bodies.push(b);
            }
            None => r.bodies.push(b),
        }
        // What main memory holds for the call at its fullest is just before a level ends: its arguments, those below it and the
        // builders.  An argument by what its block holds, which is its length or more, summed once as it comes onto the parameter
        // stack, which only this end takes from.  Doubled for what else is in flight.
        let n = (pp - base).max(0) as usize;
        r.held.truncate(n);
        let mut s = r.held.last().copied().unwrap_or(0);
        for k in base + r.held.len() as i32..pp {
            let p = self.param_stack[k];
            if p != NULL {
                s += crate::toks::CLASS_CAP[(self.toks[p] & 0xFF) as usize] as i64 + 1;
            }
            r.held.push(s);
        }
        let live = s + (tb - r.tb0) as i64 + (tx - r.tx0) as i64;
        r.peak_mem = r.peak_mem.max(live);
        r.held.truncate((ps - base) as usize);
    }

    /// The depth of the condition stack above where the call began, and with it the pops since the last event: `pushed` for §521's,
    /// `extra` the words of a node just taken that the stacks here do not have yet.
    fn cm_event(&mut self, pushed: bool, extra: i32) {
        let cond0 = self.host.hv.case_memo.r.cond0;
        let mut p = self.cond_ptr;
        let mut d = 0;
        while p != cond0 {
            if p == NULL {
                self.cm_abort("a condition ended that began before");
                return;
            }
            p = self.link(p);
            d += 1;
        }
        let before = if pushed { d - 1 } else { d };
        let r = &mut self.host.hv.case_memo.r;
        let pops = r.cond_depth - before;
        if before < 0 || pops < 0 {
            r.abort.get_or_insert("a condition ended that began before");
            return;
        }
        for _ in 0..pops {
            r.ops.push(OP_COND_POP);
        }
        if pushed {
            r.ops.push(OP_COND_PUSH);
        }
        r.cond_depth = d;
        let want = r.var0 + IF_NODE_SIZE * d + EXPR_NODE_SIZE * r.expr_stack.len() as i32 + extra;
        if self.var_used != want {
            self.cm_abort("a node of another kind");
        }
    }

    /// §510's `\\relax` for a `\\fi` or `\\else` while a condition is evaluated, or §404's: if that condition began before the call,
    /// the call read its `if_limit`, which a replay would not have.
    #[cold]
    #[inline(never)]
    pub fn case_memo_insert_relax(&mut self) {
        if self.cond_ptr == self.host.hv.case_memo.r.cond0 {
            self.cm_abort("a condition that began before");
        }
    }

    /// §521 has pushed the condition stack for `cur_chr`.
    #[cold]
    #[inline(never)]
    pub fn case_memo_cond(&mut self) {
        let this_if = self.cur_chr & (UNLESS_CODE - 1);
        match this_if {
            IF_VOID_CODE | IF_HBOX_CODE | IF_VBOX_CODE => self.cm_abort("a box"),
            IF_EOF_CODE => self.cm_abort("\\ifeof"),
            IF_FONT_CHAR_CODE => self.cm_abort("\\iffontchar"),
            IF_PDFPRIMITIVE_CODE => self.cm_abort("\\ifpdfprimitive"),
            IF_VMODE_CODE | IF_HMODE_CODE | IF_MMODE_CODE | IF_INNER_CODE => {
                let m = self.mode();
                self.host.hv.case_memo.r.reads.push((RD_MODE, 0, m));
            }
            _ => {}
        }
        self.cm_event(true, 0);
    }

    /// §1788 has pushed `q`, with `link(q)` the node below it or null.
    #[cold]
    #[inline(never)]
    pub fn case_memo_expr_push(&mut self, q: i32) {
        self.cm_event(false, EXPR_NODE_SIZE);
        let link = self.link(q);
        let (t, n) = (self.mem[q + 2].rh(), self.mem[q + 3].rh());
        let r = &mut self.host.hv.case_memo.r;
        if link != NULL && r.expr_stack.last() != Some(&link) || r.expr_stack.len() >= EXPR_CAP {
            r.abort.get_or_insert("an expression");
            return;
        }
        r.ops.push(if link == NULL { OP_EXPR_PUSH } else { OP_EXPR_PUSH_LINKED });
        r.expr.push((t, n));
        r.expr_stack.push(q);
    }

    /// §1789 is about to free `q`.
    #[cold]
    #[inline(never)]
    pub fn case_memo_expr_pop(&mut self, q: i32) {
        self.cm_event(false, 0);
        let r = &mut self.host.hv.case_memo.r;
        if r.expr_stack.pop() != Some(q) {
            r.abort.get_or_insert("an expression");
            return;
        }
        r.ops.push(OP_EXPR_POP);
    }

    /// §364 has an `&`, `\\span` or `\\cr`: with `align_state` 0 a template comes in.  The entry needs the `align_state` the call began
    /// at, as it does for one that a capture loop passes, which says nothing.
    #[cold]
    #[inline(never)]
    pub fn case_memo_tab(&mut self) {
        if self.align_state == 0 {
            self.cm_abort("an & or \\cr at align_state 0");
        }
        self.host.hv.case_memo.r.tab = true;
    }

    /// §534 has compared the bodies of the macros `n` and `m` and found `b`: no more is read of them there.
    #[cold]
    #[inline(never)]
    pub fn case_memo_ifx(&mut self, n: i32, m: i32, b: bool) {
        if n == 0 || m == 0 {
            self.cm_abort("an \\ifx of no name");
            return;
        }
        self.host.hv.case_memo.r.reads.push((RD_IFX, n, 2 * m + b as i32));
    }

    /// `\\meaning` shows `cs`: it is read for its body though it does not run.
    #[cold]
    #[inline(never)]
    pub fn case_memo_strict(&mut self, cs: i32) {
        if cs != 0 {
            self.host.hv.case_memo.r.strict.push(cs);
        }
    }

    /// A read of a place that has to be the same for a replay.
    #[cold]
    #[inline(never)]
    pub fn case_memo_read(&mut self, kind: i32, loc: i32) {
        let v = self.cm_value(kind, loc);
        let r = &mut self.host.hv.case_memo.r;
        r.reads.push((kind, loc, v));
        if kind == RD_EQTB_LIST || kind == RD_MEM_LIST {
            r.lists.push(v);
        }
    }

    /// `print_cs` or `sprint_cs` prints the text of `p`, which for a font's identifier is the name of the last `\font` that loaded it.
    #[cold]
    #[inline(never)]
    pub fn case_memo_cs_text(&mut self, p: i32) {
        if p >= FONT_ID_BASE && p < UNDEFINED_CONTROL_SEQUENCE {
            self.case_memo_read(RD_TEXT, p);
        }
    }

    /// §451 has found its place in `font_info`, `cur_val`, and is about to clear `font_info[fmem_ptr]`, which a replay does too.
    #[cold]
    #[inline(never)]
    pub fn case_memo_fontdimen(&mut self) {
        self.case_memo_read(RD_FONT, self.cur_val);
        self.host.hv.case_memo.r.fd_read = true;
    }

    /// §455's `em` or `ex`, `k` the parameter: `cur_font` and that parameter of it.
    #[cold]
    #[inline(never)]
    pub fn case_memo_font_unit(&mut self, k: i32) {
        self.case_memo_read(RD_EQTB, CUR_FONT_LOC);
        let f = self.cur_font();
        self.case_memo_read(RD_FONT, k + self.param_base[f]);
    }

    /// `\csname` has found `cur_cs`, before §400 defines it if it is undefined.
    #[cold]
    #[inline(never)]
    pub fn case_memo_csname(&mut self) {
        if self.eq_type(self.cur_cs) == UNDEFINED_CS {
            self.cm_abort("\\csname of an undefined name");
        }
        let c = self.cur_cs;
        self.host.hv.case_memo.r.extra_cs.push(c);
    }

    /// `\ifcsname` has looked up `buffer[first..m]`.
    #[cold]
    #[inline(never)]
    pub fn case_memo_ifcsname(&mut self, m: i32) {
        if self.cur_cs == UNDEFINED_CONTROL_SEQUENCE {
            let name = self.buffer.range(self.first, m).to_vec();
            self.host.hv.case_memo.r.names.push(name);
        } else {
            let c = self.cur_cs;
            self.host.hv.case_memo.r.extra_cs.push(c);
        }
    }

    /// `str_toks` and `make_string`: how far the pool and the strings have gone.
    #[cold]
    #[inline(never)]
    pub fn case_memo_strings(&mut self, made: i32) {
        let (pool, strs) = (self.pool_ptr, self.str_ptr + made);
        let r = &mut self.host.hv.case_memo.r;
        r.peak_pool = r.peak_pool.max(pool - r.pool0);
        r.peak_str = r.peak_str.max(strs - r.str0);
    }

    /// The expansion depth has come to `expand_depth`, which a recording keeps just above the deepest so far.  `need` is what §390
    /// or §388 asks to be below the limit.
    #[cold]
    #[inline(never)]
    pub fn expand_depth_reached(&mut self, need: i32) {
        if self.v_image.cm.rec != 0 {
            let r = &mut self.host.hv.case_memo.r;
            if need < r.real_expand_depth {
                r.peak_exp = r.peak_exp.max(need - r.exp0);
                self.expand_depth = need + 1;
                return;
            }
            self.cm_unlower();
        }
        self.overflow(b"expansion depth", self.expand_depth);
    }

    /// The command `c` of `conv_toks`, while recording.
    #[cold]
    #[inline(never)]
    pub fn case_memo_convert(&mut self, c: i32) {
        match c {
            NUMBER_CODE | ROMAN_NUMERAL_CODE | STRING_CODE | MEANING_CODE | EXPANDED_CODE | ETEX_REVISION_CODE | PDFTEX_REVISION_CODE
            | PDFTEX_BANNER_CODE | PDF_STRCMP_CODE | PDF_ESCAPE_STRING_CODE | PDF_ESCAPE_NAME_CODE | PDF_ESCAPE_HEX_CODE
            | PDF_UNESCAPE_HEX_CODE => {}
            _ => self.cm_abort("a convert command"),
        }
    }

    /// `scan_internal_others` for `cur_cmd` at `level`, while recording: what it reads that has no hook of its own stops it.
    #[cold]
    #[inline(never)]
    pub fn case_memo_internal(&mut self) {
        let m = self.cur_chr;
        match self.cur_cmd {
            DEF_CODE | TOKS_REGISTER | ASSIGN_TOKS | SET_FONT | DEF_FONT | DEF_FAMILY | LETTERSPACE_FONT | PDF_COPY_FONT | CHAR_GIVEN
            | MATH_GIVEN | ASSIGN_DIMEN | ASSIGN_FONT_DIMEN | REGISTER => {}
            ASSIGN_FONT_INT if m <= 1 => {}
            LAST_ITEM if m >= ETEX_EXPR && m - ETEX_EXPR + INT_VAL <= DIMEN_VAL => {}
            ASSIGN_INT => self.cm_abort("the clock"),
            _ => self.cm_abort("an internal quantity"),
        }
    }

    /// For `RSTEX_PF_STATS`.
    pub fn cm_report(&self) -> String {
        let s = &self.v_image.cm.stats;
        let mut o = format!(
            "cm: calls {}, replayed {}, recorded {}, stopped {}, context {}, changed {}, emptied {}, no room {}, first calls {}, checked {}, check failed {}, never {}, small {}, struck {}, place left {}, asleep {}\n",
            s[0], s[1], s[2], s[3], s[4], s[5], s[6], s[7], s[8], s[9], s[10], s[11], s[12], s[13], s[14], s[15]
        );
        for (why, n) in &self.host.hv.case_memo.reasons {
            o += &format!("cm stopped: {why} {n}\n");
        }

        o
    }
}
