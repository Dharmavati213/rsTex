//! Points, transforms, Bezier evaluation.

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
    pub(crate) fn add(self, o: Pt) -> Pt {
        Pt::new(self.x + o.x, self.y + o.y)
    }

    #[inline]
    pub(crate) fn sub(self, o: Pt) -> Pt {
        Pt::new(self.x - o.x, self.y - o.y)
    }

    /// `self + k * v`.
    #[inline]
    pub(crate) fn along(self, k: f64, v: Pt) -> Pt {
        Pt::new(self.x + k * v.x, self.y + k * v.y)
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

/// De Casteljau at `t`: the point, and the two points `d`, `e` of the last step, whose difference is the tangent.
#[inline]
pub(crate) fn bez(t: f64, p0: Pt, p1: Pt, p2: Pt, p3: Pt) -> (Pt, Pt, Pt) {
    let u = 1.0 - t;
    let a = Pt::new(u * p0.x + t * p1.x, u * p0.y + t * p1.y);
    let b = Pt::new(u * p1.x + t * p2.x, u * p1.y + t * p2.y);
    let c = Pt::new(u * p2.x + t * p3.x, u * p2.y + t * p3.y);
    let d = Pt::new(u * a.x + t * b.x, u * a.y + t * b.y);
    let e = Pt::new(u * b.x + t * c.x, u * b.y + t * c.y);
    (Pt::new(u * d.x + t * e.x, u * d.y + t * e.y), d, e)
}
