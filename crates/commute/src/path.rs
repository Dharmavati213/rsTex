//! Arrow paths (spec section 8), as tikz.code.tex and tikzlibrarytopaths.code.tex build them, and the squiggly decoration
//! (spec 10.3), as pgfmoduledecorations.code.tex runs its automaton along them.

use crate::node::EndRef;
use crate::pgfmath::{PT, Sp, Tm, Xy, atan2, cos, curve_at_time, dec, half, mul, normalised, sin, veclen};
use crate::spec::{Anchor, Curve, QuiverCurve};

/// A path: straight when `c` is `None`.
#[derive(Copy, Clone, Debug)]
pub(crate) struct ArrowPath {
    pub s: Xy,
    pub c: Option<(Xy, Xy)>,
    pub e: Xy,
}

/// `\tikz@to@compute@distance@main`: the distances of the two control points.
fn control_distances(cv: &Curve, a: Xy, b: Xy) -> (Sp, Sp) {
    const K: Sp = dec(b"0.3915");
    let (mut xa, mut ya) = ((b.0 - a.0).abs(), (b.1 - a.1).abs());
    let (x, y) = normalised(xa, ya);
    if x > y {
        if x != 0 {
            xa = 16 * (16 * xa / (x / 255));
        }
    } else if y != 0 {
        ya = 16 * ya / (y / 255);
        xa = 16 * ya;
    }
    let d = mul(K, xa);
    let out = mul(cv.l_out, d).max(cv.min_out).min(cv.max_out);
    let inn = mul(cv.l_in, d).max(cv.min_in).min(cv.max_in);
    (out, inn)
}

/// `\pgfpointpolar` of one radius.
#[inline]
fn polar(deg: Sp, r: Sp) -> Xy {
    (mul(cos(deg), r), mul(sin(deg), r))
}

/// Spec 8.1 and 8.2: the path between two ends.  Everything runs in picture coordinates: a translation of the arrow's
/// scope goes out on reading a node and comes back in on writing the path, without rounding.
pub(crate) fn base_path(st: &EndRef, en: &EndRef, sa: Option<Anchor>, ea: Option<Anchor>, curve: Option<&Curve>) -> ArrowPath {
    let Some(cv) = curve else {
        // `\tikz@@lineto`.
        let sp = sa.map(|a| st.anchor(a));
        let e = match ea {
            Some(a) => en.anchor(a),
            None => en.border_toward(sp.unwrap_or_else(|| st.anchor(Anchor::Center))),
        };
        let s = sp.unwrap_or_else(|| st.border_toward(e));
        return ArrowPath { s, c: None, e };
    };
    if cv.relative {
        relative_path(st, en, sa, ea, cv)
    } else {
        // `\tikz@to@compute`: the anchors at the angles, or the point given; then `\tikz@curveC`.
        let a0 = st.anchor(sa.unwrap_or(Anchor::Angle(cv.out / PT)));
        let b0 = en.anchor(ea.unwrap_or(Anchor::Angle(cv.inn / PT)));
        let (xa, xb) = control_distances(cv, a0, b0);
        let oc = polar(cv.out, xa);
        let ic = polar(cv.inn, xb);
        let c1 = (a0.0 + oc.0, a0.1 + oc.1);
        let c2 = (b0.0 + ic.0, b0.1 + ic.1);
        let s = if sa.is_some() { a0 } else { st.border_toward(c1) };
        let e = if ea.is_some() { b0 } else { en.border_toward(c2) };
        ArrowPath { s, c: Some((c1, c2)), e }
    }
}

/// `\tikz@@@to@compute@relative`.
fn relative_path(st: &EndRef, en: &EndRef, sa: Option<Anchor>, ea: Option<Anchor>, cv: &Curve) -> ArrowPath {
    let a0 = st.anchor(sa.unwrap_or(Anchor::Center));
    let b0 = en.anchor(ea.unwrap_or(Anchor::Center));
    let rotated = |about: Xy, turn: &[Sp], p: Xy| {
        let mut t = Tm::IDENTITY;
        t.shift(about.0, about.1);
        for &d in turn {
            t.rotate(d);
        }
        t.shift(-about.0, -about.1);
        t.apply(p.0, p.1)
    };
    let s = if sa.is_some() || st.is_coord() { a0 } else { st.border_toward(rotated(a0, &[cv.out], b0)) };
    let e = if ea.is_some() || en.is_coord() { b0 } else { en.border_toward(rotated(b0, &[180 * PT, cv.inn], a0)) };
    let (first, second) = control_distances(cv, s, e);
    let (xc, yc) = normalised(e.0 - s.0, e.1 - s.1);
    let control = |from: Xy, deg: Sp, dist: Sp| {
        let mut t = Tm::IDENTITY;
        t.shift(from.0, from.1);
        t.cm(xc, yc, -yc, xc, 0, 0);
        let p = polar(deg, dist);
        t.apply(p.0, p.1)
    };
    ArrowPath { s, c: Some((control(s, cv.out, first), control(e, cv.inn, second))), e }
}

