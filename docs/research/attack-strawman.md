# Attack on the rstex strawman: verdicts on (a)-(f)

**Bottom line**
- The flat-POD model survives. The snapshot idea is the biggest win, worth 45% on the thesis.
- (b), (c) and (f) can silently produce wrong output as written.
- The `MAP_PRIVATE` mechanism in (a) and (b) is the wrong choice.
- (d) is worth about 1%.
- The second-biggest win is not in the strawman: 25.5 ms per tikz-cd diagram.

Tags: [V] means measured or read in source here. [I] means inferred. Line numbers refer to `refs/pdftex/pdftex.web` unless another file is named. I compiled copies only. I wrote nothing to the project dir (the untracked `.gitignore`, `bench/`, `tests/` there are not mine).

## Measured cost map [V]

My thesis runs have a 1519 ms median, against your 1380 ms baseline, so read these as ratios.

| Phase | ms |
|---|---|
| Process start to format open (`texmf.cnf`, `ls-R`) | 7.9 |
| Undump | 36.8 |
| `\everyjob` to first input | 2.4 |
| Preamble to `\document`: `scrartcl` done by 357, `tikz-cd` by 567, `hyperref` by 641 | about 650 cumulative |
| Macro block after `hyperref` (lines 21-99) | 34 |
| `\document` hooks (LMB: 46), which a snapshot at `\document` does not save | 36 |
| Body: 34 pages, of which about 510 ms is 20 diagrams | 767 |
| After last hook to exit (fonts, xref) | about 42 |

- **Plain pages:** about 5.5 ms each (LMB median).
- **LMB:** `\document` at 720 ms, body 702 ms, 14 diagrams.
- **tikz-cd:** load 113 ms, then a linear 25.5 ms per diagram from 1 to 100 diagrams, for a 4-cell, 4-arrow diagram.

**Correction to the brief:** the installed pdfTeX has a 16-byte `memory_word`, not 8.
- `mem` is malloc'd at 80,000,016 bytes for `main_memory`=5M.
- `hash` and `eqtb` are 16 bytes per entry; `font_info` is 8.
- `str_start` is 8 bytes per entry and `link` is 8 bytes.
- `null`=0xFFFFFFFFF0000001.
- A port with 32-bit halfwords therefore really halves `mem`.

## Real engine state at `\document` (thesis) [V]

- pdfTeX RSS is 54 MB, 39.5 MB of it non-zero.
- Pages by array: `mem` 4196 (1.07M words), `eqtb` 2462, `hash` 2460, heap 1427, `font_info` 1230, trie 663, `str_pool` 320.
- `eqtb` and `hash` are fully present because `tex.ch:3649,3656` initialise 20 MB on every run.
- lz4 gives 11.2 MB; zstd -1 gives 5.9 MB.

**rstex estimate [I]:**

| Region | Size |
|---|---|
| `mem` | 8.6 MB |
| `font_info` | 4.9 MB |
| `eqtb` + `hash`, lazily initialised | about 1 MB |
| strings | 1.5 MB |
| stacks and PDF tables | under 1 MB |
| **Writable total** | **about 17 MB raw, roughly 5 MB with zstd** |
| Trie, read-only and shared | 2.7 MB |

**Cost on this machine:**
- Save: 2.4 ms foreground memcpy plus 8 ms background `write()`. The write was measured on tmpfs, so real-disk speed is untested.
- Restore: about 3.4 ms `read()` into THP plus about 1.5 ms validation [I]. That is about 5 ms against about 650 ms.

## (a) Format as a `MAP_PRIVATE` file: SOUND WITH FIXES, and "O(1)" is false

**What breaks**

Measured on a warm page cache [V]:

| Operation | Cost |
|---|---|
| `mmap` | 2 µs |
| First read, per 4K page | 0.17 µs |
| First write (CoW), per 4K page | 1.83-2.15 µs |
| `read()` into THP anon, per 4K | 0.78 µs |
| memcpy to THP, per 4K | 0.55 µs |

- **Reads are writes.** `begin_token_list` calls `add_token_ref` (line 8224ff), so calling a macro dirties its page.
- **Lost THP.** CoW pages from a file mapping are 4K. Disabling THP on real pdfTeX costs +108 ms (+7.1%) in interleaved runs [V]. Pointer chasing over 16 MB is 27.6 ns on THP against 39.7 ns on CoW'd file pages [V].
- **File stability.** A mapped file can SIGBUS if truncated, and changes to not-yet-copied pages are unspecified [I]. That also undermines Rust reference guarantees.
- **It is not only a load.** Lines 33595-33604 must still run after load: CLI overrides, `pdf_init_map_file`, `fix_date_and_time`, seed.

