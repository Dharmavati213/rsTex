//! The workspace, matrix layout (spec 7), arrows and labels (spec 8 to 11), and the picture box (spec 12).  The arithmetic is
//! PGF's, in scaled points; see pgfmath.rs.

use crate::FontParams;
use crate::color::Color;
use crate::geom::{Matrix, Pt, Seg, pt, sp};
use crate::node::{EndRef, Node, Shape};
use crate::parse::{AGAIN, Dir, MAX_PASSES, ParseOut, Parser, R, sanitize};
use crate::path::{quiver_path, ArrowPath, Zigzag, base_path, line_at_distance, zigzag};
use crate::pgfmath::{PT, Sp, Tm, Xy, curve_at_time, half, mul, normalised};
use crate::policy::Policy;
use crate::scan::Span;
use crate::spec::{
    Anchor, ArrowSpec, BaseNode, Baseline, CellSpec, Dash, End, LabelSpec, LabelXform, NextCell, Opt, OptRange, Shift, Unsupported, NoUserKeys, UserKeys};
use crate::tips::{TipForm, TipSeq, tip_seq};

/// More path segments than this in one diagram is not a diagram anyone drew; it can only come from absurd dimensions on
/// squiggly arrows.
const MAX_SEGS: usize = 1 << 20;

/// The parser reads `background color` for each arrow that draws in it (`Parser::background`), so this is only a guard.
const UNRESOLVED: Unsupported = Unsupported::new("background colour not read", Span::EMPTY);

/// A measured box, in points: a whole number of scaled points, as TeX has it.
#[derive(Copy, Clone, Debug, PartialEq, Default)]
pub struct BoxDim {
    pub w: f64,
    pub h: f64,
    pub d: f64,
}

/// A run of [`DrawList::segs`].
#[derive(Copy, Clone, Debug, PartialEq, Default)]
pub struct PathRef {
    pub start: u32,
    pub end: u32,
}

/// What a stroke is, in the order of spec 10.1.
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum Role {
    /// The `crossing over` preaction: background colour, no tips.
    Halo,
    /// The line itself, at its outer width.
    Line,
    /// The second stroke of a double line.
    Inner,
}

/// One drawing step.  Coordinates are picture coordinates unless `m` says otherwise.
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum Item {
    /// The rectangle of a cell with `draw`, before its text.  `rounded` is set for `rounded corners`.  In the matrix's frame
    /// as all of a cell: `(ox, oy)` is the origin of the cell's picture.  `color` is what the keys of `every cell` and then the
    /// cell's own leave the stroke.
    CellBorder { x: f64, y: f64, w: f64, h: f64, rounded: Option<PathRef>, color: Option<Color>, ox: f64, oy: f64 },
    /// A cell's box: its reference point goes to `(x, y)`.  `(ox, oy)` is the cell's origin in the matrix, where pgf's
    /// scopes for the cell are opened.  `base` and `opts`, in [`DrawList::opts`], are what the keys of `every cell` and of its own do to the graphics state.
    CellBox { cell: u32, x: f64, y: f64, color: Option<Color>, ox: f64, oy: f64, base: PathRef, opts: PathRef },
    /// A truly empty cell at that origin: pgf opens and closes a scope there, with `every cell`'s keys in it, and draws nothing.
    CellEmpty { ox: f64, oy: f64, base: PathRef },
    /// `n` coordinate nodes that hold a shifted arrow's ends: each is a scope with nothing in it.
    Coordinates { n: u32 },
    /// The scope of one arrow, its labels included.  `canvas` is `transform canvas`; `opts` holds what its keys do, in
    /// their order, in [`DrawList::opts`].  `draw_color` is the colour those leave the stroke.
    ArrowBegin { canvas: Pt, lw: f64, color: Option<Color>, draw_color: Option<Color>, opacity: Option<f64>, dash: Dash, opts: PathRef },
    /// A stroke of the arrow's line.  `color` overrides the scope's.
    Stroke { role: Role, path: PathRef, width: f64, color: Option<Color>, canvas: Pt },
    /// An arrow tip.  Its segments are in the tip's frame; `m` takes them to picture coordinates.  `own_width` says that the
    /// tip's body sets its `width`.
    Tip { m: Matrix, path: PathRef, width: f64, own_width: bool, cap: u8, join: u8, form: TipForm },
    /// A label's background, a rectangle in the label's frame.  Under a rotation `path` holds it, as pgf writes it.  `color`
    /// is what the label's keys leave the fill.
    LabelFill { m: Matrix, x: f64, y: f64, w: f64, h: f64, color: Color, path: PathRef },
    /// A label's box: `m` takes its reference point and axes to picture coordinates.  `opts` as for a cell.
    LabelBox { label: u32, m: Matrix, color: Option<Color>, opts: PathRef },
    ArrowEnd,
}

/// The colour a path is stroked with after `opts`, in their order, as the PDF writes them: the last of `draw=<c>`
/// (`\pgfsetstrokecolor`), `color=` (`\pgfsetcolor`) and a colour by its name (`\pgfutil@color`), which set it in TikZ's option
/// order; `None` leaves the colour around them.
pub(crate) fn stroke_color(opts: &[Opt]) -> Option<Color> {
    opts.iter().rev().find_map(|o| match *o {
        Opt::Stroke(c) | Opt::SetColor(c) | Opt::Color(c) => Some(c),
        _ => None,
    })
}

/// The colour a path is filled with after `opts`: the last of `fill=<c>` (`\pgfsetfillcolor`), `color=` and a colour by its name.
pub(crate) fn fill_color(opts: &[Opt]) -> Option<Color> {
    opts.iter().rev().find_map(|o| match *o {
        Opt::Fill(c) | Opt::SetColor(c) | Opt::Color(c) => Some(c),
        _ => None,
    })
}

/// The drawing of a diagram.  The vectors keep their capacity between diagrams.
#[derive(Clone, Debug, Default)]
pub struct DrawList {
    pub items: Vec<Item>,
    pub segs: Vec<Seg>,
    pub opts: Vec<Opt>,
}

