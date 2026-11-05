# rstex: how a release is built

DESIGN.md section 8 asks for `opt-level=3`, `lto="fat"`, `codegen-units=1`, `panic="abort"`, `+crt-static`, no position independence and PGO. The first four are in `Cargo.toml`'s release
profile, the next two are in `.cargo/config.toml`, and PGO is one command, on stable Rust, 1.89 or later for `File::try_lock`:

```sh
rustup component add llvm-tools         # llvm-profdata must be rustc's LLVM's, not the system's
python3 -I tools/pgo.py                 # leaves <target>/pgo-build/final/rstex and prints a table
```

`tools/pgo.py`'s docstring says what each directory under `<target>/pgo-build` holds. Everything stays under `target/`, because a profile derives from what it was
trained on. Under a load of 10 it took 4 minutes to the binary and 12 with the table; under a load of 40, 17 and 56; under a load of 15 with the checks of this tree, 5 to the binary. `--no-bench` stops at the binary. On a tree with no cache of
the oracle's results, the first run also compiles each training document with pdflatex, about 10 more minutes.

`commute` is the same engine in another binary: `cargo build --release -p commute-cli` leaves `<target>/release/commute`, with these flags and no profile of its own.

## What a release does

1. A TeX Live with `pdflatex`: the training documents' side files come from the oracle, through `tests/corpus/run.py`'s cache.
2. `refs/corpus`, for the long public documents: `tests/corpus/public/fetch.py` and the arXiv sample of `tests/corpus/arxiv/`. Without it the training set is the ladder and the two
   format builds, and the tool says so.
3. `python3 -I tools/pgo.py --train public`. A release does not depend on anyone's private documents, and loses nothing by it: see below.
4. `tests/gate.sh release --engine <target>/pgo-build/final/rstex --only integration --only compare --only trip`, since PGO changes the code generated and the binary shipped
   is the binary judged. Plain `tests/gate.sh ID` on the same tree adds `cargo test` and TRIP on a checked build. The gate judges a cold run that writes a checkpoint and no restored one, so against the wrapper it leaves as `target/gate/ID/rstex`, also: `tests/corpus/snapshot/run_snapshot.py --engine W --cold-engine "W --no-snapshot"`, `policy.py --engine W`, `warm_equals_cold.py --engine W`, `warm_equals_cold.py --engine W --keep-pdf`, and with the binary itself `pages.py gate --engine BIN --keep-pdf`, and `tests/until_stable/run.py --engine BIN`, since the restore paths are code that PGO generates too. A plain `tests/gate.sh ID` has them all as steps.
   Those three judge the literal engine, with the native diagram path off. `--only native` judges the path on the same binary, in half an hour more.
5. Ship it stripped, `strip -o rstex <target>/pgo-build/final/rstex`, and keep the unstripped one for symbolising reports.

Training has the native diagram path of DESIGN.md section 10 on, as a user's run has it: a document's diagrams count what the path does, and those it declines what real PGF does. So the profile sees both, and `RSTEX_NO_COMMUTE=1` or `--no-commute` on a PGO binary runs code that is trained only as far as declined diagrams and the format builds reach it.

A profile is only good for the code it was trained on: a function that changed loses its counts, and nothing says so. So a release runs the whole tool on the tagged commit.
The binary is not reproducible from one training run to the next, since the counts differ by a little.

`-enable-ext-tsp-block-placement` is a hidden LLVM option. If a later rustc refuses it the tool stops, and the flag can go at a cost of a few percent.

Windows and macOS: `-Cprofile-generate` and `-Cprofile-use` exist there, but this tool has only been run on Linux. M7's CI runs it per target or ships those without a profile. On macOS and on MSVC `--order` cannot work, since it hands lld a flag of its own.

## When TeX Live updates

The native diagram path and the native `\pickup@font` take over from macros they know by a hash of each: `crates/engine/src/native_table.rs` and `native_font_table.in`, generated from a tree. DESIGN.md section 10, "Versions", says why a set of hashes goes as a whole. On a tree in none of them, all of it runs real TeX: the output stays right and nothing in a log changes, and a document with diagrams takes some 270 times the instructions per diagram. So it can happen to anyone, and to this machine, the day Arch updates `texlive-pictures` or `texlive-latex`, with no message, and an editor just gets slower.

