# Corpus runner

`run.py` compiles a document set with the pdfTeX oracle and with a candidate engine and reports, per entry, whether they
agree and where they first differ. Results are always a pass count over a stated denominator. A hand-picked set proves little, so
this runner is built for a large corpus nobody tuned the engine against: an entry that cannot be compared stays listed, with the reason.

Python standard library only, plus `pdftex`, `pdflatex`, `bibtex`, `makeindex`, `qpdf`, `pdftoppm` and `tests/harness/compare_pdf.py`.
It supersedes `tests/harness/run_ladder.sh` as the driver: the rungs are now data in `manifests/`.

```sh
R="python3 -I tests/corpus/run.py"
$R prepare                                      # compile every entry twice with the oracle, classify, cache
$R compare --engine target/release/rstex        # the gate: both modes, every level
$R smoke --engine E                             # bring-up: under a minute, 30 entries, one line per failure. Not the gate, and it says so
$R compare --engine E --bringup --tier standard # the first months of an engine: last pass, no L1, L6 or log gate. Not the gate either
$R compare --engine E --only-failed-last-run
$R compare --engine E --resume                  # continue the latest run after an interruption
$R boxlog ladder/13-hyperref --engine E         # L2 on demand: first differing box-log line
$R bench --engine target/release/rstex -n 5 --cpu 3 --max-load 2
$R coverage --trace-sample 50
$R report                                       # reprint the latest compare summary
$R selfcheck                                    # prove the runner can pass and can fail

python3 -I tests/corpus/mutate/matrix.py        # 63 realistic engine bugs through the real compare: are they caught?
python3 -I tests/corpus/audit.py                # which entries are vacuous or duplicates; compare's "substantive" line reads this
python3 -I tests/corpus/census.py pdf           # and ops, nodes, type1, report: what the corpus never exercises
python3 -I tests/corpus/snapshot/run_snapshot.py --check-mutants     # the preamble snapshot group, and that it fails when it should
python3 -I tests/corpus/cd/compare_native.py check-negative          # the native diagram gate
python3 -I tests/corpus/table.py                # the tables in the last section of this file
```

`run.py` runs under `python3 -I`, as its shebang does, because it reads logs and manifests that come from downloads.

## How a candidate is invoked

Exactly like pdflatex, with these arguments and nothing else:

```
ENGINE [--engine-arg...] -fmt=pdflatex -interaction=batchmode -no-shell-escape -output-directory=DIR [-jobname=J] [entry args] main.tex
```

- `-fmt=pdftex` for plain entries, `-ini` in place of `-fmt` for `ini` and `ini-format` entries. An entry can name another format: 33 regression tests under `required/graphics` and `required/firstaid` pass `-fmt=latex`, the DVI format. A wrapper that finds an engine's own formats has to map that name as well, or those 33 fail with no log.
- The current directory is the entry's `cwd`, stdin is `/dev/null`, stdout and stderr go to a file kept on failure.
- L1, L2, L6 and traces replace `main.tex` by a first line that sets parameters and then `\input "main.tex"`. That moves `\everyjob` before the log opens, so those logs are only compared with each other, never with the main log.
- `--fls` adds `-recorder`.

The environment is built from nothing but `PATH` and `HOME`:

| Variable | Value | Why |
|---|---|---|
| `SOURCE_DATE_EPOCH`, `FORCE_SOURCE_DATE`, `TZ`, `LC_ALL`, `LANG` | `1700000000`, `1`, `UTC`, `C`, `C` | Determinism |
| `openout_any`, `openin_any` | `p`, `p` | Untrusted documents. `"trusted": true` leaves `openin_any` at its default |
| `MKTEXTFM`, `MKTEXPK`, `MKTEXMF`, `MKTEXTEX`, `MKTEXFMT` | `0` | It would write outside the work directory, and rstex ports only `mktexpk`. A document needing it fails on the oracle, visibly, and the candidate must fail the same way. `tests/integration/pdffont/pk_mktexpk*` runs `mktexpk` on both |
| `TEXMFHOME`, `TEXMFVAR`, `TEXMFCONFIG` | an empty directory | A file in the user's own trees shadows the distribution's for every later run, and no oracle cache can see that. It happened: a stray `mktextfm` left a `cmr10.tfm` in `~/.texlive` |
| `XDG_CACHE_HOME` | an empty directory per work directory, candidates only | So a compare judges a cold run, which also writes a checkpoint, and min-of-N in bench is not the time of a snapshot restored from the run before. Restored runs are judged in `snapshot/` |
| `TEXINPUTS` | the entry's `texinputs`, then the default | Only if the entry has any |

Every TeX run happens in `target/corpus/w/<16 hex>/o`. All these paths have the same length, so TeX wraps its log at the same columns
for the oracle and the candidate, and a masked path leaves comparable lines.

## Manifests

JSON files in `manifests/`. All `*.json` are loaded by default. `*.local.json` is gitignored and holds entries that read the owner's
private documents; those documents are never copied into the repo. A misspelt field, a duplicate id or a missing manifest is an error.

