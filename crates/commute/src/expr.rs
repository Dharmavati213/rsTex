//! The subset of pgfmath that tikz-cd documents use for lengths and numbers: `2.4em`, `10pt+1ex`, `-(1)`, `.5*1ex`,
//! `7\pgflinewidth`, `rule_thickness`.  Anything else is `None`, which the caller turns into Unsupported.  The arithmetic
//! is pgfmath's, on what it holds as text: a number with no unit stays the number TeX reads of it.

use crate::FontParams;
use crate::pgfmath::{MAX_DIMEN, PT, Sp, divide, mul};
use crate::scan::{Span, is_letter, is_ws};

/// The value of an expression, as scaled points.  `units` is `\ifpgfmathunitsdeclared`: `shift left=2` differs from
/// `shift left=2pt`.
#[derive(Copy, Clone, Debug, PartialEq)]
pub(crate) struct Value {
    pub v: Sp,
    pub units: bool,
}

pub(crate) struct Ctx {
    pub font: FontParams,
    /// `\pgflinewidth` at key time.
    pub lw: Sp,
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

/// §102 on the digits of `b[from..to]`, TeX's 17 at most.
fn round_digits(b: &[u8], from: usize, to: usize) -> Sp {
    let n = to - from;
    let mut k = n.min(17);
    let mut a: Sp = 0;
    while k > 0 {
        k -= 1;
        a = (a + Sp::from(b[from + k] - b'0') * 131072) / 10;
    }
    (a + 1) / 2
}

/// §107 `xn_over_d`, the quotient and the remainder.
fn xn_over_d_rem(x: Sp, n: Sp, d: Sp) -> (Sp, Sp) {
    (x * n / d, x * n % d)
}

/// §458: `<int>.<frac>` in a physical unit, as TeX converts it.  `num` and `den` from §458.
fn physical(int: Sp, f: Sp, num: Sp, den: Sp) -> Option<Sp> {
    let (mut v, rem) = xn_over_d_rem(int, num, den);
    let f2 = (num * f + PT * rem) / den;
    v += f2 / PT;
    let f3 = f2 % PT;
    if v >= 16384 {
        return None;
    }
    Some(v * PT + f3)
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
                    // `\pgfmath@x=#1pt \pgfmath@y=#2pt \pgfmath@x=\pgfmath@tonumber{\pgfmath@y}\pgfmath@x`
                    a = Value { v: mul(t.v, a.v), units: a.units || t.units };
                }
                b'/' => {
                    self.i += 1;
                    let t = self.factor(depth)?;
                    a = Value { v: divide(a.v, t.v)?, units: a.units || t.units };
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
            b'\\' => self.linewidth(PT),
            b'0'..=b'9' | b'.' => self.number(),
            c if is_letter(c) => self.word(),
            _ => None,
        }
    }

    /// At `\pgflinewidth`, which multiplies the number before it when written `7\pgflinewidth`.
    fn linewidth(&mut self, k: Sp) -> Option<Value> {
        const NAME: &[u8] = b"\\pgflinewidth";
        if self.b.get(self.i..self.i + NAME.len()) != Some(NAME) {
            return None;
        }
        self.i += NAME.len();
        if is_letter(self.peek()) {
            return None;
        }
        Some(Value { v: mul(k, self.ctx.lw), units: true })
    }

    fn number(&mut self) -> Option<Value> {
        let mut int: Sp = 0;
        let mut digits = 0u32;
        while self.peek().is_ascii_digit() {
            int = int.saturating_mul(10).saturating_add(Sp::from(self.peek() - b'0'));
            self.i += 1;
            digits += 1;
        }
        let mut f = 0;
        if self.peek() == b'.' {
            self.i += 1;
            let from = self.i;
            while self.peek().is_ascii_digit() {
                self.i += 1;
                digits += 1;
            }
            f = round_digits(self.b, from, self.i);
        }
        if digits == 0 {
            return None;
        }
        let two = self.b.get(self.i..self.i + 2);
        if two == Some(b"sp") && int <= MAX_DIMEN {
            self.i += 2;
            return Some(Value { v: int, units: true });
        }
        // `Dimension too large` in TeX.
        if int >= 16384 {
            return None;
        }
        let v = int * PT + f;
        self.ws();
        if self.peek() == b'\\' {
            return self.linewidth(v);
        }
        let font = self.ctx.font;
        let two = self.b.get(self.i..self.i + 2);
        let scaled = match two {
            Some(b"pt") => v,
            Some(b"cm") => physical(int, f, 7227, 254)?,
            Some(b"mm") => physical(int, f, 7227, 2540)?,
            Some(b"bp") => physical(int, f, 7227, 7200)?,
            Some(b"in") => physical(int, f, 7227, 100)?,
            Some(b"pc") => physical(int, f, 12, 1)?,
            Some(b"cc") => physical(int, f, 14856, 1157)?,
            Some(b"dd") => physical(int, f, 1238, 1157)?,
            Some(b"ex") => mul(v, crate::font_sp(font.ex)),
            Some(b"em") => mul(v, crate::font_sp(font.em)),
            _ => return Some(Value { v, units: false }),
        };
        if self.b.get(self.i + 2).copied().is_some_and(is_letter) {
            return None;
        }
        self.i += 2;
        if scaled.abs() > MAX_DIMEN {
            return None;
        }
        Some(Value { v: scaled, units: true })
    }

    fn word(&mut self) -> Option<Value> {
        let start = self.i;
        while is_letter(self.peek()) || self.peek() == b'_' {
            self.i += 1;
        }
        let f = self.ctx.font;
        match &self.b[start..self.i] {
            b"rule_thickness" => Some(Value { v: crate::font_sp(f.rt), units: true }),
            b"axis_height" => Some(Value { v: crate::font_sp(f.ax), units: true }),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pgfmath::dec;

    fn ctx() -> Ctx {
        Ctx { font: FontParams { ex: 4.0, em: 10.0, rt: 0.4, ax: 2.5 }, lw: PT / 2 }
    }

    fn ev(s: &str) -> Option<(Sp, bool)> {
        let b = s.as_bytes();
        eval(b, Span::new(0, b.len()), &ctx()).map(|v| (v.v, v.units))
    }

    #[test]
    fn lengths() {
        assert_eq!(ev("3pt"), Some((3 * PT, true)));
        assert_eq!(ev("2.5em"), Some((25 * PT, true)));
        assert_eq!(ev("10pt+1ex"), Some((14 * PT, true)));
        assert_eq!(ev("+0.5ex"), Some((2 * PT, true)));
        assert_eq!(ev(".5*1ex"), Some((2 * PT, true)));
        assert_eq!(ev("7\\pgflinewidth"), Some((7 * PT / 2, true)));
        assert_eq!(ev("\\pgflinewidth"), Some((PT / 2, true)));
        assert_eq!(ev("2*rule_thickness"), Some((2 * crate::font_sp(0.4), true)));
        assert_eq!(ev("(1pt+1pt)/2"), Some((PT, true)));
    }

    /// What TeX makes of them: `\the\dimexpr` under pdftex.
    #[test]
    fn physical_units_as_tex_converts_them() {
        assert_eq!(ev("5mm").unwrap().0, dec(b"14.22636"));
        assert_eq!(ev("1cm").unwrap().0, dec(b"28.45274"));
        assert_eq!(ev("1in").unwrap().0, dec(b"72.26999"));
        assert_eq!(ev("1bp").unwrap().0, dec(b"1.00374"));
        assert_eq!(ev("1pc").unwrap().0, 12 * PT);
        assert_eq!(ev("65536sp").unwrap().0, PT);
    }

    #[test]
    fn plain_numbers_have_no_units() {
        assert_eq!(ev("-(1)"), Some((-PT, false)));
        assert_eq!(ev("0.5"), Some((PT / 2, false)));
        assert_eq!(ev("2"), Some((2 * PT, false)));
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
    fn decimal_digits_as_tex_reads_them() {
        assert_eq!(ev("2.4"), Some((dec(b"2.4"), false)));
        assert_eq!(ev("0.56"), Some((36700, false)));
        assert_eq!(ev("1.15"), Some((75366, false)));
    }
}
