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
$B/predict.py draw                 # a fresh sample of `full` outside `standard`, to hold `standard` against
```

Run these with `-I`, as their first lines do: a file called `select.py` in a directory on Python's path hides the standard library's module of that name. A binary under test must be called `rstex`, as its messages print its name.

`set.json` holds public ids and is tracked. `set.local.json` and `landmarks.local.json` hold the owner's documents by id and are git-ignored, as the manifests' private parts are. They live in the checkout that made them, so copy both when that checkout goes; `target/bs-critic/local-copy/` holds the current copies. Without them `set.json` is the same file, and a test holds that.

**What a clean checkout can and cannot do.** From an archive of this tree, in another directory and outside any git repository: `select.py --show`, `test_select.py` (those that need the census or git say they were skipped), `benchset.py plan`. Given `census.jsonl`, `diagrams.jsonl` and `tree.json` of `target/bs-design/`, `select.py` gives `set.json` byte for byte, twice, under Python's random hash seeds. Without them it cannot select: the census needs the whole-corpus records of both machines, the PC's oracle CPU and `target/corpus/audit/substance.json`, none of which is in the repository, and it holds private rows, so it never will be. With `target` and `refs` linked in, a run of three documents went from preflight to report. The tools name no absolute path. A different `HOME` could not be tried here, as this session's sandbox refuses a command that sets it; each tool passes `HOME` through to what it runs, the runs have run.py's hermetic `TEXMFVAR`, `TEXMFHOME` and `TEXMFCONFIG`, and the PC is another `HOME` in itself.

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

**3. Families.** A family shares its macros, and often its text: 233 Stacks chapters, 15 builds of the HoTT book. So a family past 12 documents counts as 12 in every share (`FAMILY_MASS`), and a cell takes at most a fifth of its draw, or two, from one family. Two families are split in two by hash, because each alone supplies a stratum both sides of the hold-out need: the slides of `notes/internal-methods`, and the letter stand-ins. The first are not separate works: 11 pairs of near-copies, by word shingles, cross that split.

**4. Landmarks.** Named documents with a reason each, in `LANDMARKS` and in the file, in `standard` and `full` whatever the draw: the Stacks book, HoTT, the KOMA guide, the memoir manual, `source2e`, `source3`, the PGF manual, Napkin, `tex.web`, the hyperref, beamer, tikz-cd and siunitx manuals, `lshort`, the longest arXiv paper and the one with the most pictures, and the ladder documents DESIGN section 2 has targets for. The owner's five are by id in the git-ignored file.

**5. The hold-out.** Whole families, in a fixed random order, each taken while a cell of it is short of 22% of its mass and none goes far over, then for a kind or size with nothing yet the first family that has it, from families that hold no landmark, no document `tools/pgo.py` trains on, and no id a tracked tool or document names. From those families a sample of 110 real documents, in the same proportions, at least 3 of each feature that has them, and 8 regression tests: 121 rows. The rest of those families, 882 rows, is listed as `reserve`, and is in no tier: `full` does not touch them either. So nothing of the hold-out is trained on, and nothing in `standard` shares its family.

Rules for it: no tuning and no PGO training may touch it; a tool that names one of its ids fails `test_no_tool_names_a_holdout_document`; it is measured once, on the frozen engine, after the `standard` report is written, and if a stratum falls outside `standard`'s interval the report says so. Its documents have been through the correctness corpus like all others, and fixes came from there, so it is held out from speed work, not from being seen. Nothing can prove no past command line named one by hand.

**6. Tiers.** All nest but the hold-out.

- `quick`: part of `standard`, the small ladder landmarks, cells by weight, a document for each feature but `pgfplots` (no document that short has one), none over 4 s on the PC but one book.
- `standard`: a draw in proportion to each cell's mass, at least 2 per cell, then each feature up to 10 from what is left, plus 16 regression tests by suite, the square root of a suite's size giving its share. This is the tier reports quote.
- `full`: every eligible document outside the families kept apart.

**True share.** The rule never asks what rstex gains, so it cannot favour what it gains on. Each row has a `weight`, so that a pooled figure estimates the population by mass, whatever went into the tier for other reasons. Within a cell, whose total stays its mass: a landmark has its own mass, as it was taken by name and stands for itself; a row a floor added has none and is marked `entered: floor`, as it was taken for its feature, and it stays in that feature's stratum; a family past `FAMILY_MASS` is a stratum of its own, its mass in the cell shared by what was drawn of it, since the draw takes documents; and what was drawn of the rest shares the rest. Mass with nothing drawn to carry it goes to what was drawn in the cell. The rungs of the ladder and the letter stand-ins are no part of what is estimated. A report checks this after the fact: `standard`'s weighted figure against `full`'s on the same run, and a difference beyond the interval is stated, not repaired. A new draw is a new seed, a new version of this file, and every earlier figure stays with the version it was on. A later census is for `--check`, not for a new draw.

These weights replaced a weight per cell, that is its mass over the documents the tier had of it, before any engine's ratio had been read: `ae8b9b7` was committed before the sample below had run. By that weight Stacks had 10.5% of `standard` against 2.5% of the mass, three more families 5 to 6% each against 2.5%, single arXiv papers 43.7% against 55.4%, `pgfplots` 6.7 times its share and `huge-tables` 2.3. Now arXiv has 57% against 55%, Stacks 3.3%, and the features run from 0.5 to 1.6 times their share, by chance of the draw.

**7. Pins.** Each entry has `source_closure`, the SHA-256 of the sorted contents of everything it reads outside the TeX tree, so independent of path and machine, and `tree_closure`, of the tree's files it reads, by path and content. `tree` in the file holds, once, the oracle's banner, kpathsea's version, each `texlive-*` package and version, and the hashes of `pdftex`, `pdftex.map`, `pdflatex.fmt`, `texmf.cnf` and `ls-R`. A result on another closure is not on this set: say so, and count it. Hashes are cut to 20 hex digits.

**Training.** A row has `pgo: trained` if `tools/pgo.py` trains on that very document, under any id, and `pgo: family` if on another of its family. A report marks them, and gives each pooled figure with and without them. Today `tools/pgo.py` names 8 public documents in this set: `arxiv/math.AG/1912.07119v3`, `arxiv/math.AG/2007.09646v1`, `books/hott/hott-a4`, `manuals/tikz-cd/tikz-cd-doc`, and `ladder/00-plain-hello`, `03-plain-texweb`, `04-latex-hello`, `15-tikzcd-many`; the rest of the ladder is out of the set. Two more are the same document under another id: `docs/latex2e/base/sample2e.tex` is `ladder/05-latex-sample2e`, by its closure, and `knuth/woven/tex` is `ladder/03-plain-texweb` but for a line, in `SAME_TEXT`. So 10 are marked: 3 in `quick`, 8 in `standard`, all 10 in `full`, none in the hold-out. 29 more share a family with one: 14 of `books/hott`, 14 of `knuth/woven`, 1 of `manuals/tikz-cd`, of which 11 are in `standard`. The private part has 5 trained rows, which are its 5 landmarks, and 29 of a trained family. A change to the training list changes what those marks say, and `test_training_overlap_is_marked` then asks for a new set.

**The hold-out and training,** checked by content and not by id, in the oracle's own record of what each document read: of the 1,044 rows of the hold-out and reserve, public and private, none shares a file outside the tree with any of the 29 training documents, and none has a training document's closure, which `test_holdout_is_no_other_id_of_a_trained_document` holds. Word shingles of the public main files found no near-copy of a training document among them. Clean, then. What the hold-out does share is with `standard`: 9 of its 121 rows are of `notes/internal-methods`, whose slides have near-copies in `standard` and `full`, and 7 are of `docs/latex3/l3backend`, with the preamble text of `docs/latex3/l3kernel`, in `standard`. So it is held out from speed work on those documents, and it is not independent of them.

**Near-copies, counted,** on the 1,252 public real documents with a main file: 244 pairs with a Jaccard similarity of 6-word shingles at least 0.5, of which 230 are within a family, where `FAMILY_MASS` holds them, 11 across the split above, 1 the tex.web above, and 2 others. By preamble text: 15 groups, of which the largest are 232 Stacks chapters and 60 of `docs/latex3/l3kernel` and `l3backend`. In `standard`, 217 of 222 real rows have a set of the tree's files of their own, and the one group across families is the tex.web. Among the arXiv papers, one family each, no two share a package set, and only 80 documents of 1,253 share an own `.sty`, `.cls`, `.bst` or `.def` across families. So near-copies inflate the count of documents, which the family mass takes out, and they do not inflate the pooled figure. Same authors' files without a shared file cannot be seen.

**Thin strata,** in `standard`, by documents and families: `kind:beamer` 8 and 2, and in `full` 23 of its 26 are one author's slides; `kind:book` 9 and 4; `kind:small` 10 and 6; `size:one-page` 9 and 5; `size:very-long` 5 and 5, all landmarks; `plain-tex` 10 and 3; `pgfplots` 6 and 2; `kind:cs` 12 and 12; `kind:physics-other` 15 and 15. A report gives its rows and no interval for any of these under 10 documents or 8 families, so `cs` and `physics-other` have one, and the others do not. No stratum is empty but the letter, which the corpus has none of.

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
| | `standard` | 2.16 | 3.11 | 5.62 | 12.9 | 34.8 |
| | hold-out | 1.92 | 2.83 | 5.05 | 9.10 | 21.5 |
| Pages | `full` | 5 | 11 | 21 | 40 | 67 |
| | `standard` | 6 | 11 | 21 | 41 | 67 |
| | hold-out | 4 | 10 | 22 | 40 | 71 |
| Preamble share | `full` | 0.15 | 0.36 | 0.60 | 0.75 | 0.85 |
| | `standard` | 0.13 | 0.35 | 0.60 | 0.75 | 0.83 |
| | hold-out | 0.23 | 0.39 | 0.57 | 0.71 | 0.81 |

**One check against an old engine.** Once the set was committed (880355c), and only then, the last-pass ranks of the PC's old whole-corpus record were read once, by a throwaway script, to see whether the documents an old engine gained least on are in their true share. No gain is quoted here, and nothing changed after it. By the weights of that day, `standard` held 10.0% of the population's bottom tenth, 26.6% of its bottom quarter and 47.5% of its bottom half, and the hold-out 18.8%, 36.1% and 55.2%, so on that old engine it was the harder of the two. The ranks covered 1,022 of the 1,026 real documents in `full`. *Checked*, below, holds `standard` against documents it does not hold, with the weights of today.

Diagrams, public only: of 3,189 taken from documents, 1,453 compile alone under `standalone` with `amsmath`, `amssymb` and real tikz-cd, exit 0, and the others use their document's macros. Of those, 1,231 come from documents people wrote: `tests/cd` and the ladder were written for the native path. Strata are source, arrows (1, 2 to 3, 4 to 7, 8 or more) and whether it uses bends, dashes, hooks and such, at most 4 from one document while there are others. `standard` has 200, `quick` 20, the hold-out 92, from the families kept apart, and `full` every usable one not kept apart. What `commute` declines is not left out: it falls back and counts as such. One diagram alone costs pdflatex 3.0 G instructions at the median, 3.3 G at p90 and 7.7 G at most, nearly all of it loading PGF.

## Scenarios

A row of a report is a document under a scenario. *counted* is one run of each side under user-mode counters, at `-j 14` on the PC, since instructions do not care: instructions, page faults, peak memory, PDF size, and the identical-output check. *timed* is serial on one pinned core with its sibling idle, in rounds with a control: cycles, CPU and wall.

| Scenario | What it answers for a user | Sides | quick | standard | hold-out | full |
|---|---|---|---|---|---|---|
| `first` | I have just cloned this. How long until I have the right PDF? From an empty directory to the fixed point, bibliography and index tools included, as latexmk would. `tests/until_stable/compare.py` holds the two to the same files | latexmk over each pdfTeX; `rstex --until-stable`, running what it names | counted | counted to 30 s | counted to 30 s | counted |
| `last-cold` | What is the engine worth by itself? One pass, side files in place, no checkpoint (`RSTEX_NO_SNAPSHOT=1`, `RSTEX_NO_PAGES=1`), as `tools/prof/bench.py`. rstex's cache of the tree and the font map is filled, as it is from a user's second run on: so "cold" means no checkpoint, and the top of a report says what an empty cache costs | each pdfTeX, rstex; timed with rstex again as control | counted; timed to 10 s | counted; timed to 10 s | counted; timed to 10 s | counted; timed but tests |
| `warm` | What can a restore do at best? The source the checkpoint was made from, the preamble restored, proved by `RSTEX_CHECKPOINT_TRACE`, once a run has left its seal, with `RSTEX_CHECKPOINT_POLICY=always`, where a short preamble waits for a second sight under the program's own policy. `tools/prof/warm.py`. Held against the oracle, and not a save | rstex, with a control when timed | counted; timed to 10 s | counted; timed to 10 s | counted; timed to 10 s | counted; timed but tests |
| `warm-word` | My next compile, with another word in the body, one pass: the same in a mirror, then the word, from the oracle's last-pass side files. Where the preamble looked at the file that holds the body, the checkpoint does not hold, and the row is what that costs | each pdfTeX and rstex on one source, at one path, against Arch's | counted | counted to 30 s | counted to 30 s | counted to 20 s |
| `hits` | The same as `warm` with the old PDF in place: the output cache, on an untouched source | rstex, with a control when timed | counted | counted to 30 s; timed to 2 s | counted to 30 s | counted |
| `resume` | The floor of a save near the end: from the last page record, where one is taken, on an untouched source. A row that takes none is counted and shown. | rstex | counted | counted to 30 s | counted to 30 s | counted |
| `edit` | The save in my editor, generated by `edits.py`: a word near the start, middle and end, a sentence that adds lines, a new citation, a changed preamble, a save that changes nothing, in a mirror under `target/`, with Lemma's flags `-synctex=1 -interaction=nonstopmode -file-line-error -recorder -output-directory=<dir>` and `max_print_line=10000 error_line=254 half_error_line=238` | rstex; latexmk over Arch's pdflatex | counted, `edit` rows | counted, `edit` rows | counted, `edit` rows | counted to 20 s |
| `fmt` | A TeX Live update: `pdflatex.fmt` built | each pdfTeX, rstex; timed with rstex again as control | timed, 1 | timed, 3 | timed, 1 | timed, 3 |
| `pixels` | Save to pixels: the file written, the build, Poppler opening it and rendering the middle page. `tools/prof/loop.py` | rstex behind latexmk, rstex direct, its control | | timed, `pixels` rows, 4 rounds | | |
| `diagram-alone` | A figure for a paper: standalone `commute` against a `standalone` document with real tikz-cd under pdflatex, from the stratified sample of `tests/corpus/cd` | `commute`; each pdfTeX; timed with `commute` again as control | timed, 20 | timed, 200 | timed, 92 | counted, 1,231 less those kept apart |
| `native` | A document with diagrams: native path against real PGF (`RSTEX_NO_COMMUTE=1`) inside it, on rows that read `tikz-cd.sty` | rstex, each way | counted | counted to 30 s; timed to 10 s | counted to 30 s | counted |
| `old-map` | The baselines under the old map | each pdfTeX | | counted to 10 s | | counted |

"30 s" and the like are of the oracle's last pass on the PC. A regression test has only `last-cold` and `warm`, as a suite runs it; latexmk is not how it is run. A plain TeX row has no latexmk: its `first` is the passes the oracle took. `tools/prof` and `tests/until_stable` above are the tools of the performance round, at 8e83f07. `tools/prof/benchset.py` is the driver over what they do: see the next section.

Editing rule, in full, in `edits.py`. A word goes between three plain lower-case words at 5%, 50% and 95% of such places in the document, in reading order over its own files; the sentence and the citation go at the middle blank line; the citation is of the first key by sorting the `.bib` has and the `.aux` does not cite, else the first `\bibitem`; the preamble edit is an unused `\newcommand` before `\begin{document}`. A kind that does not apply to a document is counted, not replaced. A pass that fails on an edit has it taken back and counted. Of the 1,333 real public documents: `noop` applies to 1,332, `preamble` to 1,292, each word edit to 957, `sentence` to 953, `cite` to 562. A row with an `edit` mark in `standard`, 39 of them, and in the hold-out, 15, has all four kinds of body edit. For private rows nothing here reads a source: a driver asks at run time, and reports only the kind and whether it applied.

Each side of an edit runs in its own output directory, and after each save the result is held against the other side by the identical-output check below. `tests/until_stable/session.py` shows how, for two drivers over one engine.

## Running it

```sh
S="python3 -I tools/prof/benchset.py"
$S preflight --tier standard            # whether this machine can: oracles, pins, the tree, the baselines, the programs
$S plan --tier standard                 # what a run would measure, by scenario, and no run
$S run --tier standard --engine plain=BIN --engine pgo=BIN --commute BIN --method counted -j 14 --out target/bench/standard.jsonl
$S run --tier standard --engine plain=BIN --engine pgo=BIN --commute BIN --method timed --cpu 15 --max-load 1.5 --out target/bench/standard.jsonl
python3 -I tests/bench/report.py target/bench/standard.jsonl --md target/bench/standard.md --json target/bench/standard.json --estimate
python3 -I tests/bench/selfcheck.py --engine BIN
```

A run appends to its file, so kill it and run the same line again. It builds what is missing per binary: `pdflatex.fmt` and `pdftex.fmt`, and each pdfTeX's by `baselines.py`'s recipes. It copies a binary under test to `prof-runs/bench-bin/<hash>/rstex`, because its messages print its name. Rows go to `--out`, and documents run in the corpus's own `target/corpus/w`, one path per document, of the length of the oracle's own path. It does not write to the oracle cache unless `--prepare` says so: without it, a document with no valid oracle here is a row saying that. `--ids` and `--scenarios` narrow a run. Without `--no-private`, it reads `set.local.json` too. `--keep-wrong DIR` keeps what each side left where a public document's `first` or `edit` row differs. The hold-out asks for `--frozen`, since it is measured once.

**What a row is.** `id`, `scenario`, `variant`, `side`, `method` and `round`, and those make its `key`. `instructions`, `cycles`, `page_faults`, `wall_ms`, `cpu_ms`, `maxrss_kb`, `rc`, `pdf_bytes`. `check` and how it was got. `binary_sha`, `source_closure`, `tree_closure` and `pin_ok`, `tree`, `machine`, `governor`, `load_before`, `load_after`, `sibling_busy`, `clock_ok`. A file also has a `run` line per run, with the sides' hashes and formats, the tree, the plan and what a run costs that does nothing, a `job` line per document with what it took around its runs, and an `end` line. Nothing in any of them is text from a document.

**What a scenario runs,** where the plan above leaves it open:

| Scenario | Rows | In what order |
|---|---|---|
| `first` | one per side | A mirror under `target/`, with what the oracle saw the document read above its directory linked in above it, one output directory `.build` for all sides in turn, emptied between them. latexmk over each pdfTeX and `rstex --until-stable` with `-interaction=nonstopmode -file-line-error -recorder -no-shell-escape`, no SyncTeX, Lemma's environment, what rstex names run and counted in its row, and a document's cache filled for the tree but not for its search path, which a first compile here does pay for. Checked against latexmk over Arch's, whose PDF is noted against the oracle's: latexmk stops at a TeX error where the oracle's passes go on. A plain TeX row: the passes the oracle took, on every side. An entry with a prelude has it by `-usepretex` and the same first line on rstex's |
| `last-cold` | one per side; timed, a round is each rstex, each pdfTeX, each rstex again as `a2`, turning round | The oracle's last-pass files, the corpus's argv and environment, `RSTEX_NO_SNAPSHOT` and `RSTEX_NO_PAGES` |
| `warm` | `write`, `restore` | An empty checkpoint directory, `RSTEX_CHECKPOINT_POLICY=always`, `RSTEX_NO_PAGES`. The write, 2.1 s, a run that leaves the seal, then the restore with `RSTEX_CHECKPOINT_TRACE`: a row says `restored`. On the two documents tried, the trace cost no more than a run differs from the next. The same source as the oracle's, so that it can be held against it |
| `warm-word` | one per side | As `warm`, in a mirror, with its own cache. Then a word by `edits.py`'s `word-middle`, and from the oracle's last-pass side files each pdfTeX and each rstex one pass, at one output path, held against Arch's on that source. A document with no place for the word has a row saying so |
| `hits` | one | The restore again with its own last PDF in place |
| `resume` | `write`, `last` | Without `RSTEX_NO_PAGES`, its last PDF in place: the chain written, then `RSTEX_PAGE_READONLY`. A row says how many records there are and which it used. One that used none stays, and is counted |
| `edit` | `settle` and one per kind, for each rstex and latexmk over Arch's | A mirror, an output directory per side, renamed to `.build` for its turn, so that paths are the same. Each side to its fixed point; rstex twice more with 2.2 s between, for a checkpoint and a seal, since a user has those from the second or third save on. Then the seven saves, each on what the last left. A kind that does not apply is a row saying so. If latexmk's run ends otherwise than it did at the start, the edit is taken back and counted. A document that ends in an error stops after one pass on both sides without having settled, and latexmk with nothing changed does not run again where rstex does. So there those two runs of rstex leave their caches and not their outputs, or a later save would hold passes of different number against each other, and a save of nothing is `not-comparable` if the two differ. A plain TeX row has no latexmk: Arch's `pdftex` and rstex with one pass to a save, and the oracle's number of them to begin with. It has no `pixels`, which measures rstex behind latexmk |
| `native` | `native`, `pgf` | As `last-cold`, on documents the oracle saw read `tikz-cd.sty`, the second with `RSTEX_NO_COMMUTE` |
| `old-map` | one per pdfTeX | As `last-cold` with `TEXFONTMAPS`. The log names the map, so only the PDF is judged: one that differs has no old-map ratio |
| `fmt` | one per side; timed, a round is each rstex, each pdfTeX, each rstex again as `a2`, turning round | In an empty directory, from the recipes. A format's bytes are nobody's to compare, as in `tests/corpus/run.py`: it has to exist, and the log has to agree with Arch's |
| `diagram-alone` | one per side and diagram; timed, `commute`, each pdfTeX, `commute` again as `a2`, the order by the diagram's name | `census.py`'s `standalone` source from the entry's text. commute's file is plain, with base fonts, so the check is the page's size, its text in order and its pixels at 200 dpi, each channel within 64 of 255, and a verdict of `same-picture`, not `identical`. commute's exit 1 is `declined`, and charged |
| `pixels` | `direct`, `direct-aa`, `latexmk`, `poppler` by round | `tools/prof/loop.py`'s way: the same preparation, then a save, each configuration in an order that goes round, and Poppler by `ctypes` on the PDF. Each state is also built by latexmk over Arch's, once, and held against it |

"30 s" and the like are `minstr` over the set's `pc_rate_instr_per_ms`, so that nothing but `set.json` says which row runs: `select.py`'s estimate took the PC's own CPU. A `testfile` has only `last-cold` and `warm`.

**`minstr` on a row with `@OUT@`.** 1,776 of the 3,099 public rows have an environment that names the run's output directory, as `TEXINPUTS=@OUT@:`, and `census.py` of that day passed the text as it stood, so their `minstr` is of a run with another search path. `tests/corpus/run.py`, `benchset.py` and now `census.py`, `baselines.py`, `bench.py` and `warm.py` all run the row as it should be, and `test_select.py` holds them to that. By the rule above the set stands, and it was not drawn again. `minstr` is used for plan limits, a run's time limit and `report.py --estimate`, and a figure is never made from it. All 1,000 such rows of `quick`, `standard` and `full`, measured with the oracle under user-mode counters on 2026-11-05 (the hold-out and the reserve left out), against `minstr`: median 1.0002, least 0.990, and 1.0044 together. The 15 of them that are not regression tests are `knuth/woven`, median 1.111, at most 1.134. One regression test of `full` alone is 2.54, 8.7 G against 22.0 G. Against the plan's lines of 0.5, 5, 10, 20 and 30 s not one row changes sides, and it is that test against 2 s, which is the limit of `hits`, which a test has not. A run's limit is 300 s, or 10 times a pass if that is more, and on these rows it is 300 s either way. So the limits are off by nothing that decides anything, and `--estimate` by 0.4 to 1.4% on these rows. The figures of `baselines.py gap` under *Baselines* are from before this: 7 of its 70 documents carry `@OUT@`, 3 of `knuth/woven` and 4 regression tests, and 1 of the 28 of `baselines.py latexmk`. Those runs had the text as it stood on both sides, and they have not been run again.

**A reference that failed.** Where the oracle wrote a PDF and Arch's pdfTeX in a mirror wrote none, the two sides would agree on a failure and say `identical`. Those rows are `reference-failed`, on all sides: they have no ratio, and a report counts and names them. Before the mirror held what a document reads above its directory, that was 18 of the 30 private rows of `first`, 13 of 23 of `edit` and 18 of 29 of `warm-word`, and they read as right answers. With it, on the 29 of those documents run again, every reference ended with status 0, and all their rows of `first`, `edit` and `warm-word` that applied are identical.

**The check.** On every row. A row's outputs are hashed, and a verdict is kept per document by hash, so ten runs with the same bytes cost one check. The verdicts are `tests/corpus/run.py compare`'s: `identical`, `differs-content`, `differs-structure-only`, `differs-log-only`, `candidate-crash`, `candidate-timeout`, by its rules on the log, the side files, an unwritten side file, new files in the source directory and the exit status. Its L4 and L5 go first through the normalized bytes of its L6, with streams inflated and the banner, `/Producer`, `/ID` and offsets masked, and `qpdf --check` for what those masks hide; `compare_pdf.py` runs only if those differ, so a document with no difference costs milliseconds and not `compare_pdf.py`'s 0.07 s a page. `selfcheck.py` has the two agreeing on documents. L1, L2, L6 and rasters are the corpus gate's, and are not repeated per row. A SyncTeX file is held against latexmk's too, and reported, but is not in `identical`: DESIGN section 10 names the difference inside a diagram on the native path as an exception, and `report.py` names any document without a diagram whose file differs.

**Peak memory.** `ru_maxrss` from `wait4` holds the peak of the address space an `exec` replaced, whether by `fork` or by `vfork`. So a child of a Python process holding the manifests reports that process's size, at the least. `benchrun.py` is the small process runs come from. A row cannot show less than its floor, in the `run` line: 7.6 MB, a plain run of `/bin/true` here. Spawning from something that small is also why one run can give the instructions, cycles, CPU and wall together. What a run costs that does nothing is in the `run` line too, and is not taken off any time.

**Clocks.** A row is `clock_ok` only if it is timed, the load before and after it is at most `--max-load`, the governor is `performance`, and the core's sibling was idle while it ran, where it ran long enough to tell. Otherwise the instructions stand and report.py quotes no clock. Counted rows have cycles too, and they are not quoted. Pinned to one core, an engine's threads have that core alone, as they did under `tools/prof/bench.py`, which is against rstex.

**Refused.** A run stops if its file holds rows of another binary under a side's name, and leaves a document out, with exit status 3, if the file's rows for it are on another input hash. report.py refuses and counts a row from another binary, another input of the document or another tree, or whose counters were shared with another event, and gives no report at all if two binaries share a side's name across runs. A report says how many documents of its tier it has, says "partial" if that is not all, says which figures are short of the plan, says if a run did not finish, and says if the rows were made with another `set.json`. A run on a machine whose tree differs from `set.json`'s says so in its first line and in the `run` line, and its rows are on another tree than the pins.

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

**From runs.** `report.py --estimate` scales each scenario's instructions per instruction of the oracle's last pass over a tier's documents under its plan, with Python's share per row as measured and the waits for a seal. Its figures below are from all of `standard` counted on the laptop: the performance round's two binaries, both baselines and `commute`, `-j 4`, under other work at a load of 5 to 20, 473 jobs, 7,364 rows, no harness error, 118 min, then 22 min for the 1,000 rows of `warm-word`. They replace those of a dry run on `quick`, which said 3.62 CPU hours and 83 min for `standard`: what that left out was a longer tail, the longest document, and `warm-word`, which had not been thought of. These tables are of the round's binaries, for planning: they say nothing of how fast anything is, and no rate here has met the PC.

| PC, two binaries, 14 cores for counting | Counted, CPU hours | Its longest document | Over 14 cores, or that | Timed, serial | Together |
|---|---:|---:|---:|---:|---:|
| `quick` | 0.58 | 4 min | 4 min | 8 min | 12 min |
| `standard` | 5.88 | 19 min | 25 min | 85 min | 110 min |
| hold-out | 2.57 | 7 min | 11 min | 28 min | 39 min |
| `full` | 39.4 | 66 min | 169 min | 371 min | 9.0 h |

These take 5.69 G instructions a second for pdflatex, the median of the PC's oracle cache, which was made under a high `-j`, so that is on the low side, and rstex at 4.72 G by the ratio of IPC. At the 7.0 G and 6.1 G a second of CPU that a quieter run saw on the laptop, take off a fifth. The run itself, under much load, saw 3.8 G and 3.2 G, which is what a machine doing other work gives: `standard` is then 2.6 h. A document's scenarios run one after another in one worker, so a tier takes at least its longest document, whatever the cores: on `standard` the PGF manual, 6.3 T instructions. By scenario, in seconds on the first rate:

| Scenario | `standard`, counted | timed | `full`, counted | timed |
|---|---:|---:|---:|---:|
| `first` | 4,716 | | 24,425 | |
| `last-cold` | 3,978 | 1,653 | 15,046 | 10,615 |
| `warm` | 3,691 | 1,920 | 19,862 | 11,555 |
| `warm-word` | 3,044 | | 12,539 | |
| `hits` | 610 | 212 | 2,987 | |
| `resume` | 1,044 | | 5,045 | |
| `edit` | 2,922 | | 51,362 | |
| `native` | 103 | 146 | 363 | |
| `old-map` | 1,047 | | 6,749 | |
| `fmt` | | 110 | | 110 |
| `diagram-alone` | | 233 | 3,565 | |
| `pixels` | | 805 | | |

The timed column of `standard` is arithmetic on counted rows: the timed path ran on 4 documents here, under load, for whether it runs, and `pixels` on 1. A document that never ends costs its limit, now 10 times the oracle's pass on the PC and at least 300 s. At 40 times, one document that hung took 32 min of this run.

**So `standard` is nearly two hours as it stands, and an hour takes both cuts.** Neither looks at a result, and neither touches which documents are counted. A run so thinned says so in its `run` line and in the report. **The owner should start it knowing that, and `quick` first:** a run can be killed and started again with the same line.

| Cut | `standard` | Why, and what it costs |
|---|---:|---|
| `--timed-engines pgo` | 82 min | The clock is asked of the binary that ships, as `docs/BUILD.md` has it. The plain build keeps every row of instructions, so what PGO gains in instructions is still there; what PGO gains in cycles and wall on these documents is not. The first cut to make |
| `--thin timed-5s` | 92 min | No clock on a document whose oracle takes over 5 s a pass: 34 of the 226 real public rows, and not 14. A rule on the oracle's seconds. Those get cycles and wall from `full`. `full` is not touched |
| Both | 71 min | |

Preparing the PC, which has none of it today: the official TeX Live under `target/texlive-official`, `target/pc/baseline/oldmap`, and biber, which it lacks, so a biblatex row under `first` or `edit` has no bibliography until it does; `preflight` fails on any of these. Read over ssh: its `pdftex`, `texmf.cnf`, `ls-R` and the system's `pdftex.map` have the set's hashes, and it has a `pdftex.map` of 4,424 lines under `~/.texlive`, which no run reads, as all have run.py's hermetic `TEXMFVAR`. What is not in the repository: `refs/corpus`, the oracle cache in `target/corpus`, the two `.local.json` files of this directory and those of `tests/corpus/manifests`, the binaries and `commute`, and `target/bs-design/diagrams.jsonl` for `full`. Formats are built by the first run. `benchset.py preflight` says what is still missing, before an hour goes. With `-j 14`, `--cpu 15` is core 15 there, with 31 as its sibling.

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

So the official build runs about 8% fewer instructions than Arch's on these documents, 10 to 11% on macro-heavy ones, and 2% under plain TeX. It is 69 M less on the smallest LaTeX document and 41 G less on the largest in the sample. Over all 222 real public rows of `standard`, in `last-cold`: 0.918 with documents even, 0.918 by weight, from 0.883 to 1.009, with 0.897 on `heavy-tikz` and `tikz-cd`, 0.980 under plain TeX, and 270 of the 272 rows with the private part identical to the oracle's and 2 differing in structure. Peak RSS of the official build is 0.70 of Arch's, median 0.69. Page faults were 0.93, but they move with the machine's state on a loaded laptop: read them from a quiet run. Exit status: 15 of the 70 end in 1 on all four sides, as they do under the installed format, and one more only under the old map, below. PDF sizes agree within 256 bytes between the two pdfTeX on all 70. Their contents were not compared here.

**What it means.** A ratio of rstex to Arch's pdfTeX in instructions, divided by about 0.92, is the ratio to the official one, and by about 0.90 on long macro-heavy documents. Whichever a report quotes, it says which. What this does **not** say is the gap in time. Instructions cannot see the difference an 8-byte memory word makes to the cache, and it may be larger in cycles than here. So no time ratio against the official build is known until `baselines.py gap --cycles` has run on the quiet PC, in a round with a control, which it has not. Until then a time ratio against Arch's has to say that it may flatter rstex, by an amount not yet measured.

**The map.** The current map costs either pdfTeX about 500 M at each start that ships a page: median 499 M for Arch's, 509 M for the official, 465 M at the least. That is 60% of the plain hello, 42% of the LaTeX hello, 0.1% of a book, so a claim about a small document depends on it more than on anything rstex does. Old map over current, Arch's: geometric mean 0.873, minimum 0.393, median 0.926. Which one a user has depends on what they installed, so both are reported. The old map is not a whole map: under it `arxiv/stat.ML/2002.06694v2` gives no PDF and `arxiv/math.AG/2307.01711v2` another size, 6.7 G cheaper. On `standard`'s 240 rows under 10 s, 226 render the same, 5 otherwise and 9 give no PDF. A row like that has no old-map ratio in a report, so that figure leaves out documents whose fonts the old map lacks. Those two are in the means above, and move them by little. Traced by kpathsea's own debug output on the smallest document: each pdfTeX opens one `texmf.cnf`, the three `ls-R`, its own format, and the one map, or the old one under `TEXFONTMAPS`, and the two have the same version and differ in the banner. The rest of the gap between DESIGN's old figures and now, about 143 M, `target/pc/baseline/REPORT.md` puts down to the larger `ls-R`, without having proved it. It stays in both baselines here.

**latexmk over each,** on the 28 real public documents of `quick`, counted with all its children from an empty directory, then again with nothing to do: 19 of the 28 end with exit 0 under Arch's. From nothing, official over Arch's has a geometric mean of 0.931, from 0.902 to 0.960. The rerun, which is mostly latexmk's own Perl, costs 500 M at the median, 421 M at the least. The largest, 39.6 G, is a document that latexmk ran again. So a save under latexmk starts 0.4 to 0.5 G behind, whichever engine it runs.

**lualatex and xelatex,** side table only, on the 26 of those that are LaTeX, the second pass from nothing against pdflatex's second pass, instructions. Their output differs, in fonts and in line breaks, so this is not the same work, and no ratio to them may be quoted as a speed-up of rstex. Where exit 0 and a PDF came out on both, lualatex on 13 of 26: geometric mean 1.86 of pdflatex, 1.19 to 3.45. xelatex on 15 of 26: 1.17, 1.03 to 1.63. The first pass of lualatex with an empty cache was about 90 G in a trial, which is why the second pass is what counts. These runs are a sample of what pdfTeX compiles, so this says nothing of documents written for those engines.

## Statistics

Every figure names its tier, scenario and baseline, is on the set's pinned closures, and comes with these:

- **Pooled, first:** weighted by `weight`, which estimates the corpus at family mass, with the interval from resampling families within cells, as documents of a family are not independent. With and without the rows `pgo` marks. Regression tests, letter stand-ins and the ladder are not in it. This is the one to quote. `quick` has none, as its cells are not in proportion.
- **Per stratum:** the geometric mean of the ratio over documents, per kind, per size, per feature, with a bootstrap interval over families, 10,000 resamples within the stratum, seed fixed, widened for how few there are. A geometric mean because ratios multiply, and an interval because 20 documents are not a population. A stratum under 10 documents or 8 families gives its rows and no interval. What "95%" comes to is under *Checked*.
- **A total** beside each mean: all instructions of a side over all of the baseline's, unweighted and by weight, as a geometric mean counts a page like a book, and a user waits for instructions. The largest document's share is named, as it can be most of a total.
- **The whole distribution:** minimum, quartiles, p90, maximum, and the worst ten documents by id. Never a best case alone, and never an unweighted mean over the corpus: 6,400 of its entries load one package and typeset a line. The mean over a tier's rows as they are, landmarks and floors with the rest, stands in a column of its own, and estimates nothing.
- **Instructions are the exact measure.** User-mode, from outside the process, `tools/prof/proflib.py`. They do not see the kernel, a page fault's cost or the cache, so they are not the time.
- **Paired cycles with an A/A control:** in each round A, B and A again in an order that turns round, `tools/prof/compare_cycles.py`. The median over rounds of B over the A beside it, next to A over A. If the control's mean is more than 1% from 1 that day, or it is more than 5% from 1 on a tenth of the documents, cycles are not quoted. `fmt` and `diagram-alone` have the same control and the same rule, and rows from before they had one give them no clock figure at all. Clock figures go up to the plan's limit of a document's length, so they do not cover what the longest documents do: a report says over how many.
- **CPU and wall** only on a quiet machine, under a load guard: the PC, governor `performance`, CV under 1% when quiet, one pinned core and its sibling idle, `--max-load`, the load recorded per row. The laptop gives instructions only while anything else runs.
- **Also per row:** peak memory, page faults, PDF size.
- **Only right answers count.** Every row has the identical-output check: `tests/harness/compare_pdf.py` on page content, structure and normalized bytes, and the log, against the same side of Arch's pdfTeX, as `tests/corpus/run.py compare` does. Next to every figure stands `N / M identical`.
- **A row that differs or fails counts against rstex.** It stays in M and is shown by id. It has no ratio of its own, as a wrong answer has no speed, so it is charged: what the attempt took plus the baseline, over the baseline, since that is what a user who has to fall back pays. A crash or timeout with nothing to time is charged its time or the limit, plus the baseline. The figures are given charged. An uncharged figure may be given beside it, not instead.

Reports quote against the official pdflatex first, then against Arch's. A figure against Arch's alone is marked as such. A claim about small documents says which map.

## Checked

An attack on all this, by someone who did not build it, on the performance round's two binaries, on the laptop under load, in instructions. No figure of it is a final one, and none is quoted as a ratio of rstex. The tools are in `tests/bench/predict.py` and in `selfcheck.py`; the rest were scratch scripts over the same rows.

**Does `standard` predict what it does not hold?** With no ratio read, the weights were changed, as under *True share*. Then ratios were read: a fresh sample by the seed `rstex-bench-predict-1`, 300 of the 804 real documents of `full` outside `standard`, and then the other 504, so that all of `full` has `last-cold` and `warm` against Arch's: 1,022 documents with the ladder out. Hold-out and reserve were not touched.

| `standard` by weight, over | `last-cold` | `warm` |
|---|---:|---:|
| the sample of 300, at mass, both inside each other's interval | 1.041 | 1.076 |
| all 1,022 of `full`, at mass, which is no estimate, and inside `standard`'s interval | 1.038 | 1.063 |
| the same with the weights before the change | 1.003 | 1.002 |

So this draw reads 4 to 6% high, against rstex, and inside its interval. By stratum, against the 804, 16 had an interval and at least 10 documents outside `standard`: at family mass the 804's mean lies outside `standard`'s interval on 1 and on 0, and the difference is never over 2 of its errors. Nothing was changed for this: a reason that held without the number was not there, and one draw at 1.8 of its own standard deviations is a draw. The weights before the change came closer on this draw, and they are worse as a rule, as this shows: the rule run again under 300 other seeds on these ratios.

| 300 draws of the rule, against all of `full` | `last-cold`: mean error | one draw's sd | `warm`: mean error | one draw's sd |
|---|---:|---:|---:|---:|
| The weights now | +0.6% | 2.2% | -0.3% | 3.5% |
| The weights before, which flattered rstex | -2.8% | 1.5% | -5.8% | 2.8% |
| The tier's rows with documents even | -0.2% | 1.4% | +0.1% | 2.5% |

The pooled interval held the truth on 96% and 99% of 120 draws. Documents even did well here and has no reason to on another engine: it is not what the rule says it holds, so it is not what is quoted. A report's check of `standard` against `full` on one run says the same of a final engine, and it is there to be read.

**Is anything still reading a candidate's column?** No. `census.py` reads whitelisted columns of a record, `select.py` refuses the rest, and nothing else in these tools opens a record, by a search of them and by tests. `audit.py` reads none. The exclusions in the manifests are for downloads that failed, PDF-only papers and a copyright guard, and the history of the manifests of real documents has one change, which adds an entry, so the corpus was not cut by what rstex did. `predict.py` and scratch scripts read ratios, for this section, and they feed nothing but `weight`, as stated.

**Scenarios that flatter.** Found and fixed: `warm` claimed a word in the body and had none. On the sample, 76 of 184 documents restored untouched lost the checkpoint with a word in the body, 75 of them Stacks chapters, whose preamble looks at the file the body is in, at 1 to 6.5 times the instructions. By mass that moves the figure 3.4%; with documents even, 18%; the others do not move. `warm-word` is that word. Also: rows in a mirror where both sides agreed on a failure, 18 of 30 private `first` rows and more, which is under *Running it*, and which public rows did not have; a run whose first pass built an index the next did not; a tool killed by its limit that counted as having succeeded.

Tried, and clean. Argv and environment of the two sides come from one function, and differ in the program, how each finds its format and rstex's cache. `first` and `edit` have one set of flags on all sides, SyncTeX in `edit` on all. Each side writes its log, compared by the check. Formats: one recipe for all, built once outside the clock by a run, with `fmt` the scenario that times it. rstex's cache per binary holds the tree's index and the font map and no checkpoint, by listing it, and each document and run kind has its own from it. A repeated `warm` run has the same count within 0.05% on 96 pairs of the dry run, and a `last-cold` run too but for 3 documents, which was that index, so nothing is learnt from a run before. The file cache: a run hashes every input of a document before its first measured run, and each format and binary before the first document, so all are in the cache for every side. Order, in a round: rstex on both sides of its baselines, in an order that turns round. No page record but in `resume`: none of 2,132 rows of the others has a trace of one, against 354 of its 1,000. User-mode counters do not see the kernel: its share of CPU is 2 to 4% on each side over the rows of `standard` and the sample, and rstex takes less than half the page faults per instruction, so they hide no large part. rstex's threads are in the counts. None of 11,000 rows had its counters shared. Pinning and turbo are per core and the same for all, and a control is there for what is left, which nothing here could measure under load.

**Baselines.** As above: clean, on one tree with their own formats, and the gap the same on 222 documents as on 70. What a reader could misread is now marked: `edit` is against Arch's alone in every report, and a figure says so.

**Statistics.** Over what a report did: intervals over documents held the mean 86 to 89% of the time at 10 documents, and over documents of 3 families 74 to 77%. Now over families, widened for how few: 90 to 93% at 10, 90 to 94% at 20, and with 8 families or more at or over 95%; below that, rows and no interval. A/A was present in clock figures and blind to noise on single documents, and it now is not. A geometric mean over a tier's rows led the table and estimated nothing; a total, which a geometric mean can sit far under, was absent. On `standard` the total by weight is nearly 9 times the pooled mean for `resume`, and nearly twice for `warm`. The worst ten and the distribution were always there and named documents.

**Reproducibility.** Under *What a clean checkout can and cannot do*. A map under the PC's `~/.texlive` would have made every run there say it was on another tree, and preflight fail. Fixed; another `HOME` itself is not tried.

**Run time.** Under *What a tier costs*: 83 min for `standard` became 110, and the longest document can set a floor.

**A bug of the engine found on the way, for whoever has it next.** `docs/latex3/l3kernel/l3box.dtx`: a pass, then `makeindex -q l3box.idx` with no style, which is what latexmk and `--until-stable` do, then a second pass. Under latexmk, Arch's pdfTeX is done in 22 s and the official one in 20 s, each ending in an error, and both of the round's rstex binaries were killed at their limit of 965 s. In trials of 60 s it was still running with nearly all its log written, with or without `--until-stable`, a checkpoint or page records, where Arch's second pass took 44 s under that load. Those rows are charged. The reason has not been looked for.

## What a number from this set can and cannot be used to claim

A number can say: on this version of the set, this binary, this machine and this tree, against this baseline, the pooled ratio of instructions under this scenario was R, within this interval, with N of M rows identical to pdfTeX's, and these are the worst. That estimates the corpus at family mass, under that scenario. Measured once on the frozen engine, the hold-out says whether it carries over.

It cannot be used to say:

- **"N times faster for TeX users."** The corpus is not a sample of what people compile. See below.
- **"Faster than pdfTeX", from Arch's alone.** The official build runs 8 to 11% fewer instructions, and its gap in time has not been measured. A figure against Arch's is marked as such.
- **Anything about time, from instructions.** Instructions do not see the kernel or the cache. On the rows under *Checked* the kernel's share of CPU is much the same on both sides and rstex takes fewer page faults, so they hide no large part: that is on a loaded laptop, and cycles and wall say the rest.
- **"My second compile", from `warm`, `hits` or `resume`.** Those compile the source the checkpoint or the PDF was made from, `warm` with the policy forced. `warm-word` is one pass with a word in the body, and `edit` is a sitting with the program's own policy, against latexmk.
- **"From nothing", from `last-cold` or `first`.** Both have rstex's cache of the tree and the font map filled, and a first run on a machine does not. The top of a report says what that costs, about 790 M instructions once on the round's binary, against about 1,190 M for the smallest document under pdflatex. So a claim about small documents has to say so, and which font map it is on.
- **The engine's own gain, from `first` or `edit`.** They hold rstex against latexmk, whose Perl is 0.4 to 0.5 G of each run, and they count passes it does not have to make.
- **A mean of a tier's rows as they are.** Landmarks and floors are in it. Nor a stratum's mean as the corpus's, or one with no interval as more than its rows.
- **Time saved, from a geometric mean.** It counts a page like a book. The total says what a user waits for.
- **A PGO figure as a figure for what a user can build.** `tools/pgo.py --train auto` trains on the owner's documents, and a report gives its pooled figure with and without the rows it marks. In instructions those marks moved little on the round's binary; in cycles nobody has looked.
- **"Identical", of a diagram or a format.** `diagram-alone` says the same picture by a rule with thresholds, and `fmt` the same log. A SyncTeX file is reported and is not in `identical`.
- **Anything from a partial or thinned run,** beyond what it says of itself: a run goes from the cheapest document to the dearest, and a report says how many of its tier it has.
- **Anything final, from the numbers measured so far.** All of them are of the performance round's binaries.

## What the set does not represent

- **TeX users.** It is the corpus, not a sample of what people compile. The arXiv sample has quotas chosen for this project: 80 of 400 in `math.AG`, 300 in mathematics. So weights give the corpus's shares, and no one can say what a typical user's are.
- **Documents pdfTeX cannot compile,** whatever the reason, such as being written for LuaLaTeX or XeLaTeX, needing shell escape, or a package or font this tree lacks: `oracle-fails`, out by the rule. That is 55 of 1,401 real public entries and 10 of 212 private ones. The arXiv sample lost 35 of 390 this way.
- **Letters.** The corpus has no real letter, so the kind holds real documents of at most 2 pages and one-line documents of letter classes from `coverage/cls/`, marked `synthetic`.
- **Slides,** which here are one author: 23 of the 26 `beamer` rows of `full`, and all 6 of the hold-out's, are `notes/internal-methods`.
- **biber** in the census: `prepare` does not run it, so biblatex documents without a `.bbl` have an unresolved bibliography there. latexmk runs it where installed in `first` and `edit`.
- **Images at scale, non-Latin scripts, generated sources** are thin or absent: see `many-images`, whose largest document has about 200.
- **Disks and shared file systems.** A warm page cache unless a run says `--cold`. Viewer time is only in `pixels`.
- **Other machines.** Two Zen 3 CPUs.
- **How a project is really built.** A real build of the Stacks project is its Makefile, which gives each chapter the others' `.aux`, and not one chapter from nothing.
- **The private part cannot be reproduced by anyone else.** It is reported apart, by id and number only.
