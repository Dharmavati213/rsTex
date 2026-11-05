//! §99-127 Parts 7, 7b: arithmetic with scaled dimensions, random numbers.
//!
//! Arch's `integer` is 64 bits, the official pdfTeX's and ours are 32: docs/PORTING.md section 11.  `half` and `xn_over_d`
//! hold Arch's value in a local `i64`, and the rest is literal in `i32`, with negate and the like wrapping as on the official one.
#![forbid(unsafe_code)]
#![deny(unreachable_patterns, unused_variables)]
#![allow(non_snake_case, unused_mut, unused_imports, dead_code)]

use crate::access::*;
use crate::consts::*;
use crate::host::*;
use crate::raw::*;
use crate::state::*;

crate::pod_struct! {
    /// POD state private to this module.  Add fields here, not to state.rs.
    pub struct Vars {}
}

/// Non-POD state private to this module, in `self.host`.  Rarely needed.
#[derive(Default)]
pub struct HVars {}

impl Tex {
    /// §100
    pub fn half(&mut self, x: i32) -> i32 {
        if odd(x) {
            // `x+1` at 2^31-1 is Arch's in an `i64`, and it wraps on the official pdfTeX, which then says -2^30.
            ((x as i64 + 1) / 2) as i32
        } else {
            x / 2
        }
    }

    /// §102
    pub fn round_decimals(&mut self, k: i32) -> i32 {
        let mut k = k;
        let mut a = 0;
        while k > 0 {
            k -= 1;
            a = (a + self.dig[k] * TWO) / 10;
        }
        (a + 1) / 2
    }

    /// §103
    pub fn print_scaled(&mut self, s: impl Into<i64>) {
        let mut s: i64 = s.into();
        if s < 0 {
            self.print_char(b'-' as i32);
            if s == i32::MIN as i64 {
                // The official pdfTeX's negate of -2^31 is itself, so it prints a second sign, and Arch's 2^31 has none.
                self.print_char(b'-' as i32);
            }
            s = -s;
        }
        let unity = UNITY as i64;
        self.print_int(s / unity);
        self.print_char(b'.' as i32);
        s = 10 * (s % unity) + 5;
        let mut delta = 10;
        loop {
            if delta > unity {
                s = s + 0o100000 - 50000;
            }
            self.print_char(b'0' as i32 + (s / unity) as i32);
            s = 10 * (s % unity);
            delta = delta * 10;
            if s <= delta {
                break;
            }
        }
    }

    /// §105
    pub fn mult_and_add(&mut self, n: i32, x: i32, y: i32, max_answer: i32) -> i32 {
        let (mut n, mut x) = (n, x);
        if n < 0 {
            x = x.wrapping_neg();
            n = n.wrapping_neg();
        }
        if n == 0 {
            y
        } else if x <= max_answer.wrapping_sub(y) / n && x.wrapping_neg() <= max_answer.wrapping_add(y) / n {
            // The test wraps as the official pdfTeX's does: `x` or `n` of -2^31 stays as it is, and `y` from xn_over_d's
            // arith_error path can be anything.  `n` is 1 or more, or -2^31, so neither division can trap.
            n.wrapping_mul(x).wrapping_add(y)
        } else {
            self.arith_error = true;
            0
        }
    }

    /// §106
    pub fn x_over_n(&mut self, x: i32, n: i32) -> i32 {
        let (mut x, mut n) = (x, n);
        let mut negative = false;
        let result;
        if n == 0 {
            self.arith_error = true;
            result = 0;
            self.remainder = x;
        } else {
            if n < 0 {
                x = x.wrapping_neg();
                n = n.wrapping_neg();
                negative = true;
            }
            // WEB's `if x>=0` with -((-x) div n) for the rest is this one division, which rounds the same way.  The two differ
            // only at -2^31, where gcc makes the same division of WEB's on the official pdfTeX: -2^31 div 2 is -2^30, and -2^31 div
            // -2^31 is 1.  `n` is 1 or more, or -2^31, so it cannot trap.
            result = x / n;
            self.remainder = x % n;
        }
        if negative {
            self.remainder = self.remainder.wrapping_neg();
        }
        result
    }

    /// §106 on what the page builder holds in an `i64`, as Arch's `x_over_n` takes it.  `n` is not 0 at either of its two callers.
    pub fn x_over_n_wide(&mut self, x: i64, n: i32) -> i64 {
        let (x, n, negative) = if n < 0 { (-x, -(n as i64), true) } else { (x, n as i64, false) };
        let r = x % n;
        self.remainder = wrap32(if negative { -r } else { r });
        x / n
    }