```json
{
  "source": "ladder",
  "description": "free text",
  "defaults": {"passes": "auto", "l6": true},
  "exclusions": [{"what": "...", "why": "..."}],
  "entries": [{"id": "ladder/13-hyperref", "tier": "smoke", "main": "tests/ladder/13-hyperref.tex"}]
}
```

`exclusions` states a bound on coverage. It is printed with every report. Each entry overrides `defaults`:

| Field | Default | Meaning |
|---|---|---|
| `id` | required | Unique. Used by `--filter`, `boxlog` and the result files |
| `source` | manifest `source` | Reports group by it |
| `tier` | `standard` | `smoke`, `standard` or `full`. `--tier T` selects T and the tiers below it; `--tier-exact` only T |
| `mode` | `latex` | `latex` (pdflatex format), `plain` (pdftex format), `ini`, `ini-format` |
| `fmt` | by mode | Another format name, such as `etex` |
| `main` | required unless `first_line` | Absolute, `~/`, or repo-relative |
| `cwd` | directory of `main` | Where TeX runs |
| `first_line` | none | Literal first line instead of a file, such as `*pdflatex.ini`. Needs `jobname` and `cwd` |
| `prelude` | empty | TeX run before the main file, on both sides, such as `\pdfsetrandomseed 42 `, which makes a nondeterministic document comparable |
| `jobname` | stem of `main` | |
| `texinputs` | `[]` | Extra search directories, before the default path |
| `env` | `{}` | Extra environment. `@OUT@` in a value is replaced by the output directory of the run |
| `args` | `[]` | Extra engine arguments, on both sides |
| `bib` | `none` | `none`; `bibtex` (run after pass 1); `auto` (run if the `.aux` has `\bibdata` and no `.bbl` ships beside the document); `{"bbl": PATH}` (a precomputed file, in place before pass 1) |
| `index` | `none` | `makeindex` runs after pass 1 if there is an `.idx` |
| `seed_files` | `{}` | `{relative output path: source path}`, in place before pass 1 |
| `passes` | `auto` | An integer, or `auto`: stop when a pass leaves every side file unchanged, at most `max_passes` |
| `max_passes` | 6 | |
| `timeout` | 300 | Seconds per TeX pass. The candidate gets this times `--timeout-scale` |
| `expected_class` | `normal` | `normal`, `oracle-fails`, `nondeterministic`. A mismatch with what the oracle did is reported. A `nondeterministic` label survives two runs that happen to agree |
| `compare` | `pdf`, or `log` for ini modes | What the entry is judged on |
| `l6` | `false` | L6 on this entry |
| `gate_log` | `false` | A normalized log difference fails this entry. Implied when there is no PDF |
| `trusted` | `false` | Our own documents that read `../`. Leaves `openin_any` at its default |
| `mirror_dirs` | `[]` | Subdirectories to create in the output directory. Rarely needed: `\include` into a subdirectory is detected from the log and retried |
| `excluded` | none | A reason. The entry is not compiled, and it stays in every report |
| `notes` | none | Free text, carried into the results |
| `log_normalize` | `default` | `l3build` judges the log in the form of l3build's `normalize_log`, see below. Only for the LaTeX team's regression tests |
| `index_style` | none | `makeindex -s` style, such as `gind.ist` |
| `judge_failure` | `false` | A run that ends without a PDF is itself the behaviour under test: the entry stays `normal`, and the candidate must fail with the same log and exit status. A timeout or a signal stays `oracle-fails` |
| `units` | none | A batch entry: the names of what its pages exercise, in page order. The summary counts units as well as entries, and a divergence names the unit, from the page or from the last `[[unit NAME]]` line in the oracle's log |

## Generated coverage tests

`manifests/coverage.json` reaches everything instead of sampling. The generators are in `gen/`, and the documents go to `target/corpus/gen/`, which is not committed, so on a fresh
checkout run this before anything reads the manifest:

```sh
python3 -I tests/corpus/gen/all.py                # about 15 s when its plans are cached, some minutes from nothing
R prepare --manifest tests/corpus/manifests/coverage.json -j 8
python3 -I tests/corpus/gen/reach.py              # which primitives the oracle logs show as reached
```

