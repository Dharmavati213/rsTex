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

## What has run on macOS and Windows

`.github/workflows/ci.yml` runs it: the job `other-systems` on `main`, on tags, on dispatch and on the branch `cross-os`, and `cross-os-win` or `cross-os-mac` run the one system alone, with no Linux job, so that a fix for one does not pay for the other. A branch keeps its caches from one run to the next, and the caches of TeX Live and the build are saved by a step of their own, as `actions/cache` saves nothing from a job that fails. The same tools run in the `linux` job on `main`, on tags and on `cross-os`, where the sums are to be held against, and not on an ordinary push.

The runners are `windows-2022` (x86-64, Python 3.12), `macos-14` (arm64, 16 KiB pages) and `ubuntu-24.04`, with the official TeX Live 2026 from `tlnet`, by `tools/ci/install_texlive.py`, so that each system's oracle is that system's own official pdfTeX: Microsoft's compiler, Apple's and gcc. qpdf 12.4.2 and poppler 26.09 on Windows come from their own releases, in the cache of the tree; on macOS they are Homebrew's, as qpdf's own build there is for macOS 15.

| What | What it holds to | `windows-2022` | `macos-14` |
|---|---|---|---|
| `cargo test --workspace` | | all pass | all pass |
| `tests/cross_os/run.py` | 23 documents, its three and the ladder, cold and then restored: the same bytes, and the same on all three systems | 23 of 23 | 23 of 23 |
| `tests/cross_os/probes.py` | what only that system can show | all hold | all hold |
| `tests/integration/run.py` | every case against that system's own pdfTeX | 817 cases: 780 agree, 37 as the lists say | 818 cases: 784 agree, 34 as the lists say |
| `tests/cross_os/warm.py` | a warm build is a cold one, byte for byte in every file, under `--until-stable -synctex=1 -interaction=nonstopmode -file-line-error -recorder -output-directory=DIR`: a checkpoint, the output cache with the PDF in place, a sealed one, an edit and its undoing; without SyncTeX, page records, and a run from the last and after an edit | 73 of 73 | 73 of 73 |
| `tests/cross_os/kill.py` | 24 kills in each part of a run, with a checkpoint and with page records, and a build at once after each; a kill of a later pass; on Windows a PDF held in each of five ways, a PDF held from the middle of a build, a side file held, a file of the cache held; two builds at once | 42 of 42 | 5 of 5 |
| `tests/cross_os/synctex_tool.py` | TeX Live's `synctex`, 159 questions forward and back, on rstex's file of 7 gzip members against pdfTeX's, plain and in a directory with an accent | 6 of 6 | 6 of 6 |
| `tests/cross_os/stable_tools.py` | `--until-stable` with the real `bibtex` and `makeindex`: every line and status, and the limit and an error through the processes that wait | 18 of 18 | 18 of 18 |
| `tests/cd/compare_cli.py --tests-cd` | 338 diagrams against real tikz-cd under that system's pdfTeX: the box to the scaled point, the page, qpdf, pixels at 400 dpi, and a file of them against each alone | 279 drawn, all box and page equal; 111 to the pixel, 168 on the list below; with a preamble 116 of 116; a file of 279, 558 of 558 files equal | the same |
| `tests/cross_os/names.py` | spaces, accents, other alphabets, outside the BMP, what a shell reads, a tilde, a percent, on macOS NFD; in the document's path, the output directory, the cache with a run that restores, `~/texmf` and a TeX Live's own place; on Windows a document and a cache under a name of 330 characters | 52 of 52 | 64 of 64 |
| `tests/cross_os/shell.py` | `\write18`, the three kinds of shell escape, pipes, `TEXEDIT`, CRLF and CR sources, backslashes in a document's names and on the command line, a document's own names in UTF-8 and not, against pdfTeX | 40 of 40 | 41 of 41 |

