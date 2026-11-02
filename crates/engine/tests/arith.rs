//! arith.rs against the oracle: crates/engine/testdata/arith_oracle.tsv, from tests/oracle/gen_arith_tables.py.
//!
//! Each `replay` below is what the WEB does with the routine under test, so that a row of the table, which
//! the oracle produced through a primitive, can be compared with the Rust routine alone.

use rstex_engine::consts::*;
use rstex_engine::raw::*;
use rstex_engine::state::*;

const TABLE: &str = include_str!("../testdata/arith_oracle.tsv");
const MAXD: i32 = 0o7777777777;
const MAXI: i32 = 0o17777777777;

fn machine() -> &'static mut Tex {
    let t = Tex::alloc();
    t.print_init_key_variables_to_1571();
    t.pool_size = 4000;
    t.set_new_line_char(-1);
    t.selector = NEW_STRING;
    t
}

/// The same generator is in tests/oracle/gen_arith_tables.py.
struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> i64 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        (self.0 >> 33) as i64
    }
    fn nonneg(&mut self) -> i64 {
        let r = self.next();
        r >> (self.next() % 31)
    }
    fn mixed(&mut self) -> i64 {
        let v = self.nonneg();
        if self.next() & 1 != 0 { -v } else { v }
    }
    fn dimen(&mut self) -> i64 {
        let v = self.nonneg() & MAXD as i64;
        if self.next() & 1 != 0 { -v } else { v }
    }
    fn digits(&mut self, most: i64) -> String {
        let n = self.next() % (most + 1);
        (0..n).map(|_| (48 + self.next() % 10) as u8 as char).collect()
    }
}

struct Fnv(u64);

impl Fnv {
    fn new() -> Fnv {
        Fnv(0xcbf29ce484222325)
    }
    fn line(&mut self, s: &str) {
        for &b in s.as_bytes().iter().chain(b"\n") {
            self.0 = (self.0 ^ b as u64).wrapping_mul(0x100000001b3);
        }
    }
    fn hex(&self) -> String {
        format!("{:016x}", self.0)
    }
}

fn int(s: &str) -> i32 {
    s.parse().unwrap_or_else(|_| panic!("bad number in the table: {s:?}"))
}

/// A result of a replay: the value, and whether the oracle would have said so with an error.
struct R(String, bool);

impl R {
    fn show(&self) -> String {
        format!("{}{}", self.0, if self.1 { "!" } else { "" })
    }
    fn line(&self) -> String {
        format!("{} {}", self.0, self.1 as i32)
    }
}

// ---- replays ----

/// §1418 with §1415's report: on an error the register keeps its value.
fn x_over_n(t: &mut Tex, x: i32, n: i32) -> R {
    t.arith_error = false;
    let r = t.x_over_n(x, n);
    if t.arith_error { R(x.to_string(), true) } else { R(r.to_string(), false) }
}

fn mult_int(t: &mut Tex, n: i32, x: i32) -> R {
    t.arith_error = false;
    let r = t.mult_integers(n, x);
    if t.arith_error { R(n.to_string(), true) } else { R(r.to_string(), false) }
}

fn nx_plus_y(t: &mut Tex, n: i32, x: i32) -> R {
    t.arith_error = false;
    let r = t.nx_plus_y(n, x, 0);
    if t.arith_error { R(n.to_string(), true) } else { R(r.to_string(), false) }
}

/// §478: the fraction, of which only 17 digits count.
fn decimal_fraction(t: &mut Tex, digits: &str) -> i32 {
    let mut k = 0;
    for &c in digits.as_bytes() {
        if k < 17 {
            t.dig[k] = (c - b'0') as i32;
            k += 1;
        }
    }
    t.round_decimals(k)
}

/// §486
fn attach_sign(t: &mut Tex, v: i32, negative: bool) -> R {
    let mut v = v;
    let mut err = false;
    if t.arith_error || abs(v) >= 0o10000000000 {
        err = true;
        v = MAXD;
        t.arith_error = false;
    }
    R((if negative { -v } else { v }).to_string(), err)
}

