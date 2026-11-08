//! What the parser produces: fully evaluated records, one per cell, arrow and label, in reusable vectors.  Lengths are
//! scaled points, as PGF holds them.

use crate::color::Color;
use crate::expr::Value;
use crate::geom::pt;
use crate::pgfmath::{Sp, Xy};
use crate::scan::Span;
use crate::tips::GtTip;

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

/// What a key is in the document, as far as the keys it has set are known to a caller.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum UserKey {
    /// tikz-cd's and TikZ's own, or not defined.
    Standard,
    /// The document's, in `/tikz/commutative diagrams`.
    Cd,
    /// The document's, in `/tikz`.
    Tikz,
}

/// The document's keys, for a caller that can read them where they are: this is how a diagram gets at a `\tikzset` the
/// caller did not see as text (spec 5.1).
pub trait UserKeys {
    /// What `name` means now.  For anything but [`UserKey::Standard`], `out` gets the replacement text of its
    /// `\pgfk@<directory>/<name>/.@cmd`, as TeX prints it, `#1` for the value.  `\pgfkeysalso{...}` in it is how a style reads.
    fn user_key(&self, name: &[u8], out: &mut Vec<u8>) -> UserKey;

    /// `.default` of a key the document made: its `/.@def` goes to `out`.  False if it has none.
    fn default_value(&self, _name: &[u8], _out: &mut Vec<u8>) -> bool {
        false
    }

    /// What a colour name means in the document, base colours included, if the caller can tell.
    fn color(&self, _name: &[u8]) -> Option<Color> {
        None
    }

    /// Whether the document has `name` as a colour in a model PGF has no operators for, as xcolor keeps hsb and wave: real PGF stops on it.
    fn color_pgf_cannot_set(&self, _name: &[u8]) -> bool {
        false
    }

    /// Whether xcolor's base colours and its model are as `color.rs` has them, for the mixes it works out itself.
    fn base_colors(&self) -> bool {
        true
    }
}

/// No key is known but through the preset.
#[derive(Copy, Clone, Debug, Default)]
pub struct NoUserKeys;

impl UserKeys for NoUserKeys {
    #[inline]
    fn user_key(&self, _name: &[u8], _out: &mut Vec<u8>) -> UserKey {
        UserKey::Standard
    }
}

/// Which node anchor.  Local coordinates are in spec 6.2.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
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
    /// A number, truncated as `\pgfmathsetcounter` does: the border point in that direction.
    Angle(i64),
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
    pub(crate) fn two(a: Sp, b: Sp) -> Dash {
        Dash { n: 2, v: [pt(a), pt(b), 0.0, 0.0, 0.0, 0.0, 0.0, 0.0] }
    }

    #[inline]
    pub fn values(&self) -> &[f64] {
        &self.v[..usize::from(self.n).min(8)]
    }
}

/// `in`, `out`, `looseness`, `bend`, ... (spec 8.2).  Angles and factors as PGF's decimals.
#[derive(Copy, Clone, Debug, PartialEq)]
pub(crate) struct Curve {
    pub relative: bool,
    pub out: Sp,
    pub inn: Sp,
    pub l_out: Sp,
    pub l_in: Sp,
    pub min_out: Sp,
    pub max_out: Sp,
    pub min_in: Sp,
    pub max_in: Sp,
}

impl Curve {
    pub(crate) const DEFAULT: Curve = Curve {
        relative: false,
        out: 45 * crate::pgfmath::PT,
        inn: 135 * crate::pgfmath::PT,
        l_out: crate::pgfmath::PT,
        l_in: crate::pgfmath::PT,
        min_out: 0,
        max_out: 10000 * crate::pgfmath::PT,
        min_in: 0,
        max_in: 10000 * crate::pgfmath::PT,
    };
}

/// `curve=` of quiver.sty, spec 8.5.
#[derive(Copy, Clone, Debug, PartialEq)]
pub(crate) struct QuiverCurve {
    pub pos: Sp,
    pub height: Value,
}

/// One step of a parallel shift, in key order (spec 8.3).
#[derive(Copy, Clone, Debug, PartialEq)]
pub(crate) enum Shift {
    /// `shift left` by this much.
    Left(Sp),
    /// `xshift`, `yshift`, `shift`.
    Translate(Xy),
}