**What a user sees.** Nothing, unless asked: `rstex --check-tree`, whose output `README.md` shows, names the macro and the version that stand in the way. `commute` with a preamble says it each time, since it cannot write the SVG, with status 3. Lemma can read the lines `path=` of the first and show them.

**What CI sees.** The linux job installs TeX Live of today from the repository, as it is cached by year, and runs `tools/ci/check_tree.py`: a warning on the run for each path that is off, and never a failure, since an upstream release must not turn a commit red. Delete the cache, by the key in the workflow, to see what a new install gets.

**What to do, for a new tree, on that tree.** With its `pdflatex` and `kpsewhich` first on `PATH` and no other TeX there, and cargo:

```sh
python3 -I tests/cd/native/new_profile.py -j 6
```

That builds with the probe, generates the profile and adds it beside the others, builds again, and holds the result against that tree's own pdfTeX: the public diagrams of `tests/corpus/manifests/cd.json` whose packages that tree has, by dimensions, text, pixels and content stream; the check that a shift of 0.05 bp fails; the cases of `tests/cd/native`; the 8500 documents of `tests/cd/native/adversarial` and their cost; cold against warm runs; the public documents; the font path. It takes an hour or so at 6 jobs, most of it the oracle's first run, and its last line says `VERDICT pass` or `VERDICT DECLINE`. On a pass the two table files are ready to commit, and what stays behind in `target/new-profile` can go. On a decline they are back as they were, and `target/new-profile/<step>.out` says what failed: it may be that the new PGF draws something else, as 3.1.12 writes its lengths otherwise, and the model has to learn it under that profile, `commute::BpRule`. Do not lower the bar for a profile: a tree that cannot pass stays off, which costs time and nothing else.

A tree this small leaves out what its packages cannot compile, and the step's first line says how many that was. A tree needs at least what `tests/cd/native/gen_table.py` loads, or the table has less to say than on a full one: `SAFE_PACKAGES` and `STABLE`, which `tools/ci/install_texlive.py DIR --profile` adds.

What an update does to the cache needs no doing: a checkpoint and page records answer for each file they read, so the first run on a changed tree writes new ones, and `tests/cd/native/upgrade.py` holds that against a cold run, in a copy of a TeX Live tree with one macro changed at a time.

By hand, the same two generators: `gen_table.py --engine PROBE` for the profile, `--list` for what the table holds, `--drop ID` for one that no one has any more, and `tests/native_font/gen_table.py --engine PROBE`. A profile that has not passed the gate does not belong in a release.

**The release process.** Before a release, on a current TeX Live: `tools/ci/install_texlive.py` into an empty directory, `new_profile.py` there, and `rstex --check-tree`, for each of TeX Live's current state and the last Arch packages. All profiles that any user may still have stay in; the table has room for 16. A profile adds about 10% to the table where the macros are the same, and nothing to the path: its lookups are one binary search and a mask, and on the thesis the path's cost does not move.

## What was measured, and what it decided

On d3174d8 with the cold marks, before the token machine's work, on an AMD Ryzen 7 PRO 7730U, at a load of 10 unless it says otherwise. Instructions are exact to four digits. Cycles and CPU time are medians of runs
in turn and are noisy: under a load of 40 the same binaries differ by 5% from one comparison to the next, and at 10 by 2%. DESIGN.md section 16 has what a release gives now.

**PGO gains 6 to 11% of the instructions and 7 to 16% of the cycles, on every document.** Branch misses fall by up to 5% and L1d misses not at all, while IPC goes from 2.6 to 2.8 on
the thesis and from 2.3 to 2.5 on SGA3: the gain is layout and inlining on the hot path.

| | instr, no profile | ratio, public | ratio, all | cycles, no profile | ratio, public | ratio, all |
|---|---|---|---|---|---|---|
| hello | 0.137 G | 0.918 | 0.918 | 0.065 G | 0.87 | 0.87 |
| plain-hello | 0.051 G | 0.885 | 0.885 | 0.019 G | 0.90 | 0.93 |
| tikzcd-many | 10.77 G | 0.953 | 0.953 | 4.69 G | 0.84 | 0.89 |
| texweb | 2.98 G | 0.930 | 0.934 | 1.59 G | 0.87 | 0.84 |
| thesis | 10.01 G | 0.935 | 0.934 | 3.91 G | 0.83 | 0.87 |
| LMB | 10.04 G | 0.934 | 0.933 | 4.08 G | 0.85 | 0.84 |
| SGA3 | 62.49 G | 0.951 | 0.945 | 27.59 G | 0.87 | 0.87 |
| pdflatex.fmt | 48.55 G | 0.934 | 0.934 | 18.63 G | | 0.85 |

