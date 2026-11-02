//! Arrow paths (spec section 8) and the squiggly decoration (spec 10.3).

use crate::fmath::{floor_div, norm, round5, rot, sincos_deg};
use crate::geom::{Pt, bez};
use crate::node::{EndRef, dist};
use crate::spec::{Anchor, Curve};

/// A path: straight when `c` is `None`.
#[derive(Copy, Clone, Debug)]
pub(crate) struct ArrowPath {
    pub s: Pt,
    pub c: Option<(Pt, Pt)>,
    pub e: Pt,
}

/// `\tikz@to@compute@distance@main` up to the factor .3915, with pgf's integer quantisation (spec 8.2).
fn control_length(s: Pt, e: Pt) -> f64 {
    let (dx, dy) = ((e.x - s.x).abs(), (e.y - s.y).abs());
    if dx == 0.0 && dy == 0.0 {
        return 0.0;
    }
    let n = norm(Pt::new(dx, dy));
    let (nx, ny) = (round5(n.x), round5(n.y));
    if nx > ny {
        let c = (nx * 65536.0) as i64 / 255;
        if c == 0 { dx } else { dx * 256.0 / c as f64 }
    } else {
        let c = (ny * 65536.0) as i64 / 255;
        if c == 0 { dy } else { dy * 256.0 / c as f64 }
    }
}

/// The control distances for `cv` between `s` and `e`.
fn control_distances(cv: &Curve, s: Pt, e: Pt) -> (f64, f64) {
    let d = control_length(s, e) * 0.3915;
    ((d * cv.l_out).max(cv.min).min(cv.max), (d * cv.l_in).max(cv.min).min(cv.max))
}

/// Spec 8.1 and 8.2: the path between two ends.
pub(crate) fn base_path(st: &EndRef, en: &EndRef, sa: Option<Anchor>, ea: Option<Anchor>, curve: Option<&Curve>) -> ArrowPath {
    let Some(cv) = curve else {
        let sp = sa.map(|a| st.anchor(a));
        let e = match ea {
            Some(a) => en.anchor(a),
            None => en.border_toward(sp.unwrap_or_else(|| st.anchor(Anchor::Center))),
        };
        let s = sp.unwrap_or_else(|| st.border_toward(e));
        return ArrowPath { s, c: None, e };
    };
    if cv.relative {
        let a0 = st.anchor(sa.unwrap_or(Anchor::Center));
        let b0 = en.anchor(ea.unwrap_or(Anchor::Center));
        let s = if sa.is_some() || st.is_coord() {
            a0
        } else {
            st.border_toward(a0.add(rot(b0.sub(a0), cv.out)))
        };
        let e = if ea.is_some() || en.is_coord() {
            b0
        } else {
            en.border_toward(b0.add(rot(a0.sub(b0), 180.0 + cv.inn)))
        };
        let (xa, xb) = control_distances(cv, s, e);
        let u = norm(e.sub(s));
        let c1 = s.along(xa, rot(u, cv.out));
        let c2 = e.along(xb, rot(u, cv.inn));
        ArrowPath { s, c: Some((c1, c2)), e }
    } else {
        let a0 = st.anchor(sa.unwrap_or(Anchor::Angle(cv.out)));
        let b0 = en.anchor(ea.unwrap_or(Anchor::Angle(cv.inn)));
        let (xa, xb) = control_distances(cv, a0, b0);
        let (so, co) = sincos_deg(cv.out);
        let (si, ci) = sincos_deg(cv.inn);
        let c1 = a0.along(xa, Pt::new(co, so));
        let c2 = b0.along(xb, Pt::new(ci, si));
        let s = if sa.is_some() { a0 } else { st.border_toward(c1) };
        let e = if ea.is_some() { b0 } else { en.border_toward(c2) };
        ArrowPath { s, c: Some((c1, c2)), e }
    }
}

/// Steps of the arc-length walk along a curve.
const ARC_STEPS: u32 = 2000;

/// Walks a Bezier by arc length without a table.  Positions are requested in increasing order; a smaller one restarts.
struct ArcWalk {
    p: [Pt; 4],
    i: u32,
    acc: f64,
    /// Length before the last step: at or past a request means the walk has gone too far for it.
    before: f64,
    prev: Pt,
}