/// One node transform of a label, in key order (spec 11).
#[derive(Copy, Clone, Debug, PartialEq)]
pub(crate) enum LabelXform {
    Shift(Xy),
    Rotate(Sp),
    /// `scale`: a node's own transforms hold for its shape too.
    Scale(Sp),
}

/// What the diagram's last `\pgfmatrixnextcell` had as its optional argument, in the order TeX runs them: real PGF's
/// `\pgf@matrix@cell@cont` `\xdef`s it into `\pgf@matrix@last@nextcell@options` at each one (pgfmodulematrix.code.tex),
/// and the macro outlives the diagram.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub enum NextCell {
    /// No `\pgfmatrixnextcell` ran: the macro keeps what it had.
    #[default]
    None,
    /// No `[`: `0pt`.
    Plain,
    /// What stands between `[` and `]`, as the delimited argument has it before TeX takes the braces off a single group.
    Options(Span),
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

/// One thing the arrow's options do to the graphics state, in key order: TikZ runs them in that order at the start of the
/// arrow's scope, so that is the order of the operators.
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum Opt {
    /// `\pgfsetlinewidth`.
    Width(f64),
    /// `\pgfsetdash`.
    Dash(Dash),
    /// `<color>` alone: `\pgfutil@color`, which writes fill and stroke and takes the colour off the stack when its group ends.
    Color(Color),
    /// `color=<c>`: `\pgfsetcolor`, stroke and fill.
    SetColor(Color),
    /// `draw=<c>`: `\pgfsetstrokecolor`.
    Stroke(Color),
    /// `fill=<c>`: `\pgfsetfillcolor`.
    Fill(Color),
    /// `opacity=`, as written: `\pgfsetstrokeopacity` and `\pgfsetfillopacity`.
    Opacity(f64),
    /// `draw opacity=`.
    StrokeOpacity(f64),
    /// `fill opacity=`.
    FillOpacity(f64),
    /// `transform canvas=`: `\pgflowlevelsynccm` of a translation.
    Canvas(f64, f64),
    /// `line cap=`, 0 to 2.
    Cap(u8),
    /// `line join=`, 0 to 2.
    Join(u8),
}

/// A run of [`crate::DrawList::opts`] while it is being built, and of the parser's list before that.
pub(crate) type OptRange = (u32, u32);

#[derive(Copy, Clone, Debug)]
pub(crate) struct ArrowSpec {
    /// Home cell, 0-based.
    pub r: u32,
    pub c: u32,
    pub start: End,
    pub end: End,
    pub sa: Option<Anchor>,
    pub ea: Option<Anchor>,
    pub xlate_s: Xy,
    pub xlate_e: Xy,
    pub shiftable: bool,
    pub shifts: (u32, u32),
    /// `\pgflinewidth` after the last key.
    pub lw: Sp,
    pub tips: [TipName; 2],
    pub double: bool,
    /// `/tikz/double` or `double distance=` after the last `double line`: `Some(inner width)`, which then sets the widths.
    pub double_tikz: Option<Sp>,
    pub double_seen: bool,
    pub double_color: Option<Color>,
    pub dash: Dash,
    pub squiggly: bool,
    /// `segment length` and `amplitude`: evaluated when `squiggly` ran, so with the line width then.
    pub sq_seg: Sp,
    pub sq_amp: Sp,
    pub sq_pre: Option<Sp>,
    pub sq_post: Option<Sp>,
    pub draw: bool,
    pub phantom: bool,
    /// How many of this arrow's `labels=` came before its last `phantom`.
    pub phantom_at: u8,
    /// `color=` or `<color>`: stroke, fill and label text.
    pub color: Option<Color>,
    /// The labels' text colour, `\tikz@textcolor`: what the last of `text=`, `color=` and a colour by its name said.
    pub text_color: Option<Color>,
    pub opacity: Option<f64>,
    pub short_s: Sp,
    pub short_e: Sp,
    pub pos: Sp,
    pub swap: bool,
    pub sloped: bool,
    pub upside: bool,
    pub curve: Option<Curve>,
    pub quiver: Option<QuiverCurve>,
    pub is_loop: bool,
    /// `\tikz@to@bend`, as these options have left it.
    pub bend: Sp,
    /// `overlay`: what it draws does not count for the picture's box.
    pub overlay: bool,
    pub crossing: bool,
    pub canvas: Xy,
    pub opts: OptRange,
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
    pub(crate) ixs: Sp,
    pub(crate) iys: Sp,
    pub(crate) outer: Option<Sp>,
    pub(crate) min_w: Sp,
    pub(crate) min_h: Sp,
    pub(crate) pos: Sp,
    pub(crate) swap: bool,
    pub(crate) auto: bool,
    pub(crate) anchor: Anchor,
    pub(crate) asym: bool,
    pub(crate) fill: Option<Color>,
    pub(crate) name: Span,
    pub(crate) alias: Span,
    pub(crate) sloped: bool,
    pub(crate) upside: bool,
    pub(crate) overlay: bool,
    pub(crate) xforms: (u32, u32),
    /// What its own keys do to the graphics state.
    pub(crate) opts: OptRange,
}

