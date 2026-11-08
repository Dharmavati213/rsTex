//! Keys of arrows, labels and cells (spec 5.4, 5.5, 3.2).  A key outside these tables is Unsupported: the oracle would
//! do something this crate does not know about, or report an error that only the oracle should report.

use crate::color;
use crate::font_sp;
use crate::geom::pt;
use crate::parse::{Dir, MAX_DEPTH, Parser, R, check_label_text, place, same_text};
use crate::pgfmath::{PT, Sp, Xy, dec, half, mul};
use crate::scan::{Span, bracket_end, brace_end, items, keyval, skip_ws, trim, unbrace};
use crate::spec::{
    Anchor, ArrowSpec, CELL_ALIAS, CELL_DEPTH, CELL_FONT, CELL_HEIGHT, CELL_IXS, CELL_IYS, CELL_ROUNDED, CellOpts, Curve, Dash, End,
    LabelFont, LabelSpec, LabelXform, Opt, QuiverCurve, Shift, TipName, Unsupported,
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
        xlate_s: (0, 0),
        xlate_e: (0, 0),
        shiftable: false,
        shifts: (0, 0),
        lw: font_sp(p.font.rt),
        tips: [TipName::Cap, TipName::To],
        double: false,
        double_tikz: None,
        double_seen: false,
        double_color: None,
        dash: Dash::default(),
        squiggly: false,
        sq_seg: 0,
        sq_amp: 0,
        sq_pre: None,
        sq_post: None,
        draw: true,
        phantom: false,
        phantom_at: 0,
        color: None,
        text_color: None,
        opacity: None,
        short_s: 0,
        short_e: 0,
        pos: PT / 2,
        swap: false,
        sloped: false,
        upside: false,
        curve: None,
        quiver: None,
        is_loop: false,
        bend: 30 * PT,
        overlay: false,
        crossing: false,
        canvas: (0, 0),
        opts: (0, 0),
        labels: (0, 0),
        own_every_label: [Span::EMPTY; 4],
        own_every_label_n: 0,
    }
}

/// tikz-cd's `double line`: `double equal sign distance`, which makes `\tikz@double@setup` the distance between the line centres,
/// 0.45ex, and `double=<background color>`.  So it replaces what an earlier `double distance` or `double=<c>` said, as each of
/// those replaces what it says (spec 10.2).
fn double_line(a: &mut ArrowSpec) {
    a.double = true;
    a.double_tikz = None;
    a.double_color = None;
}

/// `\tikz@to@switch@on`: the key makes the path the plain curve, which also undoes an earlier `loop`, whose `to path` it
/// replaces.  So `loop,looseness=4` is no loop at all, and neither looseness 8 nor the 5mm apply.
fn switch_on(a: &mut ArrowSpec) -> &mut Curve {
    a.is_loop = false;
    a.quiver = None;
    a.curve.get_or_insert(Curve::DEFAULT)
}