/// `curve=` of quiver.sty (spec 8.5), whose `to path` has `($(a)!f!(b)!h!270:(b)$)` as the control points.
pub(crate) fn quiver_path(st: &EndRef, en: &EndRef, sa: Option<Anchor>, ea: Option<Anchor>, q: &QuiverCurve) -> ArrowPath {
    let a = st.anchor(sa.unwrap_or(Anchor::Center));
    let b = en.anchor(ea.unwrap_or(Anchor::Center));
    let control = |f: Sp| {
        // `\tikz@cc@after@num`.
        let g = PT - f;
        let (x, y) = (mul(g, a.0) + mul(f, b.0), mul(g, a.1) + mul(f, b.1));
        // `\tikz@cc@handle@rot`.
        let mut t = Tm::IDENTITY;
        t.shift(x, y);
        t.rotate(270 * PT);
        t.shift(-x, -y);
        let r = t.apply(b.0, b.1);
        let h = q.height.v;
        if q.height.units {
            // `\tikz@cc@after@unit`.
            let (nx, ny) = normalised(r.0 - x, r.1 - y);
            (x + mul(h, nx), y + mul(h, ny))
        } else {
            let g = PT - h;
            (mul(g, x) + mul(h, r.0), mul(g, y) + mul(h, r.1))
        }
    };
    let (c1, c2) = (control(q.pos), control(PT - q.pos));
    let s = if sa.is_some() { a } else { st.border_toward(c1) };
    let e = if ea.is_some() { b } else { en.border_toward(c2) };
    ArrowPath { s, c: Some((c1, c2)), e }
}

/// `\pgfpointlineatdistance`.
pub(crate) fn line_at_distance(d: Sp, p: Xy, r: Xy) -> Xy {
    if d == 0 {
        return p;
    }
    let (x, y) = normalised(r.0 - p.0, r.1 - p.1);
    (p.0 + mul(x, d), p.1 + mul(y, d))
}

/// `\pgfmathanglebetweenpoints`.
fn angle_between(a: Xy, b: Xy) -> Sp {
    let t = atan2(b.1 - a.1, b.0 - a.0);
    if t < 0 { t + 360 * PT } else { t }
}

/// One half of `\pgf@decorate@recursive@subdividecurve@left` and `right`, then `\pgf@decorate@@curvelength`.
fn curve_length_part(p: [Xy; 4], depth: u32) -> Sp {
    let (dx, dy) = ((p[3].0 - p[0].0).abs(), (p[3].1 - p[0].1).abs());
    if (dx < PT && dy < PT) || depth > 32 {
        return veclen(dx, dy);
    }
    let (l, r) = subdivide(p);
    curve_length_part(l, depth + 1) + curve_length_part(r, depth + 1)
}

fn subdivide(p: [Xy; 4]) -> ([Xy; 4], [Xy; 4]) {
    const H: Sp = PT / 2;
    const Q: Sp = PT / 4;
    const E: Sp = PT / 8;
    const T: Sp = dec(b".375");
    let f = |k: [Sp; 4], g: fn(Xy) -> Sp| mul(k[0], g(p[0])) + mul(k[1], g(p[1])) + mul(k[2], g(p[2])) + mul(k[3], g(p[3]));
    let x = |q: Xy| q.0;
    let y = |q: Xy| q.1;
    let left = [
        p[0],
        (f([H, H, 0, 0], x), f([H, H, 0, 0], y)),
        (f([Q, H, Q, 0], x), f([Q, H, Q, 0], y)),
        (f([E, T, T, E], x), f([E, T, T, E], y)),
    ];
    let right = [
        (f([E, T, T, E], x), f([E, T, T, E], y)),
        (f([0, Q, H, Q], x), f([0, Q, H, Q], y)),
        (f([0, 0, H, H], x), f([0, 0, H, H], y)),
        p[3],
    ];
    (left, right)
}

