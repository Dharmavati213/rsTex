//! Keys of arrows, labels and cells (spec 5.4, 5.5, 3.2).  A key outside these tables is Unsupported: the oracle would
//! do something this crate does not know about, or report an error that only the oracle should report.

use crate::color;
use crate::geom::Pt;
use crate::parse::{MAX_DEPTH, Parser, R, check_label_text};
use crate::scan::{Span, bracket_end, brace_end, items, keyval, skip_ws, trim, unbrace};
use crate::spec::{
    Anchor, ArrowSpec, CellOpts, Curve, Dash, End, LabelFont, LabelSpec, LabelXform, Shift, TipName, Unsupported,
};

/// Default side and tips of `every arrow`: `/tikz/draw, /tikz/line width=rule_thickness, rightarrow`.
pub(crate) fn new_arrow(p: &Parser<'_>, r: u32, c: u32) -> ArrowSpec {
    ArrowSpec {
        r,
        c,
        start: End::Home,
        end: End::Home,
        sa: None,
        ea: None,
        xlate_s: Pt::ZERO,
        xlate_e: Pt::ZERO,
        shiftable: false,
        shifts: (0, 0),
        lw: p.font.rt,
        tips: [TipName::Cap, TipName::To],
        double: false,
        double_tikz: None,
        double_seen: false,
        double_color: None,
        dash: Dash::default(),
        squiggly: false,
        sq_lw: p.font.rt,
        sq_pre: None,
        sq_post: None,
        draw: true,
        phantom: false,
        color: None,
        draw_color: None,
        opacity: None,
        short_s: 0.0,
        short_e: 0.0,
        pos: 0.5,
        swap: false,
        sloped: false,
        curve: None,
        is_loop: false,
        crossing: false,
        canvas: Pt::ZERO,
        labels: (0, 0),
        own_every_label: [Span::EMPTY; 4],
        own_every_label_n: 0,
    }
}

/// `\tikz@to@switch@on`: the key makes the path the plain curve, which also undoes an earlier `loop`, whose `to path` it
/// replaces.  So `loop,looseness=4` is no loop at all, and neither looseness 8 nor the 5mm apply.
fn switch_on(a: &mut ArrowSpec) -> &mut Curve {
    a.is_loop = false;
    a.curve.get_or_insert(Curve::DEFAULT)
}

fn switch_on_with(a: &mut ArrowSpec, d: f64, set: impl FnOnce(&mut Curve, f64)) {
    set(switch_on(a), d);
}

#[derive(Copy, Clone)]
enum Extra {
    None,
    Dashed,
    Squiggly,
}

/// The both-ends styles of spec 5.4: start tip, end tip, `double line`, extra.
fn tip_style(k: &[u8]) -> Option<(TipName, TipName, bool, Extra)> {
    use TipName::*;
    Some(match k {
        b"rightarrow" => (Cap, To, false, Extra::None),
        b"leftarrow" => (To, Cap, false, Extra::None),
        b"leftrightarrow" => (To, To, false, Extra::None),
        b"Rightarrow" => (ImpliesCap, Implies, true, Extra::None),
        b"Leftarrow" => (Implies, ImpliesCap, true, Extra::None),
        b"Leftrightarrow" => (Implies, Implies, true, Extra::None),
        b"mapsto" => (Bar, To, false, Extra::None),
        b"mapsfrom" => (To, Bar, false, Extra::None),
        b"Mapsto" => (ImpliesBar, Implies, true, Extra::None),
        b"Mapsfrom" => (Implies, ImpliesBar, true, Extra::None),
        b"hookrightarrow" => (RightHook, To, false, Extra::None),
        b"hookleftarrow" => (To, LeftHook, false, Extra::None),
        b"rightharpoonup" => (Cap, LeftTo, false, Extra::None),
        b"rightharpoondown" => (Cap, RightTo, false, Extra::None),
        b"leftharpoonup" => (RightTo, Cap, false, Extra::None),
        b"leftharpoondown" => (LeftTo, Cap, false, Extra::None),
        b"rightarrowtail" => (ToReversed, To, false, Extra::None),
        b"leftarrowtail" => (To, ToReversed, false, Extra::None),
        b"twoheadrightarrow" => (Cap, DoubleTo, false, Extra::None),
        b"twoheadleftarrow" => (DoubleTo, Cap, false, Extra::None),
        b"dash" => (Cap, Cap, false, Extra::None),
        b"equal" | b"equals" => (ImpliesCap, ImpliesCap, true, Extra::None),
        b"dashrightarrow" => (Cap, To, false, Extra::Dashed),
        b"dashleftarrow" => (To, Cap, false, Extra::Dashed),
        b"rightsquigarrow" => (Cap, To, false, Extra::Squiggly),
        b"leftsquigarrow" => (To, Cap, false, Extra::Squiggly),
        b"leftrightsquigarrow" => (To, To, false, Extra::Squiggly),
        _ => return None,
    })
}

