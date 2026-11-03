# rstex: design

`rstex` is a TeX-to-PDF compiler in Rust. `commute` is a tikz-cd-compatible commutative diagram
drawer that shares its engine. **The main goal is performance**, without giving up the ability to run
real LaTeX documents unchanged.

**Both programs are written entirely in Rust, with no C or C++ dependencies** (no `cc` or `cmake` build
steps, no bindings to C libraries beyond the system libc that every Rust program on Linux uses). At run time
rstex needs TeX Live's data files (packages, fonts, metrics) but none of its binaries. Python appears only in
development tools under `tools/` and `tests/`, and `pdflatex` only as the test oracle.

This document holds the decisions. The evidence is in `docs/research/`:

| File | Content |
|---|---|
| `time-budget.md` | Where pdflatex actually spends its time |
| `attack-strawman.md` | Adversarial attack on the snapshot and native-diagram ideas |
| `architecture-proposal.md` | Independent architecture proposal, with data-layout experiments |
| `verification-and-inventory.md` | TRIP feasibility, primitive inventory, differential harness |
| `tikzcd-summary.md` and `TIKZCD-SPEC.md` | Exact tikz-cd geometry |

## 1. What the measurements say

Thesis, 33 pp, 1380 ms on pdflatex:

| Slice | ms | Lever |
|---|---|---|
| Preamble | 590-650 | Preamble snapshot |
| 20 tikz-cd diagrams in the body | ~520 | Native diagram fast path (`commute`) |
| Other macro work in the body | ~165 | Faster token machine, microtype font-setup cost |
| fmt undump (28.5 ms of it is gzip) | 37 | Flat format image |
| kpathsea startup | 7.5 | File index |
| C typesetting (line_break, hpack, mlist, page builder) | ~12 | **None. Translate literally.** |
| All PDF output (shipout, fonts, deflate, write) | ~55 | Small; parallel deflate at most ~1% |

- The token machine is ~84% of runtime: `get_next` 39%, `macro_call` 11.5%, `end_token_list` 7.6%,
  `delete_token_ref` 3.9%, `id_lookup` 3.8%. get_next runs ~119 M calls on the thesis.
- About 67% of token fetches are copy or skip loops (macro argument scan, `pass_text`,
  `scan_general_text`) that do not need full get_next semantics.
- The oracle's memory word is 16 bytes. An 8-byte word halves `mem`.
- `hash_prime`=8501 with 57k names gives a mean 4.7 probes per hit, ~101 ns per lookup.

Very long documents have a different balance. On SGA3-English (1065 pp, 9.3 s) the preamble is under 5% and the PDF stage about 10%, so
the preamble snapshot barely helps there. Only the token machine and PDF output do, and a parallel PDF stage is worth more than on the thesis.
Do not tune for the thesis alone.

**False economies, do not build:** catcode shadow tables or a SIMD tokenizer (file reading is ~1%),
tuned hpack or line_break, node SoA, threaded dispatch for main_control, single-token pushback register,
`target-cpu=native`, parallel typesetting (impossible anyway: `\write` expands at shipout, `\pdfsavepos`
feeds back into TeX).

## 2. Targets

| Scenario | pdflatex | rstex target |
|---|---|---|
| hello.tex | 66 ms | under 5 ms |
| Thesis, cold (no cache, full pass) | 1380 ms | under 600 ms |
| Thesis, warm (preamble snapshot + native diagrams) | n/a | under 130 ms |
| Format build (raw engine throughput) | 5.0 s | under 2.5 s |
| SGA3-English, 1065 pp, full pass (preamble is only 0.4 s, so this measures the raw engine) | 9.3 s | under 4 s |

The micro-benchmark baseline is in `bench/micro/baseline-pdftex-1.40.29.json`.

Fidelity targets are in section 10. Speed never excuses wrong output. A snapshot that is not provably
valid is discarded, so an unknown situation gets slower, never wrong.

## 3. Overall shape: literal core, new layout, rewritten outer layer

**Literal: semantic core.** Scanner, expansion, eqtb, save stack, nodes, line breaking, page builder,
math, alignment, e-TeX, the pdfTeX extensions. Port section by section from the merged WEB, keeping WEB
names. The behaviour is the specification, and TRIP and e-TRIP are gates only a faithful port can use.

**Not literal: data layout.** See sections 4 and 5.

**Rewritten: everything around the core.** File access, kpathsea, printing plumbing, the C/C++ backend
(`writet1.c`, `mapfile.c`, `writefont.c`, `tounicode.c`, image handling), the format file.

**Out of scope for now, by the user's decision:** LuaTeX (Lua, OpenType, `fontspec`, `unicode-math`). Nothing here should make adding it impossible later, but no effort goes to it.

