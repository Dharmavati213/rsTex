# rstex architecture proposal (independent)

`[V]` means measured or read on this machine in this session. `[I]` means inferred or estimated. Experiments are in `/tmp/claude-1000/-home-site-Projects/8b0542a8-8750-493c-901c-0ccb2c2f69d6/scratchpad/design/arch/`. I wrote nothing into the project.

## 0. The measurements that drive the design

**Thesis time budget [V]** (min of 5 runs; 1474 ms this session against your 1380 ms)

| Slice | ms | Share | How derived |
|---|---|---|---|
| Preamble | ~690 | 47% | preamble-only variant: 687 |
| 20 `tikzcd` bodies | 562 | 38% | normal minus diagrams removed (912); 28 ms each |
| All other prose and math (31 pp) | ~210 | 14% | body from dumped format: 268 minus 60 load |
| Whole PDF backend | 33 | 2.2% | normal minus `-draftmode` (1441) |

`\pdfcompresslevel=0` saves only 14 ms. Loading the tikz-cd package accounts for about 127 ms of the preamble, and microtype for about 152 ms of the total.

**Profile by function [V]** (SIGPROF, 18k samples over 12 runs; LMB is within 1.5 points on every line)

| Function | Share |
|---|---|
| get_next | 39.2% |
| macro_call | 11.5% |
| end_token_list | 7.6% |
| delete_token_ref | 3.9% |
| expand | 3.8% |
| id_lookup | 3.8% |
| libz | 3.1% |
| get_avail | 2.8% |
| get_x_token | 2.3% |
| begin_token_list | 1.8% |
| conditional | 1.6% |
| pass_text | 1.6% |
| scan_general_text | 1.3% |
| scan_toks | 1.2% |
| back_input | 1.0% |

- The token machine is about 84% of runtime.
- try_break, hpack, line_break, mlist_to_hlist and pdf_hlist_out together are 0.23%.
- In the prose-only body, line_break is 0.29% and hpack 0.19%.

**Exact counts [V]** (hardware breakpoints via `perf_event_open`)

- get_next: 119.3M calls. macro_call: 3.8M. end_token_list: 12.4M. id_lookup: 546k.
- Callers of get_next:

| Caller | Calls | Share |
|---|---|---|
| macro_call argument scan | 42.9M | 36% |
| pass_text | 25.2M | 21% |
| get_x_token | 22.3M | 19% |
| scan_general_text | 11.3M | 9.5% |
| Other | 17.6M | 15% |

- About 67% of token fetches are copy or skip loops that do not need full get_next semantics.
- Hardware counters: 8.8G instructions, IPC 2.53, 1.08% branch miss rate, 62.9M L1d misses, 12.9M `cache-misses`.
- 120k dTLB misses, which is irrelevant.

**Inside get_next [V]**

- The load of `mem[loc]` takes 9.7% of all pdftex samples and the eqtb load 4.0%.
- The flush_list pointer walks take 6.3%.
- Prologue and epilogue (six pushes, stack canary) are 18.5% of get_next's samples, about 7% of total runtime.
- The file-reading branch is about 1% of get_next's samples: 3.67 MB of TeX source against 119M tokens.

**Token memory in the oracle [V]**

- This pdftex uses a 16-byte memory word with 64-bit halfwords (disassembly and fmt layout). Upstream may differ, so do not extrapolate from it [I].
- The thesis preamble holds 1.05M tokens, or 16.7 MB.
- Only 45.5% of links point to an adjacent word (58.5% when weighted by real calls). 12.7% jump more than 64 KB.
- The hash has 53,061 control sequences over `hash_prime`=8501: mean chain 6.26, mean probes per hit 4.73, max 41, about 101 ns per lookup.

**Consequences**

Three levers matter: preamble snapshot, token representation with capture loops, and native diagrams. Parallelism and typesetting algorithms are not levers. Translate those literally.

## 1. Organising the translation

**Crate layout.** One engine crate plus leaf crates.

- The WEB call graph is effectively one strongly connected component (for example get_next, expand, macro_call, scan_*, error). The crate DAG forbids cycles.
- Synthetic 45k-line single crate with one `impl` per file [V]: `cargo check` 10 s cold and 2.6 s incremental; opt-level=1 incremental 2.8 s; fat LTO 14.7 s. Real code is likely 2-3x slower [I].

