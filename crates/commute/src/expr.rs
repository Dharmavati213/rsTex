//! The subset of pgfmath that tikz-cd documents use for lengths and numbers: `2.4em`, `10pt+1ex`, `-(1)`, `.5*1ex`,
//! `7\pgflinewidth`, `rule_thickness`.  Anything else is `None`, which the caller turns into Unsupported.

use crate::FontParams;
use crate::scan::{Span, is_letter, is_ws};

/// Unit factors in points, as TeX's conversions print them.
const CM: f64 = 28.45274;
const MM: f64 = 2.84527;
const BP: f64 = 1.00375;
const IN: f64 = 72.27;
const PC: f64 = 12.0;
const SP: f64 = 1.0 / 65536.0;

/// The value of an expression.  `units` is `\ifpgfmathunitsdeclared`: `shift left=2` differs from `shift left=2pt`.
#[derive(Copy, Clone, Debug, PartialEq)]
pub(crate) struct Value {
    pub v: f64,
    pub units: bool,
}

pub(crate) struct Ctx {
    pub font: FontParams,
    /// `\pgflinewidth` at key time.
    pub lw: f64,
}

const MAX_DEPTH: u32 = 32;

struct P<'a> {
    b: &'a [u8],
    i: usize,
    ctx: &'a Ctx,
}

/// Evaluate `s`; `None` if it is not in the subset or divides by zero.
pub(crate) fn eval(buf: &[u8], s: Span, ctx: &Ctx) -> Option<Value> {
    let b = s.get(buf);
    let mut p = P { b, i: 0, ctx };
    let v = p.expr(0)?;
    p.ws();
    if p.i == b.len() { Some(v) } else { None }
}

