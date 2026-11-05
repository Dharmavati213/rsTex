# rstex

A TeX-to-PDF compiler written entirely in Rust, and `commute`, a tikz-cd-compatible commutative diagram tool.
The main goal is performance, while running real LaTeX documents unchanged on Linux, macOS and Windows.

**Status: under construction.** The design is in [`docs/DESIGN.md`](docs/DESIGN.md), and the measurements
behind it are in [`docs/research/`](docs/research/).

rstex is a port of pdfTeX. It reads TeX Live's or MiKTeX's data files (packages, fonts, metrics) and does not use their programs
to compile. LuaTeX features (Lua, OpenType, `fontspec`) are out of scope for now.

## Compiling until the references settle

    rstex -progname=pdflatex --until-stable [--max-passes=N] [pdfTeX's options] file.tex

Without `--until-stable` rstex is one run of pdfTeX and nothing else. With it, rstex runs passes until the side files a pass reads stop changing, by their content, as latexmk does, and at most 5
passes as latexmk does unless `--max-passes` says otherwise. The files written are pdfTeX's, the log is the last pass's, and each pass's terminal output goes to stdout. There is no parent process: a signal to the process id reaches
the pass that is running. A first pass with no `.aux` finds the file that latexmk leaves there, so a simple document takes one pass. With `-output-directory`, that directory goes on `TEXINPUTS` and `BIBINPUTS` and into
`TEXMFOUTPUT` for the run, as latexmk has it, so that a document can `\input` by name what it wrote there. As with pdfTeX, the directories of `\include`d files have to be there beforehand.

rstex does not run bibtex, biber or makeindex. When the result of one is stale, it stops and says so on stderr, and the caller runs that program, or latexmk, and comes back:

    rstex: until-stable: tool=bibtex reason=citations-changed file=build/main.aux
    rstex: until-stable: passes=1 result=tool

`tool` is `bibtex`, `biber` or `makeindex`, `file` is what to give it, up to the end of the line, and there is a line for each program. `reason` is one of:

| Reason | Means |
|---|---|
| `output-missing` | The document asks for the `.bbl` or `.ind`, and it is not there |
| `no-record` | It is there, and nothing says what it was made from. Said once |
| `citations-changed` | The lines bibtex reads of the `.aux` files have changed since the `.bbl` was written: `\citation`, `\bibdata`, `\bibstyle` |
| `bcf-changed` | The `.bcf` has changed since the `.bbl` was written |
| `idx-changed` | The `.idx` has changed since the `.ind` was written |
| `source-changed` | A `.bib` or `.bst` file has changed since |

A new label or another page number does not count as a change for bibtex, nor a `.idx` that no `\printindex` reads for makeindex, and a `.bbl` with no `.bib` file is what came with the document. What a result was made from is kept in
`<job>.rstex-tools` beside the outputs, only for a document that has such a program. Other programs, such as those of `glossaries`, are not known: use latexmk with its configuration for those.

| Status | Means | Last line ends in |
|---|---|---|
| 0 | Settled, with no error | `result=stable` |
| 1 | A pass had TeX errors, or could not run. No more passes follow, and the log says what | `result=error` |
| 10 | A program must run: see the lines before | `result=tool` |
| 11 | Still changing after the last pass allowed | `result=limit file=<a file that changed>` |

Between passes it says `rstex: until-stable: pass=1 changed=3 file=main.aux`: how many side files changed, and the first of them.

## After an update of TeX Live

rstex gets much of its speed on diagrams and on documents with microtype from knowing the macros it replaces, by a hash of each. It knows them for the versions it has been verified on, and on any other tree it does what pdfTeX does, by running them: the output stays right, and it gets slower. Nothing says so by itself, and a run's log and terminal do not change by a byte. To ask:

    rstex --check-tree

```
rstex: check-tree: tree /usr/share/texmf-dist
rstex: check-tree: LaTeX 2025-11-01/pl0, PGF 3.1.11a/2025-08-29, tikz-cd 2021/05/04/v1.0, microtype 2026/03/01/v3.2d
path=tikz-cd state=armed profile=pgf3.1.11a-latex2025-11-01-pl0
path=pickup@font state=armed
path=checkpoint state=armed
```

And after an update that it does not know yet:

```
path=tikz-cd state=off nearest=pgf3.1.12-latex2026-11-01-pl0 differ=2/1978 macro=pgfscope package=PGF version=3.1.13/2027-02-01 why=a macro or key of tikz-cd or PGF is not as installed: pgfscope
  \pgfscope, \pgfversion are not as in the nearest set this rstex knows, of PGF 3.1.12 2026-08-01 and LaTeX 2026-11-01 patch level 0.
  So every tikzcd here is drawn by real PGF.  What comes out is right, and each diagram costs some 270 times the instructions.
  A newer rstex may know this tree.  From a checkout of rstex's sources, `python3 -I tests/cd/native/new_profile.py` makes the table entry and says whether it may be used: docs/BUILD.md.
```

| Status | Means |
|---|---|
| 0 | All that is installed is armed |
| 1 | Something is off: see the lines that say `state=off` |
| 2 | No answer, as with no TeX tree |

The lines that begin `path=` are for programs, with `path`, `state`, then what there is to say, separated by blanks, and `why=` the last one, to the end of the line. `state` is `armed`, `off`, or `absent` for a package that is not installed. An editor can run this once a day, or when `rstex --version` or the tree's `ls-R` has changed, and tell its user what the run itself will not; it takes about as long as a short document. `docs/BUILD.md` says what comes next, and how a table is made.

## commute

`commute` draws a `tikzcd` diagram as a tightly cropped PDF and SVG in a few milliseconds, with the labels set by real TeX:

```
commute square.tex      # square.pdf and square.svg, from \begin{tikzcd} ... \end{tikzcd}
```

The first run on a machine builds its format, once, from the TeX Live tree. A file of many diagrams gives one PDF and one SVG for each, and a preamble can be added with `--preamble` or `--usepackage`.
What it draws is compared with `standalone` and real tikz-cd by `tests/cd/compare_cli.py`. [`docs/COMMUTE.md`](docs/COMMUTE.md) says how to use it. On a TeX tree it is not verified on, a diagram under a preamble gets a PDF from real PGF and no SVG, says why, and the status is 3.

Licensed under GPL-3.0-or-later.