/// §478-§486 for an explicit unit, with §483 if `mag` is given.
fn units(t: &mut Tex, i: i32, digits: &str, unit: &str, mag: Option<i32>) -> R {
    t.arith_error = false;
    let mut f = decimal_fraction(t, digits);
    let mut cur_val = i;
    if let Some(m) = mag {
        // prepare_mag has nothing to say, and §483:
        if m != 1000 {
            cur_val = t.xn_over_d(cur_val, 1000, m);
            f = ((1000 * f as i64 + 65536 * t.remainder as i64) / m as i64) as i32;
            cur_val = sat32(cur_val as i64 + (f / 65536) as i64);
            f %= 65536;
        }
    }
    let conv = match unit {
        "pt" => None,
        "in" => Some((7227, 100)),
        "pc" => Some((12, 1)),
        "cm" => Some((7227, 254)),
        "mm" => Some((7227, 2540)),
        "bp" => Some((7227, 7200)),
        "dd" => Some((1238, 1157)),
        "cc" => Some((14856, 1157)),
        "nd" => Some((685, 642)),
        "nc" => Some((1370, 107)),
        "sp" => return attach_sign(t, cur_val, false),
        _ => panic!("unit {unit}"),
    };
    if let Some((num, denom)) = conv {
        cur_val = t.xn_over_d(cur_val, num, denom);
        f = ((num as i64 * f as i64 + 65536 * t.remainder as i64) / denom as i64) as i32;
        cur_val = sat32(cur_val as i64 + (f / 65536) as i64);
        f %= 65536;
    }
    if cur_val >= 0o40000 {
        t.arith_error = true;
    } else {
        cur_val = cur_val * UNITY + f;
    }
    attach_sign(t, cur_val, false)
}

/// §481
fn internal(t: &mut Tex, negative: bool, a: i32, digits: &str, v: i32) -> R {
    t.arith_error = false;
    let f = decimal_fraction(t, digits);
    let y = t.xn_over_d(v, f, 0o200000);
    let r = t.nx_plus_y(a, v, y);
    attach_sign(t, r, negative)
}

/// §879, §892 and §893: `mu_mult`, on a kern or the width of a glue.
fn mkern(t: &mut Tex, q: i32, x: i32) -> R {
    let cur_mu = t.x_over_n(q, 18);
    let mut n = t.x_over_n(cur_mu, 0o200000);
    let mut f = t.remainder;
    if f < 0 {
        n -= 1;
        f += 0o200000;
    }
    let y = t.xn_over_d(x, f, 0o200000);
    R(t.nx_plus_y(n, x, y).to_string(), false)
}

/// §912.  The box is in an hbox, whose height is not below 0.
fn half(t: &mut Tex, delta: i32, axis: i32) -> R {
    R((axis + t.half(delta)).max(0).to_string(), false)
}

fn scaled(t: &mut Tex, v: i32) -> R {
    let b = t.pool_ptr;
    t.print_scaled(v);
    let s = String::from_utf8(t.str_pool.range(b, t.pool_ptr).to_vec()).unwrap();
    t.pool_ptr = b;
    R(s + "pt", false)
}

fn replay(t: &mut Tex, kind: &str, a: &[&str]) -> R {
    match kind {
        "x_over_n" => x_over_n(t, int(a[0]), int(a[1])),
        "mult_int" => mult_int(t, int(a[0]), int(a[1])),
        "nx_plus_y" => nx_plus_y(t, int(a[0]), int(a[1])),
        "units" => units(t, int(a[0]), a[1], a[2], None),
        "true" => units(t, int(a[1]), a[2], a[3], Some(int(a[0]))),
        "internal" => internal(t, a[0] == "1", int(a[1]), a[2], int(a[3])),
        "mkern" => mkern(t, int(a[0]), int(a[1])),
        "mskip" => mkern(t, int(a[0]), int(a[1])),
        "badness" | "bad_tight" => R(t.badness(int(a[0]), int(a[1])).to_string(), false),
        "half" => half(t, int(a[0]), int(a[1])),
        "scaled" => scaled(t, int(a[0])),
        "decimals" => units(t, 0, a[0], "pt", None),
        _ => panic!("kind {kind}"),
    }
}