    /// §107
    /// `t`, `u`, `v` are `i64`, as on Arch's: docs/PORTING.md section 11.  With `arith_error` set, `u` is returned without
    /// its last division.  It can then be 2^31 or more, which Arch's holds and we wrap: scan_dimen reports
    /// the error whatever the value, but `mu_mult` uses it, for a `\muskip` that `\advance` took past `\maxdimen`.
    pub fn xn_over_d(&mut self, x: i32, n: i32, d: i32) -> i32 {
        let positive;
        let mut x = x;
        if x >= 0 {
            positive = true;
        } else {
            x = x.wrapping_neg(); // -2^31 stays as it is, and so does what follows from it on the official pdfTeX
            positive = false;
        }
        let (n, d) = (n as i64, d as i64);
        let t = (x as i64 % 0o100000) * n;
        let mut u = (x as i64 / 0o100000) * n + (t / 0o100000);
        let v = (u % d) * 0o100000 + (t % 0o100000);
        if u / d >= 0o100000 {
            self.arith_error = true;
        } else {
            u = 0o100000 * (u / d) + (v / d);
        }
        if positive {
            self.remainder = (v % d) as i32;
            wrap32(u)
        } else {
            self.remainder = -(v % d) as i32;
            wrap32(-u)
        }
    }

    /// §108
    pub fn badness(&mut self, t: i32, s: i32) -> i32 {
        if t == 0 {
            0
        } else if s <= 0 {
            INF_BAD
        } else {
            let r = if t <= 7230584 {
                (t * 297) / s
            } else if s >= 1663497 {
                t / (s / 297)
            } else {
                t
            };
            if r > 1290 {
                INF_BAD
            } else {
                (r * r * r + 0o400000) / 0o1000000
            }
        }
    }

    /// §112
    pub fn make_frac(&mut self, p: i32, q: i32) -> i32 {
        let (mut p, mut q) = (p, q);
        let mut negative;
        if p >= 0 {
            negative = false;
        } else {
            p = -p;
            negative = true;
        }
        if q <= 0 {
            q = -q;
            negative = !negative;
        }
        let mut n = p / q;
        p = p % q;
        if n >= 8 {
            self.arith_error = true;
            if negative { -EL_GORDO } else { EL_GORDO }
        } else {
            n = (n - 1) * FRACTION_ONE;
            // §113 Compute f=floor(2^28(1+p/q)+1/2)
            let mut f = 1;
            let mut be_careful;
            loop {
                be_careful = p - q;
                p = be_careful + p;
                if p >= 0 {
                    f = f + f + 1;
                } else {
                    f = f + f;
                    p = p + q;
                }
                if f >= FRACTION_ONE {
                    break;
                }
            }
            be_careful = p - q;
            if be_careful + p >= 0 {
                f += 1;
            }
            if negative { -(f + n) } else { f + n }
        }
    }

    /// §114
    pub fn take_frac(&mut self, q: i32, f: i32) -> i32 {
        let (mut q, mut f) = (q, f);
        // §115 Reduce to the case that f>=0 and q>0
        let mut negative;
        if f >= 0 {
            negative = false;
        } else {
            f = f.wrapping_neg();
            negative = true;
        }
        if q < 0 {
            q = q.wrapping_neg(); // \pdfuniformdeviate of -2^31 gets here, and it stays as it is on the official pdfTeX
            negative = !negative;
        }
        let mut n;
        if f < FRACTION_ONE {
            n = 0;
        } else {
            n = f / FRACTION_ONE;
            f = f % FRACTION_ONE;
            if q <= EL_GORDO / n {
                n = n.wrapping_mul(q);
            } else {
                self.arith_error = true;
                n = EL_GORDO;
            }
        }
        f = f + FRACTION_ONE;
        // §116 Compute p=floor(qf/2^28+1/2)-q
        let mut p = FRACTION_HALF;
        if q < FRACTION_FOUR {
            loop {
                if odd(f) {
                    p = self.halfp(p.wrapping_add(q));
                } else {
                    p = self.halfp(p);
                }
                f = self.halfp(f);
                if f == 1 {
                    break;
                }
            }
        } else {
            loop {
                if odd(f) {
                    p = p.wrapping_add(self.halfp(q.wrapping_sub(p)));
                } else {
                    p = self.halfp(p);
                }
                f = self.halfp(f);
                if f == 1 {
                    break;
                }
            }
        }
        let be_careful = n.wrapping_sub(EL_GORDO);
        if be_careful.wrapping_add(p) > 0 {
            self.arith_error = true;
            n = EL_GORDO.wrapping_sub(p);
        }
        if negative { n.wrapping_add(p).wrapping_neg() } else { n.wrapping_add(p) }
    }

