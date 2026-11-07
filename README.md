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
`TEXMFOUTPUT` for the run, as latexmk has it, so that a document can `\input` by name what it wrote there. The directories of `\include`d files have to be there beforehand under `-file-line-error`, as under latexmk, whose log reader does not know that form of the message. Without that option a pass that misses one makes it and goes on, as latexmk does.

A file a pass looked for, did not find and then wrote is a reason for another pass only if the log says it was missing, as `\@input` does with `No file`. latexmk knows of no other look, so `\IfFileExists` alone takes it one run, and rstex one pass.

rstex does not run bibtex, biber or makeindex. When the result of one is stale, it stops and says so on stderr, and the caller runs that program, or latexmk, and comes back:

    rstex: until-stable: tool=bibtex reason=citations-changed file=build/main.aux
    rstex: until-stable: passes=1 result=tool

`tool` is `bibtex`, `biber` or `makeindex`, `file` is what to give it, up to the end of the line, and there is a line for each program. A run can say this before any pass, with `passes=0`, as latexmk runs such a program first: see below. `reason` is one of:

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

| Status | Means | latexmk says | Last line ends in |
|---|---|---|---|
| 0 | Settled, with no error. A document with no pages is that too, as the log says, where latexmk fails for want of a PDF | 0, or 12 for no pages | `result=stable` |
| 1 | A pass had TeX errors, or could not run. The log says what | 12 | `result=error` |
| 10 | A program must run: see the lines before | | `result=tool` |
| 11 | Still changing after the last pass allowed | 12 | `result=limit file=<a file that changed>` |

Between passes it says `rstex: until-stable: pass=1 changed=3 file=main.aux`: how many side files changed, and the first of them.

**After a TeX error,** a caller ends up with what latexmk leaves, file for file:

- That pass is the last, unless a directory it missed has been made, as above, or a file it read and did not write has changed meanwhile, on any build but the first: latexmk takes that for an edit of yours, and keeps trying.
- No program is named for that pass, whatever it wrote. What the result of one would be made from counts as seen, so a new `\index` entry in a save with an error runs no makeindex then, nor when the error goes with the entry unchanged, and the index lacks it until the next change. That is latexmk's behaviour, and it is on purpose: reruns would repair nothing.
- The next run begins with a program whose result is missing, whatever has changed, even with nothing changed: status 10, `passes=0`, nothing touched. Run it and come back, and the pass that follows reads what it made, or fails again with status 1. The line goes out once: if the program leaves nothing, the next run is a pass. A `.idx` is reason enough here, since the pass may have stopped before it came to `\printindex`, and that also goes for a document with no `\printindex`, once.
- The same for a `.bib` or `.bst` file changed since its program ran, with or without an error.
- No `.bib` file means no bibtex or biber, as under latexmk's default, until there is one.
- A `.bcf` that a pass left cut short becomes `<job>.bcf-SAVE-ERROR`, and any `.bbl` `<job>.bbl-SAVE-ERROR`, under latexmk's names.

**A document that never settles** ends where it ends under latexmk. A file it only looked for on a first build is nothing to either, so that takes one pass and status 0. Once the file is there, a build reads it: 5 passes, and status 11 where latexmk says 12 after 5 runs.

A save with nothing changed gets `rstex: until-stable: passes=0 result=stable` and status 0, with no pass, nothing on stdout and nothing written, from the second such save on: the first one keeps what it takes to answer. It takes a change to anything the pass read or looked for, a `.bib` on `BIBINPUTS` among them, or to anything it left, or another command line or search path, to run a pass. What it does not see is the clock, as latexmk does not, so a document with `\today` gets no new date until it changes. `RSTEX_NO_SHORTCUT` runs the pass.

**A format being built.** When no format is found that rstex can read, and TeX Live's are gzip files that it cannot, a first run builds one, which takes some seconds, and says so on stderr before anything else, whether or not `--until-stable` is on. That goes for a format named any way pdfTeX takes one: by the program's name, a `%&` line, `-fmt=pdflatex`, or `&pdflatex` on the command line or at the `**` prompt, as in mylatexformat's `pdflatex -ini -jobname=pre "&pdflatex" mylatexformat.ltx doc.tex`. stdout and the log have no word of it, and `RSTEX_FMT_VERBOSE` adds what the build itself says:

    rstex: building the format pdflatex
    rstex: waiting for another run to build the format pdflatex     (after 10 s of that)
    rstex: the format pdflatex did not build: see <its log>
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

**Killing a build.** A build under `--until-stable` may be killed at any moment, and the next one with the same job and output directory gives what it would have given with no kill:

- The PDF under its own name is a whole one at every moment, the last run's until a new one replaces it by rename. It is not written in place unless it is a symbolic link or has a second name. A document that asks about its own PDF after the first page, or that starts a program by then, finds what pdfTeX leaves there, an empty file, and a kill then finds it so as well.
- The side files have their names throughout, since TeX can read what it is writing, so after a kill they are cut short until the next build under the flag, which first puts back what the last whole pass left. It keeps what it needs for that in `<job>.rstex-state` beside the outputs: leave that file there. A plain run, pdflatex or latexmk does not put them back, and a caller that wants them must run the flag first. The `.log` and `.fls` are not put back: the next pass writes them again, and only the last pass's are the document's.
- A kill can leave `<job>.pdf.rstex-tmp`, `<job>.synctex(busy)` and `<job>.rstex-fls`, which the next build replaces or takes away. Only with a first line that does not name a file, so that rstex cannot tell the job's name from the command line, can it leave a `.fls` named by the process id, as pdfTeX does.

## commute

`commute` draws a `tikzcd` diagram as a tightly cropped PDF and SVG in a few milliseconds, with the labels set by real TeX:

```
commute square.tex      # square.pdf and square.svg, from \begin{tikzcd} ... \end{tikzcd}
```

The first run on a machine builds its format, once, from the TeX Live tree. A file of many diagrams gives one PDF and one SVG for each, and a preamble can be added with `--preamble` or `--usepackage`.
What it draws is compared with `standalone` and real tikz-cd by `tests/cd/compare_cli.py`. [`docs/COMMUTE.md`](docs/COMMUTE.md) says how to use it. On a TeX tree it is not verified on, a diagram under a preamble gets a PDF from real PGF and no SVG, says why, and the status is 3.

Licensed under GPL-3.0-or-later.
