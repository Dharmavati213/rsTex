//! Arithmetic that gives the same bits on every platform.
//!
//! `f64::sin`, `cos`, `tan` and `hypot` go to the platform's libm, whose last bit differs between glibc, macOS and
//! the Windows runtime, and output has to be byte-identical everywhere (DESIGN section 12).  IEEE 754 fixes `+ - * /`
//! and `sqrt` exactly, and rustc does not contract them into fused operations, so everything here is built from those.

use std::f64::consts::PI;

use crate::geom::Pt;

/// `sqrt(x*x + y*y)`.  Overflows to infinity instead of scaling, which is harmless here: lengths are at most a few
/// thousand points unless the input is absurd, and an absurd input only has to not panic.
#[inline]
pub(crate) fn hypot(x: f64, y: f64) -> f64 {
    (x * x + y * y).sqrt()
}

/// sin and cos of `r` radians for `|r| <= pi/4`: Taylor polynomials through degree 17 and 16, whose first omitted terms
/// are under 1e-17 on that interval.
#[inline]
fn kernel(r: f64) -> (f64, f64) {
    const S: [f64; 8] = [
        -1.0 / 6.0,
        1.0 / 120.0,
        -1.0 / 5040.0,
        1.0 / 362_880.0,
        -1.0 / 39_916_800.0,
        1.0 / 6_227_020_800.0,
        -1.0 / 1_307_674_368_000.0,
        1.0 / 355_687_428_096_000.0,
    ];
    const C: [f64; 8] = [
        -0.5,
        1.0 / 24.0,
        -1.0 / 720.0,
        1.0 / 40_320.0,
        -1.0 / 3_628_800.0,
        1.0 / 479_001_600.0,
        -1.0 / 87_178_291_200.0,
        1.0 / 20_922_789_888_000.0,
    ];
    let z = r * r;
    let horner = |k: &[f64; 8]| k.iter().rev().fold(0.0, |acc, &x| x + z * acc);
    (r + r * z * horner(&S), 1.0 + z * horner(&C))
}

/// sin and cos of an angle in degrees.  Exact at multiples of 90.  A non-finite angle gives NaN.
pub(crate) fn sincos_deg(deg: f64) -> (f64, f64) {
    if !deg.is_finite() {
        return (f64::NAN, f64::NAN);
    }
    let a = deg - 360.0 * (deg / 360.0).floor();
    // Nearest multiple of 90 and the remainder, at most 45 degrees either way.
    let q = (a / 90.0 + 0.5).floor();
    let rem = a - q * 90.0;
    let (s, c) = kernel(rem * (PI / 180.0));
    match (q as i64).rem_euclid(4) {
        0 => (s, c),
        1 => (c, -s),
        2 => (-s, -c),
        _ => (-c, s),
    }
}

/// tan of an angle in degrees, for the arc coefficient of the Latin Modern hook.
pub(crate) fn tan_deg(deg: f64) -> f64 {
    let (s, c) = sincos_deg(deg);
    s / c
}

/// Rotate `v` by `deg` degrees counterclockwise.
#[inline]
pub(crate) fn rot(v: Pt, deg: f64) -> Pt {
    let (s, c) = sincos_deg(deg);
    Pt::new(c * v.x - s * v.y, s * v.x + c * v.y)
}

/// Unit vector along `v`; `(0, 1)` for the zero vector, which is what pgf's `atan2(0,0)` path gives (spec 5.4, 9.3).
#[inline]
pub(crate) fn norm(v: Pt) -> Pt {
    let l = hypot(v.x, v.y);
    if l == 0.0 { Pt::new(0.0, 1.0) } else { Pt::new(v.x / l, v.y / l) }
}

/// `floor(a / b)` computed the way Python's float `//` does: through the exact remainder, so that a quotient the
/// division would round up to an integer does not count as reaching it.
#[inline]
pub(crate) fn floor_div(a: f64, b: f64) -> f64 {
    ((a - a % b) / b).round_ties_even()
}

/// Round to five decimals.
#[inline]
pub(crate) fn round5(x: f64) -> f64 {
    (x * 100_000.0 + 0.5).floor() / 100_000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn multiples_of_ninety_are_exact() {
        assert_eq!(sincos_deg(0.0), (0.0, 1.0));
        assert_eq!(sincos_deg(90.0), (1.0, 0.0));
        assert_eq!(sincos_deg(180.0), (0.0, -1.0));
        assert_eq!(sincos_deg(270.0), (-1.0, 0.0));
        assert_eq!(sincos_deg(-90.0), (-1.0, 0.0));
        assert_eq!(sincos_deg(450.0), (1.0, 0.0));
    }

    #[test]
    fn agrees_with_the_platform() {
        let mut d = -720.0f64;
        while d < 720.0 {
            let (s, c) = sincos_deg(d);
            let r = d * PI / 180.0;
            // Far from zero, the radians themselves have rounded by more than this module's own error.
            let tol = if d.abs() <= 90.0 { 5e-16 } else { 4e-15 };
            assert!((s - r.sin()).abs() < tol, "sin {d}: {s} {}", r.sin());
            assert!((c - r.cos()).abs() < tol, "cos {d}: {c} {}", r.cos());
            d += 0.37;
        }
    }

    #[test]
    fn non_finite_angles_do_not_panic() {
        assert!(sincos_deg(f64::NAN).0.is_nan());
        assert!(sincos_deg(f64::INFINITY).1.is_nan());
        let _ = sincos_deg(1e300);
    }

    #[test]
    fn floor_div_matches_python() {
        assert_eq!(floor_div(7.0, 2.0), 3.0);
        assert_eq!(floor_div(6.0, 2.0), 3.0);
        assert_eq!(floor_div(0.5, 2.0), 0.0);
    }

    #[test]
    fn norm_of_zero_points_up() {
        assert_eq!(norm(Pt::new(0.0, 0.0)), Pt::new(0.0, 1.0));
    }
}
