# rstex: how a release is built

DESIGN.md section 8 asks for `opt-level=3`, `lto="fat"`, `codegen-units=1`, `panic="abort"`, `+crt-static`, no position independence and PGO. The first four are in `Cargo.toml`'s release
profile, the next two are in `.cargo/config.toml`, and PGO is one command, on stable Rust:

```sh
rustup component add llvm-tools         # llvm-profdata must be rustc's LLVM's, not the system's
python3 -I tools/pgo.py                 # leaves <target>/pgo-build/final/rstex and prints a table
```

`tools/pgo.py`'s docstring says what each directory under `<target>/pgo-build` holds. Everything stays under `target/`, because a profile derives from what it was
trained on. Under a load of 10 it took 4 minutes to the binary and 12 with the table; under a load of 40, 17 and 56. `--no-bench` stops at the binary. On a tree with no cache of
the oracle's results, the first run also compiles each training document with pdflatex, about 10 more minutes.

## What a release does

1. A TeX Live with `pdflatex`: the training documents' side files come from the oracle, through `tests/corpus/run.py`'s cache.
2. `refs/corpus`, for the long public documents: `tests/corpus/public/fetch.py` and the arXiv sample of `tests/corpus/arxiv/`. Without it the training set is the ladder and the two
   format builds, and the tool says so.
3. `python3 -I tools/pgo.py --train public`. A release does not depend on anyone's private documents, and loses nothing by it: see below.
4. `tests/gate.sh release --engine <target>/pgo-build/final/rstex --only integration --only compare --only trip`, since PGO changes the code generated and the binary shipped
   is the binary judged. Plain `tests/gate.sh ID` on the same tree adds `cargo test` and TRIP on a checked build.
5. Ship it stripped, `strip -o rstex <target>/pgo-build/final/rstex`, and keep the unstripped one for symbolising reports.

A profile is only good for the code it was trained on: a function that changed loses its counts, and nothing says so. So a release runs the whole tool on the tagged commit.
The binary is not reproducible from one training run to the next, since the counts differ by a little.

`-enable-ext-tsp-block-placement` is a hidden LLVM option. If a later rustc refuses it the tool stops, and the flag can go at a cost of a few percent.

Windows and macOS: `-Cprofile-generate` and `-Cprofile-use` exist there, but this tool has only been run on Linux. M7's CI runs it per target or ships those without a profile.

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
  instructions. But `crtbeginT.o` then registers `.eh_frame` and libgcc walks all of it, 280,000 instructions on every start, and about 150 more for each function added: the native diagram path's 145 functions came to 0.39% on plain-hello.
  So the linker sends that object's two references to symbols defined as 0, which it tests before it calls: `--wrap` and `--defsym` in `.cargo/config.toml`. Nothing is registered, and an unwinder finds `.eh_frame_hdr` through `dl_iterate_phdr`, for the test profile's panics and any backtrace. That takes plain-hello from 5.34 to 4.96 M and hello from 46.8 to 46.3 M. The binary also gives up ASLR of the image. On, for the faults and the long documents' instructions; `.cargo/config.toml` is
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