impl DrawList {
    /// The segments of a path.
    #[inline]
    pub fn path(&self, p: PathRef) -> &[Seg] {
        self.segs.get(p.start as usize..p.end as usize).unwrap_or(&[])
    }

    /// The options of an arrow.
    #[inline]
    pub fn options(&self, p: PathRef) -> &[Opt] {
        self.opts.get(p.start as usize..p.end as usize).unwrap_or(&[])
    }
}

/// The picture box (spec 12) and the matrix.  Every length is a whole number of scaled points; `width`, `height` and `depth`
/// are what real PGF's box has.
#[derive(Copy, Clone, Debug, PartialEq, Default)]
pub struct Layout {
    pub minx: f64,
    pub miny: f64,
    pub maxx: f64,
    pub maxy: f64,
    /// Baseline, in picture coordinates.
    pub base: f64,
    /// Box width: `maxx - minx`.
    pub width: f64,
    /// Box height: `maxy - base`.
    pub height: f64,
    /// Box depth: `base - miny`.
    pub depth: f64,
    /// The matrix's `Wm`, `Hm`, `Dm`.
    pub matrix_w: f64,
    pub matrix_h: f64,
    pub matrix_d: f64,
    /// Where the matrix box's reference point is, in picture coordinates.
    pub matrix_x: f64,
    pub matrix_y: f64,
}

/// A workspace for one diagram at a time; see the crate documentation.
pub struct Diagram {
    buf: Vec<u8>,
    p: ParseOut,
    font: FontParams,
    parsed: bool,
    warns: bool,
    // Layout state.
    nodes: Vec<Option<Node>>,
    named: Vec<(Span, Node)>,
    minx: Vec<Sp>,
    maxx: Vec<Sp>,
    xs: Vec<Sp>,
    rowmax: Vec<Sp>,
    rowmin: Vec<Sp>,
    ys: Vec<Sp>,
    pts: Vec<Xy>,
    /// `\pgf@picminx`, `miny`, `maxx`, `maxy`.
    bb: [Sp; 4],
    /// `\pgf@pathminx` and the others.
    path_bb: [Sp; 4],
    /// `\ifpgf@relevantforpicturesize`.
    counting: bool,
    list: DrawList,
    layout: Layout,
}

impl Default for Diagram {
    fn default() -> Self {
        Diagram::new()
    }
}

const FAR: Sp = 16000 * PT;

impl Diagram {
    pub fn new() -> Diagram {
        Diagram {
            buf: Vec::new(),
            p: ParseOut::new(),
            font: FontParams::ARTICLE_10PT,
            parsed: false,
            warns: false,
            nodes: Vec::new(),
            named: Vec::new(),
            minx: Vec::new(),
            maxx: Vec::new(),
            xs: Vec::new(),
            rowmax: Vec::new(),
            rowmin: Vec::new(),
            ys: Vec::new(),
            pts: Vec::new(),
            bb: [0; 4],
            path_bb: [0; 4],
            counting: true,
            list: DrawList::default(),
            layout: Layout::default(),
        }
    }

    /// Parse a diagram.
    ///
    /// `env` is the text after `\begin{tikzcd}`, options included, up to `\end{tikzcd}`.  `preset` is a key list that ran
    /// before the picture, as `\tikzcdset{...}` does: it may hold `arrows=`, `every label/.append style=`, `row sep/normal=`,
    /// `arrow style=` and style definitions.  Comments are understood; the engine's token lists have none.
    pub fn parse(&mut self, preset: &[u8], env: &[u8], font: FontParams, policy: &dyn Policy) -> Result<(), Unsupported> {
        self.parse_with(preset, env, font, policy, &NoUserKeys)
    }

    /// [`Diagram::parse`], with a way to the document's keys.
    pub fn parse_with(&mut self, preset: &[u8], env: &[u8], font: FontParams, policy: &dyn Policy, keys: &dyn UserKeys) -> Result<(), Unsupported> {
        self.parsed = false;
        self.font = font;
        self.p.forget();
        let preset_len = sanitize(preset, env, &mut self.buf)?;
        let env_end = self.buf.len();
        let mut passes = 0;
        // Bytes read by all passes: long names and many styles must not cost more than real PGF would.
        const MAX_READ: usize = 1 << 24;
        let mut read = 0;
        loop {
            self.p.clear(font);
            let mut parser =
                Parser { buf: &self.buf, o: &mut self.p, font, policy, keys, style_uses: 0, env_end, dir: Dir::Cd, in_quiver: false, in_tikz: false, in_every_cell: false, cell_written: 0 };
            match parser.run(preset_len) {
                Err(u) if u.reason == AGAIN => {
                    passes += 1;
                    // A style can double its value in every pass.
                    read += self.buf.len();
                    if passes > MAX_PASSES || self.buf.len() > 2 * env_end + (1 << 20) || read > MAX_READ {
                        return Err(Unsupported::new("user styles expand too far", u.at));
                    }
                    self.p.take_pending(&mut self.buf);
                }
                r => {
                    r?;
                    break;
                }
            }
        }
        self.parsed = true;
        Ok(())
    }

    /// The source text of a span, for reporting and for typesetting.
    #[inline]
    pub fn text(&self, s: Span) -> &[u8] {
        s.get(&self.buf)
    }

    /// The cells in row order, empty ones included.  [`Diagram::layout`] takes one [`BoxDim`] per entry.
    #[inline]
    pub fn cells(&self) -> &[CellSpec] {
        &self.p.cells
    }

