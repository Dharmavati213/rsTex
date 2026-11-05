//! Points and transforms as the outside sees them: `f64`, but every value is a whole number of scaled points or a
//! coefficient TeX holds the same way, so nothing has rounded and [`sp`] gets the integer back.

use crate::pgfmath::{Sp, Tm, Xy};

/// Scaled points as points, exactly.
#[inline]
pub(crate) fn pt(v: Sp) -> f64 {
    v as f64 / 65536.0
}

/// Points as scaled points.  A value from outside that is not a whole number goes to the nearest; NaN to 0; what TeX could
/// not hold, to just past what it can.
#[inline]
pub(crate) fn sp(v: f64) -> Sp {
    ((v * 65536.0).round() as Sp).clamp(-(1 << 31), 1 << 31)
}

/// A point or vector in TeX points; y points up.  Picture coordinates have their origin at the centre of the matrix
/// node (spec section 2).
#[derive(Copy, Clone, Debug, PartialEq, Default)]
pub struct Pt {
    pub x: f64,
    pub y: f64,
}

impl Pt {
    pub const ZERO: Pt = Pt { x: 0.0, y: 0.0 };

    #[inline]
    pub const fn new(x: f64, y: f64) -> Pt {
        Pt { x, y }
    }

    #[inline]
    pub(crate) fn of(p: Xy) -> Pt {
        Pt::new(pt(p.0), pt(p.1))
    }
}

/// An affine map in PDF's order: `x' = a x + c y + e`, `y' = b x + d y + f`.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Matrix {
    pub a: f64,
    pub b: f64,
    pub c: f64,
    pub d: f64,
    pub e: f64,
    pub f: f64,
}

impl Matrix {
    pub const IDENTITY: Matrix = Matrix { a: 1.0, b: 0.0, c: 0.0, d: 1.0, e: 0.0, f: 0.0 };

    #[inline]
    pub const fn translate(e: f64, f: f64) -> Matrix {
        Matrix { a: 1.0, b: 0.0, c: 0.0, d: 1.0, e, f }
    }

    #[inline]
    pub fn apply(&self, p: Pt) -> Pt {
        Pt::new(self.a * p.x + self.c * p.y + self.e, self.b * p.x + self.d * p.y + self.f)
    }

    /// True when the linear part is the identity, so the map is a pure translation.
    #[inline]
    pub fn is_translation(&self) -> bool {
        self.a == 1.0 && self.b == 0.0 && self.c == 0.0 && self.d == 1.0
    }

    #[inline]
    pub(crate) fn of(t: &Tm) -> Matrix {
        Matrix { a: pt(t.aa), b: pt(t.ab), c: pt(t.ba), d: pt(t.bb), e: pt(t.x), f: pt(t.y) }
    }
}

/// A path segment.  Segments live in one shared vector, see [`crate::DrawList`].
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum Seg {
    Move(Pt),
    Line(Pt),
    Curve(Pt, Pt, Pt),
    /// `h`: close the subpath, used by the rounded border of a cell.
    Close,
}

impl Seg {
    /// The same segment under a transform.
    pub fn map(self, m: &Matrix) -> Seg {
        match self {
            Seg::Move(p) => Seg::Move(m.apply(p)),
            Seg::Line(p) => Seg::Line(m.apply(p)),
            Seg::Curve(a, b, c) => Seg::Curve(m.apply(a), m.apply(b), m.apply(c)),
            Seg::Close => Seg::Close,
        }
    }
}