Dropped: encTeX, MLTeX, IPC, src-specials, TCX reader (hardcode the `cp227` printable table; TRIP uses the
default table), mktex* generation, PK and Type 3 fonts. Each reports a clear error if a document needs it.
TrueType/OTF, subfonts, JBIG2, PNG, PDF inclusion and VF were first dropped or deferred, then ported after all:
`crates/fontfile` and `crates/image` hold them, compared with the oracle.

## 4. State: POD image plus Host

Everything that survives a snapshot lives in the image. This is the rule that makes snapshots sound.

```rust
#[repr(C)]
pub struct Tex {
    // scalars first (hot ones in the first cache line), then inline arrays with fixed capacities
    ...
    pub mem: [Word; MEM_CAP], pub toks: [u32; TOKS_CAP], pub eqtb: [EqWord; EQTB_CAP], ...
    pub host: Host,   // last; NOT POD; never snapshotted
}
```

- `Tex` is placed in one 2 MiB-aligned lazily committed reservation (per OS, see section 12)
  and never moved. Inline arrays compile to `disp32(%rdi,%rcx,8)`, one load fewer than `Box<[T]>`.
  Keep total POD size under 2 GiB so displacements fit. Lazily committed, so capacities cost nothing.
- All code is `impl Tex { fn x(&mut self) }`. No `static mut`, `RefCell`, `Rc`, lifetimes.
- Every POD field is integers, floats, or arrays of them, enforced by a compile-time `Pod` bound.
  No Rust enums or `bool` in POD state (`u8`/`i32` and consts).
- **No pointers in POD.** All references are `u32` indices. Former C statics and the 36 `^` arrays
  (map tree, ToUnicode table, colorstacks, image table) become indexed arenas in the image.
  `PodVec`/`PodMap` in an image arena, never `HashMap`.
- `Host` holds non-POD state: open files, the World, thread pool, `help_line` static strings.

