//! Arrow tips (spec section 9).  A tip is built on the stack each time it is drawn: what varies is the line's outer and
//! inner width, and the tables below are a few multiplications away.

use crate::fmath::{sincos_deg, tan_deg};
use crate::geom::{Pt, Seg};
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
    pub tip: f64,
    pub back: f64,
    pub line: f64,
    pub sep: f64,
    pub lw: f64,
    pub cap: u8,
    pub join: u8,
    pub swap: bool,
    pub form: TipForm,
    pub nseg: u8,
    pub segs: [Seg; 4],
    pub nhull: u8,
    pub hull: [Pt; 8],
}

impl Tip {
    fn blank() -> Tip {
        Tip {
            tip: 0.0,
            back: 0.0,
            line: 0.0,
            sep: 0.0,
            lw: 0.0,
            cap: 0,
            join: 0,
            swap: false,
            form: TipForm::WidthLast,
            nseg: 0,
            segs: [Seg::Close; 4],
            nhull: 0,
            hull: [Pt::ZERO; 8],
        }
    }

    #[inline]
    pub(crate) fn segs(&self) -> &[Seg] {
        &self.segs[..usize::from(self.nseg)]
    }

    #[inline]
    pub(crate) fn hull(&self) -> &[Pt] {
        &self.hull[..usize::from(self.nhull)]
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

/// `\pgfarrowslinewidthdependent a b c` (spec 9.1).
#[inline]
fn dep(a: f64, b: f64, c: f64, big_l: f64, big_i: f64) -> f64 {
    if big_i > 0.0 { a + b * ((1.0 - 0.5 * c) * big_l - 0.5 * c * big_i) } else { a + b * big_l }
}

const fn p(x: f64, y: f64) -> Pt {
    Pt::new(x, y)
}

/// Computer Modern Rightarrow, with `length` or the default `dep(0, 6.2, 0)`.
fn tip_cm(l: f64, i: f64, length: Option<f64>, reversed: bool, harpoon: bool, sep: f64, swap: bool) -> Tip {
    let mut t = Tip::blank();
    let len = length.unwrap_or_else(|| dep(0.0, 6.2, 0.0, l, i));
    let width = 2.096774 * len;
    let lw = dep(0.0, 1.0, 1.0, l, i);
    let a = len - lw;
    let b = width - lw;
    t.lw = lw;
    t.cap = 1;
    t.join = 1;
    let mut tip = 0.5 * lw;
    let mut back = -a - 0.5 * lw;
    let mut line = if reversed && harpoon { 0.5 * lw } else { -0.5 * lw };
    let mut sx = 1.0;
    if reversed {
        sx = -1.0;
        (tip, back) = (-back, -tip);
        line = -line;
    }
    t.tip = tip;
    t.back = back;
    t.line = line;
    t.segs[0] = Seg::Move(p(sx * -a, 0.5 * b));
    t.segs[1] = Seg::Curve(
        p(sx * -0.81731 * a, 0.2 * b),
        p(sx * -0.41019 * a, 0.05833333 * b),
        p(0.0, 0.0),
    );
    if harpoon {
        t.segs[2] = Seg::Line(p(sx * (if reversed { 0.5 } else { -1.0 }) * lw, 0.0));
    } else {
        t.segs[2] = Seg::Curve(
            p(sx * -0.41019 * a, -0.05833333 * b),
            p(sx * -0.81731 * a, -0.2 * b),
            p(sx * -a, -0.5 * b),
        );
    }
    t.nseg = 3;
    let mut hull = [
        p(0.5 * lw, 0.5 * lw),
        p(0.5 * lw, -0.5 * lw),
        p(-a + 0.5 * lw, 0.5 * b + 0.5 * lw),
        p(-a - 0.5 * lw, 0.5 * b + 0.5 * lw),
        Pt::ZERO,
        Pt::ZERO,
    ];
    if harpoon {
        hull[4] = p(-lw, -0.5 * lw);
        hull[5] = p(-a - 0.5 * lw, 0.5 * b - 0.5 * lw);
    } else {
        hull[4] = p(-a + 0.5 * lw, -(0.5 * b + 0.5 * lw));
        hull[5] = p(-a - 0.5 * lw, -(0.5 * b + 0.5 * lw));
    }
    for (slot, h) in t.hull.iter_mut().zip(hull) {
        *slot = p(sx * h.x, h.y);
    }
    t.nhull = 6;
    t.sep = sep;
    t.swap = swap;
    t
}

/// `/tikz/->`: `len = dep(1.6pt, 2.2, 0)` (spec 9.5).
fn tip_plain(l: f64, i: f64) -> Tip {
    tip_cm(l, i, Some(dep(1.6, 2.2, 0.0, l, i)), false, false, 0.0, false)
}

fn tip_bar(l: f64, i: f64, a: f64, wc: f64, c: f64, cap: u8) -> Tip {
    let mut t = Tip::blank();
    let width = dep(a, wc, c, l, i);
    let lw = dep(0.0, 1.0, 1.0, l, i);
    t.lw = lw;
    t.cap = cap;
    t.join = 0;
    t.tip = 0.5 * lw;
    t.back = -0.5 * lw;
    t.line = -0.25 * lw;
    t.form = TipForm::WidthMiddle;
    t.segs[0] = Seg::Move(p(0.0, 0.5 * width));
    t.segs[1] = Seg::Line(p(0.0, -0.5 * width));
    t.nseg = 2;
    t.hull[0] = p(0.5 * lw, 0.5 * width);
    t.hull[1] = p(-0.5 * lw, 0.5 * width);
    t.hull[2] = p(0.5 * lw, -0.5 * width);
    t.hull[3] = p(-0.5 * lw, -0.5 * width);
    t.nhull = 4;
    t
}

/// Two quarter ellipses, with pgf's arc constant (spec 9.2).
fn hook_segs(t: &mut Tip, a: f64, ry: f64) {
    const K: f64 = 0.55228;
    t.segs[0] = Seg::Move(p(0.0, 2.0 * ry));
    t.segs[1] = Seg::Curve(p(K * a, 2.0 * ry), p(a, ry + K * ry), p(a, ry));
    t.segs[2] = Seg::Curve(p(a, ry - K * ry), p(K * a, 0.0), p(0.0, 0.0));
    t.nseg = 3;
}

fn tip_hook(l: f64, i: f64, swap: bool) -> Tip {
    let mut t = Tip::blank();
    let len = dep(0.0, 3.6, 0.0, l, i);
    let width = dep(0.0, 10.8, 0.0, l, i);
    let lw = dep(0.0, 1.0, 1.0, l, i);
    t.lw = lw;
    t.cap = 1;
    t.join = 0;
    t.form = TipForm::JoinFirst;
    let a = len - 0.5 * lw;
    let b = width - lw;
    t.tip = a + 0.5 * lw;
    t.back = -0.5 * lw;
    t.line = 0.0;
    hook_segs(&mut t, a, 0.25 * b);
    let hull = [
        p(0.5 * len, -0.5 * lw),
        p(len, -0.5 * lw + 0.125 * width),
        p(len, 0.375 * width),
        p(0.5 * len, 0.5 * width),
        p(-0.5 * lw, 0.25 * width),
        p(-0.5 * lw, 0.5 * width),
    ];
    t.hull[..6].copy_from_slice(&hull);
    t.nhull = 6;
    t.swap = swap;
    t
}

/// `Hooks[width=+0pt 15,length=+0pt 4.2,arc=190,harpoon,line cap=round]` (spec 9.6).
fn tip_hook_latin_modern(l: f64, i: f64, swap: bool) -> Tip {
    let mut t = Tip::blank();
    let lw = dep(0.0, 1.0, 1.0, l, i);
    let len = dep(0.0, 4.2, 0.0, l, i);
    let width = dep(0.0, 15.0, 0.0, l, i);
    let a = len - 0.5 * lw;
    let b = width - lw;
    let ry = 0.25 * b;
    t.lw = lw;
    t.cap = 1;
    t.join = 0;
    t.form = TipForm::JoinFirst;
    t.tip = a + 0.5 * lw;
    t.back = sincos_deg(190.0).0 * a - 0.5 * lw;
    t.line = 0.0;
    let e = |deg: f64| {
        let (s, c) = sincos_deg(deg);
        p(a * c, ry + ry * s)
    };
    let arc = |d0: f64, d1: f64| {
        let kk = 4.0 / 3.0 * tan_deg((d0 - d1) / 4.0);
        let (s0, c0) = sincos_deg(d0);
        let (s1, c1) = sincos_deg(d1);
        let (p0, p1) = (e(d0), e(d1));
        Seg::Curve(
            p(p0.x + kk * a * s0, p0.y - kk * ry * c0),
            p(p1.x - kk * a * s1, p1.y + kk * ry * c1),
            p1,
        )
    };
    t.segs[0] = Seg::Move(e(100.0));
    t.segs[1] = arc(100.0, 10.0);
    t.segs[2] = arc(10.0, -50.0);
    t.segs[3] = arc(-50.0, -90.0);
    t.nseg = 4;
    let hull = [
        p(0.5 * len, -0.5 * lw),
        p(len, -0.5 * lw + 0.125 * width),
        p(len, 0.375 * width),
        p(0.5 * len, 0.5 * width),
        p(-0.5 * len, 0.5 * width),
        p(-len, 0.375 * width),
        p(-len, 0.25 * width),
    ];
    t.hull[..7].copy_from_slice(&hull);
    t.nhull = 7;
    t.swap = swap;
    t
}

/// `Hooks[harpoon]` at `arrow style=tikz`: `len = dep(.75pt, 1.25, 0)`, `W = 4 len - L`, butt cap, no back (spec 9.6).
fn tip_hook_tikz(l: f64, i: f64, swap: bool) -> Tip {
    let mut t = Tip::blank();
    let lw = dep(0.0, 1.0, 1.0, l, i);
    let len = dep(0.75, 1.25, 0.0, l, i);
    let width = 4.0 * len - l;
    t.lw = lw;
    t.cap = 0;
    t.join = 0;
    t.form = TipForm::JoinFirst;
    let a = len - 0.5 * lw;
    let b = width - lw;
    t.tip = a + 0.5 * lw;
    t.back = 0.0;
    t.line = 0.0;
    hook_segs(&mut t, a, 0.25 * b);
    let hull = [
        p(0.5 * len, -0.5 * lw),
        p(len, -0.5 * lw + 0.125 * width),
        p(len, 0.375 * width),
        p(0.5 * len, 0.5 * width),
        p(0.0, 0.25 * width),
        p(0.0, 0.5 * width),
    ];
    t.hull[..6].copy_from_slice(&hull);
    t.nhull = 6;
    t.swap = swap;
    t
}

fn tip_implies(l: f64, i: f64) -> Tip {
    let mut t = Tip::blank();
    let a = 0.25 * l + 0.25 * i;
    let b = 0.5 * l - 0.5 * i;
    t.tip = 2.06 * a + 0.5 * b;
    t.back = -1.36 * a + 0.5 * b;
    t.line = 0.0;
    t.lw = b;
    t.cap = 1;
    t.join = 1;
    t.form = TipForm::WidthFirst;
    let dx = 0.06 * a;
    t.segs[0] = Seg::Move(p(-1.4 * a + dx, 2.65 * a));
    t.segs[1] = Seg::Curve(p(-0.75 * a + dx, 1.25 * a), p(a + dx, 0.05 * a), p(2.0 * a + dx, 0.0));
    t.segs[2] = Seg::Curve(p(a + dx, -0.05 * a), p(-0.75 * a + dx, -1.25 * a), p(-1.4 * a + dx, -2.65 * a));
    t.nseg = 3;
    t.hull[0] = p(-1.4 * a, 2.8 * a);
    t.hull[1] = p(-1.4 * a, -2.8 * a);
    t.hull[2] = p(2.0 * a, 0.5 * b);
    t.hull[3] = p(2.0 * a, -0.5 * b);
    t.nhull = 4;
    t
}

/// The tips for `name` at outer width `l` and inner width `i`, under an arrow style.
pub(crate) fn tip_seq(name: TipName, style: ArrowStyle, l: f64, i: f64) -> TipSeq {
    use TipName::*;
    match (style, name) {
        (_, Cap | ImpliesCap) => TipSeq::none(),

        (ArrowStyle::LatinModern, Bar | ImpliesBar) => TipSeq::one(tip_bar(l, i, 0.0, 12.0, 0.992, 1)),
        (ArrowStyle::LatinModern, LeftHook) => TipSeq::one(tip_hook_latin_modern(l, i, false)),
        (ArrowStyle::LatinModern, RightHook) => TipSeq::one(tip_hook_latin_modern(l, i, true)),

        (ArrowStyle::Tikz, Bar | ImpliesBar) => TipSeq::one(tip_bar(l, i, 3.0, 4.0, 0.9, 0)),
        (ArrowStyle::Tikz, To | PlainTo) => TipSeq::one(tip_plain(l, i)),
        (ArrowStyle::Tikz, DoubleTo) => TipSeq { n: 2, t: [tip_plain(l, i), tip_plain(l, i)] },
        (ArrowStyle::Tikz, LeftHook) => TipSeq::one(tip_hook_tikz(l, i, false)),
        (ArrowStyle::Tikz, RightHook) => TipSeq::one(tip_hook_tikz(l, i, true)),
        // `tikzcd to` is `>` there, and these three are `tikzcd to` with an option.
        (ArrowStyle::Tikz, LeftTo) => {
            TipSeq::one(tip_cm(l, i, Some(dep(1.6, 2.2, 0.0, l, i)), false, true, 0.0, false))
        }
        (ArrowStyle::Tikz, RightTo) => {
            TipSeq::one(tip_cm(l, i, Some(dep(1.6, 2.2, 0.0, l, i)), false, true, 0.0, true))
        }
        (ArrowStyle::Tikz, ToReversed) => {
            TipSeq::one(tip_cm(l, i, Some(dep(1.6, 2.2, 0.0, l, i)), true, false, 0.0, false))
        }

        (_, Bar | ImpliesBar) => TipSeq::one(tip_bar(l, i, 0.0, 8.2, 0.89, 1)),
        (_, To) => TipSeq::one(tip_cm(l, i, None, false, false, 0.0, false)),
        (_, PlainTo) => TipSeq::one(tip_plain(l, i)),
        (_, Implies) => TipSeq::one(tip_implies(l, i)),
        (_, LeftHook) => TipSeq::one(tip_hook(l, i, false)),
        (_, RightHook) => TipSeq::one(tip_hook(l, i, true)),
        (_, DoubleTo) => {
            TipSeq { n: 2, t: [tip_cm(l, i, None, false, false, dep(0.0, -2.6, 0.0, l, i), false), tip_cm(l, i, None, false, false, 0.0, false)] }
        }
        (_, LeftTo) => TipSeq::one(tip_cm(l, i, None, false, true, 0.0, false)),
        (_, RightTo) => TipSeq::one(tip_cm(l, i, None, false, true, 0.0, true)),
        (_, ToReversed) => TipSeq::one(tip_cm(l, i, None, true, false, 0.0, false)),
    }
}