All of it over the build without a profile. The cold marks take up to 1.5% off that build, so against the build before them, d3174d8, the instructions are 0.885 to 0.940 and the cycles 0.80 to 0.87.

**Public training is enough.** Adding the private documents changes the instructions by 0.6% on SGA3, by less on the others, and the cycles by no more than the noise. On SGA3 public
training took 8% more cycles at a load of 40 and 4% more at 15, and the same at 10, in 3 to 5 rounds each, so at most a few percent. Trained on the thesis alone, SGA3 runs 1.5% more instructions than trained on SGA3 alone
and the same cycles, so the thesis does not hurt it. What does hurt is a narrow set: the one DESIGN.md first named, thesis, LMB, tikzcd and hello, has no plain TeX in it and costs texweb 5%
against a broad one. SGA3 has 7% of its counts in `expand_font`, which no public document has as much of. Training on long public documents with font expansion did not change its cycles either.

| instructions over no profile, before ext-TSP | hello | plain-hello | tikzcd-many | texweb | thesis | LMB | SGA3 |
|---|---|---|---|---|---|---|---|
| public, equal weights | 0.916 | 0.886 | 0.942 | 0.926 | 0.931 | 0.930 | 0.948 |
| public, no weights | 0.936 | 0.932 | 0.939 | 0.929 | 0.929 | 0.929 | 0.942 |
| all, equal weights | 0.916 | 0.886 | 0.941 | 0.931 | 0.930 | 0.929 | 0.940 |
| all, no weights | 0.936 | 0.932 | 0.935 | 0.922 | 0.927 | 0.926 | 0.928 |
| thesis, LMB, tikzcd-many, hello | 0.935 | 0.935 | 0.935 | 0.976 | 0.928 | 0.927 | 0.936 |
| thesis alone | 0.934 | 0.935 | 0.938 | 0.944 | 0.928 | 0.928 | 0.940 |
| SGA3 alone | 0.934 | 0.928 | 0.937 | 0.916 | 0.930 | 0.929 | 0.926 |

**Weights.** Merged as they come, a document weighs what it runs: SGA3 5.2 G counts, the pdflatex format 3.7 G, the thesis 0.8 G, hello 14 M. Scaled to the same total, hello runs 2% fewer
instructions and 3% fewer cycles, and plain-hello 5% and 10%, while the long documents run up to 1.5% more instructions and the same cycles. So equal weights are the default: the long
documents share the hot loop anyway, and startup is what hello is made of. Repeating a small document in training would do what the weight does, since a run is deterministic, and
it costs time, so don't. `--weights sqrt` lies between and gains nothing over equal.

**SyncTeX, not trained on.** `tools/prof/synctex_cost.py` and three more training runs with `-synctex=1`, hello, a paper and the thesis, on the same code.  With them, a run with SyncTeX off took 1.0042 of the base's instructions on hello cold, 1.0024 warm and 1.0029 on the thesis warm; without them 1.0002, 1.0006 and 1.0004.  And a run with SyncTeX on was no better for it: without them the thesis took 0.9976 of the instructions and 0.986 of the cycles over 41 rounds in turn, whose control was 1.029, LMB 0.9988 and 0.984, with 0.996, and SGA3 0.9987 and 1.009 over 7 rounds, with 1.004.  So there are none: the records are short straight code.

**Diagrams, both ways.** With the native diagram path on, a document of diagrams trains the geometry library where it used to train the token machine through real PGF, and equal weights give it the same mass. Trained on the path alone, documents with no diagram ran more instructions than the build before the path was merged, which had the same training otherwise: hello 1.009, summary 1.017, SGA1-VI 1.011, SGA3 1.009, `pdflatex.fmt` 1.026, where their plain builds are at 0.990 to 1.004. So `TRAIN_BOTH_WAYS` in `tools/pgo.py`, tikzcd-many, one arXiv paper, the thesis and LMB, run again with the path off, as documents of their own. Real PGF is also what a diagram that declines runs.

