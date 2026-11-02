//! Shapes (spec 6.2, 6.3): rectangle and tikz-cd's asymmetrical rectangle.

use crate::fmath::{hypot, sincos_deg};
use crate::geom::{Matrix, Pt};
use crate::spec::Anchor;

/// A placed node.  Local coordinates are relative to the left end of the text box on its baseline; `m` maps them to
/// picture coordinates.
#[derive(Copy, Clone, Debug)]
pub(crate) struct Node {
    pub ne: Pt,
    pub sw: Pt,
    pub ox: f64,
    pub oy: f64,
    pub asym: bool,
    pub ax: f64,
    pub ex: f64,
    pub m: Matrix,
}

/// Input to [`Node::new`].
pub(crate) struct Shape {
    pub w: f64,
    pub h: f64,
    pub d: f64,
    pub ixs: f64,
    pub iys: f64,
    pub ox: f64,
    pub oy: f64,
    pub min_w: f64,
    pub min_h: f64,
    pub asym: bool,
    pub ax: f64,
    pub ex: f64,
}

impl Node {
    pub(crate) fn new(s: &Shape) -> Node {
        let big_w = (s.w + 2.0 * s.ixs).max(s.min_w);
        let big_h = (s.h + s.d + 2.0 * s.iys).max(s.min_h);
        Node {
            ne: Pt::new(0.5 * big_w + 0.5 * s.w + s.ox, 0.5 * big_h - 0.5 * s.d + 0.5 * s.h + s.oy),
            sw: Pt::new(-0.5 * big_w + 0.5 * s.w - s.ox, -0.5 * big_h - 0.5 * s.d + 0.5 * s.h - s.oy),
            ox: s.ox,
            oy: s.oy,
            asym: s.asym,
            ax: s.ax,
            ex: s.ex,
            m: Matrix::IDENTITY,
        }
    }

    /// The horizontal centre, `w/2`, in local coordinates.
    #[inline]
    pub(crate) fn cx(&self) -> f64 {
        0.5 * (self.ne.x + self.sw.x)
    }

    /// An anchor in local coordinates.
    pub(crate) fn anchor_local(&self, a: Anchor) -> Pt {
        let (ne, sw) = (self.ne, self.sw);
        let cx = self.cx();
        let cy = 0.5 * (ne.y + sw.y);
        let y = if self.asym { self.ax } else { cy };
        match a {
            Anchor::Center => Pt::new(cx, y),
            Anchor::West => Pt::new(sw.x, y),
            Anchor::East => Pt::new(ne.x, y),
            Anchor::North => Pt::new(cx, ne.y),
            Anchor::South => Pt::new(cx, sw.y),
            Anchor::NorthEast => ne,
            Anchor::SouthWest => sw,
            Anchor::NorthWest => Pt::new(sw.x, ne.y),
            Anchor::SouthEast => Pt::new(ne.x, sw.y),
            Anchor::Base => Pt::new(cx, 0.0),
            Anchor::BaseWest => Pt::new(sw.x, 0.0),
            Anchor::BaseEast => Pt::new(ne.x, 0.0),
            Anchor::Mid => Pt::new(cx, 0.5 * self.ex),
            Anchor::MidWest => Pt::new(sw.x, 0.5 * self.ex),
            Anchor::MidEast => Pt::new(ne.x, 0.5 * self.ex),
            Anchor::Text => Pt::ZERO,
            Anchor::RealCenter => Pt::new(cx, cy),
            Anchor::RealWest => Pt::new(sw.x, cy),
            Anchor::RealEast => Pt::new(ne.x, cy),
            Anchor::Angle(deg) => {
                let (s, c) = sincos_deg(deg.trunc());
                self.border_local(Pt::new(c, s))
            }
        }
    }

    /// An anchor in picture coordinates.
    #[inline]
    pub(crate) fn anchor(&self, a: Anchor) -> Pt {
        self.m.apply(self.anchor_local(a))
    }

    /// The border point from the centre in direction `v`, in local coordinates.
    fn border_local(&self, v: Pt) -> Pt {
        let (ne, sw) = (self.ne, self.sw);
        let hx = 0.5 * (ne.x - sw.x);
        let cx = self.cx();
        let (cy, hy) = if self.asym {
            (self.ax, if v.y > 0.0 { ne.y - self.ax } else { self.ax - sw.y })
        } else {
            (0.5 * (ne.y + sw.y), 0.5 * (ne.y - sw.y))
        };
        let (x, y) = (v.x.abs(), v.y.abs());
        if x < 1e-9 && y < 1e-9 {
            return Pt::new(cx, cy);
        }
        let (mut px, mut py) = if x * hy >= y * hx { (hx, y * hx / x) } else { (x * hy / y, hy) };
        if v.x < 0.0 {
            px = -px;
        }
        if v.y < 0.0 {
            py = -py;
        }
        Pt::new(cx + px, cy + py)
    }

    /// The border point in direction `v` from the centre, `v` given in the node's frame; picture coordinates.
    #[inline]
    pub(crate) fn border_dir(&self, v: Pt) -> Pt {
        self.m.apply(self.border_local(v))
    }

    /// The border point towards `p`, picture coordinates in and out (spec 6.3).
    pub(crate) fn border_toward(&self, p: Pt) -> Pt {
        let m = &self.m;
        let det = m.a * m.d - m.b * m.c;
        let (dx, dy) = (p.x - m.e, p.y - m.f);
        let lx = (m.d * dx - m.c * dy) / det;
        let ly = (-m.b * dx + m.a * dy) / det;
        let c = self.anchor_local(Anchor::Center);
        self.border_dir(Pt::new(lx - c.x, ly - c.y))
    }
}

/// An arrow end: a node, or a bare coordinate when the path was shifted (spec 8.3).
#[derive(Copy, Clone, Debug)]
pub(crate) enum EndRef {
    Node(Node),
    Coord(Pt),
}

impl EndRef {
    #[inline]
    pub(crate) fn anchor(&self, a: Anchor) -> Pt {
        match self {
            EndRef::Node(n) => n.anchor(a),
            EndRef::Coord(p) => *p,
        }
    }

    #[inline]
    pub(crate) fn border_toward(&self, p: Pt) -> Pt {
        match self {
            EndRef::Node(n) => n.border_toward(p),
            EndRef::Coord(c) => *c,
        }
    }

    #[inline]
    pub(crate) fn is_coord(&self) -> bool {
        matches!(self, EndRef::Coord(_))
    }
}

/// Distance helper shared with the path code.
#[inline]
pub(crate) fn dist(a: Pt, b: Pt) -> f64 {
    hypot(b.x - a.x, b.y - a.y)
}