| Category | One entry is | Judged on |
|---|---|---|
| `coverage/pkg/X`, `cls/X` | `\usepackage{X}` for every `.sty`, `\documentclass{X}` for every `.cls`, under `tex/latex` and `tex/generic` | log and PDF |
| `coverage/babel-option/X` | `\usepackage[X]{babel}` for every language file | log and PDF |
| `coverage/babel-provide/X`, `babel-ini/X` | The same language through `provide=*`, and each locale `.ini` through `\babelprovide` | log and PDF |
| `coverage/font/table-N`, `mapline-N` | 50 fonts that have a `.tfm`, one page per font with the whole character table, found through `pdftex.map` and again through `\pdfmapline` | log and PDF, per unit |
| `coverage/font/notfm-N`, `stub-N` | The fonts with no `.tfm`: `\font` must fail the same way, and then the table again on a stub `.tfm`, so their `.pfb` and `.enc` are really read | log and PDF, per unit |
| `coverage/font/expansion-N` | Font expansion and protrusion on a sample | log and PDF, per unit |
| `coverage/font/alone-NAME` | A font that shares a PostScript name with an earlier map entry but has its own font file. pdfTeX shares one descriptor between such entries, so in a batch that file would never be read | log and PDF |
| `coverage/prim/NAME` | One per primitive: what it reports about itself, value edges, the standard scenario under several values, hand-written probes with their error paths | log and PDF |
| `coverage/prim-value/NAME=V` | A setting that is fixed once output starts, one document per value. `pdfoutput=0` is the DVI path; the `.dvi` compares as a side file | log and PDF |
| `coverage/prim-bare/NAME` | The primitive bare in each mode: the "You can't use" paths and their recovery | log |
| `coverage/prim-image/KIND` | JPEG, PNG and PDF inclusion | log and PDF |
| `coverage/prim-fatal/...` | Paths that end the job, each alone | log and exit status |
| `coverage/prim-ini/...` | INITEX: compatibility mode, `\patterns`, `\dump` | log |
| `coverage/fuzz/NNNNN` | A seeded random document, full tracing | log and PDF |
| `coverage/stress/NAME` | Scale and limits; also a performance case: `R bench --filter coverage/stress` | log and PDF |
| `extra/...` | `manifests/coverage-extra.json`, from `gen/extra.py`: all eight math styles, word lists, every pattern file, `\\meaning` of every name, real format builds, PDF features. See below | log, and PDF where there is one |

`"judge_failure": true` is set throughout: a package that does not load on the oracle stays in the set and the candidate must fail with the same message. The 200 `standard` entries are spread
over every category; the rest are `full`.

**Probes run to their end.** A probe that ended the job early would hide everything after it and leave no PDF. So `gen/primitives.py` bisects each probe on its prefixes against the oracle, finds the block
that ends the job, takes it out, and makes it an entry of its own in `prim-fatal`, holding everything before it and that block. The answer is kept in `target/corpus/gen/prim-plan.json` by
content hash. `gen/fonts.py` does the same for fonts by compiling each one alone once, in `target/corpus/gen/font/plan.json`, and a failure has to repeat to count.

**The fuzzer** (`gen/fuzz.py`) has its own generator, SplitMix64, so a seed gives the same document on any Python: `python3 -I tests/corpus/gen/fuzz.py 17` prints one. Termination holds by construction: a macro
only calls lower-numbered macros, the expansion cost of each is tracked and bounded, and an output routine adds material a bounded number of times. `all.py --seeds N` runs more. A
failing case shrinks with:

```sh
python3 -I tests/corpus/gen/shrink.py --engine target/release/rstex --id coverage/fuzz/00012 --out small.tex
```

That is delta debugging over lines, with both sides invoked as here, while the normalized logs or exit status still differ. `--contains TEXT` and `--same-line` hold it on one failure when a case has several.

**What a candidate cannot pass by design** stays in the denominator: PNG and PDF inclusion until they arrive, `letterspacefont` until VF, TrueType and OTF, the `capacity-*` limits a growable memory removes. The manifest's
`exclusions` say so, and every report prints them.

## Regression tests, in l3build's form

`manifests/public.json` runs the LaTeX team's own suites: every `.lvt`, `.pvt` and `.lit` of `latex3/latex2e`, `latex3/latex3`, and of hyperref, pgf, beamer and siunitx, at
the commits matching what is installed. The reference is the oracle's log on this machine, not the `.tlg` in the repository: those target other engines and a format built from the tree.

These entries set `"log_normalize": "l3build"`, `"gate_log": true` and `"judge_failure": true`, and they follow `l3build check` where it matters for a pdfTeX run:

| What | l3build | Here |
|---|---|---|
| Test directory | Flat copy of support files, test support and tests | The same, in `refs/corpus/l3build/<suite>/` |
| Log width | `max_print_line=9999`, `error_line=79`, `half_error_line=50` | The same, through `env` |
| Clock | `SOURCE_DATE_EPOCH=1463734800`, forced | The same |
| Passes | Up to `checkruns` | `"passes": "auto"`, `"max_passes"` is `checkruns` |
| Search path | Current directory, which is also the output directory | `TEXINPUTS=@OUT@:`, so `\IfFileExists` finds what the test wrote |
| Format | Built from the tree, with its own `hyphen.cfg` | The installed one |
| `-translate-file=ascii.tcx` | Yes | No. The candidate does not implement TCX. The form turns the top half into `^^xx` itself, which is one to one |
| `-interaction` | `nonstopmode` | `batchmode`, like every entry here |

The form is a port of `normalize_log` in `l3build-check.lua`, keeping the rules that can fire on a pdfTeX log: only the text between `START-TEST-LOG` and `END-TEST-LOG`, no `OMIT`...`TIMO`, lines rejoined, paths, dates, versions, line numbers and register numbers masked. The output directory counts as `.`.