| both ways, over | | hello | tikzcd-many | thesis | LMB | summary | SGA1-VI | SGA3 | `pdflatex.fmt` |
|---|---|---|---|---|---|---|---|---|---|
| the path alone | instructions | 0.990 | 0.990 | 0.991 | 0.991 | 0.991 | 0.993 | | 0.988 |
| | cycles | 0.985 | 0.985 | 0.981 | 0.990 | 0.992 | 0.990 | | 0.997 |
| | A/A control | 0.988 | 0.997 | 1.011 | 1.009 | 1.000 | 1.004 | | 0.999 |
| before the merge | instructions | 0.998 | | | | 1.007 | 1.004 | 1.005 | 1.011 |
| | cycles | 1.004 | | | | 1.015 | 0.998 | 0.999 | 1.020 |
| | A/A control | 0.996 | | | | 0.992 | 0.998 | 1.001 | 1.006 |

Medians of cycles in turns, a cold run each, at a load of 4 to 6: 40 rounds, or 60 for hello, tikzcd-many and the short documents against the build before the merge. So training both ways is as good or better on all of them. What is left against the build before the merge is summary and the format, 1.5 to 2%, more than their controls, which has not been explained; SGA3, SGA1-VI and hello are within theirs. A control belongs beside what it controls: a copy of the binary on tmpfs read 0.938 on hello, against btrfs.

**On top of the profile.** Over the same profile without the flag, instructions then cycles:

| | hello | thesis | tikzcd-many | texweb | SGA3 | decision |
|---|---|---|---|---|---|---|
| `-enable-ext-tsp-block-placement` | 0.998, 0.94 | 0.996, 0.99 | 0.997, 0.95 | 0.998, 0.99 | 0.997, 1.01 | on |
| `--symbol-ordering-file` | 1.000, 0.97 | 1.000, 0.98 | 1.000, 1.01 | 1.000, 1.01 | 0.998, 1.02 | off |
| `-enable-split-machine-functions` | 1.000, 0.98 | 1.000, 1.02 | 1.000, 1.01 | 1.000, 1.02 | 0.998, 1.07 | off |
| `-hot-cold-split` | 1.004, 1.00 | 1.004, 1.02 | 1.006, 1.00 | 0.998, 1.00 | 1.003, 1.12 | off |
| `-Ctarget-cpu=x86-64-v3` | 0.994, 0.97 | 1.002, 1.02 | 1.002, 1.02 | 0.994, 1.01 | 0.998, 1.04 | off |
| `-Crelocation-model=static` | 0.999, 0.99 | 0.994, 0.99 | 0.992, 0.98 | 0.980, 1.00 | 0.988, 1.06 | on |

SGA3 had two rounds in this comparison, so its cycles are the least sure. Under a load of 40 ext-TSP gave 0.93 on hello, 0.94 on the thesis, 0.95 on tikzcd-many and 0.95 on SGA3, and it cuts
branch misses by 11% on tikzcd-many. `-hot-callsite-threshold` at 1000 and 6000, in place of 3000, had a reason, since a profile takes the code the thesis runs from 359 to 555 KiB; neither changed the
instructions by 0.1%, or the cycles beyond the noise.

- **Order of the code.** With a profile, lld sorts functions by the call graph, and turning that off with `--no-call-graph-profile-sort` changes the cycles by nothing that can be seen. A symbol
  order that puts what hello runs first takes hello's faults on the text from 40 to 37 of about 1470, since glibc's and std's startup code is not in the profile, and changes nothing else. At a
  microsecond or two a fault it isn't worth a file. `--order` keeps the experiment, to repeat if hello ever nears 5 ms.
- **`-Crelocation-model=static`** gives a non-PIE binary: no `.rela.dyn` to apply, 6124 relocations, 26 fewer faults, mostly copy-on-write on `.data.rel.ro`, and up to 2% fewer
  instructions. But `crtbeginT.o` then registers `.eh_frame` and libgcc walks all of it, 280,000 instructions on every start, and about 150 more for each function added: the native diagram path's 145 functions came to 0.39% on plain-hello.
  So the linker sends that object's two references to symbols defined as 0, which it tests before it calls: `--wrap` and `--defsym` in `.cargo/config.toml`. Nothing is registered, and an unwinder finds `.eh_frame_hdr` through `dl_iterate_phdr`, for the test profile's panics and any backtrace. That takes plain-hello from 5.34 to 4.96 M and hello from 46.8 to 46.3 M. The binary also gives up ASLR of the image. On, for the faults and the long documents' instructions; `.cargo/config.toml` is
  the one place to change if ASLR comes to matter.