All of that is run 42059789389 at `1fb1291`, whose `same-pdf-everywhere` could not find the sums of the two other systems, one level down in their artifacts; `tests/cross_os/run.py --compare` was then fixed, and on those three artifacts says that the three systems agree on the 23 documents, on the PDF and SVG of 279 diagrams, and on the 4 PDFs of the bibliography, index and two-pass documents. `ubuntu-24.04` in that run: 818 cases, of which 786 agree and 32 are as `second_oracle.txt` says, 73 of 73, 5 of 5, 6 of 6, 18 of 18, the same diagrams, 55 of 55 names and 41 of 41 of the shell. The run of the last commit on the branch `cross-os` is what stands.

**What it found.**

- *Windows, `\pdffilesize` of the null device.* expl3 asks it when the format is built, to set `\c_sys_platform_str`. `std::fs::metadata` has no answer where `_wstat` has size 0, so in rstex's format on Windows the platform was `unknown` and `\sys_if_platform_windows` false, for every document. Seen as two multiletter control sequences fewer in every LaTeX log, against that system's pdfTeX.
- *Windows, the name of a file found.* `kpathsea_find_file_generic` ends there with `kpathsea_getlongpath`, each part as `FindFirstFile` has it. rstex had no such thing and took the case of `ls-R`'s table for the name: `npbt_eufom_linie.png` for `NPBT_eufom_linie.png` in the log, the recorder and `/PTEX.FileName`, and a blank at the end of a name.
- *Windows, a kill of a build left its later passes running.* A pass waits there for the next, and `TerminateProcess` ends only the one the editor started. So a job with kill-on-close, whose one handle dies with the process, and let go of at an ordinary end, so that what a document started stays.
- *Windows, a format under a long name did not build.* A build runs in a directory of its own, and `CreateProcess` refuses a working directory of `MAX_PATH` or more. Such a build now goes in the system's temporary directory and the file comes to the cache. commute's own formats start from another by a relative name, so those say there is no room.
- *Windows, messages.* `kpse_invocation_name` is `rstex.exe`, as C's is `pdftex.exe`, which moves where TeX wraps. `%lX` is 32 bits. tex-make.c's "The command name is". A recorder file that cannot be opened again stops the job, by `xfopen`, where rstex went on with status 0 and left the file under its first name.
- *macOS and Windows, getopt.* What texmfmp.c gets there is kpathsea's `getopt.c`, in its own words: ``unrecognized option `-x'``.
- *Tools.* The integration harness assumed Linux, and now goes by what is there, as the top of `tests/integration/run.py` says. `tests/cd/compare_cli.py` kept the oracle's PDFs by their source alone, so that a run on one tree judged by those of another's PGF: they now go by the tree too. And two diagrams with one text raced.
- *Not a system's, but only CI's tree shows it.* On the official tree, with PGF 3.1.12, 168 of the 279 diagrams the command draws are a few pixels off from real tikz-cd, by up to some twenty levels of 255, box and page equal, the same 168 on Linux, macOS and Windows; on Arch's PGF 3.1.11a none is. The command's geometry is that version's. `tests/cd/pixels_on_official.txt` lists them, and should be empty when the command has a table for this tree.

**What differs from the oracle on those systems, and why it is allowed.** `tests/integration/second_oracle_windows.txt` has 8 lines and `second_oracle_macos.txt` 4, each read against both logs and the Linux build's; all else on `second_oracle.txt` holds there to the hash. These are signed overflow as another compiler evaluates it (`linebreak/wide_demerits`, `mathbuild/display_shape`, `mlist/limits_wide`), a stale word or an uninitialised read, the system's regular expressions in `\pdfmatch`, where rstex has glibc's on every system so that a PDF does not depend on it, `__FILE__`, and a wrap that the longer `pdftex.exe` moves. A case of a `mktexpk` on the `PATH` is not run on Windows, as kpathsea does not look there.

**Decided.**

