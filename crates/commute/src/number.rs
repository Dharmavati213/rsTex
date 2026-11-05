//! Printing numbers without allocating, the way TeX and pgf print them.  Same bytes on every platform: only integer
//! arithmetic after the rounding to scaled points.

/// A length in points as a scaled integer, for a number already known to be small enough.
#[inline]
fn to_sp(x: f64) -> i64 {
    // `as` saturates and takes NaN to 0; the clamp keeps later arithmetic from overflowing.
    let v = (x * 65536.0 + if x < 0.0 { -0.5 } else { 0.5 }) as i64;
    v.clamp(-(1i64 << 46), 1i64 << 46)
}

/// TeX's `print_scaled` (§103): the shortest decimal that reads back as the same scaled value, at least one decimal.
pub(crate) fn print_scaled(out: &mut Vec<u8>, sp: i64) {
    let mut s = sp;
    if s < 0 {
        out.push(b'-');
        s = -s;
    }
    print_uint(out, (s >> 16) as u64);
    out.push(b'.');
    const UNITY: i64 = 65536;
    s = 10 * (s % UNITY) + 5;
    let mut delta: i64 = 10;
    loop {
        if delta > UNITY {
            s += 0o100000 - 50000; // round the last digit
        }
        out.push(b'0' + (s / UNITY) as u8);
        s = 10 * (s % UNITY);
        delta *= 10;
        if s <= delta {
            break;
        }
    }
}

fn print_uint(out: &mut Vec<u8>, mut v: u64) {
    let mut tmp = [0u8; 20];
    let mut n = tmp.len();
    loop {
        n -= 1;
        tmp[n] = b'0' + (v % 10) as u8;
        v /= 10;
        if v == 0 {
            break;
        }
    }
    out.extend_from_slice(&tmp[n..]);
}

/// How `\pgf@sys@bp` turns a length in points into what it writes in the PDF.  This is the one place so far where a version of PGF changed what the path has to draw: spec section 2.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum BpRule {
    /// PGF before 3.1.12: `\pgf@x=0.99627\pgf@x`.
    #[default]
    Factor,
    /// PGF 3.1.12: `\dimexpr\the\numexpr(2*x+1)*400/803\relax sp`, with `-1` for a negative `x`.
    Exact,
}

impl BpRule {
    /// The rule of a version of PGF, by what `\pgfversion` says: for where only that can be read, as with the library alone, when no PGF is loaded.
    pub fn of_version(version: &[u8]) -> BpRule {
        let mut parts = version.split(|&c| c == b'.').map(|p| p.iter().take_while(|c| c.is_ascii_digit()).fold(0u32, |n, &c| n.saturating_mul(10).saturating_add((c - b'0') as u32)));
        let have = (parts.next().unwrap_or(0), parts.next().unwrap_or(0), parts.next().unwrap_or(0));
        if have >= (3, 1, 12) { BpRule::Exact } else { BpRule::Factor }
    }

    /// The scaled points that `\pgf@sys@bp` leaves in `\pgf@x`, for a length in scaled points.
    #[inline]
    pub fn apply(self, sp: i64) -> i64 {
        match self {
            // TeX holds the factor as 65292/65536, and truncates.
            BpRule::Factor => (sp * 65292) / 65536,
            // `\numexpr` takes `a*b/c` as one step, and rounds half away from zero.  803 is odd, so it never comes to a half.
            BpRule::Exact => {
                let twice = if sp < 0 { -2 * sp + 1 } else { 2 * sp + 1 };
                let q = (twice * 2 * 400 + 803) / (2 * 803);
                if sp < 0 { -q } else { q }
            }
        }
    }
}

/// A length in points as pgf writes it in the PDF, printed by `print_scaled` (spec section 2).
#[inline]
pub(crate) fn pgf_bp(out: &mut Vec<u8>, rule: BpRule, pt: f64) {
    print_scaled(out, rule.apply(to_sp(pt)));
}

/// A dimensionless number, such as a CTM entry, as pgf writes it: `print_scaled` of the number taken as points.
#[inline]
pub(crate) fn pgf_num(out: &mut Vec<u8>, v: f64) {
    print_scaled(out, to_sp(v));
}

/// Thousandths, as pdfTeX's `pdf_print_real` writes them with three decimal digits.
pub(crate) fn print_milli(out: &mut Vec<u8>, q: i64) {
    if q < 0 {
        out.push(b'-');
    }
    let a = q.unsigned_abs();
    print_uint(out, a / 1000);
    let mut f = a % 1000;
    if f != 0 {
        out.push(b'.');
        let mut div = 100;
        while f != 0 {
            out.push(b'0' + (f / div) as u8);
            f %= div;
            div /= 10;
        }
    }
}