**M0, one agent before fan-out**

1. Fetch the missing change files (see risk 1) and `tie` them into one merged WEB.
2. Write `tools/webgen.py`, which generates:
   - constants and node accessors from the `@d` macros;
   - one per-module `Vars` struct per part, from the `@<Glob...@>` sections;
   - stubs for all 590 procedures, each with signature, `todo!("§N")` and the expanded Pascal body as a comment.

   It compiles from minute one.
3. Build the oracle harness.

**Conflict avoidance**

- Each file has one owner.
- Generated `state.rs` is frozen after M0. Owners add fields only to their own `Vars`.
- Each agent gets its own git worktree, its own `CARGO_TARGET_DIR` and `-j4`.

**Global state**

- One `#[repr(C)] struct Tex`, plain data with inline fixed-capacity arrays, placed in an anonymous mmap reservation (8 GiB reserve costs 3 µs [V]).
- All code is `impl Tex { fn x(&mut self) }`. No `static mut`, `RefCell`, lifetimes or `Rc`. Pointers are `u32` indices.
- Asm check [V]: inline arrays compile to `movl disp32(%rdi,%rcx,4)` with an immediate bound. `Box<[T]>` adds a base load and a length load per access.
- Hot fields go in the first cache line. Never move `Tex` by value.

**Naming**

- WEB names verbatim, with a `/// §N` comment on each function. Keywords get a small rename table (`type` becomes `ty`, `box` becomes `box_reg`).
- `link(p)` becomes `self.link(p)`. `link(p):=q` becomes `self.set_link(p,q)`.
- Inline `@<...@>` modules unless they are shared or cold, and leave a `// §N` marker.

**Gotos** (599 [V, crude classifier])