/// The one-end keys of spec 5.4: which end (0 start, 1 end) and which tip.
fn one_end(k: &[u8]) -> Option<(usize, TipName)> {
    use TipName::*;
    Some(match k {
        b"to head" => (1, To),
        b"no head" => (1, Cap),
        b"two heads" => (1, DoubleTo),
        b"harpoon" => (1, LeftTo),
        b"harpoon'" => (1, RightTo),
        b"no tail" => (0, Cap),
        b"tail" => (0, ToReversed),
        b"hook" => (0, RightHook),
        b"hook'" => (0, LeftHook),
        b"maps to" => (0, Bar),
        _ => return None,
    })
}

fn pos_key(k: &[u8]) -> Option<f64> {
    Some(match k {
        b"at start" => 0.0,
        b"very near start" => 0.125,
        b"near start" => 0.25,
        b"midway" => 0.5,
        b"near end" => 0.75,
        b"very near end" => 0.875,
        b"at end" => 1.0,
        _ => return None,
    })
}

fn thickness(k: &[u8]) -> Option<f64> {
    Some(match k {
        b"ultra thin" => 0.1,
        b"very thin" => 0.2,
        b"thin" => 0.4,
        b"semithick" => 0.6,
        b"thick" => 0.8,
        b"very thick" => 1.2,
        b"ultra thick" => 1.6,
        _ => return None,
    })
}

fn is_direction(k: &[u8]) -> bool {
    !k.is_empty() && k.iter().all(|c| matches!(c, b'r' | b'd' | b'l' | b'u'))
}

fn direction(k: &[u8]) -> End {
    let (mut dr, mut dc) = (0i64, 0i64);
    for c in k {
        match c {
            b'r' => dc += 1,
            b'l' => dc -= 1,
            b'd' => dr += 1,
            _ => dr -= 1,
        }
    }
    End::Rel(dr, dc)
}

/// `r-c`, both positive integers.
pub(crate) fn parse_rc(s: &[u8]) -> Option<(u32, u32)> {
    let dash = s.iter().position(|&c| c == b'-')?;
    Some((parse_u32(&s[..dash])?, parse_u32(&s[dash + 1..])?))
}

fn parse_u32(s: &[u8]) -> Option<u32> {
    if s.is_empty() || s.len() > 9 {
        return None;
    }
    let mut v = 0u32;
    for &c in s {
        if !c.is_ascii_digit() {
            return None;
        }
        v = v * 10 + u32::from(c - b'0');
    }
    Some(v)
}

/// The target of `to=<x>` (spec 5.4): `r-c`, else a direction string, else a name.
pub(crate) fn end_of(buf: &[u8], v: Span) -> End {
    let b = v.get(buf);
    let b2 = b.strip_prefix(b"\\tikzcdmatrixname-").unwrap_or(b);
    if let Some((r, c)) = parse_rc(b2) {
        return End::Abs(r, c);
    }
    if b.is_empty() {
        return End::Home;
    }
    if is_direction(b) {
        return direction(b);
    }
    End::Name(v)
}