impl P<'_> {
    fn ws(&mut self) {
        while self.i < self.b.len() && is_ws(self.b[self.i]) {
            self.i += 1;
        }
    }

    fn peek(&self) -> u8 {
        self.b.get(self.i).copied().unwrap_or(0)
    }

    fn expr(&mut self, depth: u32) -> Option<Value> {
        let mut a = self.term(depth)?;
        loop {
            self.ws();
            match self.peek() {
                b'+' => {
                    self.i += 1;
                    let t = self.term(depth)?;
                    a = Value { v: a.v + t.v, units: a.units || t.units };
                }
                b'-' => {
                    self.i += 1;
                    let t = self.term(depth)?;
                    a = Value { v: a.v - t.v, units: a.units || t.units };
                }
                _ => return Some(a),
            }
        }
    }

    fn term(&mut self, depth: u32) -> Option<Value> {
        let mut a = self.factor(depth)?;
        loop {
            self.ws();
            match self.peek() {
                b'*' => {
                    self.i += 1;
                    let t = self.factor(depth)?;
                    a = Value { v: a.v * t.v, units: a.units || t.units };
                }
                b'/' => {
                    self.i += 1;
                    let t = self.factor(depth)?;
                    if t.v == 0.0 {
                        return None;
                    }
                    a = Value { v: a.v / t.v, units: a.units || t.units };
                }
                _ => return Some(a),
            }
        }
    }

    fn factor(&mut self, depth: u32) -> Option<Value> {
        if depth >= MAX_DEPTH {
            return None;
        }
        self.ws();
        match self.peek() {
            b'+' => {
                self.i += 1;
                self.factor(depth + 1)
            }
            b'-' => {
                self.i += 1;
                let t = self.factor(depth + 1)?;
                Some(Value { v: -t.v, units: t.units })
            }
            b'(' => {
                self.i += 1;
                let v = self.expr(depth + 1)?;
                self.ws();
                if self.peek() != b')' {
                    return None;
                }
                self.i += 1;
                Some(v)
            }
            b'\\' => self.linewidth(1.0),
            b'0'..=b'9' | b'.' => self.number(),
            c if is_letter(c) => self.word(),
            _ => None,
        }
    }

    /// At `\pgflinewidth`, which multiplies the number before it when written `7\pgflinewidth`.
    fn linewidth(&mut self, k: f64) -> Option<Value> {
        const NAME: &[u8] = b"\\pgflinewidth";
        if self.b.get(self.i..self.i + NAME.len()) != Some(NAME) {
            return None;
        }
        self.i += NAME.len();
        if is_letter(self.peek()) {
            return None;
        }
        Some(Value { v: k * self.ctx.lw, units: true })
    }

    fn number(&mut self) -> Option<Value> {
        // All digits over one power of ten, so that up to 15 significant digits give the nearest double.
        let mut mant = 0.0f64;
        let mut den = 1.0f64;
        let mut digits = 0u32;
        while self.peek().is_ascii_digit() {
            mant = mant * 10.0 + f64::from(self.peek() - b'0');
            self.i += 1;
            digits += 1;
        }
        if self.peek() == b'.' {
            self.i += 1;
            while self.peek().is_ascii_digit() {
                mant = mant * 10.0 + f64::from(self.peek() - b'0');
                den *= 10.0;
                self.i += 1;
                digits += 1;
            }
        }
        if digits == 0 {
            return None;
        }
        let v = mant / den;
        self.ws();
        if self.peek() == b'\\' {
            return self.linewidth(v);
        }
        let f = self.ctx.font;
        let two = self.b.get(self.i..self.i + 2);
        let factor = match two {
            Some(b"pt") => 1.0,
            Some(b"cm") => CM,
            Some(b"mm") => MM,
            Some(b"bp") => BP,
            Some(b"in") => IN,
            Some(b"pc") => PC,
            Some(b"sp") => SP,
            Some(b"ex") => f.ex,
            Some(b"em") => f.em,
            _ => return Some(Value { v, units: false }),
        };
        if self.b.get(self.i + 2).copied().is_some_and(is_letter) {
            return None;
        }
        self.i += 2;
        Some(Value { v: v * factor, units: true })
    }

    fn word(&mut self) -> Option<Value> {
        let start = self.i;
        while is_letter(self.peek()) || self.peek() == b'_' {
            self.i += 1;
        }
        let f = self.ctx.font;
        match &self.b[start..self.i] {
            b"rule_thickness" => Some(Value { v: f.rt, units: true }),
            b"axis_height" => Some(Value { v: f.ax, units: true }),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> Ctx {
        Ctx { font: FontParams { ex: 4.0, em: 10.0, rt: 0.4, ax: 2.5 }, lw: 0.5 }
    }

    fn ev(s: &str) -> Option<(f64, bool)> {
        let b = s.as_bytes();
        eval(b, Span::new(0, b.len()), &ctx()).map(|v| (v.v, v.units))
    }

    #[test]
    fn lengths() {
        assert_eq!(ev("3pt"), Some((3.0, true)));
        assert_eq!(ev("2.5em"), Some((25.0, true)));
        assert_eq!(ev("10pt+1ex"), Some((14.0, true)));
        assert_eq!(ev("+0.5ex"), Some((2.0, true)));
        assert_eq!(ev(".5*1ex"), Some((2.0, true)));
        assert_eq!(ev("7\\pgflinewidth"), Some((3.5, true)));
        assert_eq!(ev("\\pgflinewidth"), Some((0.5, true)));
        assert_eq!(ev("2*rule_thickness"), Some((0.8, true)));
        assert_eq!(ev("(1pt+1pt)/2"), Some((1.0, true)));
    }

    #[test]
    fn plain_numbers_have_no_units() {
        assert_eq!(ev("-(1)"), Some((-1.0, false)));
        assert_eq!(ev("0.5"), Some((0.5, false)));
        assert_eq!(ev("2"), Some((2.0, false)));
    }

    #[test]
    fn everything_else_is_refused() {
        for s in ["", "1pt+", "foo", "1foo", "\\the\\dimen0", "1/0", "(1", "1pt)", "2^3", "1ptx", "--"] {
            assert_eq!(ev(s), None, "{s}");
        }
        let deep = "(".repeat(100_000);
        assert_eq!(ev(&deep), None);
        let deep = "-".repeat(100_000);
        assert_eq!(ev(&deep), None);
    }

    #[test]
    fn decimal_digits_round_to_the_nearest_double_for_short_numbers() {
        assert_eq!(ev("2.4"), Some((2.4, false)));
        assert_eq!(ev("0.56"), Some((0.56, false)));
        assert_eq!(ev("1.15"), Some((1.15, false)));
    }
}