Each of those masks hides something an exact port would reproduce anyway, so `compare` also measures the whole log in the default form. A difference there goes in the note, and it gates as `differs-structure-only` under `--gate-log`. A log with no `START-TEST-LOG` has no test body, and an empty form would pass, so it is judged whole instead.

`public/check_normalize.py lua` runs the port against `rewrite_log` from the installed l3build on every prepared log. `public/check_normalize.py tlg` compares the oracle's normal form with each suite's `.tlg`, which shows the tests really ran as they were meant to.

To select this source, use `--manifest tests/corpus/manifests/public.json`. Its entries have sub-sources (`public-stacks`, `public-latex2e-tests`, `public-books`, ...), so reports split by them. `public/gen_manifest.py` writes the manifest, `public/fetch.py` downloads what `public/sources.json` pins into `refs/corpus/`. Run them in that order to rebuild everything.

## Stages

**`prepare`, the oracle.** Compile with `pdflatex` or `pdftex` for the required passes, with `-recorder`, from an empty directory, twice, in
two different directories.

- *Oracle directory:* `target/corpus/oracle/<slug>/<config hash>-<closure hash>/`. The configuration hash covers the entry's fields, the
  oracle binary and the determinism settings. The closure hash covers the content of every file in the `.fls` of every pass (TDS files, the
  format and `texmf.cnf` included), the names in `cwd`, and `.bib`/`.bst` in `cwd` when bibtex runs. A later run re-hashes these
  and reuses the directory only if nothing moved.
- *Contents:* `final/` (PDF, log, `.fls`, side files), `pre_last/` (side files as they were before the last pass), `gen/1/` (bibtex and makeindex
  output), `seed/`, `meta.json`, and cached `l2/`, `l6/`, `raster-*.json`, `trace.json`.
- *Bibliography:* the system `bibtex` and `makeindex` run here only. A candidate gets their output replayed at the same point.
- *Classes:* `oracle-fails` when both runs fail (timeout, signal, or no PDF with an error). `nondeterministic` when the two runs differ in pass
  count, side files, or PDF after normalization; or in the log, where the log is what is judged. Otherwise `normal`. An entry with TeX errors
  and a PDF is `normal`: error recovery is behaviour to reproduce. An entry with no pages of output and no error is `normal` too, and is judged on its log.
- *Failures:* a failure is recorded and cached, never deleted. `--retry-oracle-fails` recompiles it; `--force` recompiles everything.

**`compare`.** With `--mode last-pass` (the default) the candidate starts from `pre_last/` and does one pass, so both sides read the same
`.aux`. With `--mode from-scratch` it does the oracle's number of passes by itself, from `seed/`, with `gen/1/` replayed after pass 1. `both` does
both, and reports them separately.

By default every level below runs, in both modes.  Each default exists because `mutate/matrix.py` found a real bug class that passes without it: a 1 sp glue error shows only in L2, a macro nothing uses only in L1,
a stale `.aux` only from scratch, a wrong line number in a message only in the log.  `--bringup` turns them off for the first months of an engine, and the summary says so in capitals.

| Level | What | How | A difference means |
|---|---|---|---|
| L0 | Every side file, bytes; that the candidate wrote what the oracle wrote in its last pass; `.fls` with `--fls` | sha256, first differing line; restored files carry a marker mtime | `differs-structure-only` |
| L1 | What each macro and register became: `\tracingassigns=1 \tracingrestores=1` | Instrumented pass, entries of at most `--l1-max-ms` per pass | `differs-structure-only` |
| L2 | Box dumps under `\tracingoutput` with depth and breadth maximised | Instrumented pass; `boxlog ID` on demand | `differs-structure-only` |
| L4 | Normalized page content, images and forms with their dictionaries | `compare_pdf.py --no-raster` | `differs-content` |
| L5 | Every page attribute and resource, Info, catalog, the outline tree with back links, dests, fonts, page tree, trailer | `compare_pdf.py` | `differs-structure-only` |
| L5 container | PDF header, xref kind, object count, object streams, which streams are compressed, objects nothing refers to, `qpdf --check` | `compare_pdf.py` | `differs-structure-only` |
| L6 | Bytes under `\pdfcompresslevel=0 \pdfobjcompresslevel=0 \pdftrailerid{}` | Instrumented pass; turn off with `--no-l6` | `differs-structure-only` |
| L7 | `pdftoppm -r 100`, every page | Hash per page from one streaming process, oracle hashes cached | `differs-content` |
| log | Whole log, normalized | Always measured; off with `--no-gate-log` | `differs-content` where the log is what the entry exercises, otherwise `differs-log-only` |
| exit | Exit status | | `differs-structure-only` |
| source | No new file in the entry's `cwd` | Names before and after the run | `differs-structure-only` |