    /// §119
    pub fn m_log(&mut self, x: i32) -> i32 {
        let mut x = x;
        if x <= 0 {
            // §121 Handle non-positive logarithm
            self.print_err_s(b"Logarithm of ");
            self.print_scaled(x);
            self.print_s(b" has been replaced by 0");
            self.help(&[
                b"Since I don't take logs of non-positive numbers,",
                b"I'm zeroing this one. Proceed, with fingers crossed.",
            ]);
            self.error();
            0
        } else {
            let mut y = 1302456956 + 4 - 100;
            let mut z = 27595 + 6553600;
            while x < FRACTION_FOUR {
                x = x + x;
                y = y - 93032639;
                z = z - 48782;
            }
            y = y + (z / UNITY);
            let mut k = 2;
            while x > FRACTION_FOUR + 4 {
                // §120 Increase k until x can be multiplied by a factor of 2^{-k}, and adjust y
                z = ((x - 1) / self.two_to_the[k]) + 1;
                while x < FRACTION_FOUR + z {
                    z = self.halfp(z + 1);
                    k = k + 1;
                }
                y = y + self.spec_log[k];
                x = x - z;
            }
            y / 8
        }
    }

    /// §122
    pub fn ab_vs_cd(&mut self, a: i32, b: i32, c: i32, d: i32) -> i32 {
        let (mut a, mut b, mut c, mut d) = (a, b, c, d);
        let mut q;
        let mut r;
        // §123 Reduce to the case that a,c>=0, b,d>0
        if a < 0 {
            a = -a;
            b = -b;
        }
        if c < 0 {
            c = -c;
            d = -d;
        }
        if d <= 0 {
            if b >= 0 {
                if (a == 0 || b == 0) && (c == 0 || d == 0) {
                    return 0;
                } else {
                    return 1;
                }
            }
            if d == 0 {
                if a == 0 {
                    return 0;
                } else {
                    return -1;
                }
            }
            q = a;
            a = c;
            c = q;
            q = -b;
            b = -d;
            d = q;
        } else if b <= 0 {
            if b < 0 && a > 0 {
                return -1;
            }
            if c == 0 {
                return 0;
            } else {
                return -1;
            }
        }
        loop {
            q = a / d;
            r = c / b;
            if q != r {
                if q > r {
                    return 1;
                } else {
                    return -1;
                }
            }
            q = a % d;
            r = c % b;
            if r == 0 {
                if q == 0 {
                    return 0;
                } else {
                    return 1;
                }
            }
            if q == 0 {
                return -1;
            }
            a = b;
            b = q;
            c = d;
            d = r;
        }
    }

    /// §124
    pub fn new_randoms(&mut self) {
        for k in 0..=23 {
            let mut x = self.randoms[k].wrapping_sub(self.randoms[k + 31]);
            if x < 0 {
                x = x.wrapping_add(FRACTION_ONE);
            }
            self.randoms[k] = x;
        }
        for k in 24..=54 {
            let mut x = self.randoms[k].wrapping_sub(self.randoms[k - 24]);
            if x < 0 {
                x = x.wrapping_add(FRACTION_ONE);
            }
            self.randoms[k] = x;
        }
        self.j_random = 54;
    }

    /// §125
    pub fn init_randoms(&mut self, seed: i32) {
        let mut j = abs(seed);
        while j >= FRACTION_ONE {
            j = self.halfp(j);
        }
        let mut k = 1;
        for i in 0..=54 {
            let jj = k;
            k = j.wrapping_sub(k);
            j = jj;
            if k < 0 {
                k = k.wrapping_add(FRACTION_ONE);
            }
            self.randoms[(i * 21) % 55] = j;
        }
        self.new_randoms();
        self.new_randoms();
        self.new_randoms();
    }

    /// §126
    pub fn unif_rand(&mut self, x: i32) -> i32 {
        self.next_random();
        let y = self.take_frac(abs(x), self.randoms[self.j_random]);
        if y == abs(x) {
            0
        } else if x > 0 {
            y
        } else {
            y.wrapping_neg()
        }
    }

    /// §127
    pub fn norm_rand(&mut self) -> i32 {
        let mut x;
        loop {
            let mut u;
            loop {
                self.next_random();
                x = self.take_frac(112429, self.randoms[self.j_random] - FRACTION_HALF);
                self.next_random();
                u = self.randoms[self.j_random];
                if abs(x) < u {
                    break;
                }
            }
            x = self.make_frac(x, u);
            let l = 139548960 - self.m_log(u);
            if self.ab_vs_cd(1024, l, x, x) >= 0 {
                break;
            }
        }
        x
    }
}