/// Every `grid` row of a kind, over the product of its axes.
fn check_grid(kind: &str) {
    let t = machine();
    let mut seen = 0;
    for row in TABLE.lines() {
        let f: Vec<&str> = row.split('\t').collect();
        if f[0] != "grid" || f[1] != kind {
            continue;
        }
        let axes: Vec<Vec<&str>> = f[2..f.len() - 1].iter().map(|a| a.split(',').collect()).collect();
        let want: Vec<&str> = f[f.len() - 1].split(',').collect();
        let mut idx = vec![0usize; axes.len()];
        for w in want.iter() {
            let args: Vec<&str> = idx.iter().zip(&axes).map(|(&i, a)| a[i]).collect();
            let got = replay(t, kind, &args).show();
            assert_eq!(&got, w, "{kind} {args:?}");
            seen += 1;
            for d in (0..idx.len()).rev() {
                idx[d] += 1;
                if idx[d] < axes[d].len() {
                    break;
                }
                idx[d] = 0;
            }
        }
    }
    assert!(seen > 0, "no rows for {kind}");
    Tex::dealloc(t);
}

fn check_sweep(name: &str, f: impl FnOnce(&mut Tex, &mut Fnv) -> usize) {
    let row = TABLE.lines().find(|l| l.starts_with(&format!("sweep\t{name}\t"))).expect("sweep row");
    let w: Vec<&str> = row.split('\t').collect();
    let t = machine();
    let mut h = Fnv::new();
    let n = f(t, &mut h);
    assert_eq!(n.to_string(), w[2], "{name}: the number of rows");
    assert_eq!(h.hex(), w[3], "{name}: run gen_arith_tables.py --dump {name} and compare");
    Tex::dealloc(t);
}

// ---- grids ----

#[test]
fn x_over_n_on_extremes() {
    check_grid("x_over_n");
}
#[test]
fn mult_integers_on_extremes() {
    check_grid("mult_int");
}
#[test]
fn nx_plus_y_on_extremes() {
    check_grid("nx_plus_y");
}
#[test]
fn xn_over_d_with_each_unit() {
    check_grid("units");
}
#[test]
fn xn_over_d_with_true_units() {
    check_grid("true");
}
/// Including `v` beyond `\maxdimen`, where only `xn_over_d`'s i64 keeps its `arith_error`.
#[test]
fn xn_over_d_and_nx_plus_y_on_internal_units() {
    check_grid("internal");
}
/// `n=-1` and `f` near 65536 give `x` and `y` of opposite signs in §105, with `\muskip` beyond `\maxdimen` too.
#[test]
fn nx_plus_y_past_its_bound_through_mskip() {
    check_grid("mskip");
}
/// The remainder of `x_over_n` and of `xn_over_d`, through `\mkern`.
#[test]
fn remainders_through_mkern() {
    check_grid("mkern");
}
#[test]
fn badness_on_extremes() {
    check_grid("badness");
    check_grid("bad_tight");
}
#[test]
fn half_through_vcenter() {
    check_grid("half");
}
#[test]
fn print_scaled_on_extremes() {
    check_grid("scaled");
}
#[test]
fn round_decimals_on_extremes() {
    check_grid("decimals");
}

// ---- sweeps ----

#[test]
fn sweep_print_scaled() {
    check_sweep("scaled", |t, h| {
        let mut g = Lcg(1);
        let mut n = 0;
        for v in 0..70000 {
            h.line(&scaled(t, v).0);
            n += 1;
        }
        for _ in 0..130000 {
            h.line(&scaled(t, g.dimen() as i32).0);
            n += 1;
        }
        n
    });
}

#[test]
fn sweep_x_over_n() {
    check_sweep("x_over_n", |t, h| {
        let mut g = Lcg(2);
        for _ in 0..100000 {
            let (a, b) = (g.mixed() as i32, g.mixed() as i32);
            h.line(&x_over_n(t, a, b).line());
        }
        100000
    });
}

#[test]
fn sweep_mult_integers() {
    check_sweep("mult_int", |t, h| {
        let mut g = Lcg(3);
        for _ in 0..100000 {
            let (a, b) = (g.mixed() as i32, g.mixed() as i32);
            h.line(&mult_int(t, a, b).line());
        }
        100000
    });
}

