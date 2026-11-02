The tikz-cd spec is written, and a Python model built from it reproduces pdflatex's output to within 0.031 bp. The model is an independent float implementation. It takes only cell and label box sizes as input and recomputes every path.

**Verification**
- 235 diagrams matched, including all 34 from the thesis and LMB, with 1382 stroke and fill operators compared.
- Worst deviations:
  - **Coordinates:** 0.031 bp; the next worst is 0.012 bp.
  - **Bounding box:** 0.0035 pt.
  - **Text origin:** 0.013 bp.
- Not compared:
  - 10 diagrams use syntax the model doesn't implement.
  - 2 are inside `\setbox` and never reach the PDF.
  - 4 in test 23 are expected errors.
  - Part of test 27 can't be aligned with the log, because amsmath `align` typesets its body twice.
- The tests are 25 files with about 240 small diagrams, more than the 15–25 you asked for per file set. Test 23 has 11 intentional errors and the rest compile clean.

**Main findings in the spec**
- **Fonts:**
  - `ex` and `em` come from the surrounding text font, including italic in theorems.
  - Line width is `\fontdimen8\textfont3`. It was 0.39998 pt at every size tested, so read it from the font and never derive it.
  - `\tikzcd` cells are text style and labels are `\scriptstyle`, with no struts.
- **Layout:**
  - Each column width is the widest cell plus inner sep (1ex horizontally, 0.85ex vertically).
  - Arrows are drawn along the math axis, from a rectangle border that includes 0.2 pt outer sep.
  - By default the matrix centre sits on the baseline; a 1-row diagram sits on its row's baseline.
  - Bezier control points, label rectangles and tip hulls all affect the output box; cell rectangles don't.
- **Tips:** the spec has formulas, hulls and literal PDF operators for every style.
  - **Stem:** the arrow stem stops `lw` before the end for `cm to`, and 5.2`lw` for a tail.
  - **Double lines:** outer width is `lw+.45ex`, inner width is `.45ex−lw`.
  - **Squiggly:** includes the exact zigzag rule.
  - **Crossing over:** the halo is included.

**Quirks that would break a naive reimplementation**
- `Rrightarrow` and `tail reversed` don't exist in v1.0; the oracle reports an error and draws the default arrow.
- A trailing `\\` adds a row.
- `every loop`'s `->` and `shorten` have no effect.
- `every node/.style` in the environment options does nothing.
- `scale` alone has no effect on the diagram.
- `dashed` reads the line width at the point the key appears, so `dashed,thick` differs from `thick,dashed`.
- `\arrow[r] {B}` silently deletes `B`.
- A blank line in the body causes a fatal error cascade.

**Could not determine (spec section 17)**
- `arrow style=math font` is read from code only and just compiled; its geometry isn't checked.
- Byte-identical numbers would require emulating pgfmath's sin/cos tables. The real-number algorithm stays within the deviations above.
- Squiggly arrows shorter than `8lw` (about 3.2 pt) are only approximated, because pgf emits odd collinear points there. They render as a short stub with a tip.
- `transform shape` wasn't modelled. In the one test it moved the labels but not the cells.
- These weren't tested:
  - `relative`, `controls`
  - `transform nodes` used directly
  - `execute before arrows`
  - non-default cell shapes
  - multi-line cells
  - the coordinates of `rounded corners`
  - `opacity` resource dictionaries

For generic TikZ inside a diagram, I recommend falling back to the real package. How to load it after `\begin{document}` is an open engine design question.

The reference model isn't in the repo because of the write rules for this task. It lives in scratch, so it's worth copying if the Rust work wants it as an oracle.

Files are in `/home/site/Projects/rstex`:
- `docs/TIKZCD-SPEC.md`
- `tests/cd/*.tex`

The model and its tools are in `/tmp/claude-1000/-home-site-Projects/8b0542a8-8750-493c-901c-0ccb2c2f69d6/scratchpad/design/tikzcd/tools/`: `model.py`, `run.py`, `pdfops.py`, `instr.tex`.