- `calledit`: ported. On Windows the first word of `TEXEDIT` is a program that `SearchPath` finds along `PATH`, `.exe` and then `.bat`; a name that begins with `.`, `/` or `\` is refused, and a value that begins with a drive stands as it is. There was no reason not to, and refusing is what the code is for. A name with nothing after it is where C has no end for its buffer and asks for what lies beyond it in memory; rstex asks for the name.
- Redirected standard output and standard error stay LF, where pdfTeX has CRLF: DESIGN section 12. `--until-stable`'s lines are for reading by line, and the log, which is what latexmk and an editor read of TeX's words, is LF in pdfTeX as well. No tool was found that needs CRLF there, so nothing asks for it otherwise. What a program started by a run prints on its standard error is in that program's text mode, as it would be anywhere.

**Speed, as `tests/cross_os/speed.py` prints it, wall clock from the caller, minimum of 3 or 7, in run 42059789389.** The runners are not one machine, and nothing here is a gate.

| | `ubuntu-24.04` | `macos-14` | `windows-2022` |
|---|---|---|---|
| LaTeX hello, cold / warm / pdfTeX | 10 / 5 / 58 ms | 17 / 9 / 75 ms | 29 / 19 / 107 ms |
| 50 pages, cold / warm / pdfTeX | 43 / 39 / 195 ms | 83 / 69 / 394 ms | 77 / 65 / 293 ms |
| 40 tikz-cd diagrams, cold / warm / pdfTeX | 610 / 547 / 1290 ms | 792 / 785 / 2589 ms | 848 / 748 / 1830 ms |
| start `rstex --version` / `pdftex --version` | 0.4 / 0.6 ms | 3.4 / 2.9 ms | 3.3 / 37 ms |
| a stat, open-read-close, rename, make a file | 2, 10, 29, 34 us | 2, 20, 66, 142 us | 18, 39, 65, 102 us |
| read the 11 MB format | 0.5 ms | 1.8 ms | 2.9 ms |
| page faults of a warm hello | 562 | 1261, of 16 KiB | 3756 |

Windows takes 1.4 times Linux on the diagrams, 1.7 on the long document and 3.6 on a warm hello, so what it has more of is a constant at the start, some 14 ms, and not the engine's work. Each call costs that runner several times Linux's, as the table has them. Of those 14 ms, starting the process is 3, reading an image in place of mapping it 2.5, and there are more than 3000 more page faults, as no mapping leaves pages untouched and every byte read lands in a page of its own. The rest has not been taken apart, as `GetProcessTimes` counts in steps of 15.6 ms and a run this short cannot be from outside: it will be the stats of the format's list and the index, at 18 us each to 2, the files a run makes and renames, and the commits of chunks. None of that is one thing to fix, so no change was made for speed. Against pdfTeX on the same runner, which takes 37 ms just to say its version, a warm hello is 5 times as fast. The cost of Defender's real-time scanning is not known: a runner has it off, and asked to turn it on says `RealTimeProtectionEnabled : False` all the same.

**Still unknown, for no runner shows it.**

- MiKTeX: `initexmf --report` as `parse_miktex_report` reads it, where `pdftex.map` is, whether `tex-ini-files` has `pdflatex.ini` where the built-in recipe looks, and the cost of checking the stamp of every directory the walk saw, 6234 on a full tree, on NTFS. Only a tree of that shape on Linux has run.
- A console: a TeX run with bytes that are no UTF-8 in its output. Control-C, with or without passes waiting: `SetConsoleCtrlHandler` and `signal` are read, not run, and the job object has not been tried with it.
- An x86-64 Mac, and `macos-13` would run it. HFS+, which hands out NFD: on APFS a file made in NFD and asked for in NFC is found, on both engines. Windows on arm.
- A TeX Live of its own under a name with an accent, by a link on the `PATH`: pdfTeX cannot find its format there, and neither can rstex, which reads what `kpsewhich` prints in the code page, so no comparison can be made. Under a space it works on both. A `kpsewhich` that is not on the `PATH`, from `/Library/TeX/texbin`, has not been run.
- A code page other than 1252's on Windows, `TZ` there, a UNC name as more than a probe, and a network drive. A file system other than NTFS and APFS, FAT above all, where `std`'s rename has no POSIX semantics and `publish`'s retries are all there is.
- `tools/pgo.py` on either system: releases there have no profile yet.
- Which scanners a user has, other than the Defender above.

**Costs.** With the caches filled, the Windows job takes 13 minutes, macOS 10, and Linux 9 with the wide steps; an install of TeX Live adds some 3 minutes. On a private repository the two are $0.13 and $0.62 a run, at $0.010 and $0.062 a minute.

**The first run** (run 41957327432, at a0779ec) got as far as `cargo test` on both, 39 failures on Windows and 9 on macOS, and each was read. On Windows 17 were git's CRLF in the oracle's tables, now `.gitattributes`. The rest, on both, were tests that assumed Linux, but for one: `a_full_stack_has_a_message_and_a_status`, whose process had not installed the handler a real run installs first, and which the engine's thread now installs itself. Reading before that had left these to be shown, and they have been: the programs link and start, with `no_main` and the static CRT, in a release with fat LTO and `panic = "abort"`; `Tex::alloc` and the handler for its pages work, on 16 KiB pages too; `\dump` of the formats and their load; a stack of 1 GiB where one is asked for and a message from a full one; `half a million boxes in a box`; a restore over private mappings and through `commit_range`; a PDF held with no sharing; a copy of the program; a UNC name and one of 365 characters; `\input nul`; `\write18` with quotes through `cmd.exe`.

**Read, and not run, of what is above.** Replacing a format that another run has open, beyond `runs_at_the_same_time_share_one_build`. `--until-stable` takes more memory on Windows, as each pass waits for the next with its image committed, up to `--max-passes` of them: nothing has measured it. `MAP_PRIVATE | MAP_ANON` of 2 GiB and more with no `MAP_NORESERVE` on a macOS under memory pressure.

**Read for this, and nothing to change, on any of the three.** No use of a page size assumes 4 KiB. `image_load_bytes`, `see_chain` and `prefault_heap` ask the system, and a mapping only happens where the file's offset agrees with the page: `FMT_ALIGN` is 16 KiB, a larger page would be read. The 4096s left are of file formats, the sums of `cachefile.rs`, `ZERO_RUN` and `PAGE` of the page records, none given to a system call, but for the guard page of `run_on_new_stack`, which is Linux's, and which `mprotect` rounds up to a page. `madvise` and huge pages are Linux's alone. The handler's signal stack is 64 KiB, against Apple's minimum of 32. The image's layout is `repr(C)` of types the same size on all three, and a format is keyed on the target. There is no `char`: bytes are `u8`, and `c_char` is only in `main`'s signature, unused. Floating point is SSE2 or NEON, never x87, and rustc does not contract to FMA, so `glue_set` and PDF numbers are the same; `format!` of a float is Rust's own, `atan` in `fontfile/src/cstr.rs` is its own too, and the only call of the system's libm that can reach a PDF is `powf` in `image/src/png.rs`, whose result is rounded to an integer, as in libpng. Unaligned reads go through `read_unaligned`. Atomics are `Relaxed` only for a flag and the profiler's counters, and the slots of the Windows handler are `AcqRel`. zlib-rs 0.6.8 chooses by CPU only in `compare256`, `slide_hash`, `adler32` and `crc32`, which give the same answers, and it has no CRC hash, so its bytes should not depend on the CPU; `same-pdf-everywhere` is what can show otherwise. On Windows a name is WTF-8 inside and wide at the system, as under `command_line_encoding = utf-8`, TeX Live's default: `knj.c`, which is what `names.rs` follows, and a `.synctex` file's `Input:` lines are in the code page of the file calls, as synctex.c's `chgto_oem` has them, which was missing. A key that spells a drive letter `C:` or `c:` is only another file in the cache. A long name is `std`'s to prefix, and a UNC name is left as it is by `normalize_path`.

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