/// Fields of [`CellOpts`] by bit, as [`CellOpts::nodes_set`] has them.
pub(crate) const CELL_IXS: u8 = 1;
pub(crate) const CELL_IYS: u8 = 2;
pub(crate) const CELL_FONT: u8 = 4;
pub(crate) const CELL_ROUNDED: u8 = 8;
pub(crate) const CELL_ALIAS: u8 = 16;
pub(crate) const CELL_HEIGHT: u8 = 32;
pub(crate) const CELL_DEPTH: u8 = 64;

/// A cell's options, from `every cell`, `cells=` and `|[...]|`.
#[derive(Copy, Clone, Debug)]
pub(crate) struct CellOpts {
    pub ixs: Sp,
    pub iys: Sp,
    pub font: Span,
    pub color: Option<Color>,
    /// The text colour from `nodes=` in `every cell`, which runs after what `every cell` says itself.
    pub nodes_color: Option<Color>,
    /// Which of the fields `CELL_*` a `nodes=` in `every cell` has set, which so hold against what `every cell` says itself.
    pub nodes_set: u8,
    pub draw: bool,
    pub rounded: Sp,
    pub alias: Span,
    pub text_height: Option<Sp>,
    pub text_depth: Option<Sp>,
    /// What the keys do to the graphics state: those of `every cell`, which run in the cell's scope, then those of its `nodes=` and the cell's own, which run around the node.
    pub opts_base: OptRange,
    pub opts_nodes: OptRange,
    pub opts_own: OptRange,
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
    pub sep: Sp,
    pub fixed: bool,
}

#[derive(Copy, Clone, Debug)]
pub(crate) struct ColSpec {
    /// Separation before this column, and its flag.
    pub sep: Sp,
    pub fixed: bool,
    /// The row in which this column came into being: `fixed` is not visible before it (spec 7).
    pub created_row: u32,
}

/// `baseline=` (spec 12).
#[derive(Copy, Clone, Debug, PartialEq)]
pub(crate) enum Baseline {
    /// `baseline=<dim>`, default 0.
    Dim(Sp),
    /// `baseline=([yshift=..]<node>.<anchor>)`.
    Node { node: BaseNode, anchor: Anchor, yshift: Sp },
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
    pub rowsep: Sp,
    pub colsep: Sp,
    pub row_fixed: bool,
    pub col_fixed: bool,
    pub cell: CellOpts,
    pub mat_inner: Sp,
    pub baseline: Baseline,
    pub bg: Color,
    pub clearance: Sp,
    pub math_mode: bool,
    pub style: ArrowStyle,
    pub cell_lw: Sp,
    pub nodes_in_empty: bool,
    pub amp: Span,
    /// `>=`.
    pub gt: GtTip,
    /// Which tips `<name>/.tip` has changed, by [`TipName`].
    pub tips_changed: u16,
    /// `every cell` has set a font, its own or by `nodes=`: the picture's `font=` no longer reaches the cells.
    pub cell_font: bool,
}
