//! Arrow tips (spec section 9), as pgflibraryarrows.meta.code.tex computes them.  A tip is built on the stack each time it
//! is drawn: what varies is the line's outer and inner width, and the tables below are a few multiplications away.

use crate::geom::{Pt, Seg, pt};
use crate::pgfmath::{PT, Sp, Tm, Xy, cos, dec, half, mul, reciprocal, sin};
use crate::spec::{ArrowStyle, TipName};

/// The order a tip's setup is written in, around `[ ] 0.0 d` (spec 15).  pgf's definitions differ, and the width
/// appears only under a double line.
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum TipForm {
    /// `<w> w [ ] 0.0 d <cap> J <join> j`: Implies.
    WidthFirst,
    /// `[ ] 0.0 d <cap> J <join> j <w> w`: Computer Modern Rightarrow.
    WidthLast,
    /// `[ ] 0.0 d <w> w <cap> J <join> j`: Bar.
    WidthMiddle,
    /// `[ ] 0.0 d <join> j <cap> J <w> w`: Hooks.
    JoinFirst,
}

#[derive(Copy, Clone, Debug)]
pub(crate) struct Tip {
    pub tip: Sp,
    pub back: Sp,
    pub line: Sp,
    pub sep: Sp,
    /// `\pgfarrowlinewidth`.
    pub lw: Sp,
    /// Whether the body sets its width: it is not the width in force.
    pub own_width: bool,
    pub cap: u8,
    pub join: u8,
    pub swap: bool,
    pub form: TipForm,
    pub nseg: u8,
    pub segs: [Seg; 4],
    pub nhull: u8,
    pub hull: [Xy; 8],
}

impl Tip {
    fn blank() -> Tip {
        Tip {
            tip: 0,
            back: 0,
            line: 0,
            sep: 0,
            lw: 0,
            own_width: false,
            cap: 0,
            join: 0,
            swap: false,
            form: TipForm::WidthLast,
            nseg: 0,
            segs: [Seg::Close; 4],
            nhull: 0,
            hull: [(0, 0); 8],
        }
    }

    #[inline]
    pub(crate) fn segs(&self) -> &[Seg] {
        &self.segs[..usize::from(self.nseg)]
    }

    #[inline]
    pub(crate) fn hull(&self) -> &[Xy] {
        &self.hull[..usize::from(self.nhull)]
    }

    /// `\pgfarrowshullpoint`.
    #[inline]
    fn hull_point(&mut self, x: Sp, y: Sp) {
        self.hull[usize::from(self.nhull)] = (x, y);
        self.nhull += 1;
    }

    /// `\pgfarrowsupperhullpoint`.
    #[inline]
    fn upper(&mut self, x: Sp, y: Sp, harpoon: bool) {
        self.hull_point(x, y);
        if y > 0 && !harpoon {
            self.hull_point(x, -y);
        }
    }
}

/// Up to two tips for one end, in the order they are drawn (spec 9.3).
#[derive(Copy, Clone)]
pub(crate) struct TipSeq {
    pub n: u8,
    pub t: [Tip; 2],
}

impl TipSeq {
    fn none() -> TipSeq {
        TipSeq { n: 0, t: [Tip::blank(), Tip::blank()] }
    }

    fn one(t: Tip) -> TipSeq {
        TipSeq { n: 1, t: [t, Tip::blank()] }
    }

    #[inline]
    pub(crate) fn tips(&self) -> &[Tip] {
        &self.t[..usize::from(self.n)]
    }
}

const MINUS_HALF: Sp = -PT / 2;

/// `\pgfarrowslinewidthdependent a b c` (spec 9.1).  `big_l` is `\pgflinewidth`, `big_i` is `\pgfinnerlinewidth`.
#[inline]
fn dep(a: Sp, b: Sp, c: Sp, big_l: Sp, big_i: Sp) -> Sp {
    if big_i > 0 {
        let mut xa = big_l + mul(c, mul(MINUS_HALF, big_l));
        xa += mul(c, mul(MINUS_HALF, big_i));
        a + mul(b, xa)
    } else {
        a + mul(b, big_l)
    }
}

#[inline]
fn s(x: Sp, y: Sp) -> Pt {
    Pt::new(pt(x), pt(y))
}