| Pattern | Rust form |
|---|---|
| Forward `done`/`exit` | `'done: { ... break 'done }` |
| Backward `restart`/`continue` | `'restart: loop { ... }` |
| Irreducible cases (main_control's lig/kern loop, hyphenate/reconstitute) | `let mut pc = Pc::X; loop { match pc {...} }` |

**Fatal errors**

- `overflow`, `confusion`, `fatal_error` and `succumb` return `!` and reach `jump_out() -> !`, which closes files and calls `process::exit`.
- `panic="abort"`. No `Result` in the engine. Recoverable `error()` stays an ordinary call and return, as in WEB.
- A daemon forks per run (fork at 40 MiB RSS takes 70 µs [V]).

**Acceptance gates**

1. trip and etrip logs match, with memory-usage lines filtered.
2. Line-identical `.log` on the corpus with tracing on.
3. Identical decompressed content streams and object numbering.
4. `pdftoppm` pixel diff.

## 2. In-memory layout

| Structure | Decision | Evidence |
|---|---|---|
| mem word | 8 bytes: `{info:u32, link:u32}`, `{b0:u16, b1:u16, link}`, `i32`, `f64`, `4×u16`. Set `mem_bot=1` so `null=0`. Pointers `u32` | `i32` needs `movslq`, `u32` is free [V asm]. Quarterword must be u16: 225 and 277 fonts in the logs [V]. Keep f64 glue ratio so rounding matches [I] |
| Token lists | Leave `mem`. Contiguous `u32` heap with blocks `[hdr][t0..tn][0]`, size-class allocator, O(1) free | Real-trace replay (3.1M calls, real layout), unchecked: 16B linked 2.46, 8B linked 2.05, 4B contiguous 1.36 ns/token [V]. Footprint 16.7 MB becomes about 4.2 MB. Removes most of the 14.4% in end_token_list, delete_token_ref and get_avail [I] |
| Sentinel 0 | Terminates lists | 0 occurrences in 1,044,066 real tokens [V]; cmd 0 char tokens cannot occur [I] |
| Macro header | Refcount, plus param count and body offset | Skips the match-part walk when m=0 [I] |
| eqtb | Keep flat index, 8-byte entries. Allocate region 2 sequentially in a large virtual range, replacing web2c's `hash_extra` | Eliminates scattered first-probe slots; eqtb load is 4% of samples [V] |
| Hash | Open addressing, entry `hash32<<32 \| p`. Never deletes, cached hash, word-at-a-time hashing | 4.73 probes and 101 ns today [V]; about 3% gain [I]. Not semantically observable [I, trip will confirm] |
| String pool | Keep (`u8` plus `u32` offsets). Messages become `&'static [u8]`, no pool file | Not hot |
| lo-mem nodes | Keep `get_node`/`free_node` verbatim. Fixed lo/hi regions make `is_char_node` a constant compare | `get_node` 0.69% [V] |
| font_info, trie, save_stack, nest | Keep | Not hot |

Use whole-word stores on freshly built nodes. Two 4-byte stores followed by an 8-byte load defeats store forwarding [I, consistent with the churn bench].

The token API must be there from day one, because retrofitting touches the same sites later. Line counts in pdftex.web [V]: `store_new_token` 26, `temp_head` 73, `flush_list` 29, `delete_token_ref` 59, `show_token_list`/`token_show` 35. The only mid-list mutation I found by reading is macro_call's brace strip (§Tidy up), which becomes truncate plus advance.

**False economies (do not build)**

- Catcode shadow table or SIMD tokenizer (file branch about 1%).
- Per-font dense metric tables, node SoA, tuned hpack or line_break.
- Threaded dispatch for main_control (0.57%).
- Single-token pushback register (back_input 1%).
- Huge pages.
- `target-cpu=native` (1.37 vs 1.36 ns/token [V]).
- Masked indexing: 2.41, worse than bounds-checked 2.27, against unchecked 2.05 on the linked walk [V]. The mask adds a data dependency while a check is only a predicted branch.
- Display-list offload of ship_out (pdf_hlist_out 0.04%).

## 3. Parallelism

Parallelism is worth at most about 25 ms of 1474 ms [I]. Treat it as cheap plumbing.

- **Cannot be parallel:** expansion, execution, page building, ship_out traversal. `\write` expands macros at shipout, `\pdfsavepos` feeds back into TeX, and line_break synchronously feeds `\prevgraf` and the page builder. It is also under 0.3%.
- **Can be parallel:** deflate per stream, Type 1 parse and subset per font, image embedding.

**Threads**

- The engine runs on one dedicated thread with a 1 GiB stack (spawn and join 123 µs [V]), because TeX recurses deeply.
- 3 workers start lazily at the first shipout.
- The main thread assigns object numbers and output slots, and hands whole byte buffers to the workers.
- An ordered writer appends completed slots, so output is deterministic regardless of timing.
- Use `crossbeam-channel`, not rayon.

## 4. Hot paths

**get_next**

Split it into an `#[inline(always)]` leaf holding only the token-list branch, and `#[inline(never)] get_next_slow` for end of list, file state, `\noexpand` and outer checks. Inline the leaf into get_x_token, get_token, pass_text and x_token only. Keep `cur_cmd` and `cur_chr` as fields; LLVM elides reloads under `&mut self`. Returning tuples is a false economy.

**Capture loops are the biggest new idea here.** The group-contribution loop (§Contribute an entire group) and scan_general_text never need get_next when the input is a token list:

```rust
loop { let t = self.toks[i];
  if t < 0x600 { match t >> 8 {
      1 => depth += 1,
      2 => { depth -= 1; if depth == 0 { break } }
      3 => {}
      _ => return self.capture_slow(..) } }   // 0 end of list, 4 &, 5 out_param
  else if t >= CS_FLAG && self.special_cs(t) { return self.capture_slow(..) }
  out.push(t); i += 1; }
self.align_state += opens - closes;
```

- `special_cs` covers `\par` when non-long, the `\noexpand` token, `\endwrite`, and any user `\outer`.
- `\endwrite` is the only outer cs in the thesis preamble format [V], so a sticky `any_user_outer` flag makes this a few compares.
- This replaces about 54M get_next plus store_new_token paths with a few instructions per token.
- pass_text gets the same treatment. Char tokens (78% of static tokens [V]) need no eqtb load, cs tokens need one, and out_param falls back to slow.
- Implement this only after trip passes on the generic path, and keep a `slow_capture` feature for differential testing.

**The rest of the machine**

- end_token_list and begin_token_list: 24-byte frames, O(1) block free.
- Move `dyn_used` and similar statistics behind a feature. The oracle does `addq` on every get_avail [V].
- scan_*: benefits automatically through the inlined fast path.
- main_control: plain `match` on `abs(mode)+cmd`.
- hpack, line_break, mlist: literal translation.

**Bounds checks and unsafe**

- `raw.rs` holds all unchecked accessors. Every other module has `#![forbid(unsafe_code)]`.
- Release builds are unchecked. `--features checked` plus overflow checks runs in CI over the corpus.
- Checks cost 0-10% in the tightest loop [V, noisy]. The formal unsoundness of safe accessors is an accepted, documented trade.

**Build**

```toml
[profile.release]
opt-level=3
lto="fat"
codegen-units=1
panic="abort"
overflow-checks=false
debug="line-tables-only"
```

- Add `-C target-feature=+crt-static` (0.38 vs 0.75 ms exec [V]).
- PGO: system `llvm-profdata` (LLVM 23.1.3) rejects rustc's raw profile (LLVM 22.1.8) [V]. Run `rustup component add llvm-tools`; I did not install it. Train on thesis, LMB, tikzcd and hello. Expect 10-25% [I].
- `llvm-bolt` is absent [V]. Skip it.
- Allocator: keep the engine free of malloc after init, keep the system allocator, reuse worker buffers.
- Profiling: copy `prof/shim.c` and `prof/pstat.c` into `tools/`. They work without perf [V].

## 5. PDF backend

**Compression choice: `libdeflater` 1.26** (C via `cc`, no cmake, whole-buffer API, which is all we need).

Corpus: 158 real streams, 1.83 MB, from both documents, single-threaded [V].

| Codec / level | ms | Output bytes |
|---|---|---|
| libdeflate 1 | 7.6 | 954,497 |
| libdeflate 6 | 14.2 | 924,349 |
| libdeflate 9 | 23.8 | 919,684 |
| libdeflate 12 | 212 | 912,861 |
| zlib-rs 6 | 18.9 | 929,066 |
| zlib-rs 1 | 7.3 | 1,096,260 |
| miniz_oxide 6 | 27.1 | 928,491 |
| system zlib 9 (via Python) | ~45 | 927,061 |

- libdeflate 6 beats zlib 9 on both size and speed.
- zlib-rs is an acceptable pure-Rust swap if a C dependency is ever vetoed. This decision is worth only single-digit milliseconds.
- zlib-ng could not be tested: `libz-ng-sys` needs cmake, which is absent [V].

**Levels and formats**

- Content and ObjStm at level 6. FontFile at level 1, because eexec-encrypted fonts stay at 95.2% of size at zlib 9 [V].
- JPEG passes through. PNG copies IDAT where possible, as writepng.c does.

**Object strategy**

- Keep pdfTeX's numbering so oracle diffs work.
- ObjStm cap as observed (368 objects in 4 streams [V]) plus an xref stream.
- Honour `SOURCE_DATE_EPOCH`.

**Fonts**

- Port writet1.c semantics and the MD5 subset tag.
- Add a content-addressed subset cache keyed by pfb identity, encoding and glyph set. T1 is about 6.5 ms per document [V].
- Fonts are 633 KB of 924 KB output [V], so CFF conversion is a possible later size win, not a speed priority.

**Ordering and PDF inclusion**

- TTF, PK and JBIG2 come last. Both documents embed only Type 1 pfb files in the log's font list [V].
- PDF inclusion uses a small own reader (xref streams, ObjStm, filters) rather than `lopdf` [opinion].

## 6. Caching, incremental compilation, commute

1. **Base image.** The format is the raw `Tex` memory image with page-aligned sections mapped `MAP_PRIVATE|MAP_FIXED` over the reservation.
   - mmap 16 µs, read fault 241 ns/page, COW 2 µs/page, against `read()` of 28.9 MB at 6.6 ms [V, tmpfs].
   - Today the fmt is 17.1 MB raw and gzip-loaded; `\stop` takes 46 ms [V].
2. **Automatic preamble snapshot.** This is the largest single win.
   - Proof of concept [V]: pdfTeX's own `\dump` of the real thesis preamble (tikz-cd, hyperref, babel, microtype) gives a PDF identical after normalising dates, ID, xref and lengths, except for one extra `0 g 0 G`.
   - That line is probably colorstack C-side state that pdfTeX's `\dump` does not save [I]. This is the bug class a whole-image snapshot avoids. It saved 568 ms even with gzip. Not tested on LMB.
   - Take the snapshot immediately before `\begin{document}`, so `.aux` is re-read.
   - Key: engine build, preamble bytes, jobname, output directory, relevant environment, calendar day.
   - Validate by stat of every file read and every recorded negative lookup (211 files in the thesis [V]; about 0.2 ms [I]).
   - Store the log prefix and replay it.
   - Bail out on `\write18`, open read or write handles, or `\immediate\openout`.
3. **Rule: if it survives a snapshot, it lives in the image.** Leaf-crate persistent state (map lines, ToUnicode, colorstacks) uses `PodVec` in the image arena, never `HashMap`.
4. **kpathsea replacement.** Binary path index, mmapped, keyed on ls-R and texmf.cnf (kpsewhich is 8 ms [V]). Keep kpathsea's search order. Fall back to `kpsewhich` on a miss.
5. **`--until-stable`.** Rerun from the snapshot until the `.aux` hash is stable.
6. **Incremental, last phase.** `--watch` forks a checkpoint at each shipout and resumes from the last checkpoint before the edit, reusing the prefix only. Suffix convergence (Zobrist hash over eqtb) waits until there is demand. Plain-data state keeps this possible at no cost now.
7. **commute is first-class, not an extra.**
   - After the snapshot, diagrams are 73% of the remaining body time [V arithmetic].
   - Leaf crate plus a thin `.sty`, with one generic primitive. TeX typesets the cells into boxes, Rust computes layout and arrows, and output is `\pdfliteral`.
   - Remap `tikz-cd.sty` only when the document needs nothing else from TikZ.
   - On an unsupported key, restart with real tikz-cd and record that in the manifest.

**Expected results [I]**

| Scenario | pdfTeX | rstex |
|---|---|---|
| hello | 66 ms | ~8 ms cold, ~2 ms warm |
| Thesis, cold, real tikz-cd | 1474 ms | ~600 ms |
| Thesis, snapshot, real tikz-cd | n/a | ~330 ms |
| Thesis, snapshot and commute | n/a | ~110 ms |

## 7. Crates, modules, ownership

WEB line counts come from a crude classifier [V]. Rust counts are estimates [I]. A0 is the M0 agent, then the integrator.

**`crates/engine`, about 41.7k lines**

| Modules | WEB parts | WEB lines | Rust | Owner |
|---|---|---|---|---|
| `state`, `raw`, accessors (mostly generated) | 8, 11, node `@d` | n/a | 3,000 | A0 |
| `print`, `errors`, `strings`, `arith` | 1-7b | 1,470 | 1,800 | A1 |
| `web2c` (tex.ch and other change-file additions) | change files | n/a | 1,000 | A1 |
| `files`, `tfm` | 29-30 | 866 | 1,100 | A1 |
| `mem`, `nodes`, `show`, `copy` | 9, 10, 12-14 | ~1,140 with 8, 11 | 1,400 | A2 |
| `pack` | 33 | 834 | 1,000 | A2 |
| `eqtb`, `hash`, `save`, `nest` | 15-19 | 2,126 | 2,600 | A3 |
| `toks`, `input`, `getnext`, `expand`, `macrocall` | 20-25 | 1,322 | 2,300 | A4 (strongest) |
| `scan` | 26 | 931 | 1,100 | A5 |
| `buildtoks`, `cond` | 27-28 | 1,212 | 1,500 | A5 |
| `mathdata`, `mlist` | 34-36 | 1,258 | 1,500 | A6 |
| `align` | 37 | 581 | 700 | A7 |
| `linebreak` | 38-39 | 1,427 | 1,700 | A7 |
| `hyph` | 40-43 | 952 | 1,150 | A7 |
| `page` | 44-45 | 903 | 1,100 | A8 |
| `control`, `build`, `mathbuild` | 46-48 | 2,291 | 2,800 | A8 |
| `prefixed` | 49 | 1,088 | 1,300 | A9 |
| `image`, `main` | 50-52 | 745 | 700 | A9 |
| `ext` | 53 | 3,273 | 3,900 | A10 |
| `etex` | 53a | 2,499 | 3,000 | A11 |
| `dvi` (needed for trip) | 31-32 | 786 | 900 | A12 |
| `pdfout`, `pdfobj` | 32a-d | 1,945 | 2,300 | A12 |
| `pdffont`, `pdfship` | 32e-f | 3,201 | 3,800 | A12 |

**Leaf crates, about 21k lines** (never call back into the engine)

| Crate | Source | Rust | Owner |
|---|---|---|---|
| `rstex-t1` | writet1.c | 1,500 | A13 |
| `rstex-fontmap` | mapfile, writeenc, writefont, tounicode, subfont | 2,200 | A13 |
| `rstex-pdfio` | writer, ObjStm, xref, deflate pool | 900 | A13 |
| `rstex-fontx` (late) | writettf, writet3, pkin | 2,000 | A13 |
| `rstex-kpse` | kpathsea subset, binary index | 1,200 | A14 |
| `rstex-cache` | snapshots, manifest, subset cache | 700 | A14 |
| `rstex` (CLI) | texmfmp.c subset | 500 | A14 |
| `rstex-img` | writeimg, png, jpg (jbig2 +700 late) | 1,400 | A14 |
| `rstex-epdf` | pdftoepdf.cc, own reader | 1,800 | A14 |
| `rstex-util` | md5, `\pdfmatch`, dates, escapes | 800 | A14 |
| `rstex-synctex` | synctex.c | 900 | A14 |
| `commute` | new | 4,000 | A15 |
| `xtask` oracle, `tools/webgen.py` | new | 2,300 | A0 |

Total is about 62.5k lines.

## 8. Top 3 risks

**1. The refs are not the source of the oracle [V].**

- The upstream `pdftex.am` tie list includes these files missing from `refs/`: `tracingstacklevels.ch`, `partoken-102.ch`, `partoken.ch`, `locnull-optimize.ch`, `showstream.ch`, `zlib-fmt.ch`, `enctex*.ch`, `unbalanced-braces.ch`, `char-warning-pdftex.ch`, `tex-binpool.ch`, plus the synctex change files.
- The installed binary contains the strings `partokenname`, `partokencontext`, `showstream`, `tracingstacklevels` and `mubyte`; none appear in refs' WEB or change files. latex.ltx references `\tracingstacklevels`, `\partokencontext` and `\mubyte`.
- The oracle's get_next also reads a `writenoexpanding` global that is absent from refs. It is also absent from upstream trunk's pdftex.web, tex.ch and unbalanced-braces.ch. Its origin is unidentified, possibly a distro patch [I].
- Mitigation: this is M0's first job. Pin one TeX Live tag and find where that symbol comes from before any fan-out.

**2. The token rewrite departs from WEB in the most sensitive code.**

- Failure modes: `align_state` drift, `\outer` and runaway detection, `\noexpand`, error-context printing, non-LIFO frees while arguments cross a list end.
- Mitigation: token API from day one, generic paths first. Capture loops only after trip and etrip pass, behind `slow_capture`, with both modes diffed on the corpus.

**3. Snapshot soundness: stale or incomplete state.**

- Sources: dependencies outside the fingerprint (environment, negative lookups, `\pdffilemoddate`, shell escape, time, random seed), and non-image state in leaf crates. These would produce silently wrong PDFs, which is worse than slowness.
- Mitigation: taint-based bail-out, `--no-snapshot`, the image-only rule, and CI that compares snapshot and cold PDFs for every corpus document.

Runner-up: commute fidelity against real tikz-cd output, bounded by the restart fallback.

## Caveats

- The token replay is synthetic (body walk and eqtb fetch only). Use it for ratios, not absolute times.
- mmap timings were on tmpfs.
- Sampling is tick-based at about 1 kHz.
- The corpus is two mathematician documents. Prose-heavy books could raise the typesetting share, but the prose-only body stayed token-dominated [V].
- A safety check blocked one `rm` of my own scratch files, so a stale `zb/corpus/` directory remains in the scratchpad. I used `zb/corpus2/` and did not retry the removal.
- `bench/`, `docs/` and `tests/` now exist in the project, and they are not from me.