**Fix**
- Use a flat, unswapped, ungzipped format (`texmfmp.c:3060,3067` show the gzread and swap).
- Copy writable regions eagerly into 2 MiB-aligned `MADV_HUGEPAGE` anon memory.
- Map only read-only regions (trie).
- Publish by tmp + rename, and key on build-id.
- Make the format simply a snapshot taken at `\dump`, with one loader.

**Cost:** about 1.5 ms instead of 37 ms [I]. That is 55% of hello, 2.7% of the thesis, and irrelevant on a snapshot hit.

## (b) Preamble checkpoint: UNSOUND AS WRITTEN, SOUND WITH FIXES, and the top priority

**What holds up [V]**
- `\document` starts by running a hook (`latex.ltx:9470`).
- `.aux` is read at line 9489 and `\@mainaux` opened at 9492, so the snapshot point precedes both.
- In the thesis, `main.aux` and `main.toc` are only touched after that point.

**State that is not flat POD [V]**
- **File variables:** `input_file[]` (7803), `read_file[]` (11581), `write_file[]` (33757), `log_file`, `pdf_file`, `synctex`.
- **Raw pointer in a "flat" array:** `pdf_font_map: ^fm_entry_ptr` (17955), which points into an AVL tree.
- **36 `^` arrays** in the WEB source.
- **C statics:**
  - `glyph_unicode_tree` (`tounicode.c:27`)
  - font map trees (`mapfile.c`)
  - `colstacks` (`utils.c:1044`, reached via line 11315)
  - `start_time_str` (`utils.c:796,802`; also feeds /ID at 715)
  - `c_stream` (`writezip.c:31`)
  - `image_entry` with `FILE*` and `doc` pointers (`image.h`)
  - the `pdfDocuments` list (`pdftoepdf.cc:161`)
- **Time and seed:** `epochseconds`, `microseconds`, `random_seed`.
- **Absolute paths** are in `str_pool`: a path-length change altered 138 of 329 pool pages.

**What silently gives wrong output**

1. **Clock.** A two-clock run gave exactly 7 differing control sequences out of 55,215 [V]: `\c_sys_{year,month,day,hour,minute}_int`, `\c_sys_timestamp_str` (set in `\everyjob`, `expl3-code.tex:8170-8180`) and `\pgfmath@rnd@z` (`pgfmathfunctions.random.code.tex:31-32`). So naive read-taint marks every LaTeX document. A user `\edef\d{\today}` goes stale with no trace. `hyperxmp`, `datetime`, `datetime2`, `minted` and `pdfx` all contain `\pdfcreationdate`.
2. **Failed lookups.** There are 25 in the thesis preamble, 13 unique, including `hyperref.cfg`, `typearea.cfg`, `english.cfg`. Each probes the output dir first (`texmfmp.c:3473`), then `.`, then disk trees. Dropping a `.cfg` in place must invalidate.
3. **`\write18` and `\input|`.** `\write18{git ... > x.tex}\input{x}` validates the old `x.tex` before the command would regenerate it.
4. **Preamble `\openout`.** `\makeindex` at `latex.ltx:17712`, `filecontents`, glossaries. The stream handle is gone and the file state is wrong.
5. **Log and `.fls` prefixes** are lost. latexmk relies on them, and the user's `.build` has `main.fdb_latexmk`.
6. **`\pdfmapline`, `\pdfglyphtounicode`, `\pdfximage` in the preamble** vanish with the C side: wrong embedding or broken copy-paste, with no error [I].
7. **Key too narrow.** `-jobname`, `-output-directory`, `-draftmode`, `-shell-escape`, cwd, `TEXINPUTS`, `SOURCE_DATE_EPOCH`, `FORCE_SOURCE_DATE` (the last three read via getenv [V]).
8. **"Main file offset" is too narrow.** `\begin{document}` may be in an `\input` file, so every stacked file needs path and offset. The rest of the line is already in `buffer`, so the offset must be at line end and the hash must cover the whole line. Preamble code may also read the main file itself (`\pdfmdfivesum file`, `\pdffiledump`, `\CatchFileDef`).
9. **Preamble errors:** `history` and `error_count` carry over; do not snapshot past them.