/// Computer Modern Rightarrow, with `length` or the default `dep(1.6pt, 2.2, 0)`.
fn tip_cm(l: Sp, i: Sp, length: Sp, reversed: bool, harpoon: bool, sep: Sp, swap: bool) -> Tip {
    const WIDTH: Sp = dec(b"2.096774");
    const A: Sp = dec(b"-0.81731");
    const B: Sp = dec(b".2");
    const C: Sp = dec(b"-0.41019");
    const D: Sp = dec(b"0.05833333");
    let mut t = Tip::blank();
    // `width'`, then `line width`, the late options.
    let width = mul(WIDTH, length);
    let lw = dep(0, PT, PT, l, i);
    t.lw = lw;
    t.own_width = lw != l;
    t.cap = 1;
    t.join = 1;
    t.form = TipForm::WidthLast;
    let a = length - lw;
    let b = width - lw;
    let hl = half(lw);
    t.tip = hl;
    t.back = -a - hl;
    t.line = if reversed && harpoon { hl } else { -hl };
    t.hull_point(hl, hl);
    t.hull_point(hl, -hl);
    t.upper(-a + hl, half(b) + hl, harpoon);
    t.upper(-a - hl, half(b) + hl, harpoon);
    if harpoon {
        t.hull_point(-lw, -hl);
        t.hull_point(-a - hl, half(b) - hl);
    }
    if reversed {
        // `\pgf@arrows@handle@reverse`.
        (t.tip, t.back) = (-t.back, -t.tip);
        t.line = -t.line;
        for h in &mut t.hull[..usize::from(t.nhull)] {
            h.0 = -h.0;
        }
    }
    // `\pgftransformxscale{-1}` when reversed.
    let sx = |x: Sp| if reversed { -x } else { x };
    t.segs[0] = Seg::Move(s(sx(-a), half(b)));
    t.segs[1] = Seg::Curve(s(sx(mul(A, a)), mul(B, b)), s(sx(mul(C, a)), mul(D, b)), s(0, 0));
    if harpoon {
        t.segs[2] = Seg::Line(s(sx(if reversed { hl } else { -lw }), 0));
    } else {
        t.segs[2] = Seg::Curve(s(sx(mul(C, a)), mul(-D, b)), s(sx(mul(A, a)), mul(-B, b)), s(sx(-a), -half(b)));
    }
    t.nseg = 3;
    t.sep = sep;
    t.swap = swap;
    t
}

const LEN_SCALE: Sp = dec(b"6.2");

/// `cm to`: `length=+0pt 6.2`.
fn cm_length(l: Sp, i: Sp) -> Sp {
    dep(0, LEN_SCALE, 0, l, i)
}

/// `/tikz/->`: `length = +1.6pt 2.2`, the default (spec 9.5).
fn plain_length(l: Sp, i: Sp) -> Sp {
    dep(dec(b"1.6"), dec(b"2.2"), 0, l, i)
}

/// `Bar`: `Tee Barb[length=+0pt, width=...]` with its `inset'` nothing.
fn tip_bar(l: Sp, i: Sp, width: Sp, cap: u8) -> Tip {
    let mut t = Tip::blank();
    let lw = dep(0, PT, PT, l, i);
    let hl = half(lw);
    t.lw = lw;
    t.own_width = lw != l;
    t.cap = cap;
    t.join = 0;
    t.tip = hl;
    t.back = -hl;
    t.line = -mul(dec(b".25"), lw);
    t.form = TipForm::WidthMiddle;
    t.upper(hl, half(width), false);
    t.upper(-hl, half(width), false);
    t.segs[0] = Seg::Move(s(0, half(width)));
    t.segs[1] = Seg::Line(s(0, -half(width)));
    t.nseg = 2;
    t
}

/// `\pgfmathparse{1.333333333*tan(.25*<d>)}`.
fn arc_factor(d: Sp) -> Sp {
    const FOUR_THIRDS: Sp = dec(b"1.333333333");
    let q = mul(d, PT / 4);
    let tangent = mul(reciprocal(cos(q)), sin(q));
    mul(tangent, FOUR_THIRDS)
}

