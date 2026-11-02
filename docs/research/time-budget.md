# pdflatex time breakdown (pdfTeX 1.40.29, Ryzen 7 7730U)

**The main result:** about 90% of the time is macro interpretation, and real typesetting plus PDF output is under 5%. On the thesis, `line_break`, `hpack`, `mlist` and the page builder take about 12 ms, and all PDF output takes about 39 ms, out of 1380 ms.

**Tools:** `strace` and `perf` are absent, but gdb and `perf_event_open` both work. I wrote an `LD_PRELOAD` profiler with these parts:
- A SIGPROF sampler with libgcc backtraces.
- A walk of TeX's `input_stack`, giving macro-level profiles.
- Interposed libc and zlib calls for a file timeline.
- Hardware counters, and debug-register breakpoints for exact call counts.

Symbols came from debuginfod.archlinux.org, stored in scratch.

**Noise:** other agents pushed load average up to 8 during the runs. Phase splits and deltas therefore rest on user-mode cycle and instruction counters, which repeat well, plus min-of-N wall times. Sampler overhead is about 2.6%.

## Budget per phase (ms)

Columns reconcile to the unprofiled wall time, ±1 ms for hello and about ±10 ms for thesis and LMB.

| Phase | hello | thesis | LMB |
|---|---|---|---|
| exec + ld.so to `main` | 3.5 | 4 | 4 |
| kpathsea: `texmf.cnf` + `ls-R` (0.8 MB) | 7 | 7.5 | 7.5 |
| fmt undump (inflate 28.5, rest 8.5) | 35.5 | 37 | 37.5 |
| preamble | 10.5 | 590 | 650 |
| body: tikzcd diagrams | 0 | ~520 | ~490 |
| body: other macro work | 1.4 | ~165 | ~125 |
| body: C typesetting (`line_break`, `hpack`, `mlist`, page builder) | 0.1 | ~12 | ~16 |
| shipout, excluding deflate | 4.5 | 17 | 19 |
| font subset + embed | 0.6 | 20 | 24.5 |
| compression (deflate, wall) | 0.3 | 17 | 24 |
| final PDF write (objects, xref, fwrite) | 0.5 | 1.5 | 1.5 |
| process exit | 1.5 | 2.2 | 2.4 |
| **total (unprofiled wall, min–median)** | **65–66** | **1381–1400** | **1396–1408** |

- **Shipout on hello:** almost all of it is `pdftex.map` parsing (4.5 ms).
- **Shipout on thesis and LMB:** it includes the map parse (4.5), VF (3) and the JPEG copy (2.1, thesis only).
- **Font subsetting:** `writet1` costs about 0.95 ms per font (19 fonts thesis, 24 LMB).
- **Compression:** total deflate splits into page streams (10.4 thesis, 15.8 LMB) and object streams plus fonts (7, 8.5).
- **Phase boundary:** the preamble ends at the first read of `\jobname.aux`, so about 25 ms of `\AtBeginDocument` work counts as body.
- **Per page, excluding diagrams:** 4.3 ms median on the thesis and 3.1 ms on LMB. Most of that is one-off microtype font setup, so steady text costs about 1.5 ms per page (inferred from the microtype delta below).
- **System time:** hello spends 13 ms in sys (5148 minor faults). Thesis and LMB spend about 28 ms.
- **`tikzcd` range:** the ~520 and ~490 figures are midpoints. Cycle deltas give 507 and 478, instruction-scaled deltas give 555 and 508, and hook timing gives 562 and 517 (with hook overhead).

### Preamble by package (inclusive ms, min of 7, hooks add about 3%)

| Package | thesis | LMB |
|---|---|---|
| `scrartcl`/`scrbook` | **318** | **360** |
| `tikz-cd` (with tikz and pgf) | 118 | 128 |
| `hyperref` | 64 | 64 |
| `bookmark` | 14.5 | 20 |
| `microtype` | 21 | 18 |
| `babel` | 19 | 20 |
| `amsthm` | 13 | 17 |
| `xcolor` | 13 | 13 |
| `nameref` | 12 | 12 |
| `amsmath` | 11 | 12 |

- **KOMA class:** it alone is 23% of the thesis. Macro-level samples put about 22% of the preamble in `\DeclareSectionCommand`, 10% in `\DeclareNewTOC` and 12% in `hook_gput_code:nnn`.
- **Body hot spots (thesis):** the pgfmath parser is about 30% of the body, `\MT@setupfont` about 12% and `\@outputpage` about 8%.

### On/off deltas (ms)