#[test]
fn sweep_nx_plus_y() {
    check_sweep("nx_plus_y", |t, h| {
        let mut g = Lcg(4);
        for _ in 0..100000 {
            let (a, b) = (g.dimen() as i32, g.mixed() as i32);
            h.line(&nx_plus_y(t, a, b).line());
        }
        100000
    });
}

#[test]
fn sweep_units() {
    const UNITS: [&str; 11] = ["pt", "in", "pc", "cm", "mm", "bp", "dd", "cc", "nd", "nc", "sp"];
    check_sweep("units", |t, h| {
        let mut g = Lcg(5);
        for _ in 0..100000 {
            let i = g.nonneg() as i32;
            let d = g.digits(20);
            let u = UNITS[(g.next() % 11) as usize];
            h.line(&units(t, i, &d, u, None).line());
        }
        100000
    });
}

#[test]
fn sweep_internal() {
    check_sweep("internal", |t, h| {
        let mut g = Lcg(6);
        for _ in 0..50000 {
            let sign = g.next() & 1 != 0;
            let a = g.nonneg() as i32;
            let d = g.digits(18);
            let mut v = g.next();
            v = (v >> (g.next() % 31)).min((1 << 31) - 2);
            if g.next() & 1 != 0 {
                v = -v;
            }
            h.line(&internal(t, sign, a, &d, v as i32).line());
        }
        50000
    });
}

#[test]
fn sweep_mkern() {
    check_sweep("mkern", |t, h| {
        let mut g = Lcg(10);
        for _ in 0..30000 {
            let (q, x) = (g.dimen() as i32, g.dimen() as i32);
            h.line(&mkern(t, q, x).0);
        }
        30000
    });
}

#[test]
fn sweep_badness() {
    check_sweep("badness", |t, h| {
        let mut g = Lcg(7);
        for _ in 0..100000 {
            let mut a = (g.nonneg() & MAXD as i64) as i32;
            if a == 0 {
                a = 1;
            }
            let s = (g.nonneg() & MAXD as i64) as i32;
            h.line(&t.badness(a, s).to_string());
        }
        100000
    });
    check_sweep("bad_tight", |t, h| {
        let mut g = Lcg(8);
        for _ in 0..50000 {
            let mut s = g.nonneg() & MAXD as i64;
            if s == 0 {
                s = 1;
            }
            let a = 1 + g.next() % s;
            h.line(&t.badness(a as i32, s as i32).to_string());
        }
        50000
    });
}

#[test]
fn sweep_half() {
    check_sweep("half", |t, h| {
        let mut g = Lcg(11);
        for _ in 0..30000 {
            let d = g.next();
            let d = (d >> (g.next() % 31)).min((1 << 31) - 2);
            h.line(&half(t, d as i32, 0).0);
        }
        for _ in 0..30000 {
            let d = -(g.nonneg() & MAXD as i64);
            h.line(&half(t, d as i32, MAXD).0);
        }
        60000
    });
}

#[test]
fn sweep_round_decimals() {
    check_sweep("decimals", |t, h| {
        let mut g = Lcg(9);
        for _ in 0..100000 {
            let d = g.digits(20);
            h.line(&units(t, 0, &d, "pt", None).0);
        }
        100000
    });
}

// ---- the random numbers ----

#[test]
fn random_numbers_against_the_oracle() {
    let t = machine();
    let ops: Vec<&str> =
        TABLE.lines().find(|l| l.starts_with("random_ops\t")).unwrap().split('\t').nth(1).unwrap().split(',').collect();
    let mut seen = 0;
    for row in TABLE.lines().filter(|l| l.starts_with("random\t")) {
        let f: Vec<&str> = row.split('\t').collect();
        let seed = int(f[1]);
        let want: Vec<&str> = f[2].split(',').collect();
        assert_eq!(want.len(), ops.len());
        t.init_randoms(seed);
        for (k, op) in ops.iter().enumerate() {
            let got = if *op == "n" { t.norm_rand() } else { t.unif_rand(int(op)) };
            assert_eq!(got.to_string(), want[k], "seed {seed}, step {k}, op {op}");
        }
        seen += 1;
    }
    assert!(seen >= 18);
    Tex::dealloc(t);
}