/// `\pgf@arc` of radius 1pt under `m`, from `a` to `b`; `last` is `\pgf@path@last`.
fn arc(m: &Tm, a: Sp, b: Sp, last: &mut Xy) -> Seg {
    const QUARTER: Sp = dec(b"0.55228475");
    let delta = (b - a).abs();
    let k = if delta == 90 * PT { QUARTER } else { arc_factor(delta) };
    let r = mul(k, PT);
    let polar = |deg: Sp, rad: Sp| m.apply(mul(cos(deg), rad), mul(sin(deg), rad));
    let up = b > a;
    let p = polar(if up { a + 90 * PT } else { a - 90 * PT }, r);
    let c1 = (last.0 + p.0 - m.x, last.1 + p.1 - m.y);
    let from = polar(a, PT);
    let to = polar(b, PT);
    let e = (last.0 - from.0 + to.0, last.1 - from.1 + to.1);
    let p = polar(if up { b - 90 * PT } else { b + 90 * PT }, r);
    let c2 = (e.0 + p.0 - m.x, e.1 + p.1 - m.y);
    *last = e;
    Seg::Curve(s(c1.0, c1.1), s(c2.0, c2.1), s(e.0, e.1))
}

/// `Hooks`, with what tikz-cd gives it: `harpoon` always, a round cap or a butt cap, and the arc.
fn tip_hooks(l: Sp, i: Sp, width: Sp, length: Sp, arc_deg: Sp, round: bool, swap: bool) -> Tip {
    const P125: Sp = dec(b".125");
    const P375: Sp = dec(b".375");
    let mut t = Tip::blank();
    let lw = dep(0, PT, PT, l, i);
    let hl = half(lw);
    t.lw = lw;
    t.own_width = lw != l;
    t.cap = u8::from(round);
    t.join = 0;
    t.form = TipForm::JoinFirst;
    t.swap = swap;
    t.hull_point(half(length), -hl);
    t.hull_point(length, -hl + mul(P125, width));
    t.upper(length, mul(P375, width), true);
    t.upper(half(length), half(width), true);
    if arc_deg > 180 * PT {
        t.upper(-half(length), half(width), true);
        t.upper(-length, mul(P375, width), true);
        t.upper(-length, mul(PT / 4, width), true);
    } else {
        let x = if round { -hl } else { 0 };
        t.upper(x, mul(PT / 4, width), true);
        t.upper(x, half(width), true);
    }
    // "Adjust width and length: Take line thickness into account."
    let a = length - hl;
    let b = width - lw;
    t.tip = a + hl;
    t.line = 0;
    if arc_deg > 180 * PT {
        t.back = mul(sin(arc_deg), a) - hl;
    } else if round {
        t.back = -hl;
    }
    let start = arc_deg - 90 * PT;
    let mut m = Tm::IDENTITY;
    m.cm(a, 0, 0, PT, 0, 0);
    m.cm(PT, 0, 0, mul(PT / 4, b), 0, 0);
    // `\pgfpointpolar{+\pgfarrowarc}{+1pt}\advance\pgf@y by1pt`.
    let mut last = m.apply(mul(cos(start), PT), mul(sin(start), PT) + PT);
    t.segs[0] = Seg::Move(s(last.0, last.1));
    let mut n = 1;
    // `\pgfpatharc`'s steps.
    let end = -90 * PT;
    let mut at = start;
    while (at - end).abs() > 90 * PT {
        let step = if (at - end).abs() > 115 * PT { 90 * PT } else { 60 * PT };
        t.segs[n] = arc(&m, at, at - step, &mut last);
        n += 1;
        at -= step;
    }
    t.segs[n] = arc(&m, at, end, &mut last);
    t.nseg = n as u8 + 1;
    t
}

fn tip_implies(l: Sp, i: Sp) -> Tip {
    const K206: Sp = dec(b"2.06");
    const K136: Sp = dec(b"-1.36");
    const K14: Sp = dec(b"-1.4");
    const K28: Sp = dec(b"2.8");
    const K06: Sp = dec(b".06");
    const K265: Sp = dec(b"2.65");
    const K75: Sp = dec(b"-0.75");
    const K125: Sp = dec(b"1.25");
    const K05: Sp = dec(b"0.05");
    let mut t = Tip::blank();
    let a = mul(PT / 4, l) + mul(PT / 4, i);
    let b = half(l) + mul(MINUS_HALF, i);
    t.tip = mul(K206, a) + half(b);
    t.back = mul(K136, a) + half(b);
    t.line = 0;
    t.lw = b;
    t.own_width = true;
    t.cap = 1;
    t.join = 1;
    t.form = TipForm::WidthFirst;
    t.upper(mul(K14, a), mul(K28, a), false);
    t.upper(2 * a, half(b), false);
    // `\pgftransformxshift{.06\pgfutil@tempdima}`.
    let dx = mul(K06, a);
    t.segs[0] = Seg::Move(s(mul(K14, a) + dx, mul(K265, a)));
    t.segs[1] = Seg::Curve(s(mul(K75, a) + dx, mul(K125, a)), s(a + dx, mul(K05, a)), s(2 * a + dx, 0));
    t.segs[2] = Seg::Curve(s(a + dx, -mul(K05, a)), s(mul(K75, a) + dx, -mul(K125, a)), s(mul(K14, a) + dx, -mul(K265, a)));
    t.nseg = 3;
    t
}