/// An anchor name, or a number.
pub(crate) fn parse_anchor(p: &Parser<'_>, s: Span) -> Option<Anchor> {
    let t = trim(p.buf, s);
    Some(match t.get(p.buf) {
        b"center" => Anchor::Center,
        b"west" => Anchor::West,
        b"east" => Anchor::East,
        b"north" => Anchor::North,
        b"south" => Anchor::South,
        b"north east" => Anchor::NorthEast,
        b"north west" => Anchor::NorthWest,
        b"south east" => Anchor::SouthEast,
        b"south west" => Anchor::SouthWest,
        b"base" => Anchor::Base,
        b"base west" => Anchor::BaseWest,
        b"base east" => Anchor::BaseEast,
        b"mid" => Anchor::Mid,
        b"mid west" => Anchor::MidWest,
        b"mid east" => Anchor::MidEast,
        b"text" => Anchor::Text,
        b"real center" => Anchor::RealCenter,
        b"real west" => Anchor::RealWest,
        b"real east" => Anchor::RealEast,
        _ => {
            let v = p.value(t, 0.4).ok()?;
            if v.units {
                return None;
            }
            Anchor::Angle(v.v)
        }
    })
}

/// Keys tikz-cd itself defines: a `.style` on one of them changes tikz-cd, which DESIGN section 10 sends to real PGF.
pub(crate) fn is_builtin_key(k: &[u8]) -> bool {
    tip_style(k).is_some()
        || one_end(k).is_some()
        || matches!(
            k,
            b"to" | b"from"
                | b"shorten"
                | b"description"
                | b"marking"
                | b"phantom"
                | b"crossing over"
                | b"cramped"
                | b"row sep"
                | b"column sep"
                | b"sep"
                | b"math mode"
                | b"arrow style"
                | b"double line"
                | b"dashed"
                | b"squiggly"
                | b"start anchor"
                | b"end anchor"
                | b"shift left"
                | b"shift right"
                | b"transform nodes"
                | b"shift"
                | b"xshift"
                | b"yshift"
                | b"arrows"
                | b"labels"
                | b"cells"
                | b"diagrams"
                | b"background color"
                | b"crossing over clearance"
                | b"center yshift"
                | b"execute before arrows"
        )
}