#[test]
fn long_runs_of_random_numbers_against_the_oracle() {
    let t = machine();
    let mut seen = 0;
    for row in TABLE.lines().filter(|l| l.starts_with("sweep_random\t")) {
        let f: Vec<&str> = row.split('\t').collect();
        let seed = int(f[1]);
        let n: usize = f[2].parse().unwrap();
        let mut g = Lcg(seed as u64);
        let mut h = Fnv::new();
        t.init_randoms(seed);
        for _ in 0..n / 2 {
            let x = g.mixed() as i32;
            h.line(&t.unif_rand(x).to_string());
            h.line(&t.norm_rand().to_string());
        }
        assert_eq!(h.hex(), f[3], "seed {seed}");
        seen += 1;
    }
    assert_eq!(seen, 3);
    Tex::dealloc(t);
}

// ---- what no primitive reaches directly: properties from the WEB's own statements ----

/// §111: `make_frac(p,q)` is `2^28 p/q` rounded, and `arith_error` beyond 8.
#[test]
fn make_frac_is_the_rounded_quotient() {
    let t = machine();
    let mut g = Lcg(101);
    for _ in 0..200000 {
        let (p, q) = (g.mixed() as i32, g.mixed() as i32);
        if q == 0 {
            continue;
        }
        t.arith_error = false;
        let got = t.make_frac(p, q) as i128;
        let exact = (p as i128 * (1 << 28)) as f64 / q as f64;
        if abs(p) / abs(q) >= 8 {
            assert!(t.arith_error, "{p} {q}");
            assert_eq!(got.abs(), MAXI as i128, "{p} {q}");
        } else {
            assert!(!t.arith_error, "{p} {q}");
            // |exact| below 2^31, where f64 has 2^-21 to spare: compare with the rounding of the exact rational.
            let (num, den) = ((p as i128).abs() * (1 << 28), (q as i128).abs());
            let rounded = (2 * num + den) / (2 * den);
            let want = if (p < 0) != (q < 0) { -rounded } else { rounded };
            // WEB: "correct to within one": ties and the last unit go by its loop.
            assert!((got - want).abs() <= 1, "{p} {q}: {got} against {want}, about {exact}");
        }
    }
    Tex::dealloc(t);
}

/// §114: `take_frac(q,f)` is `q f/2^28` rounded.
#[test]
fn take_frac_is_the_rounded_product() {
    let t = machine();
    let mut g = Lcg(102);
    for _ in 0..200000 {
        let (q, f) = (g.mixed() as i32, g.mixed() as i32);
        t.arith_error = false;
        let got = t.take_frac(q, f) as i128;
        let num = (q as i128).abs() * (f as i128).abs();
        let rounded = (2 * num + (1 << 28)) / (1 << 29);
        let want = if (q < 0) != (f < 0) { -rounded } else { rounded };
        if rounded > MAXI as i128 {
            assert!(t.arith_error, "{q} {f}");
            assert_eq!(got.abs(), MAXI as i128, "{q} {f}");
        } else {
            assert!((got - want).abs() <= 1, "{q} {f}: {got} against {want}");
        }
    }
    Tex::dealloc(t);
}

/// §122: the sign of `ab-cd`, exactly.
#[test]
fn ab_vs_cd_is_the_sign_of_the_difference() {
    let t = machine();
    let mut g = Lcg(103);
    let edge = [0, 1, -1, 2, -2, 7, MAXI, -MAXI, MAXI - 1, 1 << 30, -(1 << 30)];
    for &a in &edge {
        for &b in &edge {
            for &c in &edge {
                for &d in &edge {
                    let want = (a as i128 * b as i128 - c as i128 * d as i128).signum() as i32;
                    assert_eq!(t.ab_vs_cd(a, b, c, d), want, "{a} {b} {c} {d}");
                }
            }
        }
    }
    for _ in 0..500000 {
        let (a, b, c, d) = (g.mixed() as i32, g.mixed() as i32, g.mixed() as i32, g.mixed() as i32);
        let want = (a as i128 * b as i128 - c as i128 * d as i128).signum() as i32;
        assert_eq!(t.ab_vs_cd(a, b, c, d), want, "{a} {b} {c} {d}");
    }
    Tex::dealloc(t);
}