/// A fixed-point decimal with at most `digits` decimals, trailing zeros removed: pdfTeX's text positions and xcolor's
/// colour components.  `digits` is at most 9.
pub(crate) fn fixed(out: &mut Vec<u8>, v: f64, digits: u32) {
    let scale = 10i64.pow(digits.min(9));
    let scaled = (v * scale as f64 + if v < 0.0 { -0.5 } else { 0.5 }) as i64;
    let scaled = scaled.clamp(-(1i64 << 60), 1i64 << 60);
    if scaled < 0 {
        out.push(b'-');
    }
    let abs = scaled.unsigned_abs();
    print_uint(out, abs / scale as u64);
    let mut frac = abs % scale as u64;
    if frac != 0 {
        out.push(b'.');
        let mut div = scale as u64 / 10;
        while frac != 0 && div != 0 {
            out.push(b'0' + (frac / div) as u8);
            frac %= div;
            div /= 10;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scaled(sp: i64) -> String {
        let mut o = Vec::new();
        print_scaled(&mut o, sp);
        String::from_utf8(o).unwrap()
    }

    fn bp(pt: f64) -> String {
        let mut o = Vec::new();
        pgf_bp(&mut o, BpRule::Factor, pt);
        String::from_utf8(o).unwrap()
    }

    fn exact(sp: i64) -> String {
        scaled(BpRule::Exact.apply(sp))
    }

    fn fx(v: f64, d: u32) -> String {
        let mut o = Vec::new();
        fixed(&mut o, v, d);
        String::from_utf8(o).unwrap()
    }

    #[test]
    fn print_scaled_like_tex() {
        assert_eq!(scaled(0), "0.0");
        assert_eq!(scaled(65536), "1.0");
        assert_eq!(scaled(-65536), "-1.0");
        assert_eq!(scaled(32768), "0.5");
        assert_eq!(scaled(1), "0.00002");
        assert_eq!(scaled(26214), "0.4");
        // \maxdimen
        assert_eq!(scaled(1073741823), "16383.99998");
    }

    #[test]
    fn pgf_writes_points_times_0_99627() {
        assert_eq!(bp(0.39998), "0.39848");
        assert_eq!(bp(0.4), "0.3985");
        assert_eq!(bp(0.0), "0.0");
        assert_eq!(bp(0.8), "0.79701");
        assert_eq!(bp(1.0), "0.99628");
    }

    /// What `pgfsys.code.tex` of PGF 3.1.12 writes, as pdflatex ran it: `tests/cd/native/gen_table.py` holds both rules against the tree's own `\pgf@sys@bp` over many more.
    #[test]
    fn pgf_3_1_12_writes_points_times_800_over_803() {
        for (sp, written) in [
            (0, "0.0"),
            (1, "0.00002"),
            (-1, "-0.00002"),
            (2, "0.00003"),
            (26214, "0.39851"),
            (32768, "0.49814"),
            (65536, "0.99628"),
            (-65536, "-0.99628"),
            (12345, "0.18767"),
            (100000, "1.52019"),
            (805306, "12.2421"),
            (170393, "2.59029"),
            (1073741823, "16322.78954"),
            (-1073741823, "-16322.78954"),
        ] {
            assert_eq!(exact(sp), written);
        }
    }

    #[test]
    fn a_version_says_which_rule() {
        for v in [&b"3.1.12"[..], b"3.1.13", b"3.2", b"3.2.0a", b"4.0.0", b"3.1.12a", b"10.0"] {
            assert_eq!(BpRule::of_version(v), BpRule::Exact, "{}", String::from_utf8_lossy(v));
        }
        for v in [&b"3.1.11a"[..], b"3.1.11", b"3.1.10", b"3.1", b"3.0.1a", b"2.10", b""] {
            assert_eq!(BpRule::of_version(v), BpRule::Factor, "{}", String::from_utf8_lossy(v));
        }
    }

    #[test]
    fn fixed_trims_zeros() {
        assert_eq!(fx(0.5, 3), "0.5");
        assert_eq!(fx(1.0, 3), "1");
        assert_eq!(fx(-2.652, 3), "-2.652");
        assert_eq!(fx(0.0, 3), "0");
        assert_eq!(fx(0.30000000000000004, 5), "0.3");
        assert_eq!(fx(0.08, 5), "0.08");
    }

    #[test]
    fn absurd_values_do_not_panic() {
        for v in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, 1e300, -1e300] {
            let mut o = Vec::new();
            pgf_bp(&mut o, BpRule::Factor, v);
            pgf_bp(&mut o, BpRule::Exact, v);
            pgf_num(&mut o, v);
            fixed(&mut o, v, 3);
        }
    }
}
