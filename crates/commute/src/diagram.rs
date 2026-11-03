//! The workspace, matrix layout (spec 7), arrows and labels (spec 8 to 11), and the picture box (spec 12).

use crate::FontParams;
use crate::color::Color;
use crate::fmath::{norm, rot};
use crate::geom::{Matrix, Pt, Seg, bez};
use crate::node::{EndRef, Node, Shape};
use crate::parse::{ParseOut, Parser, R, sanitize};
use crate::path::{ArrowPath, Zigzag, base_path, zigzag};
use crate::policy::Policy;
use crate::scan::Span;
use crate::spec::{
    Anchor, ArrowSpec, BaseNode, Baseline, CellSpec, Dash, End, LabelSpec, LabelXform, Shift, Unsupported,
};
use crate::tips::{Tip, TipForm, TipSeq, tip_seq};

/// More path segments than this in one diagram is not a diagram anyone drew; it can only come from absurd dimensions on
/// squiggly arrows.
const MAX_SEGS: usize = 1 << 20;

/// A measured box, in points.
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
    /// The rectangle of a cell with `draw`, before its text.  `rounded` is set for `rounded corners`.
    CellBorder { x: f64, y: f64, w: f64, h: f64, rounded: Option<PathRef>, color: Option<Color> },
    /// A cell's box: its reference point goes to `(x, y)`.
    CellBox { cell: u32, x: f64, y: f64, color: Option<Color> },
    /// The scope of one arrow, its labels included.  `canvas` is `transform canvas`.
    ArrowBegin { canvas: Pt, lw: f64, color: Option<Color>, draw_color: Option<Color>, opacity: Option<f64>, dash: Dash },
    /// A stroke of the arrow's line.  `color` overrides the scope's.
    Stroke { role: Role, path: PathRef, width: f64, color: Option<Color>, canvas: Pt },
    /// An arrow tip.  Its segments are in the tip's frame; `m` takes them to picture coordinates.
    Tip { m: Matrix, path: PathRef, width: f64, cap: u8, join: u8, form: TipForm },
    /// A label's background, a rectangle in the label's frame.
    LabelFill { m: Matrix, x: f64, y: f64, w: f64, h: f64, color: Color },
    /// A label's box: `m` takes its reference point and axes to picture coordinates.
    LabelBox { label: u32, m: Matrix, color: Option<Color> },
    ArrowEnd,
}

/// The drawing of a diagram.  Both vectors keep their capacity between diagrams.
#[derive(Clone, Debug, Default)]
pub struct DrawList {
    pub items: Vec<Item>,
    pub segs: Vec<Seg>,
}

impl DrawList {
    /// The segments of a path.
    #[inline]
    pub fn path(&self, p: PathRef) -> &[Seg] {
        self.segs.get(p.start as usize..p.end as usize).unwrap_or(&[])
    }
}

/// The picture box (spec 12) and the matrix.
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
    // Layout state.
    nodes: Vec<Option<Node>>,
    named: Vec<(Span, Node)>,
    minx: Vec<f64>,
    maxx: Vec<f64>,
    xs: Vec<f64>,
    rowmax: Vec<f64>,
    rowmin: Vec<f64>,
    ys: Vec<f64>,
    pts: Vec<Pt>,
    bb: [f64; 4],
    list: DrawList,
    layout: Layout,
}

impl Default for Diagram {
    fn default() -> Self {
        Diagram::new()
    }
}

