# tikz-cd: exact specification for a native reimplementation

Target: `commute`, a Rust layouter whose output is visually indistinguishable from
`tikz-cd` without loading PGF.

Reference versions (TeX Live 2026 on this machine):

| Component | Version | File |
|---|---|---|
| tikz-cd | v1.0, 2021/05/04 | `tex/generic/tikz-cd/tikzlibrarycd.code.tex`, `tex/latex/tikz-cd/tikz-cd.sty` |
| PGF/TikZ | 3.1.11a, 2025-08-29 | `tex/generic/pgf/...` |
| Documentation | not installed | `/usr/share/texmf-dist/doc` does not exist, so this spec comes from source and experiments only |

Claims carry a tag at row, paragraph or list level:

- **[V]** verified numerically against pdflatex output (see section 1).
- **[C]** read from the source code, not separately measured.
- **[I]** inferred.

---

## 1. How this spec was verified

Two tools compare with pdflatex. **`tests/corpus/cd/compare_library.py`** is the stricter: it feeds the box of every cell and label, as the
oracle measured them, to `Diagram::layout`, writes the operators with `pdf::emit`, and compares with the oracle's page: every stroke, fill, tip and box origin to the
number, the picture's box and its width, height and depth to the scaled point, and pgf's operators token for token. What pdfTeX writes itself, its `1 0 0 1 x y cm`, is judged
through the positions it leads to; a box's contents are not compared. A document's `\tikzset` and `\tikzcdset` are found as text. Policy is `AllowAll`, so
a macro that hides an arrow shows as a difference to classify. Its `file` command does this diagram by diagram for one file of `tests/cd`.

Over `tests/corpus/manifests/cd.json` and `cd.local.json`, 3246 diagrams, TeX Live 2026, on the state of this commit:

| Measure | Result |
|---|---|
| Oracle did not get to compare | 328: 137 with a TeX error at the diagram, and in documents the oracle did not finish, 121 never reached, 55 with no page, 15 before the matrix |
| Judged | 2918 |
| Declined by the library | 279 |
| Accepted | 2639 |
| A macro hides an arrow or a `\\` (with `AllowAll`, the wrong drawing, which `StandardLatex` and the engine's policy decline) | 3 of the 2639 |
| Same drawing | 2636 of 2639 |
| Worst deviation of a point | 0.004 bp |
| Picture box, width, height and depth exact to the scaled point | 2636 of 2636 |
| pgf's operators the same, token for token | 2072 of 2636, and the other 564 but for the empty `q Q` of `\pgfsetlayers` (section 13) |

The other tool is **`tests/cd/model/`**, a floating-point Python model of sections 6 to 12. It recomputes every stroke, tip, fill,
text origin and the picture bounding box from the box dimensions of cells and labels only, and compares the result with the operators in the PDF.
The engine's path goes further: every diagram on it, 2385 of the 3246, has real PGF's content stream byte for byte, by `compare_native.py streams`. It takes fewer than the library accepts, since it also has to prove that a cell's text is what the library takes it for, and that nothing after the diagram could tell (DESIGN section 10).
It gets to 0.031 bp, which is what the reals of this spec allow (section 8.4), over `tests/cd/*.tex` plus 34 diagrams of the two real documents:

| Measure | Result |
|---|---|
| Diagrams matching | 235 |
| Stroke/fill operators compared | 1382 |
| Worst coordinate deviation | 0.031 bp; next worst 0.012 bp |
| Worst bounding-box deviation | 0.0035 pt |
| Worst text-origin deviation | 0.013 bp |

Not model-compared:

- 10 diagrams skipped because they use syntax the model does not implement:
  `-Stealth`, `arrow style=math font`, `transform shape`, `execute at begin picture`,
  `diagrams=`, user styles, and the two undefined keys.
- 2 diagrams inside `\setbox` that never reach the PDF. Their box sizes were checked by hand.
- 4 diagrams of `23-unknown-keys.tex` where an error is the expected result: 3 target nonexistent cells, 1 is a
  bare `&` in `align`. The last one shows as a `DIFF` in the run, which is meaningless for error recovery output.
- Part of `27-macros-styles-context.tex`: everything after the `align` block cannot be aligned with
  the log, because amsmath `align` typesets its body twice. That produced one `DIFF` and 2 exceptions. These are harness artefacts, not spec errors, but those diagrams are therefore unverified
  by the model. Their constructs are verified elsewhere: theorem font in test 18, inline math in test 17.

---

## 2. Conventions

- All lengths below are TeX points (pt). y points up. Picture coordinates have their origin
  at the centre of the matrix node (section 7). The formulas are written in reals; PGF and the library
  (`crates/commute/src/pgfmath.rs`) work in whole scaled points and truncate where TeX does (section 17).
- PDF output factor: **pgf writes `pt * 0.99627`** (`\pgf@sys@bp`, `pgfsys.code.tex:36`), printed
  by TeX with up to 5 decimals. True bp is 0.996264. Text positions come from pdfTeX and use the
  true factor with 3 decimals. The difference is under 0.001 bp per 100 pt. [V]
  **PGF 3.1.12 writes `(2x+1)*400/803`**, `-1` for a negative `x`, of `x` in scaled points, by `\pgf@sys@bp@correct`: 0.4pt is `0.39851` there and `0.3985` before. `commute::BpRule` has both; the numbers in this spec
  are of the first. [V] on 53 lengths, each tree's own macro against the rule, by `tests/cd/native/gen_table.py`.
- Names used below:
  - `ex`, `em`: current text font (section 4).
  - `rt`: rule thickness. `ax`: axis height.
  - `lw`: arrow line width (default `rt`).
  - `L`, `I`: outer stroke width and inner white width (`I=0` unless double).

---

## 3. Syntax

### 3.1 Environment

```
\begin{tikzcd}[<options>] <cell> & <cell> ... \\[<dim>] ... \end{tikzcd}
```

- `\tikzcd` opens with `\iffalse{\fi\ifnum0=`}\fi`, so it can sit inside `tabular`-like
  alignments. [C], and `tabular` tested in test 27. [V]
- `&` is active inside the body and means `\pgfmatrixnextcell`. It must be tokenized after the
  environment starts. Inside amsmath `align`, macro arguments and `\footnote`, the `&` already
  has catcode 4, so pgf raises `Single ampersand used with wrong catcode`. The user fix is
  `ampersand replacement=\&`. [V tests 23, 27]
- A native version may accept both catcodes. That is a superset of the oracle. [I]
- `&[<list>]` adds to the separation before the next column. **Only effective the first time that
  column is created**, normally row 1. [V test 02]
- `\\[<list>]` adds to the separation after this row. [V test 02]
- `<list>` items are comma separated: dimensions (added), `between origins`, `between borders`.
- `\\` directly before `\end{tikzcd}` **creates an extra row** with one empty cell, of height 0
  plus `row sep`. This changes the centring and suppresses `1-row diagram`. [V test 16]
- Nesting tikzcd inside tikzcd gives the error `Diagrams cannot be nested`. [C]
- A blank line (`\par`) inside the body gives `Missing $ inserted`, because cells are math, and cascades
  into a fatal error stream with no PDF. Not kept in the test files for that reason. [V]
- Comments and line breaks are fine. Spaces are insignificant in math cells.
- A nested `\begin{matrix}...&...\end{matrix}` in a cell compiles but widens: the active `&`
  expands to `&\pgf@matrix@correct@calltrue&`, giving an extra array column. Width is 30.49 pt
  against 20.49 pt with `ampersand replacement=\&`, i.e. one extra `2\arraycolsep`. [V test 26]
  Keep this behaviour by making the cell separator an active-`&` macro.

### 3.2 Cells

- A cell starts after skipping spaces. If its first token is `|`, the syntax `|[<node options>]|`
  gives per-cell options. [C `tikzlibrarymatrix`, V test 26]
- Cell types:
  - **Truly empty** (no tokens before `&`/`\\`): no node at all, zero metrics. An arrow to it is an
    error `No shape named tikz@f@N-r-c`. [V tests 23, 26]
  - `{}` or a cell holding only `\arrow`: has a node with an empty box (`w=h=d=0`). [V test 26]
  - With env option `nodes in empty cells`: empty cells also get a node with an empty box. [V test 26]
- Macro expansion is live. User macros may hide `\arrow`, `\\`, cell content. Example:
  `\newcommand{\pb}{\arrow[dr,phantom,"\lrcorner" very near start]}`. [V test 27]
  So the implementation must run inside the normal TeX expansion loop, not pre-scan source text.

### 3.3 Arrow commands

Commands are recorded while the cell is scanned and drawn after the matrix, in source order.

New syntax: `\arrow[<opts>]` or `\ar[<opts>]`.

| Old syntax [V test 19] | Meaning |
|---|---|
| `\arrow{r}` | `to=r` |
| `\arrow[o]{r}[lo1]{f}[lo2]{g}` | `to={r},o,"{f}"{lo1},"{g}"{lo2}` |
| `\rar \lar \dar \uar \urar \ular \drar \dlar` | `\arrow{r}` etc. with `[o]` and labels following |

Scanning quirks [C]:

- `\arrow[...]` followed (after optional spaces) by `{` is parsed as old syntax. In
  `A \arrow[r] {B} & C`, the `{B}` becomes `to={B}` placed *before* the options, so `r` overrides it
  and `B` silently vanishes from the cell. No error. [V]
- The bracket argument is delimited by the first `]` outside braces, so write `"{[x]}"`.

### 3.4 Option lists

Comma separated with brace protection. A label item starts with `"`:

```
"<text>"                        node options none
"<text>"'                       swap
"<text>"{<opts>}                anything after the closing brace is ignored
"<text>"'{<opts>}               swap, then opts
"<text>" <opts up to next comma>     e.g.  "f" near start
"<text>"' <opts>
```

- `<text>` ends at the next `"` outside braces.
- Commas in text need braces: `"{f,g}"`. `"a=b"` works unbraced. [V test 25]

---

## 4. Font parameters

Read at use time, in the **ambient** font. `font=` inside a cell or label does not change them. [V tests 16, 18]

| Symbol | Definition | Notes |
|---|---|---|
| `ex` | `\fontdimen5` of the current text font after `\selectfont` | pgfmath restores the font despite `\nullfont` in pictures |
| `em` | `\fontdimen6` of the same font | ambient italic/bold/tt fonts count |
| `rt` | `\fontdimen8\textfont3` | after `$\relax$` to refresh math fonts |
| `ax` | `\fontdimen22\textfont2` | same |

Measured [V]:

| Context | ex | em | rt | ax |
|---|---|---|---|---|
| article 10pt, cmr10 | 4.30554 | 10.00002 | 0.39998 | 2.5 |
| article `\tiny` (5pt) | 2.15277 | 6.80565 | 0.39998 | 1.25 |
| article `\small` (9pt) | 3.87498 | 9.24994 | 0.39998 | 2.25 |
| article `\large` (12pt) | 5.16667 | 11.74988 | 0.39998 | 3.0 |
| article `\LARGE` (17.28pt) | 7.43889 | 15.84985 | 0.39998 | 4.31999 |
| article `\Huge` (24.88pt) | 10.71062 | 22.82086 | 0.39998 | 6.22 |
| article `\ttfamily` | 4.30554 | 10.4999 | 0.39998 | 2.5 |
| theorem italic, cmti10 | 4.30554 | 10.22217 | 0.39998 | 2.5 |
| thesis: scrartcl 12pt, T1, lmodern, amsmath | 5.16667 | 11.74983 | 0.39998 | 3.0 |
| LMB: scrbook 11pt, T1, lmodern, amsmath | 4.71451 | 10.95 | 0.39998 | 2.73749 |

`rt` stayed 0.39998 at every size because the extension font is not size-scaled in these setups.
Read it from the font and never derive it. Line width therefore does **not** grow with `\Huge`,
while `ex` and `em` (and so spacing, `Rightarrow` gap, shifts) do.

---

## 5. Keys and defaults

### 5.1 Resolution model

Key lookup is pgfkeys-like. Options run in `/tikz/commutative diagrams`, with this `.unknown`
handler [C, lines 91-107]:

1. If the key name consists only of `r d l u` (any count, empty excluded): it sets the arrow target
   relative to the current cell. The value is ignored. Last one wins, no accumulation between keys.
2. Otherwise retry as `/tikz/<key>=<value>`.
3. `/tikz/.unknown` tries, in order [C, tikz.code.tex:1653]:
   - `/pgf/<key>`
   - a color (name contains `!`, or is a defined color): sets stroke, fill and text color
   - contains `-`: arrow spec
   - known shape name: shape
   - otherwise the error `I do not know the key '/tikz/<key>'`, which is recoverable (key ignored).

Handlers bind in the **current path**, so `every node/.style={...}`, `column 2/.style={...}` written
in tikzcd options define `/tikz/commutative diagrams/...` and have **no effect**. [V test 16]
`/tikz/every node/.style` is required for an effect. [C]

`\tikzset{x/.style={hook}}` fails because `hook` is not a `/tikz` key; `\tikzcdset` is needed. [V test 23]
`\tikzset{lab/.style={"#1"}}` works. [V test 27]

`\tikzset` runs in `/tikz`, `\tikzcdset` in `/tikz/commutative diagrams`, and `<path>/.cd` moves a list to
another directory. A style made in `/tikz` reads its keys in `/tikz`: `dashed` there is `/tikz/dashed`
(`on 3pt off 3pt`, not tikz-cd's), and `xshift` is not tikz-cd's. A style made in tikz-cd's directory reads them there.
A key written `/tikz/commutative diagrams/k` is tikz-cd's from anywhere, `commutative diagrams/k` from `/tikz`, and `/tikz/k` is TikZ's.
The options of `|[...]|` are read in `/tikz`, those of an arrow, a label and `cells=` in tikz-cd's directory: `\tikzcdset{s/.style={inner sep=1pt}}` works in
`cells={s}` and is an error in `|[s]|`.
A key of tikz-cd hides what `/tikz` has by that name when reached from a diagram's options. [V test 28]

A key's name is what TeX read: a line end and runs of spaces inside it are one space, so `bend⏎   left` is
`bend left`. [V test 28]

What the library does with the document's keys, in line with DESIGN section 10 (recognised subset, or real PGF
for this one diagram):

- It takes them as text (`preset`, as `\tikzcdset` would) or asks a `UserKeys` for the replacement text of
  `\pgfk@.../.@cmd` and of `/.@def`, and for what a colour's name means, which is how the engine gets at what it did not see being set. The engine answers from `eqtb` at the diagram, so groups and macros are as TeX has them: DESIGN section 10.
  A name of colour asked of it is what `\definecolor` left, base colours included; `base_colors` says whether a mix can be worked out from `color.rs`.
