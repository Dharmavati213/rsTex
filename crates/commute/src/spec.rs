//! What the parser produces: fully evaluated records, one per cell, arrow and label, in reusable vectors.

use crate::color::Color;
use crate::geom::Pt;
use crate::scan::Span;

/// A diagram outside the recognised subset.  The caller runs real PGF for it (DESIGN section 10).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Unsupported {
    /// Why, in a few words.
    pub reason: &'static str,
    /// The source text concerned; [`crate::Diagram::text`] gives it, empty if there is none.
    pub at: Span,
}

impl Unsupported {
    pub(crate) const fn new(reason: &'static str, at: Span) -> Unsupported {
        Unsupported { reason, at }
    }
}

/// Which node anchor.  Local coordinates are in spec 6.2.
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum Anchor {
    Center,
    West,
    East,
    North,
    South,
    NorthEast,
    NorthWest,
    SouthEast,
    SouthWest,
    Base,
    BaseWest,
    BaseEast,
    Mid,
    MidWest,
    MidEast,
    Text,
    RealCenter,
    RealWest,
    RealEast,
    /// A number: `border(polar(trunc(a), 1pt))`.
    Angle(f64),
}

/// An arrow end before the nodes exist.
#[derive(Copy, Clone, Debug, PartialEq)]
pub(crate) enum End {
    /// The current cell.
    Home,
    /// Relative to the current cell, in rows and columns.
    Rel(i64, i64),
    /// `r-c`, 1-based as written.
    Abs(u32, u32),
    /// An alias or a label's name.
    Name(Span),
}

/// The tip names of spec 9.2, as tikz-cd's styles select them.
#[derive(Copy, Clone, Debug, PartialEq)]
pub(crate) enum TipName {
    Cap,
    ImpliesCap,
    To,
    ToReversed,
    LeftTo,
    RightTo,
    DoubleTo,
    Bar,
    ImpliesBar,
    LeftHook,
    RightHook,
    Implies,
    /// `/tikz/->`: Computer Modern Rightarrow with its own defaults (spec 9.5).
    PlainTo,
}

/// `arrow style=` (spec 9.6).
#[derive(Copy, Clone, Debug, PartialEq)]
pub(crate) enum ArrowStyle {
    Default,
    LatinModern,
    Tikz,
}

/// A dash pattern in points, phase 0.  `n == 0` means solid.
#[derive(Copy, Clone, Debug, PartialEq, Default)]
pub struct Dash {
    pub n: u8,
    pub v: [f64; 8],
}

impl Dash {
    pub(crate) const fn two(a: f64, b: f64) -> Dash {
        Dash { n: 2, v: [a, b, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0] }
    }

    #[inline]
    pub fn values(&self) -> &[f64] {
        &self.v[..usize::from(self.n).min(8)]
    }
}

/// `in`, `out`, `looseness`, `bend`, ... (spec 8.2).
#[derive(Copy, Clone, Debug, PartialEq)]
pub(crate) struct Curve {
    pub relative: bool,
    pub out: f64,
    pub inn: f64,
    pub l_out: f64,
    pub l_in: f64,
    pub min: f64,
    pub max: f64,
}

impl Curve {
    pub(crate) const DEFAULT: Curve = Curve {
        relative: false,
        out: 45.0,
        inn: 135.0,
        l_out: 1.0,
        l_in: 1.0,
        min: 0.0,
        max: 1e4,
    };
}

/// One step of a parallel shift, in key order (spec 8.3).
#[derive(Copy, Clone, Debug, PartialEq)]
pub(crate) enum Shift {
    /// `shift left` by this many points.
    Left(f64),
    /// `xshift`, `yshift`, `shift`.
    Translate(Pt),
}

/// One node transform of a label, in key order (spec 11).
#[derive(Copy, Clone, Debug, PartialEq)]
pub(crate) enum LabelXform {
    Shift(Pt),
    Rotate(f64),
}

/// A label's font, for whoever typesets it.
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum LabelFont {
    /// `every label`'s default: `\scriptstyle` through `\everymath`.
    Script,
    /// `font=` with nothing, from `phantom`, `marking` or by hand: text style.
    Text,
    /// `font=<cmd>`, which replaces the `\scriptstyle` setup (spec 5.5).
    Command(Span),
}

