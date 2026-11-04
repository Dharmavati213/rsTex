# The benchmark set

What rstex's final numbers are measured on, and how, fixed before anyone sees them. `select.py` picks the documents by a rule that reads only what the oracle did. A figure quoted from this set says which tier and scenario it is, and gives its distribution, its worst documents and how many of its rows were identical to pdfTeX's.

Nothing in this file is a final figure for rstex, and none is quoted. The numbers here describe the corpus and pdfTeX. They were measured on 2026-11-04 on the laptop, which was under load for most of that (load average up to 30), so they are instruction counts unless a line says otherwise.

```sh
B="python3 -I tests/bench"
$B/census.py measure -j 4          # the oracle's instructions on each document: a last pass, and the preamble alone
$B/census.py diagrams -j 4         # each public diagram alone under pdflatex with real tikz-cd
$B/census.py build                 # target/bs-design/census.jsonl and tree.json
$B/census.py show                  # the distribution quoted below
$B/select.py                       # tests/bench/set.json, set.local.json
$B/select.py --show                # what a set holds, and what it costs
$B/select.py --check               # whether each pin still holds, against the census
$B/baselines.py formats            # pdflatex.fmt for each pdfTeX, by one recipe
$B/baselines.py sample             # the documents the gap is measured on, by a rule
$B/baselines.py gap --sample target/bs-design/gap-sample.txt -j 4    # add --cycles on a quiet machine
$B/baselines.py latexmk --sample target/bs-design/quick-sample.txt -j 4
$B/baselines.py others --sample target/bs-design/quick-sample.txt -j 4
$B/baselines.py report
$B/test_select.py                  # the rule holds what this file says
```

Run these with `-I`, as their first lines do: a file called `select.py` in a directory on Python's path hides the standard library's module of that name. A binary under test must be called `rstex`, as its messages print its name.

`set.json` holds public ids and is tracked. `set.local.json` and `landmarks.local.json` hold the owner's documents by id and are git-ignored, as the manifests' private parts are. They live in the checkout that made them, so copy both when that checkout goes; `target/bs-design/local-copy/` holds the copies of this one. Without them `set.json` is the same file, and a test holds that.

## The census

`census.py` makes `target/bs-design/census.jsonl`, a row per manifest entry, from the oracle's side only: its columns of the whole-corpus records of both machines, each pass's CPU from both machines' oracle caches, the inputs each oracle read, and two runs of its own of every document but the micro ones: one last pass and one preamble alone, under user-mode counters. Packages are read from the names of the tree's files a document opened, which is what its `.fls` lists. Own-text counts are of public documents only. A private row has counts, flags and hashes, and no name or text.

**Preamble and body** can be told, by running the document with a prelude that ends the job at the hook `begindocument/end`: side files read, `\AtBeginDocument` done, nothing shipped. No source is touched. Its instructions, plus the 556 M that the first page of the smallest document costs beyond them (mostly reading the font map, which a run with no page does not do), over the instructions of a last pass, is the preamble share. Of the documents with 3 pages or more, none had a preamble run within 3% of its last pass, so the hook does fire.

| Population | Entries | Normal | Oracle fails | Excluded or no record | What it is, and how it enters |
|---|---:|---:|---:|---:|---|
| real | 1,401 | 1,333 | 55 | 13 | Written to be read. The population of the strata. It includes 63 test files of microtype's suite, which count as `testfile` |
| private | 212 | 179 | 10 | 23 | The owner's. Landmarks, and a stratum of their own, reported apart. 109 of the 179 are on the PC |
| ladder | 20 | 20 | 0 | 0 | The project's own bring-up rungs. Only the 4 that DESIGN section 2 has targets for enter, as landmarks |
| l3build | 1,824 | 1,823 | 1 | 0 | Regression tests, a quarter of a second each at the median on the PC. A small stratum, never pooled with real documents |
| diagrams | 477 | 368 | 109 | 0 | One page per `tikzcd`. They enter through the diagram scenarios, by diagram |
| micro | 10,362 | 10,357 | 5 | 0 | `coverage/` and `extra/`: see below |
| private diagrams | 12 | 12 | 0 | 0 | Not in the set |

**The micro documents** are not workloads: 4,644 load one package and typeset a line, 2,000 are fuzz cases, 1,511 are one language, about 1,450 one primitive. An unweighted figure over them measures process start and the format. They stay what they are, the correctness gate of `tests/corpus/run.py compare`, and are in no tier. Two small exceptions, each marked: the corpus has no real letter, so 22 one-line documents of letter classes from `coverage/cls/` are in the kind `small`, as `synthetic`, with 4 in `standard`, and out of every pooled figure; and the 21 usable `coverage/stress/` cases are listed under `side_stress`, as a side table in `full` on how things scale.

**Real documents with a normal oracle, 1,333:**

| Measure | min | p25 | median | p75 | p90 | max |
|---|---:|---:|---:|---:|---:|---:|
| Pages | 0 | 7 | 21 | 46 | 78 | 7,710 |
| Instructions of a last pass, G | 0.28 | 3.24 | 6.69 | 12.1 | 27.9 | 1,530 |
| Preamble share, of 1,261 under LaTeX with a preamble run | 0.005 | 0.36 | 0.59 | 0.79 | 0.92 | 1 |
| PDF, MB | 0 | 0.27 | 0.45 | 0.63 | 1.0 | 34.7 |
| Peak RSS under pdflatex, MB | 48 | 74 | 77 | 86 | 94 | 155 |