**Correct invalidation rule**

A checkpoint records an ordered log of every observation of the outside world. It is valid only if each observation would get the same answer now.

- **Config:** build-id, format hash, relevant CLI flags, cwd, output dir, expanded path specs, relevant env.
- **`lookup(name, kind, path-spec)`:** the resolved path, or NOT FOUND, including shadow probes.
- **Reads:**
  - whole-file hash for closed files
  - prefix hash up to the read position for open files
  - project files: always content-hashed
  - TDS files: size, mtime, inode, plus the `ls-R` stamp
- **Stat-like primitives:** `\pdffilesize`, `\pdffilemoddate`, `\pdfmdfivesum`, `\pdffiledump`, image headers.
- **Volatile reads**, hooked in `scan_something_internal` and the convert primitives:
  - Date read: key on YYYYMMDD.
  - `\time`, seed, deviates, `\pdfcreationdate`, `\pdfelapsedtime`: hard taint.
  - Exception: a whitelist of exactly two sites, expl3 everyjob and the pgfmath seed.
  - An unknown reader gets slower, never wrong.
- **Hard taint, no checkpoint past this point:** `\write18`, pipe input, terminal `\read`, error state, mktex* runs.
- **The run's own writes:** journalled with bytes and re-materialised on restore, including log and `.fls`.

On restore, refresh `\year`..`\time` unless the preamble assigned them, patch the `\c_sys_*` constants, and recompute start time and `/ID`. Reseeding `\pgfmath@rnd@z` is optional: a stale seed makes PGF random numbers repeat, which should be documented.

**Lookup scale in the thesis preamble [V]:** 284 lookups, 139 files, 3.4 MB excluding the format.

**Do not validate by byte-comparing state.** One digit in `\pgfmath@rnd@z` shifted 281,416 `mem` words on 1927 of 4196 pages [V]. Canonical decoding is the valid test tool (`csdiff.py`).

**Extension worth having [I]:** treat checkpoints as general, at any top-level main-file line with `in_open=1`, no group, `nest_ptr=0`. Editing a `\newcommand` then costs about 34 ms, not 650.

A second tier after `begindocument/end`, also keyed on `.aux`, saves 36-46 ms. By then `main.pdf` is already open (`pgfsys-pdftex.def:374-376`, confirmed in the IO log), so it needs the journal.

## (c) Page checkpoints: UNSOUND AS STATED, SOUND WITH FIXES, DEFER

**Safe by construction**

Assuming every global is in the snapshot, these are all fine: `\mark`, held floats, page-builder state, `\pdfrefxform` lists, fragmentation, and even mid-paragraph shipouts. `build_page` runs with a partial hlist on the nest stack (lines 28099, 29941, 30636, 31420) [V]. The rest of the PDF state is POD arrays too, including `pdf_os_buf`; I did not audit every list [I].

**What actually breaks**

1. **"Last page before the edit" is wrong.** TeX reads whole paragraphs before shipping. On 25 of 33 page boundaries, lines already read at shipout k are typeset on page k+1: median 11 lines, 15.2% of body lines [V]. Restoring by page number leaves the reader past the edit, so the edit is silently ignored. Rule: take the latest checkpoint whose read position is at or before the edit, per file.
2. **`.aux` and `.toc` are observations.** A pagination-changing edit altered 18 of 136 aux lines and the toc [V]. All body checkpoints then die, and the second pass must restart at the `\document` tier.
3. **Output offsets.** Every stream needs a journal and a stored path, because `write_open[]` has no name. Truncating the PDF in place endangers viewers and crash safety; use per-checkpoint chunks and rename [I].
4. **`\write18` in the body** taints from that point.