impl Diagram {
    pub fn new() -> Diagram {
        Diagram {
            buf: Vec::new(),
            p: ParseOut::new(),
            font: FontParams::ARTICLE_10PT,
            parsed: false,
            nodes: Vec::new(),
            named: Vec::new(),
            minx: Vec::new(),
            maxx: Vec::new(),
            xs: Vec::new(),
            rowmax: Vec::new(),
            rowmin: Vec::new(),
            ys: Vec::new(),
            pts: Vec::new(),
            bb: [0.0; 4],
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
        self.parsed = false;
        self.font = font;
        self.p.clear(font);
        let preset_len = sanitize(preset, env, &mut self.buf)?;
        let mut parser = Parser { buf: &self.buf, o: &mut self.p, font, policy, style_uses: 0 };
        parser.run(preset_len)?;
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
        self.p.s.cell_lw
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
        self.nodes.clear();
        self.named.clear();
        self.bb = [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];

        let (matrix, wm, hm, dm) = self.matrix(cells)?;

        // Arrows are only ever appended to `p.arrows` by the parser, so indexing here stays in range.
        for i in 0..self.p.arrows.len() {
            let a = self.p.arrows[i];
            self.arrow(&a, labels)?;
        }

        let [minx, miny, maxx, maxy] = self.bb;
        let base = self.baseline(&matrix)?;
        self.layout = Layout {
            minx,
            miny,
            maxx,
            maxy,
            base,
            width: maxx - minx,
            height: maxy - base,
            depth: base - miny,
            matrix_w: wm,
            matrix_h: hm,
            matrix_d: dm,
            matrix_x: matrix.m.e,
            matrix_y: matrix.m.f,
        };
        Ok(&self.layout)
    }

    #[inline]
    fn add(&mut self, p: Pt, r: f64) {
        let b = &mut self.bb;
        b[0] = b[0].min(p.x - r);
        b[1] = b[1].min(p.y - r);
        b[2] = b[2].max(p.x + r);
        b[3] = b[3].max(p.y + r);
    }

    // --------------------------------------------------------------------------------------------------------- matrix

    /// Spec section 7.  Returns the matrix node and `Wm`, `Hm`, `Dm`.
    fn matrix(&mut self, boxes: &[BoxDim]) -> R<(Node, f64, f64, f64)> {
        let f = self.font;
        let (nrows, ncols) = (self.p.rows.len(), self.p.cols.len());
        let (ox, oy) = (0.5 * self.p.s.cell_lw, 0.5 * self.p.s.cell_lw);

        self.minx.clear();
        self.minx.resize(ncols, f64::INFINITY);
        self.maxx.clear();
        self.maxx.resize(ncols, f64::NEG_INFINITY);
        self.rowmax.clear();
        self.rowmax.resize(nrows, 0.0);
        self.rowmin.clear();
        self.rowmin.resize(nrows, 0.0);

        for (ci, cell) in self.p.cells.iter().enumerate() {
            let (r, c) = (cell.row as usize, cell.col as usize);
            let (mut l, mut rr, mut t, mut b) = (0.0, 0.0, 0.0, 0.0);
            let mut node = None;
            if cell.has_node {
                let o = &cell.opts;
                let d = boxes[ci];
                let n = Node::new(&Shape {
                    w: d.w,
                    h: o.text_height.unwrap_or(d.h),
                    d: o.text_depth.unwrap_or(d.d),
                    ixs: o.ixs,
                    iys: o.iys,
                    ox,
                    oy,
                    min_w: 1.0,
                    min_h: 1.0,
                    asym: true,
                    ax: f.ax,
                    ex: f.ex,
                });
                let cx = n.cx();
                l = n.sw.x + ox - cx;
                rr = n.ne.x - ox - cx;
                t = n.ne.y - oy;
                b = n.sw.y + oy;
                if o.draw {
                    let half = 0.5 * self.p.s.cell_lw;
                    l -= half;
                    rr += half;
                    t += half;
                    b -= half;
                }
                node = Some(n);
            }
            self.nodes.push(node);
            // `fixed` of the next column counts only once that column exists: it comes into being in cell order.
            if let Some(next) = self.p.cols.get(c + 1) {
                if next.created_row <= cell.row && next.fixed {
                    rr = 0.0;
                }
            }
            let this = self.p.cols[c];
            if this.fixed {
                l = 0.0;
            }
            if c > 0 {
                l -= this.sep;
            }
            if r > 0 && self.p.rows[r - 1].fixed {
                t = 0.0;
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
        self.ys.resize(nrows, 0.0);
        for r in (1..nrows).rev() {
            self.ys[r - 1] = self.ys[r] + self.rowmax[r] - self.rowmin[r - 1];
        }
        self.xs.clear();
        self.xs.resize(ncols, 0.0);
        self.xs[0] = -self.minx[0];
        for c in 1..ncols {
            self.xs[c] = self.xs[c - 1] + self.maxx[c - 1] - self.minx[c];
        }
        let wm: f64 = (0..ncols).map(|c| self.maxx[c] - self.minx[c]).sum();
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
            min_w: 1.0,
            min_h: 1.0,
            asym: false,
            ax: f.ax,
            ex: f.ex,
        });
        let mc = mat.anchor_local(Anchor::Center);
        let (offx, offy) = (-mc.x, -mc.y);
        mat.m = Matrix::translate(offx, offy);
        self.add(Pt::new(offx + mat.sw.x + ox, offy + mat.sw.y + oy), 0.0);
        self.add(Pt::new(offx + mat.ne.x - ox, offy + mat.ne.y - oy), 0.0);

        // Place the cells and draw them, in row order, before any arrow.
        for ci in 0..self.p.cells.len() {
            let cell = self.p.cells[ci];
            let Some(n) = self.nodes[ci].as_mut() else { continue };
            let (r, c) = (cell.row as usize, cell.col as usize);
            let x = self.xs[c] + offx - n.cx();
            let y = self.ys[r] + offy;
            n.m = Matrix::translate(x, y);
            let n = *n;
            let o = &cell.opts;
            if o.draw {
                let x0 = x + n.sw.x + ox;
                let y0 = y + n.sw.y + oy;
                let x1 = x + n.ne.x - ox;
                let y1 = y + n.ne.y - oy;
                let rounded = if o.rounded > 0.0 { Some(self.rounded_rect(x0, y0, x1, y1, o.rounded)?) } else { None };
                self.list.items.push(Item::CellBorder { x: x0, y: y0, w: x1 - x0, h: y1 - y0, rounded, color: o.draw_color });
            }
            self.list.items.push(Item::CellBox { cell: ci as u32, x, y, color: o.color });
        }
        Ok((mat, wm, hm, dm))
    }

    /// pgf's path for a rectangle with rounded corners, starting at the top right (seen in test 26).
    fn rounded_rect(&mut self, x0: f64, y0: f64, x1: f64, y1: f64, r: f64) -> R<PathRef> {
        if 2.0 * r > (x1 - x0).min(y1 - y0) {
            // pgf then shortens the corners, which no test has shown.
            return Err(Unsupported::new("rounded corners larger than the cell", Span::EMPTY));
        }
        const K: f64 = 0.55228;
        let q = r - K * r;
        let start = self.list.segs.len() as u32;
        self.list.segs.extend_from_slice(&[
            Seg::Move(Pt::new(x1 - r, y1)),
            Seg::Line(Pt::new(x0 + r, y1)),
            Seg::Curve(Pt::new(x0 + q, y1), Pt::new(x0, y1 - q), Pt::new(x0, y1 - r)),
            Seg::Line(Pt::new(x0, y0 + r)),
            Seg::Curve(Pt::new(x0, y0 + q), Pt::new(x0 + q, y0), Pt::new(x0 + r, y0)),
            Seg::Line(Pt::new(x1 - r, y0)),
            Seg::Curve(Pt::new(x1 - q, y0), Pt::new(x1, y0 + q), Pt::new(x1, y0 + r)),
            Seg::Line(Pt::new(x1, y1 - r)),
            Seg::Curve(Pt::new(x1, y1 - q), Pt::new(x1 - q, y1), Pt::new(x1 - r, y1)),
            Seg::Close,
            Seg::Move(Pt::new(x0, y0)),
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
        let i = self.p.cells.iter().position(|c| !c.opts.alias.is_empty() && c.opts.alias.get(&self.buf) == n)?;
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

    fn arrow(&mut self, a: &ArrowSpec, label_boxes: &[BoxDim]) -> R {
        if self.list.segs.len() > MAX_SEGS {
            return Err(Unsupported::new("diagram too large", Span::EMPTY));
        }
        let st = self.resolve(a.start, a)?;
        let en = if a.is_loop { st } else { self.resolve(a.end, a)? };
        let (st, en) = (EndRef::Node(st), EndRef::Node(en));

        let mut path = base_path(&st, &en, a.sa, a.ea, a.curve.as_ref());
        let relevant = a.canvas == Pt::ZERO;

        if a.shiftable {
            // The dry path counts in the bounding box and is not drawn (spec 8.3).
            if relevant {
                self.add(path.s, 0.0);
                if let Some((c1, c2)) = path.c {
                    self.add(c1, 0.0);
                    self.add(c2, 0.0);
                }
                self.add(path.e, 0.0);
            }
            let mut p = path.s.add(a.xlate_s);
            let mut q = path.e.add(a.xlate_e);
            let shifts = a.shifts.0 as usize..a.shifts.1 as usize;
            for s in &self.p.shifts[shifts.clone()] {
                if let Shift::Translate(t) = *s {
                    p = p.add(t);
                    q = q.add(t);
                }
            }
            for s in &self.p.shifts[shifts] {
                if let Shift::Left(r) = *s {
                    let n = norm(p.sub(q));
                    let d = Pt::new(r * n.y, -r * n.x);
                    p = p.add(d);
                    q = q.add(d);
                }
            }
            path = match a.curve {
                None => ArrowPath { s: p, c: None, e: q },
                Some(ref cv) => base_path(&EndRef::Coord(p), &EndRef::Coord(q), None, None, Some(cv)),
            };
        }

        let (s, e) = (path.s, path.e);
        let ctrl = path.c;

        // Widths (spec 10.2).
        let lw = a.lw;
        let (mut outer, mut inner) = (lw, 0.0);
        if a.double {
            let q = 0.45 * self.font.ex;
            outer = lw + q;
            inner = q - lw;
        }
        if let Some(d) = a.double_tikz {
            outer = 2.0 * lw + d;
            inner = d;
        }

        if relevant {
            self.add(s, 0.0);
            self.add(e, 0.0);
            if let Some((c1, c2)) = ctrl {
                self.add(c1, 0.0);
                self.add(c2, 0.0);
            }
        }

        // The decoration runs before shortening.
        let mut decorated = false;
        if a.squiggly {
            let mut pts = std::mem::take(&mut self.pts);
            let z = zigzag(&path, a.sq_lw, lw, a.sq_pre, a.sq_post, &mut pts);
            self.pts = pts;
            match z {
                Zigzag::None => {}
                Zigzag::Points => decorated = true,
                Zigzag::TooDense => return Err(Unsupported::new("squiggly arrow with too many segments", Span::EMPTY)),
            }
        }

        if a.crossing {
            let width = self.p.s.clearance;
            let halo = if decorated {
                self.push_points(None, None)
            } else if let Some((c1, c2)) = ctrl {
                let hs = s.along(a.short_s, norm(c1.sub(s)));
                let he = e.along(a.short_e, norm(c2.sub(e)));
                self.push_segs(&[Seg::Move(hs), Seg::Curve(c1, c2, he)])
            } else {
                let u = norm(e.sub(s));
                let hs = s.along(a.short_s, u);
                let he = e.along(-a.short_e, u);
                self.push_segs(&[Seg::Move(hs), Seg::Line(he)])
            };
            let bg = self.p.s.bg;
            self.list.items.push(Item::Stroke { role: Role::Halo, path: halo, width, color: Some(bg), canvas: a.canvas });
            if relevant {
                self.add(s, 0.5 * width);
                self.add(e, 0.5 * width);
                if let Some((c1, c2)) = ctrl {
                    self.add(c1, 0.5 * width);
                    self.add(c2, 0.5 * width);
                }
            }
        }

        self.list.items.push(Item::ArrowBegin {
            canvas: a.canvas,
            lw,
            color: a.color,
            draw_color: a.draw_color,
            opacity: a.opacity,
            dash: a.dash,
        });

        if a.draw {
            let style = self.p.s.style;
            let sta = tip_seq(a.tips[0], style, outer, inner);
            let ena = tip_seq(a.tips[1], style, outer, inner);
            let stop = |q: &TipSeq| q.tips().last().map_or(0.0, |t| t.tip + t.sep - t.line);
            let d_e = stop(&ena) + a.short_e;
            let d_s = stop(&sta) + a.short_s;

            let mut ref_e = ctrl.map_or(s, |c| c.1);
            let mut ref_s = ctrl.map_or(e, |c| c.0);
            if decorated {
                ref_e = self.pts[self.pts.len() - 2];
                ref_s = self.pts[1];
            }
            let ue = norm(ref_e.sub(e));
            let e2 = e.along(d_e, ue);
            if ctrl.is_none() && !decorated {
                ref_s = e2;
            }
            let us = norm(ref_s.sub(s));
            let s2 = s.along(d_s, us);

            let line = if decorated {
                self.push_points(Some(s2), Some(e2))
            } else if let Some((c1, c2)) = ctrl {
                self.push_segs(&[Seg::Move(s2), Seg::Curve(c1, c2, e2)])
            } else {
                self.push_segs(&[Seg::Move(s2), Seg::Line(e2)])
            };
            self.list.items.push(Item::Stroke { role: Role::Line, path: line, width: outer, color: None, canvas: a.canvas });
            if inner > 0.0 {
                let bg = a.double_color.unwrap_or(self.p.s.bg);
                self.list.items.push(Item::Stroke { role: Role::Inner, path: line, width: inner, color: Some(bg), canvas: a.canvas });
            }
            if relevant {
                self.add(s, 0.5 * outer);
                self.add(e, 0.5 * outer);
                if let Some((c1, c2)) = ctrl {
                    self.add(c1, 0.5 * outer);
                    self.add(c2, 0.5 * outer);
                }
            }
            self.tips(&sta, s, ref_s, a.short_s, relevant);
            self.tips(&ena, e, ref_e, a.short_e, relevant);
        }

        for li in a.labels.0..a.labels.1 {
            let l = self.p.labels[li as usize];
            self.label(a, &path, &l, li, label_boxes[li as usize], relevant);
        }
        self.list.items.push(Item::ArrowEnd);
        Ok(())
    }

    fn push_segs(&mut self, segs: &[Seg]) -> PathRef {
        let start = self.list.segs.len() as u32;
        self.list.segs.extend_from_slice(segs);
        PathRef { start, end: self.list.segs.len() as u32 }
    }

    /// The decorated points as a path, with the first and last replaced when given.
    fn push_points(&mut self, first: Option<Pt>, last: Option<Pt>) -> PathRef {
        let start = self.list.segs.len() as u32;
        let n = self.pts.len();
        for (i, &p) in self.pts.iter().enumerate() {
            let seg = if i == 0 {
                Seg::Move(first.unwrap_or(p))
            } else if i + 1 == n {
                Seg::Line(last.unwrap_or(p))
            } else {
                Seg::Line(p)
            };
            self.list.segs.push(seg);
        }
        PathRef { start, end: self.list.segs.len() as u32 }
    }

    /// Spec 9.3.
    fn tips(&mut self, seq: &TipSeq, p: Pt, reference: Pt, shorten: f64, relevant: bool) {
        let tips = seq.tips();
        if tips.is_empty() {
            return;
        }
        let total: f64 = tips.iter().map(|t| t.tip + t.sep - t.back).sum::<f64>() + shorten;
        let mut cum = -total;
        let u = norm(p.sub(reference));
        for t in tips {
            let t: &Tip = t;
            let o = p.along(cum - t.back, u);
            let sy = if t.swap { -1.0 } else { 1.0 };
            let m = Matrix { a: u.x, b: u.y, c: -sy * u.y, d: sy * u.x, e: o.x, f: o.y };
            if relevant {
                for &h in t.hull() {
                    self.add(m.apply(h), 0.0);
                }
            }
            let path = self.push_segs(t.segs());
            self.list.items.push(Item::Tip { m, path, width: t.lw, cap: t.cap, join: t.join, form: t.form });
            cum += t.tip + t.sep - t.back;
        }
    }

    // --------------------------------------------------------------------------------------------------------- labels

    /// Spec 6.1, 6.2, section 11.
    fn label(&mut self, a: &ArrowSpec, path: &ArrowPath, l: &LabelSpec, index: u32, dim: BoxDim, relevant: bool) {
        let f = self.font;
        let (pos, tan) = match path.c {
            None => (path.s.along(l.pos, path.e.sub(path.s)), norm(path.e.sub(path.s))),
            Some((c1, c2)) => {
                let (q, d, e) = bez(l.pos, path.s, c1, c2, path.e);
                (q, norm(e.sub(d)))
            }
        };
        let outer = l.outer.unwrap_or(0.5 * a.lw);
        let mut node = Node::new(&Shape {
            w: dim.w,
            h: dim.h,
            d: dim.d,
            ixs: l.ixs,
            iys: l.iys,
            ox: outer,
            oy: outer,
            min_w: l.min_w,
            min_h: l.min_h,
            asym: l.asym,
            ax: f.ax,
            ex: f.ex,
        });
        let anchor = if l.auto {
            auto_anchor(if l.sloped { Pt::new(1.0, 0.0) } else { tan }, l.swap)
        } else {
            l.anchor
        };
        // The rotation of the node's frame, then its transforms in key order, so a shift acts in the frame at that point.
        let mut r = Pt::new(1.0, 0.0);
        if l.sloped {
            r = tan;
            if r.x < 0.0 && !l.upside {
                r = Pt::new(-r.x, -r.y);
            }
        }
        let mut at = pos;
        for x in &self.p.xforms[l.xforms.0 as usize..l.xforms.1 as usize] {
            match *x {
                LabelXform::Shift(t) => at = at.add(Pt::new(r.x * t.x - r.y * t.y, r.y * t.x + r.x * t.y)),
                LabelXform::Rotate(d) => r = rot(r, d),
            }
        }
        let ap = node.anchor_local(anchor);
        let (c, sn) = (r.x, r.y);
        let (ox, oy) = (at.x - (c * ap.x - sn * ap.y), at.y - (sn * ap.x + c * ap.y));
        node.m = Matrix { a: c, b: sn, c: -sn, d: c, e: ox, f: oy };
        if !l.name.is_empty() {
            self.named.push((l.name, node));
        }
        if !l.alias.is_empty() {
            self.named.push((l.alias, node));
        }
        // Background rectangle: no outer sep.
        let (x0, x1) = (node.sw.x + node.ox, node.ne.x - node.ox);
        let (y0, y1) = (node.sw.y + node.oy, node.ne.y - node.oy);
        if relevant {
            for p in [Pt::new(x0, y0), Pt::new(x0, y1), Pt::new(x1, y0), Pt::new(x1, y1)] {
                self.add(node.m.apply(p), 0.0);
            }
        }
        if let Some(color) = l.fill {
            self.list.items.push(Item::LabelFill { m: node.m, x: x0, y: y0, w: x1 - x0, h: y1 - y0, color });
        }
        self.list.items.push(Item::LabelBox { label: index, m: node.m, color: l.color });
    }

    // ------------------------------------------------------------------------------------------------------- baseline

    /// Spec section 12.
    fn baseline(&self, matrix: &Node) -> R<f64> {
        if self.p.rows.len() == 1 {
            // `1-row diagram` runs after the matrix and so wins over `baseline=`.
            return Ok(matrix.anchor(Anchor::Base).y);
        }
        match self.p.s.baseline {
            Baseline::Dim(d) => Ok(d),
            Baseline::Node { node, anchor, yshift } => {
                let [_, miny, _, maxy] = self.bb;
                let y = match node {
                    BaseNode::BoundingBox => match anchor {
                        Anchor::North | Anchor::NorthEast | Anchor::NorthWest => maxy,
                        Anchor::South | Anchor::SouthEast | Anchor::SouthWest => miny,
                        _ => 0.5 * (miny + maxy),
                    },
                    BaseNode::Matrix => matrix.anchor(anchor).y,
                    BaseNode::Cell(r, c) => self
                        .cell_node(i64::from(r) - 1, i64::from(c) - 1)
                        .ok_or(Unsupported::new("baseline names a cell without a node", Span::EMPTY))?
                        .anchor(anchor)
                        .y,
                    BaseNode::Name(s) => {
                        self.named_node(s).ok_or(Unsupported::new("baseline names an unknown node", s))?.anchor(anchor).y
                    }
                };
                Ok(y + yshift)
            }
        }
    }
}

/// The table of spec section 11.
fn auto_anchor(v: Pt, swap: bool) -> Anchor {
    const E: f64 = 0.05;
    let (x, y) = (v.x, v.y);
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
        } else if y > 0.0 {
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
    } else if y > 0.0 {
        Anchor::West
    } else {
        Anchor::East
    }
}