**The instrumented pass.** One extra pass per entry on each side, from `pre_last/`, with all of L1, L2 and L6 turned on at once, so they cost one compile and not three, and with `max_print_line=1000000`. The log can reach hundreds of MB (450 MB for a 400-page Stacks chapter, over 1 GB for a paper that loops in pgfmath), so only a hash stays in the oracle directory, computed in a process of its own by `lognorm.py`; a difference makes a new oracle run to find the first differing line. The oracle runs it twice: a trace can move between clean runs where the page does not, and such an entry's L1 and L2 say `unavailable`, with the count in the summary. If the prelude changes the oracle's exit status the levels are `unavailable` too. L1 is left out where the oracle takes more than `--l1-max-ms` (2000) of CPU per pass; the summary counts those entries. CPU and not wall clock, so what L1 judges does not depend on how busy the machine was when the oracle was prepared. A file over the 2 GiB limit also leaves that entry's levels `unavailable`.

L6 reports `equal`, or `equal-masked` when the only difference is the version string in `/PTEX.Fullbanner` or `/PTEX_Fullbanner`, `(rstex)` for the oracle's, with the xref offsets its length moves, since an engine with
its own banner cannot avoid that. Such an entry is `identical`, and the summary counts the two apart. A mask hides the offsets, so `equal-masked` also needs `qpdf --check` to find nothing wrong with the cross-reference table. A banner a document copies somewhere else,
hyperxmp's metadata for one, or its CRC in `\pdfuniqueresname`, is no longer only that string: those entries are on `known_divergences.txt`. A build with the feature `oracle-banner` has the oracle's string, and with `tests/gate.sh ID --oracle-banner` a document's L6 must be plainly `equal` or `equal-inflated`. A document can turn compression back on after the prelude, as `pdfx` and `hyperxmp` do, and then L6 would be comparing deflate output. So where the bytes differ and a stream is compressed, both files are compared again with their streams decompressed, each `/Length` and the body of a cross-reference stream masked, and agreement there is `equal-inflated`, which also needs `qpdf --check` to find nothing wrong. A `/ModDate` of a file the run itself wrote and embedded is the time of the run, which `FORCE_SOURCE_DATE` does not pin: if that is all L5 finds it does not count, and L6 masks it too, for any embedded file. `\pdffilemoddate` itself is compared on the integration cases.

**What the PDF comparison does not judge, on purpose:** object numbers, `/ID`, subset tags, deflate bytes and level (DESIGN section 9 picks its own levels, so only compressed or not is compared; L6 sees numbering and tags). Dates count, because both sides run under one clock.

Log normalization removes the banner, the `%&` and `\write18` notes, the output directory, the memory and PDF statistics, the byte count of the
output file, the program name in `!pdfTeX error:` and `pdfTeX warning:` messages along with the line wrap that name moves, and in format builds the dump statistics. L2 compares only the dumps themselves, from `Completed box being shipped out` to the blank
line ending each one, so a file path in a message cannot hide the first box difference.

## Results

`target/corpus/runs/<run>/` has `run.json`, `results.jsonl` (appended per entry; this is what `--resume` reads), and at the end `results.json`
and `summary.txt`. `--runs-dir` puts them elsewhere. A failing candidate's output is kept under `fail/`.

| Outcome | Meaning | Counted as comparable |
|---|---|---|
| `identical` | Every level that ran agreed | yes |
| `differs-content` | Page content, page count, raster, or the log where the log is judged | yes |
| `differs-structure-only` | Pages agree; L5, L0, L1, L2, L6 or exit status do not | yes |
| `differs-log-only` | Everything else agrees and the log does not: a line number in a message, a dropped warning | yes |
| `candidate-crash` | Signal, exit 100 or above, no PDF where the oracle made one, unreadable PDF | yes |
| `candidate-timeout` | | yes |
| `harness-error` | The tooling failed on this entry; a bug to fix here, not a pass | yes |
| `known-divergence` | It differs, and `known_divergences.txt` accepts that, with a reason. It keeps its first divergence, and what it would have been in `differs_as` | no |
| `oracle-fails` | The candidate runs once, and must stop the same way, with the same log and exit status: otherwise `differs-log-only`, or `candidate-crash` if it crashes or hangs. If the oracle itself timed out, nothing is compared | no |
| `nondeterministic` | The candidate is run to catch crashes, but not compared | no |
| `excluded` | By the manifest, with its reason | no |
| `skipped` | Over the disk budget. Exit status 1, and `--only-failed-last-run` picks it up | no |

The printed summary has counts per tier and per source, `PASS mode: X / Y comparable entries identical`, the same with vacuous entries and duplicates out of the denominator
(`substantive:`, from `audit.py`), and every entry that is not identical with its first divergence: page and operator, then the box-log line. Identical entries are printed up to 100 and
are always in `summary.txt`.

Read a pass count by source. Of the 13,965 comparable entries, about 6,400 only load one package and typeset a line, 2,000 are fuzz cases, about 1,800 are regression tests judged on their log, about 1,500 are primitive and font tests, 366 are diagram documents; what tests typesetting at length is the 1,419 real documents.