/// The tips for `name` at outer width `l` and inner width `i`, under an arrow style.
pub(crate) fn tip_seq(name: TipName, style: ArrowStyle, l: Sp, i: Sp) -> TipSeq {
    use TipName::*;
    let hooks_cm = |swap| tip_hooks(l, i, dep(0, dec(b"10.8"), 0, l, i), dep(0, dec(b"3.6"), 0, l, i), 180 * PT, true, swap);
    let hooks_lm = |swap| tip_hooks(l, i, dep(0, 15 * PT, 0, l, i), dep(0, dec(b"4.2"), 0, l, i), 190 * PT, true, swap);
    let hooks_tikz = |swap| {
        // `length = +0.75pt 1.25` and `width' = +0pt 4 -1`.
        let len = dep(dec(b".75"), dec(b"1.25"), 0, l, i);
        tip_hooks(l, i, 4 * len - l, len, 180 * PT, false, swap)
    };
    let to = |harpoon, swap, reversed, sep| tip_cm(l, i, cm_length(l, i), reversed, harpoon, sep, swap);
    let plain = |harpoon, swap, reversed| tip_cm(l, i, plain_length(l, i), reversed, harpoon, 0, swap);
    match (style, name) {
        (_, Cap | ImpliesCap) => TipSeq::none(),

        (ArrowStyle::LatinModern, Bar | ImpliesBar) => TipSeq::one(tip_bar(l, i, dep(0, 12 * PT, dec(b".992"), l, i), 1)),
        (ArrowStyle::LatinModern, LeftHook) => TipSeq::one(hooks_lm(false)),
        (ArrowStyle::LatinModern, RightHook) => TipSeq::one(hooks_lm(true)),

        (ArrowStyle::Tikz, Bar | ImpliesBar) => TipSeq::one(tip_bar(l, i, dep(3 * PT, 4 * PT, dec(b"0.9"), l, i), 0)),
        (ArrowStyle::Tikz, To | PlainTo) => TipSeq::one(plain(false, false, false)),
        (ArrowStyle::Tikz, DoubleTo) => TipSeq { n: 2, t: [plain(false, false, false), plain(false, false, false)] },
        (ArrowStyle::Tikz, LeftHook) => TipSeq::one(hooks_tikz(false)),
        (ArrowStyle::Tikz, RightHook) => TipSeq::one(hooks_tikz(true)),
        // `tikzcd to` is `>` there, and these three are `tikzcd to` with an option.
        (ArrowStyle::Tikz, LeftTo) => TipSeq::one(plain(true, false, false)),
        (ArrowStyle::Tikz, RightTo) => TipSeq::one(plain(true, true, false)),
        (ArrowStyle::Tikz, ToReversed) => TipSeq::one(plain(false, false, true)),

        (_, Bar | ImpliesBar) => TipSeq::one(tip_bar(l, i, dep(0, dec(b"8.2"), dec(b"0.89"), l, i), 1)),
        (_, To) => TipSeq::one(to(false, false, false, 0)),
        (_, PlainTo) => TipSeq::one(plain(false, false, false)),
        (_, Implies) => TipSeq::one(tip_implies(l, i)),
        (_, LeftHook) => TipSeq::one(hooks_cm(false)),
        (_, RightHook) => TipSeq::one(hooks_cm(true)),
        (_, DoubleTo) => TipSeq { n: 2, t: [to(false, false, false, dep(0, dec(b"-2.6"), 0, l, i)), to(false, false, false, 0)] },
        (_, LeftTo) => TipSeq::one(to(true, false, false, 0)),
        (_, RightTo) => TipSeq::one(to(true, true, false, 0)),
        (_, ToReversed) => TipSeq::one(to(false, false, true, 0)),
    }
}