Oracle passes: 1 pass 10, 2 passes 569, 3 passes 635, 4 passes 119. 175 have TeX errors and a PDF, 18 are judged by their failure, 22 have no page. Together a last pass of each is 21.2 T instructions, and about 57 min of CPU on the PC, 170 min with all their passes. Body instructions per page, for documents of at least 5 pages: quartiles 62 M, 95 M and 244 M, p90 503 M, maximum 3.6 G. By source, in documents and T instructions: arXiv 355 and 3.4; notes 210 and 4.7; LaTeX documentation 222 and 3.1; LaTeX3 documentation 119 and 2.2; Stacks 233 and 2.8; manuals 131 and 2.6; books 41 and 2.4; Knuth 22 and 0.03.

arXiv by field: mathematics 270, computer science 35, `hep-*` 27, statistics 13, `q-bio` 10; by year from 2015 to 2026, 26 to 36 each. How many of the 1,333 read each: hyperref 1,029; amsmath 998; graphicx 564; xy-pic 503; theorem packages 463; Latin Modern 444; TikZ 394; microtype 275; csquotes 216; `amsart` classes 192; babel 185; TikZ-CD 134; pgfplots 121; siunitx 77; cleveref 77; natbib 62; listings 53; beamer 52; KOMA classes 45; algorithm packages 38; Times-like fonts 34; long-table packages 34; Palatino-like fonts 28; biblatex 27; PSTricks 13; memoir 8; tcolorbox 8; minted 1; a letter class 0. bibtex ran on 287, a `.bbl` came with 122 more, makeindex ran on 288, and few of those have much of an index: 59 of the eligible have at least 10 `\index`. Type 1 fonts: median 23, maximum 81. Own images: 199 have any, the maximum is 203. Table rows of their own: 90% have at most 22, the maximum is 758.

Machines: the census's instructions are the laptop's, under load, which does not move them. The smallest document came to 1,187 M, as in the earlier figures, and `target/pc/baseline/REPORT.md` has the two machines agreeing within 0.3 M on read-only programs. Times are not from here.

## The rule

`select.py`, from `census.jsonl`, with the seed `rstex-bench-set-1`. No clock, no engine, no result: the same census gives the same bytes, in any row order.

**What it may not read.** `census.py` keeps `ORACLE_COLUMNS` of a corpus record and drops the rest at the door: `candidate_ms`, `candidate_cpu_ms`, `speedup`, `outcome`, `levels` and all else rstex produced. `select.py` stops on a row holding such a column. `test_select.py` holds both, and also that no such name is in the program outside the list that refuses it, and that two corpus files differing only in what rstex did give the same columns. The diagram sample does not ask `commute` what it can handle either: what it declines stays in.

**1. Eligible.** The oracle's class is `normal` (`tests/corpus/run.py`: two clean runs that agree), it is not a test judged by the oracle's failing, a page came out (a regression test may be judged on its log alone), `audit.py` finds it neither vacuous nor a duplicate, and its oracle is current. A document pdflatex fails on is not comparable and stays out by this rule. Nothing in the rule can see a document rstex fails on or differs on, so that one stays, and counts against it by the rule under Statistics. A document with TeX errors and a PDF is `normal`, stays in, and carries `errors` in its row, so a report can split it off.