Exit status of `compare`: 0 when every comparable entry is identical and at least one entry really was. 1 otherwise. 2 for an empty selection.
130 when interrupted. A SIGINT kills the running candidates and those kills are not recorded; `--resume` runs those entries again. A resumed run must have the
same configuration, the same engine binary included, and an entry whose oracle changed runs again.

## The accepted list

`known_divergences.txt` holds, one per line, an entry that differs and a reason a reader can check, as `tests/integration/known_divergences.txt` does for the integration cases. A crash, a timeout or a tool error is never accepted. How an entry differs is not checked, so a new bug in it would hide: each line says what the difference is. A line that names no entry is an error when all manifests are loaded, and one that now agrees is said to agree in the summary, so that the list does not go stale. `tests/gate_verdict.py` counts them as agreeing, and prints how many.

## Timing

`compare` records wall-clock milliseconds for both sides, but the oracle's figure comes from `prepare` and the machine's load is whatever it was. Those
numbers are for spotting anomalies.

`bench` is the measurement: entries run one at a time, last pass from `pre_last/`, `-n` runs per side (default 5) in alternating order, the minimum kept. It
reports worst, p10, median, p90, best and geometric mean of `oracle_ms / candidate_ms`, plus total time and the median by CPU time, then the same by source and by size of the job. p10 means 90% of entries are at or
above it; p90 means 90% are at or below it.

- **Only right answers count.** An entry counts only if a compare run of this very binary (by hash) found it identical. Without such a run `bench` stops; `--allow-unverified` gets past that and the report says UNVERIFIED. A wrong answer can be fast.
- **The engine's cache is empty on every run.** `--keep-engine-cache` lets it persist and the report says WARM. The first run and the minimum are both reported, so a cache at work shows.
- **`--cold`** drops every file the compile reads, the engine included, from the page cache before every run, with `posix_fadvise(POSIX_FADV_DONTNEED)`, which needs no root. Pages another process has mapped stay. Default is a warm page cache, which measures the engine and not the disk.
- **`--all-passes`** times the whole job from nothing, as `latexmk` would run it.
- **A median over the corpus is not a claim about documents.** 6,400 of the entries load one package and typeset a line, so an unweighted median mostly measures process start and the format. Quote the by-size table, the totals, and the by-source lines.
- The selection is recorded and printed, so a number from `--filter` cannot pass as a number for the corpus.
- `--cpu K` pins both sides to one CPU. `--max-load L` waits until the 1-minute load average is under L before each entry; `--load-patience` bounds the wait, and the load seen is recorded per entry. Without `--max-load` the report says load was not controlled.
- The cost of spawning `/bin/true` is reported as a floor and not subtracted. It matters for claims near a millisecond.
- `-recorder` is not passed on either side.
- Entries that are not `normal` are excluded and listed. So are candidates that fail.

## Coverage

`coverage` aggregates the `.fls` of every oracle: classes, packages, fonts by kind, encodings, map files and file types, each set against what is installed
(`.cls`, `.sty`, `.tfm`, `.pfb`, `.vf` in `ls-R`, `.pfb` named in `pdftex.map`). Logs add pages, errors and box warnings.

Primitives come from a trace: `\tracingcommands=2` from `pre_last/`, with the log as a FIFO read live, so hundreds of MB never reach the disk. `--trace-sample N`
(default 50, stratified by source, seeded), or `--trace-all`. The list is `tests/primitives.json`. Each primitive ends up executed, not traced but mentioned in a loaded file, or neither.

Counts come from the `{...}` lines, and names without a backslash count too, because microtype sets `\escapechar=-1`. Conditional outcomes (`{true}`, `{case 2}`), characters and
pdfTeX's `{/path/x.enc}` notes are not commands. Lines the parser does not understand are reported, not dropped.

The blind spot is real: a primitive read as an internal quantity, such as `\the\pdflastobj`, prints nothing. So "not traced" does not mean "never used", and commands run only while a format
was built are not seen unless a format build is itself an entry. Cross-check: tracing the real format build gives 19,392,987 commands, against 19.39 M in
`docs/research/verification-and-inventory.md`.

## Bounds

Everything here is stated so that nothing is dropped silently.