#[derive(Copy, Clone, Debug)]
pub(crate) struct ArrowSpec {
    /// Home cell, 0-based.
    pub r: u32,
    pub c: u32,
    pub start: End,
    pub end: End,
    pub sa: Option<Anchor>,
    pub ea: Option<Anchor>,
    pub xlate_s: Pt,
    pub xlate_e: Pt,
    pub shiftable: bool,
    pub shifts: (u32, u32),
    pub lw: f64,
    pub tips: [TipName; 2],
    pub double: bool,
    /// `/tikz/double`: `Some(inner width)`.
    pub double_tikz: Option<f64>,
    pub double_seen: bool,
    pub double_color: Option<Color>,
    pub dash: Dash,
    pub squiggly: bool,
    /// `\pgflinewidth` when `squiggly` ran: its lengths are evaluated then.
    pub sq_lw: f64,
    pub sq_pre: Option<f64>,
    pub sq_post: Option<f64>,
    pub draw: bool,
    pub phantom: bool,
    /// `color=` or `<color>`: stroke, fill and label text.
    pub color: Option<Color>,
    /// `draw=<c>`: stroke only.
    pub draw_color: Option<Color>,
    pub opacity: Option<f64>,
    pub short_s: f64,
    pub short_e: f64,
    pub pos: f64,
    pub swap: bool,
    pub sloped: bool,
    pub curve: Option<Curve>,
    pub is_loop: bool,
    pub crossing: bool,
    pub canvas: Pt,
    pub labels: (u32, u32),
    /// `labels=` given on this arrow.
    pub own_every_label: [Span; 4],
    pub own_every_label_n: u8,
}

/// A label, fully evaluated except for the box the caller measures.
#[derive(Copy, Clone, Debug)]
pub struct LabelSpec {
    /// The text between the quotes, or an old-syntax argument, without outer braces.
    pub text: Span,
    pub font: LabelFont,
    /// The text colour, if not the current one.
    pub color: Option<Color>,
    pub(crate) raw_swap: bool,
    pub(crate) raw_opts: Span,
    pub(crate) ixs: f64,
    pub(crate) iys: f64,
    pub(crate) outer: Option<f64>,
    pub(crate) min_w: f64,
    pub(crate) min_h: f64,
    pub(crate) pos: f64,
    pub(crate) swap: bool,
    pub(crate) auto: bool,
    pub(crate) anchor: Anchor,
    pub(crate) asym: bool,
    pub(crate) fill: Option<Color>,
    pub(crate) name: Span,
    pub(crate) alias: Span,
    pub(crate) sloped: bool,
    pub(crate) upside: bool,
    pub(crate) xforms: (u32, u32),
}

/// A cell's options, from `every cell`, `cells=` and `|[...]|`.
#[derive(Copy, Clone, Debug)]
pub(crate) struct CellOpts {
    pub ixs: f64,
    pub iys: f64,
    pub font: Span,
    pub color: Option<Color>,
    pub draw: bool,
    pub draw_color: Option<Color>,
    pub rounded: f64,
    pub alias: Span,
    pub text_height: Option<f64>,
    pub text_depth: Option<f64>,
}

/// A cell of the matrix, in row order.
#[derive(Copy, Clone, Debug)]
pub struct CellSpec {
    /// Row and column, 0-based.
    pub row: u32,
    pub col: u32,
    /// False for a truly empty cell: no node, zero metrics (spec 3.2).
    pub has_node: bool,
    pub(crate) content: (u32, u32),
    pub(crate) opts: CellOpts,
}

impl CellSpec {
    /// `font=` in force for this cell, empty if none.
    #[inline]
    pub fn font(&self) -> Span {
        self.opts.font
    }

    /// Text colour, if not the current one.
    #[inline]
    pub fn color(&self) -> Option<Color> {
        self.opts.color
    }
}

#[derive(Copy, Clone, Debug)]
pub(crate) struct RowSpec {
    pub first_cell: u32,
    pub ncells: u32,
    /// Separation after this row, and its `between origins` flag.
    pub sep: f64,
    pub fixed: bool,
}

#[derive(Copy, Clone, Debug)]
pub(crate) struct ColSpec {
    /// Separation before this column, and its flag.
    pub sep: f64,
    pub fixed: bool,
    /// The row in which this column came into being: `fixed` is not visible before it (spec 7).
    pub created_row: u32,
}

/// `baseline=` (spec 12).
#[derive(Copy, Clone, Debug, PartialEq)]
pub(crate) enum Baseline {
    /// `baseline=<dim>`, default 0.
    Dim(f64),
    /// `baseline=([yshift=..]<node>.<anchor>)`.
    Node { node: BaseNode, anchor: Anchor, yshift: f64 },
}

#[derive(Copy, Clone, Debug, PartialEq)]
pub(crate) enum BaseNode {
    Matrix,
    Cell(u32, u32),
    Name(Span),
    BoundingBox,
}

/// Environment settings.
#[derive(Copy, Clone, Debug)]
pub(crate) struct Settings {
    pub rowsep: f64,
    pub colsep: f64,
    pub row_fixed: bool,
    pub col_fixed: bool,
    pub cell: CellOpts,
    pub mat_inner: f64,
    pub baseline: Baseline,
    pub bg: Color,
    pub clearance: f64,
    pub math_mode: bool,
    pub style: ArrowStyle,
    pub cell_lw: f64,
    pub nodes_in_empty: bool,
    pub amp: Span,
}