**Word** is 8 bytes: `{ rh: u32, lh: u32 }` or `{ rh: u32, b0: u16, b1: u16 }`, `i32` (int, sc), `f64`
(glue ratio; must be f64 to match the oracle's rounding), `4×u16` (qqqq). Quarterword is u16: 225 and 277
fonts are seen in real logs. The oracle's `integer` is 64 bits, ours is `i32`: values at or above 2^31 clamp where they can arise, and
print differently (`docs/PORTING.md`, section 11).

**Pointers:** `type Ptr = u32`, `NULL = 0`, `mem_bot = 1`, `min_halfword = 0`. `mem` lo and hi regions are
split at a compile-time boundary so `is_char_node(p)` is a constant compare. Halfword fields that hold
integers cast at the site.

**Eqtb:** the `eqtb` entry is 8 bytes. The classic fixed regions stay at the bottom; `frozen_*` keep their
meaning. Control sequences created at run time are allocated sequentially above `eqtb_size`, each entry
initialised on allocation, which removes the 20 MB init web2c does on every run.

**Hash:** open addressing with `(hash32 << 32) | p` entries, word-at-a-time hash, grows by rehash, never
deletes. `text[p]` is a parallel array. Not observable except in statistics lines, which TRIP masks.

**Strings:** `str_pool: [u8]`, `str_start: [u32]`. WEB string literals become byte literals
(`print_s(b"...")`, `print_esc_s(b"...")`, `help` lines are `&'static [u8]` in Host); there is no pool file.
`primitive()` makes strings at INITEX time; single-character strings 0-255 are initialised as in TeX.

**Bounds checks:** unchecked in release, checked under `--features checked`, both through `raw.rs`
accessors. All other engine modules are `#![forbid(unsafe_code)]`. CI runs the corpus with `checked`.

## 5. Token lists leave `mem`

Token lists are the single largest data structure and the hottest. They live in a contiguous `u32` heap
`toks`. In a replay of real traces, unchecked: 16-byte linked 2.46, 8-byte linked 2.05, 4-byte contiguous
1.36 ns/token, and 16.7 MB becomes ~4.2 MB. This also removes most of the 14.4% in `end_token_list`,
`delete_token_ref` and `get_avail`, and enables capture loops (section 8). Designed in from day one,
because retrofitting touches the same sites later.

- **Token** is `u32` as in pdfTeX: `cmd*256+chr`, or `CS_TOKEN_FLAG + cs`. Value 0 never occurs in a list
  (0 occurrences in 1,044,066 real tokens; `cmd` 0 char tokens cannot occur), so **0 terminates a list**. WEB can store a 0 on
  one error path, `\pdfprimitive\fi` before a condition has been evaluated; `equiv(0)=1` closes that (`docs/PORTING.md`, section 15).
- **Block:** `[hdr][t0]...[tn][0]`. `hdr = (rc << 8) | size_class`. `rc` follows TeX's `token_ref_count`
  convention (refs minus one, so `null` means one reference). Every list has a header, including argument
  lists and backed-up tokens, so `flush_list` is O(1) by size class, with no walk.
- **`TokPtr = u32`** is the header index. `0` is the null (empty) list. `toks[0] = toks[1] = 0`, so both
  `loc = 0` and `loc = 0 + 1` read as end of list.
- **Macro body** keeps TeX's order: `[hdr][params... end_match][body...][0]`, with `protected_token` first
  if protected.
- **Allocator:** exact size classes up to 64 tokens, geometric above; per-class free lists; bump otherwise.
  No coalescing.
- **Reading:** `link(p)` walks become `i += 1`. `loc = null` tests in token-list state become
  `toks[loc] == 0` (`self.at_list_end()`).
- **Fixed lists** like `omit_template` and `null_list` sit at reserved low indices of `toks`, above 255, since §532 tells
  `frozen_endv` from a character by `cur_chr>255`.
- **Building:** a LIFO builder stack `tb` replaces all `temp_head` / `def_ref` / `backup_head` /
  `store_new_token` patterns.  `\csname`, `\ifcsname` and `scan_keyword` use a second stack `tx`, because their tokens in
  flight must not show in `runaway`'s display.

  ```rust
  let m = self.tb_mark();     // start of a new list
  self.tb_push(t);            // store_new_token / fast_store_new_token
  let p = self.tb_commit(m);  // allocate block, copy, pop. rc = 0.
  self.tb_discard(m);         // abandon
  ```

  Nesting is LIFO because nested expansion finishes before the outer builder resumes. Functions that
  returned a tail pointer (`the_toks`, `str_toks`, `read_toks`) append to the current builder, so
  `scan_toks` needs no copy on `\the`. Macro arguments must be committed to the heap, since the caller's
  arguments are freed under the callee's: frees are non-LIFO. `runaway()` reads the in-flight list via a
  saved mark, the analogue of `def_ref`.
- `\noexpand` prepending a `frozen_dont_expand` token to a backed-up list becomes one fresh two-token block.
  Brace stripping in `macro_call` becomes truncate and advance.

`mem` keeps `get_avail` one-word cells for char nodes and the few non-token uses (hyphenation lists and the
like).

## 6. Host, World, journal

All side effects pass through one World interface in Host. **No direct syscalls elsewhere.** Retrofitting
this would be the hardest item, so it exists from the first line.

- **World operations:** lookup(name, kind), read, stat, clock, env, shell, terminal. Each is recorded as
  an *observation* in a log kept for checkpoints, and may mark the run *tainted*.
- **Files are memory.** Input is read whole (or mmap); `input_ln` is memchr plus copy. Every level of
  `input_stack`, `read_file[]` and `write_file[]` stores path and logical position, with whole-line semantics.
- **Output journal:** log, `.fls`, `\openout` streams, SyncTeX. Bytes can be re-materialised on restore.
- **PDF output is a chunk list with deferred offsets and no seek-back.** `/Length` is known at write time.
  Keep pdfTeX's object numbering so oracle diffs work.
- **Determinism:** `SOURCE_DATE_EPOCH` and `FORCE_SOURCE_DATE` honoured. Every run is reproducible from its
  observations.

## 7. Snapshots

**Format = flat image of what `\dump` writes.** One loader for formats and preamble checkpoints. Flat, ungzipped. A format holds only what
§1485-1507 dump, since its load runs in the middle of §1517 with the command line already in `buffer`; only a checkpoint restores the
whole image (`docs/PORTING.md`, notes on `image.rs`).
A format load puts each section directly in its place. A section of 64 KiB or more is mapped privately from the file, page-aligned there for that
purpose, so a document pays only for the pages it touches: `font_info` (5 MB in the LaTeX format, nearly all of it expl3's `\fontdimen` tables), the trie, `str_pool`, `str_start`, `hash`, `arena`. Those
that a run writes all over, `toks`, `eqtb`, `hash_tab`, are mapped too and their pages copied at once with `MADV_POPULATE_WRITE`: that beats a fault per page, and a read, which zeroes each page before filling it. Windows reads.
Huge pages only where a first touch is not a waste of zeroing 2 MiB: past the first chunks of `toks` and `mem`; see `Tex::alloc`.
Publish by tmp + rename, also so that a running job keeps the file it mapped. Key on build-id, from `crates/engine/build.rs`. Sections store only the used part of each array.
The hash table is written with room to spare, or every run would move all of it at the first few thousand new names.

After load, §-level run-time init still runs: CLI overrides, `fix_date_and_time`, seed.

**Preamble checkpoint** is taken just before `\document` runs (first `macro_call` of that control sequence;
one predictable compare). `.aux` is read at `latex.ltx:9489` and `\@mainaux` opened at 9492, both after this
point. Worth ~600 ms on the thesis.

**The validity rule:** a checkpoint records an ordered log of every observation of the outside world and is
valid only if each observation would get the same answer now.

- **Config:** build-id, format hash, relevant CLI flags, cwd, output dir, expanded path specs, relevant env
  (including `SOURCE_DATE_EPOCH`, `FORCE_SOURCE_DATE`, `TEXINPUTS`).
- **`lookup(name, kind, path-spec)`:** the resolved path or NOT FOUND, including shadow probes (25 failed
  lookups in the thesis preamble: `hyperref.cfg`, `typearea.cfg`, ...).
- **Reads:** whole-file hash for closed files; prefix hash through the read position for open files. Project
  files always content-hashed; TDS files by size, mtime, inode plus the `ls-R` stamp.
- **Stat-like primitives:** `\pdffilesize`, `\pdffilemoddate`, `\pdfmdfivesum`, `\pdffiledump`, image headers.
- **Volatile reads**, hooked in `scan_something_internal` and the convert primitives. Date read: key on
  YYYYMMDD. `\time`, seed, deviates, `\pdfcreationdate`, `\pdfelapsedtime`: hard taint. Exactly two sites
  whitelisted: expl3 `\everyjob` (`\c_sys_*`, patched on restore) and the pgfmath seed.
- **Hard taint, no checkpoint past this point:** `\write18`, pipe input, terminal `\read`, error state.
- **The run's own writes** (`\openout` in preamble, log prefix, `.fls`) are journalled with bytes and
  re-materialised on restore.
- **Stacked files:** `\begin{document}` may be in an `\input` file. Every stacked file stores path and offset;
  the offset is at line end and the hash covers the whole line, because the rest of the line is in `buffer`.
- **Do not validate by byte-comparing state**: one changed digit shifts 281k words. Use a canonical decode for
  testing.

On restore: refresh `\year`..`\time` unless the preamble assigned them, patch `\c_sys_*`, recompute start time
and `/ID`.

**Generalisation:** a safe point is any top-level main-file line with `in_open=1`, no group, `nest_ptr=0`, so
preamble checkpoints can be taken every few top-level lines. Editing a `\newcommand` then costs ~34 ms, not 650.

**Page checkpoints (editor mode only, last):** shipouts are safe points, but "last page before the edit" is
wrong, since TeX reads whole paragraphs first. Take the latest checkpoint whose read position is at or before the
edit, per file. Plain 2.4 ms memcpy at a shipout at least 100 ms after the previous, dedupe 4K pages in a worker.
No kernel tricks, no fork.

## 8. Hot paths

Implement after TRIP passes on the generic path; keep a `slow_capture` feature for differential testing.

- **get_next** splits into an `#[inline(always)]` leaf (token-list branch only) and an `#[inline(never)]`
  slow path (end of list, file state, `\noexpand`, outer checks). Inline the leaf into `get_x_token`, `get_token`,
  `pass_text`, `x_token` only. `cur_cmd`/`cur_chr` stay fields.
- **Capture loops** in the group-contribution loop, `scan_general_text`, `pass_text`, macro argument scan: when the
  input is a token list, copy tokens with brace counting and bail to slow on end of list, `&`, out_param,
  `\par` when non-long, the `\noexpand` token, `\endwrite`, or any user `\outer` (a sticky `any_user_outer` flag).
  Replaces ~54 M get_next + store paths per thesis.
- **Stats** (`dyn_used`, etc.) behind a feature.
- **Build:** `opt-level=3`, `lto="fat"`, `codegen-units=1`, `panic="abort"`, `+crt-static` (0.38 vs 0.75 ms exec;
  also skips the system `ld.so.preload`), PGO trained on thesis/LMB/tikzcd/hello.
- **Engine stack:** 1 GiB, since TeX recurses deeply: only reserved.  With glibc the process's one thread moves to it by `swapcontext`; elsewhere it is a dedicated thread.
- **Start-up:** `main` is the C runtime's, without Rust's `lang_start`, which reads `/proc/self/maps` and installs handlers that only change how a crash looks.
- **Fatal errors:** `overflow`, `confusion`, `fatal_error`, `succumb` return `!` and reach `jump_out()`, which closes
  files and calls `process::exit`. No `Result` in the engine. `error()` remains an ordinary call and return.
- **Gotos:** forward `done`/`exit` → `'done: { ... break 'done }`; backward `restart`/`continue` → `'restart: loop`;
  irreducible cases (main_control's lig/kern loop, hyphenate/reconstitute) → `let mut pc = Pc::X; loop { match pc {...} }`.

## 9. PDF backend and fonts

- **Deflate:** `zlib-rs`, pure Rust. In a 158-stream test its level 6 took 18.9 ms and 929,066 bytes, against
  14.2 ms and 924,349 bytes for libdeflate (C) and ~45 ms for system zlib 9. Staying pure Rust costs about 5 ms on the
  thesis. Content and ObjStm at level 6, FontFile at level 1 (eexec is incompressible). A stream is always deflated
  whole, in one call: zlib-rs's bytes depend on where the pieces begin (38 of 240 cases at levels 0, 1, 5 and 6), so
  piecewise writing and workers would give different bytes for the same document.
- **Parallelism is plumbing, at most ~1%.** Workers start lazily at the first shipout; the main thread assigns object
  numbers and slots; an ordered writer keeps output deterministic. Build after everything else and re-measure.
- **Fonts:** port `writet1.c` semantics and the MD5 subset tag. The first run that parses `pdftex.map` keeps its tables in the cache directory with the file's stamp, instead of
  the 4.5 ms per-run parse; a run maps them, and an entry moves to the image's own tables when something first asks for one of its names (`pdffont.rs`, at the top). Leaf crates are
  stateless functions; they never call back into the engine.
- **Font expansion is mandatory.** microtype turns it on in all the user's documents; 1293 of 2441 `Tm` operators in the
  thesis have horizontal scale other than 1. Protrusion too.
- **Required file readers:** TFM, PFB, ENC, MAP, JPEG. Then PNG, PDF inclusion, VF.

## 10. commute

The library crate `commute` computes geometry from the spec in `docs/TIKZCD-SPEC.md` (checked against pdflatex to 0.031 bp by
`tests/cd/model/model.py`). It has no engine dependency.

**Inside the compiler (fast path, not replacement):**

- `\usepackage{tikz-cd}` keeps loading the real package, because it also provides `\color`, `\definecolor`,
  `\includegraphics`, `\setkeys`, `\tikzset`, `\pgfmathparse`, `\foreach`, and reports xcolor, graphicx, keyval, tikz as
  loaded. Under the preamble snapshot the 113 ms load vanishes anyway.
- At `\begin{tikzcd}` a native primitive scans options and body unexpanded, up to `\end{tikzcd}`. If every option is in the
  recognised subset and the tikz-cd styles are unmodified, take the native path. Otherwise push the tokens back and run real PGF
  for this one diagram.
- Native path: Rust parses the token list into cells and arrows, injects tokens that typeset cells and labels into real TeX
  boxes, then a final primitive reads box dimensions, calls the `commute` library, and emits an hbox with `\pdfliteral`-equivalent
  whatsits and positioned box copies. Worth ~480 ms on the thesis.
- Gated by rendered comparison and box dimensions against real PGF. Bezier details may differ by under 0.05 bp; if a diagram cannot
  be matched within tolerance, it falls back.

**Standalone `commute` CLI:** reads a diagram (a `tikzcd` body, or a file of them), typesets labels with the embedded engine and a
prebuilt format, writes a tightly cropped PDF, and SVG with glyph outlines from the PFB. Target: under 10 ms per diagram.

## 11. Verification

Oracle: system `pdftex`/`pdflatex` (1.40.29, TeX Live 2026). Levels, from `docs/research/verification-and-inventory.md`:

| Level | Check | When |
|---|---|---|
| Rung -1 | TRIP and e-TRIP (`tests/trip/run_trip.sh`), masked diffs equal to system pdftex | Gate before anything else |
| L0 | `.aux .toc .out .bbl` bytes | Every rung |
| L2 | Box logs: `\tracingoutput=1 \showboxbreadth=\showboxdepth=2147483647` | Every rung, first-divergence tool |
| L1 | Macro trace, streamed, stop at first difference | On demand |
| L4 | Normalised content streams, `tests/harness/compare_pdf.py` | Every rung |
| L5 | PDF structure: info, outlines, dests, fonts, ToUnicode, FontFile hash | Every rung |
| L6 | Uncompressed bytes under `\pdfcompresslevel=0 \pdfobjcompresslevel=0 \pdftrailerid{}` and `SOURCE_DATE_EPOCH=1700000000 FORCE_SOURCE_DATE=1` | Goal for ladder documents |
| L7 | `pdftoppm -r 100` raster diff | Safety net |

DVI is for TRIP and early rungs only; the thesis in DVI mode changes drivers and loses font expansion.

**Ladder:** `tests/ladder/` rungs 00 (plain hello) through 17 (classes), then the user's private documents, the thesis among them
(read from their own directories, not copied into this repo, and listed in the file that `RSTEX_LADDER_PRIVATE` names; the verbatim
diagram excerpts `tests/cd/21-*.tex` and `tests/cd/22-*.tex` are gitignored). Driver: `tests/harness/run_ladder.sh`.

**Corpus.** A small hand-picked set cannot establish trust, so the ladder is only the bring-up path. The real gate is the corpus in
`tests/corpus/`: large, not chosen to suit the engine, and always reported as a pass count over a stated denominator. It covers the
user's own documents, large public books, official LaTeX and expl3 regression suites, a random sample of arXiv sources, and generated
coverage tests (every package load, every mapped font, every primitive). No result is dropped silently: an entry the oracle cannot
compile is recorded as such, and every failure stays listed with its first divergence. Performance claims are also reported over the
corpus (median and p90 speed-up), not only on chosen documents. See `tests/corpus/README.md`.

**Snapshot gate:** cold and snapshot-restored PDFs must be byte-identical for every rung (L6 mode). Determinism oracle: pinned clock,
state hash per shipout, fresh versus restored comparison.

## 12. Platforms

**Linux, macOS (arm64 and x86-64) and Windows (x86-64) are all first-class.** Only 64-bit little-endian targets are supported
(`compile_error!` otherwise). The pure-Rust rule helps here: no C toolchain is needed on any OS.

**All OS-specific code lives in `crates/engine/src/platform/`** (`mod.rs`, `unix.rs`, `windows.rs`) and in `crates/kpse`. `cfg(unix)`,
`cfg(windows)`, `cfg(target_os)`, `libc::` and `windows_sys::` appear nowhere else. Linux-only tuning is a `cfg` inside `platform`, never a
semantic difference: every platform must produce the same PDF bytes for the same inputs.

| Concern | Linux | macOS | Windows |
|---|---|---|---|
| Reserve lazily zeroed POD region | `mmap` `MAP_NORESERVE`, `MADV_NOHUGEPAGE` with chosen parts `MADV_HUGEPAGE` | `mmap` `MAP_ANON` (no THP; arm64 pages are 16 KiB) | `VirtualAlloc(MEM_RESERVE)`; a vectored exception handler commits on first touch in 2 MiB chunks |
| Filling a region from a snapshot | `read`, or a private mapping of the file | `read`, or a private mapping of the file | `platform::commit_range` first, since `ReadFile` into uncommitted pages fails rather than faulting |
| Static C runtime | `+crt-static` | default (cannot be static) | `+crt-static` |
| Local time for `\time`, `/CreationDate` | `localtime_r` | `localtime_r` | `GetLocalTime` plus zone bias |
| `\write18` | `sh -c` | `sh -c` | `cmd /c` |
| Cache directory | `$XDG_CACHE_HOME/rstex` or `~/.cache/rstex` | `~/Library/Caches/rstex` | `%LOCALAPPDATA%\\rstex\\cache` |
| Publish a cache file | tmp + rename | tmp + rename | tmp + rename with retry, since a reader may hold it open |
| File identity for validation | size, mtime, inode | size, mtime, inode | size, mtime |

Everything else is built on `std`:

- **File names.** TeX sees bytes. The World converts at the boundary: bytes on Unix, UTF-8 to wide strings on Windows, and it accepts both `/` and `\\`
  plus drive letters on Windows. Names printed in the log are what TeX was given or found, so logs may differ in paths; PDFs may not.
- **Text.** Input is binary-read; as in web2c, `input_ln` strips trailing spaces, `\r` and `\n`, so CRLF sources work everywhere. Output is `\n` on every
  platform, for byte-identical results.
- **Input files are read, not mapped.** Windows locks mapped files, which would break editors saving during a compile.
- **Finding the TeX tree.** No hardcoded `/usr/share`. Order: `RSTEX_TEXMF` env; else ask `kpsewhich` once (TeX Live, MacTeX) or `initexmf --report` (MiKTeX),
  and cache the answer; else well-known locations. These programs are used only to locate data, never to compile. Our own `texmf.cnf` reader follows kpathsea's
  variable and search semantics (`SELFAUTOLOC` and friends, `;` as separator on Windows). MiKTeX has no `ls-R`, so the index comes from walking the trees. Lookups in the
  tree index fall back to case-insensitive matching on macOS and Windows. MiKTeX's install-on-demand is not triggered; a missing file is a clean error naming the file.
- **Deep recursion.** The engine's 1 GiB stack is requested through `std::thread::Builder`, or mapped for `swapcontext` under glibc; only reserved memory, on all three.
- **The tables of `texmf.cnf`, `ls-R` and `pdftex.map`** stay in the cache directory, `crates/kpse/src/index.rs` and `cachefile.rs`: valid while each environment variable and each path looked at, there or not, has the answer it had, and checked by sums as far as they are read. Mapped on Unix, read on Windows, where a mapping would hold the file against the rename that replaces it.
- **Formats** are built when needed, once, in the cache directory, `crates/rstex/src/autofmt.rs`, keyed on the build-id and the tree, and valid while everything the build read has the stamp it had. TeX Live's gzip formats are no use to this engine.
- **The performance targets of section 2 are measured on Linux.** Inline-array displacement addressing is x86-64 specific; on arm64 LLVM materialises the offset, which costs a little.
  PGO is per target, so it is done in release builds per OS.

**Gates.** From M0 on, `cargo check --workspace --target` must pass for `x86_64-unknown-linux-gnu`, `aarch64-apple-darwin` and `x86_64-pc-windows-msvc` (all installed
locally; no linker or C toolchain required). That only proves it compiles. Real runs need real machines, so GitHub Actions runs a matrix of `ubuntu`, `macos`, `windows` with TeX Live: unit tests, the
format build, the smoke tier of the corpus, and a **cross-OS check that the PDFs have the same hash**. On a private repository on GitHub Free the included usage is $12 a month, at $0.006/min for Linux, $0.010/min for Windows and $0.062/min for macOS (about 2000, 1200 or 190 minutes); standard runners are free on public repositories. So Linux runs on every push;
macOS and Windows run on `main`, on tags and on manual dispatch.

## 13. Privacy

**The rule.** The user's thesis must never reach GitHub through this project or its testing: not in commits, branches, CI, fixtures,
logs, reports or commit messages. Testing against it locally is wanted, read-only. The other private documents kept outside the repo
are treated the same way.

- **Results go under `target/`**, which is gitignored: ladder output, corpus runs, logs and reports. That includes anything copied out of
  them. A first-divergence record, a box log, a TeX error message or a `pdftotext` excerpt of a private document quotes that document.
- **CI never gets private documents.** The `*.local.json` corpus manifests and the ladder's private list are not in the repo, so CI has
  nothing to read, and tests that need them are skipped there.
- **A minimised reproduction** derived from the thesis must be rewritten with neutral content before it may be committed: new objects,
  labels, names and wording, keeping only the mechanism that fails.
- **Agents must not quote thesis text** in commit messages or reports. Docs may say "the thesis" and give its page counts and timings,
  but no path, no file name, and nothing taken from its content, such as macro or label names.
- **Do not skip the hooks.** No `--no-verify`, no `core.hooksPath`, no pushing to a URL that avoids the gate. When the guard stops
  something that looks harmless, say so and ask.

**Enforcement.** `tools/privacy_guard.py` reads the private directories at run time, including every version of their text files in any
git history there, and stores nothing derived from them. Its header lists the rules. In short it blocks:

| Rule | What |
|---|---|
| a | private lines, a cluster of three short ones, the source line TeX quotes in an error |
| b | 8 private words in a row, or 6 when two of them are more than stock phrasing, and a 30-word window that is half private three-word runs |
| c | diagram and equation bodies, diagrams of the same structure, diagrams holding most of the cell or label strings of a private one |
| d | distinctive titles, labels, bib keys, macros and file names, and names from the title page or the store's `identity.terms` |
| e | a private file's hash, as the content of a file or written out in text |
| f | paths and refs containing the thesis words or `.local.`, and such paths in text. A source file such as `benches/thesis.rs` may have the word in its name, and its content is scanned like any other |
| g | binaries and SVG, long encoded runs, and any shorter run or number table that decodes to text, a file format or compressed data |
| h | engine output whatever it says: PDF text operators, box logs, runner records of a `user/` entry |

A blob is also read as what it may hold: string escapes, per-line prefixes such as diff marks or `cat -n`, markup, encoded runs, and
cheap scrambling such as ROT13 or reversed text. Commit messages, author and tagger fields, file names and ref names go through the same
rules.

`tools/install-hooks.sh` installs `pre-commit`, `pre-merge-commit`, `commit-msg` and `pre-push` hooks. Run it once per clone, since git
does not copy hooks, and again after changing the guard, because hooks run the copy it places in `.git/rstex-guard/`: the tree being
committed has no say in what checks it. It records the private directories as absolute paths in `.git/config`, with the amount of text
they hold. A hook stops if a directory is missing, if the sources have shrunk by more than half, if the guard crashes, or if `python3`
is gone. `RSTEX_PRIVATE_DIRS` and `HOME` do not change what a hook checks; the variable applies to manual runs and to the install.
`--no-private-sources` is for a machine that never had the documents, and is refused where they exist.

`pre-push` asks the remote which commits it holds, with `git ls-remote`, and scans what is not among them. A local tracking ref proves
nothing, and a push to an empty remote scans the whole history.

`.gitignore` covers the same names, but it only keeps them out of `git add`.

**Binaries.** A binary, SVG, or text with encoded data or engine output needs a line in `tools/privacy-allowed-binaries.txt`, and that line
counts only once HEAD holds it for a commit, or the remote for a push, so adding it in the commit that needs it does nothing. A new entry
therefore also goes into `.git/rstex-guard/approved-binaries.txt`, which is outside every tree. A person does that, after looking at the
file. Agents ask.

**The gate.** `tools/install-gate.sh` sets a remote's push URL to a local bare repository whose `pre-receive` hook runs the guard and then
forwards to the real URL. It stops `--no-verify`, `core.hooksPath`, `git send-pack` to the gate, hooks that are not executable and a guard edited in
the tree. It is optional, and it cannot be tested against GitHub without pushing there.

**What this does not cover.**

- **Anything that avoids both.** A fresh clone before `install-hooks.sh`, or `--no-verify` without the gate, runs no check at all. With the gate,
  a push that names the real URL, a new clone, `gh`, API uploads and release assets still go straight to GitHub. They all run as the same
  user with the same credentials, and nothing on GitHub checks again. Keeping the GitHub credential where only the gate's hook finds it
  would raise the bar; it would not close this for an agent with a shell.
- **Short pastes.** A source line that is short or mostly formula passes on its own, which is about half of them. Of pastes taken from the thesis, about a third of two-line ones pass, a tenth of five-line ones and almost none past twelve lines; `pdftotext` excerpts behave alike. A one-line bug reproduction copied from the thesis is the realistic case, and only the rule about rewriting
  reproductions covers it.
- **Reworded text.** Replacing every tenth word is caught, every eighth mostly, every sixth about half the time and every fourth rarely. Single words, three-word headings, short labels and the title-page lines that are stock phrases pass by design.
- **Rewritten diagrams.** One small change to a thesis diagram is caught about three times in four, and two changes about one time in two. A diagram with too few distinct cell and label strings has nothing to recognise it by. A plain commutative square with new letters passes, as the reproduction rule intends.
- **Small pieces of engine output.** A box log under 40 lines, PDF operators under 24 letters, or a record without its `user/` id hold a few glyphs each.
  The real fix is for `tests/corpus/run.py` to write no document text for `user/` entries, and it does not do that yet.
- **Deliberate obfuscation.** Encryption, private ciphers, lookalike letters, or a text cut into pieces of under six words in separate files.
- **Statistics.** Page counts, timings and diagram counts of the thesis are allowed in docs by the rule above, and the guard does not try to
  recognise them. Nor does it block the name of the private store in text, since the corpus tooling has to name it.
- **Documents that are not sources.** Only the thesis directory and the private store are read. Other documents the ladder compiles are
  protected by not being in the tree and by rule h, not by matching. To cover one, link it into the store.

## 14. Layout of the repository

```
crates/engine      rstex-engine: the port. One crate, since WEB's call graph is one strongly connected component.
crates/kpse        kpathsea subset + binary path index
crates/fontfile    Type 1 parse/subset, enc, map-line parser, ToUnicode. Stateless.
crates/pdfio       deflate, chunk writer, MD5
crates/image       JPEG, PNG, PDF-inclusion reader
crates/commute     diagram geometry (no engine dependency)
crates/rstex       `rstex` CLI: pdflatex-compatible flags, format auto-build, --until-stable and the snapshot driver to come
crates/commute-cli `commute` CLI
tools/             WEB tooling (section lookup, coverage), profilers, privacy guard
refs/              Upstream sources, gitignored
tests/             trip, ladder, cd, harness
bench/             micro-benchmarks and baseline
```

License: GPL-3.0-or-later. `pdftex.web` is GPL-2-or-later. `tex.web` permits modified versions whose file is not named `tex.web`, and
the program is not to be called plain TeX; `rstex` is a distinct name in the same way as pdfTeX or XeTeX, and the banner identifies
rstex. Compatibility values such as `\pdftexversion` keep the numbers
packages test for.

## 15. Milestones

| # | Content | Gate |
|---|---|---|
| M0 | Merged WEB, section tools, generated constants and state, stubs for all procedures, token heap, Host/World, `docs/PORTING.md`. Pilot: port Parts 20-25 | `cargo check`; porting contract survives adversarial reading |
| M1 | Port Parts 1-19, 26-30 and 33-49 in parallel | Coverage script: every WEB section accounted for |
| M2 | Port extensions, e-TeX, DVI, PDF backend, kpse, fontfile, pdfio, image, main/format. `commute` crate in parallel | Coverage; `commute` matches `model.py` |
| M3 | Integrate. INITEX runs. **TRIP and e-TRIP** | Rung -1 |
| M4 | LaTeX format builds. Ladder rung by rung to the thesis | L4/L5 identical through thesis |
| M5 | Performance: format image, preamble snapshot, file index, capture loops, get_next split, PGO | Targets of section 2; snapshot gate |
| M6 | commute fast path and CLI | Rendered comparison on `tests/cd` and the user's 48 diagrams |
| M7 | Driver polish, SyncTeX, adversarial review, docs, benchmark report, Windows and macOS green on real CI | Everything green on all three |