**Cost [V, on pdfTeX's 16-byte words]**

| Window | Avg dirty pages per checkpoint | Whole-document total |
|---|---|---|
| Every page | 1697 (6.9 MB) | 225 MB |
| Every 5 pages | 2354 | 64 MB |
| Every 16 pages | 2771 | 32 MB |

- `mem` is about 1500 of those pages each page, out of about 4300. About 65% of dirty pages actually changed. The union saturates early: 4116 of 5126 pages by the second shipout.
- Expect about half at 8 bytes [I].
- Tracking options:
  - `mprotect`: 6 µs per fault.
  - Soft-dirty: 0.75 µs per fault, but with THP it is 2 MB-granular (8 one-byte writes marked 4096 pages).
  - `fork`: destroyed THP (AnonHugePages 32768 kB to 0) at 2.9 µs per write.

**Fix:** no kernel tricks. Do a plain 2.4 ms memcpy at a shipout at least 100 ms since the last checkpoint (about 2.4% overhead [I]) and dedupe 4K pages in a worker. Enable it only in watch or editor mode.

**Re-sync, the real prize [V]** (pdfTeX is bitwise deterministic: `mem`, `eqtb`, `hash`, `font_info` were identical across runs)

| Edit | Pages changed | State vs. old run |
|---|---|---|
| Same-length word | 17 only | Bitwise identical at next shipout (0/4319 `mem` pages differ) |
| Added sentence, same pagination | 17 only | 989/4334 `mem` pages differ to the end; canonically only 11 hash-slot permutations |
| Inserted paragraph | 17-27 | Text and geometry identical again from page 28; state compared only through page 34, never bitwise equal |

Splicing later pages needs an address-independent fingerprint, such as a running hash over `eqtb` meanings [I]. Do this later.

## (d) Parallel compression and fonts: SOUND, nearly worthless

**Amdahl bound [V, interleaved 15x]:** compression off is −14.7 ms (−1.0%). `-draftmode` (no PDF at all) is −48.5 ms (−3.2%). The end phase including 19 subset fonts is about 42 ms.

**Fix**
- Keep shipout free of file offsets (chunk list, single ordered writer, `/Length` known at write time).
- Stream compression must be finished between objects; `\write` and literal expansion stay on the main thread.
- Join workers before any checkpoint.
- Try a faster deflate before adding threads.

**Cost:** the chunk list is cheap and needed anyway; threads should come last. Re-measure on a graphics-heavy document before building them.

## (e) Mapped file index: SOUND WITH FIXES, small gain

**Gain [V]:** `kpse_find_file` totals 10 ms over 565 calls (0.7%), plus part of the 7.9 ms startup. It matters for hello only.

**What breaks a simple name-to-path index [V]**
- 278 duplicate basenames among tex, sty, cfg, def, cls, ldf and fd files, for example `amsmath.sty` in `tex/latex` and `tex/latex-dev`. Results depend on path-spec order, so key per progname and format.
- Only `!!` trees are database-only. `~/.texlive/*` and `~/texmf` are searched on disk, as are `.` and the output dir.
- 165 failed `.vf` lookups in the body (`ec-lmr12+11.vf`), so negatives must be O(1).

**Also needed [I]:** `.tex` suffix order, `texfonts.map` aliases, mktex* fallback, `openin_any`, `openout_any`.

**Fix:** stamp the index with size and mtime of each `ls-R`, a `texmf.cnf` hash, and env. Fall through to disk for everything else.

**Cost:** about 3 stats per run after one index build.

## (f) Native tikz-cd replacing the package: UNSOUND AS SPECIFIED, the fast-path form is the second priority

**What breaks [V]**
- `\usepackage{tikz-cd}` also provides `\color`, `\definecolor`, `\includegraphics`, `\setkeys`, `\tikzset`, `\pgfmathparse`, `\foreach`, and reports xcolor, graphicx, keyval and tikz as loaded. It adds 11,889 control sequences and 222K words.
- The user's own corpus has 48 tikzcd environments, 75 `xymatrix` matches and no `\tikzset`. Long-tail keys do occur: `from=leftpath`, `"..."{name=...}`, `shorten`, `yshift`, 18 `phantom`.

**What breaks [I]**
- Fallback cannot be decided from the preamble, and PGF cannot be loaded cleanly mid-body.
- Flat snapshots cannot be merged either, because their addresses conflict.
- Box dimensions must match to the sp, or line and page breaks diverge from the oracle.

**Load cost is the wrong target.** Under (b), the 113 ms load vanishes anyway.

**Fix**
- Keep loading the real package.
- At `\begin{tikzcd}`, take a native path only if all options are in a recognised subset and the tikz-cd styles are unmodified. Otherwise run real PGF, per diagram.
- Cells stay real TeX boxes.
- Gate with exact box dimensions and rendered comparison against PGF.
- Separately consider a per-diagram memo keyed on the meaning hashes of everything read. It also covers `xy` and needs no PGF reimplementation, but any diagram with global side effects must be excluded [I].

**Cost:** about 480 ms saved on the thesis, 33% [I]; about 300 ms on LMB [I]. The snapshot grows by about 1.8 MB.

## Literal port or idiomatic rewrite?

**Neither pure form. Use a literal core, a redesigned data layout, and a rewritten outer layer.**

**Literal: the semantic core** (scanner, expansion, `eqtb`, save stack, nodes, line breaking, page builder, math, alignment).
- The source has about 1865 sections, 1900 `@d` macros and 615 gotos [V]. The behaviour is the specification, and trip and etrip give a gate only a faithful port can use.
- It is already index-based, so it snapshots naturally, and its determinism is proven above.
- An idiomatic node and token model needs a serialiser, loses the trip gate, and likely adds allocation cost [I].
- Gotos become labelled loops and blocks. Run on a large stack thread.

**Not literal: layout.**
- 32-bit halfwords and an 8-byte word.
- `null`=0 and zero-as-default regions.
- Lazy `eqtb`/`hash` extras, replacing 20 MB of init.
- u32 indices inside guarded reservations, which removes bounds checks [I].
- Integers and newtypes, not Rust enums or `bool`, in POD state.
- `get_node` and `get_avail` behind a replaceable API, since scatter from `rover` (line 3273) plus refcounts dirties about 35% of `mem` per page.

**Rewrite: everything else.** All I/O and print paths, kpathsea, and the 11,721 lines of C/C++ backend. All backend tables live in POD arenas.

**Speed:** a literal core does not limit the big wins, because (b) and fast diagrams dominate. Hot-path work can follow incrementally, with the determinism oracle as a safety net [I].

## Ranked: design in from day one

1. **`State` versus `Host` split.** `State` is repr(C), POD, with no pointers; a compile-time bound enforces it. Files, threads and caches live in `Host`. All 36 `^` arrays and the former C statics are indexed arenas in `State`.
2. **One `World` interface** for lookup, read, stat, clock, env, shell and terminal, recording observations and taint. No direct syscalls elsewhere. Retrofitting this is the hardest item.
3. **Output journal** for log, `.fls`, `\openout`, SyncTeX, plus PDF as a chunk list with deferred offsets and no seek-back.
4. **Determinism oracle from the first run:** pinned clock, state hash per shipout, fresh-versus-restored byte comparison, trip and etrip, corpus diff against pdflatex on decoded page content.
5. **Layout:** 8-byte word, zero defaults, lazy extras, 2 MiB-aligned writable regions, separate read-only regions, no `fork`, eager load.
6. **Input and stream model:** every input and stream level stores path and logical position, with whole-line semantics preserved.
7. **Volatile-read hooks and safe-point detection** at `macro_call` and `scan_something_internal`.

## Ranked: can wait

1. Generic checkpoint plus preamble snapshot implementation, built on items 1-7 (−45%).
2. tikz-cd fast path or memo (−33%).
3. Flat format, unified with the snapshot loader (−37 ms; matters for hello and cache misses).
4. Hot-path work after trip passes.
5. File index (about 1%; more on hello).
6. Page checkpoints and semantic re-sync, for editor mode only.
7. Parallel compression (≤1%).

## Reproduction files

- `/tmp/claude-1000/-home-site-Projects/8b0542a8-8750-493c-901c-0ccb2c2f69d6/scratchpad/design/attack/bin/supervise.py`
- `/tmp/claude-1000/-home-site-Projects/8b0542a8-8750-493c-901c-0ccb2c2f69d6/scratchpad/design/attack/bin/snaprun.py`
- `/tmp/claude-1000/-home-site-Projects/8b0542a8-8750-493c-901c-0ccb2c2f69d6/scratchpad/design/attack/bin/csdiff.py`
- `/tmp/claude-1000/-home-site-Projects/8b0542a8-8750-493c-901c-0ccb2c2f69d6/scratchpad/design/attack/bin/iolog.c`
- `/tmp/claude-1000/-home-site-Projects/8b0542a8-8750-493c-901c-0ccb2c2f69d6/scratchpad/design/attack/bench/b.c`
- `/tmp/claude-1000/-home-site-Projects/8b0542a8-8750-493c-901c-0ccb2c2f69d6/scratchpad/design/attack/bench/thp.c`