**2. Cells and features.** A cell is a kind and a size. Sizes by the oracle's pages: `one-page` 1; `note` 2 to 9; `paper` 10 to 39; `long-paper` 40 to 119; `book` 120 to 599; `very-long` 600 and up. Kinds, the first that fits: `testfile` (the l3build suites and microtype's test suite); `beamer` (it read `beamer.cls`); `small` (at most 2 pages, or a letter class); `math` (arXiv `math.*`, notes, Stacks chapters); `cs` (arXiv `cs.*`); `physics-other` (arXiv `hep-*`, `stat.*`, `q-bio.*`); `book` (`public-books`, the Stacks book); `manual` (manuals, LaTeX and LaTeX3 documentation, Knuth's listings).

Features overlap, and each has a floor rather than a cell, all thresholds set before any engine's result was read:

| Feature | Rule on the census |
|---|---|
| `heavy-tikz` | read `tikz.sty` and at least 10 `tikzpicture`s or `\tikz`s in its own text |
| `tikz-cd` | read `tikz-cd.sty`, at least 5 diagrams |
| `xy-pic` | read `xy`, at least 5 uses |
| `pgfplots` | read `pgfplots.sty`, at least 1 axis |
| `microtype` | read `microtype.sty` |
| `biblatex` | read `biblatex.sty` |
| `bibtex` | the oracle's run needed bibtex |
| `index` | the oracle's run needed makeindex, at least 10 `\index` |
| `many-images` | at least 10 image files of its own |
| `huge-tables` | at least 200 table rows of its own, or a long table and at least 50 |
| `mostly-text` | at least 5 pages, in the cheapest quarter by body instructions per page, none of the heavy features above |
| `nearly-all-preamble` | preamble share at least 0.9 |
| `plain-tex` | runs under plain TeX |

Own-text counts exist only for public documents: a private source is not read, by this rule or by anyone working on it.

**3. Families.** A family shares its macros, and often its text: 233 Stacks chapters, 15 builds of the HoTT book. So a family past 12 documents counts as 12 in every share (`FAMILY_MASS`), and a cell takes at most a fifth of its draw, or two, from one family. Two families of separate works are split in two by hash, because each alone supplies a stratum both sides of the hold-out need: the slides of `notes/internal-methods`, and the letter stand-ins.

**4. Landmarks.** Named documents with a reason each, in `LANDMARKS` and in the file, in `standard` and `full` whatever the draw: the Stacks book, HoTT, the KOMA guide, the memoir manual, `source2e`, `source3`, the PGF manual, Napkin, `tex.web`, the hyperref, beamer, tikz-cd and siunitx manuals, `lshort`, the longest arXiv paper and the one with the most pictures, and the ladder documents DESIGN section 2 has targets for. The owner's five are by id in the git-ignored file.

**5. The hold-out.** Whole families, in a fixed random order, each taken while a cell of it is short of 22% of its mass and none goes far over, then for a kind or size with nothing yet the first family that has it, from families that hold no landmark, no document `tools/pgo.py` trains on, and no id a tracked tool or document names. From those families a sample of 110 real documents, in the same proportions, at least 3 of each feature that has them, and 8 regression tests: 121 rows. The rest of those families, 882 rows, is listed as `reserve`, and is in no tier: `full` does not touch them either. So nothing of the hold-out is trained on, and nothing in `standard` shares its family.

Rules for it: no tuning and no PGO training may touch it; a tool that names one of its ids fails `test_no_tool_names_a_holdout_document`; it is measured once, on the frozen engine, after the `standard` report is written, and if a stratum falls outside `standard`'s interval the report says so. Its documents have been through the correctness corpus like all others, and fixes came from there, so it is held out from speed work, not from being seen. Nothing can prove no past command line named one by hand.

**6. Tiers.** All nest but the hold-out.

- `quick`: part of `standard`, the small ladder landmarks, cells by weight, a document for each feature but `pgfplots` (no document that short has one), none over 4 s on the PC but one book.
- `standard`: a draw in proportion to each cell's mass, at least 2 per cell, then each feature up to 10 from what is left, plus 16 regression tests by suite, the square root of a suite's size giving its share. This is the tier reports quote.
- `full`: every eligible document outside the families kept apart.

**True share.** The rule never asks what rstex gains, so it cannot favour what it gains on. Each row has a `weight`, its cell's mass over the documents the tier has of it, so a pooled figure gives a cell its share in the corpus however many documents the floors added. A report checks this after the fact: `standard`'s weighted quantiles against `full`'s on the same run, and a difference beyond the interval is stated, not repaired. A new draw is a new seed, a new version of this file, and every earlier figure stays with the version it was on. A later census is for `--check`, not for a new draw.

**7. Pins.** Each entry has `source_closure`, the SHA-256 of the sorted contents of everything it reads outside the TeX tree, so independent of path and machine, and `tree_closure`, of the tree's files it reads, by path and content. `tree` in the file holds, once, the oracle's banner, kpathsea's version, each `texlive-*` package and version, and the hashes of `pdftex`, `pdftex.map`, `pdflatex.fmt`, `texmf.cnf` and `ls-R`. A result on another closure is not on this set: say so, and count it. Hashes are cut to 20 hex digits.

**Training.** A row has `pgo: trained` if `tools/pgo.py` trains on that very document, and `pgo: family` if on another of its family. A report marks them, and gives each pooled figure with and without them. Today `tools/pgo.py` names 8 public documents in this set: `arxiv/math.AG/1912.07119v3`, `arxiv/math.AG/2007.09646v1`, `books/hott/hott-a4`, `manuals/tikz-cd/tikz-cd-doc`, and `ladder/00-plain-hello`, `03-plain-texweb`, `04-latex-hello`, `15-tikzcd-many`; the rest of the ladder is out of the set. 3 of them are in `quick`, 7 in `standard`, all 8 in `full`, none in the hold-out. 15 more share a family with one: 14 of `books/hott`, 1 of `manuals/tikz-cd`, of which 5 are in `standard`. The private part has 5 trained rows, which are its 5 landmarks, and 29 of a trained family. A change to the training list changes what those marks say, and `test_training_overlap_is_marked` then asks for a new set.

## What the set holds

Each cell is `standard` / `hold-out` / `full`, public documents. Regression tests are one stratum whatever their size.

| Kind | one-page | note | paper | long-paper | book | very-long | all |
|---|---:|---:|---:|---:|---:|---:|---:|
| `math` |  | 11/6/38 | 65/34/237 | 33/14/235 | 3/1/25 |  | 112/55/535 |
| `cs` |  | 4/2/9 | 6/3/15 | 2/1/2 |  |  | 12/6/26 |
| `physics-other` |  | 2/1/6 | 11/6/30 | 2/1/2 |  |  | 15/8/38 |
| `beamer` | 2/0/2 | 1/1/1 | 3/3/18 | 2/2/5 |  |  | 8/6/26 |
| `manual` |  | 16/10/108 | 16/14/130 | 13/2/53 | 8/1/16 | 3/0/3 | 56/27/310 |
| `book` |  |  | 4/0/15 | 1/0/1 | 2/2/4 | 2/2/7 | 9/4/27 |
| `small` | 11/4/56 | 3/3/30 |  |  |  |  | 14/7/86 |
| `testfile` | 14/3/893 | 2/4/145 | 0/1/10 |  |  |  | 16/8/1048 |

| Feature | Eligible documents | `quick` | `standard` | hold-out | `full` |
|---|---:|---:|---:|---:|---:|
| `heavy-tikz` | 67 | 2 | 19 | 12 | 49 |
| `tikz-cd` | 61 | 2 | 14 | 8 | 39 |
| `xy-pic` | 264 | 3 | 31 | 11 | 247 |
| `pgfplots` | 8 | 0 | 6 | 2 | 6 |
| `microtype` | 210 | 5 | 40 | 21 | 134 |
| `biblatex` | 26 | 1 | 10 | 6 | 16 |
| `bibtex` | 285 | 6 | 37 | 8 | 277 |
| `index` | 59 | 2 | 20 | 1 | 58 |
| `many-images` | 54 | 1 | 11 | 8 | 39 |
| `huge-tables` | 16 | 1 | 10 | 3 | 11 |
| `mostly-text` | 229 | 3 | 40 | 26 | 183 |
| `nearly-all-preamble` | 122 | 3 | 17 | 3 | 98 |
| `plain-tex` | 24 | 2 | 10 | 5 | 18 |

| Tier | Public rows | Real documents | Regression tests | Letter stand-ins | Landmarks | Families | With an edit session | Pixels |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| `quick` | 30 | 27 | 2 | 1 | 5 | 26 | 7 | 3 |
| `standard` | 242 | 222 | 16 | 4 | 20 | 136 | 39 | 12 |
| `holdout` | 121 | 113 | 8 | 0 | 0 | 80 | 15 | 0 |
| `full` | 2096 | 1026 | 1048 | 22 | 20 | 353 | 39 | 12 |
| `reserve` | 882 | 114 | 768 | 0 | 0 | 55 | 0 | 0 |

The private part, `set.local.json`, by the same rule and reported apart: 4 in `quick`, 30 in `standard`, 16 in the hold-out, 71 in `full`, 25 in reserve, 5 landmarks, of which 1 is not on the PC and runs on the laptop only. Other rows are documents the PC holds, under 60 s a pass.

`select.py --show` prints this and more. Measured against the population on what the oracle says, each tier by its weights and the population by family mass:

| | | p10 | p25 | median | p75 | p90 |
|---|---|---:|---:|---:|---:|---:|
| Instructions of a pass, G | `full` | 2.24 | 3.33 | 5.80 | 12.1 | 31.4 |
| | `standard` | 2.18 | 3.38 | 6.24 | 12.9 | 35.6 |
| | hold-out | 1.92 | 2.79 | 5.10 | 9.94 | 22.3 |
| Pages | `full` | 5 | 11 | 21 | 40 | 67 |
| | `standard` | 5 | 11 | 21 | 39 | 74 |
| | hold-out | 4 | 10 | 20 | 38 | 71 |
| Preamble share | `full` | 0.15 | 0.36 | 0.60 | 0.75 | 0.85 |
| | `standard` | 0.14 | 0.37 | 0.59 | 0.75 | 0.86 |
| | hold-out | 0.23 | 0.38 | 0.57 | 0.72 | 0.81 |

**One check against an old engine.** Once the set was committed (880355c), and only then, the last-pass ranks of the PC's old whole-corpus record were read once, by a throwaway script, to see whether the documents an old engine gained least on are in their true share. No gain is quoted here, and nothing changed after it. By weight, `standard` holds 10.0% of the population's bottom tenth, 26.6% of its bottom quarter and 47.5% of its bottom half. The hold-out holds 18.8%, 36.1% and 55.2%, so on that old engine it is the harder of the two. The ranks cover 1,022 of the 1,026 real documents in `full`.

Diagrams, public only: of 3,189 taken from documents, 1,453 compile alone under `standalone` with `amsmath`, `amssymb` and real tikz-cd, exit 0, and the others use their document's macros. Of those, 1,231 come from documents people wrote: `tests/cd` and the ladder were written for the native path. Strata are source, arrows (1, 2 to 3, 4 to 7, 8 or more) and whether it uses bends, dashes, hooks and such, at most 4 from one document while there are others. `standard` has 200, `quick` 20, the hold-out 92, from the families kept apart, and `full` every usable one not kept apart. What `commute` declines is not left out: it falls back and counts as such. One diagram alone costs pdflatex 3.0 G instructions at the median, 3.3 G at p90 and 7.7 G at most, nearly all of it loading PGF.

## Scenarios

A row of a report is a document under a scenario. *counted* is one run of each side under user-mode counters, at `-j 14` on the PC, since instructions do not care: instructions, page faults, peak memory, PDF size, and the identical-output check. *timed* is serial on one pinned core with its sibling idle, in rounds with a control: cycles, CPU and wall.

| Scenario | What it answers for a user | Sides | quick | standard | hold-out | full |
|---|---|---|---|---|---|---|
| `first` | I have just cloned this. How long until I have the right PDF? From an empty directory to the fixed point, bibliography and index tools included, as latexmk would. `tests/until_stable/compare.py` holds the two to the same files | latexmk over each pdfTeX; `rstex --until-stable`, running what it names | counted | counted to 30 s | counted to 30 s | counted |
| `last-cold` | What is the engine worth by itself? One pass, side files in place, no checkpoint (`RSTEX_NO_SNAPSHOT=1`, `RSTEX_NO_PAGES=1`), as `tools/prof/bench.py` | each pdfTeX, rstex; timed with rstex again as control | counted; timed to 10 s | counted; timed to 10 s | counted; timed to 10 s | counted; timed but tests |
| `warm` | My second compile, with another word in the body: the preamble restored, proved by `RSTEX_CHECKPOINT_TRACE`, once a run has left its seal. `tools/prof/warm.py` | rstex, with a control when timed | counted; timed to 10 s | counted; timed to 10 s | counted; timed to 10 s | counted; timed but tests |
| `hits` | The same with the old PDF in place: the output cache | rstex, with a control when timed | counted | counted to 30 s; timed to 2 s | counted to 30 s | counted |
| `resume` | I typed near the end: from the last page record, where one is taken. A row that takes none is counted and shown. Refused under SyncTeX (`pages.rs`), so not what Lemma gets as it stands | rstex | counted | counted to 30 s | counted to 30 s | counted |
| `edit` | The save in my editor, generated by `edits.py`: a word near the start, middle and end, a sentence that adds lines, a new citation, a changed preamble, a save that changes nothing, in a mirror under `target/`, with Lemma's flags `-synctex=1 -interaction=nonstopmode -file-line-error -recorder -output-directory=<dir>` and `max_print_line=10000 error_line=254 half_error_line=238` | rstex; latexmk over Arch's pdflatex | counted, `edit` rows | counted, `edit` rows | counted, `edit` rows | counted to 20 s |
| `fmt` | A TeX Live update: `pdflatex.fmt` built | each pdfTeX, rstex | timed, 1 | timed, 3 | timed, 1 | timed, 3 |
| `pixels` | Save to pixels: the file written, the build, Poppler opening it and rendering the middle page. `tools/prof/loop.py` | rstex behind latexmk, rstex direct, its control | | timed, `pixels` rows, 4 rounds | | |
| `diagram-alone` | A figure for a paper: standalone `commute` against a `standalone` document with real tikz-cd under pdflatex, from the stratified sample of `tests/corpus/cd` | `commute`; each pdfTeX | timed, 20 | timed, 200 | timed, 92 | counted, 1,231 less those kept apart |
| `native` | A document with diagrams: native path against real PGF (`RSTEX_NO_COMMUTE=1`) inside it, on rows that read `tikz-cd.sty` | rstex, each way | counted | counted to 30 s; timed to 10 s | counted to 30 s | counted |
| `old-map` | The baselines under the old map | each pdfTeX | | counted to 10 s | | counted |

"30 s" and the like are of the oracle's last pass on the PC. A regression test has only `last-cold` and `warm`, as a suite runs it; latexmk is not how it is run. A plain TeX row has no latexmk: its `first` is the passes the oracle took. `tools/prof` and `tests/until_stable` above are the tools of the performance round, at 8e83f07. `tools/prof/benchset.py` is the driver over what they do: see the next section.

Editing rule, in full, in `edits.py`. A word goes between three plain lower-case words at 5%, 50% and 95% of such places in the document, in reading order over its own files; the sentence and the citation go at the middle blank line; the citation is of the first key by sorting the `.bib` has and the `.aux` does not cite, else the first `\bibitem`; the preamble edit is an unused `\newcommand` before `\begin{document}`. A kind that does not apply to a document is counted, not replaced. A pass that fails on an edit has it taken back and counted. Of the 1,333 real public documents: `noop` applies to 1,332, `preamble` to 1,292, each word edit to 957, `sentence` to 953, `cite` to 562. A row with an `edit` mark in `standard`, 39 of them, and in the hold-out, 15, has all four kinds of body edit. For private rows nothing here reads a source: a driver asks at run time, and reports only the kind and whether it applied.

Each side of an edit runs in its own output directory, and after each save the result is held against the other side by the identical-output check below. `tests/until_stable/session.py` shows how, for two drivers over one engine.

## Running it

```sh
S="python3 -I tools/prof/benchset.py"
$S plan --tier standard                 # what a run would measure, by scenario, and no run
$S run --tier standard --engine plain=BIN --engine pgo=BIN --commute BIN --method counted -j 14 --out target/bench/standard.jsonl
$S run --tier standard --engine plain=BIN --engine pgo=BIN --commute BIN --method timed --cpu 15 --max-load 1.5 --out target/bench/standard.jsonl
python3 -I tests/bench/report.py target/bench/standard.jsonl --md target/bench/standard.md --json target/bench/standard.json --estimate
python3 -I tests/bench/selfcheck.py --engine BIN
```

A run appends to its file, so kill it and run the same line again. It builds what is missing per binary: `pdflatex.fmt` and `pdftex.fmt`, and each pdfTeX's by `baselines.py`'s recipes. It copies a binary under test to `prof-runs/bench-bin/<hash>/rstex`, because its messages print its name. Rows go to `--out`, and documents run in the corpus's own `target/corpus/w`, one path per document, of the length of the oracle's own path. It does not write to the oracle cache unless `--prepare` says so: without it, a document with no valid oracle here is a row saying that. `--ids` and `--scenarios` narrow a run. Without `--no-private`, it reads `set.local.json` too.

**What a row is.** `id`, `scenario`, `variant`, `side`, `method` and `round`, and those make its `key`. `instructions`, `cycles`, `page_faults`, `wall_ms`, `cpu_ms`, `maxrss_kb`, `rc`, `pdf_bytes`. `check` and how it was got. `binary_sha`, `source_closure`, `tree_closure` and `pin_ok`, `tree`, `machine`, `governor`, `load_before`, `load_after`, `sibling_busy`, `clock_ok`. A file also has a `run` line per run, with the sides' hashes and formats, the tree, the plan and what a run costs that does nothing, a `job` line per document with what it took around its runs, and an `end` line. Nothing in any of them is text from a document.

**What a scenario runs,** where the plan above leaves it open:

| Scenario | Rows | In what order |
|---|---|---|
| `first` | one per side | A mirror under `target/`, one output directory `.build` for all sides in turn, emptied between them. latexmk over each pdfTeX and `rstex --until-stable` with `-interaction=nonstopmode -file-line-error -recorder -no-shell-escape`, no SyncTeX, Lemma's environment, what rstex names run and counted in its row. Checked against latexmk over Arch's. A plain TeX row: the passes the oracle took, on every side |
| `last-cold` | one per side; timed, a round is each rstex, each pdfTeX, each rstex again as `a2`, turning round | The oracle's last-pass files, the corpus's argv and environment, `RSTEX_NO_SNAPSHOT` and `RSTEX_NO_PAGES` |
| `warm` | `write`, `restore` | An empty checkpoint directory, `RSTEX_CHECKPOINT_POLICY=always`, `RSTEX_NO_PAGES`. The write, 2.1 s, a run that leaves the seal, then the restore with `RSTEX_CHECKPOINT_TRACE`: a row says `restored`. On the two documents tried, the trace cost no more than a run differs from the next. The same source as the oracle's, so that it can be held against it: a checkpoint does not care about a word in the body, and `edit` is where a word moves |
| `hits` | one | The restore again with its own last PDF in place |
| `resume` | `write`, `last` | Without `RSTEX_NO_PAGES`, its last PDF in place: the chain written, then `RSTEX_PAGE_READONLY`. A row says how many records there are and which it used. One that used none stays, and is counted |
| `edit` | `settle` and one per kind, for each rstex and latexmk over Arch's | A mirror, an output directory per side, renamed to `.build` for its turn, so that paths are the same. Each side to its fixed point; rstex twice more with 2.2 s between, for a checkpoint and a seal, since a user has those from the second or third save on. Then the seven saves, each on what the last left. A kind that does not apply is a row saying so. If latexmk's run ends otherwise than it did at the start, the edit is taken back and counted. A document that ends in an error stops after one pass on both sides without having settled, and latexmk with nothing changed does not run again where rstex does: so there latexmk is forced with `-g` wherever rstex runs on nothing changed, or a later save would hold passes of different number against each other |
| `native` | `native`, `pgf` | As `last-cold`, on documents the oracle saw read `tikz-cd.sty`, the second with `RSTEX_NO_COMMUTE` |
| `old-map` | one per pdfTeX | As `last-cold` with `TEXFONTMAPS`. The log names the map, so only the PDF is judged: one that differs has no old-map ratio |
| `fmt` | one per side | In an empty directory, from the recipes. A format's bytes are nobody's to compare, as in `tests/corpus/run.py`: it has to exist, and the log has to agree with Arch's |
| `diagram-alone` | one per side and diagram | `census.py`'s `standalone` source from the entry's text. commute's file is plain, with base fonts, so the check is the page's size, its text in order and its pixels at 200 dpi, each channel within 64 of 255, and a verdict of `same-picture`, not `identical`. commute's exit 1 is `declined`, and charged |
| `pixels` | `direct`, `direct-aa`, `latexmk`, `poppler` by round | `tools/prof/loop.py`'s way: the same preparation, then a save, each configuration in an order that goes round, and Poppler by `ctypes` on the PDF. Each state is also built by latexmk over Arch's, once, and held against it |

"30 s" and the like are `minstr` over the set's `pc_rate_instr_per_ms`, so that nothing but `set.json` says which row runs: `select.py`'s estimate took the PC's own CPU. A `testfile` has only `last-cold` and `warm`.

**The check.** On every row. A row's outputs are hashed, and a verdict is kept per document by hash, so ten runs with the same bytes cost one check. The verdicts are `tests/corpus/run.py compare`'s: `identical`, `differs-content`, `differs-structure-only`, `differs-log-only`, `candidate-crash`, `candidate-timeout`, by its rules on the log, the side files, an unwritten side file, new files in the source directory and the exit status. Its L4 and L5 go first through the normalized bytes of its L6, with streams inflated and the banner, `/Producer`, `/ID` and offsets masked, and `qpdf --check` for what those masks hide; `compare_pdf.py` runs only if those differ, so a document with no difference costs milliseconds and not `compare_pdf.py`'s 0.07 s a page. `selfcheck.py` has the two agreeing on documents. L1, L2, L6 and rasters are the corpus gate's, and are not repeated per row. A SyncTeX file is held against latexmk's too, and reported, but is not in `identical`: DESIGN section 10 names the difference inside a diagram on the native path as an exception, and `report.py` names any document without a diagram whose file differs.

**Peak memory.** `ru_maxrss` from `wait4` holds the peak of the address space an `exec` replaced, whether by `fork` or by `vfork`. So a child of a Python process holding the manifests reports that process's size, at the least. `benchrun.py` is the small process runs come from. A row cannot show less than its floor, in the `run` line: 7.6 MB, a plain run of `/bin/true` here. Spawning from something that small is also why one run can give the instructions, cycles, CPU and wall together.

**Clocks.** A row is `clock_ok` only if it is timed, the load before and after it is at most `--max-load`, the governor is `performance`, and the core's sibling was idle while it ran, where it ran long enough to tell. Otherwise the instructions stand and report.py quotes no clock. Counted rows have cycles too, and they are not quoted. Pinned to one core, an engine's threads have that core alone, as they did under `tools/prof/bench.py`, which is against rstex.

**Refused.** A run stops if its file holds rows of another binary under a side's name, and leaves a document out, with exit status 3, if the file's rows for it are on another input hash. report.py refuses and counts a row from another binary, another input of the document or another tree, and gives no report at all if two binaries share a side's name across runs. A run on a machine whose tree differs from `set.json`'s says so in its first line and in the `run` line, and its rows are on another tree than the pins.

`selfcheck.py` holds all this: A/A, a binary slowed by a known count, one wrong byte, a killed run, the refusals, the selection byte for byte, and more that its first lines list.

## What a tier costs

| Tier | Public, at most | Public, rstex at half that | of it counted, CPU over 14 cores | of it timed, serial | Private part adds, at most |
|---|---:|---:|---:|---:|---:|
| `quick` | 8 min | 6 min | 0.5 h | 6 min | 2 min |
| `standard` | 90 min | 63 min | 6.1 h | 63 min | 13 min |
| hold-out | 29 min | 21 min | 2.2 h | 20 min | 5 min |
| `full` | 7.4 h | 5.4 h | 40 h | 4.5 h | 21 min |

How the table above was got, before any driver existed, and it is an estimate, since nothing had run then. A document's seconds are its oracle's CPU per pass in the PC's own cache (read by ssh into `target/bs-design/pc-oracle-cpu.jsonl`), made under some `-j`, so on the long side. A document the PC has not comes from its instructions here at 5.69 G a second, that cache's median over documents of a second or more. A run of rstex is taken as a run of pdflatex, as the rule may not know better: that makes the first column a bound, and the second says what half would come to. `PLAN` and `SCENARIOS` in `select.py` give which sides of which scenario run on which documents up to how many seconds of a pass; they are in `set.json` too. Counted work is divided by 14 cores, and adds the check of each row at 0.1 s and 0.07 s a page (82 s on the 1,180 pages of the PGF manual). Timed work is one round per document, or three under 0.5 s, serial.

So `standard` is over an hour only if rstex is as slow as pdflatex. To stay near an hour, it does not time documents over 10 s a pass, which are counted: 12 of its rows, the landmarks among them, get cycles and wall from `full`. `first`, `hits`, `resume` and `native` stop at 30 s, so the 6 longest there have only `last-cold` and `warm`, counted. If a real run is much longer than this, say so and cut `PLAN`, not the documents.

Preparing the PC, which has none of it today: the official TeX Live under `target/texlive-official`, `target/pc/baseline/oldmap`, and biber, which it lacks, so a biblatex row under `first` or `edit` has no bibliography until it does. Its `pdftex` and `pdftex.map` have the same hashes as the laptop's.

## Baselines

A ratio needs a denominator, and pdfTeX is not one program.

| Baseline | What it is | Why |
|---|---|---|
| Arch's pdflatex, current map | The oracle: pdfTeX 1.40.29, `--disable-dump-share`, 64-bit integer, 16-byte word, 42,419-line `pdftex.map` | What every identical-output check is against |
| Official pdflatex, current map | TeX Live's binary, 32-bit integer, 8-byte word, on Arch's full tree, with its own formats. `baselines.py` shows how | What most people run |
| Each, old map | `TEXFONTMAPS=target/pc/baseline/oldmap`, 4,734 lines, which stands for the 4,400 or so the tree had | A smaller install, and what earlier figures were on |
| latexmk over each | `-norc -pdf -pdflatex=...`, from nothing, and a rerun with nothing to do | What a person types |
| lualatex, xelatex | A side table on documents they compile | Their output differs, in fonts and in line breaks, so this is not the same work and no ratio to it may be quoted as a speed-up |

**The gap, measured.** `baselines.py gap` on 70 documents by a rule from the set: all of `quick`, the first of each cell and feature of `standard` in a fixed order, the landmarks under 400 G instructions, the owner's 5 landmarks. 65 public, 5 private, 7 of one page, 15 notes, 25 papers, 11 long papers, 9 books, 3 very long; 280 runs. Instructions, user mode, one run each, on the laptop under load. Each pdfTeX has formats it built itself by one recipe, on one tree, under one environment. Arch's count here is within 0.4% of the census's with the installed format, median 0.02%.

| Official over Arch's, instructions | n | geometric mean | min | p25 | median | p75 | max |
|---|---:|---:|---:|---:|---:|---:|---:|
| All, current map | 70 | **0.917** | 0.884 | 0.900 | 0.915 | 0.929 | 0.990 |
| All, old map | 70 | **0.905** | 0.871 | 0.894 | 0.902 | 0.909 | 0.963 |
| LaTeX, a pass of 20 G or more | 16 | 0.900 | | | | | |
| LaTeX, under 5 G | 19 | 0.926 | | | | | |
| Plain TeX | 5 | 0.977 | | | | | |
| `heavy-tikz` | 5 | 0.894 | 0.884 | | 0.896 | | 0.904 |
| `tikz-cd` | 5 | 0.892 | 0.886 | | 0.889 | | 0.902 |
| `pdflatex.fmt`, a build, 71.95 G and 64.52 G | 1 | 0.897 | | | | | |

So the official build runs about 8% fewer instructions than Arch's on these documents, 10 to 11% on macro-heavy ones, and 2% under plain TeX. It is 69 M less on the smallest LaTeX document and 41 G less on the largest in the sample. The sample's cells are not in proportion: by `standard`'s weights, over the 61 rows that are in it, the mean is 0.920. Peak RSS of the official build is 0.70 of Arch's, median 0.69. Page faults were 0.93, but they move with the machine's state on a loaded laptop: read them from a quiet run. Exit status: 15 of the 70 end in 1 on all four sides, as they do under the installed format, and one more only under the old map, below. PDF sizes agree within 256 bytes between the two pdfTeX on all 70. Their contents were not compared here.

**What it means.** A ratio of rstex to Arch's pdfTeX in instructions, divided by about 0.92, is the ratio to the official one, and by about 0.90 on long macro-heavy documents. Whichever a report quotes, it says which. What this does **not** say is the gap in time. Instructions cannot see the difference an 8-byte memory word makes to the cache, and it may be larger in cycles than here. So no time ratio against the official build is known until `baselines.py gap --cycles` has run on the quiet PC, in a round with a control, which it has not. Until then a time ratio against Arch's has to say that it may flatter rstex, by an amount not yet measured.

**The map.** The current map costs either pdfTeX about 500 M at each start that ships a page: median 499 M for Arch's, 509 M for the official, 465 M at the least. That is 60% of the plain hello, 42% of the LaTeX hello, 0.1% of a book, so a claim about a small document depends on it more than on anything rstex does. Old map over current, Arch's: geometric mean 0.873, minimum 0.393, median 0.926. Which one a user has depends on what they installed, so both are reported. The old map is not a whole map: under it `arxiv/stat.ML/2002.06694v2` gives no PDF and `arxiv/math.AG/2307.01711v2` another size, 6.7 G cheaper. A row like that has no old-map ratio. These two are in the means above, and move them by little. The rest of the gap between DESIGN's old figures and now, about 143 M, `target/pc/baseline/REPORT.md` puts down to the larger `ls-R`, without having proved it. It stays in both baselines here.

**latexmk over each,** on the 28 real public documents of `quick`, counted with all its children from an empty directory, then again with nothing to do: 19 of the 28 end with exit 0 under Arch's. From nothing, official over Arch's has a geometric mean of 0.931, from 0.902 to 0.960. The rerun, which is mostly latexmk's own Perl, costs 500 M at the median, 421 M at the least. The largest, 39.6 G, is a document that latexmk ran again. So a save under latexmk starts 0.4 to 0.5 G behind, whichever engine it runs.

**lualatex and xelatex,** side table only, on the 26 of those that are LaTeX, the second pass from nothing against pdflatex's second pass, instructions. Their output differs, in fonts and in line breaks, so this is not the same work, and no ratio to them may be quoted as a speed-up of rstex. Where exit 0 and a PDF came out on both, lualatex on 13 of 26: geometric mean 1.86 of pdflatex, 1.19 to 3.45. xelatex on 15 of 26: 1.17, 1.03 to 1.63. The first pass of lualatex with an empty cache was about 90 G in a trial, which is why the second pass is what counts. These runs are a sample of what pdfTeX compiles, so this says nothing of documents written for those engines.

## Statistics

Every figure names its tier, scenario and baseline, is on the set's pinned closures, and comes with these:

- **Per stratum:** the geometric mean of the ratio over documents, per kind, per size, per feature, with a bootstrap interval over documents: percentile, 95%, 10,000 resamples within the stratum, seed fixed. A geometric mean because ratios multiply, and an interval because 20 documents are not a population. A stratum under 10 documents gives its rows and no interval.
- **The whole distribution:** minimum, quartiles, p90, maximum, and the worst ten documents by id. Never a best case alone, and never an unweighted mean over the corpus: 6,400 of its entries load one package and typeset a line.
- **Pooled, once, with care:** weighted by `weight`, with the interval from resampling families within cells, as documents of a family are not independent. With and without the rows `pgo` marks. Regression tests and letter stand-ins are not in it.
- **Instructions are the exact measure.** User-mode, from outside the process, `tools/prof/proflib.py`. They do not see the kernel, a page fault's cost or the cache, so they are not the time.
- **Paired cycles with an A/A control:** in each round A, B and A again in an order that turns round, `tools/prof/compare_cycles.py`. The median over rounds of B over the A beside it, next to A over A. If the control is more than 1% from 1 that day, cycles are not quoted.
- **CPU and wall** only on a quiet machine, under a load guard: the PC, governor `performance`, CV under 1% when quiet, one pinned core and its sibling idle, `--max-load`, the load recorded per row. The laptop gives instructions only while anything else runs.
- **Also per row:** peak memory, page faults, PDF size.
- **Only right answers count.** Every row has the identical-output check: `tests/harness/compare_pdf.py` on page content, structure and normalized bytes, and the log, against the same side of Arch's pdfTeX, as `tests/corpus/run.py compare` does. Next to every figure stands `N / M identical`.
- **A row that differs or fails counts against rstex.** It stays in M and is shown by id. It has no ratio of its own, as a wrong answer has no speed, so it is charged: what the attempt took plus the baseline, over the baseline, since that is what a user who has to fall back pays. A crash or timeout with nothing to time is charged its time or the limit, plus the baseline. The figures are given charged. An uncharged figure may be given beside it, not instead.

Reports quote against the official pdflatex first, then against Arch's. A figure against Arch's alone is marked as such. A claim about small documents says which map.

## What the set does not represent

- **TeX users.** It is the corpus, not a sample of what people compile. The arXiv sample has quotas chosen for this project: 80 of 400 in `math.AG`, 300 in mathematics. So weights give the corpus's shares, and no one can say what a typical user's are.
- **Documents pdfTeX cannot compile,** whatever the reason, such as being written for LuaLaTeX or XeLaTeX, needing shell escape, or a package or font this tree lacks: `oracle-fails`, out by the rule. That is 55 of 1,401 real public entries and 10 of 212 private ones. The arXiv sample lost 35 of 390 this way.
- **Letters.** The corpus has no real letter, so the kind holds real documents of at most 2 pages and one-line documents of letter classes from `coverage/cls/`, marked `synthetic`.
- **biber** in the census: `prepare` does not run it, so biblatex documents without a `.bbl` have an unresolved bibliography there. latexmk runs it where installed in `first` and `edit`.
- **Images at scale, non-Latin scripts, generated sources** are thin or absent: see `many-images`, whose largest document has about 200.
- **Disks and shared file systems.** A warm page cache unless a run says `--cold`. Viewer time is only in `pixels`.
- **Other machines.** Two Zen 3 CPUs.
- **How a project is really built.** A real build of the Stacks project is its Makefile, which gives each chapter the others' `.aux`, and not one chapter from nothing.
- **The private part cannot be reproduced by anyone else.** It is reported apart, by id and number only.