impl ArcWalk {
    fn new(p: [Pt; 4]) -> ArcWalk {
        ArcWalk { p, i: 0, acc: 0.0, before: 0.0, prev: p[0] }
    }

    fn step(&mut self) {
        self.i += 1;
        let t = f64::from(self.i) / f64::from(ARC_STEPS);
        let q = bez(t, self.p[0], self.p[1], self.p[2], self.p[3]).0;
        self.before = self.acc;
        self.acc += dist(self.prev, q);
        self.prev = q;
    }

    fn total(p: [Pt; 4]) -> f64 {
        let mut w = ArcWalk::new(p);
        while w.i < ARC_STEPS {
            w.step();
        }
        w.acc
    }

    /// Position and unit tangent at arc length `x`, at the first step at or past it.
    fn at(&mut self, x: f64) -> (Pt, Pt) {
        if self.i > 0 && self.before >= x {
            *self = ArcWalk::new(self.p);
        }
        while self.acc < x && self.i < ARC_STEPS {
            self.step();
        }
        let t = f64::from(self.i) / f64::from(ARC_STEPS);
        let (q, d, e) = bez(t, self.p[0], self.p[1], self.p[2], self.p[3]);
        (q, norm(e.sub(d)))
    }
}

/// More zigzag points than this means an absurd line width or distance; the caller falls back.
pub(crate) const MAX_ZIGZAG: f64 = 8192.0;

pub(crate) enum Zigzag {
    /// The decoration collapses; draw the plain path.
    None,
    /// `out` holds the path's points: move to the first, lines to the rest.
    Points,
    /// Too many points.
    TooDense,
}

/// The decorated points (spec 10.3).  pgf evaluates `segment length` and `amplitude` when the `squiggly` key runs, so they
/// scale with the line width `key_lw` at that time; `pre length` and `post length` when it decorates, so with the final
/// `lw`.  `squiggly,thick` shows both at once.
pub(crate) fn zigzag(path: &ArrowPath, key_lw: f64, lw: f64, pre: Option<f64>, post: Option<f64>, out: &mut Vec<Pt>) -> Zigzag {
    out.clear();
    let seg = 9.25 * key_lw;
    let amp = 1.9 * key_lw;
    let pre = pre.unwrap_or(6.0 * lw);
    let post = post.unwrap_or(6.0 * lw);
    let hs = 0.5 * seg;
    if hs.is_nan() || hs <= 0.0 {
        return Zigzag::None;
    }
    let (s, e) = (path.s, path.e);
    let total = match path.c {
        None => dist(s, e),
        Some((c1, c2)) => ArcWalk::total([s, c1, c2, e]),
    };
    let m = total - pre - post;
    let k = if m >= hs { floor_div(m, hs) } else { 0.0 };
    if k.is_nan() || k > MAX_ZIGZAG {
        return Zigzag::TooDense;
    }
    if total < 8.0 * lw && path.c.is_none() {
        return Zigzag::None;
    }
    let k = k as u32;
    let u = norm(e.sub(s));
    let mut walk = path.c.map(|(c1, c2)| ArcWalk::new([s, c1, c2, e]));
    let mut at = |x: f64| match &mut walk {
        None => (s.along(x, u), u),
        Some(w) => w.at(x),
    };

    out.push(s);
    out.push(at(pre).0);
    let mut x0 = pre;
    let mut sign = 1.0;
    for _ in 0..k {
        // The point is (s/4, ±amp) in the frame at x0.
        let (p, t) = at(x0);
        let dx = 0.25 * seg;
        let y = sign * amp;
        out.push(Pt::new(p.x + dx * t.x - y * t.y, p.y + dx * t.y + y * t.x));
        x0 += hs;
        sign = -sign;
    }
    if k > 0 {
        out.push(at(x0).0);
    }
    // A straight input repeats the current point; a curved one has the true end of the main section.
    if path.c.is_none() {
        out.push(at(x0).0);
    } else {
        out.push(at(total - post).0);
    }
    out.push(e);
    Zigzag::Points
}