    /// The text of a cell: its source with the arrow commands taken out, one run per piece.
    pub fn cell_content(&self, cell: usize) -> impl Iterator<Item = &[u8]> + '_ {
        let (a, b) = self.p.cells.get(cell).map_or((0, 0), |c| c.content);
        self.p.content[a as usize..b as usize].iter().map(|s| s.get(&self.buf))
    }

    /// The same runs as [`Diagram::cell_content`], as spans: the engine maps them back to its tokens.
    pub fn cell_runs(&self, cell: usize) -> &[Span] {
        let (a, b) = self.p.cells.get(cell).map_or((0, 0), |c| c.content);
        &self.p.content[a as usize..b as usize]
    }

    /// The labels in the order they are drawn.  [`Diagram::layout`] takes one [`BoxDim`] per entry.
    #[inline]
    pub fn labels(&self) -> &[LabelSpec] {
        &self.p.labels
    }

    /// False under `math mode=false`: cells and labels are typeset without `$...$`.
    #[inline]
    pub fn math_mode(&self) -> bool {
        self.p.s.math_mode
    }

    /// `ampersand replacement=` is in force: `&` keeps its category in the cells.
    #[inline]
    pub fn has_ampersand_replacement(&self) -> bool {
        !self.p.s.amp.is_empty()
    }

    /// What the last `\pgfmatrixnextcell` was given: an `&` between cells, the replacement, or an `&` of an alignment in a
    /// cell when `&` is active.
    #[inline]
    pub fn last_next_cell(&self) -> NextCell {
        self.p.next_cell
    }

    /// The `[...]` of every `\pgfmatrixnextcell` that has one, which real PGF `\xdef`s whether its column is new or not.
    #[inline]
    pub fn next_cell_options(&self) -> &[Span] {
        &self.p.next_cell_opts
    }

    /// The number of rows; an extra one if the body ends with `\\` (spec 3.1).
    #[inline]
    pub fn rows(&self) -> usize {
        self.p.rows.len()
    }

    /// The number of columns.
    #[inline]
    pub fn cols(&self) -> usize {
        self.p.cols.len()
    }

    /// The number of arrows.
    #[inline]
    pub fn arrows(&self) -> usize {
        self.p.arrows.len()
    }

    /// Whether an arrow is a `loop`, whose `to path` runs `every loop`.
    pub fn has_loop(&self) -> bool {
        self.p.arrows.iter().any(|a| a.is_loop)
    }

    /// Whether the last [`Diagram::layout`] has an arrow from a node to itself, of which real PGF has something to say in the log.
    #[inline]
    pub fn has_arrow_to_itself(&self) -> bool {
        self.warns
    }

    /// Whether an arrow is `squiggly`, for which tikz-cd asks for a library.
    pub fn has_squiggly(&self) -> bool {
        self.p.arrows.iter().any(|a| a.squiggly)
    }

    /// The drawing, after [`Diagram::layout`].
    #[inline]
    pub fn draw_list(&self) -> &DrawList {
        &self.list
    }

    /// The picture box, after [`Diagram::layout`].
    #[inline]
    pub fn result(&self) -> &Layout {
        &self.layout
    }

    /// The width of the picture's default line, `0.4pt` unless the options say `line width=`.
    #[inline]
    pub fn cell_line_width(&self) -> f64 {
        pt(self.p.s.cell_lw)
    }

    /// The font parameters the diagram was parsed with.
    #[inline]
    pub fn font(&self) -> FontParams {
        self.font
    }

    /// `opacity=` values in use, for the page's ExtGState resources: `/pgf@CA<x>` and `/pgf@ca<x>`.
    pub fn opacities(&self) -> impl Iterator<Item = f64> + '_ {
        self.p.arrows.iter().filter_map(|a| a.opacity)
    }

    /// Compute the drawing from measured boxes.  `cells` and `labels` run parallel to [`Diagram::cells`] and
    /// [`Diagram::labels`]; a cell without a node ignores its entry.
    ///
    /// Errors are things only the measured diagram shows: an arrow to a cell without a node, or to a name that does not
    /// exist yet.
    pub fn layout(&mut self, cells: &[BoxDim], labels: &[BoxDim]) -> Result<&Layout, Unsupported> {
        if !self.parsed {
            return Err(Unsupported::new("layout without a parsed diagram", Span::EMPTY));
        }
        if cells.len() < self.p.cells.len() || labels.len() < self.p.labels.len() {
            return Err(Unsupported::new("fewer measured boxes than cells and labels", Span::EMPTY));
        }
        self.list.items.clear();
        self.list.segs.clear();
        self.list.opts.clear();
        self.nodes.clear();
        self.named.clear();
        self.warns = false;
        self.bb = [FAR, FAR, -FAR, -FAR];
        self.path_bb = [FAR, FAR, -FAR, -FAR];
        crate::pgfmath::take_too_large();

        let (matrix, wm, hm, dm) = self.matrix(cells)?;

        // Arrows are only ever appended to `p.arrows` by the parser, so indexing here stays in range.
        for i in 0..self.p.arrows.len() {
            let a = self.p.arrows[i];
            self.arrow(&a, labels)?;
        }
        if crate::pgfmath::take_too_large() {
            return Err(Unsupported::new("real PGF stops at Dimension too large", Span::EMPTY));
        }

        let [minx, miny, maxx, maxy] = self.bb;
        let base = self.baseline(&matrix)?;
        self.layout = Layout {
            minx: pt(minx),
            miny: pt(miny),
            maxx: pt(maxx),
            maxy: pt(maxy),
            base: pt(base),
            width: pt(maxx - minx),
            height: pt(maxy - base),
            depth: pt(base - miny),
            matrix_w: pt(wm),
            matrix_h: pt(hm),
            matrix_d: pt(dm),
            matrix_x: pt(matrix.m.x),
            matrix_y: pt(matrix.m.y),
        };
        Ok(&self.layout)
    }

    /// `\pgf@protocolsizes`: the point counts for the picture and for the current path.
    #[inline]
    fn protocol(&mut self, p: Xy) {
        self.add_pic(p);
        let b = &mut self.path_bb;
        b[0] = b[0].min(p.0);
        b[1] = b[1].min(p.1);
        b[2] = b[2].max(p.0);
        b[3] = b[3].max(p.1);
    }

    #[inline]
    fn add_pic(&mut self, p: Xy) {
        if !self.counting {
            return;
        }
        let b = &mut self.bb;
        b[0] = b[0].min(p.0);
        b[1] = b[1].min(p.1);
        b[2] = b[2].max(p.0);
        b[3] = b[3].max(p.1);
    }

    /// The end of `\pgfusepath`: a stroke of this width widens the picture by half of it round the path's box, and the
    /// path's box starts anew.
    #[inline]
    fn use_path(&mut self, stroke: Option<Sp>) {
        if let Some(w) = stroke {
            let h = half(w);
            let p = self.path_bb;
            self.add_pic((p[0] - h, p[1] - h));
            self.add_pic((p[2] + h, p[3] + h));
        }
    }

    // --------------------------------------------------------------------------------------------------------- matrix

    /// Spec section 7.  Returns the matrix node and `Wm`, `Hm`, `Dm`.
    fn matrix(&mut self, boxes: &[BoxDim]) -> R<(Node, Sp, Sp, Sp)> {
        let f = self.font;
        let (ax, ex) = (sp(f.ax), sp(f.ex));
        let (nrows, ncols) = (self.p.rows.len(), self.p.cols.len());
        let cell_lw = self.p.s.cell_lw;
        // `.5\pgflinewidth`.
        let (ox, oy) = (half(cell_lw), half(cell_lw));

        self.minx.clear();
        self.minx.resize(ncols, Sp::MAX);
        self.maxx.clear();
        self.maxx.resize(ncols, Sp::MIN);
        self.rowmax.clear();
        self.rowmax.resize(nrows, 0);
        self.rowmin.clear();
        self.rowmin.resize(nrows, 0);

        for (ci, cell) in self.p.cells.iter().enumerate() {
            let (r, c) = (cell.row as usize, cell.col as usize);
            // `\pgf@picminx` and the others: an empty picture is all zero.
            let (mut l, mut rr, mut t, mut b) = (0, 0, 0, 0);
            let mut node = None;
            if cell.has_node {
                let o = &cell.opts;
                let d = boxes[ci];
                let mut n = Node::new(&Shape {
                    w: sp(d.w),
                    h: o.text_height.unwrap_or_else(|| sp(d.h)),
                    d: o.text_depth.unwrap_or_else(|| sp(d.d)),
                    ixs: o.ixs,
                    iys: o.iys,
                    ox,
                    oy,
                    min_w: PT,
                    min_h: PT,
                    asym: true,
                    ax,
                    ex,
                });
                // Anchor `base`.
                let cx = n.real_center().0;
                n.m.shift(-cx, 0);
                l = n.sw.0 + ox - cx;
                b = n.sw.1 + oy;
                rr = n.ne.0 - ox - cx;
                t = n.ne.1 - oy;
                if o.draw {
                    let h = half(cell_lw);
                    l -= h;
                    rr += h;
                    t += h;
                    b -= h;
                }
                node = Some(n);
            }
            self.nodes.push(node);
            // `fixed` of the next column counts only once that column exists: it comes into being in cell order.
            if let Some(next) = self.p.cols.get(c + 1) {
                if next.created_row <= cell.row && next.fixed {
                    rr = 0;
                }
            }
            let this = self.p.cols[c];
            if this.fixed {
                l = 0;
            }
            if r > 0 && self.p.rows[r - 1].fixed {
                t = 0;
            }
            if c > 0 {
                l -= this.sep;
            }
            self.minx[c] = self.minx[c].min(l);
            self.maxx[c] = self.maxx[c].max(rr);
            self.rowmax[r] = self.rowmax[r].max(t);
            self.rowmin[r] = self.rowmin[r].min(b);
        }

        for r in 0..nrows.saturating_sub(1) {
            let row = self.p.rows[r];
            self.rowmin[r] = if row.fixed { -row.sep } else { self.rowmin[r] - row.sep };
        }

        let dm = -self.rowmin[nrows - 1];
        self.ys.clear();
        self.ys.resize(nrows, 0);
        for r in (1..nrows).rev() {
            self.ys[r - 1] = self.ys[r] + self.rowmax[r] - self.rowmin[r - 1];
        }
        self.xs.clear();
        self.xs.resize(ncols, 0);
        self.xs[0] = -self.minx[0];
        for c in 1..ncols {
            self.xs[c] = self.xs[c - 1] + self.maxx[c - 1] - self.minx[c];
        }
        let wm: Sp = (0..ncols).map(|c| self.maxx[c] - self.minx[c]).sum();
        let hm = self.ys[0] + self.rowmax[0];

        let m = self.p.s.mat_inner;
        let mut mat = Node::new(&Shape {
            w: wm,
            h: hm,
            d: dm,
            ixs: m,
            iys: m,
            ox,
            oy,
            min_w: PT,
            min_h: PT,
            asym: false,
            ax,
            ex,
        });
        let mc = mat.anchor_local(Anchor::Center);
        mat.m.shift(-mc.0, -mc.1);
        let (offx, offy) = (mat.m.x, mat.m.y);
        // `\pgfpathrectanglecorners` under `\pgf@normalrect`.
        let corner = (offx + mat.sw.0 + ox, offy + mat.sw.1 + oy);
        self.add_pic(corner);
        self.add_pic((corner.0 + (mat.ne.0 - ox) - (mat.sw.0 + ox), corner.1 + (mat.ne.1 - oy) - (mat.sw.1 + oy)));

        // Place the cells and draw them, in row order, before any arrow.
        for ci in 0..self.p.cells.len() {
            let cell = self.p.cells[ci];
            let (r, c) = (cell.row as usize, cell.col as usize);
            let base = self.copy_opts(&[cell.opts.opts_base]);
            let opts = self.copy_opts(&[cell.opts.opts_nodes, cell.opts.opts_own]);
            let (cx0, cy0) = (self.xs[c] + offx, self.ys[r] + offy);
            let Some(n) = self.nodes[ci].as_mut() else {
                self.list.items.push(Item::CellEmpty { ox: pt(cx0), oy: pt(cy0), base });
                continue;
            };
            // `\pgf@shift@node`, twice.
            n.m.x += cx0;
            n.m.y += cy0;
            let n = *n;
            let (x, y) = (n.m.x, n.m.y);
            let o = &cell.opts;
            if o.draw {
                // pgf strokes it in the node's scope, inside that of the cell, with the colour the keys of both left.
                let color = stroke_color(self.list.options(opts)).or_else(|| stroke_color(self.list.options(base)));
                let x0 = x + n.sw.0 + ox;
                let y0 = y + n.sw.1 + oy;
                let x1 = x + n.ne.0 - ox;
                let y1 = y + n.ne.1 - oy;
                let rounded = if o.rounded > 0 { Some(self.rounded_rect(x0, y0, x1, y1, o.rounded)?) } else { None };
                self.list.items.push(Item::CellBorder {
                    x: pt(x0),
                    y: pt(y0),
                    w: pt(x1 - x0),
                    h: pt(y1 - y0),
                    rounded,
                    color,
                    ox: pt(cx0),
                    oy: pt(cy0),
                });
            }
            self.list.items.push(Item::CellBox { cell: ci as u32, x: pt(x), y: pt(y), color: o.color, ox: pt(cx0), oy: pt(cy0), base, opts });
        }
        Ok((mat, wm, hm, dm))
    }

    /// pgf's path for a rectangle with rounded corners, starting at the top right (seen in test 26).
    fn rounded_rect(&mut self, x0: Sp, y0: Sp, x1: Sp, y1: Sp, r: Sp) -> R<PathRef> {
        if 2 * r > (x1 - x0).min(y1 - y0) {
            // pgf then shortens the corners, which no test has shown.
            return Err(Unsupported::new("rounded corners larger than the cell", Span::EMPTY));
        }
        const K: Sp = crate::pgfmath::dec(b"0.5522847");
        let q = r - mul(K, r);
        let start = self.list.segs.len() as u32;
        let p = |x: Sp, y: Sp| Pt::new(pt(x), pt(y));
        self.list.segs.extend_from_slice(&[
            Seg::Move(p(x1 - r, y1)),
            Seg::Line(p(x0 + r, y1)),
            Seg::Curve(p(x0 + q, y1), p(x0, y1 - q), p(x0, y1 - r)),
            Seg::Line(p(x0, y0 + r)),
            Seg::Curve(p(x0, y0 + q), p(x0 + q, y0), p(x0 + r, y0)),
            Seg::Line(p(x1 - r, y0)),
            Seg::Curve(p(x1 - q, y0), p(x1, y0 + q), p(x1, y0 + r)),
            Seg::Line(p(x1, y1 - r)),
            Seg::Curve(p(x1, y1 - q), p(x1 - q, y1), p(x1 - r, y1)),
            Seg::Close,
            Seg::Move(p(x0, y0)),
        ]);
        Ok(PathRef { start, end: self.list.segs.len() as u32 })
    }

    // ---------------------------------------------------------------------------------------------------------- nodes

    fn cell_node(&self, r: i64, c: i64) -> Option<Node> {
        let row = self.p.rows.get(usize::try_from(r).ok()?)?;
        let col = u32::try_from(c).ok()?;
        if col >= row.ncells {
            return None;
        }
        self.nodes.get((row.first_cell + col) as usize).copied().flatten()
    }

    fn named_node(&self, name: Span) -> Option<Node> {
        let n = name.get(&self.buf);
        if let Some(&(_, node)) = self.named.iter().rev().find(|(s, _)| s.get(&self.buf) == n) {
            return Some(node);
        }
        // The last node of a name is the one the name means.
        let i = self.p.cells.iter().rposition(|c| !c.opts.alias.is_empty() && c.opts.alias.get(&self.buf) == n)?;
        self.nodes.get(i).copied().flatten()
    }

    fn resolve(&self, end: End, a: &ArrowSpec) -> R<Node> {
        let (r, c) = (i64::from(a.r), i64::from(a.c));
        let (n, at) = match end {
            End::Home => (self.cell_node(r, c), Span::EMPTY),
            End::Rel(dr, dc) => (self.cell_node(r.saturating_add(dr), c.saturating_add(dc)), Span::EMPTY),
            End::Abs(r1, c1) => (self.cell_node(i64::from(r1) - 1, i64::from(c1) - 1), Span::EMPTY),
            End::Name(s) => (self.named_node(s), s),
        };
        n.ok_or(Unsupported::new("arrow end is not a node", at))
    }

    // --------------------------------------------------------------------------------------------------------- arrows

    fn protocol_path(&mut self, path: &ArrowPath) {
        self.protocol(path.s);
        if let Some((c1, c2)) = path.c {
            self.protocol(c1);
            self.protocol(c2);
        }
        self.protocol(path.e);
    }

    fn arrow(&mut self, a: &ArrowSpec, label_boxes: &[BoxDim]) -> R {
        if self.list.segs.len() > MAX_SEGS {
            return Err(Unsupported::new("diagram too large", Span::EMPTY));
        }
        self.counting = !a.overlay;
        let st = self.resolve(a.start, a)?;
        let en = if a.is_loop { st } else { self.resolve(a.end, a)? };
        // `loop`'s `\let\tikztotarget=\tikztostart` takes the start's anchor with it, and `end anchor` goes unread.
        let ea = if a.is_loop { a.sa } else { a.ea };
        // pgf writes to the log that there is no point of the border towards the node's own centre: on a straight line or a bend, whose angles are from that line, and with no anchor.
        self.warns |= st.m == en.m && a.sa.is_none() && ea.is_none() && a.curve.as_ref().is_none_or(|c| c.relative);
        let (st, en) = (EndRef::Node(st), EndRef::Node(en));

        let mut path = match a.quiver {
            Some(ref q) => quiver_path(&st, &en, a.sa, ea, q),
            None => base_path(&st, &en, a.sa, ea, a.curve.as_ref()),
        };
        // A `to path` that starts with a move of its own, after a move no node's border replaces.
        let mut extra_move = a.quiver.is_some() && a.sa.is_some();

        if a.shiftable {
            // `@shiftabletopath`: the dry path counts in the bounding box and is not drawn (spec 8.3), and two coordinate
            // nodes are made on it; then two more for each `shift left`.
            self.protocol_path(&path);
            self.path_bb = [FAR, FAR, -FAR, -FAR];
            let mut p = (path.s.0 + a.xlate_s.0, path.s.1 + a.xlate_s.1);
            let mut q = (path.e.0 + a.xlate_e.0, path.e.1 + a.xlate_e.1);
            let shifts = a.shifts.0 as usize..a.shifts.1 as usize;
            for s in &self.p.shifts[shifts.clone()] {
                if let Shift::Translate(t) = *s {
                    p = (p.0 + t.0, p.1 + t.1);
                    q = (q.0 + t.0, q.1 + t.1);
                }
            }
            let mut nodes = 2;
            for s in &self.p.shifts[shifts] {
                if let Shift::Left(r) = *s {
                    let (nx, ny) = normalised(p.0 - q.0, p.1 - q.1);
                    let d = (mul(ny, r), -mul(nx, r));
                    p = (p.0 + d.0, p.1 + d.1);
                    q = (q.0 + d.0, q.1 + d.1);
                    nodes += 2;
                }
            }
            self.list.items.push(Item::Coordinates { n: nodes });
            // `loop`'s `to path` starts with `\let\tikztotarget=\tikztostart`, on this run as on the dry one: from `tikzcd@nodea` back to it, wherever the dry path's end went.
            if a.is_loop {
                q = p;
            }
            path = match a.curve {
                None => match a.quiver {
                    Some(ref c) => {
                        extra_move = true;
                        quiver_path(&EndRef::Coord(p), &EndRef::Coord(q), None, None, c)
                    }
                    None => ArrowPath { s: p, c: None, e: q },
                },
                Some(ref cv) => {
                    extra_move = cv.relative;
                    base_path(&EndRef::Coord(p), &EndRef::Coord(q), None, None, Some(cv))
                }
            };
        }

        self.protocol_path(&path);
        let (s, e) = (path.s, path.e);

        // Widths (spec 10.2): `\tikz@double@setup`, as the last of `double line` and `double distance` made it.
        let lw = a.lw;
        let (mut outer, mut inner) = (lw, 0);
        if a.double {
            let q = mul(crate::pgfmath::dec(b"0.45"), sp(self.font.ex));
            outer = lw + q;
            inner = q - lw;
        }
        if let Some(d) = a.double_tikz {
            outer = 2 * lw + d;
            inner = d;
        }

        // The labels are made while the path is scanned, so they count whatever happens to the arrow's options.
        let relevant = a.canvas == (0, 0);
        let canvas = Pt::of(a.canvas);

        // The decoration runs before shortening.
        let mut decorated = false;
        if a.squiggly {
            let mut pts = std::mem::take(&mut self.pts);
            let pre = a.sq_pre.unwrap_or(6 * lw);
            let post = a.sq_post.unwrap_or(6 * lw);
            let z = zigzag(&path, a.sq_seg, a.sq_amp, pre, post, &mut pts);
            self.pts = pts;
            match z {
                Zigzag::Points => decorated = true,
                Zigzag::TooDense => return Err(Unsupported::new("squiggly arrow with too many segments", Span::EMPTY)),
                Zigzag::Lost => return Err(Unsupported::new("squiggly arrow on a curve too short to follow", Span::EMPTY)),
            }
            for i in 1..self.pts.len() {
                let q = self.pts[i];
                self.protocol(q);
            }
        }

        if a.crossing {
            let width = self.p.s.clearance;
            // The preaction's `/tikz/arrows=-`: only `shorten` applies.
            let (hs, he) = self.shortened(&path, decorated, a.short_s, a.short_e);
            let halo = self.push_line(&path, decorated, hs, he, extra_move);
            let bg = self.p.s.bg.ok_or(UNRESOLVED)?;
            self.list.items.push(Item::Stroke { role: Role::Halo, path: halo, width: pt(width), color: Some(bg), canvas });
            self.use_path(Some(width));
        }

        let opts = self.copy_opts(&[a.opts]);
        let draw_color = stroke_color(self.list.options(opts));
        self.list.items.push(Item::ArrowBegin {
            canvas,
            lw: pt(lw),
            color: a.color,
            draw_color,
            opacity: a.opacity,
            dash: a.dash,
            opts,
        });

        if a.draw {
            let style = self.p.s.style;
            let gt = self.p.s.gt;
            let sta = tip_seq(a.tips[0], style, gt, outer, inner);
            let ena = tip_seq(a.tips[1], style, gt, outer, inner);
            let total = |q: &TipSeq, add: Sp| q.tips().iter().map(|t| t.tip - t.back + t.sep).sum::<Sp>() + add;
            let stop = |q: &TipSeq, add: Sp| q.tips().last().map_or(0, |t| t.tip + t.sep - t.line) + add;

            let (d_e, d_s) = (stop(&ena, a.short_e), stop(&sta, a.short_s));
            // A decorated path ends where the decoration's last line ended.
            let (s, e) = if decorated { (self.pts[0], self.pts[self.pts.len() - 1]) } else { (s, e) };
            let ref_e = self.end_reference(&path, decorated);
            let e2 = line_at_distance(d_e, e, ref_e);
            let ref_s = self.start_reference(&path, decorated, e2);
            let s2 = line_at_distance(d_s, s, ref_s);

            let line = self.push_line(&path, decorated, s2, e2, extra_move);
            self.list.items.push(Item::Stroke { role: Role::Line, path: line, width: pt(outer), color: None, canvas });
            if inner > 0 {
                let bg = match a.double_color {
                    Some(c) => c,
                    None => self.p.s.bg.ok_or(UNRESOLVED)?,
                };
                self.list.items.push(Item::Stroke { role: Role::Inner, path: line, width: pt(inner), color: Some(bg), canvas });
            }
            if relevant {
                self.use_path(Some(outer));
            }
            self.tips(&sta, total(&sta, a.short_s), s, ref_s, relevant);
            self.tips(&ena, total(&ena, a.short_e), e, ref_e, relevant);
        }
        self.path_bb = [FAR, FAR, -FAR, -FAR];

        for li in a.labels.0..a.labels.1 {
            let l = self.p.labels[li as usize];
            self.counting = !a.overlay && !l.overlay;
            self.label(a, &path, &l, li, label_boxes[li as usize]);
        }
        self.counting = true;
        self.list.items.push(Item::ArrowEnd);
        Ok(())
    }

    fn copy_opts(&mut self, runs: &[OptRange]) -> PathRef {
        let start = self.list.opts.len() as u32;
        for &(x, y) in runs {
            self.list.opts.extend_from_slice(&self.p.opts[x as usize..y as usize]);
        }
        PathRef { start, end: self.list.opts.len() as u32 }
    }

    /// `\pgf@prep@straightend` and its degenerate cases: the point the end of the path is measured from.
    fn end_reference(&self, path: &ArrowPath, decorated: bool) -> Xy {
        if decorated {
            return self.pts[self.pts.len() - 2];
        }
        match path.c {
            None => path.s,
            Some((c1, c2)) => {
                if path.e != c2 {
                    c2
                } else if path.e != c1 {
                    c1
                } else {
                    path.s
                }
            }
        }
    }

    /// `\pgf@prep@straightstart`.  `e2` is the end of a straight path, already shortened.
    fn start_reference(&self, path: &ArrowPath, decorated: bool, e2: Xy) -> Xy {
        if decorated {
            return self.pts[1];
        }
        match path.c {
            None => e2,
            Some((c1, c2)) => {
                if path.s != c1 {
                    c1
                } else if path.s != c2 {
                    c2
                } else {
                    e2
                }
            }
        }
    }

    /// The path's ends after a shortening with no tips.
    fn shortened(&self, path: &ArrowPath, decorated: bool, ds: Sp, de: Sp) -> (Xy, Xy) {
        let (s, e) = if decorated { (self.pts[0], self.pts[self.pts.len() - 1]) } else { (path.s, path.e) };
        let ref_e = self.end_reference(path, decorated);
        let e2 = line_at_distance(de, e, ref_e);
        let ref_s = self.start_reference(path, decorated, e2);
        (line_at_distance(ds, s, ref_s), e2)
    }

    fn push_line(&mut self, path: &ArrowPath, decorated: bool, s2: Xy, e2: Xy, extra_move: bool) -> PathRef {
        let start = self.list.segs.len() as u32;
        if decorated {
            let n = self.pts.len();
            for i in 0..n {
                let p = self.pts[i];
                let seg = if i == 0 {
                    Seg::Move(Pt::of(s2))
                } else if i + 1 == n {
                    Seg::Line(Pt::of(e2))
                } else {
                    Seg::Line(Pt::of(p))
                };
                self.list.segs.push(seg);
            }
        } else if let Some((c1, c2)) = path.c {
            if extra_move {
                self.list.segs.push(Seg::Move(Pt::of(path.s)));
            }
            self.list.segs.push(Seg::Move(Pt::of(s2)));
            self.list.segs.push(Seg::Curve(Pt::of(c1), Pt::of(c2), Pt::of(e2)));
        } else {
            self.list.segs.push(Seg::Move(Pt::of(s2)));
            self.list.segs.push(Seg::Line(Pt::of(e2)));
        }
        PathRef { start, end: self.list.segs.len() as u32 }
    }

    /// `\pgftransformarrow`, then `\pgf@arrow@draw@arrow` (spec 9.3).
    fn tips(&mut self, seq: &TipSeq, total: Sp, p: Xy, reference: Xy, relevant: bool) {
        let tips = seq.tips();
        if tips.is_empty() {
            return;
        }
        let mut t = Tm::IDENTITY;
        t.shift(p.0, p.1);
        let (x, y) = normalised(p.0 - reference.0, p.1 - reference.1);
        t.cm(x, y, -y, x, 0, 0);
        t.shift(-total, 0);
        for tip in tips {
            let mut g = t;
            g.shift(-tip.back, 0);
            if tip.swap {
                g.cm(PT, 0, 0, -PT, 0, 0);
            }
            if relevant {
                for &h in tip.hull() {
                    // `\pgf@arrow@hull@point`: the matrix without the identity shortcut, which is the same arithmetic.
                    let q = (mul(g.aa, h.0) + mul(g.ba, h.1) + g.x, mul(g.ab, h.0) + mul(g.bb, h.1) + g.y);
                    self.add_pic(q);
                }
            }
            let start = self.list.segs.len() as u32;
            self.list.segs.extend_from_slice(tip.segs());
            let path = PathRef { start, end: self.list.segs.len() as u32 };
            self.list.items.push(Item::Tip { m: Matrix::of(&g), path, width: pt(tip.lw), own_width: tip.own_width, cap: tip.cap, join: tip.join, form: tip.form });
            t.shift(tip.tip + tip.sep - tip.back, 0);
        }
    }

    // --------------------------------------------------------------------------------------------------------- labels

    /// `\tikz@timer`: `\pgftransformlineattime` or `\pgftransformcurveattime`.  `scope` is the translation of the arrow's scope:
    /// the timer works without it, and a curve's midpoints round differently there.
    fn timer(path: &ArrowPath, time: Sp, sloped: bool, upside: bool, scope: Xy) -> Tm {
        let mut t = Tm::IDENTITY;
        let (s, e) = (path.s, path.e);
        let dir = match path.c {
            None => {
                let (dx, dy) = (e.0 - s.0, e.1 - s.1);
                t.shift(s.0 + mul(time, dx), s.1 + mul(time, dy));
                (dx, dy)
            }
            Some((c1, c2)) => {
                let off = |p: Xy| (p.0 - scope.0, p.1 - scope.1);
                let (q, a, b) = curve_at_time(time, off(s), off(c1), off(c2), off(e));
                t.shift(q.0 + scope.0, q.1 + scope.1);
                (a.0 - b.0, a.1 - b.1)
            }
        };
        if sloped {
            let (mut dx, mut dy) = dir;
            if !upside && dx < 0 {
                dx = -dx;
                dy = -dy;
            }
            let (x, y) = normalised(dx, dy);
            t.cm(x, y, -y, x, 0, 0);
        }
        t
    }

    /// Spec 6.1, 6.2, section 11.
    fn label(&mut self, a: &ArrowSpec, path: &ArrowPath, l: &LabelSpec, index: u32, dim: BoxDim) {
        let f = self.font;
        let scope = self.p.shifts[a.shifts.0 as usize..a.shifts.1 as usize].iter().fold((0, 0), |d, s| match *s {
            Shift::Translate(t) => (d.0 + t.0, d.1 + t.1),
            Shift::Left(_) => d,
        });
        let outer = l.outer.unwrap_or(half(a.lw));
        let mut node = Node::new(&Shape {
            w: sp(dim.w),
            h: sp(dim.h),
            d: sp(dim.d),
            ixs: l.ixs,
            iys: l.iys,
            ox: outer,
            oy: outer,
            min_w: l.min_w,
            min_h: l.min_h,
            asym: l.asym,
            ax: sp(f.ax),
            ex: sp(f.ex),
        });
        let anchor = if l.auto {
            // `\tikz@auto@pre`: the opposite of `sloped`, upside down allowed; then the matrix's first column, normalised.
            let t = Self::timer(path, l.pos, !l.sloped, true, scope);
            let (x, y) = normalised(t.aa, t.ab);
            auto_anchor(x, y, l.swap)
        } else {
            l.anchor
        };
        let mut t = Self::timer(path, l.pos, l.sloped, l.upside, scope);
        // The node's own transforms, in key order, so a shift acts in the frame at that point.
        for x in &self.p.xforms[l.xforms.0 as usize..l.xforms.1 as usize] {
            match *x {
                LabelXform::Shift(d) => t.shift(d.0, d.1),
                LabelXform::Rotate(d) => t.rotate(d),
                LabelXform::Scale(d) => t.scale(d),
            }
        }
        // `\pgfmultipartnode`.
        let ap = node.anchor_local(anchor);
        t.shift(-ap.0, -ap.1);
        node.m = t;
        if !l.name.is_empty() {
            self.named.push((l.name, node));
        }
        if !l.alias.is_empty() {
            self.named.push((l.alias, node));
        }
        // The background rectangle, without the outer sep.
        let (x0, x1) = (node.sw.0 + node.ox, node.ne.0 - node.ox);
        let (y0, y1) = (node.sw.1 + node.oy, node.ne.1 - node.oy);
        let start = self.list.segs.len() as u32;
        if t.is_identity() {
            // `\pgf@normalrect`.
            let (px, py) = t.apply(x0, y0);
            self.add_pic((px, py));
            self.add_pic((px + (x1 - x0), py + (y1 - y0)));
        } else {
            // `\pgf@specialrect`: top right, counterclockwise, closed, and a move to the bottom left.
            let tr = t.apply(x1, y1);
            let tl = t.apply(x0, y1);
            let bl = t.apply(x0, y0);
            let br = t.apply(x1, y0);
            for q in [tr, tl, bl, br] {
                self.add_pic(q);
            }
            if l.fill.is_some() {
                self.list.segs.extend_from_slice(&[
                    Seg::Move(Pt::of(tr)),
                    Seg::Line(Pt::of(tl)),
                    Seg::Line(Pt::of(bl)),
                    Seg::Line(Pt::of(br)),
                    Seg::Close,
                    Seg::Move(Pt::of(bl)),
                ]);
            }
        }
        let m = Matrix::of(&t);
        let opts = self.copy_opts(&[l.opts]);
        if let Some(fill) = l.fill {
            // A colour after `fill=` or `description` fills it too: pgf sets the fill colour in the order of the label's keys.
            let color = fill_color(self.list.options(opts)).unwrap_or(fill);
            let path = PathRef { start, end: self.list.segs.len() as u32 };
            self.list.items.push(Item::LabelFill { m, x: pt(x0), y: pt(y0), w: pt(x1 - x0), h: pt(y1 - y0), color, path });
        }
        self.list.items.push(Item::LabelBox { label: index, m, color: l.color, opts });
    }

    // ------------------------------------------------------------------------------------------------------- baseline

    /// Spec section 12.
    fn baseline(&self, matrix: &Node) -> R<Sp> {
        if self.p.rows.len() == 1 {
            // `1-row diagram` runs after the matrix and so wins over `baseline=`.
            return Ok(matrix.anchor(Anchor::Base).1);
        }
        match self.p.s.baseline {
            Baseline::Dim(d) => Ok(d),
            Baseline::Node { node, anchor, yshift } => {
                let y = match node {
                    BaseNode::BoundingBox => {
                        let [minx, miny, maxx, maxy] = self.bb;
                        let n = Node {
                            ne: (maxx, maxy),
                            sw: (minx, miny),
                            ox: 0,
                            oy: 0,
                            asym: false,
                            ax: 0,
                            ex: sp(self.font.ex),
                            m: Tm::IDENTITY,
                        };
                        n.anchor(anchor).1
                    }
                    BaseNode::Matrix => matrix.anchor(anchor).1,
                    BaseNode::Cell(r, c) => {
                        self.cell_node(i64::from(r) - 1, i64::from(c) - 1)
                            .ok_or(Unsupported::new("baseline names a cell without a node", Span::EMPTY))?
                            .anchor(anchor)
                            .1
                    }
                    BaseNode::Name(s) => {
                        self.named_node(s).ok_or(Unsupported::new("baseline names an unknown node", s))?.anchor(anchor).1
                    }
                };
                Ok(y + yshift)
            }
        }
    }
}

/// `\tikz@auto@anchor` and `\tikz@auto@anchor@prime`.
fn auto_anchor(x: Sp, y: Sp, swap: bool) -> Anchor {
    const E: Sp = crate::pgfmath::dec(b"0.05");
    if !swap {
        if x > E {
            if y > E {
                Anchor::SouthEast
            } else if y < -E {
                Anchor::SouthWest
            } else {
                Anchor::South
            }
        } else if x < -E {
            if y > E {
                Anchor::NorthEast
            } else if y < -E {
                Anchor::NorthWest
            } else {
                Anchor::North
            }
        } else if y > 0 {
            Anchor::East
        } else {
            Anchor::West
        }
    } else if x > E {
        if y > E {
            Anchor::NorthWest
        } else if y < -E {
            Anchor::NorthEast
        } else {
            Anchor::North
        }
    } else if x < -E {
        if y > E {
            Anchor::SouthWest
        } else if y < -E {
            Anchor::SouthEast
        } else {
            Anchor::South
        }
    } else if y > 0 {
        Anchor::West
    } else {
        Anchor::East
    }
}