/// `\pgf@decorate@curvelength`: it always begins with a subdivision.
fn curve_length(p: [Xy; 4]) -> Sp {
    let (l, r) = subdivide(p);
    curve_length_part(l, 0) + curve_length_part(r, 0)
}

/// More zigzag points than this means an absurd line width or distance; the caller falls back.
pub(crate) const MAX_ZIGZAG: Sp = 8192;

pub(crate) enum Zigzag {
    /// `out` holds the path's points: move to the first, lines to the rest.
    Points,
    /// Too many points.
    TooDense,
    /// A curve too short to follow, on which real PGF may stop with `Dimension too large`.
    Lost,
}

/// How far past the curve's end, at 1, the search for a distance along it may go.  Rounding takes it a step past, and a step is at most 1/32.
const TIME_LOST: Sp = dec(b"1.25");

/// The state of the automaton along the one input segment an arrow has.
struct Walk {
    /// The search for a distance went off the curve: `Zigzag::Lost`.
    lost: bool,
    p: [Xy; 4],
    curved: bool,
    length: Sp,
    /// `\pgf@decorate@currentpathlength`: the section's.
    section: Sp,
    /// `\pgfdecoratedinputsegmentcompleteddistance` and `remaining`.
    in_done: Sp,
    in_left: Sp,
    /// The `\pgfdecoratedremainingdistance` of the current section.
    left: Sp,
    time: Sp,
    angle: Sp,
    /// `\pgfpoint@decorated@pathlast`.
    last: Xy,
}

impl Walk {
    /// `\pgf@decorate@transformtoinputsegment`.
    fn frame(&self) -> Tm {
        let mut t = Tm::IDENTITY;
        if self.curved {
            let (at, a, b) = curve_at_time(self.time, self.p[0], self.p[1], self.p[2], self.p[3]);
            t.shift(at.0, at.1);
            let (x, y) = normalised(a.0 - b.0, a.1 - b.1);
            t.cm(x, y, -y, x, 0, 0);
        } else {
            let q = line_at_distance(self.in_done, self.p[0], self.p[3]);
            t.shift(q.0, q.1);
            t.rotate(self.angle);
        }
        t
    }

    /// `\pgf@decorate@movealonginputsegment@curve`.
    fn advance_time(&mut self, d: Sp) {
        if !self.curved || d == 0 {
            return;
        }
        let at = |s: Sp| curve_at_time(s, self.p[0], self.p[1], self.p[2], self.p[3]).0;
        let mut a = at(self.time);
        let mut xb = self.time;
        let mut yb = if self.section < 128 * PT {
            dec(b".03125")
        } else if self.section < 512 * PT {
            dec(b".015625")
        } else if self.section < 2048 * PT {
            dec(b".00390625")
        } else {
            dec(b".0009765625")
        };
        let mut count: Sp = 1;
        let mut total = 0;
        loop {
            xb += count * yb;
            // On a curve so short that the steps of the search come to less than the distance, the time runs off the curve, past where the sums of real PGF hold.
            if xb > TIME_LOST {
                self.lost = true;
                break;
            }
            let q = at(xb);
            let step = veclen(q.0 - a.0, q.1 - a.1);
            a = q;
            total += count * step;
            if count > 0 {
                if total > d {
                    count = -count;
                    yb = half(yb);
                }
            } else if total < d {
                count = -count;
                yb = half(yb);
            }
            if yb == 0 {
                break;
            }
        }
        self.time = xb;
    }

    /// `\pgf@decorate@processnextinputsegmentobject` finds the end of the input.
    fn input_over(&mut self) {
        self.in_done = 0;
        self.in_left = self.length;
        self.left = 0;
    }

    /// `\pgf@decorate@movealongpath`.
    fn move_along(&mut self, d: Sp) {
        self.left -= d;
        self.in_done += d;
        self.in_left -= d;
        if self.in_left > 0 {
            self.advance_time(d);
            // "Hacking to control some inaccuracies."
            if self.time > PT {
                let t = self.time;
                self.input_over();
                self.time = t - PT;
            }
        } else {
            self.input_over();
            self.time = 0;
        }
    }

    /// The end of the section: `\pgf@decorate@invoke` after the run, up to the final state's code.  The move along the input
    /// is inside a group there.
    fn set_last(&mut self, global_left: Sp, end: Xy) {
        if global_left < PT {
            self.last = end;
        } else {
            let time = self.time;
            self.advance_time(self.left);
            let t = self.frame();
            self.last = (t.x, t.y);
            self.time = time;
        }
    }

