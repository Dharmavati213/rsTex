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

## What a release does

1. A TeX Live with `pdflatex`: the training documents' side files come from the oracle, through `tests/corpus/run.py`'s cache.
2. `refs/corpus`, for the long public documents: `tests/corpus/public/fetch.py` and the arXiv sample of `tests/corpus/arxiv/`. Without it the training set is the ladder and the two
   format builds, and the tool says so.
3. `python3 -I tools/pgo.py --train public`. A release does not depend on anyone's private documents, and loses nothing by it: see below.
4. `tests/gate.sh release --engine <target>/pgo-build/final/rstex --only integration --only compare --only trip`, since PGO changes the code generated and the binary shipped
   is the binary judged. Plain `tests/gate.sh ID` on the same tree adds `cargo test` and TRIP on a checked build. The gate judges a cold run that writes a checkpoint and no restored one, so against the wrapper it leaves as `target/gate/ID/rstex`, also: `tests/corpus/snapshot/run_snapshot.py --engine W --cold-engine "W --no-snapshot"`, `policy.py --engine W` and `warm_equals_cold.py --engine W`.
5. Ship it stripped, `strip -o rstex <target>/pgo-build/final/rstex`, and keep the unstripped one for symbolising reports.

A profile is only good for the code it was trained on: a function that changed loses its counts, and nothing says so. So a release runs the whole tool on the tagged commit.
The binary is not reproducible from one training run to the next, since the counts differ by a little.

`-enable-ext-tsp-block-placement` is a hidden LLVM option. If a later rustc refuses it the tool stops, and the flag can go at a cost of a few percent.

Windows and macOS: `-Cprofile-generate` and `-Cprofile-use` exist there, but this tool has only been run on Linux. M7's CI runs it per target or ships those without a profile. On macOS and on MSVC `--order` cannot work, since it hands lld a flag of its own.

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
  instructions. But `crtbeginT.o` then registers `.eh_frame` and walks all of it, 280,000 instructions on every start: with everything else in, that is the difference between 5.3 and 5.0 M on plain-hello, at
  the same time within the noise (minimum 3.5 against 3.6 ms over 21 runs), and hello is the same either way. It also gives up ASLR of the image. On, for the faults and the long documents' instructions; `.cargo/config.toml` is
  the one place to change if ASLR comes to matter.
- **Warm runs in training.** The restore's and the checkpoint writer's loops have count 0 in a profile of cold runs, and so get no vectorising. Adding a run that writes a checkpoint and one that restores it, of hello and the
  thesis, takes hello's first run, the one that writes, from 65.1 to 50.2 M instructions and 27 to 22 ms of CPU, at the minimum over 3, and its warm run from 19.7 to 19.3 M, 8.9 to 8.7 ms; the cold runs do not move, 1.000 on hello, plain-hello, texweb and the thesis. All twenty LaTeX documents of the ladder as well gave the same for the checkpoint's paths, and cost texweb 0.7%. `--no-warm` leaves them out.
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