/// §119: `m_log(x)` is `2^8 ln(x/2^16)` scaled by 2^16, with an error of a few units.
#[test]
fn m_log_is_the_logarithm() {
    let t = machine();
    let mut g = Lcg(104);
    for _ in 0..100000 {
        let x = g.nonneg() as i32;
        if x <= 0 {
            continue;
        }
        let want = 256.0 * (x as f64 / 65536.0).ln() * 65536.0;
        let got = t.m_log(x) as f64;
        assert!((got - want).abs() < 8.0, "m_log({x}) = {got}, about {want}");
    }
    Tex::dealloc(t);
}

/// §100, including where `x+1` would wrap in 32 bits: the oracle's `half_through_vcenter` row for 2^31-1 is this.
#[test]
fn half_rounds_as_written() {
    let t = machine();
    for (x, want) in [
        (0, 0), (1, 1), (-1, 0), (2, 1), (-2, -1), (3, 2), (-3, -1), (4, 2), (5, 3), (-5, -2),
        (MAXI, 1 << 30), (MAXI - 1, (1 << 30) - 1), (-MAXI, -((1 << 30) - 1)), (-(MAXI - 1), -((1 << 30) - 1)),
    ] {
        assert_eq!(t.half(x), want, "half({x})");
    }
    Tex::dealloc(t);
}

/// §107: where no `arith_error` arises, the exact quotient and remainder, with the sign of `x`.
#[test]
fn xn_over_d_is_exact() {
    let t = machine();
    let mut g = Lcg(105);
    for _ in 0..300000 {
        let x = g.mixed() as i32;
        let n = (g.nonneg() as i32) & 0o177777;
        let d = 1 + ((g.nonneg() as i32) & 0o177777);
        t.arith_error = false;
        let got = t.xn_over_d(x, n, d);
        let (num, den) = (abs(x) as i64 * n as i64, d as i64);
        if num / den >= 1 << 30 {
            assert!(t.arith_error, "{x} {n} {d}");
        } else {
            assert!(!t.arith_error, "{x} {n} {d}");
            let s = if x >= 0 { 1 } else { -1 };
            assert_eq!(got as i64, s * (num / den), "{x} {n} {d}");
            assert_eq!(t.remainder as i64, s * (num % den), "{x} {n} {d}: remainder");
        }
    }
    // A value clamped near 2^31 must still set arith_error, in both signs.  §107 returns `u` as it stands
    // then, which is `x*n` without the last division, and the caller discards it.
    for x in [MAXI, -MAXI] {
        t.arith_error = false;
        let r = t.xn_over_d(x, 1000, 1);
        assert!(t.arith_error);
        assert_eq!(r, if x > 0 { 65535999 } else { -65535999 });
    }
    // A `u` beyond 32 bits, which the oracle keeps and the port clamps.
    t.arith_error = false;
    assert_eq!(t.xn_over_d(MAXI, MAXI, 1), MAXI);
    assert!(t.arith_error);
    assert_eq!(t.xn_over_d(-MAXI, MAXI, 1), -MAXI);
    Tex::dealloc(t);
}

/// §106: `x_over_n`: the remainder has the sign of `x`, then is negated for negative `n`.  §105 with `n=0`.
#[test]
fn x_over_n_remainder_and_zero() {
    let t = machine();
    for (x, n, q, r) in [
        (7, 2, 3, 1), (-7, 2, -3, -1), (7, -2, -3, 1), (-7, -2, 3, -1), (0, 5, 0, 0), (5, 1, 5, 0), (MAXI, MAXI, 1, 0),
        (-MAXI, MAXI, -1, 0),
    ] {
        t.arith_error = false;
        assert_eq!(t.x_over_n(x, n), q, "{x} {n}");
        assert_eq!(t.remainder, r, "{x} {n}: remainder");
        assert!(!t.arith_error);
    }
    t.arith_error = false;
    assert_eq!(t.x_over_n(9, 0), 0);
    assert_eq!(t.remainder, 9);
    assert!(t.arith_error);
    t.arith_error = false;
    assert_eq!(t.mult_and_add(0, 5, 77, MAXI), 77);
    assert!(!t.arith_error);
    Tex::dealloc(t);
}