impl Parser<'_> {
    // ---------------------------------------------------------------------------------------------------- arrow keys

    pub(crate) fn arrow_item(&mut self, a: &mut ArrowSpec, item: Span, depth: u32) -> R {
        let buf = self.buf;
        if depth > MAX_DEPTH {
            return Err(Unsupported::new("key lists nested too deeply", item));
        }
        if item.get(buf).first() == Some(&b'"') {
            return self.quoted_label(item);
        }
        let (ks, vs) = keyval(buf, item);
        let k = ks.get(buf);

        if let Some(first) = self.user_style(k) {
            return self.apply_style_arrow(a, first, ks, depth);
        }
        if is_direction(k) {
            a.end = direction(k);
            return Ok(());
        }
        if let Some((st, en, dbl, extra)) = tip_style(k) {
            a.tips = [st, en];
            a.double |= dbl;
            match extra {
                Extra::None => {}
                Extra::Dashed => a.dash = Dash::two(7.0 * a.lw, 4.0 * a.lw),
                Extra::Squiggly => {
                    a.squiggly = true;
                    a.sq_lw = a.lw;
                }
            }
            return Ok(());
        }
        if let Some((end, tip)) = one_end(k) {
            a.tips[end] = tip;
            return Ok(());
        }
        if let Some(t) = pos_key(k) {
            a.pos = t;
            return Ok(());
        }
        if let Some(w) = thickness(k) {
            a.lw = w;
            return Ok(());
        }
        let need = |v: Option<Span>| v.ok_or(Unsupported::new("key needs a value", ks));
        match k {
            b"to" => a.end = end_of(buf, need(vs)?),
            b"from" => a.start = end_of(buf, need(vs)?),
            b"double line" => a.double = true,
            b"dashed" => a.dash = Dash::two(7.0 * a.lw, 4.0 * a.lw),
            b"dotted" => a.dash = Dash::two(a.lw, 2.0),
            b"densely dotted" => a.dash = Dash::two(a.lw, 1.0),
            b"loosely dotted" => a.dash = Dash::two(a.lw, 4.0),
            b"densely dashed" => a.dash = Dash::two(3.0, 2.0),
            b"loosely dashed" => a.dash = Dash::two(3.0, 6.0),
            b"dash pattern" => a.dash = self.dash_pattern(need(vs)?, a.lw)?,
            b"squiggly" => {
                a.squiggly = true;
                a.sq_lw = a.lw;
                if let Some(v) = vs {
                    for it in items(buf, v) {
                        let (k2, v2) = keyval(buf, it);
                        let v2 = v2.ok_or(Unsupported::new("decoration key without a value", it))?;
                        // Evaluated when the path is decorated, when the width may be another.
                        if v2.get(buf).windows(13).any(|w| w == b"\\pgflinewidth") {
                            return Err(Unsupported::new("decoration length in terms of the line width", it));
                        }
                        match k2.get(buf) {
                            b"pre length" => a.sq_pre = Some(self.dim(v2, a.lw)?),
                            b"post length" => a.sq_post = Some(self.dim(v2, a.lw)?),
                            _ => return Err(Unsupported::new("decoration key other than pre and post length", it)),
                        }
                    }
                }
            }
            b"line width" => a.lw = self.dim(need(vs)?, a.lw)?,
            b"phantom" => {
                a.draw = false;
                a.phantom = true;
            }
            b"draw" => match vs {
                None => a.draw = true,
                Some(v) if v.get(buf) == b"none" => a.draw = false,
                Some(v) => {
                    a.draw = true;
                    a.draw_color = Some(self.color(v)?);
                }
            },
            b"color" => a.color = Some(self.color(need(vs)?)?),
            b"opacity" => {
                let o = self.dim(need(vs)?, a.lw)?;
                if !(0.0..=1.0).contains(&o) {
                    return Err(Unsupported::new("opacity outside [0,1]", ks));
                }
                a.opacity = Some(o);
            }
            b"shift left" | b"shift right" => {
                a.shiftable = true;
                let ex = self.font.ex;
                let r = match vs {
                    None => 0.56 * ex,
                    Some(v) => {
                        let val = self.value(v, a.lw)?;
                        if val.units { val.v } else { val.v * 0.56 * ex }
                    }
                };
                let r = if k == b"shift right" { -r } else { r };
                self.o.shifts.push(Shift::Left(r));
            }
            b"xshift" => {
                a.shiftable = true;
                let d = self.dim(need(vs)?, a.lw)?;
                self.o.shifts.push(Shift::Translate(Pt::new(d, 0.0)));
            }
            b"yshift" => {
                a.shiftable = true;
                let d = self.dim(need(vs)?, a.lw)?;
                self.o.shifts.push(Shift::Translate(Pt::new(0.0, d)));
            }
            b"shift" => {
                a.shiftable = true;
                let d = self.coordinate(need(vs)?, a.lw)?;
                self.o.shifts.push(Shift::Translate(d));
            }
            b"transform canvas" => {
                let t = self.translation_list(need(vs)?, a.lw)?;
                a.canvas = a.canvas.add(t);
            }
            b"shorten" => {
                let d = self.dim(need(vs)?, a.lw)?;
                a.short_s = d;
                a.short_e = d;
            }
            b"shorten <" => a.short_s = self.dim(need(vs)?, a.lw)?,
            b"shorten >" => a.short_e = self.dim(need(vs)?, a.lw)?,
            b"start anchor" | b"end anchor" => {
                let (t, anchor) = self.anchor_spec(need(vs)?, a.lw)?;
                if let Some(t) = t {
                    a.shiftable = true;
                    if k == b"start anchor" {
                        a.xlate_s = a.xlate_s.add(t);
                    } else {
                        a.xlate_e = a.xlate_e.add(t);
                    }
                }
                if k == b"start anchor" {
                    a.sa = anchor;
                } else {
                    a.ea = anchor;
                }
            }
            b"bend left" | b"bend right" => {
                let ang = match vs {
                    None => 30.0,
                    Some(v) => self.dim(v, a.lw)?,
                };
                let cv = switch_on(a);
                cv.relative = true;
                if k == b"bend left" {
                    cv.out = ang;
                    cv.inn = 180.0 - ang;
                } else {
                    cv.out = -ang;
                    cv.inn = 180.0 + ang;
                }
            }
            b"in" => switch_on_with(a, self.dim(need(vs)?, a.lw)?, |cv, d| cv.inn = d),
            b"out" => switch_on_with(a, self.dim(need(vs)?, a.lw)?, |cv, d| cv.out = d),
            b"looseness" => switch_on_with(a, self.dim(need(vs)?, a.lw)?, |cv, d| {
                cv.l_out = d;
                cv.l_in = d;
            }),
            b"in looseness" => switch_on_with(a, self.dim(need(vs)?, a.lw)?, |cv, d| cv.l_in = d),
            b"out looseness" => switch_on_with(a, self.dim(need(vs)?, a.lw)?, |cv, d| cv.l_out = d),
            b"distance" => switch_on_with(a, self.dim(need(vs)?, a.lw)?, |cv, d| {
                cv.min = d;
                cv.max = d;
            }),
            b"min distance" => switch_on_with(a, self.dim(need(vs)?, a.lw)?, |cv, d| cv.min = d),
            b"max distance" => switch_on_with(a, self.dim(need(vs)?, a.lw)?, |cv, d| cv.max = d),
            b"loop" | b"loop right" | b"loop above" | b"loop left" | b"loop below" => {
                let cv = a.curve.get_or_insert(Curve::DEFAULT);
                match k {
                    b"loop right" => (cv.out, cv.inn) = (15.0, -15.0),
                    b"loop above" => (cv.out, cv.inn) = (105.0, 75.0),
                    b"loop left" => (cv.out, cv.inn) = (195.0, 165.0),
                    b"loop below" => (cv.out, cv.inn) = (285.0, 255.0),
                    _ => {}
                }
                a.is_loop = true;
            }
            b"crossing over" => a.crossing = true,
            b"->" => a.tips = [TipName::Cap, TipName::PlainTo],
            b"<-" => a.tips = [TipName::PlainTo, TipName::Cap],
            b"<->" => a.tips = [TipName::PlainTo, TipName::PlainTo],
            b"-" => a.tips = [TipName::Cap, TipName::Cap],
            b"double" => {
                a.double_seen = true;
                if a.double_tikz.is_none() {
                    a.double_tikz = Some(0.6);
                }
                if let Some(v) = vs {
                    a.double_color = Some(self.color(v)?);
                }
            }
            b"double distance" => a.double_tikz = Some(self.dim(need(vs)?, a.lw)?),
            b"pos" => a.pos = self.number(need(vs)?, a.lw)?,
            b"swap" | b"'" => a.swap = !a.swap,
            b"sloped" => a.sloped = true,
            b"labels" => {
                let v = need(vs)?;
                let n = usize::from(a.own_every_label_n);
                if n == a.own_every_label.len() {
                    return Err(Unsupported::new("too many labels= on one arrow", ks));
                }
                a.own_every_label[n] = v;
                a.own_every_label_n += 1;
            }
            _ if vs.is_none() && color::looks_like_color(k) => a.color = Some(self.color(ks)?),
            _ => return Err(Unsupported::new("unknown arrow key", ks)),
        }
        Ok(())
    }

    fn apply_style_arrow(&mut self, a: &mut ArrowSpec, first: usize, at: Span, depth: u32) -> R {
        self.spend_style(at)?;
        let buf = self.buf;
        let name = self.o.styles[first].0;
        let mut i = first;
        while i < self.o.styles.len() {
            let (n, v) = self.o.styles[i];
            if n.get(buf) == name.get(buf) {
                if v.get(buf).contains(&b'#') {
                    return Err(Unsupported::new("style with a parameter", at));
                }
                for it in items(buf, v) {
                    self.arrow_item(a, it, depth + 1)?;
                }
            }
            i += 1;
        }
        Ok(())
    }

    fn number(&self, v: Span, lw: f64) -> R<f64> {
        self.dim(v, lw)
    }

    /// `on a off b ...`, at most four pairs.
    fn dash_pattern(&self, v: Span, lw: f64) -> R<Dash> {
        let buf = self.buf;
        let bad = Unsupported::new("dash pattern outside `on a off b ...`", v);
        let mut d = Dash::default();
        let (mut i, end) = (v.start as usize, v.end as usize);
        loop {
            i = skip_ws(buf, i, end);
            if i >= end {
                break;
            }
            let word = if buf[i..end].starts_with(b"on") {
                b"on".as_slice()
            } else if buf[i..end].starts_with(b"off") {
                b"off".as_slice()
            } else {
                return Err(bad);
            };
            if (usize::from(d.n) % 2 == 0) != (word == b"on") || usize::from(d.n) == d.v.len() {
                return Err(bad);
            }
            i += word.len();
            let start = i;
            // The length runs up to the next `on` or `off`.
            while i < end && !buf[i..end].starts_with(b"on") && !buf[i..end].starts_with(b"off") {
                i += 1;
            }
            d.v[usize::from(d.n)] = self.dim(trim(buf, Span::new(start, i)), lw)?;
            d.n += 1;
        }
        if d.n == 0 || d.n % 2 != 0 {
            return Err(bad);
        }
        Ok(d)
    }

    /// A list of `xshift`, `yshift` and `shift`.
    fn translation_list(&self, list: Span, lw: f64) -> R<Pt> {
        let buf = self.buf;
        let mut t = Pt::ZERO;
        for it in items(buf, list) {
            let (k, v) = keyval(buf, it);
            let bad = Unsupported::new("transform other than xshift, yshift and shift", it);
            let v = v.ok_or(bad)?;
            match k.get(buf) {
                b"xshift" => t.x += self.dim(v, lw)?,
                b"yshift" => t.y += self.dim(v, lw)?,
                b"shift" => t = t.add(self.coordinate(v, lw)?),
                _ => return Err(bad),
            }
        }
        Ok(t)
    }

    /// `[<transform>]<anchor>`, both parts optional (spec 5.4).
    fn anchor_spec(&self, v: Span, lw: f64) -> R<(Option<Pt>, Option<Anchor>)> {
        let buf = self.buf;
        let bad = Unsupported::new("anchor outside the supported forms", v);
        let mut t = None;
        let mut rest = trim(buf, v);
        if rest.get(buf).first() == Some(&b'[') {
            let close = bracket_end(buf, rest.start as usize, rest.end as usize).ok_or(bad)?;
            let inner = trim(buf, Span::new(rest.start as usize + 1, close));
            if !inner.is_empty() {
                t = Some(self.translation_list(inner, lw)?);
            }
            rest = trim(buf, Span::new(close + 1, rest.end as usize));
        }
        if rest.is_empty() {
            return Ok((t, None));
        }
        Ok((t, Some(parse_anchor(self, rest).ok_or(bad)?)))
    }

    // -------------------------------------------------------------------------------------------------------- labels

    /// `"text"`, `"text"'`, `"text"{opts}`, `"text"'{opts}`, `"text" opts`, `"text"' opts` (spec 3.4).
    fn quoted_label(&mut self, item: Span) -> R {
        let buf = self.buf;
        let (a, end) = (item.start as usize, item.end as usize);
        let mut i = a + 1;
        let mut depth = 0i64;
        loop {
            if i >= end {
                return Err(Unsupported::new("label without a closing quote", item));
            }
            match buf[i] {
                b'\\' => {
                    i += 2;
                    continue;
                }
                b'{' => depth += 1,
                b'}' => depth -= 1,
                b'"' if depth == 0 => break,
                _ => {}
            }
            i += 1;
        }
        let text = unbrace(buf, Span::new(a + 1, i));
        let mut j = skip_ws(buf, i + 1, end);
        let mut swap = false;
        if j < end && buf[j] == b'\'' {
            swap = true;
            j = skip_ws(buf, j + 1, end);
        }
        let opts = if j < end && buf[j] == b'{' {
            // Anything after the closing brace is ignored.
            let close = brace_end(buf, j, end).ok_or(Unsupported::new("unbalanced label options", item))?;
            Span::new(j + 1, close)
        } else {
            Span::new(j, end)
        };
        self.push_raw_label(text, swap, opts)
    }

    pub(crate) fn push_raw_label(&mut self, text: Span, swap: bool, opts: Span) -> R {
        check_label_text(self.buf, text, self.policy)?;
        let ex = self.font.ex;
        self.o.labels.push(LabelSpec {
            text,
            font: LabelFont::Script,
            color: None,
            raw_swap: swap,
            raw_opts: opts,
            ixs: 0.5 * ex,
            iys: 0.5 * ex,
            outer: None,
            min_w: 1.0,
            min_h: 1.0,
            pos: 0.5,
            swap: false,
            auto: true,
            anchor: Anchor::Center,
            asym: false,
            fill: None,
            name: Span::EMPTY,
            alias: Span::EMPTY,
            sloped: false,
            upside: false,
            xforms: (0, 0),
        });
        Ok(())
    }

    /// Evaluate this arrow's labels, once all of its options have run (spec 5.2).
    pub(crate) fn finish_labels(&mut self, a: &ArrowSpec) -> R {
        let buf = self.buf;
        for li in a.labels.0..a.labels.1 {
            let mut l = self.o.labels[li as usize];
            l.pos = a.pos;
            l.swap = a.swap;
            l.sloped = a.sloped;
            l.color = a.color;
            let at = self.o.xforms.len() as u32;
            if a.phantom {
                l.font = LabelFont::Text;
                l.auto = false;
                l.anchor = Anchor::Center;
            }
            let mut i = 0;
            while i < self.o.every_label.len() {
                let sp = self.o.every_label[i];
                for it in items(buf, sp) {
                    self.label_item(&mut l, a, it, 0)?;
                }
                i += 1;
            }
            for n in 0..usize::from(a.own_every_label_n) {
                for it in items(buf, a.own_every_label[n]) {
                    self.label_item(&mut l, a, it, 0)?;
                }
            }
            if l.raw_swap {
                l.swap = !l.swap;
            }
            for it in items(buf, l.raw_opts) {
                self.label_item(&mut l, a, it, 0)?;
            }
            l.xforms = (at, self.o.xforms.len() as u32);
            self.o.labels[li as usize] = l;
        }
        Ok(())
    }

    fn label_item(&mut self, l: &mut LabelSpec, a: &ArrowSpec, item: Span, depth: u32) -> R {
        let buf = self.buf;
        if depth > MAX_DEPTH {
            return Err(Unsupported::new("key lists nested too deeply", item));
        }
        let (ks, vs) = keyval(buf, item);
        let k = ks.get(buf);
        if let Some(first) = self.user_style(k) {
            self.spend_style(ks)?;
            let name = self.o.styles[first].0;
            let mut i = first;
            while i < self.o.styles.len() {
                let (n, v) = self.o.styles[i];
                if n.get(buf) == name.get(buf) {
                    if v.get(buf).contains(&b'#') {
                        return Err(Unsupported::new("style with a parameter", ks));
                    }
                    for it in items(buf, v) {
                        self.label_item(l, a, it, depth + 1)?;
                    }
                }
                i += 1;
            }
            return Ok(());
        }
        if let Some(t) = pos_key(k) {
            l.pos = t;
            return Ok(());
        }
        let need = |v: Option<Span>| v.ok_or(Unsupported::new("key needs a value", ks));
        match k {
            b"above" | b"below" | b"left" | b"right" => {
                l.auto = false;
                l.anchor = match k {
                    b"above" => Anchor::South,
                    b"below" => Anchor::North,
                    b"left" => Anchor::East,
                    _ => Anchor::West,
                };
                if let Some(v) = vs {
                    let d = self.dim(v, a.lw)?;
                    let t = match k {
                        b"above" => Pt::new(0.0, d),
                        b"below" => Pt::new(0.0, -d),
                        b"left" => Pt::new(-d, 0.0),
                        _ => Pt::new(d, 0.0),
                    };
                    self.o.xforms.push(LabelXform::Shift(t));
                }
            }
            b"xshift" => {
                let d = self.dim(need(vs)?, a.lw)?;
                self.o.xforms.push(LabelXform::Shift(Pt::new(d, 0.0)));
            }
            b"yshift" => {
                let d = self.dim(need(vs)?, a.lw)?;
                self.o.xforms.push(LabelXform::Shift(Pt::new(0.0, d)));
            }
            b"shift" => {
                let d = self.coordinate(need(vs)?, a.lw)?;
                self.o.xforms.push(LabelXform::Shift(d));
            }
            b"rotate" => {
                let d = self.dim(need(vs)?, a.lw)?;
                self.o.xforms.push(LabelXform::Rotate(d));
            }
            b"allow upside down" => l.upside = true,
            b"swap" | b"'" => l.swap = !l.swap,
            b"pos" => l.pos = self.dim(need(vs)?, a.lw)?,
            b"inner sep" => {
                let d = self.dim(need(vs)?, a.lw)?;
                l.ixs = d;
                l.iys = d;
            }
            b"inner xsep" => l.ixs = self.dim(need(vs)?, a.lw)?,
            b"inner ysep" => l.iys = self.dim(need(vs)?, a.lw)?,
            b"outer sep" => l.outer = Some(self.dim(need(vs)?, a.lw)?),
            b"minimum width" => l.min_w = self.dim(need(vs)?, a.lw)?,
            b"minimum height" => l.min_h = self.dim(need(vs)?, a.lw)?,
            b"description" => {
                l.auto = false;
                l.anchor = Anchor::Center;
                l.fill = Some(self.o.s.bg);
            }
            b"marking" => {
                l.auto = false;
                l.anchor = Anchor::Center;
                l.asym = true;
                l.sloped = true;
                l.font = LabelFont::Text;
            }
            b"anchor" => {
                let v = need(vs)?;
                l.auto = false;
                l.anchor = parse_anchor(self, v).ok_or(Unsupported::new("unknown anchor", v))?;
            }
            b"sloped" => l.sloped = true,
            b"name" => l.name = need(vs)?,
            b"alias" => l.alias = need(vs)?,
            b"font" => match vs {
                Some(v) if !v.is_empty() => l.font = LabelFont::Command(v),
                _ => l.font = LabelFont::Text,
            },
            b"text" | b"color" => l.color = Some(self.color(need(vs)?)?),
            b"fill" => {
                let v = need(vs)?;
                l.fill = if v.get(buf) == b"none" { None } else { Some(self.color(v)?) };
            }
            _ if vs.is_none() && color::looks_like_color(k) => l.color = Some(self.color(ks)?),
            _ => return Err(Unsupported::new("unknown label key", ks)),
        }
        Ok(())
    }

    // --------------------------------------------------------------------------------------------------------- cells

    pub(crate) fn cell_list(&mut self, o: &mut CellOpts, list: Span, depth: u32) -> R {
        let buf = self.buf;
        if depth > MAX_DEPTH {
            return Err(Unsupported::new("key lists nested too deeply", list));
        }
        for it in items(buf, list) {
            let (ks, vs) = keyval(buf, it);
            let k = ks.get(buf);
            if let Some(first) = self.user_style(k) {
                self.spend_style(ks)?;
                let name = self.o.styles[first].0;
                let mut i = first;
                while i < self.o.styles.len() {
                    let (n, v) = self.o.styles[i];
                    if n.get(buf) == name.get(buf) {
                        if v.get(buf).contains(&b'#') {
                            return Err(Unsupported::new("style with a parameter", ks));
                        }
                        self.cell_list(o, v, depth + 1)?;
                    }
                    i += 1;
                }
                continue;
            }
            let need = |v: Option<Span>| v.ok_or(Unsupported::new("key needs a value", ks));
            match k {
                b"inner sep" => {
                    let d = self.dim(need(vs)?, self.o.s.cell_lw)?;
                    o.ixs = d;
                    o.iys = d;
                }
                b"inner xsep" => o.ixs = self.dim(need(vs)?, self.o.s.cell_lw)?,
                b"inner ysep" => o.iys = self.dim(need(vs)?, self.o.s.cell_lw)?,
                b"nodes" => self.cell_list(o, unbrace(buf, need(vs)?), depth + 1)?,
                b"font" => o.font = vs.unwrap_or(Span::EMPTY),
                b"draw" => match vs {
                    None => o.draw = true,
                    Some(v) if v.get(buf) == b"none" => o.draw = false,
                    Some(v) => {
                        o.draw = true;
                        o.draw_color = Some(self.color(v)?);
                    }
                },
                b"rounded corners" => {
                    o.rounded = match vs {
                        None => 4.0,
                        Some(v) => self.dim(v, self.o.s.cell_lw)?,
                    }
                }
                b"sharp corners" => o.rounded = 0.0,
                b"alias" => o.alias = need(vs)?,
                b"text height" => o.text_height = Some(self.dim(need(vs)?, self.o.s.cell_lw)?),
                b"text depth" => o.text_depth = Some(self.dim(need(vs)?, self.o.s.cell_lw)?),
                b"text" | b"color" => o.color = Some(self.color(need(vs)?)?),
                _ if vs.is_none() && color::looks_like_color(k) => o.color = Some(self.color(ks)?),
                _ => return Err(Unsupported::new("unknown cell key", ks)),
            }
        }
        Ok(())
    }
}
