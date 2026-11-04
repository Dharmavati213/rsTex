# rstex

A TeX-to-PDF compiler written entirely in Rust, and `commute`, a tikz-cd-compatible commutative diagram tool.
The main goal is performance, while running real LaTeX documents unchanged on Linux, macOS and Windows.

**Status: under construction.** The design is in [`docs/DESIGN.md`](docs/DESIGN.md), and the measurements
behind it are in [`docs/research/`](docs/research/).

rstex is a port of pdfTeX. It reads TeX Live's or MiKTeX's data files (packages, fonts, metrics) and does not use their programs
to compile. LuaTeX features (Lua, OpenType, `fontspec`) are out of scope for now.

## commute

`commute` draws a `tikzcd` diagram as a tightly cropped PDF and SVG in a few milliseconds, with the labels set by real TeX:

```
commute square.tex      # square.pdf and square.svg, from \begin{tikzcd} ... \end{tikzcd}
```

The first run on a machine builds its format, once, from the TeX Live tree. A file of many diagrams gives one PDF and one SVG for each, and a preamble can be added with `--preamble` or `--usepackage`.
What it draws is compared with `standalone` and real tikz-cd by `tests/cd/compare_cli.py`. [`docs/COMMUTE.md`](docs/COMMUTE.md) says how to use it.

Licensed under GPL-3.0-or-later.
