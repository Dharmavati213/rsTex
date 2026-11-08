//! Shapes (spec 6.2, 6.3): `rectangle` and tikz-cd's `asymmetrical rectangle`, as pgfmoduleshapes.code.tex and
//! tikzlibrarycd.code.tex compute them.

use crate::pgfmath::{PT, Sp, Tm, Xy, border_rectangle, cos, dec, half, mul, sin};
use crate::spec::Anchor;

thread_local! {
    /// [`Node::border_toward`] was asked for the border towards a point within 0.02pt of the node's `center`.
    static CENTRE_RETURNED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Whether that has happened since the last call.  `\pgfpointshapeborder` then writes `Returning node center instead of a point on node border` to the log and the terminal, which only real PGF does.
pub(crate) fn take_centre_returned() -> bool {
    CENTRE_RETURNED.with(|f| f.replace(false))
}

/// A placed node.  Local coordinates are relative to the left end of the text box on its baseline; `m` maps them to
/// picture coordinates.
#[derive(Copy, Clone, Debug)]
pub(crate) struct Node {
    /// `\northeast` and `\southwest`.
    pub ne: Xy,
    pub sw: Xy,
    pub ox: Sp,
    pub oy: Sp,
    pub asym: bool,
    pub ax: Sp,
    pub ex: Sp,
    pub m: Tm,
}

/// Input to [`Node::new`].
pub(crate) struct Shape {
    pub w: Sp,
    pub h: Sp,
    pub d: Sp,
    pub ixs: Sp,
    pub iys: Sp,
    pub ox: Sp,
    pub oy: Sp,
    pub min_w: Sp,
    pub min_h: Sp,
    pub asym: bool,
    pub ax: Sp,
    pub ex: Sp,
}

impl Node {
    /// The saved anchors `\northeast` and `\southwest`.
    pub(crate) fn new(s: &Shape) -> Node {
        let big_w = (s.w + 2 * s.ixs).max(s.min_w);
        let big_h = (s.h + s.d + 2 * s.iys).max(s.min_h);
        Node {
            ne: (half(big_w) + half(s.w) + s.ox, half(big_h) - half(s.d) + half(s.h) + s.oy),
            sw: (-half(big_w) + half(s.w) - s.ox, -half(big_h) - half(s.d) + half(s.h) - s.oy),
            ox: s.ox,
            oy: s.oy,
            asym: s.asym,
            ax: s.ax,
            ex: s.ex,
            m: Tm::IDENTITY,
        }
    }

    /// `\pgf@anchor@rectangle@center`.
    #[inline]
    pub(crate) fn real_center(&self) -> Xy {
        (half(self.ne.0) + half(self.sw.0), half(self.ne.1) + half(self.sw.1))
    }

    /// The anchor `center` of this node's shape.
    #[inline]
    fn center(&self) -> Xy {
        let c = self.real_center();
        if self.asym { (c.0, self.ax) } else { c }
    }

    /// An anchor in local coordinates.
    pub(crate) fn anchor_local(&self, a: Anchor) -> Xy {
        let (ne, sw) = (self.ne, self.sw);
        let c = self.real_center();
        match a {
            Anchor::Center => self.center(),
            Anchor::West => (sw.0, if self.asym { self.ax } else { c.1 }),
            Anchor::East => (ne.0, if self.asym { self.ax } else { c.1 }),
            Anchor::North => (c.0, ne.1),
            Anchor::South => (c.0, sw.1),
            Anchor::NorthEast => ne,
            Anchor::SouthWest => sw,
            Anchor::NorthWest => (sw.0, ne.1),
            Anchor::SouthEast => (ne.0, sw.1),
            Anchor::Base => (c.0, 0),
            Anchor::BaseWest => (sw.0, 0),
            Anchor::BaseEast => (ne.0, 0),
            Anchor::Mid => (c.0, half(self.ex)),
            Anchor::MidWest => (sw.0, half(self.ex)),
            Anchor::MidEast => (ne.0, half(self.ex)),
            Anchor::Text => (0, 0),
            Anchor::RealCenter => c,
            Anchor::RealWest => (sw.0, c.1),
            Anchor::RealEast => (ne.0, c.1),
            Anchor::Angle(deg) => {
                // `\pgfqpointpolar{<count>}{1pt}`.
                let d = deg * PT;
                self.border_local(mul(cos(d), PT), mul(sin(d), PT))
            }
        }
    }

    /// `\pgfpointanchor`, the picture's transformation being a translation that cancels out.
    #[inline]
    pub(crate) fn anchor(&self, a: Anchor) -> Xy {
        let (x, y) = self.anchor_local(a);
        self.m.apply(x, y)
    }

    /// The shape's `\anchorborder`.
    fn border_local(&self, xb: Sp, yb: Sp) -> Xy {
        let (ne, sw) = (self.ne, self.sw);
        let hx = half(ne.0 - sw.0);
        let hy = half(ne.1 - sw.1);
        let xa = sw.0 + hx;
        let ya = sw.1 + hy;
        if self.asym {
            let d = self.ax;
            let yc = if yb > 0 { ne.1 - d } else { -sw.1 + d };
            let (x, y) = border_rectangle(xb, yb, hx, yc);
            (x + xa, y + d)
        } else {
            let (x, y) = border_rectangle(xb, yb, hx, hy);
            (x + xa, y + ya)
        }
    }

    /// `\pgfpointshapeborder`, picture coordinates in and out.
    pub(crate) fn border_toward(&self, p: Xy) -> Xy {
        const NEAR: Sp = dec(b"0.04");
        let (mut xa, mut ya) = self.m.invert().apply(p.0, p.1);
        let c = self.center();
        xa -= c.0;
        ya -= c.1;
        let xb = if xa < PT && xa > -PT { 10 * xa } else { 10 * PT };
        let yb = if ya < PT && ya > -PT { 10 * ya } else { 10 * PT };
        let (x, y) = if mul(xb, xb) + mul(yb, yb) < NEAR {
            // The branch with `\pgfwarning`.
            CENTRE_RETURNED.with(|f| f.set(true));
            c
        } else {
            self.border_local(xa, ya)
        };
        self.m.apply(x, y)
    }
}

/// An arrow end: a node, or a bare coordinate when the path was shifted (spec 8.3).
#[derive(Copy, Clone, Debug)]
pub(crate) enum EndRef {
    Node(Node),
    Coord(Xy),
}

impl EndRef {
    #[inline]
    pub(crate) fn anchor(&self, a: Anchor) -> Xy {
        match self {
            EndRef::Node(n) => n.anchor(a),
            EndRef::Coord(p) => *p,
        }
    }

    #[inline]
    pub(crate) fn border_toward(&self, p: Xy) -> Xy {
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