    /// `\pgfpathlineto{\pgfpointdecoratedpathlast}` in the frame: the point goes through the inverse and back.
    fn line_to_last(&self, out: &mut Vec<Xy>) {
        let t = self.frame();
        let q = t.invert().apply(self.last.0, self.last.1);
        out.push(t.apply(q.0, q.1));
    }

    /// The decoration `lineto` over a section of `width`.
    fn lineto_section(&mut self, width: Sp, global_left: &mut Sp, end: Xy, out: &mut Vec<Xy>) {
        let w = width.min(*global_left);
        *global_left -= w;
        self.left = w;
        self.section = w;
        // State `initial`: `switch if input segment less than` to `input segment`, or `width=\pgfdecoratedremainingdistance`.
        loop {
            if self.in_left < self.left {
                // State `input segment`, of the input's width: when less is left, `final`.
                if self.left == 0 || self.left < self.length {
                    break;
                }
                let t = self.frame();
                let q = t.invert().apply(self.p[3].0, self.p[3].1);
                out.push(t.apply(q.0, q.1));
                self.move_along(self.length);
                continue;
            }
            if self.left == 0 {
                break;
            }
            self.move_along(self.left);
        }
        self.set_last(*global_left, end);
        self.line_to_last(out);
        self.advance_time(self.left);
    }

    /// The decoration `zigzag` over a section of `width`.
    fn zigzag_section(&mut self, width: Sp, seg: Sp, amp: Sp, global_left: &mut Sp, end: Xy, out: &mut Vec<Xy>) {
        let w = width.min(*global_left);
        *global_left -= w;
        self.left = w;
        self.section = w;
        let hs = half(seg);
        let qs = mul(PT / 4, seg);
        // `up from center` then, in turn, `big down` and `big up`, each of width `.5 segment length`.  `up from center`
        // ends in `final` if less is left, the others in `center finish`.
        let mut up = true;
        let mut first = true;
        loop {
            if first {
                if self.left == 0 || self.left < hs {
                    break;
                }
                first = false;
            } else if self.left == 0 || self.left < hs {
                // `center finish` has width 0pt, whose test goes to `final` when nothing is left.
                if self.left != 0 {
                    let t = self.frame();
                    out.push(t.apply(0, 0));
                    self.move_along(0);
                }
                break;
            }
            let t = self.frame();
            out.push(t.apply(qs, if up { amp } else { -amp }));
            self.move_along(hs);
            up = !up;
        }
        self.set_last(*global_left, end);
        self.line_to_last(out);
        self.advance_time(self.left);
    }
}

/// The decorated points (spec 10.3).  pgf evaluates `segment length` and `amplitude` when the `squiggly` key runs, and
/// `pre length` and `post length` when it decorates.
pub(crate) fn zigzag(path: &ArrowPath, seg: Sp, amp: Sp, pre: Sp, post: Sp, out: &mut Vec<Xy>) -> Zigzag {
    out.clear();
    let (s, e) = (path.s, path.e);
    let (p, curved, length) = match path.c {
        None => ([s, s, e, e], false, veclen(e.0 - s.0, e.1 - s.1)),
        Some((c1, c2)) => {
            let p = [s, c1, c2, e];
            (p, true, curve_length(p))
        }
    };
    let hs = half(seg);
    if hs <= 0 {
        return Zigzag::TooDense;
    }
    if length / hs > MAX_ZIGZAG {
        return Zigzag::TooDense;
    }
    // `\pgf@decorate@processnextinputsegmentobject`, and the hack for a path that goes nowhere.
    let in_len = if length == 0 { dec(b"0.05") } else { length };
    let angle = if curved { angle_between(p[0], p[1]) } else { angle_between(p[0], p[3]) };
    let mut w = Walk {
        lost: false,
        p,
        curved,
        length: in_len,
        section: length,
        in_done: 0,
        in_left: in_len,
        left: length,
        time: 0,
        angle,
        last: e,
    };
    out.push(s);
    let mut global = length;
    // The meta-decoration `tikz@internal`.  Each state's `width` key goes to `final` if less is left.
    if global >= pre {
        w.lineto_section(pre, &mut global, e, out);
        let main = global - post;
        if global >= main {
            w.zigzag_section(main, seg, amp, &mut global, e, out);
        }
    }
    let rest = global;
    w.lineto_section(rest, &mut global, e, out);
    if w.lost { Zigzag::Lost } else { Zigzag::Points }
}