- **Warm runs in training.** The restore's and the checkpoint writer's loops have count 0 in a profile of cold runs, and so get no vectorising. Adding a run that writes a checkpoint and one that restores it, of hello and the
  thesis, takes hello's first run, the one that writes, from 65.1 to 50.2 M instructions and 27 to 22 ms of CPU, at the minimum over 3, and its warm run from 19.7 to 19.3 M, 8.9 to 8.7 ms; the cold runs do not move, 1.000 on hello, plain-hello, texweb and the thesis. All twenty LaTeX documents of the ladder as well gave the same for the checkpoint's paths, and cost texweb 0.7%. `--no-warm` leaves them out.
- **The output cache in training.** The third warm run keeps the last PDF and goes back to the preamble with `RSTEX_RESUME=0`, so a profile holds a run that finds every stream as well as one that finds none, each writing page records, and a fourth resumes from the last of them. With it, on the same tree, hits take about the same share as on a plain build, 0.817 against 0.811 of the instructions on the thesis and 0.571 against 0.562 on hello (DESIGN.md section 9), and cold runs 0.11 to 0.25% more instructions than the base's profile did, which is what two profiles differ by; 0.04% of that on the thesis is the tracking in `write_t1` that `CarryUse` needs, as a plain build shows. On the merge of all of it, DESIGN.md section 16, hits and a run from the last record are 0.776 and 0.060 of the base's warm run with PGO and 0.775 and 0.060 plain, so nothing there goes untrained; what each training run is worth on its own was not measured.
- **`-Ctarget-cpu=x86-64-v3`** settles what DESIGN.md section 1 says of `native`: nothing hot here vectorises, and it stays off.
- **`#[cold]`** was on none of the error procedures. It now is, on `error`, `print_err`, `help`, `succumb`, `fatal_error`, `overflow`, `confusion` and those that only report. Without a profile that
  saves up to 1.5% of the instructions. With one it saves nothing, since those blocks have count 0.
- **glibc's `memcpy` and `memset`.** `+crt-static` does not stop IFUNC: the static binary applies 31 `IRELATIVE` relocations at startup and picks `__memmove_avx_unaligned_erms` and
  `__memset_avx2_unaligned_erms`. They show in hello, 17 and 2% of the time, because they are where the format is copied into memory not touched before. By `GLIBC_TUNABLES`: no AVX2, no
  `rep movsb`, no non-temporal copies and no `rep stosb` all give hello the same cycles within 1%, though no AVX2 takes 3 M more instructions, so the choice is real. Replacing them would
  gain nothing. Copying less is the lever.
- **Frame pointers are not free.** `-Cforce-frame-pointers=yes` costs 3.3% of the instructions on hello and 5.1% on the thesis, so a build from `rstex_prof.py --build` overstates prologues by that much.
- **`debug="line-tables-only"` is free.** The same instructions to four digits, `.text` within 320 bytes of 2.9 MB, and no `.debug_*` section in a `PT_LOAD`.
- **Size and page-in.** 11.8 MB unstripped, 3.7 MB stripped, and what is more than that is never mapped. Warm, a stripped binary starts in the same time, `--version` and hello alike. With the binary
  evicted from the page cache, hello takes 10 to 30 ms longer under a load of 30 and stripping gives back about 10 of those. Hence step 5.

## What CI has to settle

Windows and macOS have been compile-checked on Linux, for all targets with their tests, and never run: there is no wine, no mingw, no `x86_64-pc-windows-gnu` target and no macOS here. What follows is what careful reading could not settle. `.github/workflows/ci.yml` runs all of it; `tests/cross_os/probes.py` holds what `cargo test` cannot show, and it prints what it measures. A first run on each system answers each item, and the answer goes here.