| Change | thesis | LMB |
|---|---|---|
| no microtype | −14 pre, −115 body | −20 pre, −130 body |
| no hyperref | −85 pre, −39 body | −91 pre, −10 body |
| no babel | −24 | −24 |
| no tikzcd package load | −100 to −116 | −98 to −110 |
| `\pdfcompresslevel=0` and `\pdfobjcompresslevel=0` (saving is the deflate time above) | −17, PDF +39% | −24, PDF +108% |
| `-draftmode` | −39 | −51 |
| `-synctex=1` | +45 | +50 |

- **Pagination:** removing the diagrams changes the thesis from 34 to 32 pages.
- **`tikzcd` scaling (N=1 to 100):** exactly linear at 22.9 ms and 283 M instructions per 4-arrow square. A triangle costs 18.7 ms and a single arrow 7.3 ms.

## Top 5 opportunities (ms saved, thesis / LMB)

Items 1–3 overlap, so they don't add up.

1. **Token engine: about 1250 ms is in scope on both documents.**
   - Inferred savings on the thesis are about 830 ms at 3× and about 1000 ms at 5×. The factors are assumptions; the base is measured.
   - Evidence from the thesis is in the counts and hot-spot lists below.
2. **Preamble snapshot: −571 / −625 ms, measured.**
   - This used pdfTeX's own `mylatexformat` (0 errors, same page count).
   - The dump is 5.9 MB gzipped, 29.4 MB raw. A raw fmt reaches −617 on the thesis.
3. **Native `commute` replacing tikz-cd: −480 to −520 / −480 to −510 ms in the body.** It also removes −100 to −116 / −98 to −110 ms of package load, which overlaps item 2.
4. **microtype per-font setup: −115 / −130 ms in the body.** `\MT@setupfont` runs on each font's first selection, so tables could be cached natively. hyperref adds −39 / −10 ms in the body.
5. **Startup: about 48 ms on every document (73% of hello).**
   - A gunzipped fmt alone took hello from 74 to 49 ms under the same conditions.
   - Other measured floors: `-ini` with an immediate `\end` is 20 ms, and a static no-op binary is 1.0 ms.
   - An mmap image plus a prebuilt file index should save about 45 ms (inferred).

**After the top 5:**
- The PDF tail is worth about 40–50 ms, through parallel deflate and a subset cache.
- Items 2 and 3 together leave about 280 / 260 ms. Adding 1, 4 and 5 suggests about 100–130 ms (inferred).

**Not worth optimizing:**
- Hyphenation: under 3 ms per document (inferred from +11 µs per 100 words).
- Whole line breaking: 8 / 12 ms.
- Math: 2 ms.
- TFM loading: about 1 ms.
- Kpathsea lookups after startup: about 11 ms.

### Call counts and CPU counters

| Thesis counter | Value |
|---|---|
| `get_next` | 119.6 M calls (LMB 118.4 M) |
| `getavail` | 41.3 M, more with the inlined fast path |
| `end_token_list` | 12.5 M |
| `begin_token_list` | 8.6 M |
| `expand` | 5.8 M |
| `delete_token_ref` | 5.05 M |
| `macro_call` | 3.8 M |
| `prefixed_command` | 1.95 M |
| `conditional` | 1.39 M |
| `id_lookup` | 548 k |
| `hpack` / `mlist_to_hlist` / `line_break` / `pdf_ship_out` | 19.6 k / 6.1 k / 418 / 34 |
| instructions / cycles | 15.98 G / 5.93 G (IPC 2.69) |
| branch misses | 1.07% |
| L1d misses | 1.9% (114 M) |
| dTLB misses | 207 k (huge pages would not help) |

### Where the engine time goes (thesis)

- **Cost per token:** 11.5 ns and 133 instructions per `get_next` call.
- **`get_next` self: 562 ms.** The `mem[loc]` fetch is 222 ms of it and the `eqtb[cs]` fetch 68 ms. Reading from files is under 5%.
- **`macro_call` self: 170 ms.**
- **List-cell churn: 210 ms** (`end_token_list` + `delete_token_ref` + `getavail`). 43% of `end_token_list` is one pointer-walk instruction in the inlined `flush_list`.
- **`id_lookup`: 47 ms.** `hash_prime` is 8501 for 57.5k names, so chains are long. The `csname_lookup_existing` micro-benchmark costs 1359 ns at about 130k names.
- **Word size:** `memoryword` is 16 bytes in this build, verified in DWARF and by `shl $4` in `getnext`. Each list token therefore takes 16 bytes, so compact tokens should reduce cache misses (inferred).
- **Format build (5.08 s):** also almost all macro work. Hyphenation patterns take only about 35 ms.

## Micro-benchmark baseline