- `.style`, `.append style` and `.default` are understood, `#1` included. So is `.initial` on `row sep/<name>`
  and `column sep/<name>`. A key made by `.code` and its like is known to be there, and harmless until it is used; then the
  diagram is declined, unless the code is one this crate knows (`\pgfsetarrowsstart{tikzcd to}`, `\pgfsetarrowsstart{Implies}`,
  quiver's `curve`, 8.5). Styles over several arguments likewise. Keys under `/pgf` decline. Keys in any other tree do not reach a diagram.
- Changes to tikz-cd's own keys other than appending to `every ...` decline, and so does a style TikZ runs by itself
  (`every node`, `every path`, ...), a `.tip` on a tip the diagram draws, an `arrow style` other than the first, and any
  option set for `/tikz` in general (`\tikzset{nodes=draw}`) other than `>` (9.5).

### 5.2 Defaults (verbatim, lines 526-562)

```
every arrow:   /tikz/draw, /tikz/line width=rule_thickness, rightarrow
every label:   /tikz/auto, /tikz/font=\everymath\expandafter{\the\everymath\scriptstyle}, /tikz/inner sep=+0.5ex
every cell:    /tikz/shape=asymmetrical rectangle, /tikz/inner xsep=+1ex, /tikz/inner ysep=+0.85ex
every matrix:  /tikz/inner sep=+0pt
every diagram: row sep=normal, column sep=normal, /tikz/baseline=+0pt
1-row diagram: /tikz/baseline/.expanded=(\tikzcdmatrixname.base)
math mode=true, center yshift=axis_height,
crossing over clearance=+1.5ex, shift left/.default=+0.56ex, shift right/.default=1, background color=white
```

| Named sep | tiny | small | scriptsize | normal | large | huge |
|---|---|---|---|---|---|---|
| `row sep` | .45em | .9em | 1.35em | 1.8em | 2.7em | 3.6em |
| `column sep` | .6em | 1.2em | 1.8em | 2.4em | 3.6em | 4.8em |

Evaluation order of options:

- Picture: `every diagram`, then env options.
- Matrix: `/tikz/matrix of math nodes` (or `matrix of nodes` if `math mode=false`), then `every matrix`.
- Each cell: `anchor=base` (`cells={anchor=base}`), `nodes={execute at begin node=$, execute at end node=$}`, `every cell`.
- Each arrow: `every arrow`, then its options in order.
- Each label: `execute at begin node=$`, `execute at end node=$`, `every label`, then its options. Label options run after **all** arrow options.

### 5.3 Environment keys

| Key | Meaning | Status |
|---|---|---|
| `row sep=`, `column sep=` | Named size, else a list as in 3.1. `\tikzcdset{row sep/normal=1cm}` redefines a name. A new one has to be made by `row sep/<name>/.initial=2cm`: `row sep/<name>=2cm` is the error `I do not know the key`, and later `Unknown function <name>` | [V tests 28, 31, the error by hand] |
| `sep=` | Both; named sizes resolve per axis | [V] |
| `cells={<opts>}` | Append to `every cell`, which runs in the cell's scope and outside its node: `draw` here asks for no border, and `draw=<c>` leaves only the stroke's colour. `cells={nodes={...}}` reaches the cell's node: `draw` there also widens the cell metrics (section 7). As `every node` runs when the node starts, after all of `every cell`, what `nodes=` sets there holds against every cell's own keys before or after it, `cramped`'s `inner sep` and a `\tikzcdset` among them: `cells={nodes={inner sep=1pt}},cramped` has 1pt. A cell's `\|[...]\|` comes last. `nodes=` among those, or within `nodes=`, comes after `every node` has run and reaches no node of the diagram; the library declines it | [V test 38] |
| `nodes={<opts>}` | `/tikz/nodes`: appended to `every node`, so it also hits the matrix node and the labels | [C] |
| `arrows={<opts>}` | Append to `every arrow` | [V] |
| `labels={<opts>}` | Append to `every label` | [V] |
| `diagrams={<opts>}` | Append to `every diagram` | [C] |
| `every arrow/.append style={}` and likewise `every label`, `every cell`, `every matrix`, `every diagram` | | [V] |
| `execute before arrows={<tex>}` | Runs after the matrix, before the arrows | [C] |
| `ampersand replacement=\x` | `\x` means next cell instead of `&` | [V] |
| `cramped` | `every matrix`: `inner sep=-0.3em`; `every cell`: `inner sep=+0.3em` | [V] |
| `math mode=false` | Cells and labels without `$...$`. Label `font` is then inert (text size) | [V] |
| `background color=<c>` | Used by `description` fill, `crossing over` halo, `double line` inner stroke | [V] |
| `crossing over clearance=<dim>` | Halo width | [V] |
| `baseline=<dim>` or `baseline=(<coord>)` | See section 12 | [V] |
| `nodes in empty cells` | See 3.2 | [V] |
| `font=<cmd>` | `/tikz/font`: text font of all cells. Not labels (they set their own `font`). Not `ex`/`em`. It is the picture's, and `every cell` runs in each cell's scope after it, so a font that `every cell` sets, by `cells=`, `every cell/.append style`, `\tikzcdset` or a `nodes=` there, is the cells' whatever the order of the two: `[cells={font=\large},font=\small]` has `\large` cells, as `[font=\small,cells={font=\large}]` has. A cell's own `font=` comes last | [V test 39] |
| `scale=<x>` | Not on cells, labels or straight arrows, as far as the eye goes; but it scales what `shift left`, `bend` and `distance` say, and every point of an arrow goes through it and back with what that rounds, 0.002 bp. Only `scale=1` is taken here | [V test 16, 28] |
| `arrow style=Latin Modern\|tikz\|math font` | Section 9.6 | [V] / math font [C] |
| `N-row diagram/.style={...}` | `.try` after the matrix, `N` = row count | [C] |

### 5.4 Arrow keys

#### Ends

| Key | Meaning |
|---|---|
| `rr`, `dl`, ... | Target relative to current cell |
| `to=<x>` | Target. Resolution order: `<matrix>-<x>` is a node (so `2-3`), else a direction string relative to the current cell, else a plain node name (alias or label name) |
| `from=<x>` | Same for the start |

Default start and target are both the current cell. [V]

Label names must already exist when the arrow runs, so they must come from earlier arrows.
Cell aliases and `r-c` references may point anywhere. [V tests 12, 21]

A self-arrow without `loop` (`\arrow[]`) draws a stub: start and end are the centre, direction falls
back to `(0,1)`, giving a length-`lw` stroke with an upward tip. [V test 26]

#### Style keys (both ends, lines 437-465) [V tests 03-05]

Tip names are from 9.2. `dbl` means the key also runs `double line`.

| Key | start tip | end tip | extra |
|---|---|---|---|
| `rightarrow` | cap | to | |
| `leftarrow` | to | cap | |
| `leftrightarrow` | to | to | |
| `Rightarrow` | implies cap | implies | dbl |
| `Leftarrow` | implies | implies cap | dbl |
| `Leftrightarrow` | implies | implies | dbl |
| `mapsto` | bar | to | |
| `mapsfrom` | to | bar | |
| `Mapsto` | implies bar | implies | dbl |
| `Mapsfrom` | implies | implies bar | dbl |
| `hookrightarrow` | right hook | to | |
| `hookleftarrow` | to | left hook | |
| `rightharpoonup` | cap | left to | |
| `rightharpoondown` | cap | right to | |
| `leftharpoonup` | right to | cap | |
| `leftharpoondown` | left to | cap | |
| `rightarrowtail` | to reversed | to | |
| `leftarrowtail` | to | to reversed | |
| `twoheadrightarrow` | cap | double to | |
| `twoheadleftarrow` | double to | cap | |
| `dash` | cap | cap | |
| `equal`, `equals` | implies cap | implies cap | dbl |
| `dashrightarrow` | cap | to | dashed |
| `dashleftarrow` | to | cap | dashed |
| `rightsquigarrow` | cap | to | squiggly |
| `leftsquigarrow` | to | cap | squiggly |
| `leftrightsquigarrow` | to | to | squiggly |

**Not defined in v1.0** [V test 23]: `Rrightarrow`, `tail reversed`. The oracle reports
`I do not know the key '/tikz/Rrightarrow'`, ignores the key and draws the default arrow.
quiver.sty defines `tail reversed` (start = to), `2tail reversed` (start = implies), `2tail` and `no body`; all but `2tail` are
taken here. [V test 30]

#### One-end keys [V test 06]

| Key | Effect |
|---|---|
| `to head` | end = to |
| `no head` | end = cap |
| `two heads` | end = double to |
| `harpoon` | end = left to |
| `harpoon'` | end = right to |
| `no tail` | start = cap |
| `tail` | start = to reversed |
| `hook` | start = right hook |
| `hook'` | start = left hook |
| `maps to` | start = bar |

Later keys override earlier ones per end. Tips chosen by a style can be overridden by a later
one-end key.

#### Line keys

| Key | Meaning |
|---|---|
| `dashed` (cd version) | `dash pattern=on 7*lw off 4*lw`, phase 0, with `lw` **at key time**: `thick,dashed` gives 5.6/3.2 pt, `dashed,thick` gives 2.8/1.6 pt. With `Rightarrow,thick,dashed` it uses `lw`=.8, not `L` [V test 15] |
| `dotted`, `densely dotted`, `loosely dotted` | `on lw off 2pt`, `off 1pt`, `off 4pt`, `lw` at key time [V dotted, C others] |
| `densely dashed`, `loosely dashed` | `on 3pt off 2pt`, `on 3pt off 6pt` [C] |
| `dash pattern=on a off b ...` | Literal [V] |
| `squiggly` or `squiggly={<decoration keys>}` | Section 10.3 |
| `double line` | Section 10.2 |
| `double`, `double distance=<d>` | `/tikz`: outer `2L+0.6pt`, inner `0.6pt`; or outer `2L+d`, inner `d`. Tips scale with the outer width and get huge [V test 15]. With `double line` the last of them counts, and a bare `double` does not undo it (10.2) [V test 36] |
| `thick` etc. | `ultra thin` .1, `very thin` .2, `thin` .4, `semithick` .6, `thick` .8, `very thick` 1.2, `ultra thick` 1.6 pt [V thick, ultra thin] |
| `line width=<d>` | [V] |
| `draw=<c>` | Stroke color only; the tip also fills with it. Label text unaffected [V] |
| `draw=none` | No stroke, no tips, labels stay, path points still count in the bbox [V] |
| `<color>` or `color=<c>` | Stroke, fill and text color of labels [V test 15] |
| `text=<c>` | Text color of labels only. It, `color=` and `<color>` all set `\tikz@textcolor`, so the last of them in the arrow's options, those of `arrows=` first, colours the labels: `text=red,blue` gives blue ones [V test 35] |
| `opacity=<x>` | `/pgf@CAx gs /pgf@cax gs`, needs ExtGState resources [V raw, test 15] |
| `crossing over` | Section 10.4 |
| `shorten=`, `shorten <=`, `shorten >=` | Section 9.3 [V] |
| `->`, `<-`, `<->`, `-` | `/tikz` tips, `>` of 9.5; `-` means no tip [V tests 15, 29] |
| `solid` | `/tikz`: an empty dash pattern [V test 28] |
| `overlay` | Neither the arrow nor its labels count for the picture's box [V test 28] |
| `auto`, `auto=false`, `left`, `right`, `above`, `below` without a value | No effect: every label's `every label` sets `auto` after them, and its own keys decide [V test 28]. With a value they move the labels and are declined here |

#### Geometry keys

| Key | Meaning |
|---|---|
| `shift left[=x]`, `shift right[=x]` | Section 8.3 |
| `xshift=`, `yshift=`, `shift={(x,y)}` | Section 8.3 |
| `transform nodes={<transform>}` | Applies only to the two end coordinates [C] |
| `transform canvas={xshift=..,yshift=..,...}` | PDF `cm` around the whole arrow and its labels; excluded from the bbox [V] |
| `start anchor=[<transform>]<anchor>`, `end anchor=` | Section 8.1, 8.3 [V] |
| `bend left[=a]`, `bend right[=a]` | Section 8.2. Default angle 30 [V] |
| `in=`, `out=`, `looseness=`, `in looseness=`, `out looseness=`, `distance=`, `min distance=`, `max distance=` | Section 8.2 [V] |
| `relative` | Forces the relative form [C] |
| `loop`, `loop right`, `loop above`, `loop left`, `loop below` | Section 8.2 [V] |
| `curve={height=<h>,pos=<p>}` of quiver.sty | Section 8.5 [V test 30] |

#### Shorthands that set anything on labels too

- `phantom`: `draw=none` plus `labels={font=,anchor=center}`. So label text is textstyle and
  centred; order relative to the label in the option list does not matter. [V] Being `labels=`, it is appended to `every label`
  where it stands: after the diagram's `labels=` and what `\tikzcdset` appended, which it so overrides, and among the arrow's own
  `labels=`, those of `arrows=` first, in the order written: `labels={font=\small},phantom` is textstyle, `phantom,labels={font=\small}`
  is `\small`. A label's own options still come last. [V test 37]
- `near start` and the like at arrow level set the default `pos`. `swap` at arrow level toggles
  the default side. Label options then override or toggle again. [V test 07]

### 5.5 Label keys

| Key | Meaning |
|---|---|
| `'`, `swap` | Toggle side |
| `at start`, `very near start`, `near start`, `midway`, `near end`, `very near end`, `at end` | pos = 0, .125, .25, .5, .75, .875, 1 |
| `pos=<t>` | Also values outside [0,1] [C] |
| `description` | `anchor=center`, `fill=<background color>`. Rectangle shape, so **bbox centre**, not axis [V] |
| `marking` | `font=` (textstyle), `sloped`, `shape=asymmetrical rectangle`, `anchor=center`, so centred on the math axis and rotated with the line [V test 07] |
| `anchor=<a>` | Explicit, turns auto off [V] |
| `above[=d]`, `below[=d]`, `left[=d]`, `right[=d]` | Anchor south/north/east/west, auto off, optional translation d [V test 25] |
| `sloped` | Section 11 [V] |
| `allow upside down` | Keep `R` rotated by `T` even if `T.x<0` [V test 15] |
| `inner sep=`, `inner xsep=`, `inner ysep=`, `outer sep=`, `minimum width=`, `minimum height=` | [V inner sep] |
| `font=<cmd>` | **Replaces** the `\scriptstyle` setup. `font=\footnotesize` gives textstyle at 8pt [V test 16] |
| `text=<c>`, `<color>` | Text color [V] |
| `fill=<c>` | Rectangle fill [V] |
| `name=<n>`, `alias=<n>` | Node name for `from`/`to` [V name] |
| `xshift=`, `yshift=`, `shift={(x,y)}`, `rotate=` | Node transform after pos, auto anchor unchanged [V] |
| `scale=` | A node's own transform, so its shape and text scale too, about what `pos` gave, and the shifts after it scale [V test 28] |
| `overlay` | This label does not count for the picture's box [V test 28] |
| `color=<c>`, `<color>`, `fill=<c>` as operators | Section 10.5 [V test 28] |

Empty label `""{name=x}` is the standard way to make a 2-cell anchor. It has a node with `w=h=d=0`
and is placed with auto like any other label. [V test 21, thesis line 923]

---

## 6. Typesetting cells and labels, node geometry

### 6.1 Boxes [C, V via dimensions]

```
cell box  = \hbox{ <font option> $ { <tokens> } $ }      text style, not display style
label box = \hbox{ <font option> $ \scriptstyle { <tokens> } $ }
```

- The `\scriptstyle` is delivered by `\everymath`, so a label's own `\displaystyle` still works.
- The braces are a real math sub-group (`\bgroup..\egroup`), with `\unskip` after it. Italic correction
  of a single letter is kept: `C` measures 7.86249 pt, not 7.15 pt. [V]
- No strut: cell height and depth are those of the content. `text height=` and `text depth=` override
  the box `\ht`/`\dp` before geometry. [V test 26]
- `\mathsurround` applies if the document sets it. [C]
- `\arrow` commands in a cell leave nothing in the box.
- Color: text uses `.`, the current color at the start of the picture, unless overridden. [V test 15]

### 6.2 Rectangle and asymmetrical rectangle

Input: box `w,h,d`; `ixs`, `iys` (inner sep); `ox`, `oy` (outer sep, default `0.5*\pgflinewidth`);
minimum size 1 pt.

```
W = max(w + 2*ixs, 1pt)                 H = max(h + d + 2*iys, 1pt)
NE = ( W/2 + w/2 + ox ,  H/2 + (h-d)/2 + oy )
SW = (-W/2 + w/2 - ox , -H/2 + (h-d)/2 - oy )
cx = w/2                 cy = (h-d)/2
```

Coordinates are relative to the box's left end on its baseline.

Outer sep values [V]:

- Cells: line width in force at cell time, 0.4 pt, so `ox=oy=0.2pt`. Env option `line width=1pt`
  makes it 0.5 pt, and leaves arrows at `rt`. [test 15]
- Matrix: same.
- Labels: `0.5*lw` of the arrow. `double` does not change it; `thick` makes it 0.4 pt.

Defaults: cells `ixs=ex`, `iys=0.85ex`. Labels `ixs=iys=0.5ex`. Matrix `0`. All use the ambient `ex`,
also for a label with `font=\tiny`.

Anchors, local coordinates [C lines 309-366, V]:

| Anchor | rectangle | asymmetrical rectangle |
|---|---|---|
| `center` | (cx, cy) | (cx, **ax**) |
| `west`, `east` | (SW.x, cy), (NE.x, cy) | (SW.x, **ax**), (NE.x, **ax**) |
| `real center`, `real west`, `real east` | n/a | as rectangle's center/west/east |
| `north`, `south` | (cx, NE.y), (cx, SW.y) | same |
| `north east`, `north west`, `south east`, `south west` | corners | same |
| `base`, `base west`, `base east` | (cx, 0), (SW.x, 0), (NE.x, 0) | same |
| `mid`, `mid west`, `mid east` | (cx, .5ex), (SW.x, .5ex), (NE.x, .5ex) | same |
| `text` | (0, 0) | same |
| number `a` | `border(polar(trunc(a), 1pt))` | same, asymmetrical border |

Here `ax` is the axis height at evaluation time, i.e. ambient (section 4).

### 6.3 Border point

```
border(node, P):                     // P in the node's frame; result same frame
  c  = anchor(center)
  v  = P - c
  if |v| < 0.02pt: return c          // pgf warns
  hw = (NE.x - SW.x)/2
  if asymmetrical:
      hh = (v.y > 0) ? NE.y - ax : ax - SW.y
  else:
      hh = (NE.y - SW.y)/2
  if |v.x|*hh >= |v.y|*hw:  return c + (sign(v.x)*hw, v.y*hw/|v.x|)
  else:                     return c + (v.x*hh/|v.y|, sign(v.y)*hh)
```

So a cell's border is the rectangle *with outer sep*, but the ray starts on the math axis: horizontal
arrows sit at the axis, not at the geometric centre. [V all tests]

pgf's own arithmetic uses limited precision (`8192pt/count` reciprocals). Exact arithmetic deviates
by under 0.004 bp. [V]

---

## 7. Matrix layout

Inputs:

- Cells in row order. For each present node, `w,h,d` and its node geometry (cell origin is the
  `base` anchor, so the text box starts at `x = -w/2`).
- `rowsep`, `colsep` values and their `between origins` flags.

```
for each row r, for each cell c (in order):
    // decide the next column's separation the first time it is needed
    if c is not the last cell of this row and column c+1 has no separation yet:
        colsep[c+1] = colsep_opt + extras from "&[...]" that ended this cell
        fixed[c+1]  = last of between origins/borders in (colsep_opt, extras)

    if node:  l = -W/2;  rr = W/2;  t = NE.y - oy;  b = SW.y + oy
              // if node is stroked (draw): l-=lw/2, rr+=lw/2, t+=lw/2, b-=lw/2 with lw = 0.4pt
    else:     l = rr = t = b = 0

    if fixed[c]:   l  = 0
    if fixed[c+1]: rr = 0
    if c > 1:      l -= colsep[c]
    if r > 1 and rowfixed[r-1]: t = 0

    colmin[c] = min(colmin[c], l);   colmax[c] = max(colmax[c], rr)
    rowmax[r] = max(rowmax[r], t);   rowmin[r] = min(rowmin[r], b)   // both start at 0

at the end of row r < n:
    y = rowsep_opt + extras from "\\[...]"
    rowfixed[r] = last of between origins/borders
    rowmin[r] = rowfixed[r] ? -y : rowmin[r] - y

Y[n] = 0;   Y[r-1] = Y[r] + rowmax[r] - rowmin[r-1]
X[1] = -colmin[1];   X[c+1] = X[c] + colmax[c] - colmin[c+1]

Wm = sum_c(colmax[c] - colmin[c])
Hm = Y[1] + rowmax[1]
Dm = raw depth of the last row (rowmin[n] without any y)
```

`between origins` semantics [V test 02]:

- Distance between origins is exactly `y`.
- Later rows' heights are dropped, so the matrix top is only the first row's height.
- The same applies to columns: internal columns contribute no width at all.

Matrix node: rectangle with inner sep `m` (0, or `-0.3em` if cramped), outer sep 0.2 pt, anchor
`center` at the picture origin. With `W' = max(Wm+2m,1pt)`, `H' = max(Hm+Dm+2m,1pt)`:

```
picture position of cell origin (r,c) = ( X[c] - Wm/2 ,  Y[r] - (Hm-Dm)/2 )
matrix background rectangle           = [-W'/2, W'/2] x [-H'/2, H'/2]
```

Check on `01-square.tex`: `Wm=57.64157`, `Hm=42.64586`, `Dm=3.65973`, offset (`-28.82079`, `-19.49306`). [V]

Cell drawing order: row-major, before any arrow. Cell text box origin: cell origin minus `(w/2, 0)`.

---

## 8. Arrow paths

### 8.1 Straight

`A`, `B` are start and end nodes; `a`, `b` optional anchors from `start anchor`, `end anchor`.

```
pa = a ? anchor(A,a) : none
pb = b ? anchor(B,b) : none
E  = pb ?: border(B, pa ?: anchor(A,center))
S  = pa ?: border(A, E)
```

`S`, `E` are the "timer" points for labels. Later shortening and tips do not move labels. [V]

Named label nodes used as ends are rectangles and follow the same rule. [V test 12, 21]

### 8.2 Curves

Common control length [V]:

```
dist(S,E):
   dx=|E.x-S.x|, dy=|E.y-S.y|;  (nx,ny) = unit(dx,dy), rounded to 5 decimals as pt values
   if nx > ny:  c = floor(nx*65536)/255 (integer division);  len = dx*256/c
   else:        c = floor(ny*65536)/255;                     len = dy*256/c
   d = 0.3915 * len
   xa = clamp(d*out_looseness, out_min, out_max)
   xb = clamp(d*in_looseness,  in_min,  in_max)
```

Defaults: looseness 1, min 0 pt, max 10000 pt.

**Relative form** (`bend left=a`: out=a, in=180-a. `bend right=a`: out=-a, in=180+a. Default a=30, or the last `a` these options gave a bend: `bend right=20,bend left` bends by 20 [V test 33]):

```
A0 = a ? anchor(A,a) : anchor(A,center);    B0 = likewise
S  = a or A is a coordinate ? A0 : border(A, A0 + rot(B0-A0, out))
E  = b or B is a coordinate ? B0 : border(B, B0 + rot(A0-B0, 180+in))
u  = unit(E-S);   (xa,xb) from dist(S,E)
c1 = S + xa*rot(u, out);   c2 = E + xb*rot(u, in)
```

**Absolute form** (`in=`, `out=`, `looseness` without bend; defaults out=45, in=135):

```
A0 = a ? anchor(A,a) : anchor(A, trunc(out));    B0 = b ? anchor(B,b) : anchor(B, trunc(in))
(xa,xb) from dist(A0,B0)
c1 = A0 + xa*polar(out);   c2 = B0 + xb*polar(in)
S  = a ? A0 : border(A, c1);   E = b ? B0 : border(B, c2)
```

Mixing: `in=`/`out=` after a `bend` keep the relative flag, so `[bend right,in=150]` is relative. [V test 11]

**Loops**: `loop` means target = start, `looseness=8`, `min distance=5mm`, absolute form.

| Key | out | in |
|---|---|---|
| `loop right` | 15 | -15 |
| `loop above` | 105 | 75 |
| `loop left` | 195 | 165 |
| `loop below` | 285 | 255 |

The `looseness=8` and `min distance=5mm` belong to `loop`'s `to path`, not to keys that a later key could change:
any later key that switches to the plain curve (`looseness`, `distance`, `in`, `out`, `bend left`, ...) replaces
that `to path` and takes them with it. `loop right,looseness=4` is a curve from the node to itself with looseness 4 and
no minimum distance; `loop right,bend left` is a degenerate one; `looseness=4,loop right` is the plain `loop right`. [V test 31]

`loop`'s `to path` starts with `\let\tikztotarget=\tikztostart`, so the end is the start, anchor included: a `start anchor` holds at both ends and an `end anchor` goes unread, its `[...]` too. It runs on each pass over
the `to path`, so it does the same after 8.3's dry path: the arrow goes from `a` back to `a`, and `b` counts only for the direction of a `shift left`. [V test 32]

`every loop` is `{->,shorten >=1pt}` in TikZ, but it runs inside the `to` scope and has **no effect**
on the arrow: tikz-cd tips and no shortening remain. [V test 11, including `hook,thick`]

`loop right` also sets `/tikz/right`, overwritten by each label's `auto`. So labels use the normal
auto logic. [V]

### 8.3 Shifts and end transforms

If any of `shift left/right`, `xshift`, `yshift`, `shift`, `transform nodes`, or a `[...]` prefix on
an anchor is present:

```
(S0,c10,c20,E0) = path as in 8.1/8.2                      // "dry" path, contributes to bbox, not drawn
a = S0 + start_transform;   b = E0 + end_transform
```

where `start_transform` is the `[..]` of `start anchor` plus all of `xshift`, `yshift`, `shift`; same
for the end. Then for each shift-left value `r`, in key order:

```
n = unit(a-b)
a += (r*n.y, -r*n.x);   b += (r*n.y, -r*n.x)
```

Value of `r` [V test 10, 22]:

- A dimension as is: `shift left=1.15ex`.
- A unitless number is multiplied by `0.56ex`: `shift left=2` gives 1.12ex.
- `shift right=x` is `shift left=-(x)`.
- Default `shift left` is 0.56ex; `shift right` is `-1 * 0.56ex`.

Finally the path is rebuilt with `a`, `b` as plain coordinates: a line `a--b`, or 8.2 with `A0=S=a`,
`B0=E=b`. Since `xshift` also applies to the labels through their timer, labels move with the line. [V]

Shifted arrows do not re-intersect borders: both ends translate parallel to themselves. [V]

### 8.4 Precision

`dist` above depends on the normalised vector of `E-S`, which `\pgfpointnormalised` takes from pgfmath's tables through the
angle: `(nx,ny) = (cos, sin)` of `atan2`. A rounding residue in `dy` of 0.00002 pt gives `nx=0.99998`, which changes `c` by one and the
control length by 0.4%, 0.031 bp in a control point. With those tables and TeX's arithmetic, as `pgfmath.rs` has them, it is the same
number as in PGF. [V: corpus, section 1]

### 8.5 `curve` of quiver.sty

`to path={(\tikztostart) .. controls ($(\tikztostart)!\pv{pos}!(\tikztotarget)!\pv{height}!270:(\tikztotarget)$) and
($(\tikztostart)!1-\pv{pos}!(\tikztotarget)!\pv{height}!270:(\tikztotarget)$) .. (\tikztotarget)\tikztonodes}`, with `pos`
0.35 and `height` 0 unless given. With `A0`, `B0` the anchors, or centres, of the ends:

```
control(f):
   P = (1-f)*A0 + f*B0                         // !f!
   R = B0 rotated by 270 about P
   Q = h a length ?  P + h*unit(R-P)           // !h!270:
                  :  (1-h)*P + h*R
c1 = control(pos);    c2 = control(1-pos)
S  = a ? A0 : border(A, c1);    E = b ? B0 : border(B, c2)
```

It is a `to path`, so a later `bend left` or the like replaces it, and it replaces an earlier one. `shift left` and the rest
of 8.3 apply as to a curve, with coordinates as the ends. The path's own first move stands next to the one before it,
`x y m x y m`, unless a node's border took the place of that one: so with `start anchor` and with shifts. [V test 30]

The library knows `curve` by its text, and that of `settings`: another style of that name is another style. Changing `/tikz/quiver/pos` or `height`
for all arrows in a `\tikzset` declines; a `UserKeys` caller has to see that for itself.

---

## 9. Arrow tips

### 9.1 Parameter helper

```
dep(a,b,c) = a + b*L                         if I == 0
           = a + b*((1-c/2)*L - (c/2)*I)     if I > 0
```

`t = dep(0,1,1)` is the tip's own stroke width (equals `lw`, both cases). `line width` in the tip PDF
is written only if it differs from `\pgflinewidth`, except for `Implies`, which always writes it. Its place among the
tip's settings depends on the tip [V tests 03-06, 31, section 15]:

| Tip | Operators in front of the path, `[w]` if written |
|---|---|
| `Computer Modern Rightarrow` | `[ ] 0.0 d 1 J 1 j [w]` |
| `Implies` | `w w [ ] 0.0 d 1 J 1 j` |
| `Bar` | `[ ] 0.0 d [w] c J 0 j` |
| `Hooks` | `[ ] 0.0 d 0 j c J [w]` |
| `stealth`, `latex` | nothing; filled, not stroked |

### 9.2 Definitions

All are stroked, dash pattern reset to solid. `tip` and `back` are the extreme x; `line` is where the line stops. [V all]

**`to` = `cm to` = `Computer Modern Rightarrow[length=+0pt 6.2]`**. cap round, join round.

```
len = dep(0,6.2,0); wid = 2.096774*len; a = len-t; b = wid-t
tip = t/2; back = -a - t/2; line = -t/2
M(-a, b/2)  C(-.81731a,.2b)(-.41019a,.05833333b)(0,0)
            C(-.41019a,-.05833333b)(-.81731a,-.2b)(-a,-b/2)
hull: (t/2,±t/2), (-a+t/2, ±(b/2+t/2)), (-a-t/2, ±(b/2+t/2))
```

**`to reversed`**: draw with x negated.

```
tip = a + t/2; back = -t/2; line = t/2; hull x negated
```

**`left to`** (harpoon) and **`right to`** (the same with y negated):

```
same tip, back, line as `to`
M(-a,b/2) C(-.81731a,.2b)(-.41019a,.05833333b)(0,0) L(-t,0)
hull: (t/2,±t/2), (-a+t/2, b/2+t/2), (-a-t/2, b/2+t/2), (-t,-t/2), (-a-t/2, b/2-t/2)
```

**`double to`** = two `to` tips, the first with `sep = dep(0,-2.6,0)`. Same order at start and end.

**`bar`, `implies bar`** = `Bar[width=+0pt 8.2 0.89, line cap=round]`, cap round, join miter.

```
W = dep(0,8.2,.89)
tip = t/2; back = -t/2; line = -t/4
M(0,W/2) L(0,-W/2)
hull: (±t/2, ±W/2)
```

**`left hook`** and **`right hook`** (y negated) = `Hooks[width=+0pt 10.8,length=+0pt 3.6,harpoon,line cap=round]`,
cap round, join miter.

```
len = dep(0,3.6,0); W = dep(0,10.8,0); a = len - t/2; b = W - t; r = b/4; k = 0.55228
tip = len; back = -t/2; line = 0
M(0,2r) C(ka,2r)(a,r+kr)(a,r) C(a,r-kr)(ka,0)(0,0)
hull: (len/2,-t/2), (len,-t/2+W/8), (len,3W/8), (len/2,W/2), (-t/2,W/4), (-t/2,W/2)
```

**`implies`** = `Implies`. cap round, join round, stroke `b`.

```
a = (L+I)/4; b = (L-I)/2       // b equals rt-based lw
tip = 2.06a + b/2; back = -1.36a + b/2; line = 0
points in x are offset by +.06a:
M(-1.4a,2.65a) C(-.75a,1.25a)(a,.05a)(2a,0) C(a,-.05a)(-.75a,-1.25a)(-1.4a,-2.65a)
hull: (-1.4a,±2.8a), (2a,±b/2)
```

**`cap`, `implies cap`**: no tip.

### 9.3 Placement, shortening

For each end, with path end `P` (`E` or `S`), reference `R` (the neighbouring point: `S`/`E` for a line,
`c2`/`c1` for a curve, the adjacent decorated point for squiggly), user shorten `s`, tips `t1..tn` in
the listed order:

```
u = unit(P - R)              // outward; (0,1) if zero
xa = n>0 ? tip_n + sep_n - line_n : 0
P' = P - (xa + s)*u          // new line end; curve control points are NOT moved
cum = -( sum_i(tip_i + sep_i - back_i) + s )
for each tip i:
    origin = P + (cum - back_i)*u
    CTM    = [u.x u.y -u.y u.x origin]     // swap (y negated): [u.x u.y u.y -u.x origin]
    draw tip i
    cum += tip_i + sep_i - back_i
```

So the tip's front edge touches `P` when `s=0`. With `s`, the tip moves back with the line.

Start and end are handled the same way. For a start tip `u` points from second point to `S`, so a
rightward arrow gets `-1 0 0 -1 x y cm`. Hook-at-start gets `-1 0 0 1 x y cm` (swap). [V literal, section 15]

Consequence: `hook` lies on the left side of the direction of travel; `hook'` on the right. [V]

Bezier ends are shortened along the straight chord to the control point, not along the curve. There
is no `bending` library in tikz-cd. [V]

### 9.4 Sizes at the default `lw = 0.39998 pt` [V]

| Tip | Length of line stop from P | Key size |
|---|---|---|
| to | `lw` = 0.39998 | barb reach 6.2`lw` = 2.47988, width 5.19976 |
| to reversed | 5.2`lw` = 2.07991 | |
| bar | .75`lw` = 0.29999 | width 8.2`lw` = 3.27984 |
| hook | 3.6`lw` = 1.43993 | width 10.8`lw` = 4.31978 |
| double to | `lw` | head spacing 3.6`lw` = 1.43993 |
| implies at 10pt | 2.19561 (L=2.33747, I=1.53751) | |

### 9.5 Plain TikZ tips

`->`, `<-`, `<->` use `>`, which starts as `Computer Modern Rightarrow` with defaults, `len = dep(1.6pt, 2.2, 0)`. This
is nearly identical at `lw≈0.4` but does not scale proportionally with `thick`. [V]

`>=` from a `\tikzset`, or in a diagram's options, or in `diagrams=`, reaches these and all of `arrow style=tikz`, and no other tip.
`stealth` and `latex` are drawn here; another value, `>=to` among them, declines if an arrow would draw it. `stealth` and `latex` are of the old form of
declaration [V test 29]:

```
b = I>0 ? .6*L - .4*I : L;   a = .28pt + .3*b
stealth: tip = 5a; back = -3a; line = 0
         M(5a,0) L(-3a,4a) L(0,0) L(-3a,-4a) f
latex:   tip = 9a; back = -a;  line = 0
         M(9a,0) C(6.3333a,.5a)(2a,2a)(-a,3.75a) L(-a,-3.75a) C(2a,-2a)(6.3333a,-.5a)(9a,0) f
```

They have no hull: only the path counts for the box. Of their options only `reversed` shows, in x negated and `tip`, `back` swapped as
for `to reversed`, and `swap` in the `cm`; `harpoon` draws the whole tip.

Literal `q..Q` bodies for other tips reached via `/tikz` (not model-compared, from test 15, `lw`=0.39998):

```
-Stealth: q [ ] 0.0 d 0 j 4.21457 0.0 m 0.6434 1.3457 l 1.81776 0.0 l 0.6434 -1.3457 l h B Q
-latex:   q 3.5864 0.0 m 2.52373 0.19923 0.79697 0.79697 -0.39848 1.49432 c -0.39848 -1.49432 l 0.79697 -0.79697 2.52373 -0.19923 3.5864 0.0 c f Q
|->:      q [ ] 0.0 d 0 J 0 j 0.0 2.29138 m 0.0 -2.29138 l S Q     (Bar with default width 3pt+4lw)
```

### 9.6 Arrow styles

`\tikzcdset{arrow style=...}`:

**Latin Modern** [V test 20]:

- `tikzcd bar` = `Bar[width=+0pt 12 .992,line cap=round]`.
- `tikzcd left hook` = `Hooks[width=+0pt 15,length=+0pt 4.2,arc=190,harpoon,line cap=round]`.
- With `a=len-t/2`, `r=(W-t)/4`: ellipse centre `(0,r)`, radii `(a,r)`. Three Bezier arcs at parameter
  angles 100 to 10, 10 to -50, -50 to -90 (`k = 4/3*tan(d/4)`).
- `tip = a+t/2`, `back = sin(190deg)*a - t/2`, `line = 0`.
- Literal: `q [ ] 0.0 d 0 j 1 J -0.25601 2.76823 m 0.5459 2.90198 1.31062 2.39546 1.45201 1.63689 c 1.54344 1.14624 1.35118 0.64653 0.94772 0.32631 c 0.68224 0.11559 0.34659 0.0 0.0 0.0 c S Q`

**tikz** [V test 20]:

- `tikzcd to` = `>` (defaults of 9.5).
- `tikzcd bar` = `Bar[width=+3pt 4 0.9]`, cap butt.
- `tikzcd left hook` = `Hooks[harpoon]`, with `len=dep(.75pt,1.25,0)`, `W=4len-L`, cap butt, `back=0`.
- `tikzcd double to` = two `>` with no sep.

**math font** [C only]: the `Glyph` tip clips to `(0,-1ex)..(len,+1ex)` and typesets the font's own
arrow glyph. `tikzcd to` = `Glyph[glyph math command=rightarrow]`, with `glyph length=0.9ex`,
`glyph shorten=-0.1ex`, `glyph axis=axis_height`, `tip=len+shorten`. See lines 385-408 and 507-523.
Needs the engine to typeset math into the tip. `double line` then uses
`double distance=2*(height("$=$")-axis_height-rule_thickness)`. Recommended: fallback.

---

## 10. Stroke rendering

### 10.1 Order inside one arrow [V]

1. Halo, if `crossing over`.
2. Outer stroke of the line.
3. Inner stroke (double only).
4. Start tips.
5. End tips.
6. Labels in order: fill, then text.

Arrows run in source order after all cells, so an arrow can only cover earlier arrows.

Default line stroke: butt cap, miter join, current color.

### 10.2 Double line

`double line` means `double equal sign distance` (`double distance between line centers=0.45ex`) and
`double=<background color>`:

```
L = lw + 0.45ex     I = 0.45ex - lw
```

- Stroke the path at `L` in the stroke color, then at `I` in the background color. Both inherit the
  dash pattern.
- Tips then use `L`, `I`.
- If `I<=0`, no inner stroke. [C]
- Label outer sep still uses `lw`. Dash coefficients use `lw` at key time (not `L`). [V tests 05, 15]
- TikZ keeps one `\tikz@double@setup` and one inner colour, and each key writes them in the arrow's option order, `arrows=` first:
  `double line` (so `Rightarrow`, `equal`, `Mapsto` and the others marked `dbl` in 5.4) both, `double distance=<d>` the setup,
  `double=<c>` the colour, and a bare `double` neither, as it only turns double lines on. So `double,Rightarrow`,
  `Rightarrow,double` and `double distance=2pt,Rightarrow` are all the 0.45ex line of `Rightarrow`, `Rightarrow,double=red` has a
  red inner stroke and `double=red,Rightarrow` one in the background color, and `Rightarrow,double distance=2pt` has `I = 2pt` and `L = 2lw+2pt`. [V test 36]

At 10pt: `L=2.33747`, `I=1.53751`. [V] `equal` draws only the two strokes, up to the border.

### 10.3 Squiggly

Keys: `decorate`, `decoration={zigzag, segment length=9.25lw, amplitude=1.9lw, post=lineto,
post length=6lw, pre=lineto, pre length=6lw, <args>}`. The decoration runs before shortening.

`segment length` and `amplitude` are evaluated when the key runs, so with the line width at that point.
`pre length` and `post length` are evaluated when the path is decorated, so with the final line width. `squiggly,thick` and
`thick,squiggly` therefore differ. [V test 31]

With `D` the path length (arc length for a curve), `s = 9.25lw`, `amp = 1.9lw`, `p = 6lw` (`lw` as just said),
`M = D - 2p`, `k = M >= s/2 ? floor(M/(s/2)) : 0`:

```
points along the path, as (arc position x, normal offset y) with +y to the left of travel:
m  (0,0)
l  (p,0)
l  (p + (i-1)*s/2 + s/4 along the tangent at x0=p+(i-1)*s/2,  (-1)^(i-1)*amp),  i = 1..k
if k > 0:  l  (p + k*s/2, 0)
line segment case:  l (p + k*s/2, 0)      // duplicate point
curve case:         l (D - p, 0)          // real end of main section
l  (D, 0)    shortened by the end tip
```

Each zigzag point is computed in the frame at `x0` (position and tangent there), then displaced by
`(s/4, ±amp)` in that frame. Only the last segment and the first are shortened. Tips read their
direction from those segments. [V test 05, 26: 0.0065 bp on a curve]

If `D < 8lw` the points are collinear and collapse in irregular ways, sometimes overshooting by less
than `lw`. Draw a straight line. [V, approximate below this length]

On a very short curve, or one nearly a straight line along an axis, such as `in=30,squiggly` from a node to itself or `d,looseness=0.1,squiggly`, PGF's walk along it asks for the angle of a vector whose one part is next to nothing, and
`\pgfmathreciprocal@` of less than 0.00006 has no dimension: `Dimension too large`, any number of times, and other numbers from there on. The library declines such a diagram, and one whose search for a distance runs past time 1.25 on the curve,
and that is real PGF's to draw. In the engine, where only the measured boxes show it, that starts the job again. [V test 33]

Oracle requires `\usetikzlibrary{decorations.pathmorphing}`, otherwise
`You need to say \usetikzlibrary{decorations.pathmorphing} to use squiggly arrows`. [V]

### 10.4 Crossing over

A preaction stroke of the same path: color `background color`, width `crossing over clearance`,
no tips, not double, butt cap. It uses user `shorten` but no tip shortening. Its width counts in the
bbox. [V test 14]

### 10.5 Color operators

A bare colour key goes through `\color`: `1 0 0 rg 1 0 0 RG`, fill first, and a push on the colour stack. When its group ends the
stack is popped, and a pop writes what is then on top, so `0 g 0 G` stands before the arrow's closing `Q`. `color=<c>` is `\pgfsetcolor`:
`1 0 0 RG 1 0 0 rg`, stroke first, no pop. `draw=<c>` is `0 1 0 RG`, and each tip of the arrow then has `q 0 1 0 rg` in front of its
`cm`, unless a later colour key made stroke and fill the same. `text=<c>` writes nothing among the options. Color models follow xcolor
(`cyan` gives `1 0 0 0 K`, `yellow` is CMYK, `red!50!blue` gives `.5 0 .5`). [V tests 15, 28]

A node runs its colour options twice: in its `\pgfscope`, and in a scope around its path, which holds a label's fill and a cell's border:

```
label `"f"{red}`:     q  c c  q c c  <pop: c c>  Q  <box>  <pop: what the arrow left>  Q
label `description`:  q  c  q c  x y w h re f  Q  <box>  Q          (c = background color, as `fill=`)
cell `|[red]|`:       q q  c c  q c c  <pop>  Q  <box>  <pop>  Q Q
cell `|[draw]|`:      q q  x y w h re S  q cm q 0 G 0 g <text> Q Q  Q Q
```

Around a box stands `q a b c d 0.0 0.0 cm q c1 RG c2 rg <text> Q Q`, with the colour of the node: pgf's `\pgfsetcolor{.}`. [V tests 26, 28]

So what is filled or stroked has the colour that the keys left, in their order: a colour key after `fill=<c>` or `description` fills
a label's box in that colour, as `"f"{fill=yellow,red}` and `labels={fill=yellow}` with `"f"{red}` have a red one, and a colour key
after `draw=<c>` strokes a cell's border and an arrow's line, with `every cell`'s keys before the cell's own: `|[draw=blue,red]|`
and `[cells={draw=red}]` with `|[draw]|` have a red border, `draw=red,blue` a blue line. The SVG takes these colours from the same
list of options as the PDF. [V test 40]

`\usepackage{tikz-cd}` must make xcolor available, since pgf loads it. [C]

---

## 11. Label placement

```
time t  = label pos ?: arrow pos ?: 0.5
line:   P = S + t*(E-S);          T = unit(E-S)
curve:  P = bezier(t);            T = unit(e - d), d and e being the two points of de Casteljau's last step
```

`S`, `E` here are the unshortened, shifted end points. [V]

Auto anchor, on the unit vector `(x,y)`; `(x,y)=(1,0)` if the label is sloped [V test 08, 07]:

| | y > .05 | \|y\| <= .05 | y < -.05 |
|---|---|---|---|
| left (default), x > .05 | south east | south | south west |
| left, x < -.05 | north east | north | north west |
| swap, x > .05 | north west | north | north east |
| swap, x < -.05 | south west | south | south east |

For `|x| <= .05`: left gives `east` if `y>0`, else `west`. Swap gives `west` if `y>0`, else `east`.

Transform:

```
R = identity, or if sloped rotation by T (flip T if T.x < 0 unless allow upside down)
then node keys (xshift, yshift, rotate, ...) in key order, so xshift acts in the rotated frame [V test 15]
text box origin O = P + R*(shift) - R*anchor_point
```

The rectangle for the fill and bbox runs from `SW+outer` to `NE-outer`, transformed by `R`.

Check: `f` on `01-square.tex`: `P=(-0.32031,15.15976)`, anchor south, `O=(-2.6623,18.8738)` pt,
which is PDF `-2.652 18.803`. [V]

`phantom`, `description`, `anchor=center` use the rectangle centre; `marking` uses the axis point.
Therefore a `\square` in a `phantom` arrow sits with its bbox centre at the midpoint of the
border-to-border chord. [V test 09, 22]

---

## 12. Picture box and baseline

Start with empty bbox. Add [V]:

- Matrix rectangle (section 7).
- Each arrow:
  - dry path points if any (8.3);
  - `S,c1,c2,E` of the real path (control points count, and `E`/`S` before shortening);
  - decorated points [V: corpus];
  - if stroked, and without `transform canvas`, widen the path's bbox by half the stroke width (outer `L` for double, halo width if halo);
  - without `transform canvas`, every tip hull point under its CTM.
- Each label: the 4 corners of its background rectangle (no outer sep). If drawn, widen by half the line width.

Under `transform canvas` the points and labels above count where they would be without it. With `overlay` on the arrow nothing of it and
its labels counts, and with `overlay` on a label nothing of that label. [V test 28, corpus]

Cell rectangles do **not** count: each cell picture interrupts the bounding box. With `cramped` the
bbox is therefore smaller than the union of the cell rectangles. [V test 16]

Baseline value `base` [V test 17]:

| Setting | `base` |
|---|---|
| default, more than one row | `0`, so the matrix centre sits on the text baseline |
| 1-row diagram | `-(Hm-Dm)/2`, the baseline of that row is on the text baseline |
| `baseline=<dim>` | that value (`-0.5ex` gives -2.15277) |
| `baseline=(<node>.<anchor>)` | y of that anchor, evaluated at the end of the picture |
| `(current bounding box.center)` | y centre of the final bbox |
| `([yshift=-.5ex]current bounding box.center)` | same with shift |

`\tikzcdmatrixname`, `\tikzcdmatrixname-r-c`, aliases and label names are valid node names.

Output box:

```
width  = maxx - minx
height = maxy - base
depth  = base - miny
origin of picture coordinates at ( -minx , -base ) from the box reference point
```

Check: `BOX 57.17142pt, 25.88748pt, 23.15279pt` and `56.80893pt, 13.22772pt, 3.65973pt` from `\setbox`. [V test 17]

- In text, the box starts a paragraph (`\leavevmode`). In math, it is an Ord atom. So
  `\[ \begin{tikzcd}...\end{tikzcd} \]` is centred on the baseline by box dimensions, not on the axis. [C, V test 17]
- Inside amsmath `align` the body runs twice, so typesetting must have no side effects. [V test 27]

---

## 13. PDF stream structure

This is what pgf writes, and `pdf::emit` writes the same operators, token for token, on all but the diagrams section 1 lists.

```
1 0 0 1 Ox Oy cm                       pdfTeX moves to the picture origin
q 0 G 0 g 0.3985 w                     pgfpicture start, 0.4pt default width
q q q
  1 0 0 1 mx my cm  1.0 0.0 0.0 1.0 0.0 0.0 cm      matrix box origin
  q
    1 0 0 1 ... cm                     to the cell's origin; an empty cell: q Q
    q q <node options> q cm q c c <text> Q Q Q Q    10.5
    ...
  1 0 0 1 mx' my' cm  Q
  1 0 0 1 -mx -my cm
Q Q
q <arrow options> <line> S
  q ux uy -uy ux Tx Ty cm q <tip> S Q Q  ...
  q q 1 0 0 1 tx ty cm 1.0 0.0 0.0 1.0 0.0 0.0 cm q 0 G 0 g <text> Q 1 0 0 1 -tx -ty cm Q Q     label
Q ...
Q n Q
1 0 0 1 -Ox -Oy cm
```

- The arrow's options come in key order (`every arrow` first): `<w> w`, dash, colours, `gs`, `cm` of `transform canvas`, `J`, `j`. [V]
- A colour by itself is `\color`: it goes on `\pdfcolorstack`, as one literal `r g b rg r g b RG`, and comes off as its group ends, when what is then on top is written. A node with a text colour, by `text=`, a colour by itself or `color=`, has that `\color` once more around its text inside its box, after the `RG rg` of `\pgfsetcolor`: so a `\textcolor` in the text comes back to the node's colour, and that to what is around it. A sink that moves TeX itself gets these as `color_push`, `color_pop` and `box_color`, and the stack is pdfTeX's. [V test 15, native r7-colorstack]
- `every cell` runs in the cell's first scope, once, and a cell's own keys and those of `nodes=` around its node, twice: `q <every cell> q <own> q <own> ... Q ... Q Q`, and a colour of `every cell` comes off after the last `Q`. [V native r7-cells]
- With double: `q L w <path> S q I w 1 G <path> S Q <tips> Q`.
- Dash: `[ a b ] 0.0 d` in the arrow scope, reset by the tip.
- A shifted arrow has `q Q` for each of its coordinate nodes (2, and 2 per `shift left`) in front of the arrow's own scope.
- The `1 0 0 1 x y cm` are pdfTeX's own, from where TeX stands when a literal or a box comes: `pdf_set_origin` in thousandths of a bp, not moving under
  `min_bp_val`. A `PdfSink` that moves TeX itself gets these positions and leaves them to pdfTeX; otherwise `pdf::emit` writes them as pdfTeX would.
- `\pgfsetlayers` adds an empty `q Q` for each layer: those before `main` after the picture's `0.3985 w`, those after it before the `n`. The library hears of them from its `PdfSink`, and the engine reads `\pgf@layerlist`. The tikzit package and the library `backgrounds` set it.
- TeX rules inside labels (`\frac`) appear as pdfTeX's `[]0 d 0 J` strokes, distinct from pgf's `[ ] 0.0 d`.

---

## 14. Worked example: `tests/cd/01-square.tex`

Inputs from TeX (article 10pt): `ex=4.30554, em=10.00002, rt=0.39998, ax=2.5`. Cells: A (7.50002, 6.83331, 0),
B (8.0868, ...), C (7.86249, ...), D (8.55695, ...).

```
half widths with inner sep:  col1 max(8.05556, 8.23678)=8.23678   col2 max(8.34894, 8.58401)=8.58401
colsep 2.4em = 24.00005;  rowsep 1.8em = 18.00004
Wm = 8.23678*2 + 24.00005 + 8.58401*2 = 57.64163  (TeX: 57.64157)
Y[2] = 0;  Y[1] = 10.49302 + 3.65971 + 18.00004 = 32.15277
Hm = 42.64579;  Dm = 3.65971;  centre offset = (28.8208, 19.493)
cell origins: A(-20.584, 12.660)  B(20.2368, 12.660)  C(-20.584, -19.493)  D(20.2368, -19.493)
A->B at y = 12.660 + 2.5 = 15.160:
   S.x = -20.584 + (3.75+4.30554+.2) = -12.32846        PDF -12.28255 m
   E.x =  20.2368 - (4.0434+4.30554+.2) = 11.68786
   line end = E.x - lw = 11.28788                        PDF  11.24583 l
   tip origin = E.x - lw/2 = 11.48787                    PDF  11.44507
```

Box: width 58.66599, height 25.88748, depth 26.35966. The bbox's left and bottom come from labels `g` and `k`,
not from the matrix.

---

## 15. Literal operators per style

Canonical diagram: `\begin{tikzcd} A \arrow[r,<style>] & B \end{tikzcd}` in article 10pt (1 row, `column sep=normal`
unless noted). Coordinates relative to the picture origin, `rt=0.39998pt`. Taken verbatim from the PDF of tests 03-06. [V]

Abbreviations for the `q..S Q` bodies:

```
{TO}   = q [ ] 0.0 d 1 J 1 j -2.07213 2.39091 m -1.69356 0.95633 -0.84995 0.27893 0.0 0.0 c -0.84995 -0.27893 -1.69356 -0.95633 -2.07213 -2.39091 c S Q
{REV}  = q [ ] 0.0 d 1 J 1 j 2.07213 2.39091 m 1.69356 0.95633 0.84995 0.27893 0.0 0.0 c 0.84995 -0.27893 1.69356 -0.95633 2.07213 -2.39091 c S Q
{HARP} = q [ ] 0.0 d 1 J 1 j -2.07213 2.39091 m -1.69356 0.95633 -0.84995 0.27893 0.0 0.0 c -0.39848 0.0 l S Q
{BAR}  = q [ ] 0.0 d 1 J 0 j 0.0 1.63379 m 0.0 -1.63379 l S Q
{DBAR} = q [ ] 0.0 d 0.3985 w 1 J 0 j 0.0 2.50446 m 0.0 -2.50446 l S Q
{HOOK} = q [ ] 0.0 d 0 j 1 J 0.0 1.95256 m 0.68224 1.95256 1.2353 1.51546 1.2353 0.97627 c 1.2353 0.43709 0.68224 0.0 0.0 0.0 c S Q
{IMP}  = q 0.39848 w [ ] 0.0 d 1 J 1 j -1.29324 2.55753 m -0.66591 1.20638 1.02301 0.04825 1.98813 0.0 c 1.02301 -0.04825 -0.66591 -1.20638 -1.29324 -2.55753 c S Q
```

```
rightarrow
    0.39848 w -12.04836 -0.91324 m 11.06526 -0.91324 l S q 1.0 0.0 0.0 1.0 11.26451 -0.91324 cm {TO} Q
leftarrow
    0.39848 w -11.64987 -0.91324 m 11.46375 -0.91324 l S q -1.0 0.0 0.0 -1.0 -11.84912 -0.91324 cm {TO} Q
leftrightarrow
    0.39848 w -11.64987 -0.91324 m 11.06526 -0.91324 l S q -1.0 0.0 0.0 -1.0 -11.84912 -0.91324 cm {TO} Q q 1.0 0.0 0.0 1.0 11.26451 -0.91324 cm {TO} Q
Rightarrow
    0.39848 w q 2.32874 w -12.04836 -0.91324 m 9.27638 -0.91324 l S q 1.53175 w 1 G -12.04836 -0.91324 m 9.27638 -0.91324 l S Q q 1.0 0.0 0.0 1.0 9.27638 -0.91324 cm {IMP} Q Q
Leftarrow
    0.39848 w q 2.32874 w -9.86098 -0.91324 m 11.46375 -0.91324 l S q 1.53175 w 1 G -9.86098 -0.91324 m 11.46375 -0.91324 l S Q q -1.0 0.0 0.0 -1.0 -9.86098 -0.91324 cm {IMP} Q Q
Leftrightarrow
    0.39848 w q 2.32874 w -9.86098 -0.91324 m 9.27638 -0.91324 l S q 1.53175 w 1 G -9.86098 -0.91324 m 9.27638 -0.91324 l S Q q -1.0 0.0 0.0 -1.0 -9.86098 -0.91324 cm {IMP} Q q 1.0 0.0 0.0 1.0 9.27638 -0.91324 cm {IMP} Q Q
mapsto
    0.39848 w -11.7495 -0.91324 m 11.06526 -0.91324 l S q -1.0 0.0 0.0 -1.0 -11.84912 -0.91324 cm {BAR} Q q 1.0 0.0 0.0 1.0 11.26451 -0.91324 cm {TO} Q
mapsfrom
    0.39848 w -11.64987 -0.91324 m 11.16489 -0.91324 l S q -1.0 0.0 0.0 -1.0 -11.84912 -0.91324 cm {TO} Q q 1.0 0.0 0.0 1.0 11.26451 -0.91324 cm {BAR} Q
Mapsto
    0.39848 w q 2.32874 w -11.74948 -0.91324 m 9.27638 -0.91324 l S q 1.53175 w 1 G -11.74948 -0.91324 m 9.27638 -0.91324 l S Q q -1.0 0.0 0.0 -1.0 -11.8491 -0.91324 cm {DBAR} Q q 1.0 0.0 0.0 1.0 9.27638 -0.91324 cm {IMP} Q Q
Mapsfrom
    0.39848 w q 2.32874 w -9.86098 -0.91324 m 11.16487 -0.91324 l S q 1.53175 w 1 G -9.86098 -0.91324 m 11.16487 -0.91324 l S Q q -1.0 0.0 0.0 -1.0 -9.86098 -0.91324 cm {IMP} Q q 1.0 0.0 0.0 1.0 11.2645 -0.91324 cm {DBAR} Q Q
equal
    0.39848 w q 2.32874 w -12.04836 -0.91324 m 11.46375 -0.91324 l S q 1.53175 w 1 G -12.04836 -0.91324 m 11.46375 -0.91324 l S Q Q
equals
    0.39848 w q 2.32874 w -12.04836 -0.91324 m 11.46375 -0.91324 l S q 1.53175 w 1 G -12.04836 -0.91324 m 11.46375 -0.91324 l S Q Q
dash
    0.39848 w -12.04836 -0.91324 m 11.46375 -0.91324 l S
hookrightarrow
    0.39848 w -10.6138 -0.91324 m 11.06526 -0.91324 l S q -1.0 0.0 0.0 1.0 -10.6138 -0.91324 cm {HOOK} Q q 1.0 0.0 0.0 1.0 11.26451 -0.91324 cm {TO} Q
hookleftarrow
    0.39848 w -11.64987 -0.91324 m 10.02919 -0.91324 l S q -1.0 0.0 0.0 -1.0 -11.84912 -0.91324 cm {TO} Q q 1.0 0.0 0.0 1.0 10.02919 -0.91324 cm {HOOK} Q
twoheadrightarrow
    0.39848 w -12.04836 -0.91324 m 11.06526 -0.91324 l S q 1.0 0.0 0.0 1.0 9.82996 -0.91324 cm {TO} Q q 1.0 0.0 0.0 1.0 11.26451 -0.91324 cm {TO} Q
twoheadleftarrow
    0.39848 w -11.64987 -0.91324 m 11.46375 -0.91324 l S q -1.0 0.0 0.0 -1.0 -10.41457 -0.91324 cm {TO} Q q -1.0 0.0 0.0 -1.0 -11.84912 -0.91324 cm {TO} Q
rightarrowtail
    0.39848 w -9.97621 -0.91324 m 11.06526 -0.91324 l S q -1.0 0.0 0.0 -1.0 -9.77698 -0.91324 cm {REV} Q q 1.0 0.0 0.0 1.0 11.26451 -0.91324 cm {TO} Q
leftarrowtail
    0.39848 w -11.64987 -0.91324 m 9.3916 -0.91324 l S q -1.0 0.0 0.0 -1.0 -11.84912 -0.91324 cm {TO} Q q 1.0 0.0 0.0 1.0 9.19237 -0.91324 cm {REV} Q
rightharpoonup
    0.39848 w -12.04836 -0.91324 m 11.06526 -0.91324 l S q 1.0 0.0 0.0 1.0 11.26451 -0.91324 cm {HARP} Q
rightharpoondown
    0.39848 w -12.04836 -0.91324 m 11.06526 -0.91324 l S q 1.0 0.0 0.0 -1.0 11.26451 -0.91324 cm {HARP} Q
leftharpoonup
    0.39848 w -11.64987 -0.91324 m 11.46375 -0.91324 l S q -1.0 0.0 0.0 1.0 -11.84912 -0.91324 cm {HARP} Q
leftharpoondown
    0.39848 w -11.64987 -0.91324 m 11.46375 -0.91324 l S q -1.0 0.0 0.0 -1.0 -11.84912 -0.91324 cm {HARP} Q
rightsquigarrow
    0.39848 w -12.04836 -0.91324 m -9.65741 -0.91324 l -8.73592 -0.15611 l -6.8929 -1.67035 l -5.0499 -0.15611 l -3.20688 -1.67035 l -1.36386 -0.15611 l 0.47913 -1.67035 l 2.32214 -0.15611 l 4.16515 -1.67035 l 6.00816 -0.15611 l 7.85118 -1.67035 l 8.77269 -0.91324 l 8.77269 -0.91324 l 11.06526 -0.91324 l S q 1.0 0.0 0.0 1.0 11.26451 -0.91324 cm {TO} Q
leftsquigarrow
    0.39848 w -11.64987 -0.91324 m -9.65741 -0.91324 l -8.73592 -0.15611 l -6.8929 -1.67035 l -5.0499 -0.15611 l -3.20688 -1.67035 l -1.36386 -0.15611 l 0.47913 -1.67035 l 2.32214 -0.15611 l 4.16515 -1.67035 l 6.00816 -0.15611 l 7.85118 -1.67035 l 8.77269 -0.91324 l 8.77269 -0.91324 l 11.46375 -0.91324 l S q -1.0 0.0 0.0 -1.0 -11.84912 -0.91324 cm {TO} Q
leftrightsquigarrow
    0.39848 w -11.64987 -0.91324 m -9.65741 -0.91324 l -8.73592 -0.15611 l -6.8929 -1.67035 l -5.0499 -0.15611 l -3.20688 -1.67035 l -1.36386 -0.15611 l 0.47913 -1.67035 l 2.32214 -0.15611 l 4.16515 -1.67035 l 6.00816 -0.15611 l 7.85118 -1.67035 l 8.77269 -0.91324 l 8.77269 -0.91324 l 11.06526 -0.91324 l S q -1.0 0.0 0.0 -1.0 -11.84912 -0.91324 cm {TO} Q q 1.0 0.0 0.0 1.0 11.26451 -0.91324 cm {TO} Q
[column sep=large] rightsquigarrow
    0.39848 w -18.02608 -0.91324 m -15.63513 -0.91324 l -14.71364 -0.15611 l -12.87062 -1.67035 l -11.02762 -0.15611 l -9.1846 -1.67035 l -7.34158 -0.15611 l -5.49858 -1.67035 l -3.65556 -0.15611 l -1.81256 -1.67035 l 0.03044 -0.15611 l 1.87346 -1.67035 l 3.71646 -0.15611 l 5.55948 -1.67035 l 7.4025 -0.15611 l 9.2455 -1.67035 l 11.08852 -0.15611 l 12.93152 -1.67035 l 13.85304 -0.91324 l 13.85304 -0.91324 l 17.043 -0.91324 l S q 1.0 0.0 0.0 1.0 17.24225 -0.91324 cm {TO} Q
[column sep=huge] squiggly
    0.39848 w -24.00374 -0.91324 m -21.6128 -0.91324 l -20.6913 -0.15611 l -18.84828 -1.67035 l -17.00528 -0.15611 l -15.16226 -1.67035 l -13.31924 -0.15611 l -11.47624 -1.67035 l -9.63322 -0.15611 l -7.79022 -1.67035 l -5.9472 -0.15611 l -4.10419 -1.67035 l -2.26118 -0.15611 l -0.41817 -1.67035 l 1.42484 -0.15611 l 3.26784 -1.67035 l 5.11086 -0.15611 l 6.95386 -1.67035 l 8.79688 -0.15611 l 10.6399 -1.67035 l 12.4829 -0.15611 l 14.32591 -1.67035 l 16.16893 -0.15611 l 18.01193 -1.67035 l 19.85495 -0.15611 l 20.77646 -0.91324 l 20.77646 -0.91324 l 23.02065 -0.91324 l S q 1.0 0.0 0.0 1.0 23.2199 -0.91324 cm {TO} Q
rd,rightsquigarrow
    0.39848 w -12.31192 8.76726 m -10.44116 7.27863 l -9.24876 7.29727 l -8.7495 4.96501 l -6.36469 5.00229 l -5.86539 2.67001 l -3.48058 2.70729 l -2.9813 0.37503 l -0.59648 0.41231 l -0.09721 -1.91995 l 2.28758 -1.88266 l 2.78687 -4.21494 l 5.17168 -4.17766 l 5.67097 -6.50993 l 8.05579 -6.47264 l 8.30542 -7.63881 l 8.30542 -7.63881 l 11.15192 -9.90404 l S q 0.78241 -0.62265 0.62265 0.78241 11.3078 -10.02809 cm {TO} Q
dashrightarrow
    0.39848 w [ 2.78941 1.59395 ] 0.0 d -12.04836 -0.91324 m 11.06526 -0.91324 l S q 1.0 0.0 0.0 1.0 11.26451 -0.91324 cm {TO} Q
dashleftarrow
    0.39848 w [ 2.78941 1.59395 ] 0.0 d -11.64987 -0.91324 m 11.46375 -0.91324 l S q -1.0 0.0 0.0 -1.0 -11.84912 -0.91324 cm {TO} Q
dashed
    0.39848 w [ 2.78941 1.59395 ] 0.0 d -12.04836 -0.91324 m 11.06526 -0.91324 l S q 1.0 0.0 0.0 1.0 11.26451 -0.91324 cm {TO} Q
dotted
    0.39848 w [ 0.39848 1.99255 ] 0.0 d -12.04836 -0.91324 m 11.06526 -0.91324 l S q 1.0 0.0 0.0 1.0 11.26451 -0.91324 cm {TO} Q
dash,dashed
    0.39848 w [ 2.78941 1.59395 ] 0.0 d -12.04836 -0.91324 m 11.46375 -0.91324 l S
thick
    0.39848 w 0.79701 w -12.04836 -0.91324 m 10.66673 -0.91324 l S q 1.0 0.0 0.0 1.0 11.06523 -0.91324 cm q [ ] 0.0 d 1 J 1 j -4.1445 4.7821 m -3.38733 1.91281 -1.70001 0.5579 0.0 0.0 c -1.70001 -0.5579 -3.38733 -1.91281 -4.1445 -4.7821 c S Q Q
thick,dashed
    0.39848 w 0.79701 w [ 5.57916 3.1881 ] 0.0 d -12.04836 -0.91324 m 10.66673 -0.91324 l S q 1.0 0.0 0.0 1.0 11.06523 -0.91324 cm q [ ] 0.0 d 1 J 1 j -4.1445 4.7821 m -3.38733 1.91281 -1.70001 0.5579 0.0 0.0 c -1.70001 -0.5579 -3.38733 -1.91281 -4.1445 -4.7821 c S Q Q
line width=1pt
    0.39848 w 0.99628 w -12.04836 -0.91324 m 10.46747 -0.91324 l S q 1.0 0.0 0.0 1.0 10.9656 -0.91324 cm q [ ] 0.0 d 1 J 1 j -5.18063 5.97765 m -4.23416 2.391 -2.12502 0.69739 0.0 0.0 c -2.12502 -0.69739 -4.23416 -2.391 -5.18063 -5.97765 c S Q Q
ultra thin
    0.39848 w 0.09962 w -12.04836 -0.91324 m 11.3641 -0.91324 l S q 1.0 0.0 0.0 1.0 11.41393 -0.91324 cm q [ ] 0.0 d 1 J 1 j -0.51808 0.59778 m -0.42342 0.23909 -0.2125 0.06972 0.0 0.0 c -0.2125 -0.06972 -0.42342 -0.23909 -0.51808 -0.59778 c S Q Q
Rightarrow,dashed
    0.39848 w [ 2.78941 1.59395 ] 0.0 d q 2.32874 w -12.04836 -0.91324 m 9.27638 -0.91324 l S q 1.53175 w 1 G -12.04836 -0.91324 m 9.27638 -0.91324 l S Q q 1.0 0.0 0.0 1.0 9.27638 -0.91324 cm {IMP} Q Q
hook,dashed
    0.39848 w [ 2.78941 1.59395 ] 0.0 d -10.6138 -0.91324 m 11.06526 -0.91324 l S q -1.0 0.0 0.0 1.0 -10.6138 -0.91324 cm {HOOK} Q q 1.0 0.0 0.0 1.0 11.26451 -0.91324 cm {TO} Q
[column sep=large] Rightarrow,squiggly
    0.39848 w q 2.32874 w -18.02608 -0.91324 m -15.63513 -0.91324 l -14.71364 -0.15611 l -12.87062 -1.67035 l -11.02762 -0.15611 l -9.1846 -1.67035 l -7.34158 -0.15611 l -5.49858 -1.67035 l -3.65556 -0.15611 l -1.81256 -1.67035 l 0.03044 -0.15611 l 1.87346 -1.67035 l 3.71646 -0.15611 l 5.55948 -1.67035 l 7.4025 -0.15611 l 9.2455 -1.67035 l 11.08852 -0.15611 l 12.93152 -1.67035 l 13.85304 -0.91324 l 13.85304 -0.91324 l 15.25412 -0.91324 l S q 1.53175 w 1 G -18.02608 -0.91324 m -15.63513 -0.91324 l -14.71364 -0.15611 l -12.87062 -1.67035 l -11.02762 -0.15611 l -9.1846 -1.67035 l -7.34158 -0.15611 l -5.49858 -1.67035 l -3.65556 -0.15611 l -1.81256 -1.67035 l 0.03044 -0.15611 l 1.87346 -1.67035 l 3.71646 -0.15611 l 5.55948 -1.67035 l 7.4025 -0.15611 l 9.2455 -1.67035 l 11.08852 -0.15611 l 12.93152 -1.67035 l 13.85304 -0.91324 l 13.85304 -0.91324 l 15.25412 -0.91324 l S Q q 1.0 0.0 0.0 1.0 15.25412 -0.91324 cm {IMP} Q Q
to head
    0.39848 w -12.04836 -0.91324 m 11.06526 -0.91324 l S q 1.0 0.0 0.0 1.0 11.26451 -0.91324 cm {TO} Q
two heads
    0.39848 w -12.04836 -0.91324 m 11.06526 -0.91324 l S q 1.0 0.0 0.0 1.0 9.82996 -0.91324 cm {TO} Q q 1.0 0.0 0.0 1.0 11.26451 -0.91324 cm {TO} Q
tail
    0.39848 w -9.97621 -0.91324 m 11.06526 -0.91324 l S q -1.0 0.0 0.0 -1.0 -9.77698 -0.91324 cm {REV} Q q 1.0 0.0 0.0 1.0 11.26451 -0.91324 cm {TO} Q
hook
    0.39848 w -10.6138 -0.91324 m 11.06526 -0.91324 l S q -1.0 0.0 0.0 1.0 -10.6138 -0.91324 cm {HOOK} Q q 1.0 0.0 0.0 1.0 11.26451 -0.91324 cm {TO} Q
hook'
    0.39848 w -10.6138 -0.91324 m 11.06526 -0.91324 l S q -1.0 0.0 0.0 -1.0 -10.6138 -0.91324 cm {HOOK} Q q 1.0 0.0 0.0 1.0 11.26451 -0.91324 cm {TO} Q
harpoon
    0.39848 w -12.04836 -0.91324 m 11.06526 -0.91324 l S q 1.0 0.0 0.0 1.0 11.26451 -0.91324 cm {HARP} Q
harpoon'
    0.39848 w -12.04836 -0.91324 m 11.06526 -0.91324 l S q 1.0 0.0 0.0 -1.0 11.26451 -0.91324 cm {HARP} Q
maps to
    0.39848 w -11.7495 -0.91324 m 11.06526 -0.91324 l S q -1.0 0.0 0.0 -1.0 -11.84912 -0.91324 cm {BAR} Q q 1.0 0.0 0.0 1.0 11.26451 -0.91324 cm {TO} Q
no head
    0.39848 w -12.04836 -0.91324 m 11.46375 -0.91324 l S
leftarrow,no tail
    0.39848 w -12.04836 -0.91324 m 11.46375 -0.91324 l S
tail,two heads
    0.39848 w -9.97621 -0.91324 m 11.06526 -0.91324 l S q -1.0 0.0 0.0 -1.0 -9.77698 -0.91324 cm {REV} Q q 1.0 0.0 0.0 1.0 9.82996 -0.91324 cm {TO} Q q 1.0 0.0 0.0 1.0 11.26451 -0.91324 cm {TO} Q
hook,two heads
    0.39848 w -10.6138 -0.91324 m 11.06526 -0.91324 l S q -1.0 0.0 0.0 1.0 -10.6138 -0.91324 cm {HOOK} Q q 1.0 0.0 0.0 1.0 9.82996 -0.91324 cm {TO} Q q 1.0 0.0 0.0 1.0 11.26451 -0.91324 cm {TO} Q
maps to,two heads
    0.39848 w -11.7495 -0.91324 m 11.06526 -0.91324 l S q -1.0 0.0 0.0 -1.0 -11.84912 -0.91324 cm {BAR} Q q 1.0 0.0 0.0 1.0 9.82996 -0.91324 cm {TO} Q q 1.0 0.0 0.0 1.0 11.26451 -0.91324 cm {TO} Q
dash,tail
    0.39848 w -9.97621 -0.91324 m 11.46375 -0.91324 l S q -1.0 0.0 0.0 -1.0 -9.77698 -0.91324 cm {REV} Q
dash,hook
    0.39848 w -10.6138 -0.91324 m 11.46375 -0.91324 l S q -1.0 0.0 0.0 1.0 -10.6138 -0.91324 cm {HOOK} Q
```

---

## 16. Reference tests

`tests/cd/*.tex` are standalone (`\pdfcompresslevel=0`). Run:

```
pdflatex -interaction=nonstopmode -output-directory=<scratch> tests/cd/NN.tex
qpdf --qdf --object-streams=disable <scratch>/NN.pdf out.pdf
```

Only `23-unknown-keys.tex` has errors, on purpose (11). All others compile with 0 errors. [V]

Per diagram, against the library, with real PGF's boxes: `python3 -I tests/corpus/cd/compare_library.py file --tool <libcheck> tests/cd/NN.tex [--diff N]`.

| File | Covers |
|---|---|
| `01-square` | Canonical square; worked example |
| `02-seps` | Named sizes, dimensions, `sep`, `between origins`, `&[..]`, `\\[..]`, uneven rows |
| `03-heads-plain` | rightarrow, Rightarrow, mapsto, Mapsto, equal, dash and mirrored forms |
| `04-heads-hooks` | hook, two head, tail, harpoon families |
| `05-heads-squiggly-dashed` | squiggly, dashed, dotted, thick, line width, combinations with double |
| `06-end-modifiers` | One-end keys and combinations, diagonal hooks |
| `07-labels` | Positions, swap, description, marking, sloped, inner sep, font, color, multiple labels |
| `08-auto-anchors` | All eight directions, both sides, near-axis slopes |
| `09-phantom` | phantom, `\square`, corners, `draw=none` |
| `10-shift` | shift left/right, xshift, yshift, shift, transform canvas |
| `11-bend-loop` | bend, in/out, looseness, loop variants, distance |
| `12-from-to-anchor` | from/to, alias, anchors, anchor transforms, 2-cell from named labels |
| `14-shorten-crossing-over` | shorten family; crossing over, cube, background color, clearance |
| `15-colors-tikzkeys` | Colors, draw, opacity, `-Stealth`, `-latex`, `->`, `\|->`, `double`, ambient color; key order of dash and thick, env `line width`, sloped with xshift, `allow upside down`, `in looseness`, `min distance`, `max distance` |
| `16-env-options` | cramped, math mode, ampersand replacement, every X, cells, font, scale, `tikzcdset` |
| `17-baseline` | Baseline variants in text, display, center, box dimensions |
| `18-font-size` | Size and family dependence |
| `19-old-syntax` | `\arrow{r}{f}`, `\rar` etc., positional labels |
| `20-arrow-styles` | Latin Modern, tikz, math font |
| 21 (private, gitignored) | Verbatim from thesis lines 159, 923, 1181, 1768, 1861 with its preamble |
| 22 (private, gitignored) | Verbatim from LMB lines 332, 483, 1142, 1965 with its preamble |
| `23-unknown-keys` | `Rrightarrow`, `tail reversed`, out-of-range targets, arrows to empty cells, cd key in a `\tikzset` style, bare `&` in `align` |
| `25-label-keys` | above/below/left/right, anchor, label shifts, rotate, `"a=b"`, braces |
| `26-cells` | Empty cells, `\|[..]\|`, text height, nested matrix, self-arrow, short squiggly, tall cells |
| `27-macros-styles-context` | User styles, macros hiding arrows, align, figure, theorem, inline, tabular, comments |
| `28-user-keys` | `\tikzset` against `\tikzcdset`, paths on keys, styles with a value, `.default`, `.initial`, key names over lines, keys without effect, `overlay`, a label's `scale`, colour operators |
| `29-gt-tips` | `>=stealth`, `>=latex`, `/tikz` arrows and `arrow style=tikz` under them |
| `30-quiver` | quiver.sty: `curve`, with shifts, anchors, later curve keys; `tail reversed`, `2tail reversed`, `no body` |
| `31-loop-squiggly-widths` | A later curve key on a loop, which width each squiggly length takes, the width among a tip's settings, a separation name redefined |
| `32-loop-shift-anchor` | A loop that is shifted or has an anchor: the end is the start on each pass over its `to path` |
| `33-bend-again-short-squiggly` | A bend with no angle after one with an angle; a squiggly curve on which real PGF comes to `Dimension too large` |
| `35-text-color-order` | An arrow's `text=` against a later `color=` or `<color>`: the last one colours its labels |
| `36-double-order` | `double`, `double distance` and `double=<c>` before and after `double line`, `Rightarrow`, `equal` and `Mapsto` |
| `37-phantom-labels-order` | `phantom` against the diagram's `labels=`, `\tikzcdset`, `arrows=` and the arrow's own `labels=`, before and after it |
| `38-cells-nodes-order` | `cells={nodes={...}}` against `every cell`'s own keys after it, `cramped` and `\tikzcdset`; `nodes=` where it reaches no node |
| `39-diagram-font-cells-font` | The diagram's `font=` before and after a font of `every cell`, by `cells=`, `nodes=`, `\tikzcdset` and `every cell/.append style`, and a cell's own |
| `40-svg-colour-order` | A colour key after `fill=`, `description` or `draw=` on a label, a cell and an arrow, and from `labels=` and `cells=`: what the SVG paints |

---

## 17. Behaviour not determined, or limits of this spec

1. **`arrow style=math font`**: read from code only, tested for compilation (needs amssymb for
   `\twoheadrightarrow`). The geometry of `Glyph` is not model-checked. [C]
2. **Exact TeX arithmetic**: the spec is in reals, and so is `tests/cd/model/model.py`, which gets to 0.031 bp. pgf's rounding (pgfmath sin/cos,
   `\pgf@sys@tonumber`, the ±1 in `dist`) is what the rest was. `crates/commute/src/pgfmath.rs` has it, which is how the library reaches the
   box to the scaled point and pgf's operators token for token (section 1).