| Job | With the caches filled | With nothing cached | $ per run on a private repository |
|---|---|---|---|
| Linux, every push: guard, `cargo check` for the two other systems, `cargo test`, release build, the integration cases, the public documents | 7 min | 22 min | 0.04 |
| Windows, on `main`, tags and dispatch | 12 min | 27 min | 0.12 |
| macOS, on `main`, tags and dispatch | 9 min | 24 min | 0.56 |
| The comparison, after them | 1 min | 1 min | 0.01 |

These are estimates from `cargo test` here at 4 jobs and from the size of a TeX Live, not from a run. A push to a branch is then 7 minutes, and a merge to `main` 29, or $0.73: the $12 a month pays for 16 merges and no branch pushes, or 8 merges and 1000 minutes of pushes. If the first run says otherwise, the macOS job is the one to move to tags only.

**Every system.**

- `cargo test --workspace` green. Tests that need a tree skip without one, and this job has one. Some of the tests have only ever seen Linux: a failure may be the test's.
- `tests/cross_os/run.py`: the three public documents give the same SHA-256 on all three, and the same again when the third run restores a checkpoint. A difference with the same TeX Live is a bug. Different versions of LaTeX can differ for their own reasons, so read the versions first: all three jobs install from one repository, but on different days if a cache has expired. `plain.tex` depends on neither.
- `RSTEX_ORACLE_DIR`, Linux only: `tests/integration/run.py` against the pdfTeX of an official TeX Live, which is another build than Arch's: PORTING section 11. `tests/integration/second_oracle.txt` lists how it differs, each case with its mechanism, and a case that differs otherwise fails. Reproduced locally without CI: the same script into `target/texlive-official`, on a `PATH` with no other TeX. It has not been run on a runner since that list was made.

**Windows.**

- That the program links and starts at all: a `no_main` with `extern "C" main`, static CRT, `panic = "abort"` and fat LTO.
- The image. `Tex::alloc` on the first access, each first touch a fault that the vectored handler commits: the first page of each array, the file-level reads of `toks` below `NULL_LIST`. Read off the probes' times: hello against Linux on the same class of machine, and whether the exception cost shows. A run takes on the order of 30 chunks, so it should not.
- The format load: `commit_range` and then `ReadFile`; `\dump` of the LaTeX format, which `commit_range` lets `WriteFile` read. A format that loads is the proof.
- That `SetThreadStackGuarantee` of 32 KiB leaves enough for the handler: `platform::tests::a_full_stack_has_a_message_and_a_status`, and a stack of 1 GiB as `std::thread` asks for it: `half a million boxes in a box`.
- Names: the file with spaces and non-ASCII, `\input` with a drive letter, and a main file `C:\...`.
- A PDF held open with no sharing: `a_file_held_open_keeps_the_bytes_under_another_name` and the probe. The first is the rename and the write failing, the second what the job says, which on the first open is TeX's "I can't write on file".
- Replacing a format that another run has open. `std` opens with `FILE_SHARE_DELETE`, and its rename, as far as memory and the symbols in its library go, is `SetFileInformationByHandle` with POSIX semantics where the file system has them, so that should simply work on NTFS, and on FAT goes to `MoveFileEx`, where `publish`'s retries are all there is. Not probed yet: two runs at once on a new format, which `runs_at_the_same_time_share_one_build` does.
- MiKTeX, by hand or in a job of its own, for it is not in this workflow: `initexmf --report` as `parse_miktex_report` reads it; where `pdftex.map` is; whether `tex-ini-files` has `pdflatex.ini` where the built-in recipe looks. **The cost:** a run checks the stamp of every directory the walk saw, 4308 of them under `tex/` and 1926 under `fonts/` in a full TeX Live, and on NTFS `std::fs::metadata` opens each file. The walk works, on a tree of that shape on Linux: 27 s under a load of 25 for the first run with its format build, 0.95 s for the next, which makes the font map's tables, then 51 ms. If it costs hundreds of milliseconds on Windows, the answer is to keep fewer stamps, as MiKTeX's own database and `ls-R` get by without any: to be decided with that number.
- Console: a TeX run with bytes that are no UTF-8 in its output, at a real console, which no runner has. By hand.
- Control-C: `SetConsoleCtrlHandler` runs on a thread of its own and sets `interrupt`. By hand, for a runner has no console to send it on.
- `\write18` with quotes through `%COMSPEC% /c`: `tests/integration/ext/` does this only on Linux.
- `TZ`: `\time` goes by the system's zone. TeX Live's `localtime` is msvcrt's, which reads `TZ`. Whether anyone sets it there is not known.

