# rstex: the porting contract

Every agent porting WEB to Rust follows this document.  `docs/DESIGN.md` holds the decisions; this holds how
to carry them out, site by site.  If this document and your judgement disagree, follow this document and report
the case in your final message.

## Privacy

The user's thesis must never reach GitHub through this project or its testing: not in commits, branches, CI, fixtures,
logs, reports or commit messages.  Testing against it locally is wanted, read-only, with results under `target/`.  Do not
quote its text in commit messages or reports.  A minimised reproduction derived from it must be rewritten with neutral
content before it may be committed.  `tools/privacy_guard.py` enforces this through git hooks; do not use `--no-verify`.
See DESIGN section 13.

Contents: [1 Sources](#1-sources) · [2 Files and ownership](#2-files-and-ownership) ·
[3 Working in the shared tree](#3-working-in-the-shared-tree) · [4 Types](#4-types) ·
[5 Globals](#5-globals) · [6 Accessors](#6-accessors) · [7 Strings](#7-strings) ·
[8 Modules and helpers](#8-modules-and-helpers) · [9 Gotos](#9-gotos) · [10 Case](#10-case) ·
[11 Pascal arithmetic](#11-pascal-arithmetic) · [12 Arrays and records](#12-arrays-and-records) ·
[13 For loops](#13-for-loops) · [14 Booleans and conditions](#14-booleans-and-conditions) ·
[15 Token lists](#15-token-lists) · [16 One-word lists in mem](#16-one-word-lists-in-mem) ·
[17 Errors](#17-errors) · [18 Printing and files](#18-printing-and-files) · [19 Dropped features](#19-dropped-features) ·
[20 Notes per module](#20-notes-per-module) · [Appendix A: macros](#appendix-a-macros) ·
[Appendix B: changed signatures](#appendix-b-changed-signatures) · [Pilot notes](#pilot-notes) ·
[Critic findings resolved](#critic-findings-resolved)

## 1. Sources

`refs/merged/pdftex-merged.web` is the specification.  Section numbers `§N` are weave's numbers in that file (1 to
1933, checked against weave's output).  **Not** tex.web's and not pdftex.web's numbers.  Look things up with
`tools/web.py`:

```
python3 -I tools/web.py sec 415          # print a section; `sec 415 427` prints a range
python3 -I tools/web.py part 25          # a part's section range; `parts` lists them all
python3 -I tools/web.py proc macro_call  # where a procedure is, and its header
python3 -I tools/web.py procs 414-427    # the procedures in a range
python3 -I tools/web.py mod "Scan a param"   # a named module: definition and use sections
python3 -I tools/web.py macro link       # the @d macro
python3 -I tools/web.py grep 'temp_head' # regex over the merged file, with section numbers
python3 -I tools/web.py uses temp_head   # the sections using an identifier in code
```

**Provenance** (`tools/make_merged_web.sh`, hashes in `tools/pinned-refs.txt`).  `refs/pinned/` is
TeX-Live/texlive-source tag `texlive-2026.0`, commit `f26cc5ed05`, with `texk/web2c/tex.ch` as patched by upstream
commit `b50e042b46`.  That is exactly what the oracle is built from: Arch's `texlive-bin` 2026.0-4 PKGBUILD checks out
that tag and applies that commit as `b50e042b.patch`.  (Its other two patches touch ptexenc and lua only.)  The order is
`pdftex_ch_srcs` of `pdftexdir/am/pdftex.am` with `pdftex_ch_synctex` of `synctexdir/am/synctex.am`, minus the three
encTeX files, plus our `tools/oracle-compat.ch`.  `tie` reports no errors and `tangle` and `weave` accept the result.

**encTeX is omitted from the merged WEB.**  Without `-enc`, all 800 lines of its difference reduce to the same behaviour,
except one, which `tools/oracle-compat.ch` restores: `\message` and `\errmessage` build their string with `^^`-notation
already expanded, so the line-break test `term_offset+length(s)>max_print_line-2` counts the expanded length.

**The `writenoexpanding` mystery is solved.**  It is encTeX's global `write_noexpanding` (`enctexdir/enctex1.ch`), read in
`get_next` at both places where a control sequence is looked up (`enctex2.ch`, `[24.354]` and `[24.357]`), and set during
`\special` and `\write` scanning when `\mubyteout` is in effect.  When true, a control sequence in the `\mubyte` output table
turns into `\relax` with `cur_chr=256`.  It is false unless a format was built with `-enc`, so it has no effect on the oracle
here.  The earlier search looked only in pdftex.web, tex.ch and unbalanced-braces.ch.  It is not a distro patch.

**MLTeX, IPC and src-specials stay in the merged WEB** because they live inside `tex.ch`, interleaved with the rest; the
port drops them: see [section 19](#19-dropped-features).

## 2. Files and ownership

`crates/engine/src/`, one owner per file.  A file owns the **procedures** declared in its sections:

| Module | Sections | What |
|---|---|---|
| `print.rs` | 1-37, 54-71 | Parts 1-3, 5: introduction, character set, I/O, printing |
| `strings.rs` | 38-53 | Part 4 |
| `errors.rs` | 72-98 | Part 6 |
| `arith.rs` | 99-127 | Parts 7, 7b |
| `mem.rs` | 128-150, 180-190 | Parts 8, 9, 11 |
| `nodes.rs` | 151-179 | Part 10 |
| `show.rs` | 191-216 | Part 12 |
| `copy.rs` | 217-224 | Parts 13, 14 |
| `eqtb.rs` | 225-228, 238-273 | Parts 15, 17 |
| `nest.rs` | 229-237 | Part 16 |
| `hash.rs` | 274-289 | Part 18 |
| `save.rs` | 290-310 | Part 19 |
| `toks.rs` | 311-318 | Part 20, and the token heap |
| `input.rs` | 319-353 | Parts 21-23 |
| `getnext.rs` | 354-387 | Part 24 |
| `expand.rs` | 388-413 | Part 25 up to macro_call |
| `macrocall.rs` | 414-427 | Part 25: macro_call |
| `capture.rs` | none | The capture loops of DESIGN section 8, in front of the literal loops |
| `scan.rs` | 428-489 | Part 26 |
| `buildtoks.rs` | 490-512 | Part 27 |
| `cond.rs` | 513-536 | Part 28 |
| `files.rs` | 537-564 | Part 29 |
| `tfm.rs` | 565-609 | Part 30 |
| `dvi.rs` | 610-671 | Parts 31, 32 |
| `pdfout.rs` | 672-693 | Parts 32a-c |
| `pdfobj.rs` | 694-702 | Part 32d |
| `pdffont.rs` | 703-726 | Part 32e |
| `pdfship.rs` | 727-815 | Part 32f |
| `pack.rs` | 816-855 | Part 33 |
| `mathdata.rs` | 856-874 | Part 34 |
| `mlist.rs` | 875-943 | Parts 35, 36 |
| `align.rs` | 944-988 | Part 37 |
| `linebreak.rs` | 989-1067 | Parts 38, 39 |
| `hyph.rs` | 1068-1143 | Parts 40-43 |
| `page.rs` | 1144-1205 | Parts 44, 45 |
| `control.rs` | 1206-1232 | Part 46 |
| `build.rs` | 1233-1313 | Part 47 |
| `mathbuild.rs` | 1314-1385 | Part 48 |
| `prefixed.rs` | 1386-1476 | Part 49 |
| `image.rs` | 1477-1509 | Part 50 |
| `main.rs` | 1510-1519 | Parts 51, 52 (a module; the crate has `autobins = false`) |
| `ext.rs` | 1520-1647 | Part 53 |
| `etex.rs` | 1648-1866 | Part 53a |
| `web2c.rs` | 1867-1933 | Parts 54, 55: web2c, string recycling, MLTeX, SyncTeX |
| `host.rs` | | Host and World; owned by whoever owns files, reached by everyone |
| `platform/` | | `mod.rs`, `unix.rs`, `windows.rs`: everything specific to an OS (DESIGN section 12); same owner as `host.rs` |

Frozen after M0, do not edit: `consts.rs`, `state.rs`, `access.rs`, `raw.rs`, `lib.rs`.  They are generated by
`tools/webgen.py` from the WEB (plus `tools/templates/access_hand.rs`).  **Never run `webgen.py --stubs` again: it overwrites
every module.**  If a frozen file is wrong or lacks something shared, report it; do not work around it silently.

**The unit of porting is the procedure.**  Every WEB procedure and function has a stub in the module owning its section, with `/// §N` and a typed signature, and the crate compiles from the start.  **Replace stubs in place**, keeping the signature.  Forward declarations in WEB are nothing here: all procedures are methods of `Tex`.  You may call any other module's stub; it panics with `todo!` until its owner arrives, which is fine for `cargo check`.

**A procedure includes every module reachable from its body, wherever that module was defined**, exactly as tangle does.  So a section's code *lands* in the file of each procedure that reaches it, whichever part it is printed in:

- `print_cmd_chr` is `input.rs`'s, and it contains all 40 `@<Cases of |print_cmd_chr|...@>` sections from every part.
- `main_control` is `control.rs`'s, with all the `@<Cases of |main_control|...@>`.
- `initialize` is `print.rs`'s, with all of `@<Set initial values of key variables@>` and `@<Initialize table entries...@>`.
- `init_prim` is `main.rs`'s, with all of `@<Put each of \TeX's primitives into the hash table@>`.
- e-TeX's `@<Cases for |conditional|@>` land in `conditional`, in `cond.rs`, not in `etex.rs`.
- `@<Handle a math node in |hlist_out|@>` lands in both `hlist_out` (`dvi.rs`) and `pdf_hlist_out` (`pdfship.rs`), and each ports its own copy.  This is what tangle does, and nobody has to agree on a helper's signature.

This is what makes the parallel port conflict-free: **no cross-file helper needs negotiating**.  A section holding procedure declarations stays with the procedures.

```
python3 -I tools/web.py tangle macro_call  # the whole procedure with every module inlined, marked {§N}: your input
python3 -I tools/web.py landing 1716       # which procedures, and files, contain §1716
```

External functions (the C side of web2c and pdfTeX, which WEB calls and does not define) are stubs too, marked `External:` with a note, in the module that uses them most.  The file operations (`a_open_in`, `input_ln`, `getc`, ...) are real, in `host.rs`.

**Completeness gate.**  Every section landing in your file carries a `// §N` or `/// §N` comment at the code implementing it, or is listed in your module's header as `//! DROPPED §N: reason` or `//! DROPPED §N-M: reason`.

```
python3 -I tools/coverage.py --module scan      # your module: everything landing in scan.rs
python3 -I tools/coverage.py crates/engine/src/scan.rs 428-489   # one range only
python3 -I tools/coverage.py --all              # everyone, one line each
python3 -I tools/coverage.py --stubs            # WEB procedures against Rust functions
```

It reports sections with no marker, and procedures with no function or still a `todo!`.  Sections with no code, and sections holding only declarations or macros that already exist, are exempt; so are MLTeX's sections, and sections of yours that land only in other files.  `--strict` drops the exemptions.  You are done when `--module` says `COMPLETE`.  A marker means you ported the section, not that you saw it.

The tool also fails on a misspelt constant, and it requires a finished module's header to say `#![deny(unreachable_patterns, unused_variables)]` and to drop `unused_variables` from its `allow`: see [section 10](#10-case).  Change the header when your procedures are done, since a `todo!` stub's parameters are unused.

## 3. Working in the shared tree

About a dozen agents work in the same tree, on disjoint files, without coordinating.  Therefore:

- **Edit only your own files.**  A helper you need from someone else's module does not get added to their file; it
  exists already (Appendix A), or you write it privately in yours.
- **Keep your file parsing at every moment.**  A syntax error in any file stops `cargo check` for everybody.  Replace a whole
  stub in one edit, from `pub fn` to its closing brace.  Never leave an unbalanced brace between edits.  If you need a helper
  you have not written, add it in the same edit with `todo!()`.
- **Your own target directory, and `-j6`:**
  ```
  export CARGO_TARGET_DIR=/home/site/Projects/rstex/target/<your-agent-name>
  cargo check -p rstex-engine -j6 --message-format=short 2>&1 | grep '^crates/engine/src/scan\.rs'
  ```
  Filter to your own file: other agents are mid-edit.  A clean run for your file is the bar after each procedure.  Name-resolution
  errors in someone else's file can hide your type errors for a moment; check again.
- **Do not run `cargo fmt`**: it touches other agents' files.  `rustfmt --edition 2024 crates/engine/src/scan.rs` formats
  only yours, and it is optional.
- **Never `git commit`.**  Only the user uses git.  Touch nothing outside `/home/site/Projects/rstex` and your scratch directory.
- Python is for `tools/` and `tests/` only, run with `python3 -I`.  Pure Rust: no `cc`, no `cmake`, no C bindings.
- **Three platforms are first-class**, and the gate holds from M0: `cargo check --workspace --target T` must pass for `x86_64-unknown-linux-gnu`, `aarch64-apple-darwin` and `x86_64-pc-windows-msvc`.  `cfg(unix)`, `cfg(windows)`, `cfg(target_os)`, `libc::`, `windows_sys::` and `std::os::` appear only in `platform/` and `crates/kpse`; `tests/foundation.rs` checks this by grep.  Every platform must produce the same PDF bytes for the same inputs.  Output is `\n` everywhere.
- Report what you ran.  Do not claim anything you did not verify.

A method name is global to the crate, since every module has `impl Tex`: even a private helper in your file collides
(E0592) if another module uses the same name.  So **prefix every helper that is not a WEB name with your module's
name**: `mlist_clean_up`, not `clean_up`.  A WEB macro you translate yourself (Appendix A, "left to the owner") may keep its WEB
name, since it is used nowhere else.

## 4. Types

Every integer-like WEB type is plain `i32` in variables, parameters, return values and fields.  That is `integer`, `pointer`,
`halfword`, `quarterword`, `eight_bits`, `small_number`, `scaled`, `nonnegative_integer`, `str_number`, `pool_pointer`,
`internal_font_number`, `ASCII_code`, `font_index`, `group_code`, `glue_ord`, `trie_pointer`, `hyph_pointer`, `save_pointer`, and any
subrange such as `0..max_selector`.  Tokens are `i32` too.

| WEB | Rust |
|---|---|
| integer-like, subrange | `i32` |
| `boolean` | `bool` |
| `real`, `glue_ratio` | `f64` (must be f64 to match the oracle's rounding) |
| `longinteger` | `i64`; a parameter is `impl Into<i64>`, so callers pass `i32` or `i64` with no cast |
| `four_quarters` | `FourQuarters`: `b0`..`b3` are public `i32` fields |
| `memory_word`, `two_halves`, `fmemory_word` | `Word` |
| `list_state_record`, `in_state_record`, ... | `ListStateRecord`, `InStateRecord`, `ObjEntry`, `VfStackRecord`, `DestNameEntry`, `PdfLinkStackRecord` |
| `alpha_file`, `byte_file`, `word_file` | `FileId`, a small `i32` (`LOG_FILE`, `read_file(n)`, ...) |

```pascal
procedure foo(p:pointer; s:small_number; x:scaled; b:boolean);
var q:pointer; g:glue_ratio; i:four_quarters;
```
```rust
pub fn foo(&mut self, p: i32, s: i32, x: i32, b: bool) {
    let mut q: i32; let g: f64; let i: FourQuarters;
```

Do not invent newtypes, do not use `usize` or `u32` for WEB values, and do not write casts between integer types: the array wrappers
take `i32` (section 12).  There are no Rust enums for WEB codes: command codes, node types and so on are `i32` constants.  The one exception to the
no-cast rule is an intermediate that must be `i64`, because the oracle's `integer` is 64 bits: [section 11](#11-pascal-arithmetic).

Pascal locals may be uninitialised; Rust locals may not be read before assignment, so give `let mut x = 0;` where the compiler
requires it.  A WEB function's result is a return value; where the function name is assigned and execution continues, use a
local `let mut result`.  Globals remain globals: `cur_val` stays `self.cur_val`, not a return value.

Be careful with a local that shadows a global.  `old_setting`, `l`, `k`, `c`, `f`, `g`, `p`, `first` and others exist as both.  Read
the procedure's `var` part (and any `@<Local variables...@>` module): a declared local is a Rust `let`, otherwise it is `self.x`.

## 5. Globals

Every WEB global is a field of `#[repr(C)] struct Tex` in `state.rs`, with its WEB name: `self.cur_val`, `self.eqtb`, `self.mem`.
Rust keywords are renamed: see section 6.

**Not fields:**

- File variables (`log_file`, `read_file`, ...) are `FileId`s, in Host: section 18.
- `help_line`: `self.host.help_line`.  `help_ptr` stays a field.
- `mem_bot`, `mem_top`, `mem_min`, `mem_max` are constants: `MEM_BOT` (1), `MEM_TOP`, `MEM_MIN`, `MEM_MAX`.
- `yzmem`, `zmem`, `zeqtb`, `yhash`: the arrays are simply `mem`, `eqtb`, `hash`.
- `pdf_buf`, a pointer in WEB, is replaced by `pdf_buf_is_os: bool`, which selects `pdf_os_buf` or `pdf_op_buf`.
- `dump_name`, `bound_name`, `bound_default`, `TEX_format_default`, `output_comment` are in Host or gone.

web2c's size variables (`pool_size`, `max_strings`, `stack_size`, `buf_size`, `error_line`, ...) remain `i32` fields, set at startup,
at most their capacities (`POOL_CAP`, `STR_CAP`, `STACK_CAP`, ...).  Keep their tests as written: `if self.pool_ptr + l > self.pool_size`.

**Adding a global.**  Do not touch `state.rs`.  Each module owns a POD struct in its own file, embedded in `Tex` as `v_<module>`:

```rust
// in mlist.rs
crate::pod_struct! {
    /// POD state private to this module.  Add fields here, not to state.rs.
    pub struct Vars {
        pub my_counter: i32,
        pub my_table: A<i32, 256>,
    }
}
// use: self.v_mlist.my_counter
```

So adding one conflicts with nobody.  Fields must be integers, floats, `bool`, `Word`, `A<..>`, or other `pod_struct!`s, because the image starts as zeroed
memory and is snapshotted: no pointers, no `Vec`, no `String`, no references.  An all-zero value must be a valid initial state; that is
why `pod_struct!` checks every field.  Keep `Vars` small: it is copied with every snapshot.  State that was a C static or a `^` array and may
be large goes in the arena: `let v: PodVec<T> = self.arena_vec(cap)`, `self.arena_push(&mut v, x)`, `v.get(&mut self.arena, i)`, storing the `PodVec`
in `Vars`.  The arena is a bump allocator of `ARENA_CAP` words, 512 MiB, untouched until used.  It gives nothing back: `arena_push` doubles a vector that is full, in place if it is the last thing in the arena, and otherwise by moving it past what is there, which leaves its old place as a hole.  So a vector that grows alone uses what it holds, and two that grow in turn use up to twice that.  `avl_nodes` is the largest there is: 20 bytes for each of up to 8388607 objects.  Rarely, a module needs state that cannot be POD and need not survive a snapshot: `HVars` in the same file, reached as
`self.host.hv.mlist`.

**The rule:** if it must survive a snapshot, it lives in the image.

## 6. Accessors

`access.rs` has a method for every WEB macro that is an expression or a simple statement over globals.  Use them.

```pascal
link(p):=q;  q:=link(link(p));  type(p):=glue_node;  width(r):=width(q)+x;
incr(glue_ref_count(g));  cur_list.mode_field:=0;  tail:=p;  loc:=loc+1;  tolerance:=200;
```
```rust
self.set_link(p, q);  q = self.link(self.link(p));  self.set_ty(p, GLUE_NODE);  self.set_width(r, self.width(q) + x);
self.set_glue_ref_count(g, self.glue_ref_count(g) + 1);  self.cur_list.mode_field = 0;  self.set_tail(p);  self.set_loc(self.loc() + 1);  self.set_tolerance(200);
```

- Keywords are renamed: **`type` is `ty`**, **`box` is `box_reg`**.  So `box(n)` is `self.box_reg(n)`, `box(n):=p` is `self.set_box_reg(n, p)`.
- Parameterless macros are methods too: `head`, `tail`, `mode`, `aux`, `prev_depth`, `space_factor`, `clang`, `loc`, `state`, `start`, `limit`, `name`, `index`, `token_type`, `param_start`,
  every `int_par`, `dimen_par` and `glue_par` name (`self.hbadness()`, `self.set_hbadness(x)`), `every_par`, `cur_font`, and so on.
- Aliases are provided at each name: `font`, `character`, `fam`, `glue_ptr`, `leader_ptr`, `pre_break`, `math_type`, `node_size`, `span_count`, ...
- A macro over a four-quarters value takes it: `self.char_tag(i)`, `self.skip_byte(cur_i)`.  You may also read `i.b0` directly.
- Direct word access: `mem[p].hh.rh` is `self.mem[p].rh()`, `mem[p].hh.b0` is `self.mem[p].b0()`, `mem[p].sc` is `self.mem[p].sc()`, `mem[p].int:=x` is `self.mem[p].set_int(x)`, `mem[p].gr` is
  `self.mem[p].gr()`, `mem[p].qqqq` is `self.mem[p].qqqq()`, `mem[p].qqqq.b0` is `self.mem[p].q0()`.  Whole-word copies are plain: `mem[r]:=mem[p]` is `self.mem[r] = self.mem[p];`.
- **`.hh` copies are whole-word copies**: `mem[a].hh:=mem[b].hh` is `self.mem[a] = self.mem[b];`, and `mem[p].hh:=empty_field` is `self.mem[p] = self.empty_field;`.  A word here is exactly an `hh`.

**Borrow rules for `self`.**  `self.f(self.g())` compiles when `g` takes `&self` (two-phase borrows), and is an error when `g` takes `&mut self`: bind it first.
`self.mem[p].set_sc(self.width(q))` is an error, because the `IndexMut` receiver is taken before the argument: use the accessor `self.set_width(..)`, or bind:

```rust
let v = self.get_avail();  self.set_link(p, v);       // not self.set_link(p, self.get_avail())
let w = self.width(q);     self.mem[p].set_sc(w);
```

Fast node construction may use whole-word stores, to avoid a partial store followed by a wide load: `self.mem[p] = Word::hb(link, ty, subtype);`, `Word::hh(rh, lh)`.

`qi`, `qo`, `hi`, `ho`, `si` and `so` are the identity: drop them.  `min_quarterword` and `min_halfword` are 0.

**Quarterwords.**  `b0`, `b1` and `qqqq.b*` are `u16` in storage, `i32` at the accessor.  A store truncates; no WEB code stores a negative there (audited, see `raw.rs`).  Halfword fields are plain `i32`, so a negative value
(`link(p):=-1`, `info(p)` holding a count) just works.  `MAX_QUARTERWORD` is the storage limit, `0xFFFF`.  **The three semantic limits use `WEB_MAX_QUARTERWORD`, 255, because they are observable**:
§296 (grouping levels: `if self.cur_level == WEB_MAX_QUARTERWORD { self.overflow(b"grouping levels", WEB_MAX_QUARTERWORD) }`), §973-974 (the 256 spans test and `link(end_span)`), §1298 (`replace_count`).

`null` is `NULL`, which is 0.  The oracle's `null` is `-0xFFFFFFF`, so watch for change-file code that compares with `null` a field where 0 is a legitimate number.  `tex.web` itself works with 0.  One such field is known, `pdf_dest_xyz_zoom`: on the oracle `zoom 0` writes `/XYZ 72 72 0.0` and no zoom writes `/XYZ 72 72 null`.  So §1565's `:=null`, §784's `=null` and §1603's `<>null` are all `PDF_ZOOM_NULL`, which is that same `-0xFFFFFFF`, and the oracle's aliasing at `zoom -268435455` comes along (checked: it writes `null`).  The other two fields both compared with `null` and assigned numbers, `pdf_action_struct_id` and `pdf_dest_objnum`, reject values `<=0` at §1556 and §1565, so `NULL` is right for them.

**A number used as a pointer.**  `mem_bot` is 1 here and 0 in the oracle.  The null rule covers pointers, not a plain number that WEB hands to a pointer routine: §1603 prints `final_skip(p)`, a dimension, with `print_spec`, so the port passes `final_skip(p) + MEM_BOT` to read the same word as the oracle (0 is `zero_glue` there, and would be null here).  If you see WEB pass a number where a pointer goes, do the same.

## 7. Strings

There is no pool file.  A WEB string literal of more than one character becomes a byte literal; a one-character literal is an ASCII code.

| WEB | Rust |
|---|---|
| `print("abc")` | `self.print_s(b"abc")` |
| `print("c")`, one character | `self.print(b'c' as i32)` |
| `print(s)`, `s` a string number | `self.print(s)` |
| `print_char("c")` | `self.print_char(b'c' as i32)` |
| `slow_print("abc")` | `self.print_s(b"abc")` |
| `slow_print(s)` | `self.slow_print(s)` |
| `print_esc("abc")` | `self.print_esc_s(b"abc")` |
| `print_esc(s)`, `print_esc("/")` | `self.print_esc(s)`, `self.print_esc(b'/' as i32)` |
| `print_nl("abc")`, `print_nl("")` | `self.print_nl_s(b"abc")`, `self.print_nl_s(b"")` |
| `print_nl("c")`, one character | `self.print_nl(b'c' as i32)` |
| `print_nl(s)` | `self.print_nl(s)` |
| `print_err("abc")` | `self.print_err_s(b"abc")` |
| `print_ignored_err("abc")` | `self.print_ignored_err_s(b"abc")` |
| `prompt_input("abc")` | `self.prompt_input_s(b"abc")` |
| `primitive("abc",c,o)` | `self.primitive(b"abc", C, O)`; `primitive(" ",ex_space,0)` is `self.primitive(b" ", EX_SPACE, 0)` |
| `overflow("abc",n)` | `self.overflow(b"abc", n)` |
| `confusion("abc")`, `fatal_error("abc")` | `self.confusion(b"abc")`, `self.fatal_error(b"abc")` |
| `scan_keyword("abc")` | `self.scan_keyword(b"abc")` |
| `pdf_print("abc")`, `pdf_print_ln("abc")` | `self.pdf_print_s(b"abc")`, `self.pdf_print_ln_s(b"abc")` |
| `pdf_print(s)`, `pdf_print("c")` | `self.pdf_print(s)`, `self.pdf_print(b'c' as i32)` |
| `pdf_error("a","b")`, `pdf_warning("a","b",x,y)`, `pdf_warning(0,"b",x,y)` | `self.pdf_error(b"a", b"b")`, `self.pdf_warning(b"a", b"b", x, y)`, `self.pdf_warning(b"", b"b", x, y)` |
| `print_glue(x,o,"pt")`, `print_glue(x,o,0)` | `self.print_glue(x, o, b"pt")`, `self.print_glue(x, o, b"")`: the body tests `!s.is_empty()` for `s<>0` |
| `print_spec(p,"pt")`, `print_spec(p,0)` | `self.print_spec(p, b"pt")`, `self.print_spec(p, b"")` |
| `chr_cmd("abc")` | `self.chr_cmd(b"abc", chr_code)` |

Appendix B lists every parameter that is a byte slice.

**Help.**  Lines are given in reading order, as in the WEB source; the method stores them in the reverse order `error` expects.

```pascal
help2("Where was the left brace? You said something like `\def\a}',")@/
  ("which I'm going to interpret as `\def\a{}'.");
```
```rust
self.help(&[
    b"Where was the left brace? You said something like `\\def\\a}',",
    b"which I'm going to interpret as `\\def\\a{}'.",
]);
```

`help0` is `self.help(&[])`.  In a Rust byte string a backslash is `\\` and a double quote is `\"`; WEB's `""` is one quote and `@@` is one `@`.  Line by line, `help_line[0]:="x"` is `self.set_help_line(0, b"x")`, with index as in WEB; reading is `self.help_line(k)`, a `&'static [u8]`, so §90 is
`self.help_ptr -= 1; let l = self.help_line(self.help_ptr); self.print_nl_s(l);`.  `help_ptr` is an ordinary field.

**Literals that must be real string numbers**, because they are stored in or compared with `str_number` variables, are the well-known strings, made at INITEX right after the 256 single characters:
`S_EMPTY` (`""`, 256), `S_DOT_TEX`, `S_DOT_LOG`, `S_DOT_DVI`, `S_DOT_PDF`, `S_DOT_FLS`, `S_DOT_FMT`, `S_DOT_VF`, `S_TEXPUT`, `S_INITEX_IDENT`, `S_FONT`, `S_NULLFONT`, `S_NOTEXPANDED`, `S_PDFPRIMITIVE`, `S_ENDTEMPLATE`, `S_INACCESSIBLE`, `S_ENDWRITE`, `S_PDFTEXSPACE`, `S_QUESTIONS`, `S_QUESTION`, from `access.rs`.

| WEB | Rust |
|---|---|
| `cur_ext:=".tex"`, `cur_area:=""`, `if cur_ext="" then` | `self.cur_ext = S_DOT_TEX`, `self.cur_area = S_EMPTY`, `if self.cur_ext == S_EMPTY {` |
| `pack_job_name(".log")`, `pack_job_name(format_extension)` | `self.pack_job_name(S_DOT_LOG)`, `self.pack_job_name(S_DOT_FMT)` |
| `print_file_name(n,a,"")`, `read_font_info(u,nom,"",s)` | `self.print_file_name(n, a, S_EMPTY)`, `self.read_font_info(u, nom, S_EMPTY, s)` |
| `prompt_file_name("input file name","")` | `self.prompt_file_name(b"input file name", S_EMPTY)` |
| `if s="input file name" then` (`s` a byte-slice parameter) | `if s == b"input file name" {` |
| `s:="???"` in print §59 | `s = S_QUESTIONS`; a parameter `s` must be `let mut s = s` |
| `text(frozen_protection):="inaccessible"` | `self.set_text(FROZEN_PROTECTION, S_INACCESSIBLE)` |
| `text(frozen_relax):="relax"; eqtb[frozen_relax]:=eqtb[cur_val]` just after `primitive("relax",..)` | `let s = self.text(self.cur_val); self.set_text(FROZEN_RELAX, s); ...`: `primitive` already made the string |
| `format_ident:=" (INITEX)"`, `if format_ident=" (INITEX)"` | `S_INITEX_IDENT` |
| a local `s:="hbox"` used only to print, §1679 | `let s: &[u8] = b"hbox"; self.print_esc_s(s);` |
| `print(eTeX_revision)`, `wterm(banner)`, `length("PDF:")` | `self.print_s(ETEX_REVISION)`, `self.wterm_s(BANNER)`, `4`; string macros are `&[u8]` constants |
| `max_reg_help_line:="...255."` §1812 | `max_reg_help_line` is an `i32`: store 0 or 1, and select the line at §1811 |

`primitive()` copies its name into `buffer`, calls `id_lookup` and keeps the string it makes.  Other INITEX-time literals that need a number can use `self.lit_string(b"..")`, which makes a fresh string each call, so never at run time.
Do not add to the well-known table from another module.

Compact character arrays use `.at` and `.put`: `buffer[k]` is `self.buffer.at(k)`, `str_pool[k]:=c` is `self.str_pool.put(k, c)`.  See section 12.

## 8. Modules and helpers

A `@<module@>` is inlined at its use, with its section number.  Start from `tools/web.py tangle NAME`.

```rust
if self.tracing_macros() > 0 {
    // §427 Show the text of the macro being expanded
    self.begin_diagnostic();
    ...
}
```

A module used at **several sites in your file** may become a helper, with `/// §N`, named from the module and prefixed with your module's name, if it has no goto out of it.  A goto out of a helper does not exist: inline such a module, or have the helper return a value the caller branches on.  A helper's parameters are the locals it reads; locals it writes go through `&mut` or the return value.  A module used in **two files** is ported in each, inline.

Accumulated modules are part of the procedure that uses them, as in section 2.  Their pieces arrive in section order; keep that order, since the order of `@<Cases ...@>` can matter and that of initialisations does.  `@<Check the ``constant'' values for consistency@>` is not ported: capacities are checked at compile time, in `state.rs` and `tests/foundation.rs`.

Comments: keep a WEB comment only when it says something the Rust does not.  Keep WEB's names for everything.  Do not improve the algorithm; **the port is literal**.  A change of behaviour is a bug even if it looks like a fix.  Later agents diff against the oracle by the bit.

A very large procedure (`initialize`, `print_cmd_chr`, `init_prim`, `prefixed_command`) is fine as one Rust function.  Splitting a long run of plain initialisations into helpers is allowed where it reads better: name them with your prefix.

## 9. Gotos

| WEB | Rust |
|---|---|
| Forward, to a label later in the procedure: `done`, `found`, `exit` | Labelled block: `'done: { ...; break 'done; ... }`.  `goto exit` at the procedure's end is `return`, or `break 'exit` if cleanup follows. |
| Backward: `restart`, `continue` | Labelled loop: `'restart: loop { ...; continue 'restart; ...; break; }`.  End the body with `break` when control falls out. |
| Irreducible: main_control's lig/kern loop, `hyphenate`, `reconstitute` | A state machine: `let mut pc = Pc::Start; loop { match pc { Pc::Start => { ...; pc = Pc::Next; } ... } }`, with a private `enum Pc` local to the function.  (Enums are not allowed in POD, but a local control-flow enum is fine.) |

```pascal
restart: get_next; if cur_cmd=x then goto restart;
loop begin ...; if a then goto done; if b then goto found; ... end;
done: foo;
found: bar;
```
```rust
loop { self.get_next(); if self.cur_cmd != X { break; } }
'found: {
    'done: {
        loop {
            // ...
            if a { break 'done; }
            if b { break 'found; }
            // ...
        }
    }
    self.foo(); // done:
}
self.bar();     // found:
```

`return` is `goto exit`.  A `repeat S until C` is `loop { S; if C { break; } }`; a `while` is `while`.  `loop` (WEB's macro, `while true do`) is `loop`.

When a label is reached by falling through and also by goto, nest the labelled blocks so the code after each block is the code after its label, as above.  When gotos jump into the middle of a sequence, use the state machine; do not duplicate large code.

## 10. Case

A WEB `case` is a `match` on `i32`, with constants in the patterns.  A case with no `othercases` falls through silently in the oracle's C: write `_ => {}`.  `othercases x` is `_ => x`.

**A misspelt constant in a pattern compiles**, as a variable binding.  In a one-value `match`, with its `_` arm, the binding catches everything, the `_` arm is unreachable, and `#![deny(unreachable_patterns)]` reports it.  **In a tuple arm nothing does**: `(HMODE, SPACERR) => ...` catches every remaining hmode command, and the `_` arm stays reachable for the other modes.  Two gates catch that, and `coverage.py --module` requires both: every upper-case name must be a `const` of the crate, and a finished module's header denies `unused_variables`, since the binding is never used.  So write constants in upper case, as `consts.rs` has them.

```pascal
case cur_cmd of
assign_int,assign_dimen: x;
top_bot_mark: begin y; z; end;
othercases w
endcases
```
```rust
match self.cur_cmd {
    ASSIGN_INT | ASSIGN_DIMEN => x,
    TOP_BOT_MARK => { y; z; }
    _ => w,
}
```

**`abs(mode)+cur_cmd` and `state+cur_cmd`.**  Rust has no arithmetic in patterns.  Match a tuple.  It reads like the WEB, and `any_mode(x)` is `(_, X)`.

```pascal
case abs(mode)+cur_cmd of
hmode+letter,hmode+other_char,hmode+char_given: goto main_loop;
any_mode(relax),vmode+spacer,mmode+spacer,mmode+no_boundary: do_nothing;
non_math(sup_mark): insert_dollar_sign;
```
```rust
match (abs(self.mode()), self.cur_cmd) {
    (HMODE, LETTER) | (HMODE, OTHER_CHAR) | (HMODE, CHAR_GIVEN) => { ... }
    (_, RELAX) | (VMODE, SPACER) | (MMODE, SPACER) | (MMODE, NO_BOUNDARY) => {}
    (VMODE, SUP_MARK) | (HMODE, SUP_MARK) => self.insert_dollar_sign(),
    _ => {}
}
```

`any_mode(x)` is `(_, X)`, `non_math(x)` is `(VMODE, X) | (HMODE, X)`, and `any_state_plus(x)` is `(_, X)` on `(state, cat)`, or `(MID_LINE|SKIP_BLANKS|NEW_LINE, X)` if another state could occur.  `add_delims_to(state)` is `(STATE, MATH_SHIFT|TAB_MARK|MAC_PARAM|SUB_MARK|LETTER|OTHER_CHAR)`.

For other arithmetic in a pattern, use a named constant, local to the function: `const X1: i32 = X + 1;`.  Range patterns work with constants: `X..=Y`.

## 11. Pascal arithmetic

| WEB | Rust | Notes |
|---|---|---|
| `a div b` | `a / b` | Truncates toward zero, in Pascal, C and Rust alike. |
| `a mod b` | `a % b` | The sign of the dividend, in C and Rust alike. |
| `a/b` on reals, or on integers | `a as f64 / b as f64` | Pascal's `/` is real division. |
| `odd(x)` | `odd(x)` from `raw` | `x & 1 != 0`.  True for negative odd numbers too. |
| `abs(x)` | `abs(x)` from `raw` | The port never makes `i32::MIN`: `round` and `sat32` stop at -(2^31-1). |
| `half(x)` | `self.half(x)` | The WEB function §100, which rounds in its own way. |
| `round(r)` | `round(r)` from `raw` | web2c's `zround`: half away from zero, clamped to +-2147483647 (`-2147483647` for `r` too low, not `-2^31`). |
| `float(x)`, `unfloat(x)` | `x` if it is `f64`, else `x as f64` | Both are the identity on `f64`.  A `glue_set` is `f64` already.  Pascal promotes integers in a real expression; Rust needs `as f64`: `glue_temp:=float(glue_set(b))*stretch(g)` is `self.glue_set(b) * self.stretch(g) as f64`. |
| `float_constant(20000)` | `20000.0` | |
| `set_glue_ratio_zero(glue_set(p))` | `self.set_glue_set(p, 0.0)` | |
| `vet_glue(e); ... glue_temp` | `let glue_temp = vet_glue(e);` | Returns the clamped value. |
| `fabs(x)` | `x.abs()` | |
| `incr(x)`, `decr(x)` | `x += 1`, `x -= 1`, or the setter | |
| `negate(x)` | `x = -x` | |
| `double(x)` | `x = x + x` | |
| `x:=x+1` on a `longinteger` | `i64` | Mix with `as i64` only where one side is `i32`. |

**The oracle's `integer` is 64 bits, ours is 32.**  web2c's `integer` and `scaled` are C `long`, and a memory word holds one, which is why the word is 16 bytes.  So the oracle does not wrap: checked on `/usr/bin/pdftex`, `\count1=2147483647 \advance\count1 by 1` gives `2147483648`, three `\advance`s of `2147483647` give `6442450941`, and four `\hskip16383pt` in an `\hbox` give `\wd0=65532.0pt`, on which `\dimen0=\wd0` says "Dimension too large".

Rust's `i32` stays, per the type decision, and `eqtb` and nearly all of `mem` hold 32 bits.  All profiles set `overflow-checks = false`, so plain `+ - *` do not panic and are correct ported code wherever a result stays in range, which is nearly everywhere: Knuth bounds dimensions by 2^30 so that two of them add.  Do not write `wrapping_add`.  An audit run may turn checks on with `RUSTFLAGS="-C overflow-checks=on"`.

**Where WEB's arithmetic can leave the range, compute in `i64` and store with `sat32(x)` from `raw`**, which clamps to +-(2^31-1).  What the store keeps is the sign and any comparison with a value below 2^31, the "Dimension too large" test in particular, where a wrapped value would keep neither.  What it does not keep is the number, so printing it differs, and so does anything computed from it later: "Known divergences" in [section 15](#known-divergences) says exactly what.  So a stored clamp is a last resort, and a sum that is not stored as it runs is kept exact: see the row for §991 below.  These sites are known, from reading every product in the WEB and the accumulating sums.  `grep sat32` finds them all:

| § | WEB | Rust |
|---|---|---|
| 1416 | `if q=advance then cur_val:=cur_val+w` | `self.cur_val = sat32(self.cur_val as i64 + w as i64)`.  For `\count` and `\dimen`: WEB checks neither. |
| 1417 | `width(q):=width(q)+width(r)`, and `stretch`, `shrink` | the same on each.  `\advance\skip` is unchecked too. |
| 483 | `f:=(1000*f+@'200000*remainder) div mag` | `f = ((1000 * f as i64 + 65536 * self.remainder as i64) / self.mag() as i64) as i32`; the result fits.  §484's `num*f` stays below 2^31. |
| 823-833, 844-848 | `hpack`, `vpackage`: `x`, `h`, `d`, `w`, `s` | Those are `i64` locals, so `x += self.width(p) as i64`.  Store with `self.set_width(r, sat32(w))`, and the same for `height`, `depth`.  `h`, `w` parameters are `i32`, widened on entry. |
| 818, 824, 832, 847 | `total_stretch`, `total_shrink` | **`A<i64, 4>` fields**, exact and not clamped: their ratio gives `glue_set`, and a clamped total would move the glue.  `self.total_stretch[o] += self.stretch(g) as i64`. |
| 834, 840, 849, 852 | `glue_set(r):=unfloat(x/total_stretch[o])` | `self.set_glue_set(r, x as f64 / self.total_stretch[o] as f64)` |
| 836, 843, 850, 853 | `badness(x,total_stretch[normal])` | `pack_badness(x, self.total_stretch[NORMAL])`, `badness` on `i64`s in `pack.rs`, also used by `linebreak.rs` and `page.rs`: on the oracle `\hbox to\maxdimen` with five `plus \maxdimen` gives 1, and clamping gives 12.  `tests/integration/pack/limits_badness.tex`. |
| 842, 852 | `print_scaled(-x-total_shrink[normal])` | `self.print_scaled(-x - self.total_shrink[NORMAL])`, as it is: `print_scaled` takes an `impl Into<i64>`, and the oracle's message says 47900.0pt. |
| 834, 840 | `divide_scaled(x,font_stretch,3)` | `self.divide_scaled(sat32(x), font_stretch, 3)` |
| 972 | `glue_stretch(u):=total_stretch[o]`, `glue_shrink(u):=total_shrink[o]` | `sat32(..)` on each store |
| 1379 | `w-total_shrink[normal]+q<=z` | `w as i64 - self.total_shrink[NORMAL] + q as i64 <= z as i64`, and `self.total_shrink[FIL] != 0` as written |
| 107, 689 | `xn_over_d`, `round_xn_over_d`: `t`, `u`, `v` | `i64` locals.  Ranges are safe for `x<2^30`, but a box dimension clamped near 2^31 reaches `xn_over_d` through `0.9\wd0`, and `u` would wrap past §107's `arith_error` test, losing "Dimension too large". |
| 689 | `divide_scaled`: `q` | an `i64` local, and `sat32` on return.  `(10*r) div m` stays in `i32`, by its own guard.  A tiny `font_stretch` makes `q` large, and `fix_int` must then see the right sign. |
| 1035-1037 | `try_break`'s `d`, `minimal_demerits`, `minimum_demerits`, `fewest_demerits`, `total_demerits` | `i64`, the last in a whole word of the active node, like `active_short`.  They stay below `awful_bad`, but with negative demerit parameters near their limit the sum over a paragraph goes below -2^31 and chooses the breaks (`linebreak/wide_demerits`, `coverage/fuzz/01881`). |
| 216 | show_box's room test `pool_ptr+depth_threshold>=pool_size` | in `i64`: `\showboxdepth=2147483647` would wrap. |
| 1163, 1185-1187 | `x_over_n(height(p),1000)*count(n)`, `x_over_n(w,count(n))*1000` | `sat32(self.x_over_n(..) as i64 * n as i64)`.  `\vsize=\maxdimen` with `\count` below 1000 reaches this, and §1187's next line then clamps to `dimen(n)-height(r)`, as in the oracle. |
| 991-1037, 1045-1047 | `active_width`, `cur_active_width`, `background`, `break_width`, `disc_width` | **`A<i64, 9>` fields, exact and not clamped**, with plain additions.  The sum is measured from the first active node, and a line's own width is that sum less the deltas before its node, so a clamped sum changes lines that are far below 2^31 themselves: with `plus 16383pt` in each space, a line with 16383pt of stretch got 2pt.  `shortfall` is a local `i64`, `badness` is `pack_badness`.  `tests/integration/linebreak/wide_sums.tex`, `etex/lastlinefit.tex`. |
| 1019-1020, 1037 | a delta node's `mem[q+k].sc` | `self.linebreak_delta(r, k)` and `self.linebreak_set_delta(q, &v)`.  A delta can pass 2^31 too, when more than that is discarded after one break.  The words stay `sc`s unless one needs more; then `subtype(q)`, which WEB does not use, is 1 and all are whole words through `Word::raw`.  The test is one predictable branch per node, and an `sc` leaves `lh` alone, as the oracle's does: section 15, on stale words. |
| 1842-1853 | `active_short`, `active_glue`, `best_pl_short`, `best_pl_glue`, `g` | `linebreak_active_short(p)` and the like, always a whole word; `A<i64, 4>` fields; a local `i64`.  §1846's `fract` takes them whole, and §1853 stores with `sat32`. |
| 1147-1153 | `active_height`, `cur_height` | `active_height` is `active_width`, so it is exact too, and `cur_height` is written `self.active_width[1]`.  §1151 stores `best_height_plus_depth` with `sat32`. |
| 103 | `print_scaled(s)` | takes an `impl Into<i64>`, for the sums above: `s=` and `g=` in `\tracingparagraphs`, `Overfull`. |
| 1799 | `fract(x,n,d,max_answer)` | `x`, `n`, `d` are `impl Into<i64>` and the body is in `i64`, as on the oracle; the result is at most `max_answer`. |
| 875-943 | `mlist.rs`: what `make_over`, `make_under`, `make_vcenter`, `make_radical`, `make_fraction`, `make_op`, `make_scripts`, `make_left_right` and `var_delimiter` add, and `3*t` | `i64` locals, or `add_sat` and `sub_sat` of that file, and `sat32` on store.  A font parameter or `\delimiterfactor` can be anything.  `abs(math_x_height*4)` is in `i64`, and §938's `(delta1 div 500)*delimiter_factor` reaches `var_delimiter` clamped. |
| 1426 | `get_node(2*n+1)` | `self.get_node(sat32(2 * n as i64 + 1))`: `\parshape 1073741824` would wrap and ask for a negative size, and the oracle overflows main memory. |
| 1222 | `width(main_p):=width(main_p)+extra_space(cur_font)` | the sum in `i64`, `sat32` on store.  `\fontdimen7` can be anything. |
| 1265 | `depth(cur_box):=depth(cur_box)-h+height(cur_box)` | the same: a `\vtop`'s depth. |
| 1278 | `height(tail):=height(p)+depth(p)` | the same: an insertion's height. |
| 1303 | `shift_amount(p):=x-h`, `round((w-a)/2+h*t-x*s)`, `new_kern(-a-delta)` | the same on each: the accent's position, with a font's `\fontdimen` as big as it likes. |
| 1067 | `pen` | a local `i64`, `self.new_penalty(sat32(pen))`: four penalties of 2^31-1 add up on the oracle. |
| 1144-1205 | `page.rs`: `page_total`, `page_goal`, `page_so_far`, `least_page_cost`, `insert_penalties`, an insertion's `height` | each sum in `i64`, `sat32` on store, not only the products above: three `\maxdimen` rules with no break between them would turn `page_total` negative and move the break.  Those are read by `\pagetotal` and the like, so they are not wide. |
| 1125 | `trie_node`'s `abs(trie_c[p]+1009*trie_o[p]+2718*trie_l[p]+3142*trie_r[p]) mod trie_size` | the key in `i64`, as in the oracle.  `3142*trie_r[p]` passes 2^31 at the default `trie_size`, and a wrapped key would hash the node elsewhere. |

For a site not listed, the same rule: **a local `i64` in a routine that is not the inner token loop costs nothing and is never less faithful than `i32`.**  A sum that is not stored as it runs, or one that comes back, may be a wide field, as in `WIDE_GLOBALS` of `tools/webgen.py`: that is `total_stretch`, `total_shrink` and line breaking's sums and no others, and adding to it means a macro's accessor goes too (`SKIP_MACROS`).  A `Word` has room for an `i64`, and only `linebreak.rs` uses that, in nodes nothing else reads; `.sc()` everywhere else is 32 bits.  `x_over_n`, `nx_plus_y`, `mult_integers`, `badness`, §471, §479 and the `\numexpr` code (§1788-1799) guard their own arithmetic; port them as they are.

**`ext_xn_over_d` is in doubles**, as in utils.c, not 64-bit integers.  `r>DBL_EPSILON` adds one half and otherwise it subtracts one half, then it truncates; there is a warning at +-`maxinteger` and no `arith_error`.  Font expansion is mandatory and depends on that rounding: r=0.64 gives 1.  It is written, in `pack.rs`.

Division by zero panics in Rust and traps in the oracle.  WEB guards it; do not add guards.

`mult_and_add` (§105) stays literal `i32`.  Its guard `max_answer-y` wraps on `xn_over_d`'s arith_error path, where `y` is clamped, and an `overflow-checks=on` audit flags that line; without checks every oracle row on those paths matches, and widening it made one row wrong.  Also `half` (§100) uses `i64` for `x+1`.

`nx_plus_y(n,x,y)` is `self.nx_plus_y(n, x, y)`; `mult_integers(n,x)` is `self.mult_integers(n, x)`.

## 12. Arrays and records

`A<T, N>` is a zero-based fixed array, indexed by `i32` directly.  `AO<T, LO, N>` has a lower bound `LO`.  Both are unchecked in release and bounds-checked under `--features checked`.  **WEB's subscripts stay literal, with no casts:**

```rust
self.eqtb[p]          self.mem[p]          self.font_size[f]          self.active_width[1]          self.page_so_far[6]
self.xeq_level[p]     // AO: array[int_base..eqtb_size]
self.trie_op_hash[h]  // AO: array[neg_trie_op_size..trie_op_size]
```

A Pascal `array[1..n]` is `A<T, {n+1}>` with slot 0 unused.  An array over `very_loose_fit..tight_fit` or another named range uses the constants themselves.

**Compact storage** (`u8`, `u16` arrays) goes through `.at(i)` and `.put(i, v)`, which take and return `i32`; indexing them gives a `u8`, which will not type-check where an `i32` is meant.  They are `buffer`, `str_pool`, `name_of_file`, `xord`, `xchr`, `xprn`, `dvi_buf`, `pdf_op_buf`, `pdf_os_buf`, `trie_c`, `trie_trc`, and `pdf_char_used[f]` (32 bytes per font; `self.pdf_char_used[f].at(i)`).  Everything else is `i32`, `bool`, `Word` or a record, and is indexed directly, apart from `WIDE_GLOBALS`, which are `i64` (section 11).

Records are plain structs with WEB's field names: `self.cur_input.loc_field`, `self.nest[p].mode_field`, `self.input_stack[self.base_ptr].name_field`, `self.obj_tab[k].int2`, `self.vf_stack[s].stack_h`, `self.dest_names[k].objname`, `self.pdf_link_stack[k].link_node`.  Whole-record assignment copies: `input_stack[input_ptr]:=cur_input` is `self.input_stack[self.input_ptr] = self.cur_input;`.  `four_quarters` values are `FourQuarters` with `i32` fields.

## 13. For loops

```pascal
for k:=a to b do S       for k:=a downto b do S
```
```rust
for k in a..=b { S }     for k in (b..=a).rev() { S }
```

Pascal and the oracle's C evaluate `b` once, before the loop, as Rust does.  Do **not** use `..=` when any of these holds, and write the C translation out instead:

- **`k` is read after the loop.**  In the oracle's C, the loop is `k=a; for_end=b; if (k<=for_end) do S while (k++<for_end);`, so after normal exit `k` is `b+1` (`b-1` for `downto`) if the body ran at all, and `a` if it did not; after a goto, `k` is its current value.  Read the code after the loop.
- **The body assigns to `k`**, which Pascal forbids but web2c permits.

```rust
let mut k = a; let e = b;
while k <= e { S; k += 1; }
```

A loop variable that is a global, like `font_k` in `for font_k:=font_base to font_max do`, cannot be the Rust loop variable: write the `while` form with `self.font_k`.

## 14. Booleans and conditions

`and`, `or`, `not` on booleans are `&&`, `||`, `!`.  web2c emits C's `&&` and `||`, which short-circuit, so the oracle short-circuits: `&&` and `||` are exactly right, side effects included, even though Pascal does not promise it.

The only `and` on integers is §1536 `(pdf_action_named_id(p) and 1)=1`; `access.rs` writes WEB's evident meaning, `&`.  How web2c translates it was not checked; the effect of any difference is at most which token list gets freed.

`if x then` where `x` is `cinttype` (`file_line_error_style_p`, `shellenabledp`, ...): those fields are `i32`; write `x != 0`.

`init ... tini`: compiled always; INITEX is one binary.  `Init ... Tini` is `if self.ini_version { ... }`.  `stat ... tats`: compiled always, as in the oracle.  (DESIGN's `stats` feature for `dyn_used` etc. comes later.)  `debug ... gubed`: not compiled in the oracle; drop the code, and a section with nothing else is `DROPPED`.

## 15. Token lists

Token lists do not live in `mem`.  **This is the one place where the port is not literal, and it is the most sensitive code.**  `toks.rs` has the heap; read its header.  Every site below must be converted; do not call `self.info(p)` or `self.link(p)` on a token list: it compiles and reads garbage from `mem`.

### Representation

```text
block:  [hdr][t0][t1]...[tn][0]        hdr = (rc << 8) | size_class        rc = refs - 1, so rc 0 is one reference
TokPtr: i32, the index of hdr in self.toks.  0 is the null list.  toks[0] = toks[1] = 0.
```

- `info(p)` for a token at position `i` is `self.toks[i]`.  `link(p)` walks are `i += 1`.  **The first token of list `p` is at `p + 1`**, and it is 0 for an empty or null list.
- A token is `cmd*256+chr`, or `CS_TOKEN_FLAG + cs`.  0 never occurs in a list, so 0 terminates it; see "Known divergences" below for the one path where WEB does store a 0, and `tb_push` asserts it under `checked`.
- **Every** list has a header: macros, arguments, backed-up tokens, templates, marks.  There are no headerless lists any more.
- The heap starts at `NULL_LIST`, above `BUF_CAP`.  The cells below it stay 0 and are in no format or checkpoint: `get_next_leaf` reads `toks[loc]` in any state, and a buffer index must read as the end of a list.  **Never write `toks[0]` or `toks[1]`.**  `flush_list` guards that; `add_token_ref` and `delete_token_ref` need non-null `p`, as below.
- `flush_list(p)` is O(1), by size class, and allows null.  `delete_token_ref(p)` and `add_token_ref(p)` need non-null `p`, as in WEB.
- Lists never move, and frees are non-LIFO, as in WEB.

### The API

```rust
let m = self.tb_mark();         // start a list on the builder stack
self.tb_push(t);                // store_new_token(t), fast_store_new_token(t)
let p = self.tb_commit(m);      // make a block, pop.  Empty gives 0, like link(temp_head)=null.  rc = 0.
let p = self.tb_commit_ref(m);  // always a block, even empty: for def_ref.  rc = 0.
let p = self.tb_commit_strip(m);// §426, without the first and last token
self.tb_discard(m);             // abandon
self.tb_len(m);                 // tokens since the mark
self.tb_push_list(p);           // append list p (or null)
self.tb_snapshot(m);            // a copy of the in-flight list, without popping: for runaway

let m = self.tx_mark();         // a transient list: only §398, §433 and §1767, see below
self.tx_push(t); self.tx_len(m);
let p = self.tx_commit(m);      // a block, or 0 when empty
self.tx_discard(m);

self.tok_one(t); self.tok_two(t0, t1);   // small lists, rc = 0
self.tok_copy(p);                        // a fresh copy; null gives null
self.tok_prepend(p, t); self.tok_append(p, t);  // new list replacing p
self.tok_concat(q, r);                   // q then r; both consumed
self.tok_len(p); self.tok_is_empty(p);   // tok_is_empty(null) is true
self.back_prepend(t);                    // §348, §393, §394
self.at_list_end();                      // loc=null, in token-list state
self.rec_at_list_end(r);                 // the same for a saved level, r = input_stack[k]: §1309

self.flush_list(p); self.delete_token_ref(p); self.add_token_ref(p);
self.token_ref_count(p); self.set_token_ref_count(p, r);
```

The builder is a stack.  Nesting is LIFO because nested expansion finishes before the outer builder resumes: a `get_x_token` between two `tb_push`es may build, commit and discard above your mark, and leaves `tb_ptr` where it was.  A mark never survives past the function that took it.

**`tb` holds only what `runaway` may show.**  `runaway` displays `tb[mark..tb_ptr]`, so whenever `get_next` can run, the top of `tb` must be the list named by the current `scanner_status`.  Three sites break this if they use `tb`, because they call `get_x_token` with tokens in flight while the status stays that of an enclosing list: `\csname` §398, `\ifcsname` §1767 and `scan_keyword` §433.  In WEB their tokens are on a list of their own, which `def_ref`, `temp_head` and `hold_head` never see.  With `\edef\a{x\csname y` and the end of the file the oracle prints `Runaway definition?` and `->x`, not `->xy`.  So those three use the **transient stack** `tx`.  Nothing else does.

### Input stack conventions

```rust
// begin_token_list(p, t), §345.  Same for all types: start=p, a header or 0.
self.push_input();
self.set_state(TOKEN_LIST); self.set_start(p); self.set_token_type(t);
if t >= MACRO {
    self.add_token_ref(p);
    if t == MACRO { self.set_param_start(self.param_ptr); } else { self.set_loc(p + 1); /* tracing_macros */ }
} else {
    self.set_loc(p + 1);        // was loc:=p, since the list had no header; p=0 reads as end of list
}
```

`loc` is the position of the next token.  **`loc=null` becomes `self.at_list_end()`**, which is `state==TOKEN_LIST && toks[loc]==0`.  The state test is required, even where WEB dropped it (locnull-optimize.ch): in a file level `loc` is a buffer index and `toks[loc]` is an unrelated cell.  Every site, including the one on `input_stack`:

| § | WEB | Rust |
|---|---|---|
| 334 | `(token_type<>backed_up) or (loc<>null)` | `self.token_type() != BACKED_UP \|\| !self.at_list_end()`; the state is known there |
| 336 | `backed_up: if loc=null` | `if self.at_list_end()` |
| 347 | `while (loc=null) and (token_type<>v_template) and (token_type<>output_text)` | `while self.at_list_end() && ...` |
| 379 | `if loc<>null then begin t:=info(loc); loc:=link(loc);` | `let t = self.toks[self.loc()]; if t != 0 { self.set_loc(self.loc() + 1);` |
| 380 | `cur_cs:=info(loc)-cs_token_flag; loc:=null` | `self.cur_cs = self.toks[self.loc()] - CS_TOKEN_FLAG; self.set_loc(NULL);`; 0 reads as end of list |
| 416 | as §347 | as §347 |
| 1203 | `(state=token_list)and(loc=null)and(token_type=backed_up)` | `self.at_list_end() && self.token_type() == BACKED_UP` |
| 1203 | `(state<>token_list)or(loc<>null)or(token_type<>output_text)` | `!self.at_list_end() \|\| self.token_type() != OUTPUT_TEXT` |
| 1204 | `repeat get_token; until loc=null` | `loop { self.get_token(); if self.at_list_end() { break; } }`; loops forever on a file, as in WEB |
| 1309 | `input_stack[base_ptr].loc_field=null` and `<>null`, each with `state_field=token_list` | `self.rec_at_list_end(r)`, below.  A literal port compiles, and then raises the "interwoven" fatal error at the first `&` or `\cr` of every `\halign` |

```rust
// §1309 do_endv
self.base_ptr = self.input_ptr;
self.input_stack[self.base_ptr] = self.cur_input;
let mut r = self.input_stack[self.base_ptr];
while r.index_field != V_TEMPLATE && self.rec_at_list_end(r) {
    self.base_ptr -= 1;
    r = self.input_stack[self.base_ptr];
}
if r.index_field != V_TEMPLATE || !self.rec_at_list_end(r) {
    self.fatal_error(b"(interwoven alignment preambles are not allowed)");
}
```

`end_token_list`, §346: `flush_list(start)` for `backed_up`, `inserted`; `delete_token_ref(start)` otherwise; parameters are flushed as written.  `parameter`, `u_template`, `v_template` free nothing.

`show_context` §341: both branches become `self.show_token_list(self.start() + 1, self.loc(), 100000)`.  `start` is a header in every case.

### show_token_list

`show_token_list(p,q,l)` takes **positions of tokens**, not headers.  `p` is the first token to show, `q` is the magic position or 0.

```rust
// §314
pub fn show_token_list(&mut self, p: i32, q: i32, l: i32) {
    let mut p = p;
    let (mut match_chr, mut n) = (b'#' as i32, b'0' as i32);
    self.tally = 0;
    // toks_show_more(p): p is outside the heap, or toks[p] != 0.  p is not read if it is wild: toks is unchecked.
    while self.toks_show_more(p) && self.tally < l {
        if p == q { self.set_trick_count(); }    // §342
        // §315: the list bound is toks_top, since token lists are not in mem
        if p < 0 || p >= self.toks_top { self.print_esc_s(b"CLOBBERED."); return; }
        // ... t = self.toks[p] ...
        p += 1;
    }
    if self.toks_show_more(p) { self.print_esc_s(b"ETC."); }
}
```

| WEB | Rust |
|---|---|
| `show_token_list(link(h),null,n)`, with `h` a header | `self.show_token_list(h + 1, NULL, n)`; works for `h=0` |
| §426 `show_token_list(pstack[n-1],null,1000)` | `self.show_token_list(self.pstack[n - 1] + 1, NULL, 1000)` |
| §341 `show_token_list(start,loc,100000)`, `(link(start),loc,100000)` | both `self.show_token_list(self.start() + 1, self.loc(), 100000)` |
| §194 print_mark `(p<hi_mem_min)or(p>mem_end)` | `p <= 0 \|\| p >= self.toks_top`.  0 is below the oracle's `hi_mem_min`: it prints `\pdfrefobj{\CLOBBERED.}` for `\pdfobj reserveobjnum` then `\pdfrefobj` of it. |
| §241, §251 `show_token_list(link(equiv(n)),null,32)` | `self.show_token_list(self.equiv(n) + 1, NULL, 32)` |
| §317 token_show | `if p != NULL { self.show_token_list(p + 1, NULL, 10000000); }` |
| `token_show(temp_head)` §1475, §1753 | see below: `self.token_show(p)` on the committed list |
| §727, §1615 `show_token_list(link(h),null,pool_size-pool_ptr)` | `self.show_token_list(h + 1, NULL, self.pool_size - self.pool_ptr)` |
| `tokens_to_string(p)` §686 | the same: `p + 1` |

When `loc` is at the terminator, as `q`, `p==q` is never true inside the loop, the same as WEB's `q=null`.

### Builders replacing temp_head, hold_head, backup_head, def_ref

`temp_head`, `hold_head` and `backup_head` remain in `mem` as **node** list heads; only their token uses go away.

**`store_new_token`:**

```pascal
store_new_token(cur_tok)    fast_store_new_token(info(r))    link(p):=get_avail; p:=link(p); info(p):=cur_tok
```
```rust
self.tb_push(self.cur_tok)  // all three: the p and q locals disappear
```

**`str_toks(b)`, `the_toks`, `scan_general_text`** returned a tail, with the head in `link(temp_head)`.  They now **append to the current builder** and return nothing, so `scan_toks` needs no copy on `\the`.

```pascal
{§490} function str_toks(b:pool_pointer):pointer; ... str_toks:=p
{§493} link(garbage):=the_toks; ins_list(link(temp_head))
{§496} link(garbage):=str_toks(b); ins_list(link(temp_head))
{§504} q:=the_toks; if link(temp_head)<>null then begin link(p):=link(temp_head); p:=q; end
{§1475} p:=the_toks; ...; token_show(temp_head); flush_list(link(temp_head))
{§1688} scan_general_text; if c=1 then the_toks:=cur_val else ... token_show(p); flush_list(p) ...
{§1753} scan_general_text; ...; token_show(temp_head); ...; flush_list(link(temp_head))
```
```rust
// §490 str_toks
self.str_room(1);
let mut k = b;
while k < self.pool_ptr {
    let t = self.str_pool.at(k);
    let t = if t == b' ' as i32 { SPACE_TOKEN } else { OTHER_TOKEN + t };
    self.tb_push(t);
    k += 1;
}
self.pool_ptr = b;

// §493 ins_the_toks
let m = self.tb_mark(); self.the_toks(); let p = self.tb_commit(m); self.ins_list(p);

// §496, and every `link(garbage):=str_toks(b); ins_list(link(temp_head))` in §497
let m = self.tb_mark(); self.str_toks(b); let p = self.tb_commit(m); self.ins_list(p);

// §504: inside scan_toks, the current builder is the list being built
self.the_toks();

// §1475 show_the
let m = self.tb_mark(); self.the_toks(); let p = self.tb_commit(m);
self.print_nl_s(b"> "); self.token_show(p); self.flush_list(p);

// §1688 in the_toks.  Here the tokens stay on the caller's builder if c=1.
if odd(self.cur_chr) {
    let c = self.cur_chr;
    if c == 1 { self.scan_general_text(); return; }
    let m = self.tb_mark();
    self.scan_general_text();
    let p = self.tb_commit(m);
    let old_setting = self.selector; self.selector = NEW_STRING; let b = self.pool_ptr;
    self.token_show(p); self.flush_list(p);
    self.selector = old_setting;
    self.str_toks(b);
    return;
}

// §492
if self.cur_val_level == IDENT_VAL { self.tb_push(CS_TOKEN_FLAG + self.cur_val); }
else if self.cur_val != NULL { self.tb_push_list(self.cur_val); }

// §1753 pseudo_start
let m = self.tb_mark(); self.scan_general_text(); let p = self.tb_commit(m);
let old_setting = self.selector; self.selector = NEW_STRING;
self.token_show(p); self.selector = old_setting; self.flush_list(p);
```

`token_show(p)` needs a header, or 0.  With an empty list `tb_commit` gives 0, and `ins_list(0)` is the same harmless empty level as `ins_list(null)`.

**`scan_toks`** returns `def_ref` (a header with `rc = 0`, never null), not the tail.  Its callers' uses of the result as a tail go away.  `def_ref` is only assigned once the list exists; while building, `tb_def_mark` is `def_ref`'s analogue.

```rust
// §499
pub fn scan_toks(&mut self, macro_def: bool, xpand: bool) -> i32 {
    self.scanner_status = if macro_def { DEFINING } else { ABSORBING };
    self.warning_index = self.cur_cs;
    let m = self.tb_mark();
    let saved_mark = self.tb_def_mark;
    self.tb_def_mark = m;
    // ... §500-§505, with self.tb_push(..) for store_new_token ...
    // found:
    self.scanner_status = NORMAL;
    if hash_brace != 0 { self.tb_push(hash_brace); }
    self.def_ref = self.tb_commit_ref(m);
    self.tb_def_mark = saved_mark;
    self.def_ref
}
```

| WEB | Rust |
|---|---|
| `p:=scan_toks(false,true); write_tokens(tail):=def_ref` | `self.scan_toks(false, true); self.set_write_tokens(self.tail(), self.def_ref);` |
| `link(garbage):=scan_toks(false,false); flush_list(def_ref)` | `let d = self.scan_toks(false, false); self.flush_list(d);` |
| `call_func(scan_toks(false,true))` | `self.scan_toks(false, true);` |
| §504 `info(link(cur_chr))=protected_token` | `self.toks[self.cur_chr + 1] == PROTECTED_TOKEN`; null_list reads 0 |
| §1396 `if j<>0 then begin q:=get_avail; info(q):=j; link(q):=link(def_ref); link(def_ref):=q; end` | `if j != 0 { self.def_ref = self.tok_prepend(self.def_ref, j); }` |
| §1404 `if link(def_ref)=null then begin sa_define(p,null)(p,undefined_cs,null); free_avail(def_ref); end` | `if self.tok_is_empty(self.def_ref) { self.sa_define(a, e, p, NULL, p, UNDEFINED_CS, NULL); self.flush_list(self.def_ref); }` |
| §1404 `link(q):=get_avail; q:=link(q); info(q):=right_brace_token+"}"; q:=get_avail; info(q):=left_brace_token+"{"; link(q):=link(def_ref); link(def_ref):=q;` | `let d = self.tok_append(self.def_ref, RIGHT_BRACE_TOKEN + b'}' as i32); self.def_ref = self.tok_prepend(d, LEFT_BRACE_TOKEN + b'{' as i32);` |
| §1466 shift_case | `self.scan_toks(false, false); let mut i = self.def_ref + 1; while self.toks[i] != 0 { /* §1467: change self.toks[i] */ i += 1; } self.back_list(self.def_ref);` |
| §497 `ins_list(link(def_ref)); free_avail(def_ref)` | `self.ins_list(self.def_ref);`: the block carries its own header; `end_token_list` frees it |
| §497 `s:=tokens_to_string(def_ref); delete_token_ref(def_ref); def_ref:=save_def_ref;` | as written |

`save_def_ref:=def_ref` and the like stay as written.  Save and restore of the mark are inside `scan_toks` and `scan_general_text`.

```rust
// §1683 scan_general_text: appends to the caller's builder.
let (s, w, d) = (self.scanner_status, self.warning_index, self.def_ref);
self.scanner_status = ABSORBING; self.warning_index = self.cur_cs;
let saved_mark = self.tb_def_mark; self.tb_def_mark = self.tb_mark();
self.scan_left_brace();
let mut unbalance = 1;
loop {
    self.get_token();
    if self.cur_tok < RIGHT_BRACE_LIMIT {
        if self.cur_cmd < RIGHT_BRACE { unbalance += 1; }
        else { unbalance -= 1; if unbalance == 0 { break; } }
    }
    self.tb_push(self.cur_tok);
}
self.tb_def_mark = saved_mark;
self.cur_val = TEMP_HEAD;    // WEB leaves a pointer here, and §497 can read it: see below
self.scanner_status = s; self.warning_index = w; self.def_ref = d;
```

`cur_val` means nothing after this, in WEB's callers or ours, with one exception.  §497's `\pdfcolorstackinit` sets `cur_val` to the literal mode, then scans a text; `\unexpanded` or `\detokenize` in that text leaves WEB's pointer there, and the oracle then stops at shipout with "This can't happen (literal1)".  Any non-mode value reproduces that, so `TEMP_HEAD`, which is what WEB assigns for an empty list.

**`read_toks`, §508:**

```rust
self.scanner_status = DEFINING; self.warning_index = r;
let mk = self.tb_mark(); let saved_mark = self.tb_def_mark; self.tb_def_mark = mk;
self.tb_push(END_MATCH_TOKEN);
// ... §509, §1761, with self.tb_push(self.cur_tok) ...
self.def_ref = self.tb_commit_ref(mk);
self.tb_def_mark = saved_mark;
self.cur_val = self.def_ref; self.scanner_status = NORMAL; self.align_state = s;
```

**`runaway`, §328**, reads the in-flight list:

```rust
let m = match self.scanner_status {
    DEFINING => { self.print_nl_s(b"Runaway definition"); self.tb_def_mark }
    MATCHING => { self.print_nl_s(b"Runaway argument"); self.tb_match_mark }
    ALIGNING => { self.print_nl_s(b"Runaway preamble"); self.tb_align_mark }
    _ /* ABSORBING */ => { self.print_nl_s(b"Runaway text"); self.tb_def_mark }
};
self.print_char(b'?' as i32); self.print_ln();
let p = self.tb_snapshot(m);
self.show_token_list(p + 1, NULL, self.error_line - 10);
self.flush_list(p);
```

**`scan_keyword`, §433.**  `backup_head` and `backup_backup` (§388) go away.  Its tokens go on `tx`:

```rust
pub fn scan_keyword(&mut self, s: &[u8]) -> bool {
    let m = self.tx_mark();
    let save_cur_cs = self.cur_cs;
    let mut k = 0;
    while k < s.len() {
        self.get_x_token();                   // recursion is possible here
        let c = s[k] as i32;
        if self.cur_cs == 0 && (self.cur_chr == c || self.cur_chr == c - b'a' as i32 + b'A' as i32) {
            self.tx_push(self.cur_tok);
            k += 1;
        } else if self.cur_cmd != SPACER || self.tx_len(m) != 0 {
            self.back_input();
            if self.tx_len(m) != 0 { let p = self.tx_commit(m); self.back_list(p); }
            self.cur_cs = save_cur_cs;
            return false;
        }
    }
    self.tx_discard(m);
    true
}
```

**`\csname`, §398, and `\ifcsname`, §1767**, keep their characters on `tx`:

```rust
let m = self.tx_mark();
// ... repeat get_x_token; if cur_cs=0 then tx_push(cur_tok); until cur_cs<>0 ...
// §400: j:=first; p:=link(r); while p<>null do ...
let mut j = self.first;
let mut i = m;
while i < self.tx_ptr {
    // ... buffer overflow test ...
    self.buffer.put(j, self.tx[i] % 256);
    j += 1; i += 1;
}
// ... the lookup ...
self.tx_discard(m);      // flush_list(r)
```

**Backed-up and inserted tokens:**

| § | WEB | Rust |
|---|---|---|
| 347 back_input | `p:=get_avail; info(p):=cur_tok; ...; start:=p; token_type:=backed_up; loc:=p` | `let p = self.tok_one(self.cur_tok); ...; self.set_start(p); self.set_token_type(BACKED_UP); self.set_loc(p + 1);` |
| 348 | `if a then begin p:=get_avail; info(p):=cur_tok; link(p):=loc; loc:=p; start:=p; ...` | `if a { self.back_prepend(self.cur_tok); ...` |
| 359 | `p:=get_avail; info(p):=cs_token_flag+cur_cs; back_list(p)` | `let p = self.tok_one(CS_TOKEN_FLAG + self.cur_cs); self.back_list(p);` |
| 361 | `info(p):=...; aligning: q:=p; p:=get_avail; link(p):=q; info(p):=cs_token_flag+frozen_cr; ...; ins_list(p)` | compute the token(s); `let p = self.tok_one(t);` or `self.tok_two(CS_TOKEN_FLAG + FROZEN_CR, RIGHT_BRACE_TOKEN + b'}' as i32);`, then `self.ins_list(p);` |
| 393 `\noexpand` | `back_input; if (t>=cs_token_flag)and(t<>end_write_token) then begin p:=get_avail; info(p):=cs_token_flag+frozen_dont_expand; link(p):=loc; start:=p; loc:=p; end` | `self.back_input(); if t >= CS_TOKEN_FLAG && t != END_WRITE_TOKEN { self.back_prepend(CS_TOKEN_FLAG + FROZEN_DONT_EXPAND); }`: the result is one fresh two-token block |
| 394 `\pdfprimitive` | as §393, with `frozen_primitive` | `self.back_input(); self.back_prepend(CS_TOKEN_FLAG + FROZEN_PRIMITIVE);` |
| 1242 off_save | `p:=get_avail; link(temp_head):=p; ...; ins_list(link(temp_head))` | §1243 computes the tokens: `let p = self.tok_one(t);` or `self.tok_two(CS_TOKEN_FLAG + FROZEN_RIGHT, OTHER_TOKEN + b'.' as i32);`.  Print what §1243 prints.  Then `self.ins_list(p);` |
| 1618 | `q:=get_avail; info(q):=right_brace_token+"}"; r:=get_avail; link(q):=r; info(r):=end_write_token; ins_list(q); begin_token_list(write_tokens(p),write_text); q:=get_avail; info(q):=left_brace_token+"{"; ins_list(q);` | `let q = self.tok_two(RIGHT_BRACE_TOKEN + b'}' as i32, END_WRITE_TOKEN); self.ins_list(q); self.begin_token_list(self.write_tokens(p), WRITE_TEXT); let q = self.tok_one(LEFT_BRACE_TOKEN + b'{' as i32); self.ins_list(q);` |
| 1577 concat_tokens | | `self.tok_concat(q, r)` |

### Reading a macro's list

| § | WEB | Rust |
|---|---|---|
| 504, 1473, 1772 | `info(link(x))=protected_token` | `self.toks[x + 1] == PROTECTED_TOKEN`; null_list reads 0 |
| 1828 | `if link(sa_bot_mark(q))=null then {an empty token list}` | `if self.tok_is_empty(self.sa_bot_mark(q)) {`.  A literal port compiles, reads `mem` at a `toks` index, and on a garbage 0 deletes a non-empty mark, so `\topmarks n` is lost on the next page: expl3 uses mark classes |

§534, Test if two macro texts match:

```pascal
p:=link(cur_chr); q:=link(equiv(n));
if p=q then b:=true
else begin while (p<>null)and(q<>null) do
    if info(p)<>info(q) then p:=null
    else begin p:=link(p); q:=link(q); end;
  b:=((p=null)and(q=null));
  end;
```
```rust
let (mut p, mut q) = (self.cur_chr + 1, self.equiv(n) + 1);
if p == q {
    b = true;
} else {
    while self.toks[p] != 0 && self.toks[q] != 0 && self.toks[p] == self.toks[q] { p += 1; q += 1; }
    b = self.toks[p] == 0 && self.toks[q] == 0;
}
```

### macro_call

```rust
// §415 macro_call.  r, s, t, u, v are positions in toks; a walk `link(x)` is x + 1.
let ref_count = self.cur_chr;
let mut r = ref_count + 1;
let mut n = 0;
'exit: {
    if self.tracing_macros() > 0 { /* §427: token_show(ref_count) */ }
    if self.toks[r] == PROTECTED_TOKEN { r += 1; }
    if self.toks[r] != END_MATCH_TOKEN {
        // §417
        self.scanner_status = MATCHING;
        let mut unbalance = 0;
        self.long_state = self.eq_type(self.cur_cs);
        if self.long_state >= OUTER_CALL { self.long_state -= 2; }
        loop {
            let mark = self.tb_mark();        // link(temp_head):=null
            self.tb_match_mark = mark;
            let s;
            let mut m = 0;
            let mut match_chr = 0;
            if self.toks[r] > MATCH_TOKEN + 255 || self.toks[r] < MATCH_TOKEN {
                s = NULL;
            } else {
                match_chr = self.toks[r] - MATCH_TOKEN;
                s = r + 1;
                r = s;
            }
            // §418.  `continue:` is the top of this loop.
            'found: loop {
                self.get_token();
                if self.cur_tok == self.toks[r] {     // §420
                    r += 1;
                    if self.toks[r] >= MATCH_TOKEN && self.toks[r] <= END_MATCH_TOKEN {
                        if self.cur_tok < LEFT_BRACE_LIMIT { self.align_state -= 1; }
                        break 'found;
                    }
                    continue;
                }
                if s != r {                           // §423
                    if s == NULL {                    // §424
                        // print_err, help4 ...
                        self.error();
                        break 'exit;
                    }
                    let mut t = s;
                    loop {                            // repeat ... until t=r
                        self.tb_push(self.toks[t]);
                        m += 1;
                        let (mut u, mut v) = (t + 1, s);
                        loop {
                            if u == r {
                                if self.cur_tok != self.toks[v] { break; }
                                r = v + 1;
                                continue 'found;
                            }
                            if self.toks[u] != self.toks[v] { break; }
                            u += 1;
                            v += 1;
                        }
                        t += 1;                       // done:
                        if t == r { break; }
                    }
                    r = s;
                }
                if self.cur_tok == self.par_token && self.long_state != LONG_CALL {
                    self.macrocall_runaway_abort(mark, n, unbalance);  // §422
                    break 'exit;
                }
                if self.cur_tok < RIGHT_BRACE_LIMIT {
                    if self.cur_tok < LEFT_BRACE_LIMIT {   // §425
                        unbalance = 1;
                        loop {
                            self.tb_push(self.cur_tok);
                            self.get_token();
                            if self.cur_tok == self.par_token && self.long_state != LONG_CALL {
                                self.macrocall_runaway_abort(mark, n, unbalance);
                                break 'exit;
                            }
                            if self.cur_tok < RIGHT_BRACE_LIMIT {
                                if self.cur_tok < LEFT_BRACE_LIMIT { unbalance += 1; }
                                else { unbalance -= 1; if unbalance == 0 { break; } }
                            }
                        }
                        self.tb_push(self.cur_tok);   // done1: the right brace
                    } else {                          // §421
                        // back_input, print_err, help6, ins_error ...
                        continue;
                    }
                } else {                              // §419
                    if self.cur_tok == SPACE_TOKEN
                        && self.toks[r] <= END_MATCH_TOKEN && self.toks[r] >= MATCH_TOKEN { continue; }
                    self.tb_push(self.cur_tok);
                }
                m += 1;
                if self.toks[r] > END_MATCH_TOKEN { continue; }
                if self.toks[r] < MATCH_TOKEN { continue; }
                break 'found;
            }
            // found:
            if s != NULL {
                // §426.  rbrace_ptr is gone: the last token is at tb_ptr-1, and m=1 means the list is one token or
                // one group, so a last token below right_brace_limit is the group's closing brace.
                self.pstack[n] = if m == 1 && self.tb[self.tb_ptr - 1] < RIGHT_BRACE_LIMIT {
                    self.tb_commit_strip(mark)
                } else {
                    self.tb_commit(mark)
                };
                n += 1;
                if self.tracing_macros() > 0 && (self.tracing_stack_levels() == 0 || self.input_ptr < self.tracing_stack_levels()) {
                    self.begin_diagnostic();
                    self.print_nl(match_chr);
                    self.print_int(n);
                    self.print_s(b"<-");
                    self.show_token_list(self.pstack[n - 1] + 1, NULL, 1000);
                    self.end_diagnostic(false);
                }
            }
            if self.toks[r] == END_MATCH_TOKEN { break; }
        }
    }
    // §416
    while self.at_list_end() && self.token_type() != V_TEMPLATE && self.token_type() != OUTPUT_TEXT {
        self.end_token_list();
    }
    self.begin_token_list(ref_count, MACRO);
    self.set_name(self.warning_index);
    self.set_loc(r + 1);
    // ... param_stack as written; n's entries are headers ...
}
// exit:
self.scanner_status = save_scanner_status;
self.warning_index = save_warning_index;
```

§422: `pstack[n]:=link(temp_head); align_state:=align_state-unbalance; for m:=0 to n do flush_list(pstack[m]); return` is: the runaway message, then `self.tb_discard(mark); self.align_state -= unbalance; for k in 0..n { self.flush_list(self.pstack[k]); }`.  The list in flight has no block yet.

A macro's block stays valid during matching, as in WEB: `get_token` does not execute anything.

### Templates

| § | WEB | Rust |
|---|---|---|
| 959 | `p:=hold_head; link(p):=null; ...; link(p):=get_avail; p:=link(p); info(p):=cur_tok; ...; (p=hold_head)` | `let m = self.tb_mark(); self.tb_align_mark = m; ...; self.tb_push(self.cur_tok); ...; (self.tb_len(m) == 0)` |
| 955 | `u_part(cur_align):=link(hold_head)` | `let p = self.tb_commit(m); self.set_u_part(self.cur_align, p);`; null when empty |
| 960 | `p:=hold_head; link(p):=null` at its start | as §959: a new mark, in `tb_align_mark` too |
| 960 | `done2: link(p):=get_avail; p:=link(p); info(p):=end_template_token` | `self.tb_push(END_TEMPLATE_TOKEN);`, then commit into `v_part` |
| 964, 965 | `begin_token_list(u_part(cur_align),u_template)` | as written; `u_part` is a header or 0 |
| 965 | `begin_token_list(omit_template,v_template)` | as written: `OMIT_TEMPLATE` is a reserved list in `toks` |
| 966 | `info(omit_template):=end_template_token` | done by `toks_init`; the section lands in `initialize`, so `DROPPED` in print.rs |
| 956 | `equiv(frozen_endv):=null_list` | as written: `NULL_LIST` is a reserved list in `toks` |
| 970 | the copy loops | `let p1 = self.tok_copy(self.u_part(self.cur_loop)); self.set_u_part(p, p1);` and the same for `v_part` |
| 977 | `flush_list(u_part(q)); flush_list(v_part(q))` | as written |

`toks_init` is part of `@<Initialize table entries@>`: `initialize` calls it, in INITEX mode.

### Reference counts and everything else

`token_ref_count(x)` is `self.token_ref_count(x)`.  §1156 `token_ref_count(split_first_mark):=token_ref_count(split_first_mark)+2`, is `self.set_token_ref_count(m, self.token_ref_count(m) + 2);`.  §221 `add_token_ref`.  §218 `delete_token_ref`.  These, and all sites holding a list in a node or an `eqtb` entry (`mark_ptr`, `write_tokens`, `pdf_literal_data`, `equiv` of a `call` or `toks` register, `sa_ptr`, `sa_top_mark` and the other `sa_*_mark`, `obj_*_attr`, `cur_mark`), need no change: the field simply holds a header in `toks`.  Storing, comparing with `null` and passing such a field is fine.  **`link(f)` or `info(f)` of one is not**, and a grep of those over every such field finds §241, 251, 341, 492, 497, 504, 534, 727, 1396, 1404, 1466, 1473, 1772, 1828 and 1880, each with its row or sample in this section.

`link(temp_head)` where the list is **nodes**, not tokens, stays as written.  Sites using `temp_head` for nodes: §930, 936, 992, 1055, 1057, 1063, 1145, 1372, 1374, 1707, 1709, 1734, 1737, 1738.  Sites using `hold_head` for nodes: §984, 1082-1094, 1191-1194.  `backup_head` has no use left.

### Summary of token-use sites

The merged WEB's token-list sites are, by section: 194, 241, 251, 314-318, 328, 334, 336, 341, 345-348, 359, 361, 379-381, 388, 393, 394, 398, 399, 412, 416-427, 433, 490-493, 496, 497, 499-505, 508, 509, 534, 686, 727, 955, 959, 960, 964-966, 970, 977, 1137, 1156, 1189, 1193, 1203, 1204, 1242, 1243, 1309, 1396, 1404, 1405, 1457, 1466, 1473, 1475, 1519, 1532-1534, 1537-1540, 1544, 1548, 1552, 1556, 1558, 1563, 1565, 1566, 1577-1582, 1587-1591, 1599, 1603-1605, 1615, 1617, 1618, 1630, 1637, 1683, 1688, 1753, 1761, 1767, 1772, 1823, 1826-1831, 1838, 1880.  If you own one of these, your section is listed here because of tokens: check it against this document.

### Known divergences

Each is deliberate, and a difference from the oracle in one of these is not a bug in your port.  None shows in the reference logs of TRIP and e-TRIP, by grep: no `\BAD.`, and no printed value at or above 2^31.  Rung -1 is the real check.  `tests/integration/known_divergences.txt` lists the cases that show one, each with its reason, and `run.py` keeps those apart: the 2^31 clamp, `mlist/limits_wide`, `pack/limits` and `scan/dimen_units`, and the version string in the job id, `pdfout/resname` and `pdfship/uniqueres`.  Any other difference is a bug.

- **Values at or above 2^31, stored.**  The oracle holds them, since its `integer` is 64 bits, in `eqtb`, in a node's word and in every global.  The port has 32 bits in all of those but the sums of section 11, and clamps to +-(2^31-1).  So a stored value at or above 2^31 is another number, and it does not differ only when printed.  Each of these was run on the oracle:
  - **Printing it.**  `\count1=2147483647 \advance\count1 by 1 \the\count1` is `2147483648` there and `2147483647` here.  `\hbox{\hskip 16383pt\hskip 16383pt\hskip 16383pt}` has `\the\wd0` `49149.0pt` there and `32767.99998pt` here, and `\showbox` shows the same, as it does for a `\kern35000.0`.
  - **Arithmetic on it.**  `\dimen0=.5\wd0` of that box is "Dimension too large" there, and here it is `16383.99998pt` with no error; `.25\wd0` is `12287.25pt` there and `8191.99998pt` here.
  - **A sum that comes back through it.**  `\advance\count1 by 1 \advance\count1 by -2` from 2147483647 ends at 2147483646 there and 2147483645 here.  `\hbox{\copy0\hskip-16383pt\hskip-16383pt}` is `16383.0pt` wide there and `1.99998pt` here, while `\hbox{\unhcopy0\hskip-16383pt\hskip-16383pt}` is right on both, since `hpack`'s `x` is an `i64`.
  - **§1790's "Arithmetic overflow".**  `f>infinity` cannot be true of a clamped `\count`, so `\numexpr\count1\relax` after that `\advance` is "Arithmetic overflow" and 0 there, and 2147483647 here with no error.
  - **§986's glue set.**  `glue_stretch` and `glue_shrink` of an unset node are words, stored with `sat32` at §972, so `(t-width(r))/glue_stretch(r)` has another divisor for a column with more than 2^31 of stretch: `glue set 0.3332` there and `0.49979` here.  This moves glue in the output.
  - **`var_delimiter`'s `v`.**  §938's `(delta1 div 500)*delimiter_factor` reaches it clamped, and the box it builds has dimensions in words: `\delimiterfactor=2000000` with a `\left(` around a rule of 20pt and 20pt gives a delimiter of 90000pt there and of 32768pt here.  This is typeset output too, though it takes such a factor.

  What stays the same: the sign, and any comparison with a value below 2^31.  `\ifdim\wd0>16383pt` is true on both and `\dimen0=\wd0` is "Dimension too large" on both.  And what is not stored stays exact: the glue totals and the excess in `hpack` and `vpack`, so `glue_set` and badness are right and an `Overfull` message says the true number; and line breaking's and `vert_break`'s running sums with their deltas, which can pass 2^31 as far as the oracle's can, so that every `b=`, `d=`, `s=` and `g=` is the oracle's (`linebreak/wide_sums`).
- **`xn_over_d`'s result at 2^31.**  Same class.  On its arith_error path WEB returns `u` without the last division, which can reach 2^31; the port returns `sat32(u)`.  `scan_dimen` reports the error either way, but `mu_mult` uses the value: `\fontdimen6=-294912sp` with a `\muskip` of 2^31-2, which only `\advance` can make, gives `\mskip` width 1073741823 on the oracle and 0 here.
- **The version string in the job id.**  The banner says `(rstex)` where the oracle's says `(TeX Live 2026/Arch Linux)`, which `run.py` masks.  `\pdfuniqueresname`'s prefix cannot be masked: it is the CRC of the job id (utils.c `setjobid` and `getresnameprefix`), which holds that string, so `/F1QUkwD0` here is `/F1i47QM4` there.  With the oracle's string in `VERSION_STRING`, both cases agree.
- **A line box whose glue fields were never set.**  §834 and §840 return from `hpack(cal_expand_ratio)` before setting `glue_sign`, `glue_order` and `glue_set`.  §823 goes on to a second `hpack` if `font_expand_ratio<>0`; if it rounds to 0, because the excess is below a two-thousandth of the font stretch, which for a line at `\pdfadjustspacing=2` is a few thousandths of a point, the box stays the line's as it is.  §1713 shows the box in any case when there is an LR problem.  So those three fields are whatever the words held, on the oracle too, and `show_box` and `hlist_out` read them.  This cannot be followed exactly, because the words differ.  The oracle's has `lh` and `gr` in 8 bytes and `rh` and `sc` in another 8, since its `halfword` is as wide as its `integer`, its `null` is negative, and its pointers to hi-mem have other values; here `sc` shares `gr`'s low half, `lh` is `gr`'s high half, `null` is 0, and an old `null` or pointer in those words gives another `glue_set`, or `glue_sign`.  What the port does keep: `b0` and `b1` overlap `lh` as in `texmfmem.h`, so an old `lh` reads as `glue_sign` 0 on both (`pack/font_expansion`, `pack/expansion_unset_glue`), and an old box's glue set reads the same unless another node has been there since.  Over 40 made-up documents of 300 such lines each, all with old boxes of every kind around, the log differs in 38 and the PDF in 6.  Making the fields 0 in that path is worse: 31 of the 40 PDFs differ, since the oracle does use what was there when it is a real glue set.  A different PDF needs a stale `glue_sign` that is not 0 and a `glue_order` that some glue in the line has, since §653 and §735 take any sign but stretching for shrinking, and the effect is on that line's glue alone.
- **Numbers in overflow messages.**  There is no pool file, so `init_str_ptr` is far smaller: "number of strings=N" and likely "pool size=N" differ.  `HASH_SIZE` is 0, so "hash size=N" prints `hash_extra` where the oracle prints 15000+`hash_extra`.  The stop happens at the same kind of condition, not at the same count.  Token lists are not in `mem`, so what fills `main_memory` on the oracle fills the token heap or the builders here: the message says main memory all the same, with the runaway text before it, but it comes at another token, and where the input stack stands shows that (`coverage/babel-option/limbu-limb` and others with no end to a definition).
- **The token from no control sequence.**  `\edef\x{aa\ifnum -1<\pdfprimitive\fi 2 bb\fi cc}` gives the oracle's runaway `->aa\relax \BAD.2 bbcc}`.  §394 leaves `cur_cs=0`, §405 (or §404, by `\pdfprimitive\input` during a file name) backs up `cs_token_flag+0`, and the reread takes `cur_cmd`, `cur_chr` from `eqtb[0]`, so WEB stores the token 0, which here would end the list.  So `Tex::alloc` sets `equiv(0)=1`, and that token is 1 here: cmd 0, not a terminator.  It displays as `\BAD.` in `show_token_list`, reads back as cmd 0, counts as an open brace in §503's test, as WEB's 0 does, and §405's backed-up form still displays `\IMPOSSIBLE.`.  What remains: `\if` compares its `cur_chr`, 1 where the oracle has 0.  It needs `\pdfprimitive\fi` before a condition has been evaluated, and then that very token in `\if`.  `eqtb[0]` is outside every region, §240 starts at `active_base`, and is not in the eqtb section's live range, so nothing else sees it and a load leaves it alone.

- **The rectangle of a destination that has none.**  `do_dest` §1637 under a matrix (`\pdfsetmatrix`, so a `\scalebox` or a `\rotatebox`) calls `set_rect_dimens` with `pdf_width(p)`, `pdf_height(p)` and `pdf_depth(p)`, and §1565 only sets those for `fitr`: for `xyz`, `fith`, `fitv`, `fitbh` and `fitbv` they are what the words held before `get_node` gave them out, which is a read of uninitialised memory on the oracle too, and `/XYZ`'s `y` comes out `cur_v` moved by `max(h,-d)` of those words, scaled, and `x` by `min(0,w)`.  `arxiv/cs.NI/1908.01405v1` is such a document: hyperref puts a destination on each numbered line of a `lstlisting` in a `\scalebox`, and 4 of its 23 end 0.1 to 0.5 bp away; its PDF has a link that lands a little higher on the oracle, and no text moved.  The words are close enough that most of them agree: `rh` and `sc` are one place on both, and old widths and kerns give the same numbers (`pdfship/dest_stale_words`).  What cannot agree without another word layout: an old `null` is 0 here and -268435455 there, so a depth of `null` lifts the destination by 4096pt on the oracle (`pdfship/dest_stale_null`); an old pointer has another value, one more here into variable-size memory, where `mem_bot` is 1 for its 0 (read off under `\pdfsetmatrix{100 0 0 100}`, which shows that one sp in `y`), and any other into hi-mem or to a token list, since those live elsewhere here; and the word of a `glue_set` holds the bits of a `double` here and an older `rh` there.  That last one alone could be had by keeping `gr` in the `lh` of two words, which was tried: the four destinations stay as they are.  In those four the word is a link of the free list, about 2375 here and about 44000 on the oracle by the `y` it gives, which is more than the one, and where that comes from was not found.  So it is accepted, and only the destination's rectangle is affected: `do_annot`, `do_link` and `do_thread` have their rule spec set by §1552 and §1556.

## 16. One-word lists in mem

`mem` keeps `get_avail` one-word cells for what is not a token list.  These stay exactly as written, with `self.info`, `self.link`, `self.get_avail()`, `self.free_avail(p)`, `let p = self.fast_get_avail()`:

char nodes (§609, 885, 1085, 1088, 1211, 1215), `init_list` (§1080, §1085: the characters of a ligature), `margin_char` (§224, 822, 823, 1288), `head` of the nest (§234, 235), copy heads (§222), `scan_dimen`'s digits (§478), the PDF lists built by `append_ptr` (§700), `cur_head`, `cur_pre_head` (§948), hyphenation exception lists (§1115), the LR stack (§1705, §1727), and pseudo files (§1754-1757).

**`flush_list` for those is `mem_flush_list`**: §764, §765 (`pdf_font_list`, `pdf_obj_list`, ...), §1080 (`init_list`), §1274 (`LR_save`).  Every other WEB `flush_list` is on a token list.

## 17. Errors

`overflow`, `confusion`, `fatal_error`, `pdf_error`, `succumb` and `jump_out` return `!`, and end in `process::exit` through `self.uexit`.  There is no `Result` in the engine and no unwinding.  `error()` is an ordinary call and return, as in WEB, and may insert tokens.  So code after `self.confusion(b"x")` is dead; code after `self.error()` runs.

A panic is a bug.  `panic = "abort"` in release; a `todo!` you hit means a module is missing.

## 18. Printing and files

All side effects go through `self.host` and its `World`.  **No `std::fs`, `std::env`, `std::process`, `std::io`, `std::time`, `println!` anywhere but `host.rs` and `raw.rs`.**  `crates/engine/tests/foundation.rs` checks this by grep.  Leaf crates (kpse, fontfile, pdfio, image) are pure functions the engine calls; they never call back.

| WEB | Rust |
|---|---|
| `wterm(c)`, `wterm('abc')`, `wterm_ln('abc')`, `wterm_cr` | `self.wterm_char(c)`, `self.wterm_s(b"abc")`, `self.wterm_ln_s(b"abc")`, `self.wterm_cr()` |
| `wlog(c)`, `wlog('abc')`, `wlog_ln('abc')`, `wlog_cr` | `self.wlog_char(c)`, `self.wlog_s(b"abc")`, `self.wlog_ln_s(b"abc")`, `self.wlog_cr()` |
| `write(write_file[j],c)`, `write_ln(write_file[j])` | `self.write_file_char(j, c)`, `self.write_file_ln(j)` |
| `update_terminal`, `clear_terminal`, `wake_up_terminal` | `self.update_terminal()`, `self.clear_terminal()`, `self.wake_up_terminal()` |
| `xchr[s]`, `xord[c]`, `xprn[k]` | `self.xchr.at(s)`, `self.xord.at(c)`, `self.xprn.at(k)` |

§58 print_char: `term_and_log: begin wterm(xchr[s]); wlog(xchr[s]); ...` is `let c = self.xchr.at(s); self.wterm_char(c); self.wlog_char(c); ...`, and `othercases write(write_file[selector],xchr[s])` is `self.write_file_char(self.selector, c)`.

File variables are `FileId`: `LOG_FILE`, `DVI_FILE`, `PDF_FILE`, `TFM_FILE`, `VF_FILE`, `FMT_FILE`, `TERM_IN`, `read_file(n)`, `write_file(j)`, `input_file(k)`, `self.cur_file()`.

| WEB | Rust (in `host.rs`) |
|---|---|
| `a_open_in(cur_file,kpse_tex_format)`, `a_open_out(write_file[j])`, `b_open_in(tfm_file)`, `b_open_out(dvi_file)`, `w_open_in(fmt_file)`, `w_open_out(fmt_file)` | `self.a_open_in(self.cur_file(), KPSE_TEX_FORMAT)`, `self.a_open_out(write_file(j))`, `self.b_open_in(TFM_FILE)`, `self.b_open_out(DVI_FILE)`, `self.w_open_in(FMT_FILE)`, `self.w_open_out(FMT_FILE)`.  All read `name_of_file` and update it, as openclose.c does. |
| `a_close(f)`, `b_close(f)`, `w_close(f)` | the same names |
| `input_ln(f,true)`, `input_ln(term_in,true)` | `self.input_ln(f, true)`, `self.input_ln(TERM_IN, true)` |
| `getc(tfm_file)`, `feof(tfm_file)`, `eof(f)` | `self.getc(TFM_FILE)`, `self.feof(TFM_FILE)`, `self.feof(f)` |
| `fget`, `fbyte` (§590) | `self.tfm_temp = self.getc(TFM_FILE)`, `self.tfm_temp` |
| `kpse_in_name_ok(stringcast(name_of_file+1))`, `kpse_out_name_ok(..)` | `self.kpse_in_name_ok()`, `self.kpse_out_name_ok()` |
| `date_and_time(sys_time,sys_day,sys_month,sys_year)` | `let (t, d, m, y) = self.date_and_time();` honours `SOURCE_DATE_EPOCH`, `FORCE_SOURCE_DATE` |
| `seconds_and_micros(epochseconds,microseconds)` | `self.seconds_and_micros()` |
| `runsystem(cmd)` | `self.runsystem(bytes)`; marks the run hard-tainted |
| `uexit(n)` | `self.uexit(n)` |
| `put_byte(b,f)`, `write_dvi(a,b)` | `self.put_byte(b, f)`, or `self.put_bytes(f, slice)` |
| `getc` on a closed file | -1, not a crash |

**Observations.**  Lookups, reads, stats, clock, env and shell go through `self.host.world` and are recorded: this is what makes snapshots valid (DESIGN section 7).  A new kind of external read (say an image header) must go through `world`.  A volatile read in `scan_something_internal` or `conv_toks` (`\time`, `\pdfcreationdate`, the random seed, ...) comes with a call to `self.host.world.mark_taint(..)`; M5 refines these.

**OS differences** go through `crate::platform`: `reserve_zeroed`, `commit_range`, `broken_time`, `shell`, `path_from_bytes`, `path_to_bytes`, `env_var`, `file_stat`, `is_dir_sep`, `is_absolute`, `publish`.  File names are bytes in TeX; `platform` converts at the boundary.  The cache directory is `rstex_kpse::cache_dir`, and `rstex_kpse::open_private` and `create_private` open what is in it: DESIGN section 7 says why.

**PDF output** is a chunk list, `PdfOut`, with no seek-back: `self.pdf_out_file()`, `write(bytes)`, `offset()`, `defer(n)` for a field whose value is not yet known such as `/Length`, `fill(d, bytes)`, `finish()`.  `pdf_flush` writes `pdf_op_buf` into it.  Keep pdfTeX's object numbering so that oracle diffs work.

M1a built this properly as `rstex_pdfio::ChunkWriter`.  `PdfOut` in `host.rs` duplicates it and lacks what §685 needs: `write_stream_length(length, offset)` takes an absolute offset, and `ChunkWriter::patch` provides that, also across chunks.  **`host.rs` wraps `ChunkWriter` instead of duplicating it.**

**Compressed streams are always deflated whole.**  zlib-rs's output depends on where the pieces begin, at levels 0, 1, 5 and 6 (38 of 240 cases), so writing pieces as `writezip.c` does and compressing in parallel workers as DESIGN section 9 has it would give different bytes for the same document.  So `pdf_flush` in stream mode appends to one buffer that `pdf_end_stream` deflates in a single call, or hands whole to a worker, with the same bytes (`crates/pdfio/src/workers.rs`).  Until `pdfout_settle` puts them in the file, `pdf_gone`, `pdf_offset` and `obj_offset` do not count what the workers have: the top of `pdfout.rs`.  Whoever wants those numbers right, such as a checkpoint taken after a shipout, calls it first, outside a stream.  Output is reproducible; it is not byte-equal to the oracle's compressed bytes and never was, so compare decompressed.

**Terminal input** marks the run hard-tainted.  `t_open_in` takes the first line from `self.host.command_line`, which the CLI sets.

## 19. Dropped features

| Feature | How |
|---|---|
| **encTeX** | Not in the merged WEB. |
| **MLTeX** | §1881-1894 are `DROPPED`.  `mltex_p` and `mltex_enabled_p` are always false: fields stay zero, and tests on them are dropped.  `if mltex_p then primitive(...)` is dropped.  `effective_char(e,f,c)` returns `c`; `char_info(f)(c)` is `self.char_info(f, c)`, which ignores MLTeX; §1889-1892's accent code is dead.  `char_sub_code` etc. stay in `eqtb`'s layout, so the region is unused. |
| **IPC** | `ifdef('IPC') ... endif('IPC')` is dropped.  `ipc_on` stays zero. |
| **src-specials** | `src_specials_p` is always false; `insert_src_special`, `append_src_special` are never called; §1930 is `DROPPED`, apart from `get_nullstr` and the rest of its code, which stay. |
| **TCX reader** | `read_tcx_file` installs the hardcoded cp227 printable table.  TRIP uses the default table, §49, so the table is chosen by a flag. |
| **mktex\* but `mktexpk`** | `mktextfm`, `mktexmf`, `mktextex` and `mktexfmt` do not run: a missing TFM, `.tex` or format stays missing, as under `MKTEXTFM=0` and so on.  `-mktex=tfm` and the like set a flag nothing reads. |
| **JPX, DCT, CCITTFax decoding inside an included PDF's content array** | `crates/image` stops with a clear failure.  Not dropped from the oracle's behaviour on ordinary inclusion, where streams copy through. |

**Not dropped after all.**  M1a ported TrueType, OTF, subfonts, PNG, JPEG, JBIG2 and PDF inclusion in `crates/fontfile` and `crates/image`, each compared with the oracle.  `pdffont.rs` and `ext.rs` call them, like everything else in those crates.  VF is ported in `pdfship.rs`/`pdffont.rs` as WEB has it.

**PK and Type 3 are ported too.**  `writet3.c` is `pdffont/writet3.rs`, a child module of `pdffont.rs`, for its private helpers: `.pgc` files, `writepk`, and `getpkcharwidth`, which `adv_char_width` calls.  `pkin.c`'s `readchar` is `pk_read_char` in `crates/fontfile`, stateless: the place in the file is the caller's.  `tex-glyph.c`, `tex-make.c` and `magstep.c` are `crates/kpse/src/glyph.rs`: the search by `cmr10.600pk` and `dpi600/cmr10.pk`, the tolerance of `dpi/500+1`, `texfonts.map`, `PDFTEXSIZES` or `TEXSIZES`, and `mktexpk`, which is enabled by §792 at the lowest level, so that `MKTEXPK` in the environment or `texmf.cnf` overrules it and `-mktex` and `-no-mktex` overrule those.  §1443's `kpse_make_tex_discard_errors` is in `v_pdffont`, so it survives a snapshot: it follows `\batchmode` and such, not `-interaction`, as in WEB.  It runs through `std::process` with what `xputenv` set in its environment, stdin empty, stdout its answer and stderr ours; `kpathsea: Running mktexpk ...` and what METAFONT prints go to stderr, not the log, and the bitmaps go where the script puts them, by default `$VARTEXFONTS/pk/MODE/...`, `~/.texlive/texmf-var` here.  If it fails, `missfont.log` gets the command, as in `kpathsea`, and the job ends with `Font NAME at DPI not found`.  The tests set `MKTEXPK=0`, or a `TEXMFVAR` of their own (`tests/integration/README.md`).  There is no primitive `\pdfpkfixeddpi` in 1.40.29: on the oracle it is an undefined control sequence, and here too.

Do not delete WEB's branches for them silently: mark sections `DROPPED`, or leave a `// dropped: MLTeX` comment where a line goes away.

## 20. Notes per module

**`print.rs`.**  `initialize` carries every `@<Set initial values...@>` and `@<Initialize table entries...@>`; `toks_init` is called first in the table part.

**`hash.rs`.**  Rewritten per DESIGN section 4; the behaviour of `id_lookup` stays, and the `no_new_control_sequence` result stays.  **`id_lookup` runs while a string is in progress**, because `scan_file_name` calls `get_x_token` between `more_name` calls: on the oracle `\input t8\csname brandnewname\endcsname` opens `t8` and creates `\brandnewname`.  So keep §279's `str_room(l); d:=cur_length`, the move of the current string up by `l`, the append, `text(p):=make_string` and `pool_ptr:=pool_ptr+d` exactly as written; without them the two strings merge, corrupting both the file name and the control sequence's name.  Open addressing in `self.hash_tab` with `(hash32 << 32) | p` entries, word-at-a-time hash, growth by rehash up to `HASH_TAB_CAP`, no deletes.  `HASH_SIZE` is 0, so `FROZEN_CONTROL_SEQUENCE == HASH_BASE`.  A new control sequence is `p = EQTB_SIZE + 1 + self.hash_high`; increment `hash_high`, initialise `self.eqtb[p]` as §240 would (`eq_type=UNDEFINED_CS`, `equiv=NULL`, `eq_level=LEVEL_ZERO`), set `text(p)`, increment `cs_count`; overflow when `p > eqtb_top`.  `hash_used` no longer matters.  `next` does not exist.  `prim_lookup` keeps its small chained table as written.  All tests like `p>eqtb_size` in `tex.ch` are already right, since they handle `hash_extra` the same way.

**`eqtb.rs`.**  §240's region-2 loop stops at `EQTB_SIZE`; it never touches run-time entries.  Region layout comes from `consts.rs`.  `eqtb_top` is `EQTB_CAP - 1`.

**`mem.rs`.**  `is_char_node(p)` is `p >= MEM_SPLIT`, a compile-time constant, not `p >= hi_mem_min`.  `hi_mem_min` remains the low-water mark of hi memory (one-word cells grow down from `MEM_TOP`), and `lo_mem_max` the high-water mark of lo memory.  So get_avail §138's overflow test is `hi_mem_min < MEM_SPLIT`, get_node §143's and §144's tests against `hi_mem_min` are against `MEM_SPLIT`, and the 1000-word grow-by logic is kept.  `mem_end` stays `MEM_TOP`.  §141 is `mem_flush_list`.

**`input.rs`, `getnext.rs`, `expand.rs`, `macrocall.rs`, `buildtoks.rs`**: section 15.  Port the generic paths.  The capture loops of DESIGN section 8 are M5, after TRIP passes, behind `slow_capture`.  `get_next_literal` is §363 as WEB has it, and `get_next_leaf` and `get_next_hot` go before it, DESIGN section 8: each either does all that §363 would, or nothing.  `get_token_literal` is §387, and error's §88 must call that one: it can come with `no_new_control_sequence` false, and the others leave that alone.

**`capture.rs`.**  The loops of DESIGN section 8; its header says what each may pass.  The literal loops are all still there, in `macrocall.rs`, `cond.rs`, `buildtoks.rs` and `etex.rs`, with a call in front of each read, and `slow_capture` compiles those calls out.  Whoever changes what `get_next` does to a token from a list, or gives a control sequence a new `eq_type` at or above `outer_call` other than through `eq_define` and `geq_define`, must change this file too: `any_user_outer` is set only there, and a loop that does not see it will not load `eq_type`.  `tests/integration/capture/` aims at each thing a loop stops for; run it on both builds.  `tests/capture_fuzz.py` compares random macro programs with the oracle.

**`save.rs`.**  §296 uses `WEB_MAX_QUARTERWORD`.  All else is as written, including `(p<int_base)or(p>eqtb_size)`.

**Levels that cannot show (M5).**  A level of `back_input` shows in `show_context`, unread as `<to be read again>`, and finished, on top of the stack, as `<recently read>`, until the next `get_next`.  So a read and a `back_input` may be left out only where no error can come before the next read, and where the token is not swallowed by what reads it with an error possible right after: that is why `scan_int_next_may_stay` refuses `\relax`, and why `scan_dimen` keeps its terminator when it is not `pt`, as in `100000\p@` (`Dimension too large`, with `<recently read> \p@`).  `tests/integration/scan/int_terminators.tex` and `dimen_in_pt.tex` are about this.  A level left out must not have been a new high water mark of the stack: those paths test `input_ptr` against `max_in_stack`, so that an overflow comes at the same point.  What get_next would have done to `align_state` or by §364 is why only letters, others, spaces and commands outside `tab_mark..car_ret` qualify.

**`expand.rs`.**  `expand_saving` is WEB's §388; `expand` takes the commands that scan no number without the backups.  A new way for one of those to change `cur_val`, `radix` or `cur_order` belongs in `expand_saving`.

**`strings.rs`, `web2c.rs`.**  `search_string` gives what WEB's scan gives, newest first.  The strings a format holds are in `v_web2c.str_index`, made by `str_index_make` when the format is stored, and those a run made are in an index in Host that it makes up to the string it is asked about, from `init_str_ptr`, or from 256 with no format.  Every `str_ptr += 1` outside `make_string` needs `strings_index_note_new`, as `end_name` has.  Lowering `str_ptr` needs nothing.

**`cond.rs`.**  `conditional` is what is common; `cond_other` and `cond_case` hold the rest.  §521 and §522 go through `get_node` and `free_node` as written: which node a later `get_node` gives, and what its words still hold, is read by real documents (section 15, "The rectangle of a destination that has none"), so the order of allocation is not to be changed for speed.

**`align.rs`.**  §974 uses `WEB_MAX_QUARTERWORD` in its `"256 spans"` confusion.  §973 lands in `initialize`, in `print.rs`: `link(end_span):=WEB_MAX_QUARTERWORD+1`.

**`build.rs`.**  §1298 uses `WEB_MAX_QUARTERWORD`.

**`scan.rs`, `build.rs`.**  §450, `find_effective_tail_eTeX`: the macro has locals, and each file writes its own helper, prefixed.

**`pack.rs`, `align.rs`, `mathbuild.rs`, `prefixed.rs`, `scan.rs`, `arith.rs`, `pdfout.rs`, `linebreak.rs`, `page.rs`.**  The table in section 11 has rows for you.

**`buildtoks.rs`.**  `matchstrings` and `getmatch` keep `\pdfmatch`'s C statics, `sub_match_count`, `pmatch`, `match_string` and `last_match_succeeded` from utils.c, in `v_buildtoks`.  `\pdflastmatch` reads them later, so they are state, and `match_string` goes in the arena.  The regular expressions are POSIX extended, in pure Rust.

**`pdfout.rs`.**  `pdf_buf:=pdf_os_buf` is `self.pdf_buf_is_os = true`; a read `pdf_buf[k]` selects on it.  `pdf_quick_out`, `pdf_out`, `pdf_room` are in `access.rs`.  `pdf_gone`, `pdf_save_offset` and the offset fields are `i64`.

**`pdfobj.rs`.**  `pdf_append_list(x)(l)` is `self.l = self.append_ptr(self.l, x)`; the global `pdf_append_list_arg` is unused.  `avl_*` are externals: use a `PodVec` or your own structure in the arena.

**`pdffont.rs`, `pdfship.rs`, `ext.rs`.**  Former C statics (map tree, ToUnicode table, colour stacks, image table) become `PodVec`s in the arena; `pdf_font_map[f]` is an index, not a pointer.  Leaf crates hold no state.

**`crates/fontfile`, `crates/image`.**  "`real` is `f64`" is about WEB.  The C side is not all double, and its rounding is observable, so keep `f32` where the C has `float`: `t1_scan_num` and the `FontMatrix` and `ItalicAngle` arithmetic in `writet1.c` (the output is `%g` of a `float`); `d` in `mapfile.c`, `sscanf(r, "%f %n", &d, &j)`, then `d *= 1000.0` and `(integer)(d > 0 ? d + 0.5 : d - 0.5)` for `SlantFont` and `ExtendFont`; `epdf_width`, `epdf_height`, `epdf_orig_x`, `epdf_orig_y` and `epdf_rotate` in `writeimg.c`, for PDF inclusion.  `extxnoverd` and the matrix code in `utils.c` are `double`.  Disassembling the oracle confirms single precision in `fm_scan_line`, `t1_scan_param`, `cs_store`, `read_pdf_info`, `doextension` and `loadfmtfile`.

**`tfm.rs`.**  `font_info` entries are 8 bytes, and fields of four quarters use `.set_qqqq`.  `store_four_quarters(font_info[k].qqqq)` reads through `self.getc(TFM_FILE)` into a `FourQuarters` and stores it.  `adjust(x)` is identity, since quarterwords do not need adjusting at `min_quarterword=0`, and the section still needs its marker.

**`image.rs`.  A format load is not a checkpoint restore.**  The format is flat and uses the image's layout (DESIGN section 7), but `load_fmt_file` runs in the middle of §1517: after §1512's `setup_bound_var` and `initialize`, §61's banner, and §353's `init_terminal`, so `buffer`, `first`, `last`, `cur_input`, `selector`, `history`, `ini_version`, `interaction_option`, `etex_p`, `pdf_output_option` and many more are live, and it comes before the rest of §1517.  WEB overwrites none of them, since it loads only what §1485-1507 dump.  **Neither does the port.**  Restoring `POD_SECTIONS` whole would give every `rstex hello.tex` the `\dump` run's command line, `log_opened=true`, `job_name`, `history` and `open_parens`.

So §1512 and §1517 stay literal and in WEB's order, `initialize` runs on every run, and the file holds only what WEB's holds:

- a record of scalars, a `pod_struct!` in `image.rs`, with one field for each `dump_int` and small array of §1485-1507 and §1654: `xord`, `xchr`, `xprn`, `hash_high`, `eTeX_mode`, `pool_ptr`, `str_ptr`, `lo_mem_max`, `rover`, `sa_root`, `hi_mem_min`, `avail`, `var_used`, `dyn_used`, `par_loc`, `write_loc`, `prim`, `hash_used`, `cs_count`, `fmem_ptr`, `font_ptr`, `hyph_count`, `hyph_next`, `hyph_start`, `trie_max`, `trie_op_ptr`, `trie_used`, `pdf_mem_size`, `pdf_mem_ptr`, `obj_tab_size`, `obj_ptr`, `sys_obj_ptr`, `pdf_obj_count`, `pdf_xform_count`, `pdf_ximage_count`, `head_tab` at its three indices, `pdf_last_obj`, `pdf_last_xform`, `pdf_last_ximage`, `interaction`, `format_ident`.  Add what the port has instead of WEB's structures: `toks_top`, `tok_free`, `hash_tab_size`, `hash_tab_used`, `arena_top`.  §1504's `dumpimagemeta` and `dumptounicode` put the `PodVec`s of `v_ext.images`, `v_ext.image_names` and `v_pdffont.gtu_log` in the file's extra part.  No other `v_<module>` field survives, the map tables included: the C keeps those in statics and reads the map file again on each run, and `image.rs`'s header goes through them.
- every `POD_SECTIONS` entry with `in_format`, as `(start, end, where in the file)` with `(s.live)(self)` taken at store, and the bytes after the table.  The loader takes the range from the file, not from `live`, since `hyph_size` and the like may differ in this run.  `prefix` is never in a format file.

**`store_fmt_file`** is literal apart from how bytes move, and it runs in WEB's order, since each step has effects the next one sees.  Keep §1482's group check; §1508's `format_ident` construction and opening; §1500's `\font` lines; the messages of §1487, 1489, 1496, 1498, 1502 and 1504, with their numbers computed as WEB does (`tripin.log` has these lines, and `tests/trip/run_trip.sh` masks the numbers, not the lines); §1758's `pseudo_close`; §1654's clearing of `eTeX_state`; §1489's `sort_avail`, `var_used` and `dyn_used`; §1502's `hyph_next` rule and `init_trie`; §1506's `tracing_stats:=0`.  `final_cleanup` still calls `close_files_and_terminate` afterwards, so leave the live state alone.

**`load_fmt_file`** keeps its assignments to what is not dumped: §1486's `head`, `tail`, `page_tail`; §1488's `init_str_ptr`, `init_pool_ptr` and size growth; §1490's `mem_end`; §1492's `par_token`; §1501's loops over `font_base..font_max` and `make_pdftex_banner`; §1503's recomputation of `hyph_next` from the largest occupied slot, its `op_start`, and `trie_not_ready:=false`; §1505's `int2:=-1`; §1507's `interaction_option`; §1481's `prev_depth`; and §1655's `@<Initialize variables...@>`.

Sections that no longer apply are `DROPPED`, with a reason: eqtb compression (§1493-1495), allocation.  `undump`, `dump_int` and friends have no Rust, and `image.rs` forbids `unsafe`: `pod_bytes`, `pod_bytes_mut`, `image_bytes` and `image_bytes_mut` in `raw` give the bytes.  Before reading a section into the image, call `platform::commit_range` on its range: on Windows a read into uncommitted pages fails rather than faulting.  A section may be mapped from the file instead, where the system can: `image_load_bytes`.  §1501's loop and the like that give a new image the zeros it has have no Rust where that saves touching pages: `main_init_pdf_font_arrays` says which, and `pdf_font_attr`, the one that is not zero, gets its value where a font is made.

**A preamble checkpoint (M5) is the other case**, in `checkpoint.rs`: it restores every section including `prefix`, in mid-flight, and then refreshes what DESIGN section 7 lists.  Only this one is a snapshot of everything.  An `eq_type` may hold `CK_ARMED` on top of the command while a checkpoint may yet be taken: only `get_next` and those that mask it may read one.

**`main.rs`.**  `main_body` is literal, in WEB's order: see the notes on `image.rs`.  `setup_bound_var` reads a built-in default or `texmf.cnf` through `rstex_kpse`, and clamps to the capacity constants in `state.rs`.  `xmalloc_array` and the like have no Rust: the arrays are inline.  `Tex::alloc()` gives the image; the CLI runs the engine on a 1 GiB stack, `platform::run_on_stack`.  `main` sets `eqtb_top = EQTB_CAP - 1`: nothing else does, and otherwise the first new control sequence overflows.

**`etex.rs`.**  `max_reg_help_line`: `print.rs` stores 0 for §1812's "255" line, so §1811's e-TeX mode stores 1 for "32767".  `\showgroups` prints `\insert0{` for both `\insert100{` and `\vadjust{`, because §1277's `saved(1)` is what `saved(-2)` reads: keep it.

**`ext.rs`.**  `init_start_time` (§1584) is called from `initialize`.  There is one start time, `World::init_start_time`: `\pdfcreationdate`, `/CreationDate`, `/ModDate` and `/ID` all take it through `pdfship_start_time_str`.  `\pdffilemoddate` and the like mark taint.  `scale_image` must reproduce the oracle on huge dimensions: sizes above 2^31 give "arithmetic: number too big" and it stops with "invalid image dimensions", which the section 11 clamp alone does not give.

**Wiring `crates/kpse` (`host.rs`, `main.rs`).**  None of this is automatic: `set_program_name` with the dump name, since `TEXINPUTS.pdflatex` depends on it (the default is pdftex); `set_var("TEXMF_OUTPUT_DIRECTORY", ..)` with `-output-directory`; the `kpse_init_prog` stub calls `init_prog(prefix, dpi, mode)`; `setup_bound_var` for §1512's variables; `take_recorded()` for the `.fls`, `take_warnings()` for messages; `cnf_line_env_progname` for `-cnf-line`.  `World::shell` must pass SELFAUTO*, progname, engine and `TEXMF_OUTPUT_DIRECTORY` to `\write18` children, or `\write18{kpsewhich ...}` sees a different world than under pdfTeX: kpse keeps them in a private table, so add an accessor there.

**`crates/fontfile` from `pdffont.rs`.**  `GlyphUnicodeMap` is a plain value with `parts` and `from_parts` for dump and undump: hold it in `HVars`, or materialise it from the arena, not a `PodVec` as this section said above.  The map trees, `lookup_fontmap`, `check_ff_exist`, `preset_fontmetrics`, object numbering and the rest of `writefont.c` were left to `pdffont.rs`.  The comment in `mapfile.c` about `0.5ExtendFont` is stale: this glibc skips `.8ExtendFont` as an unknown word.

**`crates/image` from `ext.rs`, `pdfship.rs`.**  Writers return a `Vec<out::Op>` replay list with `Slot` object numbers and do not call back; the replay loop and the C function behind each entry point are documented at the top of `crates/image/src/lib.rs`.  FontDescriptor and FontFile objects are the engine's to write.

**`web2c.rs`.**  §1895-1929's SyncTeX is wired through the `synctex_*` externals.  Their call sites land in other files (`dvi.rs`, `pdfship.rs`, ...), where they stay as calls.  Their bodies wait for M7, and must not panic before it, so `web2c.rs` makes them no-ops rather than `todo!`.

## Appendix A: macros

`tools/generated.json` lists them mechanically.

**Generated in `access.rs`** (581 methods, with `set_` where the macro is an lvalue): `link`, `info`, `type`, `width`, `eq_level`, `equiv`, `int_par`, `dimen_par`, `glue_par`, `text`, the whatsit fields, the pdf object fields, and all the rest that are expressions or statements over globals, including `append_char`, `flush_char`, `str_room`, `flush_string`, `tail_append`, `add_glue_ref`, `fast_delete_glue_ref`, `free_avail`, `push_input`, `pop_input`, `back_list`, `ins_list`, `check_full_save_stack`, `dvi_out`, `synch_h`, `synch_v`, `set_ff`, `set_cur_lang`, `set_hyph_index`, `pop_LR`, `push_LR`, `put_LR`, `change_box`, `set_sa_box`, `delete_action_ref`.  `set_lc_code(c)` of §1858 is renamed `set_hc0_lc_code(c)`, because `set_lc_code(c,v)` is the setter of `lc_code`.

**Hand-written in `access.rs`:**

| Macro | Method |
|---|---|
| `fast_get_avail(p)` | `let p = self.fast_get_avail();` |
| `fetch_box(p)` | `let p = self.fetch_box();` |
| `scanned_result(v)(l)` | `self.scanned_result(v, l)` |
| `char_info(f)(c)`, `orig_char_info(f)(c)` | `self.char_info(f, c)`, `self.orig_char_info(f, c)` |
| `char_width(f)(q)`, `char_italic(f)(q)`, `char_kern(f)(q)`, `lig_kern_start(f)(q)`, `lig_kern_restart(f)(q)` | same two arguments |
| `char_height(f)(hd)`, `char_depth(f)(hd)` | same; `hd` is `self.height_depth(q)` |
| `param(k)(f)`, `slant(f)`, `space(f)`, `space_stretch(f)`, `space_shrink(f)`, `x_height(f)`, `quad(f)`, `extra_space(f)` | same |
| `mathsy(k)(s)`, `math_x_height(s)`, `num1(s)`, `axis_height(s)`, ... | same |
| `mathex(k)`, `default_rule_thickness`, `big_op_spacing1`, ... | generated: they read `cur_size` |
| `define(p,t,e)`, `word_define(p,w)` | `self.define(a, p, t, e)`, `self.word_define(a, p, w)`; `a` is the local prefix |
| `sa_define(sp,sv)(p,t,e)`, `sa_word_define(p,w)`, `sa_def_box` | `self.sa_define(a, e_flag, sp, sv, p, t, e)`, `self.sa_word_define(a, e_flag, p, w)`, `self.sa_def_box(a)` |
| `append_list(a)(b)` | `self.append_list(a, b)` |
| `pdf_append_list(x)(l)` | `self.l = self.append_ptr(self.l, x)` |
| `nx_plus_y(n,x,y)`, `mult_integers(n,x)` | same |
| `scan_normal_dimen` | `self.scan_normal_dimen()` |
| `hpack(p,natural)`, `vpack(p,natural)`, `vpack(p,h,m)` | `self.hpack_natural(p)`, `self.vpack_natural(p)`, `self.vpack(p, h, m)`.  `hpack(p,w,exactly)` is `self.hpack(p, w, EXACTLY)` |
| `left_pw(p)`, `right_pw(p)` | same |
| `round_glue` | `self.round_glue(p, this_box, g_sign, g_order, &mut cur_g, &mut cur_glue)` |
| `vet_glue(x); glue_temp` | `let glue_temp = vet_glue(x);` |
| `billion` | `BILLION` |
| `is_valid_char(c)` | `self.is_valid_char(f, c)` |
| `output_one_char(c)` | `self.output_one_char(f, c)` |
| `set_box_lr(p)(x)` | `self.set_box_lr(p, x)` |
| `assign_trace(p,"x")` | `self.assign_trace(p, b"x")` |
| `pdf_print_int_ln(n)` | same, `n` is `impl Into<i64>` |
| `ensure_dvi_open`, `succumb`, `do_final_end` | same |
| `is_char_node(p)`, `text(p)`, `font_id_text(f)` | same, with setters |
| `cur_file` | `self.cur_file()` |

**Identities and control, with no Rust:** `qi qo hi ho si so`; `float unfloat float_constant`; `loop` (Rust's), `do_nothing` (`{}`), `return` (`return`), `empty` is the constant `EMPTY`; `othercases endcases`; `init tini Init Tini stat tats debug gubed`; `abort` (`goto bad_tfm`, a labelled break in `tfm.rs`); `call_func(x)` is `x;`, with `let _ =` if it warns; `any_mode non_math any_state_plus add_delims_to four_cases` (patterns, section 10); `active_height` is the array `active_width`, `trie_ref` is `trie_hash`; `const_chk setup_bound_var` (main.rs); `dump_* undump*` (image.rs); `print_ASCII` is `print`; `free`, `remainder` and `log_name` are fields of those names; `token_ref_count add_token_ref store_new_token fast_store_new_token` are section 15; `next hash_is_full prim_is_full` go with the hash.

**Left to the procedures using them**, since they use locals of their callers.  Translate as a helper (prefixed if used in two files), closure, or inline:

| Module | Macros |
|---|---|
| `input` | `begin_pseudoprint` (its `l` is a local of show_context) |
| `getnext` | `hex_to_cur_chr` |
| `scan` | `set_conversion`, `find_effective_tail` |
| `buildtoks` | `save_cur_string`, `restore_cur_string` |
| `files` | `check_quoted`, `print_quoted`, `append_to_name` |
| `tfm` | `start_font_error_message`, `read_sixteen`, `store_four_quarters`, `check_byte_range`, `current_character_being_worked_on`, `store_scaled`, `check_existence`, `adjust` |
| `pdfobj` | `is_hex_char` |
| `pdffont` | `set_char_and_font`, `vf_replace_z`, `bad_vf` |
| `pdfship`, `main` | `delete_toks(x)`: `self.delete_token_ref(x); x = NULL` |
| `pdfship` | `get_next_char` |
| `pack` | `update_adjust_list` |
| `mlist` | `mu_mult`, `choose_mlist` |
| `linebreak` | `cal_margin_kern_var`, `check_shrinkage`, `update_width`, `do_all_six`, `do_seven_eight`, `do_all_eight`, `do_one_seven_eight`, `convert_to_break_width`, `new_delta_to_break_width`, `new_delta_from_break_width`, `combine_two_deltas`, `downdate_width`, `update_active` |
| `linebreak`, `pack` | `add_char_stretch(x)(c)` is `x += self.char_stretch(f, c)`, and likewise `add_char_shrink`; `linebreak` also has `sub_char_stretch`, `sub_char_shrink`, `add_kern_stretch`, `add_kern_shrink`, `sub_kern_stretch`, `sub_kern_shrink` |
| `hyph` | `append_charnode_to_t`, `set_cur_r`, `wrap_lig`, `pop_lig_stack`, `advance_major_tail`, `clear_trie` |
| `page` | `print_plus`, `do_all_six` |
| `control` | `pack_lig`, `wrapup` |
| `build` | `fetch_effective_tail`, `find_effective_tail` |
| `arith` | `return_sign` |
| `etex` | `cancel_glue` and its continuations, `num_error`, `glue_error`, `expr_add_sub`, `expr_a`, `expr_m`, `expr_d`, `expr_s`, `get_sa_ptr`, `put_sa_ptr`, `add_sa_ptr`, `delete_sa_ptr`, `if_cur_ptr_is_null_then_return_or_goto` |
| `image` | `too_small`, `undump_*`, `format_debug`, `dump_four_ASCII`, `undump_four_ASCII`, or nothing, since the format is a snapshot |

A macro with `#` assigned to, like `check_shrinkage(p)`, is a method returning the new value, or takes `&mut i32`.

## Appendix B: changed signatures

Every stub's signature follows the WEB procedure, with the types of section 4.  These differ, and only these:

**Result:** `the_toks`, `str_toks` return nothing (they append to the builder).  `scan_general_text` leaves no list in `cur_val`.  `scan_toks` returns `def_ref`.  `jump_out`, `overflow`, `confusion`, `fatal_error`, `pdf_error` return `!`.  §141's `flush_list` is `mem_flush_list`; token lists' `flush_list` and `delete_token_ref` are implemented in `toks.rs`.

**`&[u8]` parameters** (literals at every call site): `check_pdfoutput(s,..)`, `confusion(s)`, `fatal_error(s)`, `overflow(s,..)`, `pdf_error(t,p)`, `pdf_warning(t,p,..)`, `pdf_indirect(s,..)`, `pdf_indirect_ln(s,..)`, `pdf_int_entry(s,..)`, `pdf_int_entry_ln(s,..)`, `pdf_str_entry(s,..)`, `pdf_str_entry_ln(s,..)`, `primitive(s,..)`, `print_glue(..,s)`, `print_spec(..,s)`, `print_write_whatsit(s,..)`, `prompt_file_name(s,e)` (`s` only), `restore_trace(..,s)`, `scan_keyword(s)`, `show_sa(..,s)`, `str_in_str(..,r,..)`, `substr_of_str(s,..)`, `vf_error(..,msg)`, `vf_local_font_warning(..,s)`, `warn_dest_dup(..,s1,s2)`.  A byte-slice parameter that was tested against 0 is tested with `is_empty()`.  §807's `pdf_str_entry_ln(s,"TeX")`, the one literal in the second position: `let v = self.lit_string(b"TeX")` is a leak per call; instead print its three pieces directly, as the procedure does.

**`impl Into<i64>` parameters:** `print_int(n)`, `print_scaled(s)`, `fract(x,n,d,..)`, `pdf_print_int(n)`, `pdf_print_fw_int(n,..)`, `pdf_out_bytes(n,..)`.  The body starts with `let n: i64 = n.into();`.

**`&mut` parameters:** `reverse(this_box, t, cur_g: &mut i32, cur_glue: &mut f64)`.

**`FileId` parameters:** `a_make_name_string(f)`, `b_make_name_string(f)`, `w_make_name_string(f)`.

## Pilot notes

Parts 20-25 were ported first, to test this contract on the hardest code.  What that changed, so later agents and the reviewer can check it.

**Changes to the contract.**

| What | Why | Where |
|---|---|---|
| `\csname`, `\ifcsname` and `scan_keyword` use a new stack `tx`, with `tx_mark`, `tx_push`, `tx_len`, `tx_commit`, `tx_discard` | On `tb`, their tokens in flight appeared in `runaway`'s display.  Checked against the oracle's log for `\edef\a{x\csname y\O` | toks.rs, `TX_CAP`, `tx_ptr`, `tx` in state.rs through `tools/webgen.py`; samples in section 15 |
| §534 and §1772 walk or read macro lists and were missing from section 15 and from the list of sites, as was §1473 | Ported from the old list they would read garbage from `mem`, with no error | Section 15, "Reading a macro's list" |
| §1080's `flush_list(init_list)` is on `mem` | Section 16 said every other `flush_list` was on a token list; the token `flush_list` on that pointer would corrupt `toks` | Section 16 |
| `print_nl_s` tests `selector<no_print` | §62 has it, from showstream.ch; the hand-written accessor did not, so `\showstream` output would differ | `tools/templates/access_hand.rs` |
| `show_token_list` tests a wild `p` before reading it | The sample read `toks[p]` first, unchecked, which made §315's robustness useless | Section 15, `toks_show_more` in toks.rs |
| §960 needs its own mark | The table gave it only for §959 | Section 15, "Templates" |

`DESIGN.md` section 5 still says `tb` replaces all these patterns; `tx` is the refinement.  It is a statement of intent, and I did not edit it.

`webgen.py` was run again, without `--stubs`, to make these changes; before the change it reproduced `state.rs`, `consts.rs`, `access.rs` and `generated.json` byte for byte, so the generator remains the source of those files.

**Deviations from WEB in Parts 20-25**, none observable:

- `get_next` splits off `getnext_file` (§365 and everything under it), `getnext_outer_or_dont_expand` (§379's outer branch, §380) and `getnext_insert_v_template` (§965), not inlined.  What remains is the token-list branch.  §382's `return` is the file helper's `false`, the same as falling into §364, whose test fails on `cur_cmd=0`.
- `get_next` uses `t >> 8` and `t & 255` for `t div @'400` and `t mod @'400`: a token is never negative there, and signed division by 256 would add a correction to the hottest path.  Everywhere else `/` and `%`, as section 11 says.
- `macro_call` has no `p`, `q`, `rbrace_ptr`: the argument in flight is the top of `tb`.  §426's test is on `tb[tb_ptr-1]`.  The §422 flush loop stops at `n-1`, since the list in flight has no block.
- `expand` has no `backup_backup`.

**Tests.**  `crate::toks::testkit` makes a minimal machine, usable from any module's tests: `machine()`, `def_macro`, `feed`, `set_line`, `drain`, `show`.  `tests/oracle/gen_token_machine_tables.py` regenerates `crates/engine/testdata/` from the oracle and the WEB.

**For the owners of what these tests need:** the expectations come from the oracle's log, for the same input.  If a test fails after your port, first suspect the port.

## Critic findings resolved

An independent critic attacked this contract and the foundation before fan-out.  What changed, and what to watch for.  Each was checked against the merged WEB, and the oracle's behaviour where one is quoted.

| # | Finding | Result | Where |
|---|---|---|---|
| 1 | §1309 `do_endv` tests `loc_field=null` on `input_stack` | Real.  `rec_at_list_end`, a row and a sample | toks.rs, section 15 |
| 2 | A format load restoring all of `POD_SECTIONS` loses the command line | Real.  Fixed differently from the critic's proposal, see below | notes on `image.rs`, `in_format` in `POD_SECTIONS`, DESIGN section 7 |
| 3 | §1828 `link(sa_bot_mark(q))=null` has no row | Real.  A grep of `link(` and `info(` over every list-holding field found no other | section 15 |
| 4 | `ext_xn_over_d` is in doubles, with its own rounding | Real.  Written and tested | pack.rs, section 11 |
| 5 | The oracle's `integer` is 64 bits, so it does not wrap | Real, and wider than reported: §1417, §107, §689's `q`, §1035 and §1163-1187 too.  §457 in the report is §483 | section 11, `sat32` in raw.rs, `WIDE_GLOBALS` are `i64` |
| 6 | §279's string move is missing from the `hash.rs` note | Real | notes on `hash.rs` |
| 7 | `null=0` collides with `zoom 0` | Real.  `PDF_ZOOM_NULL` | section 6, consts.rs |
| 8 | `NULL_LIST=2` breaks §532 and §14 | Real: `NULL_LIST=256`, `OMIT_TEMPLATE=258` | tools/webgen.py, toks.rs, foundation test |
| 9 | `deny(unreachable_patterns)` misses a misspelt constant in a tuple arm | Real.  Two gates | section 10, tools/coverage.py, the pilot's headers |
| 10 | The value 0 can occur as a token | Real.  Closed rather than listed: `equiv(0)=1` | section 15, "Known divergences", `Tex::alloc`, DESIGN section 5 |
| 11 | `real` is `f64` only on the WEB side | Real | notes on `crates/fontfile` |
| 12 | §222 should be §240, `cur_val` after `scan_general_text`, `\pdfmatch`'s statics | Real.  The `cur_val` case is reproduced rather than listed: `cur_val:=TEMP_HEAD` | notes per module, section 15 |

**Later agents, watch for these.**

- **A field compiles whatever it holds.**  `link(f)` on a token list, `loc_field == NULL`, `f == NULL` on a number, `x + y` past 2^31: all of findings 1, 3, 5 and 7 compile and fail at run time, with no error at the site.  If a WEB line tests `null` or takes `link` of something, ask what that field holds here.
- **The oracle's numbers are not WEB's.**  web2c's `integer` is 64 bits, its `null` is negative, its `float` is 32 bits where C says so, and `/` is real division.  When the WEB is silent, the oracle decides, and it is one command away.
- **Do not fix an oddity.**  `ext_xn_over_d`'s `r>DBL_EPSILON`, `\pdfcolorstackinit`'s literal1 stop, the `\BAD.` token and §1187's order are all observable and all kept.

**Why finding 2 differs from the critic's proposal.**  They proposed resetting at store every global that §1485-1507 do not dump, then reordering startup: parse the first line into Host, load the image, apply the CLI.  That needs a list of all non-dumped globals, where an omission fails silently, and it splits §37 and §1517, whose order is observable (`&name` on a line typed at the `**` prompt).  Loading only what WEB dumps needs neither, and it is the literal behaviour: the list to keep right is the one §1485-1507 print.  It costs `initialize` on every run, the scalars and small tables of `@<Set initial values@>`, which is negligible beside the 5 ms target and can be baked in M5 if measurement says otherwise.