3. **Squiggly arrows shorter than `8lw` (3.2 pt)**: only approximately modelled; pgf emits collinear points in
   odd order. The drawing is a short stub with a tip either way.
4. **Very thin squiggly lines**: pgf treats `remaining < 1pt` specially when `post length` is under 1 pt
   (`lw < 0.167 pt`). Not tested. [C]
5. **`transform shape` with `scale`** moved and scaled only labels in the one test, and was not modelled. `scale` on the diagram: see 5.3.
6. **Not tested**: `relative`, `controls`, `in control`, `out control`, `transform nodes` directly,
   `execute before arrows`, `every cell` shape changes, `align`/`text width` in cells or labels, `outer sep` in cells.
7. **Generic TikZ inside the environment** (`\draw` or `\node` in `execute before arrows`, custom shapes, `pin`,
   `label=`, custom tips `tikzcd to/.tip=...`, `remember picture`) is outside this spec. Recommended [I]:
   detect any key or command outside sections 3 and 5 and fall back to the real package.
   How a fallback could be loaded after `\begin{document}` is an open design question for the engine.
8. **`opacity`** was seen in raw output only (`/pgf@CA0.5 gs /pgf@ca0.5 gs`). Its resource dictionary was not inspected.
9. **`every node/.style` and `column N/.style` with an explicit `/tikz/` prefix** would take effect [C], but were
    not tested. The unprefixed forms were verified to do nothing.