Run it with `python3 /home/site/Projects/rstex/bench/micro/run.py --engine CMD --compare /home/site/Projects/rstex/bench/micro/baseline-pdftex-1.40.29.json`. There are 151 benchmarks in 12 files plus `benchlib.tex`.

Values are min of 5 runs. Net ns subtracts the loop overhead, and the empty loop itself is 60 M ops/s.

| Benchmark | ops/s | net ns | Benchmark | ops/s | net ns |
|---|---|---|---|---|---|
| `macro_0arg` | 32.8 M | 14 | `hbox_80char_lm` | 423 k | 2350 |
| `macro_1arg` | 24.5 M | 24 | `para_firstpass_lm` (100 words) | 21.9 k | 45.6 k |
| `macro_3arg` | 9.74 M | 86 | `para_hyphenate_lm` | 17.6 k | 56.9 k |
| `macro_100tok_arg` | 1.55 M | 629 | `para_microtype_lm` | 9.08 k | 110 k |
| `edef_3macro` | 8.42 M | 102 | `math_a_plus_b` | 970 k | 1014 |
| `ifx_true` | 18.5 M | 38 | `math_typical_inline` | 540 k | 1836 |
| `ifnum` | 11.2 M | 73 | `math_display` | 290 k | 3427 |
| `iffalse_skip_100tok` | 2.48 M | 387 | `ship_5para_c0` | 5.49 k | 182 k |
| `count_advance` | 11.9 M | 68 | `ship_5para_c9` | 4.61 k | 217 k |
| `numexpr_simple` | 5.13 M | 178 | `int_step_inline` | 1.94 M | 514 |
| `numexpr_reg` | 3.69 M | 254 | `seq_map_inline` | 18.1 M | 55 |
| `dimexpr` | 2.49 M | 384 | `prop_put` | 485 k | 2045 |
| `csname_create_let` | 729 k | 1355 | `keys_set` | 21.1 k | 47.3 k |
| `csname_fixed_long` | 2.37 M | 405 | `xparse_call` | 28.1 k | 35.6 k |
| `def_10tok` | 8.54 M | 100 | `fp_eval_mul` | 24.5 k | 40.8 k |
| `let` | 28.5 M | 18.5 | `regex_match` | 4.23 k | 236 k |
| `begingroup_endgroup` | 30.5 M | 16 | `usehook_empty` | 1.40 M | 699 |
| `group_5_assign` | 2.51 M | 383 | `latex_par_word` | 153 k | 6520 |
| `setbox_empty_hbox` | 6.78 M | 131 | `pgfmathparse_add` | 25.1 k | 39.8 k |
| `toks_append` | 818 k | 1207 | `pgfkeys_set_value` | 150 k | 6667 |
| `unexpanded_edef` | 1.34 M | 731 | `pgfpoint` | 25.2 k | 39.7 k |
| `openin_found` | 30.7 k | 32.6 k | `tikz_node` | 723 | 1.38 M |
| `openin_missing` | 25.5 k | 39.2 k | `tikzcd_1arrow` | 135 | 7.42 M |
| `font_load_new_scale` | 26.9 k | 37.2 k | `tikzcd_square` | 44 | 22.5 M |
| `input_file_1000lines` (per line) | 795 k | 1258 | `par_and_page_builder` | 17.6 k | 56.9 k |

- **Plain pdfTeX:** runs of files 01, 06, 07, 08 and 12 gave matching typesetting numbers (math about 400 ns cheaper). Files 02–05 and 11 are written to work there too but were not run. Files 09 and 10 need LaTeX.
- **Primitives in the LaTeX format:** `benchlib` restores primitive `\par` and clears `\everypar`, because the kernel's wrappers cost about 110 µs per paragraph before `\begin{document}`, against 6.5 µs inside a document and 0.5 µs for the primitive.

## Files and notes

- `/home/site/Projects/rstex/bench/micro/` holds the benchmarks, plus `run.py` and the baseline JSON. Those two are not `.tex`, but they sit in the directory you allowed. Nothing is committed.
- Your documents are unchanged (mtimes match, no new files in those directories).
- The system-wide `/etc/ld.so.preload` loads a library into every process, so it is part of all these baselines.

Profiler and reports in scratch:
- /tmp/claude-1000/-home-site-Projects/8b0542a8-8750-493c-901c-0ccb2c2f69d6/scratchpad/design/timing/prof.c
- /tmp/claude-1000/-home-site-Projects/8b0542a8-8750-493c-901c-0ccb2c2f69d6/scratchpad/design/timing/: `rep-*.txt` and `rept-*.txt`, one pair per document

The `rep-*` files are C-level profiles and the `rept-*` files are macro-level profiles. The `rept-*` runs were taken under load, so use their percentages rather than absolute ms.