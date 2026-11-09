# commute

`commute` turns a `tikzcd` diagram into a tightly cropped PDF and SVG, in a few milliseconds. The picture is the one `pdflatex` with real tikz-cd draws, to the pixel at 400 dpi, and the labels are real TeX: `\mathbb{Z}`, `\xrightarrow{f}`, `\text{coker}` and the rest work as in a document.

All it needs is a TeX Live tree. It uses no TeX program.

## Use

Put a diagram in a file:

```latex
% square.tex
\begin{tikzcd}
  A \arrow[r, "f"] \arrow[d, "g"'] & B \arrow[d, "h"] \\
  C \arrow[r, "k"']                & \mathbb{D}
\end{tikzcd}
```

and run

```
commute square.tex
```

That writes `square.pdf` and `square.svg` next to it. The first run on a machine takes some seconds, once, to build what later runs load. After that a run like this one takes a few milliseconds.

The page of the PDF is the picture's box and nothing else, as `\documentclass[border=0pt]{standalone}` makes it, so it goes straight into a document:

```latex
\includegraphics{square}
```

The SVG draws everything as paths, with the glyphs from the Type 1 fonts: no `<text>`, no fonts to install, no other files. It can go on a web page as it is.

A file may hold several diagrams, with anything between them. Each becomes `<name>-1.pdf`, `<name>-2.pdf` and so on, and one run does them all, so a file of many costs less per diagram than a run for each.

```
commute chapter3-figures.tex      # chapter3-figures-1.pdf, chapter3-figures-1.svg, chapter3-figures-2.pdf, ...
```

A file may also hold just what goes between `\begin{tikzcd}` and `\end{tikzcd}`, and standard input is `-`:

```
echo 'A \arrow[r, hook] & B' | commute --stem inclusion -
```

## When a diagram needs more

Without further ado cells and labels can use LaTeX with `amsmath`, `amssymb`, `graphicx` and `xcolor`, at 10pt in Computer Modern. For anything else, say so:

```
commute --usepackage mathrsfs --usepackage '[bbgreekl]{mathbbol}' figure.tex
commute --preamble macros.tex figure.tex
```

`macros.tex` is what you would have in a preamble: `\newcommand`, `\DeclareMathOperator`, `\tikzcdset`, `\usetikzlibrary` and so on. The same goes for a file that is a whole document: its class and preamble are used, and everything but the diagrams in its body is left out.

```
commute notes.tex                 # notes-1.pdf, notes-2.pdf, ... from the diagrams in the body
```

The first run with a new preamble loads it with real tikz-cd, which takes some 300 ms. Later runs take it from a checkpoint in the cache directory, which holds for as long as nothing the preamble read has changed: from 35 to 140 ms a diagram on the machines it has been measured on, since each diagram then has a run of its own.

`--preset KEYS` is `\tikzcdset{KEYS}` before every diagram, without a preamble:

```
commute --preset 'every arrow/.append style={thick}' figure.tex
```

## When it says no

`commute` draws diagrams with its own library, the one the engine uses inside documents. A diagram outside what the library knows is not guessed at:

```
$ commute figure.tex
commute: diagram 2 (line 14): not handled: an undefined control sequence in a cell: \mathscr
commute: diagram 3 (line 20): not handled: unknown arrow key: -Stealth
```

The other diagrams of the file are written, and the status is 1. The first message wants `--usepackage mathrsfs`. For the second, ask for real PGF:

```
commute --real-pgf figure.tex
```

That loads tikz-cd and runs it for those diagrams only, so it takes as long as that takes. Under a preamble this is what happens anyway. Real PGF gives a PDF and no SVG, which is said, and which has a status of its own, 3, unless you asked for `--pdf`.

Under a preamble the library is only used on a TeX tree it has been verified on. On any other, as after an update of PGF, real PGF draws every diagram: the PDF is right, it takes as long as real PGF takes, and there is no SVG, which says so and why:

```
commute: diagram 1 (line 1): no SVG: real PGF drew it, and only the library draws SVG: a macro or key of tikz-cd or PGF is not as installed: pgfscope; `commute --check-tree` says whether that is this TeX tree
```

`commute --check-tree` says what this tree is and what stands in the way, the same as `rstex --check-tree`: see the README. Without a preamble the library draws as usual on any tree, which is not to say it is what that tree's PGF would draw; there too it is as verified on the versions that command names.

It also says no to a cell it cannot vouch for, where TeX might read an arrow or a `&` otherwise than it looks, or where a real cell would come out differently from one here:

```
commute: diagram 1 (line 1): not handled: a primitive at the end of a run looks for what follows
commute: diagram 2 (line 2): not handled: a control sequence assigned in a cell
commute: diagram 3 (line 3): not handled: a primitive that has no place in a formula
commute: diagram 4 (line 4): not handled: a macro in a cell looks for its argument beyond the run: \frac
```

Those are `\iffalse \arrow[r] \fi`, a `\def` directly in a cell, a `\vskip`, and a `\frac` whose second argument stands behind the arrow. A diagram like these is a rare thing, and the message is the whole of the cost: nothing has been run, and the other diagrams are written.

An error that only shows when the cell is typeset is TeX's own, with its first line:

```
commute: diagram 5 (line 5): ! Missing $ inserted.
```

and that diagram is not written either.

## Options

| Option | Meaning |
|---|---|
| `-o DIR` | write into DIR, made if need be, instead of next to the file |
| `--stem NAME` | base name of the output files, instead of the file's |
| `--pdf`, `--svg` | only that one |
| `--preset KEYS` | keys as in `\tikzcdset{KEYS}`, before every diagram |
| `--usepackage P` | also load package P, written `P` or `[options]{P}`; may be given again |
| `--preamble FILE` | also read FILE as part of the preamble |
| `--real-pgf` | let real PGF draw what commute itself declines |
| `--box` | print each diagram's width, height and depth, as `\the` prints them |
| `--check-tree` | as the first option: whether this TeX tree is one the library is verified on |
| `--version`, `--help` | |

Exit status: 0 all written, 1 some diagrams were not, 2 usage or I/O error, 3 every diagram has its PDF and some have no SVG. What is worst counts: a diagram with nothing at all is a 1, whatever the others had.

## Good to know

- The same input gives the same bytes on every run: a PDF holds no date and no id. Nothing in the output depends on the system: CI compares the PDF and SVG of the diagrams of `tests/cd` on Linux, macOS and Windows, and they are the same bytes (`docs/BUILD.md`, "What has run on macOS and Windows").
- Diagrams of one file without a preamble share one TeX, so what a macro in a cell does globally, as `\stepcounter` does, is seen by the diagrams after it. With a preamble each has its own.
- A PDF holds no `ToUnicode` table and its fonts are not compressed, as both cost more time than a small file is worth: about 1 KB against 20. Under a preamble both are as usual.
- What is built goes to the cache directory (`~/.cache/rstex`, `~/Library/Caches/rstex`, `%LOCALAPPDATA%\rstex\cache`), and is built again if the tree or a preamble changes. It can be deleted at any time. Without a preamble the files of the format are looked at once a minute, not on every run.
- A cell that holds an image or a `\pdfliteral`, or uses a font that is not Type 1, has no outline to draw. The PDF is written, and the SVG is not, and that is said, with status 3.

The design is in [`DESIGN.md`](DESIGN.md), section 10, and `tests/cd/compare_cli.py` is what compares the output with real PGF.
