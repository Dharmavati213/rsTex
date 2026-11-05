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

A save with nothing changed gets `rstex: until-stable: passes=0 result=stable` and status 0, with no pass, nothing on stdout and nothing written, from the second such save on: the first one keeps what it takes to answer. It takes a change to anything the pass read or looked for, a `.bib` on `BIBINPUTS` among them, or to anything it left, or another command line or search path, to run a pass. What it does not see is the clock, as latexmk does not, so a document with `\today` gets no new date until it changes. `RSTEX_NO_SHORTCUT` runs the pass.

**Killing a build.** A build under `--until-stable` may be killed at any moment, and the next one with the same job and output directory gives what it would have given with no kill:

- The PDF under its own name is a whole one at every moment, the last run's until a new one replaces it by rename. It is not written in place unless it is a symbolic link or has a second name. A document that asks about its own PDF after the first page, or that starts a program by then, finds what pdfTeX leaves there, an empty file, and a kill then finds it so as well.
- The side files have their names throughout, since TeX can read what it is writing, so after a kill they are cut short until the next build under the flag, which first puts back what the last whole pass left. It keeps what it needs for that in `<job>.rstex-state` beside the outputs: leave that file there. A plain run, pdflatex or latexmk does not put them back, and a caller that wants them must run the flag first. The `.log` and `.fls` are not put back: the next pass writes them again, and only the last pass's are the document's.
- A kill can leave `<job>.pdf.rstex-tmp` and `<job>.synctex(busy)`, which the next build replaces, and in the first milliseconds of a cold run a `.fls` named by the process id, as pdfTeX does.

## commute

`commute` draws a `tikzcd` diagram as a tightly cropped PDF and SVG in a few milliseconds, with the labels set by real TeX:

```
commute square.tex      # square.pdf and square.svg, from \begin{tikzcd} ... \end{tikzcd}
```

The first run on a machine builds its format, once, from the TeX Live tree. A file of many diagrams gives one PDF and one SVG for each, and a preamble can be added with `--preamble` or `--usepackage`.
What it draws is compared with `standalone` and real tikz-cd by `tests/cd/compare_cli.py`. [`docs/COMMUTE.md`](docs/COMMUTE.md) says how to use it.

Licensed under GPL-3.0-or-later.