fn switch_on_with(a: &mut ArrowSpec, d: Sp, set: impl FnOnce(&mut Curve, Sp)) {
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

fn pos_key(k: &[u8]) -> Option<Sp> {
    Some(match k {
        b"at start" => 0,
        b"very near start" => dec(b"0.125"),
        b"near start" => dec(b"0.25"),
        b"midway" => dec(b"0.5"),
        b"near end" => dec(b"0.75"),
        b"very near end" => dec(b"0.875"),
        b"at end" => PT,
        _ => return None,
    })
}

fn thickness(k: &[u8]) -> Option<Sp> {
    Some(match k {
        b"ultra thin" => dec(b"0.1"),
        b"very thin" => dec(b"0.2"),
        b"thin" => dec(b"0.4"),
        b"semithick" => dec(b"0.6"),
        b"thick" => dec(b"0.8"),
        b"very thick" => dec(b"1.2"),
        b"ultra thick" => dec(b"1.6"),
        _ => return None,
    })
}

/// `min distance=5mm` of `loop`, as TeX converts it.
pub(crate) const LOOP_MIN: Sp = dec(b"14.22636");

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
            let v = p.value(t, CELL_LW).ok()?;
            if v.units {
                return None;
            }
            // `\\pgfmathsetcounter`.
            Anchor::Angle(v.v / PT)
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

/// The tips a name's `.tip` reaches: `tikzcd to` is under `tikzcd left to`, and the `cm` names under the `tikzcd` ones.
pub(crate) fn tips_of_name(name: &[u8]) -> u16 {
    use TipName::*;
    let n = name.strip_prefix(b"tikzcd ").or(name.strip_prefix(b"cm ")).unwrap_or(b"");
    let b = tip_bit;
    match n {
        b"to" => b(To) | b(LeftTo) | b(RightTo) | b(ToReversed) | b(DoubleTo),
        b"left to" => b(LeftTo) | b(RightTo),
        b"right to" => b(RightTo),
        b"to reversed" => b(ToReversed),
        b"double to" => b(DoubleTo),
        b"bar" | b"|" => b(Bar) | b(ImpliesBar),
        b"left hook" => b(LeftHook) | b(RightHook),
        b"right hook" => b(RightHook),
        b"implies" => b(Implies),
        b"implies cap" => b(ImpliesCap),
        b"implies bar" => b(ImpliesBar),
        b"cap" => b(Cap),
        _ => 0,
    }
}

#[inline]
pub(crate) fn tip_bit(t: TipName) -> u16 {
    1 << t as u16
}

/// quiver.sty's `curve` and the `settings` it calls, spec 8.5.
const QUIVER_CURVE: &[u8] = b"settings={#1},to path={(\\tikztostart) .. controls ($(\\tikztostart)!\\pv{pos}!(\\tikztotarget)!\\pv{height}!270:(\\tikztotarget)$) and ($(\\tikztostart)!1-\\pv{pos}!(\\tikztotarget)!\\pv{height}!270:(\\tikztotarget)$) .. (\\tikztotarget)\\tikztonodes}";
const QUIVER_SETTINGS: &[u8] = b"\\tikzset{quiver/.cd,#1} \\def\\pv##1{\\pgfkeysvalueof{/tikz/quiver/##1}}";

/// `0.4pt`, TikZ's line width where nothing has set it.
pub(crate) const CELL_LW: Sp = dec(b"0.4");

impl Parser<'_> {
    // ---------------------------------------------------------------------------------------------------- arrow keys

    /// `dashed`: `dash pattern=on 7\pgflinewidth off 4\pgflinewidth` with the width the options have reached.
    fn set_dash(&mut self, a: &mut ArrowSpec, d: Dash) {
        a.dash = d;
        self.o.opts.push(Opt::Dash(d));
    }

    /// `\tikz@semiaddlinewidth`.
    fn set_width(&mut self, a: &mut ArrowSpec, w: Sp) {
        a.lw = w;
        self.o.opts.push(Opt::Width(pt(w)));
    }

    /// `squiggly`: `segment length` and `amplitude` now, the rest later.
    fn set_squiggly(&mut self, a: &mut ArrowSpec) {
        const SEG: Sp = dec(b"9.25");
        const AMP: Sp = dec(b"1.9");
        a.squiggly = true;
        a.sq_seg = mul(SEG, a.lw);
        a.sq_amp = mul(AMP, a.lw);
    }

    pub(crate) fn arrow_item(&mut self, a: &mut ArrowSpec, item: Span, depth: u32) -> R {
        let buf = self.buf;
        if depth > MAX_DEPTH {
            return Err(Unsupported::new("key lists nested too deeply", item));
        }
        if item.get(buf).first() == Some(&b'"') {
            return self.quoted_label(item);
        }
        let (ks, vs) = keyval(buf, item);
        let (tikz, ks) = self.local_key(ks, "unknown arrow key")?;
        let k = ks.get(buf);

        if let Some(first) = self.user_style(k, tikz) {
            return self.apply_style_arrow(a, first, ks, vs, depth);
        }
        if let Some(d) = self.user_code(k, tikz) {
            return self.code_on_arrow(a, d.body, ks);
        }
        self.ask(ks)?;
        if tikz && is_builtin_key(k) {
            return self.tikz_key_of_cd_name(a, k, ks);
        }
        if !tikz && is_direction(k) {
            a.end = direction(k);
            return Ok(());
        }
        if let Some((st, en, dbl, extra)) = tip_style(k) {
            a.tips = [st, en];
            if dbl {
                double_line(a);
            }
            match extra {
                Extra::None => {}
                Extra::Dashed => self.set_dash(a, Dash::two(7 * a.lw, 4 * a.lw)),
                Extra::Squiggly => self.set_squiggly(a),
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
            self.set_width(a, w);
            return Ok(());
        }
        let need = |v: Option<Span>| v.ok_or(Unsupported::new("key needs a value", ks));
        match k {
            b"to" => a.end = end_of(buf, need(vs)?),
            b"from" => a.start = end_of(buf, need(vs)?),
            b"double line" => double_line(a),
            b"dashed" => self.set_dash(a, Dash::two(7 * a.lw, 4 * a.lw)),
            b"dotted" => self.set_dash(a, Dash::two(a.lw, 2 * PT)),
            b"densely dotted" => self.set_dash(a, Dash::two(a.lw, PT)),
            b"loosely dotted" => self.set_dash(a, Dash::two(a.lw, 4 * PT)),
            b"densely dashed" => self.set_dash(a, Dash::two(3 * PT, 2 * PT)),
            b"loosely dashed" => self.set_dash(a, Dash::two(3 * PT, 6 * PT)),
            b"dash pattern" => {
                let d = self.dash_pattern(need(vs)?, a.lw)?;
                self.set_dash(a, d);
            }
            b"squiggly" => {
                self.set_squiggly(a);
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
            b"line width" => {
                let v = need(vs)?;
                // The key evaluates it once now and once more among the options.
                if v.get(buf).windows(13).any(|w| w == b"\\pgflinewidth") {
                    return Err(Unsupported::new("line width in terms of the line width", v));
                }
                let w = self.dim(v, a.lw)?;
                self.set_width(a, w);
            }
            b"phantom" => {
                a.draw = false;
                a.phantom = true;
                a.phantom_at = a.own_every_label_n;
            }
            b"draw" => match vs {
                None => a.draw = true,
                Some(v) if v.get(buf) == b"none" => a.draw = false,
                Some(v) => {
                    a.draw = true;
                    let c = self.color(v)?;
                    self.o.opts.push(Opt::Stroke(c));
                }
            },
            // `\tikzoption{color}`, `\tikzoption{text}` and a colour by its name each end in `\def\tikz@textcolor`: the last of them
            // colours the labels.
            b"color" => {
                let c = self.color(need(vs)?)?;
                a.color = Some(c);
                a.text_color = Some(c);
                self.o.opts.push(Opt::SetColor(c));
            }
            b"text" => a.text_color = Some(self.color(need(vs)?)?),
            b"opacity" => {
                let o = self.dim(need(vs)?, a.lw)?;
                if !(0..=PT).contains(&o) {
                    return Err(Unsupported::new("opacity outside [0,1]", ks));
                }
                a.opacity = Some(pt(o));
                self.o.opts.push(Opt::Opacity(pt(o)));
            }
            b"shift left" | b"shift right" => {
                a.shiftable = true;
                let unit = mul(dec(b"0.56"), font_sp(self.font.ex));
                let r = match vs {
                    None => unit,
                    Some(v) => {
                        let val = self.value(v, a.lw)?;
                        // `\\pgfmathparse{\\pgfmathresult*+0.56ex}`.
                        if val.units { val.v } else { mul(unit, val.v) }
                    }
                };
                let r = if k == b"shift right" { -r } else { r };
                self.o.shifts.push(Shift::Left(r));
            }
            b"xshift" => {
                a.shiftable = true;
                let d = self.dim(need(vs)?, a.lw)?;
                self.o.shifts.push(Shift::Translate((d, 0)));
            }
            b"yshift" => {
                a.shiftable = true;
                let d = self.dim(need(vs)?, a.lw)?;
                self.o.shifts.push(Shift::Translate((0, d)));
            }
            b"shift" => {
                a.shiftable = true;
                let d = self.coordinate(need(vs)?, a.lw)?;
                self.o.shifts.push(Shift::Translate(d));
            }
            b"transform canvas" => {
                let t = self.translation_list(need(vs)?, a.lw)?;
                a.canvas = (a.canvas.0 + t.0, a.canvas.1 + t.1);
                self.o.opts.push(Opt::Canvas(pt(t.0), pt(t.1)));
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
                        a.xlate_s = (a.xlate_s.0 + t.0, a.xlate_s.1 + t.1);
                    } else {
                        a.xlate_e = (a.xlate_e.0 + t.0, a.xlate_e.1 + t.1);
                    }
                }
                if k == b"start anchor" {
                    a.sa = anchor;
                } else {
                    a.ea = anchor;
                }
            }
            b"bend left" | b"bend right" => {
                // `\tikz@to@bend`: one with no angle takes what the one before it in this arrow's options said.
                if let Some(v) = vs {
                    a.bend = self.dim(v, a.lw)?;
                }
                let ang = a.bend;
                let cv = switch_on(a);
                cv.relative = true;
                if k == b"bend left" {
                    cv.out = ang;
                    cv.inn = 180 * PT - ang;
                } else {
                    cv.out = -ang;
                    cv.inn = 180 * PT + ang;
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
                cv.min_out = d;
                cv.max_out = d;
                cv.min_in = d;
                cv.max_in = d;
            }),
            b"min distance" => switch_on_with(a, self.dim(need(vs)?, a.lw)?, |cv, d| {
                cv.min_out = d;
                cv.min_in = d;
            }),
            b"max distance" => switch_on_with(a, self.dim(need(vs)?, a.lw)?, |cv, d| {
                cv.max_out = d;
                cv.max_in = d;
            }),
            b"in distance" => switch_on_with(a, self.dim(need(vs)?, a.lw)?, |cv, d| {
                cv.min_in = d;
                cv.max_in = d;
            }),
            b"out distance" => switch_on_with(a, self.dim(need(vs)?, a.lw)?, |cv, d| {
                cv.min_out = d;
                cv.max_out = d;
            }),
            b"in min distance" => switch_on_with(a, self.dim(need(vs)?, a.lw)?, |cv, d| cv.min_in = d),
            b"in max distance" => switch_on_with(a, self.dim(need(vs)?, a.lw)?, |cv, d| cv.max_in = d),
            b"out min distance" => switch_on_with(a, self.dim(need(vs)?, a.lw)?, |cv, d| cv.min_out = d),
            b"out max distance" => switch_on_with(a, self.dim(need(vs)?, a.lw)?, |cv, d| cv.max_out = d),
            b"loop" | b"loop right" | b"loop above" | b"loop left" | b"loop below" => {
                let cv = a.curve.get_or_insert(Curve::DEFAULT);
                match k {
                    b"loop right" => (cv.out, cv.inn) = (15 * PT, -15 * PT),
                    b"loop above" => (cv.out, cv.inn) = (105 * PT, 75 * PT),
                    b"loop left" => (cv.out, cv.inn) = (195 * PT, 165 * PT),
                    b"loop below" => (cv.out, cv.inn) = (285 * PT, 255 * PT),
                    _ => {}
                }
                a.is_loop = true;
            }
            b"crossing over" => a.crossing = true,
            b"overlay" => a.overlay = true,
            // A node's own `auto` and anchor decide; `every label` sets `auto` for each, after these.
            b"auto" | b"left" | b"right" | b"above" | b"below" if vs.is_none() => {}
            b"auto" if vs.is_some_and(|v| v.get(buf) == b"false") => {}
            b"solid" => self.set_dash(a, Dash::default()),
            b"->" => a.tips = [TipName::Cap, TipName::PlainTo],
            b"<-" => a.tips = [TipName::PlainTo, TipName::Cap],
            b"<->" => a.tips = [TipName::PlainTo, TipName::PlainTo],
            b"-" => a.tips = [TipName::Cap, TipName::Cap],
            b"double" => {
                a.double_seen = true;
                // It turns double lines on and leaves `\tikz@double@setup` as it is: TikZ's 0.6pt unless `double line` set it.
                if a.double_tikz.is_none() && !a.double {
                    a.double_tikz = Some(dec(b"0.6"));
                }
                if let Some(v) = vs {
                    a.double_color = Some(self.color(v)?);
                }
            }
            b"double distance" => a.double_tikz = Some(self.dim(need(vs)?, a.lw)?),
            b"pos" => a.pos = self.dim(need(vs)?, a.lw)?,
            b"swap" | b"'" => a.swap = !a.swap,
            b"sloped" => a.sloped = true,
            b"allow upside down" => a.upside = true,
            b"labels" => {
                let v = need(vs)?;
                let n = usize::from(a.own_every_label_n);
                if n == a.own_every_label.len() {
                    return Err(Unsupported::new("too many labels= on one arrow", ks));
                }
                a.own_every_label[n] = v;
                a.own_every_label_n += 1;
            }
            _ if vs.is_none() && (color::looks_like_color(k) || self.keys.color(k).is_some() || self.keys.color_pgf_cannot_set(k)) => {
                let c = self.color(ks)?;
                a.color = Some(c);
                a.text_color = Some(c);
                self.o.opts.push(Opt::Color(c));
            }
            _ => return Err(Unsupported::new("unknown arrow key", ks)),
        }
        Ok(())
    }

    /// What a key of an arrow, a label or a cell says: `/tikz/<name>`, or else it is in tikz-cd's directory or reached from it.
    fn local_key(&self, ks: Span, unknown: &'static str) -> R<(bool, Span)> {
        let k = ks.get(self.buf);
        let (dir, off) = place(k, if self.in_tikz { Dir::Tikz } else { Dir::Cd });
        let name = Span::new(ks.start as usize + off, ks.end as usize);
        match dir {
            Dir::Cd => Ok((false, name)),
            Dir::Tikz => Ok((true, name)),
            Dir::Own | Dir::Pgf => Err(Unsupported::new(unknown, ks)),
        }
    }

    /// `/tikz/<k>` where tikz-cd has a `<k>` that does something else.
    fn tikz_key_of_cd_name(&mut self, a: &mut ArrowSpec, k: &[u8], ks: Span) -> R {
        match k {
            b"dashed" => self.set_dash(a, Dash::two(3 * PT, 3 * PT)),
            _ => return Err(Unsupported::new("a TikZ key under a name of tikz-cd's", ks)),
        }
        Ok(())
    }

    /// A key the document defined by code, as far as that code is simple.
    fn code_on_arrow(&mut self, a: &mut ArrowSpec, body: Span, ks: Span) -> R {
        let b = body.get(self.buf);
        if same_text(b, b"\\pgfsetarrowsstart{tikzcd to}") {
            a.tips[0] = TipName::To;
            return Ok(());
        }
        if same_text(b, b"\\pgfsetarrowsstart{Implies}") {
            a.tips[0] = TipName::Implies;
            return Ok(());
        }
        Err(Unsupported::new("key defined by code", ks))
    }

    fn apply_style_arrow(&mut self, a: &mut ArrowSpec, first: usize, at: Span, vs: Option<Span>, depth: u32) -> R {
        self.spend_style(at)?;
        let buf = self.buf;
        let crate::parse::UserStyle { name, tikz, body } = self.o.styles[first];
        if same_text(body.get(buf), QUIVER_CURVE) && body.get(buf).starts_with(b"settings") {
            if self.user_code(b"settings", tikz).is_none() {
                self.ask(Span::new(body.start as usize, body.start as usize + 8))?;
            }
            let settings = self.user_code(b"settings", tikz);
            if settings.is_some_and(|d| same_text(d.body.get(buf), QUIVER_SETTINGS)) {
                return self.quiver_curve(a, vs, at);
            }
        }
        let was = self.in_tikz;
        self.in_tikz = tikz || was;
        let mut i = first;
        while i < self.o.styles.len() {
            let d = self.o.styles[i];
            if d.tikz == tikz && d.name.get(buf) == name.get(buf) {
                let list = self.style_text(i, vs, at)?;
                for it in items(buf, list) {
                    self.arrow_item(a, it, depth + 1)?;
                }
            }
            i += 1;
        }
        self.in_tikz = was;
        Ok(())
    }

    /// `curve={height=<h>, pos=<p>}`.  The values stay in the keys until the path is made, so the last of them counts.
    fn quiver_curve(&mut self, a: &mut ArrowSpec, vs: Option<Span>, at: Span) -> R {
        let buf = self.buf;
        let v = vs.ok_or(Unsupported::new("curve without a value", at))?;
        let mut q = QuiverCurve { pos: dec(b"0.35"), height: crate::expr::Value { v: 0, units: false } };
        for it in items(buf, v) {
            let (k, val) = keyval(buf, it);
            let val = val.ok_or(Unsupported::new("curve key without a value", it))?;
            match k.get(buf) {
                b"height" => q.height = self.value(val, a.lw)?,
                b"pos" => {
                    let p = self.value(val, a.lw)?;
                    if p.units {
                        return Err(Unsupported::new("curve key without a value", it));
                    }
                    q.pos = p.v;
                }
                _ => return Err(Unsupported::new("curve key other than height and pos", it)),
            }
        }
        a.quiver = Some(q);
        a.curve = None;
        a.is_loop = false;
        Ok(())
    }

    /// `on a off b ...`, at most four pairs.
    fn dash_pattern(&self, v: Span, lw: Sp) -> R<Dash> {
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
            d.v[usize::from(d.n)] = pt(self.dim(trim(buf, Span::new(start, i)), lw)?);
            d.n += 1;
        }
        if d.n == 0 || d.n % 2 != 0 {
            return Err(bad);
        }
        Ok(d)
    }

    /// A list of `xshift`, `yshift` and `shift`.
    fn translation_list(&self, list: Span, lw: Sp) -> R<Xy> {
        let buf = self.buf;
        let mut t = (0, 0);
        for it in items(buf, list) {
            let (k, v) = keyval(buf, it);
            let bad = Unsupported::new("transform other than xshift, yshift and shift", it);
            let v = v.ok_or(bad)?;
            match k.get(buf) {
                b"xshift" => t.0 += self.dim(v, lw)?,
                b"yshift" => t.1 += self.dim(v, lw)?,
                b"shift" => {
                    let d = self.coordinate(v, lw)?;
                    t = (t.0 + d.0, t.1 + d.1);
                }
                _ => return Err(bad),
            }
        }
        Ok(t)
    }

    /// `[<transform>]<anchor>`, both parts optional (spec 5.4).
    fn anchor_spec(&self, v: Span, lw: Sp) -> R<(Option<Xy>, Option<Anchor>)> {
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
        let ex = font_sp(self.font.ex);
        self.o.labels.push(LabelSpec {
            text,
            font: LabelFont::Script,
            color: None,
            raw_swap: swap,
            raw_opts: opts,
            ixs: half(ex),
            iys: half(ex),
            outer: None,
            min_w: PT,
            min_h: PT,
            pos: PT / 2,
            swap: false,
            auto: true,
            anchor: Anchor::Center,
            asym: false,
            fill: None,
            name: Span::EMPTY,
            alias: Span::EMPTY,
            sloped: false,
            upside: false,
            overlay: false,
            xforms: (0, 0),
            opts: (0, 0),
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
            l.upside = a.upside;
            l.color = a.text_color;
            let at = self.o.xforms.len() as u32;
            let opts_at = self.o.opts.len() as u32;
            let mut i = 0;
            while i < self.o.every_label.len() {
                let sp = self.o.every_label[i];
                for it in items(buf, sp) {
                    self.label_item(&mut l, a, it, 0)?;
                }
                i += 1;
            }
            let n_own = usize::from(a.own_every_label_n);
            for n in 0..=n_own {
                // `phantom` is `labels={font=,anchor=center}`: it takes its place among this arrow's `labels=`, after the diagram's.
                // The last one is enough, as each sets the same three things.
                if a.phantom && n == usize::from(a.phantom_at) {
                    l.font = LabelFont::Text;
                    l.auto = false;
                    l.anchor = Anchor::Center;
                }
                if n == n_own {
                    break;
                }
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
            l.opts = (opts_at, self.o.opts.len() as u32);
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
        let (tikz, ks) = self.local_key(ks, "unknown label key")?;
        let k = ks.get(buf);
        if let Some(first) = self.user_style(k, tikz) {
            self.spend_style(ks)?;
            let crate::parse::UserStyle { name, tikz, .. } = self.o.styles[first];
            let was = self.in_tikz;
            self.in_tikz = tikz || was;
            let mut i = first;
            while i < self.o.styles.len() {
                let d = self.o.styles[i];
                if d.tikz == tikz && d.name.get(buf) == name.get(buf) {
                    let list = self.style_text(i, vs, ks)?;
                    for it in items(buf, list) {
                        self.label_item(l, a, it, depth + 1)?;
                    }
                }
                i += 1;
            }
            self.in_tikz = was;
            return Ok(());
        }
        if self.user_code(k, tikz).is_some() {
            return Err(Unsupported::new("key defined by code", ks));
        }
        self.ask(ks)?;
        // A node's transforms are TikZ's, the rest of tikz-cd's names are not there.
        if tikz && is_builtin_key(k) && !matches!(k, b"xshift" | b"yshift" | b"shift") {
            return Err(Unsupported::new("a TikZ key under a name of tikz-cd's", ks));
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
                        b"above" => (0, d),
                        b"below" => (0, -d),
                        b"left" => (-d, 0),
                        _ => (d, 0),
                    };
                    self.o.xforms.push(LabelXform::Shift(t));
                }
            }
            b"xshift" => {
                let d = self.dim(need(vs)?, a.lw)?;
                self.o.xforms.push(LabelXform::Shift((d, 0)));
            }
            b"yshift" => {
                let d = self.dim(need(vs)?, a.lw)?;
                self.o.xforms.push(LabelXform::Shift((0, d)));
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
            b"overlay" => l.overlay = true,
            b"scale" => {
                let d = self.dim(need(vs)?, a.lw)?;
                self.o.xforms.push(LabelXform::Scale(d));
            }
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
                let bg = self.background(item)?;
                l.fill = Some(bg);
                self.o.opts.push(Opt::Fill(bg));
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
            b"text" => l.color = Some(self.color(need(vs)?)?),
            b"color" => {
                let c = self.color(need(vs)?)?;
                l.color = Some(c);
                self.o.opts.push(Opt::SetColor(c));
            }
            b"fill" => {
                let v = need(vs)?;
                if v.get(buf) == b"none" {
                    l.fill = None;
                } else {
                    let c = self.color(v)?;
                    l.fill = Some(c);
                    self.o.opts.push(Opt::Fill(c));
                }
            }
            _ if vs.is_none() && (color::looks_like_color(k) || self.keys.color(k).is_some() || self.keys.color_pgf_cannot_set(k)) => {
                let c = self.color(ks)?;
                l.color = Some(c);
                self.o.opts.push(Opt::Color(c));
            }
            _ => return Err(Unsupported::new("unknown label key", ks)),
        }
        Ok(())
    }

    // --------------------------------------------------------------------------------------------------------- cells

    /// Whether a key may set the fields `bits` of a cell's options, and a note that one did.  In `every cell` itself, what a `nodes=`
    /// before it set stays: `nodes=` appends to `every node`, which runs as the node starts, after all of `every cell` (spec 5.3).
    pub(crate) fn cell_sets(&mut self, o: &CellOpts, bits: u8) -> bool {
        self.cell_written |= bits;
        !self.in_every_cell || o.nodes_set & bits == 0
    }

    pub(crate) fn cell_list(&mut self, o: &mut CellOpts, list: Span, depth: u32) -> R {
        let buf = self.buf;
        if depth > MAX_DEPTH {
            return Err(Unsupported::new("key lists nested too deeply", list));
        }
        for it in items(buf, list) {
            let (ks, vs) = keyval(buf, it);
            let (tikz, ks) = self.local_key(ks, "unknown cell key")?;
            let k = ks.get(buf);
            if let Some(first) = self.user_style(k, tikz) {
                self.spend_style(ks)?;
                let crate::parse::UserStyle { name, tikz, .. } = self.o.styles[first];
                let was = self.in_tikz;
                self.in_tikz = tikz || was;
                let mut i = first;
                while i < self.o.styles.len() {
                    let d = self.o.styles[i];
                    if d.tikz == tikz && d.name.get(buf) == name.get(buf) {
                        let list = self.style_text(i, vs, ks)?;
                        self.cell_list(o, list, depth + 1)?;
                    }
                    i += 1;
                }
                self.in_tikz = was;
                continue;
            }
            if self.user_code(k, tikz).is_some() {
                return Err(Unsupported::new("key defined by code", ks));
            }
            self.ask(ks)?;
            let need = |v: Option<Span>| v.ok_or(Unsupported::new("key needs a value", ks));
            match k {
                b"inner sep" => {
                    let d = self.dim(need(vs)?, self.o.s.cell_lw)?;
                    if self.cell_sets(o, CELL_IXS) {
                        o.ixs = d;
                    }
                    if self.cell_sets(o, CELL_IYS) {
                        o.iys = d;
                    }
                }
                b"inner xsep" => {
                    let d = self.dim(need(vs)?, self.o.s.cell_lw)?;
                    if self.cell_sets(o, CELL_IXS) {
                        o.ixs = d;
                    }
                }
                b"inner ysep" => {
                    let d = self.dim(need(vs)?, self.o.s.cell_lw)?;
                    if self.cell_sets(o, CELL_IYS) {
                        o.iys = d;
                    }
                }
                // Among a node's own keys, or in `nodes=` itself, it is appended to `every node` once that has run for the node, and
                // so reaches none of the diagram's.
                b"nodes" if !self.in_every_cell => return Err(Unsupported::new("nodes= where every node has run", ks)),
                b"nodes" => {
                    let at = self.o.opts.len();
                    let color = o.color;
                    let written = std::mem::take(&mut self.cell_written);
                    self.in_every_cell = false;
                    self.cell_list(o, unbrace(buf, need(vs)?), depth + 1)?;
                    self.in_every_cell = true;
                    let by_nodes = self.cell_written;
                    self.cell_written |= written;
                    o.nodes_set |= by_nodes;
                    let crate::parse::ParseOut { opts, node_opts, .. } = &mut *self.o;
                    node_opts.extend(opts.drain(at..));
                    if o.color != color {
                        o.nodes_color = o.color;
                        o.color = color;
                    }
                }
                b"font" => {
                    if self.cell_sets(o, CELL_FONT) {
                        o.font = vs.unwrap_or(Span::EMPTY);
                    }
                }
                // `every cell` is not the node's: this leaves the stroke's colour and asks for no border.
                b"draw" if self.in_every_cell => {
                    if let Some(v) = vs.filter(|v| v.get(buf) != b"none") {
                        let c = self.color(v)?;
                        self.o.opts.push(Opt::Stroke(c));
                    }
                }
                b"draw" => match vs {
                    None => o.draw = true,
                    Some(v) if v.get(buf) == b"none" => o.draw = false,
                    Some(v) => {
                        o.draw = true;
                        let c = self.color(v)?;
                        self.o.opts.push(Opt::Stroke(c));
                    }
                },
                b"rounded corners" => {
                    let r = match vs {
                        None => 4 * PT,
                        Some(v) => self.dim(v, self.o.s.cell_lw)?,
                    };
                    if self.cell_sets(o, CELL_ROUNDED) {
                        o.rounded = r;
                    }
                }
                b"sharp corners" => {
                    if self.cell_sets(o, CELL_ROUNDED) {
                        o.rounded = 0;
                    }
                }
                b"alias" => {
                    let v = need(vs)?;
                    if self.cell_sets(o, CELL_ALIAS) {
                        o.alias = v;
                    }
                }
                b"text height" => {
                    let d = self.dim(need(vs)?, self.o.s.cell_lw)?;
                    if self.cell_sets(o, CELL_HEIGHT) {
                        o.text_height = Some(d);
                    }
                }
                b"text depth" => {
                    let d = self.dim(need(vs)?, self.o.s.cell_lw)?;
                    if self.cell_sets(o, CELL_DEPTH) {
                        o.text_depth = Some(d);
                    }
                }
                b"text" => o.color = Some(self.color(need(vs)?)?),
                b"color" => {
                    let c = self.color(need(vs)?)?;
                    o.color = Some(c);
                    self.o.opts.push(Opt::SetColor(c));
                }
                _ if vs.is_none() && (color::looks_like_color(k) || self.keys.color(k).is_some() || self.keys.color_pgf_cannot_set(k)) => {
                    let c = self.color(ks)?;
                    o.color = Some(c);
                    self.o.opts.push(Opt::Color(c));
                }
                _ => return Err(Unsupported::new("unknown cell key", ks)),
            }
        }
        Ok(())
    }
}
