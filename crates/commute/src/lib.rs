//! Native tikz-cd: diagram geometry from `docs/TIKZCD-SPEC.md`.  No engine dependency (DESIGN section 10).
//!
//! A [`Diagram`] is a reusable workspace.  Its life for one diagram:
//!
//! 1. [`Diagram::parse`] takes the text after `\begin{tikzcd}`, the ambient [`FontParams`] and a [`Policy`], and either
//!    fills in the cells, arrows and labels, or returns [`Unsupported`].  Unsupported is the caller's cue to run real
//!    PGF; it is never a partial answer.
//! 2. The caller typesets [`Diagram::cells`] and [`Diagram::labels`] and measures them.
//! 3. [`Diagram::layout`] computes everything in spec sections 7 to 12 into a backend-neutral [`DrawList`].
//! 4. [`pdf::emit`] writes PDF operators in the form pgf writes them, or [`svg::emit`] an SVG.
//!
//! All lengths at the interface are TeX points in `f64`, picture coordinates with y up, and each is a whole number of
//! scaled points, so nothing has rounded on the way out.  Inside, the arithmetic is PGF's own, in scaled points with its
//! truncation at every step (pgfmath.rs): the box a diagram gets must be the box real PGF computes, to the scaled point.
//! After the first diagram of a given size nothing here allocates: all storage is in vectors that keep their capacity.
//!
//! Same output bytes on every platform: only integers, once the input is read.
#![forbid(unsafe_code)]

mod color;
mod csnames;
mod diagram;
mod expr;
mod geom;
mod keys;
mod node;
mod number;
mod parse;
mod path;
mod pgfmath;
mod policy;
mod scan;
mod spec;
mod tips;

pub mod pdf;
pub mod svg;

pub use color::{Color, base_colors};
pub use diagram::{BoxDim, Diagram, DrawList, Item, Layout, PathRef, Role};
pub use geom::{Matrix, Pt, Seg};
pub use number::BpRule;
pub use policy::{AllowAll, Policy, StandardLatex};
pub use scan::Span;
pub use spec::{Anchor, CellSpec, Dash, LabelFont, LabelSpec, NextCell, NoUserKeys, Opt, Unsupported, UserKey, UserKeys};
pub use tips::TipForm;

/// The crate's version, for the command-line tool.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// The ambient font's parameters, in points (spec section 4).  They are read in the ambient font; `font=` inside a cell
/// or a label does not change them.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct FontParams {
    /// `\fontdimen5` of the current text font.
    pub ex: f64,
    /// `\fontdimen6` of the same font.
    pub em: f64,
    /// `\fontdimen8\textfont3`, the rule thickness: the arrows' line width.
    pub rt: f64,
    /// `\fontdimen22\textfont2`, the axis height.
    pub ax: f64,
}

impl FontParams {
    /// article, 10pt, Computer Modern.
    pub const ARTICLE_10PT: FontParams = FontParams { ex: 4.30554, em: 10.00002, rt: 0.39998, ax: 2.5 };
}

/// A font parameter in scaled points.
#[inline]
pub(crate) fn font_sp(v: f64) -> pgfmath::Sp {
    geom::sp(v)
}

/// A box a backend asks the caller to place.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum BoxRef {
    /// Index into [`Diagram::cells`].
    Cell(u32),
    /// Index into [`Diagram::labels`].
    Label(u32),
}