**macOS.**

- That a thread with a stack of 1 GiB starts, and at what cost. `half a million boxes in a box`, and the handler's test: whether a guard page gives `SIGSEGV` or `SIGBUS` does not matter, since both are caught.
- Private mappings of 16 KiB pages: the third run of `tests/cross_os/run.py` restores over them, and a format load maps at offsets that `image_padding` keeps at 16 KiB. `RSTEX_CHECKPOINT_TRACE` says "restored", and the sums are the test. On Intel, 4 KiB: `macos-13` if that matters.
- The ten-times cost of the case-folded table is gone, measured on Linux with the switch on: see below. It should stay gone: read hello's time off the probes. A name in another case is found: the probe.
- `MAP_PRIVATE | MAP_ANON` of 2 GiB and more with no `MAP_NORESERVE`: Linux's overcommit is not macOS's, and a system under memory pressure might say no. `half a million boxes` shows only that it does not on a runner.
- `kpsewhich` found on `PATH`, or from `/usr/local/texlive/YYYY` without it: what MacTeX has at `/Library/TeX/texbin` is on a GUI's `PATH` only sometimes. Run `tests/cross_os/run.py` once with a `PATH` that leaves it out.
- Names in NFD on HFS+: `ls-R` and a request in NFC would miss one another. APFS does not change names. Not probed.

**What was settled here, on Linux, against the oracle.** Under `/dev/full`, under `RLIMIT_FSIZE` with `SIGXFSZ` ignored, and in read-only directories, `/usr/bin/pdftex` and rstex give the same status and the same lines, apart from where in the run a PDF's failure comes, since rstex writes it at the end:

| Case | Both |
|---|---|
| Directory takes no file | `! I can't write on file` for the log, status 1 |
| Directory takes no file, the log and the PDF exist and may be written | written in place, status 0 |
| PDF is read-only, or a directory | the prompt and `! I can't write on file`, status 1 |
| PDF is a symbolic link | written through; the link stays |
| PDF has mode 0600 | still has |
| PDF on a full disk | `!pdfTeX error: ...: fflush() failed (No space left on device)`, status 1, no file left |
| Log or `\openout` stream on a full disk | `fclose: No space left on device` on stderr, status 0 |
| `\openout` in a directory that is not there | the prompt and `! Emergency stop`, status 1 |
| Control-C | `! Interruption.`, status 1 with the answer X |

Where they differ on purpose: half a million nested boxes, where the oracle's 8 MiB stack gives a `SIGSEGV`, and this one a PDF; a full stack, which is one line and status 1, not a `SIGSEGV`; an address space under 2 GiB, in which the oracle runs and this one stops with a message, status 1; an engine panic, which the oracle has no such thing as, says it is a bug and aborts, status 134; and a run that is killed, where the oracle leaves the part of a PDF it has flushed, which no viewer can read, and this one an empty file, since the PDF only goes out at the end. The log is about as long on both, since the buffer is the 4 KiB of stdio: it was 64 KiB, which left a killed run with no log at all.

**Measured, all of this branch on the plain build, against that of the M5 merge, at a load of 38, 3 counted and 3 timed runs each.** Instructions: 1.000 on all seven of hello, plain hello, `tikzcd-many`, `texweb`, the thesis, LMB and SGA3. CPU: 0.87 to 1.14, the noise of that load. Faults: hello 642 to 632, plain hello 534 to 543, `tikzcd-many` 1170 to 1183, `texweb` 2434 to 2382, the thesis 2423 to 2385, LMB 2614 to 2425, SGA3 6203 to 6148. The smaller buffer of the log is in these. **The handler for a full stack**, which the plan said to decide with a measurement: three more system calls at each start. With it, and without it and the panic hook and Control-C, over 400 starts of `--version` in turn: minimum 812 against 822 us, median 2508 against 2491 us, 53.3 faults each. Nothing that can be seen, so it stays.

**The case-folded table, with `db_casefold` on, on Linux.** Before, a table made in each run, at its first miss: plain hello 5.4 to 301.5 M instructions and 4.8 to 156 ms of CPU, hello 46.8 to 343 M and 21 to 225 ms. Now, in the index file: 5.4 and 46.9 M, and the CPU of no fallback at all.