- **File size:** `RLIMIT_FSIZE` of 2 GiB on every child. A document that exceeds it dies with signal 25 and lands in `oracle-fails` or `candidate-crash`.
- **Runaway orphans:** `RLIMIT_CPU` of 3600 s, for a TeX left behind when the runner itself is killed with SIGKILL. No core files.
- **Disk:** `--disk-budget-gb` (default 40) is checked at the start and every 50 entries. Entries past it are `skipped`. Compilation outputs are kept once, oracle run B only when it differs.
- **L7 pixel share:** computed for the first 5 differing pages, and the result says so. The list of differing pages is complete.
- **`.fmt` bytes:** not compared across engines, which lay out memory differently. Format builds are judged on the log, and on the `.fmt` existing.
- **bibtex:** runs once, after pass 1, on `<jobname>.aux`. Biber is not run. A biblatex document or one with several `.aux` bibliographies needs `{"bbl": PATH}` or `seed_files`. `bibtex` runs with `openin_any=r`, since `p` refuses the absolute path of the `.bib`.
- **The closure hash** cannot see a failed lookup except through the names in `cwd`. A new file elsewhere on the search path needs `--force`. The user's own TeX trees are out of it, see the environment above.
- **Sharing the cache.** `target/corpus/oracle/` is shared by every checkout that links `target/`. Paths under the checkout count as the main tree's in the key, and `refs/` resolves to the main tree's, so checkouts share one oracle of an entry and a log prints the same paths; work directories take the real path of `target/`, so they have one length. An oracle younger than a day is kept whichever key it has; a `run.py` older than this one deletes any other key, under a run in progress. On 2026-11-03 such runs removed about 540 of the main checkout's oracles in one go. A compare that ends in anything but identical on an oracle that has gone runs that entry once more, and the result records it under `retried`. `RSTEX_CORPUS_TARGET=DIR` gives a run a cache of its own.
- **`%&` first lines:** the L2, L6 and trace first line bypasses a `%&-translate-file` request in the document. `-fmt` is always explicit, so a `%&format` line is ignored throughout.
- **`.fls`:** the oracle always runs with `-recorder`. A document that reads its own `.fls` (currfile does) gets `-recorder` on the candidate too; otherwise only `--fls` adds it.
- **Stale side files:** a `.aux` or `.bbl` beside the document can be read by the first pass when the output directory has none. Both sides read it alike. `prepare` lists such entries.
- **The full macro trace** (`\tracingcommands` and `\tracingmacros`), 50 times the size of L1 here, is not part of this runner. Use it by hand once L1 has found the place.
- **Batch mode only.** Every entry runs under `-interaction=batchmode`, so nothing tests terminal output, its line wrapping or interaction other than through TRIP (`trip.fot`). A candidate's stdout is kept on failure and never compared.
- **SyncTeX is not compared.** The owner's editor uses it; DESIGN section 15 puts it in M7.

## Self-check

`run.py selfcheck` runs the real CLI against `manifests/ladder.json`, `manifests/user.local.json` if present, and `fixtures/selfcheck.json`. The fixtures sit outside
`manifests/` so they never count toward a corpus denominator. It needs 7 minutes at `-j 4` and proves:

- The system pdflatex as candidate is identical on every entry, in both modes, at every level.
- A wrapper changing one word in the input is reported as `differs-content`, with a page and operator confirmed by `pdftotext` and by a direct `compare_pdf.py` run, and a box-log line in the same shipout.
- Each wrong candidate lands in its own bucket: signal, panic, no PDF, garbage PDF, hang, extra page, Info-only, aux-only, a side file never written, a file left in the source directory, an image dictionary, a page attribute and an orphan object (L5), a damaged cross-reference table, exit status, log, no log in the instrumented pass. A crash on a document where the oracle stops cleanly counts against the candidate; a clean failure there does not. A document that turns compression back on passes with another deflate and still fails on a changed word. Smoke passes pdflatex in under a minute, stays bounded when every entry hangs, and names the next command on a wrong page. Bench refuses unverified answers and does not count a cache hit as speed. The one check that reads a clock under load, a candidate slowed by 200 ms, can fail on a busy machine.
- `oracle-fails` and `nondeterministic` are recorded and stay visible.
- Selection, the oracle cache and its invalidation, resume after SIGKILL and SIGINT, the disk budget, bench against a candidate slowed by 200 ms, and the microtype trace trap work.
- Nothing changed under the private documents' directories.

## Can it pass while the output is wrong?

`mutate/matrix.py` runs 63 realistic engine bugs (`mutate/bugs.py`) through the real `compare`, under each set of flags. They are made three ways: TeX run before the document; a TFM with one kern or ligature changed, read by the real engine; a same-length byte edit of an uncompressed output. A prelude goes through a copy of the main file, not the first line, and its traces in the log are undone, so a log comparison cannot "detect" the injection instead of the bug. A control with the injection and no bug has to come out identical. A bug that did not apply is reported as such.

Under the defaults every bug here is caught. Under `--bringup`, 12 are missed, and `matrix.py --profile bringup` names them. Deflate levels 1 to 9 are not in the list, because the design picks its own: only compressed or not is compared. Run `gen/extra.py` first: two bugs use its documents.

## Vacuous entries, duplicates, and what is never exercised

`audit.py` gives every entry a verdict and writes `target/corpus/audit/substance.json`, which the summary uses. Rules are in its header: a run that stops at load on a missing file, a regression test with nothing between its markers and no page, an empty run, the same output up to the entry's own name.

