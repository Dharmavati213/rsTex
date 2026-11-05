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
feeds back into TeX).  A size-segregated allocator for `get_node`: measured at 1.1% of SGA3's instructions and 0.5% of the thesis's,
since the literal search takes 1.1 steps per call, and 40 to 80% of its calls are condition nodes.  It moves what a line box's
unset glue fields read (PORTING section 15) and so breaks `pack/expansion_unset_glue`.  A free list for condition nodes alone is the small form of the same thing, 1.4% of the thesis's instructions and 1.1% of SGA3's: in `arxiv/cs.NI/1908.01405v1` it took
the destinations under a `\scalebox` that differ from the oracle's to 27 of 190, two of them by 3264 bp, since their `/XYZ` depend on what `get_node`'s words still hold (PORTING section 15).
Measured on SGA3 after M5: a prefetch in a list walk, since the next node's address is known only when its line is read anyway, and `get_node`'s and `avail`'s order cannot change.  Font subsetting on a worker: `write_t1` is 0.25 to 0.4% of the wall clock, all fonts 0.4 to 0.7%, and the subset tags tie one font to the next.  `-split-machine-functions`, `-hot-callsite-threshold=200`, `-Copt-level=2`, and keeping `macro_call`, `conditional` and `expand`'s other cases out of line: no change of cycles beyond the noise, and the last takes 1.6% fewer instructions and 8% more L1d misses on LMB.

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
default table), mktex* generation other than `mktexpk`. Each reports a clear error if a document needs it.
TrueType/OTF, subfonts, JBIG2, PNG, PDF inclusion, VF, PK and Type 3 fonts were first dropped or deferred, then ported after all:
`crates/fontfile`, `crates/image` and `crates/kpse` hold them, compared with the oracle.

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

**Pointers:** `type Ptr = u32`, `NULL = 0`, `mem_bot = 0`, `min_halfword = 0`, so that a node has the address it has on the oracle, which an old word can show (PORTING section 15).  `zero_glue` is at `null`, as in Knuth's own settings of `tex.web`. `mem` lo and hi regions are
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
  When the bump space runs out, and only then, a free block of a larger class is split, and if there is none, adjacent free blocks are joined: no block in use moves.  Without that a job that uses lists of one
  length after another fills the heap with free blocks and stops where the oracle goes on.
- **Reading:** `link(p)` walks become `i += 1`. `loc = null` tests in token-list state become
  `toks[loc] == 0` (`self.at_list_end()`).
- **Fixed lists** like `omit_template` and `null_list` sit at the bottom of the heap, above 255, since §532 tells
  `frozen_endv` from a character by `cur_chr>255`. The heap starts above `BUF_CAP`, and the cells below it are never written or
  stored, so `toks[loc]` reads as the end of a list when `loc` is a buffer index, and `get_next`'s leaf needs no test of `state`. A reserved cell above the fixed lists holds the capture loops' `any_user_outer`, so that a format and a checkpoint carry it.
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
  saved mark, the analogue of `def_ref`, and takes no block to show it, since a full heap is one reason to be there.
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

**Format = flat image of what `\dump` writes.** One loader for formats and preamble checkpoints, `image_load_bytes` in `image.rs`. Flat, ungzipped. A format holds only what
§1485-1507 dump, since its load runs in the middle of §1517 with the command line already in `buffer`; only a checkpoint restores the
whole image (`docs/PORTING.md`, notes on `image.rs`).
A format load puts each section directly in its place. A section of 64 KiB or more that a document hardly writes is mapped privately from the file, page-aligned there for that
purpose, so a document pays only for the pages it touches: `font_info` (5 MB in the LaTeX format, nearly all of it expl3's `\fontdimen` tables), the trie, `str_pool`, `str_start`, `hash`, `arena`.
That is two thirds of the LaTeX format. Those that a run writes all over, `toks`, `eqtb`, `hash_tab`, are read: that beat a mapping, whether taking a fault for each page or copying them at once with `MADV_POPULATE_WRITE`, on hello and on the thesis alike (`fmt_mapped` in `image.rs` has the numbers). Windows reads everything.
Huge pages only where a first touch is not a waste of zeroing 2 MiB: under a section of 1 MiB or more that is read, and past the first chunks of `toks` and `mem`; see `Tex::alloc`.
A document that reaches page 64 has shown it is long, and the chunks left out, of the scalars and `eqtb`, both ends of `mem` and the font arrays, then go to huge pages by `MADV_COLLAPSE`: on SGA3 at a load of 37, 72% fewer L1 dTLB misses on small pages, 58% fewer walks on them and 0.2 to 1% fewer cycles.
A checkpoint's restore loads in the same way: its long spans lie in the file at a place they can be mapped from, those of the same sections are mapped, and `hash_tab` too, since a run after the preamble writes to few of its pages, and the rest is read. It starts from a new reservation, so nothing lands on an earlier mapping, and a page it does write afterwards is copied then. That took hello's warm run from 2506 to 680 faults, and from 20.3 to 14.7 ms of CPU, against reading it all.
Publish by tmp + rename, also so that a running job keeps the file it mapped. Key on build-id, from `crates/engine/build.rs`. Sections store only the used part of each array.
The hash table is written with room to spare, or every run would move all of it at the first few thousand new names.

After load, §-level run-time init still runs: CLI overrides, `fix_date_and_time`, seed.

**Preamble checkpoint** (`crates/engine/src/checkpoint.rs`) is taken where `\document` first comes to `\@input`, by the first fetch of that control sequence
that `main_control`'s big switch makes after the one of `\document`. `.aux` is read by that `\@input`, at `latex.ltx:9489`, and `\@mainaux` opened at 9492, both after this point. What comes before it in `\document` is the same in every run: a hook, and loading l3backend, 7.4 of the 46.8 M instructions of hello. What that is worth is in section 16: most of a short document with a large
preamble, and for a long document with a small preamble its preamble and no more. A format with no macro `\@input` takes it at `\document`.

A restore has no Rust stack to return to, so it can only resume where it can rebuild the stack: `main_control`, in its `get_x_token`, with `macro_call` next.
`checkpoint_resume` does exactly that. The test is `at_big_switch`, stored around that one `get_x_token`, with `expand_depth_count = 0` and `scanner_status = normal`. Any other way to
reach `\document`, such as `\count0=\document`, takes no checkpoint and costs only time. Otherwise no position is excluded, in a group, in a box or with `\input` files stacked: all of it is POD. So a hook of `begindocument/before` that uses `\@input` itself is simply where the moment comes, and a hook that takes `\@input` away means no checkpoint.

**The trigger costs the token loop nothing, which is better than the one compare first planned.** An *armed* control sequence has `CK_ARMED` added to its `eq_type`, which sends every fetch of it, from a
file or a token list, down `get_next`'s existing `cur_cmd>=outer_call` path to `check_outer_validity`, where `checkpoint_trap` puts `cur_cmd` right. `\document` is armed
after the format load, and again after an assignment replaces it, so a class that wraps it still gets a checkpoint. The only readers of `eqtb` that bypass `get_next` are `eq_destroy`, `show_eqtb`, `macro_call`'s `long_state`, `capture_leave` and `eq_define`'s test for a self-assignment, which mask this bit and `CD_ARMED` of section 10.

**The validity rule:** a checkpoint records an ordered log of every observation of the outside world and is
valid only if each observation would get the same answer now. The log is kept only while a checkpoint may yet be taken.

- **Config, part of the file's name:** the program's path and stat (a build-id that needs no build step), `Options` and `Init` of the driver as they are, which is everything the command line and
  `maininit` decided, the `first line` included, cwd, relevant environment, and the raw value of every variable §1512 asks for. `setup_bound_var` is not in the log: its input is in the key.
  **`SOURCE_DATE_EPOCH` is the clock, not config:** only whether it is set is in the key, since a restore refreshes all that comes from it, as from the real clock. Otherwise nothing could test
  the patches under a pinned clock. `FORCE_SOURCE_DATE` and `TZ` are in the key. Expanded path specs of each format looked up, and the stat of each `ls-R`, are compared next.
- **`lookup(name, kind, must_exist)`:** run again, against the resolved path or NOT FOUND, including shadow probes (25 failed lookups in the thesis preamble). The probe of the output directory
  is a read of a path that may not exist, and it is logged as one.
- **Reads:** whole-file hash for closed files; prefix hash through the read position for files open at the checkpoint, and a line must still end there: a last line with no terminator that has grown is another line, and so is a CR at the end of the file that has got its LF. Project files always by content. Files below an `ls-R` by their stat, then by content if that
  differs, so a `touch` is no miss; one whose stamp was younger than 2 s at the read goes by content, since a stamp that young may not show the next edit. A format by its stat alone: hashing it would cost as much as loading it, and rebuilding takes longer than a timestamp's granularity. What is not a regular file, such as `/dev/stdin` or a FIFO, cannot be asked twice, so reading it is a hard taint.
- **The stat includes the change time.** Size, mtime and inode are not enough (trap `read/tds-tree-same-stat`): an edit that restores them with `utime` still sets `ctime`. Windows has no such thing in `std`, so
  `platform::STAT_SEES_CONTENT` is false there and every file read is hashed.
- **Stat-like primitives:** `\pdffilesize` compares the size only; `\pdffilemoddate` the whole stat. `\pdfmdfivesum`, `\pdffiledump` and image headers are reads.
- **Volatile reads**, hooked in `scan_something_internal` and the convert primitives. Date read: key on YYYYMMDD. `\time`, seed, deviates, `\pdfcreationdate`, `\pdfelapsedtime`: hard taint. These read too, by other doors: `\advance`, `\multiply` and `\divide` of a date parameter, which take the old value from `eqtb`, and `show_eqtb`, which puts it in the log.
- **Two sites are whitelisted, and whitelisting a read is not enough** (traps `volatile/c-sys-copied-in-preamble`, `volatile/pgfmath-seed-used-in-body`). What is whitelisted is a *sequence*, recognised exactly, and its product is then
  watched, because the danger is a copy, not the read. expl3's `\everyjob` must read `\time`, `\time`, `\day`, `\month`, `\year`, `\pdfcreationdate` in that order with `\everyjob` on the input stack, and after each read the next `\c_sys_*` must be made
  with the value that read gives. Anything else in `\everyjob` that reads the clock is a hard taint, so a format of one's own that keeps the time is seen. pgfmath must read `\time` and `\year` straight from `pgfmathfunctions.random.code.tex` in a tree, then define `\pgfmath@rnd@z` to their product.
  **Each product is armed as soon as it is made.** Fetching one is then a read of the clock, whichever way it is done: `\the`, `\let`, `\ifx`, `\meaning`, `\edef`. `\c_sys_{day,month,year}_int` count as a date read, the rest as hard taint. A fetch that
  only copies the token, in skipped text, a macro's argument or the body of a `\def`, is no read, since what it copies is evaluated later and gets the patched value. So a package that mentions the seed in a macro costs nothing, and one that uses it costs the checkpoint.
  A restore patches both: `\c_sys_*` in place, `\pgfmath@rnd@z` in place or in a new block if the digit count changed.
- **Hard taint, no checkpoint:** `\write18`, pipes, terminal `\read`, error state, a message on stderr (there is no journal of it). A page shipped out, too: the job id is made then, and the fonts' state on the way out is Host's. A PDF file open with only its header and `\immediate` objects
  is fine: `\pdfximage` opens one, and its bytes come back. A PDF or JBIG2 file written with `\immediate` is not: what it has copied stays in Host for the next image of that file. Nor is an output that went to `TEXMFOUTPUT`, since that the first place could not be written is an observation with no way to ask again. The whitelisted sites are hard taint with `\tracingmacros`, `\tracingcommands`, `\tracingassigns` or `\tracingrestores` on, as the log then shows what the patch cannot reach.
- **Refused at the moment itself:** a date parameter assigned in a group still open, whose old value on the save stack is the clock of this run. A first line of the log with no room for the widest date. A write stream past its buffer, which is stdio's 4 KiB, with a part on the disk where a restore has all or none.
- **The run's own writes** (`\openout`, log prefix, `.fls`, terminal) are journalled with bytes and re-materialised on restore. A file the run wrote and read back is answered by the journal, not the disk: in the log an `Output` entry stands between, so
  validation replays it as an overlay, and a file it left open across the checkpoint gets the journal's bytes. What the run read has to be those bytes, and `\pdffilesize` their length, or the image holds what came of another file: something beside the run may have changed it between the close and the read (`own-writes/read-back-what-another-wrote`). Deleting such a file is no miss, since the restored run makes it again. The date in the log's first line is spliced with today's; the field has variable width, so the journal keeps where it is. Taking a checkpoint flushes nothing, as a body that reads a stream still open could tell. The terminal gets its text last, since up to there a failure means a cold run, which says it all itself. SyncTeX's file is one of these: its name and its text so far are in the meta part and its context in the image, no member of it can lie behind a checkpoint, as that takes a sheet, and a restore makes the file again, by the name `-synctex` and the job gave it.
- **Stacked files:** `\begin{document}` may be in an `\input` file. Every stacked file stores path and offset;
  the offset is at line end and the hash covers the whole line, because the rest of the line is in `buffer`. `\openin` files likewise.
- **Do not validate by byte-comparing state**: one changed digit shifts 281k words. Use a canonical decode for
  testing.

On restore: refresh `\year`..`\time` unless the preamble assigned them, patch `\c_sys_*` and `\pgfmath@rnd@z`, in a new block if the length changed (`\c_sys_timestamp_str` does, with summer time in a zone at UTC in winter), reseed unless `\pdfsetrandomseed` ran, and forget the start time, from which `/ID` and `/CreationDate` come. An assignment
that leaves a date parameter as the clock gave it is indistinguishable from none, or from a group's restoring it, so that is no checkpoint.
Images are read again at once, before anything else, in the order of the table, with nothing said or recorded and none of the parameters of now: the count of a PDF file's uses decides how long the document stays open, and with it what has been copied, and the checks and warnings of `\pdfximage` were made when it was named.

**The file:** `rstex/checkpoints/HASH.ck` in the cache directory, by the hash of the config: header, meta (the log, the journals, the spans), image (the live range of each POD section from `POD_SECTIONS`, less runs of 16 or more zero pages, since a restore starts from a zero reservation and a third of hello's bytes are `font_info` that expl3 reserved). A restore reads the meta part and answers the log before it touches the image.
XXH64 covers the header and meta. The image has a Fletcher sum in four lanes, a third of the instructions per byte, over every byte, mapped or read. A section that fails its sum leaves a dirty image, so the driver takes a new one and goes on cold.
**The seal.** That sum was a sixth of a restored hello's instructions, and it touched every page of what is mapped so that a run pays only for what it uses. So it is done until it has passed once, for a file whose stat is old enough to show any later change, 2 s as for the index in `crates/kpse`, and then the seal, a hash of the stat and the sum, goes in `seen`. A later restore with that stat does no sum. That rests on what the tree's files rest on: the stat includes the change time, so a file cannot change without it showing, and a power failure right after writing is caught by the first restore, which still sums. Windows has no such stat, `platform::STAT_SEES_CONTENT`, and sums every time.
Publish by tmp + rename. The cold path copies the image and a worker thread hashes and writes it, joined at exit; no fork, no kernel tricks, so the same code on all three systems. What the deflate workers hold is put in place first (`pdfout_settle`), since the offsets and the file's bytes are part of what is kept.

**Who can write the cache.** A checkpoint and a format are memory images: mapped, trusted, and read with unchecked indices. The hashes have no key, so they are against accidents only, and a hostile file means memory corruption. So `rstex_kpse::cache_dir`, the one idea of that directory, makes it with mode 0700, and gives none if it is a symbolic link, another user's, or something others can write; a run then goes without, quietly. An existing directory that only its owner can write to is made private, as it holds the text of preambles. A name that is not absolute counts as none, as the XDG specification says, or a cache below the current directory would be whatever a document's directory holds. root makes none in a user's home. A file in it is opened with `O_NOFOLLOW` and `O_NONBLOCK`, and must be a regular file of this user that no one else can write. Temporaries are `O_EXCL` with mode 0600, so nothing planted is followed. Lengths, counts, spans and file slots are checked before use all the same, and the build id is in the key.

A format's body has no sum. Checked once for each run, it was measured to cost 9.4% of hello's instructions (46.8 to 51.2 M) and 32 faults, and 0.1% of the thesis's. What stands against a damaged format is its size and closing word, its header's build id, the range checks of §1486 on, the stamp in its `.deps`, which sees any edit in place, and a sync before its publishing rename. A bit that rots on the disk is not seen. A hostile format is another matter: `TEXFORMATS` has `.` as TeX Live sets it, so a file `pdflatex.fmt` in a document's directory is loaded, as the oracle would load it, and only this build could have written one that loads. Nothing here stops a crafted one.

A mapped file cut short from outside gives SIGBUS at the next page touched. Since rstex publishes by rename, only an outside actor can do that. The handler that catches a full stack, section 12, says so too and exits 1; a rerun from there would show the terminal everything twice.

**Whether to write one.** Writing costs 7 to 8 ms of CPU on a quiet machine, whatever the document, which is most of a run for a small one and 1% of a long one. A run writes at once if its preamble read 200,000 bytes or more, which the log can say at no cost: the format not counted, and of a file still open only what has been read. On the ladder and the user's documents that divides those a restore saves under twice what writing costs, hello at 59 KB up to `ladder/12-graphics` at 113 KB, from those it saves more, `ladder/17-class-amsart` at 227 KB and up. So at worst a run costs a third more, and the next one gets it back several times over. Bytes and not the clock: on this machine under a load of 40 hello's process has used 52 ms of CPU at that point, ten times what it uses on a quiet one, and a run would decide by its neighbours, which also made `policy.py` fail 6 of 9 cases when it did. A preamble that computes a lot from little is only written a run later. Otherwise the run writes the identity of its preamble, a hash of the log without its stats, to the slot of its key in `seen`, and goes on; a later run that finds the same identity writes. So a file compiled once pays for some ten system calls and a file of 64 KiB, an edit of the body or a `touch` changes nothing, and a preamble being worked on writes nothing until it has stayed the same for two runs. Recording what it observes, which a run has to do from its start to be able to write at all, is 0.35 M of hello's 43.7 M instructions and 0.66 M of the 228.6 M of `ladder/07-amsmath-suite`.
`RSTEX_CHECKPOINT_POLICY=always` writes at first sight, `second` never does. Everything in `tests/` that judges a checkpoint says `always`, and `tests/corpus/snapshot/policy.py` tests what a run does by itself. A writer in a forked child was measured and not built: on hello the parent saves 13 of the 16 ms of CPU a write adds at a load of 22, at 384 more faults from copy-on-write, and 1510 on the thesis, where the difference is within the noise. All the CPU is still spent, by a process that outlives the run, once per document, and a fork in a process that may have threads, with another way on Windows, is too much risk to repay that.

**The disk.** `checkpoints/` holds at most 1 GiB, and a file not used for 30 days goes whatever the room, least recently used out first; a use marks a file once a day. A temporary file of a run that died goes after a day. `seen` has a fixed size, 4096 slots of two words with holes where the system has them, and a document on a taken slot only costs it a later checkpoint or a sum done again, so it needs no pruning, and 4096 documents never compiled twice take no more room than one. The tables of `crates/kpse`, `kpse-index-*` and `map-*`, one for each environment and search path, keep their 8 newest of each kind and whatever is under an hour old. All of this happens when a file has just been published, the one moment the directory grows, and costs a run that publishes nothing no system call. `outcache/`, section 9, holds an index for each PDF compiled twice, of some tens of KB: at most 1024, those of the last 30 days aside, looked at by 1 in 64 of the runs that add one, and the 32 KiB of its own `seen`.  So the cache holds at most 1 GiB and some 25 MB, what `outcache/` comes to, and formats as `crates/rstex/src/autofmt.rs` says.

**The driver:** a snapshot is used unless `-ini`, `--no-snapshot` or `RSTEX_NO_SNAPSHOT` says otherwise, or there is no command line to key it on. An explicit `-fmt` is fine: the format's stat is in the log. `tests/integration/run.py` and `tools/prof/` run with
`RSTEX_NO_SNAPSHOT`. `tests/corpus/run.py` gives candidates a cache per work directory, so a compare judges a cold run that also writes a checkpoint, which shows that writing one disturbs nothing. Restored runs are judged by `tests/corpus/snapshot/run_snapshot.py`
and `warm_equals_cold.py`, and runs from page records by `pages.py` too. `RSTEX_CHECKPOINT_TRACE` says on stderr what a run did and why. The output cache of section 9 goes by the same words, for what judges the literal engine, and a restored run with a PDF already open reads the old one just before it truncates it, as a cold run does.

**`--until-stable`,** `crates/rstex/src/stable.rs`, with its use in `README.md`. A pass is an ordinary run: the flags go before anything looks at the options, so it has the key of a plain run, and each takes the other's checkpoint, unless there is an output directory. Then it has the environment latexmk gives its engine, with that directory on `TEXINPUTS` and `BIBINPUTS` and in `TEXMFOUTPUT`, since a document may `\input gen` of a `gen.tex` it wrote there, and `TEXMFOUTPUT` is in the key. The engine notes what the pass does with files outside the tree in `World::pass`, `None` otherwise, which costs a test on each file opened and nothing else, and at the end of `uexit` calls the driver. That decides: an exit, or the program again by `exec`, on Windows as a child it waits for. So there is no parent to pay for, no second process image, and a signal to the process id reaches the pass that is running. There is another pass when a file read before the pass wrote it holds something else now, by XXH64 of the bytes the read had at hand, or a file read and not written has another stat and other content, or a name that was looked for and not found has been written. A restored pass knows the same from the checkpoint's log, and what a stream open at the checkpoint held is taken just before the restore makes it again. That is latexmk's rule on what its `.fls` and log tell it: a file first read after it was written counts for nothing, and neither does the job's own PDF. A save that settles at once reads only what it must, the small files it read and wrote, once more each. latexmk's seed `.aux` is kept too, for the sake of equal bytes when the first pass fails, and it spares a simple document a second pass. A pass failing ends it all, with TeX's status: latexmk stopped there on every such document of the sample (`tests/until_stable/compare.py`). The one exception is a pass that found a side file not ending a line, which only a pass killed leaves, an editor's at each newer save, and has written it whole: its error may be nothing of the document's, so there is one more pass. Over random kills, without this 9 of 92 next runs found half a line, and each of those ended in an error and a PDF with no references until the next save; with it 26 of 180 did, and none ended so. A cut that falls at a line's end and leaves a group open is not seen, as in the `.aux` of an `\include`. A kill before the job has its name leaves `pdflatexPID.fls` in the current directory, as the oracle does, and nothing takes it away. On Windows, where a process cannot be given an `argv[0]`, the next pass is told the program's name by `--until-stable-name=`, as that is in the key and in what a warning prints. The programs of `README.md` are named, not run. Each is judged by a digest of exactly what it reads against the digest of what its output was last known to come from, in `<job>.rstex-tools`: if the stamp of the output has moved, the program has run, on what TeX had left on the disk then, which is what the pass found when it began, and so the `.aux` of a pass is kept until its end. latexmk reruns bibtex for any change in the `.aux` and makeindex for any `.idx`; here a new label does not name bibtex, and a `.idx` with no `.ind` wanted does not name makeindex, which is what lets an editor stay off latexmk for a document that has them.

**Not done: a second pass that changes nothing.** The `bookmark` package writes each heading's source line to the `.aux`, as `srcline={N}` in `\BKM@entry`, so a save that adds a line before a heading changes the `.aux`, and takes a second pass by latexmk's rule and by this one. With an edit that adds a line, in 20 saves at random places: the thesis took a second pass on 19 and LMB on 16, and on 32 of those 35 the `.aux` differed in `srcline` alone. On all those 32 the second pass left every file as the first had, the log and the PDF among them. An edit of one word within a line took one pass on 39 of 40. The package uses that number only in a warning's text, so a digest that masks it would halve such a save. It is not built: a pass skipped has to give what latexmk gives in every case, the digits of the number are tokens that a pass holds, and a log's memory figures or a warning could show them, so it needs its own switch and its own gate.

**Page records** (`crates/engine/src/checkpoint/pages.rs`). An edit on page 30 of a long document changes nothing that TeX did before it read that line, so a run can start from there. A run restored from a preamble checkpoint goes on recording, and leaves a chain of records in a directory beside the checkpoint, `HASH.pg`, which hangs from that checkpoint by a hash of its header. A later run answers the preamble's log as for any restore, then the body's log in order, and takes the latest record that holds. Its image is the preamble's plus, for each page, the newest version among the records up to it, and it goes on writing from there.

- **Where.** The big switch, right after a command that can ship a page, which has no Rust stack below `main_control` and nothing of its own to bring back: `control.rs` tests one byte in the two arms that reach `ship_out`, and nowhere else. A restore has no `macro_call` to make. A record is put off to the next page if a native diagram or an error is under way, a PDF or JBIG2 file is open, whose state on the way out is Host's, a date parameter is on the save stack, or a file slot is open that cannot come back; an input the run wrote itself is such a slot. All of what an error leaves is POD, so records go on past one, and one that needs the terminal is a hard taint. Writing stops for good at a hard taint, in DVI mode, with `\pdfuniqueresname`, after a failed write, or under SyncTeX, whose stream a record cannot give back yet: with the option, or past a first page that left its file open, whatever `\synctex` says by then, since that page either opened it or turned it off for good. Only what comes before that record stays. So a chain never costs bytes, and at worst it stops being of use.
- **What a record holds.** The pages of the image that differ from their last version, by 4 KiB of the image's offset; the observations since the last record; how long each output stream was, and what waited in its buffer; how far each open file had been read. Output only grows, so there is one file for each stream, the PDF, the terminal and the `.fls`, and a record is a set of lengths. Going on from an earlier record cuts them back, the index first. A stream comes back with what was on the disk on the disk and what waited in the buffer in the buffer, so a read of it, or its next flush, is as it was. What is in a frame of the index is checked as a checkpoint's meta part is, and a frame goes in last, so a process killed at any moment leaves a chain that is good to its last frame.
- **The validity rule for a file that changed.** A record holds if each observation before it would get the same answer, as for a checkpoint, with one more way out: a file it still has open may differ past the place it had read to. For that a record keeps a hash of what was read of each open file since the last record, and no copy of it. So nothing of a document is kept twice, and an edit does not cap what a later resume can use, as a copy made when the chain began did: a letter on page 32 after an edit on page 4 cost 754 M against 241 M. A record is good for a file if each of those hashes holds on the file as it is now, and a line still ends at the place (`prefix_ends_a_line`). A later frame says what a file open at a resume has become, so that a record past the resume is judged by the file it read. A file the run wrote and read back comes back from the journal, so the hash of what it read must be that of the journal: a run beside it in the same directory could have changed what it read, and pdflatex's next run would put that right.
- **What a writer compares with.** There is no copy of the image. A page's last version is in the checkpoint if it has not changed since the preamble, and otherwise in the chain, and both are looked at in the system's cache of the file, mapped, by `platform::FileView` and over a reservation that has the chain's file at the same offsets. Reading them needs the worker to have finished with the last record, and the one buffer that goes between them says when it has. A record's bytes go through that buffer, on huge pages, not on the heap. What a compare would fault in page by page is mapped in one go. So a run that reaches no record has spent nothing, and one that writes a whole chain takes few faults more than a warm run with none, where a copy of the image took 6924 against 1118. Windows maps nothing and reads those files in when the first record is due, and `RSTEX_PAGE_NO_MAP` goes that way on any system.
- **Whether to write.** Only a run that restored something: a cold run has a checkpoint to write, and a first run of a document is not worth more. A record every `max(4, pages/64)` pages, with `pages` the largest of this run's count, the last chain's and what the run that wrote the checkpoint said of a document of more than 256 pages, in a small file where the chain will be: every page takes 1231 MB and 3.8 to 8.8% more instructions on SGA3, against 0.7 to 1.2%. Records are not reused across a changed `.aux`, whose read defines macros, and a run does not stop early when it reaches a page the last run had: the state differs by then wherever the node count changed, 90 KB on the thesis and growing.
- **No room.** A worker that cannot write, on a full disk or under a limit on a file's size, says so when it gives the buffer back, and writing stops there: what the run would compare with next is not in the file, and a mapping past its end ends in SIGBUS. What is in the chain stays good. `pages.py`, `no_room_for_the_chain`.
- **Damage.** Each part of a record has a sum, and each page of the image one of its own, since a resume reads only the newest of each. All that comes from the chain is read and summed before the disk is touched. A sum that fails takes the chain away and the run goes cold.
- **One run at a time.** A lock file in the directory, `File::try_lock`, which the system lets go of when a process dies. A run that does not get it restores the preamble and writes nothing.
- **The token heap.** Its free lists and its top are POD and its limit a constant, so a run from a record reclaims as a cold run does when the bump space runs out. A record holds the heap as far as it is live, so what `tok_coalesce` gives back to the bump space and what `tb_show` used above the top are left 0, as they are in a run resumed. With the feature `tight_heap`, which sends ordinary documents through `tok_reclaim`, the end of the bump space is in the image for the same reason: `pages.py reclaim`, on such a build, holds the state of a run from a record against the writer's.
- **The disk.** A chain counts in the 1 GiB of `checkpoints/`, stops growing at 256 MiB and stays good. `cache_prune` looks when a chain is new or has grown by 16 MiB in a run: a chain goes with its checkpoint, on its own if it is the oldest or a month old, and not while a run holds it. A use marks it once a day.
- **With the output cache of section 9.** A run that resumes makes the PDF again, which is what the cache must read before. So `pages_restore_host` has it read what the old file holds after the record: the part before it comes from the chain, and is not read. Those streams stay in the next index as they stand, or a later edit further back would deflate all that lies between. An index is a claim that is checked by a sum whenever it is used, so one that turns out to have other bytes at that place, as after an edit that has been undone, costs a stream deflated. A stream with an equal one behind the record is not looked for, and is deflated: `behind` in the trace. Fonts and CMaps go out at the end of a run, so a resume finds them all, and they were most of what a resume had left.
- **With `--until-stable`.** A pass from a record has read what the logs up to it say, in the preamble and in the body, and only the chain knows that: `pass_note_restored` hears all of it before any file is made again, or a pass would not see that the `.toc` it read on the first page has changed. A pass lets go of the lock before the next, which on Windows it waits for. Under `-recorder`, a record that fails after answering lookups that made kpathsea read something goes cold (`what_the_recorder_would_miss`), which also takes the chain away, so a second pass that a changed `.aux` needs there costs a cold run and the next save writes a chain.
- **The switches.** `RSTEX_NO_PAGES` is the path without any of it, and `--no-snapshot` and `RSTEX_NO_SNAPSHOT` have none of it either. `RSTEX_PAGE_CK=N` asks for a record every N pages, `RSTEX_RESUME=k` for no record past the k-th, `RSTEX_PAGE_READONLY` resumes and writes nothing, and `RSTEX_RESUME_FORCE` takes the record whatever the log says, the gate's negative control.
- **The gate.** `tests/corpus/snapshot/pages.py gate`, on a document made there with as much across a page break as one can hold: a resume from every record, also under `-recorder`, past errors and with no mappings; the world changed in 47 ways, each at the record it should stop at; every byte of a chain at risk, by flips and cuts; SIGKILL during a run and during a resumed run's cutting; runs at once; the real clock. `--keep-pdf` does all that again with each run finding the last PDF, where `verify` holds runs that find their streams against a chain that deflated them. `RSTEX_PAGE_HASH` and `RSTEX_PAGE_VERIFY` compare the whole state of a resumed run with that of each later record, since bytes alone saw 0 or 1 of 12 flips where the sums saw 6: the deflate workers are off for that.  Which document an image is, a place in a list of this process that would differ in the image and in every copy an arena has left behind when a table grew, is in Host. `warm_equals_cold.py` judges what a run from a record leaves against the oracle, on real documents, and `pages.py session` is a sitting at the editor against the preamble alone.

What it comes to, on its branch, before the merge with the output cache and the macro lever: section 16 has what the merge gives. A plain build at a load of 30 to 60, so instructions are what to read, and the rest are ratios in turns. *Writing* is a run from the preamble that writes a whole chain, and *last* a run from the last record, with no edit or an edit after it.

| document | records | chain MB | checkpoint MB | Minstr warm, no records | writing | last | faults warm | writing |
|---|---|---|---|---|---|---|---|---|
| hello | 0 | 0 | 9.30 | 9.92 | 9.96 | | 296 | 298 |
| the thesis | 8 | 14.5 | 14.84 | 942.7 | 976.3 | 199.9 | 1899 | 1980 |
| LMB | 8 | 14.2 | 15.19 | 956.3 | 990.4 | 221.6 | 1925 | 2043 |
| SGA1-VI | 6 | 13.1 | 10.80 | 1244.8 | 1267.2 | 187.6 | 1288 | 1353 |
| `arxiv/math.AG/1912.07119v3` | 10 | 14.9 | 12.35 | 1475.0 | 1509.6 | 176.6 | 2997 | 3294 |
| SGA3-English | 64 or so | 141 | 11.45 | 37252 | 37536 | | 3326 | 3328 |

On the thesis over 41 rounds, against a warm run with no records: writing 1.036 of its instructions, 1.063 of its cycles, 1.093 of its CPU and 1.047 of its wall; the last record 0.212, 0.137, 0.207 and 0.177. Two warm runs against each other, 1.006, 0.997 and 1.058 of cycles, CPU and wall. Most of what writing costs is the compare, and what a resume has left is mostly the fonts. The fault counts move with what huge pages the system has to give: with all it wants, writing on the thesis takes 900 to 1400. Nothing here has been through PGO.

A sitting at the editor, `pages.py session`: edits of a letter, a word or a paragraph, placed uniformly over the records, each kept, each run to the fixed point as latexmk would run it, against the preamble's checkpoint alone. The two gave the same bytes at every step.

| document, edits | first pass, instructions | to the fixed point | first pass, CPU | to the fixed point |
|---|---|---|---|---|
| the thesis, 14 | 0.492 | 0.614 | 0.540 | 0.669 |
| LMB, 14 | 0.500 | 0.595 | 0.567 | 0.692 |
| `arxiv/math.AG/1912.07119v3`, 12 | 0.583 | 0.693 | 0.591 | 0.702 |
| SGA3-English, 8 | 0.427 | 0.543 | 0.455 | 0.579 |

The same at the merge, with the output cache and the macro lever at work on both sides, so that against a run that takes its fonts from the last PDF as well. A plain build at a load of about 20, and on the thesis and LMB the PGO one too. 0 of 48 and 0 of 28 differ.

| document, edits | first pass, instructions | to the fixed point | first pass, CPU | to the fixed point | PGO: first pass, CPU | to the fixed point |
|---|---|---|---|---|---|---|
| the thesis, 14 | 0.402 | 0.545 | 0.513 | 0.653 | 0.553 | 0.689 |
| LMB, 14 | 0.385 | 0.502 | 0.560 | 0.683 | 0.542 | 0.647 |
| `arxiv/math.AG/1912.07119v3`, 12 | 0.516 | 0.644 | 0.644 | 0.789 | | |
| SGA3-English, 8 | 0.430 | 0.545 | 0.459 | 0.582 | | |

An edit that changes the `.aux` costs a second pass that has nothing to resume from, and an edit in the first pages gains nothing and pays for the writing.


**Not done.** A safe point at any top-level main-file line, so that editing a `\newcommand` costs ~34 ms, not 650; a second tier after `begindocument/end`, also keyed on the `.aux`, worth 36-46 ms. `at_big_switch` is the only part of the first that exists.
A checkpoint made with the format for the commonest class, so that cold hello would need no earlier run. Of cold hello's 46.8 M instructions, `\documentclass{article}` is 12.7 M, `\begin` up to `\document` 16.3 M, most of it lthooks setting up 71 hooks in 14,700 macro calls, and `\document` up to `\@input` 7.4 M. A format checkpoint can only hold a prefix of the document, and any `\usepackage` ends the prefix at the class: 27% of hello on a first or only run, nothing on a later one, and nothing for `\documentclass[12pt]{article}`, `report` or `book`. Only a document whose whole preamble is that one line would get the rest. For that, what it would hold depends on the main file's name, which is in the string pool under every later string and sets where the log and the terminal wrap for ever after. Too much to prove equal for that gain.

## 8. Hot paths

Implement after TRIP passes on the generic path; keep a `slow_capture` feature for differential testing.

- **get_next** is three layers, none of which needs a frame but the last. `get_next_leaf`, `#[inline(always)]`, about 16 instructions:
  character tokens, braces and control sequences that are neither outer nor §364's; it commits nothing until it knows, and
  gives the token, so `get_token` has `cur_tok` without a test. `get_next_hot`: the end of a list and §381 for the usual types, each
  followed by the leaf again, and a letter from a file. `get_next_literal`: §363 as WEB has it, for everything else. Frequencies on the
  thesis, of 133 M fetches: letter, other and space 48%, control sequences 30%, braces 11%, ends of lists 7%, §381 3%, file 0.7%.
  `cur_cmd`/`cur_chr` stay fields.
- **back_input** stays a level of its own, however short the list: §311 shows it as `<to be read again>` and then `<recently read>`,
  until the next `get_next` removes it. Decrementing `loc` would fit only 41% of the calls on the thesis, and would cost those
  lines in the context of every error. What it can do without changing anything: a level of one token backed up before takes the new token,
  if the level below stays, since ending it would free the block that `tok_one` then gets back.
- **Capture loops** in the group-contribution loop, `scan_general_text`, `pass_text`, macro argument scan: when the
  input is a token list, copy tokens with brace counting and bail to slow on end of list, `&`, out_param,
  `\par` when non-long, the `\noexpand` token, `\endwrite`, or any user `\outer` (a sticky `any_user_outer` flag, which an armed control sequence does not set).
  Replaces ~54 M get_next + store paths per thesis.  Built in `crates/engine/src/capture.rs`: 83 M of the thesis's 124 M tokens now take a loop, at 7 to 14
  instructions each where the literal path takes 29 to 74.  A copy-free argument was not built: `get_next` would have to test for a length on every token.  Neither was a
  four-at-a-time SSE2 filter: it used more instructions than it saved, since a brace or a one-letter name comes too often.
  What was built is the one case with nothing to scan: `{#n}`, an argument handed on whole, takes the same block with one more reference, if the block says that a loop made it, with no user `\outer` about and no `\par` where the next macro stops at one.  `RSTEX_NO_SHARE` turns it off.
- **`\pickup@font` for a font microtype has set up** does what its macros leave behind, in `crates/engine/src/native_font.rs`, when it can prove that that is all they would do: some 25 macros on each `\selectfont`, the largest macro cluster in a long document with microtype.  Guarded by the hash of each macro, so another version of microtype runs the real ones.  Off with `RSTEX_NO_PF`, in a checkpoint's key.  It is not under `--no-commute`: the gate's compare judges it against the oracle, byte for byte, as the literal engine.
- **expand and its neighbours.** `expand` takes a macro, `\expandafter`, `\fi`, `\csname` and the conditionals that scan no number without the four backups of §388 (`expand_saving`). `scan_int`, `scan_dimen` and
  `scan_keyword` read constants from a list at once, under the rules of PORTING section 20, "Levels that cannot show". `\def` and `\let` come first in `prefixed_command`.
- **`search_string`** looks in an index the format holds for the format's strings, and in one in Host for those the run made, so nothing is built for what is in the format and nothing is scanned twice.
- **Stats** (`dyn_used`, etc.) stay: behind a feature they would save under 0.1%, since `var_used` is one add per node and the `max_*` tests double as overflow tests.
- **Build:** `opt-level=3`, `lto="fat"`, `codegen-units=1`, `panic="abort"`, `+crt-static` (0.38 vs 0.75 ms exec;
  also skips the system `ld.so.preload`), not position independent (no relocating 6000 pointers on each start, at the price of ASLR of the image and a walk of `.eh_frame`), and PGO with ext-TSP block placement, trained on the ladder, the format builds and long public documents, cold and warm.
  `docs/BUILD.md` has the one command that makes a release, and what was measured for each choice.
- **Engine stack:** 1 GiB, since TeX recurses deeply: only reserved.  With glibc the process's one thread moves to it by `swapcontext`; elsewhere it is a dedicated thread.  A full stack gets one line of its own: `docs/BUILD.md` has what the handler costs, which is nothing that can be seen.
- **Start-up:** `main` is the C runtime's, without Rust's `lang_start`, which reads `/proc/self/maps` and installs handlers of its own.
- **Fatal errors:** `overflow`, `confusion`, `fatal_error`, `succumb` return `!` and reach `jump_out()`, which closes
  files and calls `process::exit`. No `Result` in the engine. `error()` remains an ordinary call and return.
- **Gotos:** forward `done`/`exit` → `'done: { ... break 'done }`; backward `restart`/`continue` → `'restart: loop`;
  irreducible cases (main_control's lig/kern loop, hyphenate/reconstitute) → `let mut pc = Pc::X; loop { match pc {...} }`.

## 9. PDF backend and fonts

- **Deflate:** `zlib-rs`, pure Rust, with one state per thread that goes to the level of each stream, since a state is 64 page faults. In a 158-stream test its level 6 took 18.9 ms and 929,066 bytes, against
  14.2 ms and 924,349 bytes for libdeflate (C) and ~45 ms for system zlib 9. Staying pure Rust costs about 5 ms on the
  thesis. Content and ObjStm at level 6, FontFile at level 1 (eexec is incompressible). A stream is always deflated
  whole, in one call: zlib-rs's bytes depend on where the pieces begin (38 of 240 cases at levels 0, 1, 5 and 6), so
  piecewise writing would give other bytes, and a worker that gets a whole stream gives the same ones.
- **Parallelism is plumbing, and worth about 1 in 25 on a long document.** Whole streams go to worker threads
  (`crates/pdfio/src/workers.rs`): on SGA3 that takes 3.1% of the instructions and 9% of the L1d misses off the engine thread, which
  does not wait for them, and the PDF is the same byte for byte.  One thread starts when 128 KiB has been through `write_zip`, so
  hello pays nothing, and more only if a queue forms, up to 3 and one less than the CPUs; with one CPU there are none.  The main
  thread assigns object numbers and keeps a stream's place in the chunk list; `pdf_gone` and `obj_offset` do not count what the
  workers have until `pdfout_settle`, just before the cross-reference table, when a job stops early or before a checkpoint, puts it right.
  `RSTEX_DEFLATE_WORKERS=N` asks for N from the first stream, and 0 for none.
- **Fonts:** port `writet1.c` semantics and the MD5 subset tag. The first run that parses `pdftex.map` keeps its tables in the cache directory with the file's stamp, instead of
  the 4.5 ms per-run parse; a run maps them, and an entry moves to the image's own tables when something first asks for one of its names (`pdffont.rs`, at the top). Leaf crates are
  stateless functions; they never call back into the engine.
- **Font expansion is mandatory.** microtype turns it on in all the user's documents; 1293 of 2441 `Tm` operators in the
  thesis have horizontal scale other than 1. Protrusion too.
- **Required file readers:** TFM, PFB, ENC, MAP, JPEG. Then PNG, PDF inclusion, VF.
- **The last PDF is the cache** (`crates/engine/src/outcache.rs`).  Between two runs most streams do not change, and the last PDF holds them deflated, so no stream is stored twice: `rstex/outcache/HASH.oc`, by the hash of the PDF's path, says for each stream of that file a 128-bit hash of its input and level, where its bytes lie, and a sum of those bytes.  A run reads those parts of the old file before `b_open_out` truncates it, and no others, since a picture copied as it stands is not in the index and may be nearly all of the file.  A stream in the index takes the old bytes, checked by the sum, instead of being deflated: the first point of this section says that they depend on its input and level and nothing else.  The index carries the build id and a fingerprint of the CPU, since nothing shows that zlib-rs's code paths give the same bytes, and is opened as section 7 says of the private cache.  A run that changed nothing writes nothing.

  Three kinds of stream are known without being made, which is where most of the gain is.  A Type 1 font is found before `write_t1` runs, by all that function reads, with the file by its stamp: `CarryUse` says when its result did not depend on what earlier fonts left in the statics, a tag moved by a collision or a warning means no entry, and a tag now taken means parsing it after all.  Windows has no stamp to trust, `platform::STAT_SEES_CONTENT`, so it keeps no fonts, and gets the rest.  A `/ToUnicode` CMap is found before `write_tounicode`, by the glyph names, the names of the font and the encoding, `\pdfglyphtounicode`'s entries since the format, and a hash of the format's own table, which the format holds so that no run reads all of that to ask.  A PNG is found before `write_png` by the bytes of the file and the parameters, as a list of operations with the stream bodies left out and its slots relative.  Only a stream with nothing in it takes what was found, and what was made is kept only for a stream of its length, so a stream that came between could not be taken for it.

  The first run of a PDF hashes nothing and leaves a word in `outcache/seen`, a table of fixed size as in section 7, since most of what a file compiled once would pay for is hashing it can never use; the second writes the first index; the third finds things.  `RSTEX_CHECKPOINT_POLICY=always` writes one at first sight.  `-ini`, `--no-snapshot`, `RSTEX_NO_SNAPSHOT` and `RSTEX_NO_OUTCACHE` give the literal path.  `RSTEX_OUTCACHE_TRACE` says on stderr what a run took.  `tests/gate.sh`'s step `outcache` judges it: `tests/corpus/snapshot/outcache.py`, what a change to the world may do; `tools/prof/critic/oc_adv.py`, the changes a key might not see; `warm_equals_cold.py --keep-pdf`, where every stream must come from the old file and the outputs be those of a cold run, on the ladder, the user's documents and SGA3, and under the oracle's compare; and `--check-mutants`, that this fails on caches wrong on purpose.  `crates/fontfile/tests/carry_use.rs` tests `CarryUse` on the fonts of the tree.

  What it comes to, by `tools/prof/outcache/paired.py`: each from a sealed checkpoint with the last PDF in place, against the same binary with `RSTEX_NO_OUTCACHE`, the medians of the ratios of 41 rounds, 9 for SGA3, at a load of 35 to 56.  In brackets is a second run with no cache against the first: the noise.  So instructions are exact, and cycles and CPU good to a few percent.

  | document | Minstr, plain, no cache | hits | ratio | cycles | CPU | PGO: Minstr, no cache | hits | ratio | cycles | CPU |
  |---|---|---|---|---|---|---|---|---|---|---|
  | hello | 9.94 | 5.59 | 0.562 | 0.737 (1.015) | 0.918 (1.037) | 9.24 | 5.27 | 0.571 | 0.715 (0.999) | 0.878 (1.013) |
  | the thesis | 943.0 | 765.2 | 0.811 | 0.909 (1.003) | 0.902 (1.005) | 880.5 | 719.0 | 0.817 | 0.875 (1.001) | 0.896 (1.006) |
  | LMB | 956.6 | 741.3 | 0.775 | 0.852 (1.011) | 0.866 (1.011) | 890.9 | 695.2 | 0.780 | 0.857 (1.010) | 0.857 (0.991) |
  | summary | 269.5 | 186.5 | 0.692 | 0.799 (1.010) | 0.830 (1.040) | 250.7 | 176.6 | 0.704 | 0.812 (1.001) | 0.835 (1.021) |
  | SGA1-VI | 1241 | 1085 | 0.874 | 0.927 (1.004) | 0.919 (1.016) | 1158 | 1015 | 0.877 | 0.955 (1.006) | 0.955 (1.010) |
  | SGA3-English | 37250 | 35035 | 0.941 | 0.956 (1.012) | 0.950 (1.011) | | | | | |

  On 22 arXiv papers picked at random, 5 rounds each, by `tools/prof/critic/oc_census.py`, the medians are instructions 0.78, cycles 0.84 and CPU 0.835, the largest CPU ratio 0.997 and the smallest 0.532, and every output the same.  A word added to `arxiv/math.AG/1912.07119v3` takes 0.744 of the instructions, and an edit of its preamble, with the checkpoint no use, 0.841.  The one class that used to lose is a large file that is mostly a picture copied as it stands: on 210 MB with a JPEG, by `tools/prof/critic/bigpdf.py`, the run now reads 13 KB of the old file and keeps the same RSS.  In 41 rounds, plain, instructions are 0.934, cycles 0.978 (0.962) and CPU 1.013 (0.981), and with PGO 0.938, 0.981 (0.982) and 0.981 (0.969).  The wall clock's ratios are 1.010 (1.022) and 1.033 (0.974), and those of its medians 1.001 and 0.973: no more than the noise.  It costs a first run 0.02% of hello's instructions and nothing of the thesis's, and some ten system calls for the word in `seen`; the second run, which hashes and writes, 0.2 to 0.7%, and a run that restores with no PDF to read from pays the same.  SGA3's gain is CPU on the deflate workers, which the engine's thread never paid: that thread runs 0.993 of its instructions, so the wall clock gains little.  An index is 3.5 KB for hello and 78 KB for the thesis.
- **SyncTeX's file**, `crates/engine/src/synctex.rs`, which is `refs/synctex/synctex.c`, and `docs/PORTING.md` says what that comes to. An editor always asks for it, and pdfTeX's `gzprintf` for each record is a third of a warm run of the thesis if it is paid here. So the text is the oracle's byte for byte and the container is not, by the owner's decision: gzip members one after the other, which `gzread` reads as one stream, one for what comes before the first sheet, one for each sheet, one for the end.
  - A record goes straight to a buffer, a number in one word by multiplications with no branch on its length. A member goes to the deflate workers, or is made on this thread if it is small and they are not up, as hello's are; a byte of it counts for 8 of `write_zip`'s toward starting them. A member's bytes depend on nothing but its text, so `RSTEX_DEFLATE_WORKERS` changes none of them.
  - The file comes into being when the oracle's does, for what a run may ask of it, and gets its bytes when settled, in one `write_vectored`: at the cross-reference table and at the end. `pdfout_settle` says the length at which it is whole, for what wants to go back to a place in a run.
  - Level 1. On the thesis's text with the path on, 1.18 MB, by member: level 1 455 KB and 22 M cycles, level 2 324 KB and 37 M, level 3 291 KB and 49 M, level 6 278 KB and 73 M, level 9 286 KB and 144 M. So the file is 1.6 times what the oracle's is, 23.7 MB against 14.5 MB on SGA3, and a viewer reads the same text out of it.
  - The cost, by `tools/prof/synctex_cost.py`, a PGO build, a warm run as a user has it, the option off and on.  Instructions are of 3 counted runs, of the engine's thread and of all threads.  Cycles, CPU and wall are medians of paired ratios in turn at a load of 40 to 55, 41 rounds and 7 for SGA3, with off against off in brackets.  So the times are ratios, and not of a quiet machine.

    | document | text MB | file MB | Minstr engine, off | on | ratio | all, off | on | ratio | cycles engine | cycles all | CPU |
    |---|---|---|---|---|---|---|---|---|---|---|---|
    | hello | 0.001 | 0.001 | 9.24 | 9.36 | 1.0135 | 9.24 | 9.36 | 1.0135 | 1.017 (0.990) | 1.017 (0.990) | 1.085 (0.968) |
    | the thesis | 1.18 | 0.45 | 824.1 | 828.5 | 1.0053 | 880.0 | 922.3 | 1.0481 | 1.002 (0.986) | 1.043 (0.985) | 1.033 (0.999) |
    | LMB | 1.54 | 0.59 | 816.7 | 825.9 | 1.0112 | 890.7 | 945.3 | 1.0614 | 1.005 (0.994) | 1.045 (0.994) | 1.046 (0.990) |
    | SGA3-English | 64.5 | 23.7 | 32780 | 33473 | 1.0211 | 34680 | 36866 | 1.0630 | 1.016 (1.001) | 1.055 (0.997) | 1.060 (1.008) |

    Over the 16 of the ladder's documents that restore, the engine's thread at the median 1.0022, from 0.875 to 1.0135, and all threads 1.0069, up to 1.036.  Below 1 because a member can start the workers, which then take the PDF's streams too.  The most is a cold `tex.web` in plain TeX, 26 MB of text in a run with no macros to speak of: 1.161 on the engine's thread and 1.378 in all.  A line of the text takes some 280 instructions on the engine's thread, by `tex.web`'s 275 M over its 970,286 lines, and the workers 23 to 32 for each byte.  Against what the plan feared, +34% for the thesis at pdfTeX's cost: that is 4.8% of instructions, and half a percent on the thread a user waits for.
  - The path that writes the records is not trained on: `docs/BUILD.md`, "SyncTeX, not trained on".
- With SyncTeX off a message from `ship_out` is one load and one test, and hello gets 2 calls of `unlink` at the end, as on the oracle, since that is how a stale file goes.  Instructions against the base, plain and PGO: hello cold 46.047 to 46.050 M and 43.478 to 43.485 M, warm 9.936 to 9.937 M and 9.232 to 9.238 M; the thesis warm 1.0002 and 1.0004; SGA3 1.0003 and 1.0010.  The bytes of a run do not change: `tests/synctex/off_same.py`.

## 10. commute

The library crate `commute` computes geometry from the spec in `docs/TIKZCD-SPEC.md` (checked against pdflatex to 0.031 bp by
`tests/cd/model/model.py`). It has no engine dependency.

**Inside the compiler (fast path, not replacement):** `crates/engine/src/native.rs`. `\usepackage{tikz-cd}` keeps loading the real package, because it also provides `\color`, `\definecolor`,
`\includegraphics`, `\setkeys`, `\tikzset`, `\pgfmathparse`, `\foreach`, and reports xcolor, graphicx, keyval, tikz as loaded. When the real `\tikzcd` is about to run, the engine takes over for this one
diagram if it can prove the result is the same; otherwise nothing has happened and real PGF runs. On by default; off with `--no-commute` and `RSTEX_NO_COMMUTE=1`, both part of a checkpoint's key. `tests/gate.sh`'s compare and
integration steps run with it off, so that they keep judging the literal engine byte for byte; its step `native` judges this.

- **How it is reached.** `\tikzcd` and `\endtikzcd` are armed like the checkpoint's control sequences (section 7): `CD_ARMED` on their `eq_type` sends a fetch down `get_next`'s path for `cur_cmd>=outer_call`, and `native_trap`
  puts `cur_cmd` right, so `\show`, `\meaning`, `\let` and `\ifx` see the real macro. Nothing is added to `get_next`, `macro_call`, `expand` or `main_control`. An assignment replaces the word and so disarms it, and a group's end brings the armed word back. The path acts only at `main_control`'s big switch. `\tikzcd` is armed when `start_input` has opened the library's file and the macro has come into being, through the `ck.pending` test at the tail of `prefixed_command`; a document without tikz-cd has one test per file opened. No name is interned for any of this: `find_cs` looks up without creating, and the step token below lives at a location with no name.
- **Reading ahead without a trace.** `native_read` goes down the input stack, token lists and parameters by `toks` and files by their bytes in memory, tokenising as §363 does and changing nothing: not `loc`, `line`, `buffer` or the string pool. So a decline has nothing to undo, and tokens need no pushing back, with their catcodes as they were read. A name the hash table does not hold is a decline. The read stops at `\end{tikzcd}` or `\endtikzcd`, at a blank line or `\par`, at an `\end` of something else, at another `\tikzcd`, at the end of a file or after 65536 tokens. A failed read that got into a file remembers the place, so a loop of declines does not read on from there again; each read otherwise stops at the next diagram, so the document stays linear. `&` of category 4 from a token list is a decline, since real PGF stops at it with an error there.
- **Recognising the real thing.** `crates/engine/src/native_table.rs` is generated by `tests/cd/native/gen_table.py` from runs with the path off, under `--features commute-probe`, whose `native_probe.rs` says which macros a real diagram calls and which keys it looks up. `GUARD` holds about 1950 of them, of PGF, TikZ and tikz-cd, with a hash of name, kind and tokens by what they are and not where they lie: those the same under eight different preambles and at the end of a document as at its start, what those mention in turn, since `\let\tikz@next=\tikz@do@fig` is a call no probe sees by name, a few switches, the 19 macros of LaTeX's that they run, such as `\selectfont` and `\leavevmode`, and the 80 or so primitives they name, as `\relax` and `\hbox`, by what each does. `GUARD_ANY` holds those that libraries define again, which may be as any preamble has them. `ABSENT` holds keys tikz-cd tries that a plain preamble does not define, like `every text node part`; `row N`, `column N` and `N-row diagram` are asked for by number. The list of what was checked is a block in `toks`, so a checkpoint carries it and a restored run does not check again. Each entry holds a reference to the macro's body, so the same place proves the same tokens; a diagram compares places, about 15000 instructions in all, and hashes only what moved. Another version of PGF or tikz-cd fails hundreds of hashes, and every diagram falls back.
- **What a document may have changed.** `GUARD` holds, besides what a diagram calls, what any key of `/tikz` and of tikz-cd's directory writes outside a picture, as `gen_table.py` finds by running each with a few values: a diagram reads some of that only by `\ifx` or `\let`, and so never calls it. `REGISTERS` holds the registers among it, such as what `shorten >` writes. `every scope` may hold what the library `intersections` and `spath3` append to it. A hook on `\selectfont` must be one that does nothing it matters to do more than once; `env/tikzcd/end`, which real TeX runs in the last cell, must be empty; and `\everyvbox`, `\globaldefs` and `\boxmaxdepth` as LaTeX has them, and letters letters, since real PGF's own boxes and the font descriptions it reads go by them. A hash that differs is a decline, unless the library can be told of it as a key list before the diagram: `every arrow/.append style` and the others of that family, recognised as the installed style and then more, which also covers `arrows=` and the like; `row sep/<name>`, `column sep/<name>`, `background color`, `crossing over clearance`; `math mode`; `arrow style=Latin Modern` or `tikz`, by the tips each leaves. `>` is told as `>=stealth`, `>=latex` or something else, and a tip of tikz-cd's that the document made another as `.tip`, and the library then declines only an arrow that draws them. `every loop` declines only a diagram with a loop. Since the state is read from `eqtb` each time, groups need no more than that.
- **The document's keys.** A key that a diagram uses is asked of `DocKeys`, the library's `UserKeys`, as it is reached. Its `.@cmd` in tikz-cd's directory or in `/tikz`, in that order and with what is only `r d l u` left to tikz-cd's `.unknown`, by hash against `KEYS`, the stock keys under each of `STABLE`'s preambles: if it is one of them, the library knows what it means or declines. Otherwise the library gets its replacement text, and reads a style, with `#1` and `.default`, as `\tikzset` would; a key defined by code it knows only as quiver.sty writes it, and quiver's `curve` only while `/tikz/quiver/pos` and `height` are as that package sets them. A name of colour is what `\definecolor` left in `\\color@<name>`, base colours included; a mix is worked out by the library only while all base colours and xcolor's target model are as installed.
- **Text a style made.** A style's text lies in the library's buffer past what the engine gave it, with `#1` put in, so no token stands behind it. `native_write_cs` and `native_style_text` write bytes that say what each token was: a control word as it is, any other control sequence or a character in a category no one would guess as an opaque word with its number. `native_made` reads such a span back to those tokens, and from there it is vetted and typeset as any other.
- **What may stay in a cell** is judged after the library has parsed, on the tokens it left there and in labels, by real meanings (`native_vet`). The library parses bytes, and `pos` leads from each byte back to its token. It has taken each `\arrow`, `&` and `\\` for what it looks like, and TeX may not: so what is judged is what could make them something else. A cell is judged run by run, a run being the text between arrows. Each run has to balance by itself: conditionals, with the braces of each branch, since a branch TeX skips would hide an arrow or a `&`; `\begingroup`, `\bgroup`, `\left`, `$` and environments. `\aftergroup` needs a group of the run. Directly in a cell no `\let`, `\def`, `\expandafter`, `\noexpand`, `\string`, `\meaning` or `\afterassignment`, which decide how the next token is read. What ends a run wants nothing more: not `^`, `_` or a character constant, not `\kern` or `\global` or a `\raise` whose box has not come, and a macro's arguments must be found in the run, its own and what its body starts and ends with, by §397's matching, with a budget (`native_args_fit`). A macro of the document that comes to `\futurelet` must not end a run. The runs of a cell are typeset with a token between them, where the arrow stood, `CD_RUNS`, which does what `\relax` does, so a number, a keyword or an optional argument does not run on into the next, and which `\ifx` tells from `\relax`, as it tells `\arrow`. Text on both sides of an arrow without math mode declines. `\csname` with a name that can be read is one more mention, and any other is beyond proof; `\the` is of numbers only, since a token register can hold anything. `\begin` must have a name; `\\` and `&` are an environment's own only for the alignments of LaTeX and amsmath as installed (`ENV_TABS`, `ENV_LINES`), and `\substack` and `\shortstack` only as installed; `&` in a label is not, since the matrix has ended by then. Displays decline. `$`, and an `\halign` or `\valign` in a macro, want a box of their own: in a formula they are errors from which TeX recovers by the groups it finds, and those are not real PGF's. So they stand only in the last argument of what puts text in a box, `\text`, `\mbox`, `\rotatebox` and their like as installed, in a primitive's box, or for `$` in the `\else` of `\ifmmode`. `\(` and `\)`, `\[` and `\]` decline in a label as the library declines them in a cell. `\errmessage` directly in a cell declines, since real tikz-cd adds to what it says in a label; `\openout` and `\font` read a file's name on through the `}` of a label whose last blank the library has taken off.
  In a diagram with `math mode=false` a blank that ends a run or a label declines unless a letter, another character or `$` stands before it: real TeX has that blank, which its `\unskip` takes, and the path's `\unskip` would take an earlier one. So does what only a formula has, directly, and any macro but those at home in text (`native_keeps_text`), since what it comes to is not read.
  A macro's body is walked, once per place in `toks`, of which the memo holds a reference so that a redefinition cannot land there, and again for each diagram as far as what it mentions goes, primitives included, as a `\let` can change those with the body as it is, within a budget, by the same rules: `\arrow`, `\\`, `&` and `\cr` outside an alignment of its own, `\catcode`, `\endlinechar`, `\input`, `\scantokens`, `\read`, `\shipout`, the `\show` family, a case change, a mark, a `\csname` whose name cannot be read there, anything of PGF's or TikZ's, and anything undefined, so that its error message comes from real PGF. What is only named does not count, unless it is something real tikz-cd has its way with and a macro that compares it would see something else there: parameter text, the operands of `\ifx`, the target of `\let` or `\def` if that is no primitive and not PGF's, the control sequence in a character constant. What is an error in a formula, `\par`, `\vskip`, `\hrule`, `\unhbox`, `\eqno`, declines, since real PGF's cell says so in other words; so do primitives that depend on the place on the page, `\pdfsavepos` and the link and annotation family; `\inputlineno`, `\badness`, `\currentgrouplevel` and their like, which are other numbers in a real cell; and what a cell would change for the rest of the diagram that the path has read already: `\fontdimen`, a `\global` assignment to a parameter, the `\every`s, tracing, `\pdf...` parameters, and the definition of a colour. `SAFE`, in the same table, holds some 6500 macros of LaTeX and the common math packages by the hash of name and body, without hyperref: those may stay whatever they do. A redefinition has another hash, and is walked. `KNOWN` holds the hashes of some 43000 macros, all of those preambles but PGF's and scratch: a macro of `KNOWN` is looked at only for what it hides of the structure, since it is LaTeX's or a package's and `SAFE` lacks it only for its name. What a macro of `SAFE` mentions is looked at to the end (`native_closure`): one that `KNOWN` does not have may hold nothing of the structure, wherever a document has renewed `\bfseries`. All that was found so stays so as long as each body stands where it did, which costs a comparison for each and no walk. A primitive's name is checked the same way, for all of them: `\let\over\arrow` shows wherever `\frac` is. An undefined name in a macro's body counts if it has no `@`: graphics' `\Gscale@x` and `\@nil` are defined when needed or only delimit, and a misspelt name in a document's macro has no `@`; `ifthen`'s `\equal` and its like are listed. expl3's own names are not walked if `KNOWN` has them, but a verbatim argument of `\NewDocumentCommand` declines; what a document has made with such a name is walked. Active characters are walked the same way, and so are characters with `\mathcode"8000`, all of them for every diagram, since one can come by `\char`, by `\chardef` or through a macro. An active `!` declines: a picture keeps it to itself, and real cells do not see its meaning.
- **What a cell starts with.** At the start of each cell of an `\halign`, §785, real TeX expands before the template: outside the cell's group and its math, where `\baselineskip` is what PGF has made it and `\ifmmode` is false. A macro that comes to nothing there makes an empty cell, with no node, `\the` of anything is another number there, and `|` from a macro starts a node's options. So what a cell starts with has to come to something TeX will not expand, by a look at the macros' bodies (`native_cell_start`), or the diagram declines.
- **Cells and labels are typeset by real TeX**, by the main loop, in the order real tikz-cd has them, once each: `\begingroup <font> $ { \aftergroup\unskip \ignorespaces <tokens> } $ \endgroup` in an `\hbox`, what `\tikz@do@fig` and `\tikz@fig@boxdone` leave of a node's text; labels get `every label`'s `\everymath\expandafter{\the\everymath\scriptstyle}`. `CD_STEP`, a location in the part of eqtb that the primitives' table does not reach, ends each of these lists, and its fetch brings control back with the Rust stack as `main_control` had it, so no recursion is needed and a real `\end{tikzcd}` follows, with its hooks and `\@checkend`. The text of a cell that lies on several lines of the file is several input levels, one per line, of a type of their own, `CD_LINE_TEXT`, whose end `end_token_list` tells `native_line_ended`, on a branch only such lists and those of `\every...` reach: then `line` goes as with the file, for `\inputlineno`, for what LaTeX says "on input line", and for overfull boxes, and a macro reading its argument across the lines sees no token between them. Labels wait in real PGF until the diagram's end, and so have the line of `\end{tikzcd}`. Its fetch of `\endtikzcd` turns into `\relax`. `\selectfont` and `$\relax$`, from which `rule_thickness` and `axis_height` are read, run once for the diagram, in the group of the hbox that holds the boxes as they are done. The library parses again only if that changed the font parameters.
- **What a real diagram changes**, established by `native_probe.rs`'s dump of eqtb and all registers around one diagram, which `\message{rstexprobe NAME}` writes, and by the oracle's `\tracingassigns`, whose `globally changing` lines name all that outlives the diagram's group. In registers and macros a later diagram or a document can see: `\pgf@picture@serial@count`, which `.aux` entries of `remember picture` carry, `\tikz@fig@count`, `\tikzcdmatrixname` and the three counters of the matrix; `\pgf@x` and `\pgf@y`, which end at 0; `\pgf@picminx` and its three like, the box's own; `\pgf@pathminx` and its three like, reset to 16000pt; `\pgflinewidth`, 0.4pt whatever the arrows had; `\font@name`, for which the last box runs `\selectfont`. The path leaves those the same, `tests/cd/native/state/s01`, `s87`, `s88`. Math fonts through `$`. What else changes is scratch that each real diagram sets before reading: the list under "what stays different". A cell sees what the document has set, except `\spaceskip`, `\xspaceskip`, `\tabskip`, `\everycr` and, if it changes anything, `\normalbaselines`, which are set in their groups as real PGF sets them. The picture's `\leavevmode` is real, and its box has the same shape: height all above the baseline, depth 0, raised by a negative amount, so `\lastbox` gets what it would. Inside it, a `\hskip` and then a box that ends in `\hss`, as real PGF has it, since pdfTeX's search for a character to protrude goes into boxes and past empty boxes, kerns and whatsits, and would otherwise reach the first or last character of a cell (`s82`, `s83`). `\everymath` may only choose a style, `\everyhbox` and `\everyvbox` must be empty and `\mathsurround` zero, since real PGF's many `$\relax$`, `\hbox` and its `\vbox` would run them more often than a cell does; `\globaldefs` must be zero, and so must `\TeXXeTstate`, since the picture's boxes nest otherwise and the order of what is written follows that; `\escapechar` must be a backslash, for real PGF's `\string`, and `\boxmaxdepth` have no limit, for its matrix. The primitives real PGF runs, by the oracle's `\tracingcommands`, must mean what they do in INITEX (`PRIMITIVES`, compared by place), and `\frozen@everymath`, `\check@mathfonts`, `\selectfont` and `\normalbaselines`, which real PGF runs more often than the path, must be LaTeX's, and the hook `selectfont` may only hold what leaves no trace in a group, nothing global, nothing written, as scrartcl's `\par@update`; a hook `env/tikzcd/end` runs in real PGF's last cell, before all labels, and declines too. In a cell, what reads how deep it lies declines: `\currentgrouplevel`, `\currentgrouptype`, `\currentiflevel`, `\currentiftype`, `\currentifbranch`, and `\badness`, which has real PGF's boxes before it; so does what sets `\globaldefs`, `\everyhbox`, `\everyvbox` or `\everymath`, and `\mathversion`, `\boldmath` and `\unboldmath` outside a group of the cell, whose `\aftergroup` TikZ's parser would find. `\halign` expands what starts a cell in its search for `\omit`, so in real PGF those tokens run before the cell has begun, not yet in math mode and under `\nullfont`: a cell that gets to `\ifmmode`, `\ifinner`, `\ifhmode`, `\ifvmode`, `\iffontchar`, `\fontname` or the like there declines (`native_vet_start`), which `|[...]|` before the text prevents, as `|` stops that search. What a macro before an arrow peeks at is the arrow in real PGF, and here `CD_RUNS`, and it finds neither to be what it looks for (`s69`, `s122`); what it takes as an argument is `native_args_fit`'s to find. An arrow from a node to itself with no angle to leave by declines, since only real PGF writes its two warnings, whichever two nodes share a centre. For the same reason a math version that has used up its math groups declines, as LaTeX then writes to the log after every formula; and so does a diagram that may bring that about itself, by what its cells reach of the alphabets not yet used, if four groups or fewer are left and all those alphabets would not fit. With more left, a diagram that used more than four alphabets for the first time, all at that point, would show in the number of those lines of the log. Page resources are the same, as PGF puts them on every page at load, except `opacity`, which declines.
- **The checkpoint.** Everything but workspaces and memos is POD: `\tikzcd` is armed in the image, the guards are there, and a restored run takes the path as a cold one does. The checkpoint refuses to be taken in the middle of a diagram. It is taken where `\document` comes to `\@input` (section 7), so a diagram in the preamble or in the hook `begindocument/before` lies behind it, with its boxes in the image; one that starts on the line of `\begin{document}` is read from `buffer`, in the image, and then from the open file. What decides the path is in the key by being in `Options`: its two switches, and `--commute-report`, which writes to the log. A page record is put off in the middle of a diagram. The memos are no part of the image and the references they hold to macros are, so a record says which they are, with `native_closure`'s list, which says which of them are looked at again when they move, when that has changed (`native_held`); or a run resumed from it would take them again at its next walk, and its state would be another by those counts. What was found in those bodies a resumed run finds again as it comes to them. A format dumped with tikz-cd in it, `mylatex` style, has nothing armed, since `-ini` turns the path off and its file is not opened again: such a run has real PGF for every diagram, and is otherwise right.
- **Starting again.** What cannot be declined after the fact is done over: `native_restart` has the driver run the job from the start with the path off, in this process, on a new image. Nothing of the first run counts, so that run's log and PDF are those of `--no-commute` to the byte. It restores the files the job has opened for writing since `\tikzcd` was armed, `.aux` and `.toc` above all, since the second run reads them, and removes those that were not there; so does a run from a checkpoint or a page record, for those it makes again from what it holds, which are as the job found them just before that (`pages.py restart`); the first bytes of the second run's terminal output, which the first run has given already, are left out. Under `--commute-report` the log's end says `[[commute restart]]` and why. What sets it off, none of which a document without an error or a debugging aid meets:
  - an error in a cell or label, from `error()` and so also a stop, in all interaction modes and under `-halt-on-error`. A real run gives it with other context, the file's line as far as it has been read and PGF's macros, and may go on otherwise, since its recovery works on another list;
  - tracing in a cell by what `native_vet` does not see, such as a `\tracingall` of `SAFE`, on its way through `begin_diagnostic`;
  - what should have been the end of a diagram going otherwise, `native_failed`;
  - a diagram of more than 4000pt, seen once its boxes are measured, for which real PGF's sums can come to `Dimension too large`;
  - after a diagram went by here, `\showbox` and `\showlists` with `\showboxdepth` above 0, and `\tracingoutput` at a shipout: the list inside the picture is not real PGF's;
  - after a diagram went by here, PGF's error `No shape named`, from a picture that asks for a node. The table of nodes is real PGF's, and the node may be one of that diagram's, by `alias=`, `name=` or `\tikzcdmatrixname`.

  A run that has run a command, `\write18` or a pipe, or read from the terminal, cannot be done over and stays on the path as it is: an error's context then differs. A document that errs in a cell pays for the first run up to that point and then for real PGF. A file opened for writing before tikz-cd was loaded is not restored, so a document that reads its own old `\jobname.xyz` after opening it before loading the package would read an empty one in the second run. A session of the `commute` command has no second run: what goes wrong in a cell is what its user has to hear of.
- **What stays different with the path on, by nature.** All that a document can observe otherwise is the oracle's, by `tests/cd/native/state`:
  - In the log, the figures of its statistics: strings, string characters, words of memory and multiletter control sequences, since real PGF makes names and strings that the path does not; the stack positions, each at most real PGF's; the number of bytes on the line `Output written`, which is the compressor's in any case. Font info, hyphenation exceptions, the font files, the page count and all of `PDF statistics` are the oracle's. Those are what `compare_native.py documents` and `tests/cd/native/run.py` mask, and nothing else; `compare_native.py statistics` says how.
  - In the PDF, a font's number in the page's resources where real PGF has loaded a font earlier than the path does, and what follows from that in the compressed bytes. `compare_native.py streams` allows for it. Over `tests/cd/native` there is no such case: each case's PDF has the same bytes with the path on and off.
  - PGF's scratch, which a real diagram or picture writes before it reads: `\pgf@ya`, `\pgf@path@lastx` and `\pgf@path@lasty`; the conditionals `\ifpgfmathunitsdeclared` and `\ifpgfmathmathunitsdeclared`; the macros `\pgfsyssoftpath@*`, `\pgfsysprotocol@currentprotocol`, `\pgf@strokecolor@global`, `\pgf@fillcolor@global`, `\pgf@shift@baseline`, `\pgf@trimleft@final`, `\pgf@trimright@final`, `\pgf@layerlist@`, `\pgf@matrix@*`, `\pgf@tempaa` and its like, `\pgfkeys@global@temp`, `\tikz@last@fig@name`, `\tikz@*@smuggle`, `\tikz@computed@*`, `\tikz@curve@*`, `\tikz@dcl@coord@*`, `\tikz@marshal`, `\tikz@tangent@temp`, `\tikzcd@savedpaths`; the cache of arrow tips, `\pgf@ar@*`, `\pgf@arrow@*` and `\pgf@@ar@*`; the table of nodes, `\pgf@sh@*`. `\ifcsname` of such a name, or `\meaning`, shows it, and so would a macro of a document that reads them. `\pgfgetlastxy` at the start of a later picture reads the two registers, which no real use does, since it is for a path under construction.
  - `\pgf@sh@*` by a test rather than by a picture: `\ifcsname pgf@sh@ns@K\endcsname` for a node of a diagram on the path. A picture that asks gets the error, and that starts again.
  - The inside of the picture's box, by `\unhbox` and `\lastbox` at the second level. The diagram's box holds a glue and a box with the same dimensions and shift as real PGF's, and `\lastbox` and `\unskip` find them; the list of that inner box has fewer levels than real PGF's, and in it the pieces are other nodes. Nothing but a document's taking the picture apart can see that, `\leftmarginkern` and `\rightmarginkern` do not.
  - In a cell, a SAFE macro given a PGF name as its argument, such as `\@nameuse{tikzcdmatrixname}`: the argument is text to `native_vet`. Likewise a macro of `SAFE` whose inner macro the document has changed, as the walk stops at `SAFE`, and `\let` to a name of the document of one of PGF's registers, such as `\pgfmatrixcurrentrow`.
  - A document that changes a macro of LaTeX or xcolor that real PGF calls in passing and that `native_table.rs` does not hold, such as `\strip@pt` or `\XC@getmod`, so that it does something visible in that call. `GUARD` holds PGF's, TikZ's and tikz-cd's own.
  - A macro in a cell that peeks with `\futurelet` at what follows its cell's last token: real PGF has `&` or `\\` there, the path `}`. `\xspace` and `\@ifnextchar` come to the same but for a test written for just those.
  - Time, of course: `\pdfelapsedtime`, and after a start again the clock of `\time`.
- **Gated** by `tests/corpus/cd/compare_native.py`: `geometry` on the 3246 extracted diagrams: box dimensions exactly, the text in order, and pixels at 400 dpi, where 0.05 bp is enough to fail, which `check-negative` proves. `streams` on those that take the path: the content stream byte for byte. `documents` judges whole documents, the user's and the arXiv papers that use tikz-cd: the whole log, masking only the figures listed above, the `.aux`, the page count, the text, pixels and content stream of every page, and `qpdf --check`. `tests/cd/native` holds cases for each of the requirements, must-take and must-decline, against the oracle, and runs tests/cd/28 to 31 of the library through the engine; each case's log is compared as `documents` compares one, and its exit status. `tests/cd/native/state` holds what a document can see after a diagram or in a cell: registers, macros, modes, every parameter of TeX, e-TeX and pdfTeX set around a diagram (`p01` to `p04`), side effects in cells, real pictures around, typesetting twice, moving arguments, errors under each interaction mode, `.aux` kept across a restart. `tests/cd/native/adversarial` holds some 8500 documents written to break it, by hiding what the library reads, through every kind of argument, box and environment, by every key, every macro of PGF's that a diagram calls and every register it mentions set beforehand, by each primitive of the engine and some 450 forms of LaTeX in a cell, a macro and a label, and by document classes, packages and libraries, with 26 characters made active through babel; any fate is allowed, and what takes the path is judged against the oracle. `cost.py` beside them makes the check slow on purpose, and fails if it grows by more than the document. All of these pass with every diagram declined, as they did when a wrong `\toks` in `GUARD` sent every diagram to real PGF. So `tests/corpus/cd/native_floor.json` says how many take the path, per source in `geometry` and in `documents` by the first part of the id and for the thesis and LMB by name, the step fails below that, and `compare_native.py floor-check`, with `--no-commute`, shows first that it can. `tests/corpus/snapshot/warm_equals_cold.py --path-on` holds the ladder and the user's documents to the same from a checkpoint and from page records: cold, written, restored, sealed and resumed runs equal byte for byte, logs included, and the state of a resumed run the writer's.

**SyncTeX with the path on.** The path has no matrix and no scaffolding boxes, so SyncTeX's text differs inside a diagram, and it is an exception of the same kind as the log's memory figures; `--no-commute` has the oracle's text. A cell's own records have the oracle's places, tags and lines. Over `tests/synctex/documents.py`'s table of positions, against the oracle's file: the thesis 133 of 144 answers the same in every byte and 138 the same line or page, LMB 136 and 140, `ladder/14-tikzcd-one` 44 and 48 of 63, `ladder/15-tikzcd-many`, all diagrams, 42 and 92 of 108. A document with no diagram has the oracle's text with the path on: SGA1-VI, summary and 18 of the ladder's 20 by the same tool.

**The picture's list.** `commute::pdf` tells the engine's sink each position pdfTeX reached in real PGF, in scaled points from the picture's origin. Each is an hbox of no width at that place, so pdfTeX writes the same `cm` as for real PGF, with the same rounding from the same page position. The path's literals are ordinary `\pdfliteral` whatsits holding token lists, so `\copy`, `\unhbox` and `flush_node_list` need no new case. A colour by itself is real `\pdfcolorstack` nodes, on the stack `\@pdfcolorstack` says, and so is the `\color` that a coloured node has inside its box, since what a `\textcolor` in a cell comes back to is on that stack. `\pgf@layerlist` gives the empty `q Q` of the layers. So the content stream of each page is the oracle's byte for byte, but for a font's number in the page's resources: every diagram on the path has that, by `compare_native.py streams`, and so does every page of the documents.

**What was not done.** A cell with a link or `\pdfsavepos`, though TeX now stands at the right place for them: nothing has compared them. `opacity`, which needs PGF's page resource. Diagrams with arrow tips outside the three styles, `>={Latex}` among them, and keys outside the library's subset, go to real PGF.

**What is known to differ.** `tests/cd/native/adversarial` marks 21 documents as known, and each fails if it stops differing. All are of the kinds under "what stays different". A later part of a document that tests the table of nodes, as `\ifcsname pgf@sh@ns@q\endcsname` does, where one that asks a picture for the node starts again. PGF's scratch read raw after a diagram, `\pgf@ya` and the like; `\pgf@x` and `\pgf@y` are as a real diagram leaves them. And before a diagram, a document that gives another meaning to scratch of PGF's that real PGF reads as it stands, like `\pgfutil@tempswatrue`, which the guards cannot hold since each key or call leaves something else there, or to what the corpus shows libraries define again, which `gen_table.py` leaves out of `GUARD` by name.

**Standalone `commute` CLI**, `crates/commute-cli` and `crates/engine/src/session.rs`; `docs/COMMUTE.md` is for the user. It reads a file of diagrams, a diagram's body or a whole document, and writes for each diagram a PDF whose page is the picture's box and an SVG with glyph outlines from the PFB. Cells and labels go through the main loop as on the native path, and the same library draws the rest.

- **What starts a run.** `rstex_driver::run`, in `crates/rstex/src/lib.rs`: the whole of `rstex` with its options, format auto-build and checkpoint driver, given a `session::Config`. The `commute` binary has all of it inside, and `--engine` as its first argument makes it `rstex`, which is what a format's build runs, so no other program has to be installed. `\commutesession`, made and armed when the format has loaded, as `\tikzcd` is, brings control to `session_step` with the Rust stack as `main_control` has it. Each diagram is a file in memory on the input stack, holding `\tikzcd`, the diagram and `\commutesession`, and `native_read` reads it as it reads any file.
- **With the library only**, the usual way. `commute-HASH.fmt`, built once by `autofmt::derived` on `pdflatex.fmt` from the text in `format.rs`: `article`, `amsmath`, `amssymb`, `graphicx`, `xcolor`, a `\tikzcd` that holds nothing, and `\dump` after `\begin{document}`, with `\everyjob` emptied and the math fonts at 10pt in place. Its list of files holds what `pdflatex.fmt` read as well, and that list is gone through once a minute, as its 200 `stat`s are a tenth of a short run: for that long a change to the tree can go unseen. `native_guards` does not run, as there is no real tikz-cd to be as installed, and neither do `native_primitives` and `native_closure`, as all those macros are the format's and there is no real PGF to be equal to; they would be a third of a run of one diagram. All of a file's diagrams go through one engine. After each PDF, `main_finish_pdf`, which is §794, and `session_new_pdf` put back what §620, §681 and §697 initialise and forget each font's use; a map entry and an encoding's names stay. So a global assignment in a cell reaches the next diagram, and that is the only way a diagram can tell: `tests/cd/compare_cli.py --files` compares each PDF and SVG with what it gets alone, up to a font's number in its name, which depends on what was loaded before. A decline says why and goes on to the next, since nothing has happened yet. If TeX is left where no diagram can begin, such as `\vskip` in a math cell, another run takes the rest of the file.
- **With real tikz-cd**, under `--preamble`, `--usepackage` or a document's own preamble. A driver file in the cache directory, named by the hash of its text, has the class, the packages, tikz-cd and the preamble, and the checkpoint of section 7 does the rest. Each diagram is a run of its own that restores it: what a package reserves in the PDF at load, as PGF does, is the run's to finish, and that takes a real `\shipout` and a real `\end{document}`. A decline runs real PGF, as in a document. `--real-pgf` takes this way for what the library declined in a file without a preamble.
- **The page.** `\pdfpagewidth` and `\pdfpageheight` are the box's, and `\hoffset` and `\voffset` are one scaled point more than the origin, since `standalone`'s `-72.27pt` is rounded where `1 true in` is truncated. So the content stream reads as `standalone`'s but for its colour stacks' lines at the page's start and end, and a line next to a pixel's edge stays on the same side of it. The PDF holds no date and no `/ID`, so the same input gives the same bytes on each run.
- **The SVG.** `commute::svg` draws the strokes from the draw list. `session_hlist` and `session_vlist` walk the boxes as `pdf_hlist_out` and `pdf_vlist_out` do, in scaled points with the same rounding of glue and the same leaders, and `T1Outlines` (`crates/fontfile/src/outline.rs`) gives each character's path. Colours come from the `\pdfcolorstack` nodes, and `\rotatebox` and the like from `\pdfsetmatrix`. `width` and `height` are the real size, in what SVG calls `pt` and PDF a unit; the `viewBox` is in TeX points, as the library's numbers are. A font that is not Type 1, an image or a `\pdfliteral` means no SVG, which is said. Only `+ - * /`, so the same bytes on every platform.
- **What made it fast.** On one diagram of three fonts, from 15.9 M to 7.1 M instructions, and from about 11 ms to about 8 ms of CPU at a load of 20 to 45. All sections of the format are mapped, not only those of `fmt_mapped`: a run that stops after `\begin{document}` touches few pages, and reading `toks`, `eqtb` and `hash_tab` took 3 ms. Font programs are stored, not deflated, as eexec data does not compress, about 1 KB in 20 and 1.5 M instructions; the rest has a deflate state its own size, since half a megabyte of zeroed `Deflater` was most of a file's page faults. `\pdfgentounicode=0` and `\pdfobjcompresslevel=0` in the format. `Memo`s by control sequence in pages, not one vector of tens of thousands, 1 M instructions and 300 faults. `t1_getline` looks at a line's last byte before it compares two suffixes, which `rstex` gets too. No search for a `.vf` of a font that the map knows, and none through the current directory for a font under another case. The heap's first megabyte in one `MADV_POPULATE_WRITE`.
- **Compared** by `tests/cd/compare_cli.py`: section 16 has what it says.

**What was not done.** An SVG of what real PGF draws, which would need an interpreter of its literals. A checkpoint past `\begin{document}` under a preamble, where most of a restored run's time goes. Sharing the eexec decryption between `write_t1` and `T1Outlines`, or keeping a font's outlines in the cache directory, about 1 M instructions a run. A subset kept per font and glyph set, for a file of many.

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
| Giving a region to a system call | `read`, or a private mapping of the file | `read`, or a private mapping of the file | `platform::commit_range` first, since `ReadFile` into uncommitted pages, and `WriteFile` from them, fail rather than fault; never a mapping (top of `platform/windows.rs`) |
| Static C runtime | `+crt-static` | default (cannot be static) | `+crt-static` |
| Local time for `\time`, `/CreationDate` | `localtime_r` | `localtime_r` | `SystemTimeToTzSpecificLocalTime`, which does not read `TZ`, as msvcrt's `localtime` would |
| `\write18` | `sh -c` | `sh -c` | `%COMSPEC% /c`, the command as it stands |
| A full engine stack | one line and status 1, from a handler on a static signal stack | the same | the same, from the vectored handler |
| Control-C | `signal`, which sets TeX's `interrupt` | the same | `SetConsoleCtrlHandler` |
| Terminal | the bytes | the bytes | the bytes when redirected; at a console valid UTF-8, since `std` refuses a whole buffer otherwise |
| Cache directory, mode 0700 where there are modes; section 7 says who may write it | `$XDG_CACHE_HOME/rstex` or `~/.cache/rstex` | `~/Library/Caches/rstex` | `%LOCALAPPDATA%\\rstex\\cache` |
| Publish a cache file | tmp + rename | tmp + rename | tmp + rename with retry, since a reader may hold it open |
| File identity for validation | size, mtime, inode | size, mtime, inode | size, mtime |

Everything else is built on `std`:

- **File names.** TeX sees bytes. The World converts at the boundary: bytes on Unix, on Windows WTF-8 to wide strings, by plain functions (`names::bytes_to_wide`), so that any name the system hands out comes back, and a byte that is no UTF-8 is U+FFFD, as TeX Live has it under its default `command_line_encoding = utf-8`. It accepts both `/` and `\\`
  plus drive letters on Windows. Names printed in the log are what TeX was given or found, so logs may differ in paths; PDFs may not. A name found through `ls-R` on Windows comes out in lower case, where kpathsea keeps the directory's case and the name as it was asked for: only the log shows it.
- **Text.** Input is binary-read; as in web2c, `input_ln` strips trailing spaces, `\r` and `\n`, so CRLF sources work everywhere. Output is `\n` on every
  platform, for byte-identical results.
- **Input files are read, not mapped.** Windows locks mapped files, which would break editors saving during a compile.
- **Finding the TeX tree.** No hardcoded `/usr/share`. Order: `RSTEX_TEXMF` env; else ask `kpsewhich` once (TeX Live, MacTeX) or `initexmf --report` (MiKTeX),
  and cache the answer; else well-known locations. These programs are used only to locate data, never to compile. Our own `texmf.cnf` reader follows kpathsea's
  variable and search semantics (`SELFAUTOLOC` and friends, `;` as separator on Windows). MiKTeX has no `ls-R`, so the index comes from walking the trees, and each run checks the stamp of every directory it walked. Lookups in the
  tree index compare in one case throughout on Windows, as kpathsea's `MONOCASE_FILENAMES`, and fall back to case-insensitive matching on macOS, by a table in the index file, so that the miss every LaTeX run has costs one look. MiKTeX's install-on-demand is not triggered; a missing file is a clean error naming the file.
- **Deep recursion.** The engine's 1 GiB stack is requested through `std::thread::Builder`, or mapped for `swapcontext` under glibc; only reserved memory, on all three. Under a limit on the address space it takes a quarter of that, down to 64 MiB, and the image, which cannot shrink, stops the run with a message.
- **Output that fails.** As stdio has it. A failed write to a log or an `\\openout` stream says `fclose: ...` when it closes, and the job goes on. A PDF is written at the end, by a temporary and a rename where that leaves what `fopen` leaves, and in place otherwise: a symbolic link, a second name, or a directory that takes no new file. It keeps its mode. If that fails it is `pdftex_fail`, status 1, and "Output written" does not come first. If the rename and the write both fail and the temporary is whole, as with a PDF that a Windows viewer holds, the message names it. A format that cannot be written stops with status 1, and is not published.
- **The tables of `texmf.cnf`, `ls-R` and `pdftex.map`** stay in the cache directory, `crates/kpse/src/index.rs` and `cachefile.rs`: valid while each environment variable and each path looked at, there or not, has the answer it had, and checked by sums as far as they are read. Mapped on Unix, read on Windows, where a mapping would hold the file against the rename that replaces it.
- **Formats** are built when needed, once, in the cache directory, `crates/rstex/src/autofmt.rs`, keyed on the build-id, the tree and what the environment says of it, and valid while everything the build read has the stamp it had. The build goes without the user's own trees, as `fmtutil-sys` does, or a package in one document's `TEXMFHOME` would stay for all. TeX Live's gzip formats are no use to this engine. With no cache that can be written, a format is built for the run alone, in a new directory of mode 0700 beside its outputs, and removed once the run holds it: a document compiles whatever state the cache is in.
- **The performance targets of section 2 are measured on Linux.** Inline-array displacement addressing is x86-64 specific; on arm64 LLVM materialises the offset, which costs a little.
  PGO is per target, so it is done in release builds per OS.

**Gates.** From M0 on, `cargo check --workspace --target` must pass for `x86_64-unknown-linux-gnu`, `aarch64-apple-darwin` and `x86_64-pc-windows-msvc` (all installed
locally; no linker or C toolchain required). That only proves it compiles. Real runs need real machines, so GitHub Actions runs a matrix of `ubuntu`, `macos`, `windows` with TeX Live: unit tests, the
format build, the smoke tier of the corpus, and a **cross-OS check that the PDFs have the same hash**. On a private repository on GitHub Free the included usage is $12 a month, at $0.006/min for Linux, $0.010/min for Windows and $0.062/min for macOS (about 2000, 1200 or 190 minutes); standard runners are free on public repositories. So Linux runs on every push;
macOS and Windows run on `main`, on tags and on manual dispatch. `.github/workflows/ci.yml` is that, and `docs/BUILD.md`, "What CI has to settle", says what its first runs have to answer.

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
crates/pdfio       deflate and its workers, chunk writer, MD5
crates/image       JPEG, PNG, PDF-inclusion reader
crates/commute     diagram geometry (no engine dependency)
crates/rstex       `rstex` CLI, and all of it as a library for commute: pdflatex-compatible flags, format auto-build, the snapshot driver, --until-stable
crates/commute-cli `commute` CLI, on `crates/engine/src/session.rs`
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

## 16. Where it stands

**Now.** M5's merge, and on it the branches that took speed further on small documents and on long ones, and the reviews of the token machine, the checkpoint and the other systems, and the native diagram path, with the `commute` command and two reviews of the path, and then the five tracks of the last round: the output cache, the macro lever, `--until-stable`, SyncTeX and page records, and all of it merged on dev's tree. Those have a part of their own below, measured on a loaded machine and labelled so, with the last merge at its end; the figures in milliseconds of this section are all from before them, and new quiet ones will come when a release is built. The next tables are from before the native diagram path was merged, which has a section of its own below. `tools/prof/bench.py` and `tools/prof/warm.py`, 5 timed runs and 3 counted ones, against the binaries of M5's merge. The load was 12 to 15 from other work, so what to read is instructions, which are exact, and faults; the times are in the tables further down, at a load of 2 to 3, and a ratio from runs in turn is how they reach the present.

| document | Minstr plain, M5 | now | ratio | PGO, M5 | now | ratio | faults PGO, M5 | now |
|---|---|---|---|---|---|---|---|---|
| hello | 46.8 | 46.5 | 0.993 | 43.8 | 43.6 | 0.996 | 639 | 478 |
| plain hello | 5.35 | 5.11 | 0.954 | 4.92 | 4.71 | 0.957 | 538 | 443 |
| 40 tikz-cd diagrams | 6454.5 | 6423.1 | 0.995 | 6005.7 | 5987.0 | 0.997 | 1174 | 919 |
| `tex.web` in plain TeX | 2525.0 | 2459.8 | 0.974 | 2364.1 | 2319.7 | 0.981 | 2413 | 1634 |
| the thesis | 5377.2 | 5339.1 | 0.993 | 5022.9 | 4995.2 | 0.994 | 2486 | 1876 |
| LMB | 5325.7 | 5299.9 | 0.995 | 4977.1 | 4960.6 | 0.997 | 2389 | 1677 |
| summary | 1736.0 | 1731.7 | 0.998 | 1631.8 | 1629.9 | 0.999 | 1518 | 1348 |
| SGA1-VI | 1671.9 | 1662.6 | 0.994 | 1552.0 | 1547.6 | 0.997 | 1816 | 1514 |
| SGA3-English | 38485 | 38107 | 0.990 | 35575 | 35256 | 0.991 | 6017 | 3822 |
| `pdflatex.fmt`, one build | 31826 | 31659 | 0.995 | 29682 | 29598 | 0.997 | 6086 | 6406 |

A run that restores, once the file has its seal, against a cold one and the run that writes it. PGO, with the plain build's warm run beside it. Faults are user-mode ones, by `warm.py`'s counters, which do not count a page first touched in a system call. Under the policy of section 7 a document has this from its second or third run on.

| document | checkpoint MB | Minstr cold | write | warm | warm/cold | warm at M5 | faults warm, M5 | now | plain warm, M5 | now |
|---|---|---|---|---|---|---|---|---|---|---|
| hello | 9.30 | 43.61 | 50.37 | 9.68 | 0.222 | 19.37 | 423 | 277 | 20.94 | 10.35 |
| 40 tikz-cd diagrams | 10.68 | 5987 | 5996 | 5573 | 0.931 | 5601 | 775 | 850 | 6023 | 5983 |
| the thesis | 14.84 | 4995 | 5009 | 3104 | 0.621 | 3168 | 1656 | 1406 | 3407 | 3333 |
| LMB | 15.20 | 4961 | 4975 | 2899 | 0.584 | 2958 | 1607 | 1478 | 3181 | 3113 |
| summary | 13.42 | 1630 | 1641 | 250.6 | 0.154 | 289.1 | 1098 | 1141 | 312.2 | 270.1 |
| SGA1-VI | 10.81 | 1548 | 1557 | 1154 | 0.746 | 1170 | 1402 | 1286 | 1266 | 1246 |
| SGA3-English | 11.45 | 35255 | 35266 | 34564 | 0.980 | 34894 | 4795 | 3050 | 37762 | 37375 |

The same binaries in turn, PGO, CPU, the new over M5's, at a load of 12 to 13: hello warm 0.678 at the median and 0.698 at the minimum over 101 runs, and cold 0.950 and 0.935; the thesis warm 0.906 and 0.921 over 21, and cold 0.912 and 0.971; SGA3 cold 0.964 and 0.944 over 9, and warm, where most of a run is the body, 0.970 and 1.055.

Then the load fell to 5 to 7 for a few minutes, with the IPC of the thesis at 2.2, about what M5's table below had, and these were measured. PGO, 7 runs each in turn with M5's binary, CPU at the median: hello 9.6 ms cold against 10.3, plain hello 2.5 against 2.8, the thesis 635 against 655, LMB 623 against 641, summary 208 against 213, SGA1-VI 211 against 224, `pdflatex.fmt` 2556 against 2561. SGA3 is two-humped there, 5156 against 6046 at the median, and at the minimum 4462 against 4633. A warm hello, by `warm.py` at a load of 5: 4.5 ms of CPU at the median, 4.1 at the minimum, and 4.7 and 4.2 of wall; in turn with M5's at a load of 7, 4.92 against 7.05 over 101 runs. A warm thesis was not measured at that load, and SGA3's 4.6 s below comes from the ratios: those two are estimates.

What the merge itself decided. A kept list of a directory for kpathsea's misses is good while the directory's stamp stands, and is kept once that stamp is old enough to show a change made just after it: 20 ms if it has a fraction of a second, which comes from a clock that moves at least every 10 ms, otherwise 2 s. Telling it of the run's own writes is not needed then, and it has no gap for another program's write. On one arXiv paper, in its own directory, a pass right after the last one makes 3941 system calls and 2 `getdents64`; with 2 s for every stamp it made 5938 and 794. `\tracingassigns` no longer keeps a checkpoint at `\document`, since `eq_define` sees through the armed bit. `seen` is opened as the other files of the private cache are.

Measured at the merge of M6's branches by `tools/prof/bench.py` and `tools/prof/warm.py`, on the machine of `docs/BUILD.md` at a load of 2 to 4: 5 timed and 3 counted runs. *M5* is the plain build at the merge of M5, *plain* a release build and *PGO* what `tools/pgo.py` leaves, each with the native diagram path on and, as *off*, with `RSTEX_NO_COMMUTE=1`. Instructions are exact; CPU is the median of user and system. The pdflatex columns are from M5's measurement. Since then pdflatex takes 641 M more instructions on every document here: hello 1187 M and 156 ms, the thesis 16578 M and 1683 ms. That is a constant, which fits the system's `pdftex.map`, rebuilt at 42000 lines, and it has not been looked into further; rstex keeps its tables of that file in the cache directory.

| document | Minstr pdflatex | M5 | plain | off | PGO | off | PGO/M5 | CPU ms pdflatex | M5 | plain | off | PGO | off |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| hello | 546.3 | 46.8 | 45.9 | 45.9 | 43.6 | 43.6 | 0.932 | 77.2 | 10.2 | 10.0 | 9.7 | 9.0 | 9.3 |
| plain hello | 180.5 | 5.3 | 4.5 | 4.5 | 4.4 | 4.4 | 0.819 | 34.1 | 2.6 | 2.5 | 2.5 | 2.5 | 2.3 |
| 40 tikz-cd diagrams | 17671 | 6454 | 538 | 6432 | 510 | 6051 | 0.079 | 1825 | 763 | 70 | 721 | 65 | 637 |
| `tex.web` in plain TeX | 3898 | 2525 | 2509 | 2509 | 2363 | 2363 | 0.936 | 595 | 326 | 324 | 329 | 295 | 287 |
| the thesis | 15937 | 5377 | 2973 | 5346 | 2824 | 5056 | 0.525 | 1848 | 669 | 388 | 662 | 352 | 592 |
| LMB | 15941 | 5326 | 3153 | 5291 | 2996 | 5008 | 0.563 | 1672 | 655 | 406 | 641 | 366 | 582 |
| summary | 6357 | 1736 | 1721 | 1721 | 1641 | 1641 | 0.945 | 665 | 213 | 209 | 209 | 190 | 190 |
| SGA1-VI | 4406 | 1672 | 1651 | 1651 | 1562 | 1562 | 0.934 | 504 | 220 | 216 | 217 | 200 | 200 |
| SGA3-English | 91603 | 38487 | 38315 | 38315 | 35841 | 35842 | 0.931 | 10633 | 4891 | 4854 | 4860 | 4432 | 4443 |
| `pdflatex.fmt`, one build | 71800 | 31830 | 31830 | 31830 | 29930 | 29930 | 0.940 | 5830 | 2850 | 2780 | 2780 | 2420 | 2520 |

A run that restores a preamble checkpoint, against a cold one and against the run that writes it. PGO, path on:

| document | checkpoint MB | Minstr cold | write | warm | warm/cold | CPU ms cold | write | warm |
|---|---|---|---|---|---|---|---|---|
| hello | 9.24 | 43.6 | 50.3 | 18.9 | 0.435 | 9.0 | 17.3 | 6.4 |
| 40 tikz-cd diagrams | 10.63 | 510 | 520 | 98.3 | 0.193 | 65 | 75 | 19.4 |
| the thesis | 14.68 | 2824 | 2838 | 949 | 0.336 | 363 | 367 | 151 |
| LMB | 15.09 | 2996 | 3011 | 956 | 0.319 | 370 | 382 | 139 |
| summary | 13.30 | 1641 | 1652 | 287 | 0.175 | 190 | 201 | 43 |
| SGA1-VI | 10.68 | 1562 | 1572 | 1176 | 0.752 | 201 | 210 | 154 |
| SGA3-English | 11.35 | 35841 | 35856 | 35154 | 0.981 | 4669 | 4613 | 4368 |

Against section 2. Its pdflatex took less than pdflatex at M5 on this machine, so the fourth column scales the time by the two: an estimate, not a run.

| scenario | target | measured, PGO | scaled to section 2's pdflatex | |
|---|---|---|---|---|
| hello | under 5 ms | 9.0 ms cold, 6.4 ms warm; pinned to one CPU 8.3 and 5.8 ms | 7.7 ms cold, 5.5 ms warm | not reached; plain hello is 2.5 ms |
| thesis, cold | under 600 ms | 352 ms | 263 ms | reached |
| thesis, warm with native diagrams | under 130 ms | 144 ms of CPU pinned to one CPU, 151 ms otherwise: see below | | not reached |
| format build | under 2.5 s | 2.42 s | 2.08 s | reached |
| SGA3-English | under 4 s | 4.43 s of CPU, 4.15 s of wall | 3.87 s | reached by the estimate, not by the run |

On an idle test PC, a Ryzen 9 5950X at a load of 0.1, with `TEXMFHOME` empty so that a `~/texmf` of its own stays out, by the medians of 21: hello 7.65 ms cold and 5.22 ms warm, with 4.50 ms the best of those; plain hello 1.94 ms; `tikzcd-many` 55 ms cold and 14.9 ms warm. Only public documents have been run there in this comparison.

**Long documents after M5,** on their own branch, before the merge above: what is there since is in the first table.  At a load of 30 to 45, so times say little.  *base* is M5's merge, *new* adds what follows; instructions are of 3 counted runs, and exact.  Branch misses are of the same runs, sys the median of 3 timed ones.

| document | Minstr plain, base | new | PGO, base | new | PGO ratio | branch misses PGO, M | faults | sys ms PGO |
|---|---|---|---|---|---|---|---|---|
| hello | 46.8 | 46.7 | 43.8 | 43.8 | 1.000 | 0.3, 0.3 | 634, 628 | 9.8, 8.8 |
| `tex.web` in plain TeX | 2525 | 2461 | 2364 | 2320 | 0.981 | 12.9, 13.3 | 2462, 1883 | 53, 28 |
| 40 tikz-cd diagrams | 6455 | 6455 | 6006 | 6006 | 1.000 | 33.4, 35.1 | 1097, 1164 | 33, 20 |
| the thesis | 5377 | 5356 | 5023 | 5005 | 0.996 | 37.1, 31.9 | 2357, 2024 | 57, 42 |
| LMB | 5326 | 5315 | 4977 | 4969 | 0.998 | 35.1, 33.9 | 2395, 1963 | 65, 43 |
| SGA1-VI | 1672 | 1662 | 1552 | 1545 | 0.995 | 11.4, 10.9 | 1935, 1579 | 48, 30 |
| `arxiv/math.AG/1912.07119v3` | 4003 | 3702 | 3725 | 3471 | 0.932 | 23.4, 24.2 | 5362, 2809 | 72, 49 |
| `books/hott/hott-a4`, PNG with alpha | 30941 | 24307 | 28790 | 22916 | 0.796 | 153, 153 | 8982, 4671 | 263, 156 |
| SGA3-English | 38486 | 38176 | 35576 | 35309 | 0.992 | 283, 269 | 6367, 4095 | 274, 189 |
| `pdflatex.fmt` | 31830 | 31830 | 29680 | 29720 | 1.001 | | | |

SGA3's cycles over 12 runs each in turn, PGO, before the change to huge pages of section 7: median 31.62 G to 30.85 G, minimum 29.75 G to 29.64 G.  The two commits on pictures alone take `hott-a4` from 22.11 G to 19.51 G, plain, in one round.

Where SGA3's cycles go, from counters by function on a prof build without frame pointers (`rstex_prof.py --by-event`) and AMD's IBS for addresses:

| | cycles | instr | L1d misses | L2 misses | branch misses | L1i misses |
|---|---|---|---|---|---|---|
| token machine | 75.6 | 76.0 | 56.6 | 58.0 | 68.2 | 87.0 |
| `get_node`, `free_node`, copy and flush | 9.7 | 7.7 | 27.2 | 18.4 | 15.4 | 3.5 |
| `line_break`, `hpack`, expansion | 5.3 | 5.8 | 5.6 | 8.1 | 7.3 | 3.0 |
| `ship_out` and the text path | 4.9 | 6.6 | 4.5 | 5.4 | 4.6 | 1.2 |
| math, boxes, pages, tables | 1.9 | 1.3 | 2.8 | 5.0 | 2.4 | 2.8 |
| `memmove` and the like | 1.3 | 1.1 | 2.1 | 2.6 | 1.1 | 1.4 |
| fonts | 0.7 | 0.8 | 0.5 | 0.8 | 0.4 | 0.2 |

The biggest function has 11% of the cycles, the next 5.5%, and only six have more than 3.  Against the thesis, per 1000 instructions on the PGO build: L2 data misses 1.43 and 0.56, branch misses 5.9 and 4.5, L1d misses 7.9 and 4.3, L1i misses 14.9 and 12.7.  A third of SGA3's L1d misses are in `mem`, a quarter in `toks`.  Its output routine copies 22 M nodes, of which 12.7 M are characters, for 2.5 M characters shipped: each `\copy` of a page reads a list and takes cells from `avail`, two pointer chases over memory the literal allocators have scattered, so about 1.15 L1d misses for each node, which nothing can prefetch.

What came of it.  **Tables** for `char_stretch` and `char_shrink` and for the divisions of `adv_char_width`: `round_xn_over_d` and `divide_scaled` take a third of the instructions they took on SGA3, and `tex.web` runs 2% fewer in all.  **`copy_node_list` and `flush_node_list`** take the commonest kinds with no jump, whose target was missed on 40% of the nodes: 5% fewer branch misses on SGA3.  **System calls.**  A miss in kpathsea reads the directory for a name in another case, 1069 times on SGA3 and 667 on the thesis.  The PDF's chunk list kept 1 MiB for each page, 1.3 GB of heap with 1015 `brk` calls, and wrote 2489 times.  Together that is 22 k system calls to 14 k on SGA3, 7.6 k to 4.8 k on the thesis, 14 to 36% fewer faults, and a quarter to a third less system time.  **Pictures.**  An image's bytes went to `pdf_out` one by one, and PNG's row filters are now vector code by way of the lanes of a pixel.

**Why SGA3 is not at 4 s.**  It runs 0.9% fewer instructions and about 2% fewer cycles, with a third less system time, so an estimated 4.6 s of CPU on the 4.81 of the table above if the ratios hold on a quiet machine, which could not be measured here.  What it lacks is 14%, nearly all of it in a flat token machine.  The largest thing left that is not an instruction-count question is the front end: L1i misses go from 56 M to 107 M on LMB from one run to another at a high load, so it costs much and depends on what else the core is running.  Left to try: an index of where a list's `\fi` is, for `capture_skip`'s 5.5%, perhaps 3%, which must be invalidated by a change to any conditional's meaning and by any block freed, so risk above gain; a calling convention without callee-saved registers, since a tenth of all operations are stack traffic, which is nightly Rust.  Each other item measured is below 0.5%.

**Small documents after M5,** on their own branch, before the merge above. PGO on both sides, M5's binary and that one, by `tools/prof/warm.py --policy` at a load of 30 to 45 from other work on the machine. At that load a CPU time is two to four times what it is on a quiet machine, so what can be read from these is instructions, which are exact, and user-mode faults. The CPU columns are the minimum of 5, not in turn, and only rough. Warm means a restore after the run that left its seal.

| | Minstr merge | now | faults merge | now | CPU ms merge | now |
|---|---|---|---|---|---|---|
| hello, cold | 43.76 | 43.70 | 447 | 301 | 25.1 | 24.7 |
| hello, warm | 19.32 | 9.63 | 423 | 287 | 18.4 | 10.3 |
| `ladder/07-amsmath-suite`, cold | 226.64 | 228.45 | 517 | 389 | 117.3 | 101.8 |
| `ladder/07-amsmath-suite`, warm | 100.90 | 92.94 | 486 | 612 | 61.2 | 44.5 |
| summary, cold | 1631.82 | 1635.89 | 1124 | 983 | 726.1 | 658.3 |
| summary, warm | 288.97 | 251.03 | 1010 | 1130 | 140.9 | 112.7 |
| the thesis, cold | 5022.85 | 5031.54 | 1883 | 1612 | 2260.1 | 2080.5 |
| the thesis, warm | 3167.56 | 3133.44 | 1608 | 1706 | 1429.2 | 1284.3 |

Cold, the long documents move by the difference between two profiles, up to 0.2%; the plain builds, which have none, are within 0.1%. `ladder/07-amsmath-suite` takes 0.4% more on the plain build as well, from looking up each font's glyph names in the format's table where it lies: 2.6 M instructions in a prof build, against 0.8 M for a table on the heap, which saves its 36 faults but loses in instructions past a few fonts.

Hello, in turns of 101 warm runs and 61 cold ones at a load of 25: a warm run took 0.69 of the merge's CPU at the median and 0.72 at the minimum, a cold one 0.94 and 0.93. Applied to what the merge took at a load of 2 to 3, that is about 4.5 ms warm and 9.8 ms cold. That is an estimate: the machine did not get quiet enough to measure it.

What is left of a warm hello.  Over half of its CPU is the kernel's. Reading `toks`, 2.9 MB, takes the longest single system call, and mapping it instead costs as much again in the 235 pages that get copied. `exec` and the end of the process, which frees what was touched, came to 1.1 ms in a prof build's phases at a load of 8. Of the 9.6 M instructions a third is LaTeX from the `.aux` to the shipout, and 40% the font: 2.65 M in `write_t1`, a table of glyph names, and most of the 1.4 M in deflate. The most that is left to win is a cache of subset fonts, keyed on the font's stat and its glyph set, not built: about 3.5 M instructions here and 90 M on the thesis.

What a run costs from an empty cache under the policy, as it now stands, against writing at first sight as at M5's merge. Instructions, PGO:

| | first | second | third | later | at M5's merge: first | later |
|---|---|---|---|---|---|---|
| hello | 43.96 | 50.37, writes | 12.43 | 9.68 | 50.48, writes | 19.37 |
| `ladder/07-amsmath-suite`, 275 KB read | 235.28, writes | 95.77 | 95.77 | 92.98 | 233.69, writes | 100.90 |
| summary, 1.9 MB read | 1640.63, writes | 254.50 | 254.48 | 250.55 | 1642.93, writes | 289.07 |
| the thesis, 3.6 MB read | 5009.00, writes | 3108.41 | 3108.44 | 3104.15 | 5037.74, writes | 3167.77 |

"Later" is from the run after the file is 2 s old, when the sum stops. So a file compiled once costs 0.8% more instructions than with no cache at all, not 15%. In CPU, in turns of 31 runs at a load of 45, at the minimum: 18.7 ms with no cache, 17.1 for a first run that leaves a stamp, the difference being in the noise, and 31.1 for one that writes.

### The five tracks, merged

The output cache, the macro lever, `--until-stable`, SyncTeX and page records, on one tree. **All of this was measured on a loaded machine, at a load of 8 to 33, some of it these runs' own: instructions are what to read, then the paired cycles, and no millisecond here is a quiet machine's.** The quiet figures come when a release is built.

A plain build and a PGO one, trained on `auto`, against those of the base they all branched from, by `tools/prof/compare_cycles.py`: base, new, base in turn, 40 rounds and 12 for SGA3, the instructions of a run and the median of the new's cycles over the base's beside it, with in brackets the one base over the other, which says how far the machine could be trusted. *Warm* restores the preamble's checkpoint, with `RSTEX_NO_PAGES` and no PDF there. *With the PDF* is that with the last PDF in its place, as between two runs. *From the last record* has a chain as well, and is what an edit after the last record costs, which is the floor of a run. The base has no cache and no chain, so each of those is a ratio to its warm run. A document of too few pages has no record. Plain:

| document | Minstr cold, base | now | ratio | cycles | warm, base | now | ratio | cycles | with the PDF | ratio | cycles | from the last record | ratio | cycles | faults warm, base | now | with the PDF | last |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| hello | 46.04 | 46.22 | 1.004 | 1.004 (1.022) | 9.93 | 10.03 | 1.010 | 0.994 (0.982) | 5.59 | 0.563 | 0.747 (1.005) |  |  |  | 303 | 307 | 190 |  |
| tikzcd-many | 535.4 | 537.4 | 1.004 | 1.003 (0.993) | 91.72 | 92.12 | 1.004 | 1.016 (1.022) | 71.64 | 0.781 | 0.837 (1.010) | 17.28 | 0.188 | 0.226 (1.044) | 718 | 726 | 478 | 264 |
| the thesis | 2952 | 2925 | 0.991 | 0.988 (1.011) | 942.7 | 910.4 | 0.966 | 0.948 (1.015) | 730.5 | 0.775 | 0.834 (0.996) | 56.75 | 0.060 | 0.062 (1.001) | 1808 | 1812 | 1516 | 656 |
| LMB | 3146 | 3112 | 0.989 | 1.004 (1.011) | 956.2 | 917.0 | 0.959 | 0.961 (1.014) | 699.1 | 0.731 | 0.803 (1.010) | 51.05 | 0.053 | 0.061 (0.998) | 1951 | 1876 | 1462 | 897 |
| summary | 1729 | 1733 | 1.003 | 1.029 (0.997) | 269.4 | 270.0 | 1.002 | 1.003 (0.989) | 186.0 | 0.690 | 0.815 (1.006) |  |  |  | 1119 | 1044 | 722 |  |
| SGA1-VI | 1657 | 1625 | 0.981 | 0.972 (1.025) | 1241 | 1210 | 0.975 | 0.982 (1.002) | 1051 | 0.847 | 0.902 (0.997) | 67.73 | 0.055 | 0.059 (0.997) | 1258 | 1252 | 955 | 440 |
| SGA3-English | 37977 | 34090 | 0.898 | 0.902 (0.990) | 37247 | 33391 | 0.896 | 0.861 (0.975) | 31144 | 0.836 | 0.835 (0.989) | 301.8 | 0.008 | 0.007 (1.017) | 3102 | 3250 | 3074 | 1087 |
| `pdflatex.fmt`, one build | 31780 | 31043 | 0.977 | 1.000 (1.002) |  |  |  |  |  |  |  |  |  |  |  |  |  |  |

PGO, each on its own profile:

| document | Minstr cold, base | now | ratio | cycles | warm, base | now | ratio | cycles | with the PDF | ratio | cycles | from the last record | ratio | cycles | faults warm, base | now | with the PDF | last |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| hello | 43.47 | 43.41 | 0.999 | 1.020 (0.995) | 9.22 | 9.30 | 1.008 | 1.007 (0.991) | 5.26 | 0.570 | 0.709 (1.044) |  |  |  | 283 | 284 | 178 |  |
| tikzcd-many | 505.6 | 504.9 | 0.999 | 1.011 (1.015) | 85.86 | 86.03 | 1.002 | 1.007 (0.985) | 66.91 | 0.779 | 0.821 (0.984) | 16.00 | 0.186 | 0.237 (0.991) | 711 | 712 | 456 | 245 |
| the thesis | 2789 | 2749 | 0.986 | 0.972 (0.992) | 879.6 | 846.0 | 0.962 | 0.929 (1.010) | 682.4 | 0.776 | 0.845 (0.992) | 52.90 | 0.060 | 0.066 (1.002) | 1802 | 1820 | 1463 | 626 |
| LMB | 2972 | 2925 | 0.984 | 0.976 (1.010) | 890.3 | 850.6 | 0.955 | 0.947 (1.031) | 652.3 | 0.733 | 0.799 (0.991) | 46.93 | 0.053 | 0.064 (1.010) | 1886 | 1875 | 1535 | 848 |
| summary | 1641 | 1636 | 0.997 | 0.987 (0.999) | 250.4 | 250.3 | 1.000 | 1.010 (0.999) | 175.2 | 0.700 | 0.797 (1.016) |  |  |  | 1085 | 1106 | 730 |  |
| SGA1-VI | 1553 | 1518 | 0.977 | 0.992 (0.994) | 1156 | 1124 | 0.972 | 0.981 (1.006) | 979.5 | 0.847 | 0.881 (1.007) | 62.61 | 0.054 | 0.061 (0.990) | 1316 | 1282 | 953 | 486 |
| SGA3-English | 35344 | 31670 | 0.896 | 0.923 (0.985) | 34646 | 31006 | 0.895 | 0.922 (1.018) | 28872 | 0.833 | 0.886 (0.955) | 291.4 | 0.008 | 0.009 (1.023) | 3168 | 3318 | 3056 | 1054 |
| `pdflatex.fmt`, one build | 29932 | 29118 | 0.973 | 0.975 (0.996) |  |  |  |  |  |  |  |  |  |  |  |  |  |  |

- **The floor.** The thesis from its last record with the last PDF in place: 52.9 M instructions with PGO and 56.8 M plain, 0.06 of what a warm run took at the base, in cycles too. The estimate was 53 M, by arithmetic, from when this had never been built. With the cache off the same run takes 199.4 M plain and 180.7 M with PGO: the fonts were most of what a resume had left.
- **A run that writes a whole chain,** from the preamble with the PDF in place: on the thesis 764.5 M plain and 715.5 M with PGO, 1554 and 1527 faults, against 730.5 M and 682.4 M with no chain; cycles 1.014 and 1.053 of the base's warm run, with controls of 0.986 and 1.044. So a run that pays for writing still costs what a warm run did.
- **Cold,** nothing reuses anything: the macro lever is what moves, 0.898 and 0.896 on SGA3, 0.977 and 0.973 on a format build. A document with no microtype and little to hand on pays for the code it does not use: hello 1.004 plain, 0.999 with PGO. A warm hello with no PDF to find is 1.010 and 1.008, of which about a half is the first index being written, and with its PDF 0.563 and 0.570.
- **What an editor gets of this is not the last column yet.** Lemma always passes `-synctex=1`, and a chain is refused under it (section 7). So under its flags a run has the column *with the PDF*, and SyncTeX's cost on top of that (section 9), until a record can give that file back.
- **The binary** went from 15.67 to 17.60 MB with PGO, unstripped, 15.67 to 17.79 MB plain.

All of it holds what the gate holds: at the merge, `tests/gate.sh` in all its steps on the plain build and integration, TRIP and the compare on the PGO one, and with those binaries `tests/until_stable/compare.py` against latexmk on 59 documents with `-synctex=1` and 59 without, where passes resume from records; `pages.py session` above, `resume.py --all` on 5 documents, 38 records, `resume_edit.py --session`, SIGKILL on real documents, and the state at 72 places in 6 documents, each held against a chain that deflated all it made by a run that found its streams in the PDF. Whole corpus runs were made on the macro lever alone, on its branch, and not on this merge. Linux only: macOS and Windows are `cargo check`.

### On dev's tree

The round above and the diagram branch's commands and reviews, merged into dev's tree, which had gone on meanwhile with the second oracle, the corpus triage and the wide sums of the page builder, and none of the three had seen the others. Instructions of 3 counted runs by `tools/prof/warm.py` and `bench.py`, so nothing here is a time. *Dev* is its plain build before the merge, with no path, no cache and no chain; *M6* is the diagram branch's final build, with the path and neither cache nor chain; *round* is the build the table above calls *now*. Plain:

| document | Minstr cold, dev | M6 | round | now | /dev | /M6 | /round | warm, dev | M6 | round | now | /dev | /M6 | /round | with the PDF, round | now | /round | from the last record, round | now | /round | faults warm, round | now |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| hello | 46.24 | 45.88 | 46.24 | 45.57 | 0.986 | 0.993 | 0.986 | 10.32 | 20.07 | 10.05 | 9.52 | 0.922 | 0.474 | 0.947 | 5.62 | 5.54 | 0.986 |  |  |  | 306 | 261 |
| tikzcd-many | 6402 | 538.3 | 537.4 | 537.8 | 0.084 | 0.999 | 1.001 | 5964 | 103.5 | 92.15 | 92.91 | 0.016 | 0.897 | 1.008 | 71.66 | 74.08 | 1.034 | 17.31 | 17.91 | 1.035 | 728 | 752 |
| the thesis | 5319 | 2973 | 2925 | 2914 | 0.548 | 0.980 | 0.996 | 3319 | 1001 | 910.5 | 905.9 | 0.273 | 0.905 | 0.995 | 730.6 | 743.5 | 1.018 | 56.81 | 58.44 | 1.029 | 1816 | 1752 |
| LMB | 5280 | 3153 | 3112 | 3098 | 0.587 | 0.982 | 0.995 | 3100 | 1006 | 917.0 | 909.0 | 0.293 | 0.903 | 0.991 | 699.2 | 711.9 | 1.018 | 51.11 | 52.78 | 1.033 | 1877 | 1749 |
| summary | 1727 | 1721 | 1733 | 1717 | 0.994 | 0.998 | 0.991 | 269.0 | 300.5 | 270.0 | 258.8 | 0.962 | 0.861 | 0.959 | 186.0 | 185.6 | 0.997 |  |  |  | 1034 | 968 |
| SGA1-VI | 1666 | 1651 | 1625 | 1619 | 0.972 | 0.980 | 0.997 | 1249 | 1247 | 1210 | 1202 | 0.963 | 0.964 | 0.993 | 1051 | 1058 | 1.006 | 67.76 | 67.90 | 1.002 | 1379 | 1334 |
| SGA3-English | 38211 | 38315 | 34090 | 34196 | 0.895 | 0.892 | 1.003 | 37478 | 37596 | 33390 | 33491 | 0.894 | 0.891 | 1.003 | 31144 | 31279 | 1.004 | 3762 | 3782 | 1.005 | 3203 | 3212 |
| `pdflatex.fmt`, one build | 31250 | 31830 | 31040 | 30940 | 0.990 | 0.972 | 0.997 |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |

PGO, each on its own profile; dev's PGO was not measured:

| document | Minstr cold, M6 | round | now | /M6 | /round | warm, M6 | round | now | /M6 | /round | with the PDF, round | now | /round | from the last record, round | now | /round | faults warm, round | now |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| hello | 43.62 | 43.43 | 43.24 | 0.991 | 0.996 | 18.97 | 9.33 | 9.15 | 0.482 | 0.981 | 5.28 | 5.23 | 0.989 |  |  |  | 284 | 283 |
| tikzcd-many | 510.3 | 504.9 | 507.9 | 0.995 | 1.006 | 98.36 | 86.07 | 88.28 | 0.898 | 1.026 | 66.94 | 69.60 | 1.040 | 16.03 | 16.68 | 1.041 | 715 | 781 |
| the thesis | 2824 | 2749 | 2760 | 0.977 | 1.004 | 949.4 | 846.1 | 856.6 | 0.902 | 1.012 | 682.5 | 698.0 | 1.023 | 52.97 | 54.84 | 1.035 | 1809 | 1744 |
| LMB | 2996 | 2925 | 2935 | 0.979 | 1.003 | 955.5 | 850.6 | 860.2 | 0.900 | 1.011 | 652.3 | 667.6 | 1.024 | 47.00 | 48.80 | 1.038 | 1867 | 1777 |
| summary | 1641 | 1636 | 1633 | 0.995 | 0.998 | 287.1 | 250.3 | 247.0 | 0.860 | 0.987 | 175.3 | 174.9 | 0.998 |  |  |  | 1117 | 1095 |
| SGA1-VI | 1563 | 1518 | 1520 | 0.973 | 1.001 | 1176 | 1125 | 1125 | 0.956 | 1.000 | 979.5 | 984.1 | 1.005 | 62.66 | 62.78 | 1.002 | 1271 | 1362 |
| SGA3-English | 35841 | 31670 | 31823 | 0.888 | 1.005 | 35152 | 31006 | 31157 | 0.886 | 1.005 | 28872 | 29033 | 1.006 | 3505 | 3527 | 1.006 | 3071 | 3205 |
| `pdflatex.fmt`, one build | 29930 | 29120 | 29040 | 0.970 | 0.997 |  |  |  |  |  |  |  |  |  |  |  |  |  |

- **Against dev and M6 nothing rises** in any column they have. Against the round, the columns that only it and this tree have rise where a document has diagrams, and that is what the two reviews put in. With the PDF: the thesis and LMB 1.018, and 1.023 and 1.024 with PGO, tikzcd-many 1.034 and 1.040. A diagram is 0.70 M where it was 0.60 M, and the first of a run looks through what the macros of `SAFE` mention: some 10 M on LMB, in `is_known`, `native_closure_known`, `native_name`, `native_hash_name` and `native_macro_hash`, by function on a prof build, and the thesis with the PDF rises by about as much. With a preamble of tikz-cd alone the first diagram of a run is 1.7 M more than a later one, and 1.1 M more on the round. It is not fixed for a run from the preamble's checkpoint: that would have to hold what a walk finds.
- **From the last record** a run does not walk that again, since the record has the list, but it has the list to read: the thesis 1.029 and 1.035, LMB 1.033 and 1.038, tikzcd-many 1.035 and 1.041. Without the list in the record LMB was 1.25 here.
- **SGA3** is 1.003 to 1.006 of the round in every column, plain and PGO, and 0.89 of dev and M6. SGA1-VI and SGA3 have a user `\outer`, so that `any_user_outer` holds: no argument is handed on, and each delimited call goes to `capture_delimited_outer`, 2.1% and 1.5% of their instructions, which dev keeps out of line for what it gives the others. Inlined again, the thesis takes 1.3% more, summary 1.4%, SGA1-VI 0.4% and SGA3 0.2%, so it stays. By function in a prof build the rest is thin on the loops' room tests: dev's literal stop at the builder's end, which each document pays for and which it is there to keep.
- **A run that writes a whole chain,** from the preamble with the PDF in place: the thesis 782.0 M plain and 735.4 M with PGO, 1.052 and 1.054 of a run with none, where the round had 764 M and 715.5 M, 1.046 and 1.048.
- **SGA3 from its last record** is 3782 M here because `warm.py`'s chain fills up: the run that wrote its checkpoint had `RSTEX_NO_PAGES`, so it said nothing of the document's length, and a record every 4 pages reached 256 MiB well before the end. As a user gets it, by `tools/prof/compare_cycles.py --last`, it was 303.7 M against the round's 302.0 M, plain, before the third merge.
- **One diagram,** by `tools/prof/native_cost.py`: real PGF 161.7 M, the path 0.594 M, of which cells and labels alone 0.232 M, plain; 151.2 M and 0.555 M with PGO. **The `commute` command,** per diagram, by `tools/prof/commute_cost.py`: one with the format cached 7.19 M, 6.94 M on the branch; a file of 20 7.50 M, and of 200 7.24 M, 7.47 M and 7.23 M there; under a preamble from its checkpoint 61 M, 82 M there. What the one cached diagram gained is once per run and not traced.
- **What git did not see.**
  - `\toks`' `chr` is `mem_bot`, so `GUARD`, generated where that was 1, was wrong on dev's tree, and every diagram took real PGF: right output, slower, and `compare_native.py geometry` would have passed. It was seen in a report of `--commute-report`, and `gen_table.py` makes the one line.
  - A run from a page record writes `.aux` and the like back as they stood at the record. A restart then gave the second run that partial `.aux`, another log and another PDF than `--no-commute`. A restore now keeps those files first, as `open_out` does: `pages.py restart`, which holds six ways of starting again against the path off.
  - The memos live in Host and hold references in `toks`, so a run resumed from a record took them again and its state was another: 27 of 44 states differed on `restart.tex`. So the record has them, and the closure's list, which says which of them are looked at again when they move: with the references alone every state from the record on differed in 4 of the gate's documents, and 8 of 13 on `closure.tex`. Letting all of it go at each record instead cost a writing run 10% on the thesis.
  - `{#n}` hands on a block with no look at the builder, where `capture_undelimited` had come to ask for a list of 65 tokens or more, so a job ran past the token that stops the literal loop, and past `slow_capture`: `arg_shareable` asks too (`capture/builder_full_share`).
  - A resumed run had `tight_heap`'s limit from where it first made a block: now in the image, `pages.py reclaim`, 94 of 106 states otherwise.
  - Two copies of the fix to `.fls` under `-output-directory` and of the file tags in an off run, one kept of each: `read_unrecorded`, and `synctex.rs`, literal to `synctex.c`. The wide sums move inside the prefix a record holds: `wide.tex` in `pages.py records`. `warm_equals_cold.py --native`'s lines came through git half merged, and would have been a syntax error in its wrapper: `--path-on` stays. The driver moved to a library on the branch, and all that dev and the round did to `main.rs` went there by hand. `tests/cd/native/run.py`'s warm and key steps, stricter from the round, failed on five of the reviews' cases, and it was the tool: three take no checkpoint, for an error in the preamble, and now say so, and two run twice.
- **An attack on this tree, by someone who had not written it.** Plain build, the base's; all documents neutral, written for this; each next run held against a cold run from the same files.
  - Wrong and now fixed: `checkpoint_validate` took a preamble's read of its own file on trust, which two new `own-writes` cases show on the base; a killed pass could leave the next build with an error and no references, above; a chain that met a full disk, or a limit on a file's size, ended the run with no PDF, for a document with nothing wrong with it; a pass on Windows had another key, by reading only; `proflib.build_format` raced; `adversarial/run.py` left more than the disk has.
  - The fills of `tb_show` and `tok_coalesce` protect the state's sum and no output: `runaway.tex` and `coalesce.tex` in `pages.py` differ in 2 of 6 states without them.
  - No difference found: 560 programs of `tests/token_fuzz.py` in LaTeX with shipouts through macro bodies, arguments, boxes and alignments, a record each page, 7595 records, 2345 runs from them, 21373 states against the writer's; 12 documents written to hold what a record can hold, 12 sets of flags and switches, with and without the last PDF, 2240 records, 15820 states, with an edit, the edit again and the edit undone; 200 documents on a `tight_heap` build, 88 of which stayed within its heap, with shared arguments live across shipouts, 3337 records, 9867 states, and the cold run against the plain build's; 180 trials of `--until-stable` killed at random after an edit, the next run against a literal run from those files and a build from nothing, the bytes of `.fls` and `.synctex.gz` among them; 12 changes of the world with a chain and a cache in place, the map file, a format at the same path and a file of a tree with the same stat among them; `commute` beside `rstex` in one cache.
- **Looked at, and needing nothing.** `zero_glue` at null: no test in the native path, page records, `native_font`, the SyncTeX context or `session.rs` takes a glue pointer for null or a fixed address. A checkpoint's key already has `main_memory` and `hash_extra`, and a warm run under `hash_extra=0` says `15000+20731` as a cold one does. The output cache has no address of the heap in it. A session of `commute` keeps out of that cache and of chains.

### The native diagram path

By `tools/prof/warm.py`, as above. All these diagrams take the path.

| document | diagrams | build | Minstr cold off | on | warm off | on | CPU ms cold off | on | warm off | on |
|---|---|---|---|---|---|---|---|---|---|---|
| tikzcd-many | 40 | plain | 6432 | 538 | 6002 | 104 | 723 | 71 | 671 | 19.8 |
| | | PGO | 6051 | 510 | 5643 | 98 | 638 | 65 | 584 | 19.4 |
| the thesis | 20 | plain | 5346 | 2973 | 3380 | 1001 | 646 | 382 | 419 | 153 |
| | | PGO | 5056 | 2824 | 3185 | 949 | 599 | 363 | 380 | 151 |
| LMB | 14 | plain | 5291 | 3153 | 3151 | 1007 | 672 | 404 | 416 | 150 |
| | | PGO | 5008 | 2996 | 2972 | 956 | 580 | 370 | 352 | 139 |

A run from a checkpoint gives what a cold run gives, the PDF, the log and every side file, byte for byte, with the path on as well: `tests/corpus/snapshot/warm_equals_cold.py --path-on`.

**At its merge with the second speed round.** A plain build against dev's just before it, by `tools/prof/warm.py` and `bench.py`: 3 counted runs, and instructions are what to read. *On the path* is `--commute-report`'s count, and none of these documents declined a diagram.

| document | on the path | Minstr cold, dev | merged | ratio | warm, dev | merged | ratio | faults warm, dev | merged |
|---|---|---|---|---|---|---|---|---|---|
| hello | 0 | 46.50 | 46.05 | 0.990 | 10.35 | 9.95 | 0.961 | 287 | 285 |
| tikzcd-many | 40 | 6423 | 535.4 | 0.083 | 5983 | 91.75 | 0.015 | 859 | 716 |
| the thesis | 20 | 5339 | 2952 | 0.553 | 3333 | 942.8 | 0.283 | 1402 | 1808 |
| LMB | 14 | 5300 | 3146 | 0.594 | 3113 | 956.2 | 0.307 | 1405 | 1869 |
| summary | 0 | 1732 | 1729 | 0.998 | 270.1 | 269.4 | 0.997 | 1014 | 1091 |
| SGA1-VI | 0 | 1662 | 1657 | 0.997 | 1246 | 1241 | 0.996 | 1343 | 1340 |
| SGA3-English | 0 | 38107 | 37977 | 0.997 | 37375 | 37247 | 0.997 | 3054 | 3055 |
| `pdflatex.fmt`, one build | | 31659 | 31780 | 1.004 | | | | | |

With the path off, the merged build takes 5330 M cold and 3326 M warm on the thesis, 5291 M and 3106 M on LMB, and 6408 M and 5969 M on tikzcd-many: within 0.2% of dev. Two things here have not been looked into. A warm run with diagrams takes some 400 to 500 faults more with the path on than off, 1808 against 1407 on the thesis and 1869 against 1364 on LMB, though it runs a third of the instructions. And a format build, which never takes the path, runs 0.4% more. With PGO the path also changes what the profile holds, and `docs/BUILD.md`, "Diagrams, both ways", has what that came to.

A diagram of 6 cells, 6 arrows and 5 labels, by `tools/prof/native_cost.py`, plain: 162.2 M instructions through real PGF; 0.59 M on the path, of which its cells and labels typeset in bare boxes are 0.23 M, what is beyond them 0.36 M and the library's part of that 0.17 M. PGO: 152.4 M and 0.55 M. The two reviews of section 10 took it from 0.52 M: a run looks once at what the macros of `SAFE` mention, some 1200 of them for a plain diagram by way of `\prime`'s, and each diagram after that compares their places and those of the primitives. They took the thesis's warm run from 1004 M to 1001 M all told, since the command's work took more off the start of a run than the proof put on. A decline costs what real PGF costs, and a document of 400 declines of a body with no end in reach costs per diagram what one of 100 does.

Nothing else pays: against M5's plain build, hello 0.980, plain hello 0.841, `tex.web` 0.994, summary 0.992, SGA1-VI 0.988 and SGA3 0.996 of its instructions. The first two fall because `.eh_frame` is no longer walked at the start (`docs/BUILD.md`) and because of what the `commute` command's work took off a short run, the others by `t1_getline`'s looking at a line's last byte first and what has not been looked into, and none of it is the arming, which is one test per file opened.

The diagrams' gate, on all 3246 extracted diagrams: every one of the 2965 that real PGF compiles passes, box dimensions to the scaled point, text, and pixels at 400 dpi with nothing excused; the 281 others are in documents the oracle fails on. 2390 take the path and 565 decline, and each of the 2390 has real PGF's content stream, byte for byte. Of the 89 documents of the corpus that load tikz-cd the oracle fails on 13, and the other 76 are equal to it with the path on: the log but for the figures of section 10, `.aux`, the text, pixels and content stream of every page, `qpdf`; there 1091 diagrams take the path and 463 decline. All 92 diagrams of the owner's documents among them take the path.

| source | diagrams | take the path | |
|---|---|---|---|
| arXiv sample | 1543 | 985 | 63.8% |
| books | 147 | 30 | 20.4% |
| the owner's | 57 | 55 | 96.5% |
| ladder | 41 | 41 | 100% |
| manuals | 94 | 0 | 0% |
| notes | 1165 | 1098 | 94.2% |
| `tests/cd/units` | 199 | 181 | 91.0% |

Why the rest decline: `srcltx`, whose `\src@spec` each `$` runs and which may write a `\special`, in one paper, 126; one book's `>={Latex}`, a tip of arrows.meta that the library does not draw, 112; `tikz-nfold` changing how arrows start, in two papers, 65; `\tikzset{nodes=draw}` in tikz-cd's own test file, 64; a name undefined in a cell, whose error real PGF has to give, 38; a primitive with another meaning, in one paper that loads `breqn`, 30; `/tikz/column N` styles, 27; tikz-cd's own manual, which loads the library with `"` made another character and so has another `\endtikzcd`, 27; a macro that computes a name, 21; arrow tips and other keys outside the library's subset, about 20; a math version out of math groups, 14; what a cell starts with being expanded, 8; a macro that holds something of the structure, 7.

To find what a document can set that a diagram reads, the keys of `/tikz` and tikz-cd's directory were set outside a picture with nine values each, 4401 settings, each before one diagram with several kinds of arrow, on the oracle and on the path. 4352 could be judged, the others ending the run or giving the page no fate. 2465 take the path, 1868 decline and 19 start again, and all of them have the oracle's stream. This is not part of the gate, since what a change of the guards can do is send more to real PGF; `gen_table.py` does the part that finds what a key writes.

What the thesis's warm run is made of, by `tools/prof/rstex_prof.py --warm`, measured before the two reviews, whose prof build counts 1035 M where a plain one counts 1000 M: the start and the checkpoint's restore 14 M, and 35 ms of CPU at a load of about 30, most of it kernel; the first input 6 M; up to the first shipout 133 M, which includes the first pages' text; that shipout 17 M; the remaining pages 707 M; the end of the run 9 M; fonts 146 M, of which parsing and subsetting Type 1 fonts 105 M and deflate 11 M; closing the PDF 4 M. `get_next_leaf` 8.3%, `macro_call` 6.4%, `scan_group` 5.2%, `capture_skip` 3.5%, `t1_getline` 3.4%. 283 path lookups cost 8.6 M but some 37 ms of the wall at that load.

130 ms is not reached. PGO, pinned to one CPU at a load of about 3: 949 M instructions, 503 M cycles, 144 ms of CPU by the median of 21 and 142 ms at best, 146 and 143 ms of wall; not pinned, 151 ms of CPU and 145 ms of wall. Plain: 1001 M instructions, 152 ms of CPU pinned. It was 3168 M and 404 ms before diagrams went native.

On a quiet second machine, a Ryzen 9 5950X whose CPU times are about 0.85 of this one's, PGO, on the branch before the second speed round: the thesis 324 ms cold and 135 ms warm, hello 7.4 and 4.7 ms, SGA3 4040 ms, `pdflatex.fmt` 2135 ms; pdflatex there takes 148 ms on hello, 1529 on the thesis and 9441 on SGA3. Since then the thesis's warm run has gone from 1000 to 943 M instructions, plain, by the table above, so about 127 ms there if the time goes with them: an estimate. On this machine it has not been measured quiet.

For a margin, in order of size: keep each font's subset from the last run, 146 M of the 1000 and the part least tied to the document's text; answer the 283 lookups from what the checkpoint validated, for the kernel's share; the rest is the token machine over the body, 700 M, which is no longer about diagrams.

### The commute command

By `tools/prof/commute_cost.py`, a plain release build. Per diagram, of the small test set in that tool, 2 to 4 fonts each. First on the idle test PC (Ryzen 9 5950X, load 0.1 to 0.8), 5 counted and 41 timed runs, the cache on btrfs. Then on the machine of the rest of this section, at a load of 8, 3 counted and 41 timed runs. That machine at a load of 16 took 8.4 and 16.7 ms for one with the format cached.

| | diagrams | Minstr | Mcycles | faults | wall ms, best | median | here: wall best | median |
|---|---|---|---|---|---|---|---|---|
| one, no format yet | 1 | 32653 | 11380 | 12955 | 2699 | 2715 | 3661 | 3668 |
| one, format cached | 1 | 6.94 | 3.59 | 781 | 2.44 | 2.85 | 4.09 | 4.48 |
| a file of 20 | 20 | 7.47 | 2.96 | 42 | 1.01 | 1.05 | 1.55 | 1.83 |
| a file of 200 | 200 | 7.23 | 2.72 | 4 | 0.89 | 0.91 | 1.32 | 1.45 |

The target of under 10 ms is met on both, and at a load of 16 only by the best runs. Of the 2.4 to 2.9 ms on the idle machine about 0.9 is user time and 1.5 the kernel: the `exec`, some 500 page faults and some 300 system calls, of which 100 `stat`s look at the tree. A file of many is cheaper by that and no more, since each PDF does its fonts again. The preamble's first run takes 710 to 840 M instructions, and a run from its checkpoint 82 M and 35 ms of wall here; on the idle PC a run from it took 142 ms, 113 of it system time, and 185 M instructions, from lookups through a `~/texmf` there with no `ls-R` and ten thousand directory entries, which is not this tool's and not what a TeX Live alone has.

`tests/cd/compare_cli.py --sample 300`, on all 199 diagrams of `tests/cd` and 300 of the other 2990 of the public manifest, seed 1. The diagram alone against `\documentclass[border=0pt]{standalone}`: 302 of the 302 that both draw are equal, width, height and depth to the scaled point, `/MediaBox`, every pixel at 400 dpi, and the SVG by `rsvg-convert` with every clearly dark pixel on either side within one pixel of a dark one on the other. 188 need their document's preamble and fail on the oracle; 9 are declined: `scale` on the diagram 3, an undefined name 2, and one each of an unknown arrow key, rounded corners larger than the cell, a style TikZ runs by itself and an unknown label key. The diagram under its document's preamble, class and all, against the oracle with the page made by hand, as `standalone` does not get along with a document's `geometry` or headers: 278 of 278 by the same tests, and 32 fail on the oracle; 57 of those have no SVG, as real PGF drew them or a font has none. `--files`: 186 diagrams of `tests/cd` in one file, 372 of 372 files equal to what each writes alone. `--check-negative`: a diagram off by 0.05 bp fails on pixels, and an SVG off by 0.5 bp on the renderer's; how little more than that the SVG's test can see has not been measured.