`census.py` looks for what `coverage` cannot see, each against a fixed list of what exists: node types and modes in a stratified sample, PDF structure in every oracle PDF, content operators in a sample, and every Type 1 feature in every `.pfb` the map file can reach. `gen/extra.py` writes tests for what it found missing: math in all eight styles by construction, `\showhyphens` over a word list, every pattern file through `\patterns`, `\meaning` of every name in the format and major packages, and PDF features by primitive.

**Line coverage.** All of this measures the oracle's output, and a node discarded before shipout leaves no trace. The answer that needs no list is to build rstex with `-C instrument-coverage`, run the corpus, and map the regions that never ran back to WEB sections, by way of the section numbers kept in the port. Do this at M4. A section nothing executes is then a named gap, and it is a much sharper tool than anything here.

## The snapshot group

`snapshot/run_snapshot.py` is the gate for the preamble snapshot of DESIGN section 7. 343 cases, one group per kind of observation: configuration, lookups and shadow probes, reads, what earlier passes left behind, stat-like primitives, volatile reads, hard taint, the run's own writes, stacked files, what a restore has to bring back, runs with no output directory, the cache itself. Each changes the outside world between runs and asserts that the next run equals a cold compile in PDF bytes, normalized log, side files and exit status, and also what the terminal saw when the cold engine is the same program: `--cold-engine "ENGINE --no-snapshot"`. A case whose change a cold compile cannot see fails as VACUOUS, so a test that could not catch anything cannot pass. The cases of the cache group that have a protocol of their own hold a hostile or damaged file, a race, a kill in mid-run, no HOME and no space; they are for rstex only.

`--check-mutants` proves the group can fail: the system pdflatex must pass all of it, an engine that replays its first run must fail every visible change, and a real preamble dump by `mylatexformat` keyed on the preamble's text must fail where a naive snapshot would. The last gives a realistic idea of what an unsound snapshot gets wrong.

Three cases were traps for section 7 as first written, not for a bug in an implementation, and their `why` says how: `volatile/c-sys-copied-in-preamble`, `volatile/pgfmath-seed-used-in-body` and `read/tds-tree-same-stat`. Section 7 now says what is done about each.

A case tests a restore only if there was one, so each run also reports how many second runs and check runs really restored. rstex says so through `RSTEX_CHECKPOINT_TRACE`, and an engine with no cache reports none.

`snapshot/warm_equals_cold.py` is the other half, on real documents: under `run.py compare`, every engine call runs cold, writing and warm from the same state of the output directory. All three must leave the same bytes, and compare then judges what the warm run left against the oracle. `tools/prof/warm.py` measures the same three.

## The native diagram gate

`cd/extract.py` takes every `tikzcd` from every document in the corpus, the arXiv sample and the owner's documents included, and writes one page per diagram under its own preamble: `manifests/cd.json` and the untracked `cd.local.json`. `cd/compare_native.py eligibility` asks `commute` which it handles. `geometry` is the gate of DESIGN section 10: box dimensions exactly, the text in order, and pixels at 400 dpi, where 0.05 bp is enough to fail; `check-negative` shows that. A diagram an engine declines to real PGF proves nothing about the native path, so the engine has to report those and the tool states the share.

## Adding a source

Put downloads in `refs/corpus/<source>/` and write a manifest. A generator script lives outside the download directory and takes it as an argument. Never run anything from a download. Use `"passes": "auto"`
and `"bib": "auto"`, leave `trusted` off, and put what you cannot cover in `exclusions`.

## The `local` source

Every LaTeX document the owner already has, read in place. `manifests/local.local.json` is gitignored and comes from `gen_local_manifest.py`, in two phases:

```sh
G="python3 -I tests/corpus/gen_local_manifest.py --extra-root thesis=DIR"   # DIR stays out of tracked text, see tools/privacy_guard.py
S="python3 -I tests/corpus/gen_sga4_shim.py"
$S; $S --french                                 # stand-ins under target/corpus/, read from the owner's macro files
$G                                              # phase 1: every master
python3 -I tests/corpus/run.py prepare --manifest tests/corpus/manifests/local.local.json -j 8 --run-id local-1
$G --results target/corpus/runs/local-1/results.json --user-results RESULTS_COVERING_user.local.json    # phase 2
```

- **Masters** are found by content, and whatever else is a fragment. `target/corpus/local-sweep.json` lists each with whether a compiled master reads it.
- **Phase 2** marks a copy `excluded`, when its PDF apart from `/ID` and its private inputs by content equal an earlier entry's, assigns tiers, and records what the oracle did.
- **Stand-ins:** SGA 4 is written for LuaLaTeX, so both versions compile through generated macro files, with their sources unmodified. English uses `texinputs`. French uses `prelude`, because it reaches its configuration by a relative path, which a search path cannot intercept. `aurical.sty` is missing here, so the EGA parts get a one-line stand-in.
- **What the stand-ins do not show:** fonts, language and line breaks differ from what the owner reads. These entries test pdfTeX on this mathematics, and say nothing about how well SGA 4 renders on pdfTeX.
- **Overlap:** a volume and its exposes share their text, so count pages in the denominator with that in mind.
