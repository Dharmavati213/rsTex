//! The bits of libc whose exact behaviour the C sources lean on: NUL-terminated strings over byte
//! buffers, glibc's `sscanf` conversions as far as the fonts use them, `%g`, `atan`.
//!
//! All of this is pure Rust so that every platform gives the same bytes; the system `atan` and
//! `sscanf` are exactly what must not be used.  The oracle's `sscanf` is `__isoc23_sscanf`
//! (`nm -D /usr/bin/pdftex`), so `%i` knows `0b`.

/// `buf[i]`, with everything past the end reading as NUL.
#[inline]
pub(crate) fn at(buf: &[u8], i: usize) -> u8 {
    match buf.get(i) {
        Some(&c) => c,
        None => 0,
    }
}

/// `strlen(buf + start)`.
#[inline]
pub(crate) fn strlen(buf: &[u8], start: usize) -> usize {
    let mut i = start;
    while i < buf.len() && buf[i] != 0 {
        i += 1;
    }
    i - start
}

/// The C string starting at `start`.
#[inline]
pub(crate) fn cstr(buf: &[u8], start: usize) -> &[u8] {
    if start >= buf.len() {
        return &[];
    }
    &buf[start..start + strlen(buf, start)]
}

/// `str_prefix(buf + start, s)`: `strncmp(s1, s2, strlen(s2)) == 0`.  `s` holds no NUL, so a NUL in
/// `buf` is a mismatch, as it is in `strncmp`.
#[inline]
pub(crate) fn str_prefix(buf: &[u8], start: usize, s: &[u8]) -> bool {
    start <= buf.len() && buf[start..].starts_with(s)
}

/// `strchr(buf + start, c)`, `c` not NUL.
pub(crate) fn strchr(buf: &[u8], start: usize, c: u8) -> Option<usize> {
    let mut i = start;
    while i < buf.len() && buf[i] != 0 {
        if buf[i] == c {
            return Some(i);
        }
        i += 1;
    }
    None
}

/// `strstr(buf + start, pat)`, as an index into `buf`.
pub(crate) fn strstr(buf: &[u8], start: usize, pat: &[u8]) -> Option<usize> {
    let s = cstr(buf, start);
    if pat.is_empty() {
        return Some(start);
    }
    if s.len() < pat.len() {
        return None;
    }
    let first = pat[0];
    let last = s.len() - pat.len();
    let mut i = 0;
    while i <= last {
        if s[i] == first && &s[i..i + pat.len()] == pat {
            return Some(start + i);
        }
        i += 1;
    }
    None
}

/// `str_suffix` of writet1.c: does `buf[begin..end]`, without one trailing newline, end in `s`?
pub(crate) fn str_suffix(buf: &[u8], begin: usize, end: usize, s: &[u8]) -> bool {
    if end == 0 {
        return false;
    }
    let mut s1 = end as isize - 1;
    if at(buf, s1 as usize) == 10 {
        s1 -= 1;
    }
    let mut s2 = s.len() as isize - 1;
    while s1 >= begin as isize && s2 >= 0 {
        if at(buf, s1 as usize) != s[s2 as usize] {
            return false;
        }
        s1 -= 1;
        s2 -= 1;
    }
    s2 < 0
}

/// `strcasecmp(s, t) == 0` in the C locale.
pub(crate) fn ascii_ieq(s: &[u8], t: &[u8]) -> bool {
    s.len() == t.len() && s.iter().zip(t).all(|(a, b)| a.eq_ignore_ascii_case(b))
}

/// C's `isspace` in the C locale.
#[inline]
pub(crate) fn is_space(c: u8) -> bool {
    c == b' ' || (9..=13).contains(&c)
}

/// `(int) f`: x86's `cvttss2si`, which gives `INT_MIN` for NaN and out of range.  The C has
/// undefined behaviour there; this is what the oracle does on every platform we build for.
#[inline]
pub(crate) fn ftoi(f: f32) -> i32 {
    if f.is_nan() || f >= 2147483648.0 || f < -2147483648.0 {
        i32::MIN
    } else {
        f as i32
    }
}

/// `(long) f`, for the oracle's 64-bit `integer`.
#[inline]
pub(crate) fn dtol(d: f64) -> i64 {
    if d.is_nan() || d >= 9223372036854775808.0 || d < -9223372036854775808.0 {
        i64::MIN
    } else {
        d as i64
    }
}

/// `zround` of web2c's lib/zround.c, which is what `round` means in pdfTeX's C files.
pub(crate) fn zround(r: f64) -> i64 {
    if r > 2147483647.0 {
        2147483647
    } else if r < -2147483647.0 {
        -2147483647
    } else if r >= 0.0 {
        dtol(r + 0.5)
    } else {
        dtol(r - 0.5)
    }
}

/// Appends an unsigned decimal, as `%u` or `%lu` would.
pub(crate) fn push_u64(out: &mut Vec<u8>, mut n: u64) {
    let mut d = [0u8; 20];
    let mut i = d.len();
    loop {
        i -= 1;
        d[i] = b'0' + (n % 10) as u8;
        n /= 10;
        if n == 0 {
            break;
        }
    }
    out.extend_from_slice(&d[i..]);
}

/// Appends a signed decimal, as `%i` would.
pub(crate) fn push_i64(out: &mut Vec<u8>, n: i64) {
    if n < 0 {
        out.push(b'-');
    }
    push_u64(out, n.unsigned_abs());
}

/// `%.{width}X` of an unsigned value: upper-case hex, at least `width` digits.
pub(crate) fn push_hex(out: &mut Vec<u8>, n: u64, width: usize) {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut d = [0u8; 16];
    let mut i = d.len();
    let mut v = n;
    while v != 0 {
        i -= 1;
        d[i] = HEX[(v & 15) as usize];
        v >>= 4;
    }
    let digits = d.len() - i;
    for _ in digits..width {
        out.push(b'0');
    }
    out.extend_from_slice(&d[i..]);
}

/// A cursor for `sscanf`.  The input ends at the first NUL or the end of the slice, which is EOF.
pub(crate) struct Scan<'a> {
    s: &'a [u8],
    pub i: usize,
}

impl<'a> Scan<'a> {
    pub fn new(s: &'a [u8], start: usize) -> Scan<'a> {
        Scan { s, i: start }
    }

    #[inline]
    fn peek(&self) -> u8 {
        at(self.s, self.i)
    }

    /// A blank in a format, and what every conversion but `%n` does first.
    pub fn skip_ws(&mut self) {
        while is_space(self.peek()) {
            self.i += 1;
        }
    }

    /// A run of ordinary characters in a format.  A blank in `pat` skips any white space.
    pub fn lit(&mut self, pat: &[u8]) -> bool {
        for &p in pat {
            if p == b' ' {
                self.skip_ws();
            } else if self.peek() == p {
                self.i += 1;
            } else {
                return false;
            }
        }
        true
    }

    /// `%i` (`base` 0), `%X` (`base` 16), with a maximum field width.  The result is what `strtol`
    /// gives, saturated at the `long` range; callers truncate as the C's argument type does.
    pub fn int(&mut self, mut base: u32, mut width: usize) -> Option<i64> {
        self.skip_ws();
        let mut neg = false;
        let mut got_any = false;
        if width > 0 && (self.peek() == b'-' || self.peek() == b'+') {
            neg = self.peek() == b'-';
            self.i += 1;
            width -= 1;
        }
        if width > 0 && self.peek() == b'0' {
            got_any = true;
            self.i += 1;
            width -= 1;
            let c = self.peek().to_ascii_lowercase();
            if width > 0 && c == b'x' && (base == 0 || base == 16) {
                base = 16;
                self.i += 1;
                width -= 1;
            } else if width > 0 && c == b'b' && base == 0 {
                // C23's 0b, in __isoc23_sscanf
                base = 2;
                self.i += 1;
                width -= 1;
            } else if base == 0 {
                base = 8;
            }
        }
        if base == 0 {
            base = 10;
        }
        let mut v: i64 = 0;
        let mut over = false;
        while width > 0 {
            let d = match self.peek() {
                c @ b'0'..=b'9' => (c - b'0') as u32,
                c @ b'a'..=b'f' => (c - b'a') as u32 + 10,
                c @ b'A'..=b'F' => (c - b'A') as u32 + 10,
                _ => 99,
            };
            if d >= base {
                break;
            }
            got_any = true;
            v = match v.checked_mul(base as i64).and_then(|x| x.checked_add(d as i64)) {
                Some(x) => x,
                None => {
                    over = true;
                    i64::MAX
                }
            };
            self.i += 1;
            width -= 1;
        }
        if !got_any {
            return None;
        }
        if over {
            return Some(if neg { i64::MIN } else { i64::MAX });
        }
        Some(if neg { -v } else { v })
    }

    /// `%255s`: the span of the word, after skipping white space.
    pub fn word(&mut self, width: usize) -> Option<(usize, usize)> {
        self.skip_ws();
        let start = self.i;
        while self.i - start < width && self.peek() != 0 && !is_space(self.peek()) {
            self.i += 1;
        }
        if self.i == start {
            None
        } else {
            Some((start, self.i))
        }
    }

    /// `%g` and `%f` on a `float`, following glibc's vfscanf state machine: it collects a candidate
    /// greedily, then `strtof` takes what it can of it.
    pub fn float(&mut self) -> Option<f32> {
        self.skip_ws();
        let start = self.i;
        let mut got_sign = 0;
        let c0 = self.peek();
        if c0 == 0 {
            return None;
        }
        if c0 == b'-' || c0 == b'+' {
            got_sign = 1;
            self.i += 1;
            if self.peek() == 0 {
                return None;
            }
        }
        let neg = c0 == b'-';
        let c = self.peek().to_ascii_lowercase();
        if c == b'n' {
            for &p in b"nan" {
                if self.peek().to_ascii_lowercase() != p {
                    return None;
                }
                self.i += 1;
            }
            return Some(if neg { -f32::NAN } else { f32::NAN });
        }
        if c == b'i' {
            for &p in b"inf" {
                if self.peek().to_ascii_lowercase() != p {
                    return None;
                }
                self.i += 1;
            }
            if self.peek().to_ascii_lowercase() == b'i' {
                // "infinity", all or nothing once the fourth letter is seen
                for &p in b"inity" {
                    if self.peek().to_ascii_lowercase() != p {
                        return None;
                    }
                    self.i += 1;
                }
            }
            return Some(if neg { f32::NEG_INFINITY } else { f32::INFINITY });
        }
        let mut hexa = false;
        let mut exp_char = b'e';
        let mut got_digit = false;
        let mut got_dot = false;
        let mut got_e = false;
        if c == b'0' {
            self.i += 1;
            if self.peek().to_ascii_lowercase() == b'x' {
                hexa = true;
                exp_char = b'p';
                self.i += 1;
            } else {
                got_digit = true;
            }
        }
        loop {
            let c = self.peek();
            if c == 0 {
                break;
            }
            let last = if self.i > 0 { self.s[self.i - 1].to_ascii_lowercase() } else { 0 };
            if c.is_ascii_digit() {
                got_digit = true;
            } else if !got_e && hexa && c.is_ascii_hexdigit() {
                got_digit = true;
            } else if got_e && self.i > start && last == exp_char && (c == b'-' || c == b'+') {
            } else if got_digit && !got_e && c.to_ascii_lowercase() == exp_char {
                got_e = true;
                got_dot = true;
            } else if !got_dot && c == b'.' {
                got_dot = true;
            } else {
                break;
            }
            self.i += 1;
        }
        let collected = &self.s[start..self.i];
        if collected.len() == got_sign || (hexa && collected.len() == 2 + got_sign) {
            return None;
        }
        // An exponent with no digits is a matching failure in this glibc (BZ #12701), whatever the
        // comment in fm_scan_line says: the oracle takes `.8ExtendFont` for an unknown word.
        if got_e {
            let n = collected.len();
            let last = collected[n - 1].to_ascii_lowercase();
            if last == exp_char
                || ((last == b'+' || last == b'-') && n >= 2 && collected[n - 2].to_ascii_lowercase() == exp_char)
            {
                return None;
            }
        }
        strtof(collected)
    }
}

/// `strtof` over a candidate collected by `Scan::float`: the longest valid prefix, or None.
fn strtof(s: &[u8]) -> Option<f32> {
    let n = s.len();
    let (neg, body) = match s[0] {
        b'-' => (true, 1),
        b'+' => (false, 1),
        _ => (false, 0),
    };
    if n >= body + 2 && (s[body + 1] == b'x' || s[body + 1] == b'X') {
        return strtof_hex(&s[body + 2..]).map(|v| if neg { -v } else { v });
    }
    if !s[body..n].iter().any(|c| c.is_ascii_digit()) {
        return None;
    }
    let text = core::str::from_utf8(&s[..n]).ok()?;
    text.parse::<f32>().ok()
}

/// The hexadecimal part of `strtof`, correctly rounded.  Fonts do not use this; the scanner reaches
/// it for input such as `0x1p-3 SlantFont`.
fn strtof_hex(s: &[u8]) -> Option<f32> {
    let mut m: u64 = 0;
    let mut sticky = false;
    let mut e: i64 = 0;
    let mut i = 0;
    let mut dot = false;
    let mut any = false;
    while i < s.len() {
        let c = s[i];
        if c == b'.' && !dot {
            dot = true;
        } else if let Some(d) = (c as char).to_digit(16) {
            any = true;
            if m >> 60 == 0 {
                m = (m << 4) | d as u64;
                if dot {
                    e -= 4;
                }
            } else {
                if d != 0 {
                    sticky = true;
                }
                if !dot {
                    e += 4;
                }
            }
        } else {
            break;
        }
        i += 1;
    }
    if !any {
        return Some(0.0);
    }
    if i < s.len() && (s[i] == b'p' || s[i] == b'P') {
        let mut j = i + 1;
        let mut eneg = false;
        if j < s.len() && (s[j] == b'+' || s[j] == b'-') {
            eneg = s[j] == b'-';
            j += 1;
        }
        let mut x: i64 = 0;
        let mut digits = false;
        while j < s.len() && s[j].is_ascii_digit() {
            digits = true;
            x = (x * 10 + (s[j] - b'0') as i64).min(1 << 40);
            j += 1;
        }
        if digits {
            e += if eneg { -x } else { x };
        }
    }
    if m == 0 {
        return Some(0.0);
    }
    let bits = 64 - m.leading_zeros() as i64;
    // value = m * 2^e; keep 24 bits, or fewer in the subnormal range.
    let mut shift = bits - 24;
    if e + shift < -149 {
        shift = -149 - e;
    }
    if shift > 0 {
        if shift >= 64 {
            let r = if shift == 64 && (m >> 63) != 0 && (m << 1 != 0 || sticky) { 1u64 } else { 0 };
            return Some(scale(r, e + shift));
        }
        let rem = m & ((1u64 << shift) - 1);
        let half = 1u64 << (shift - 1);
        let mut r = m >> shift;
        if rem > half || (rem == half && (sticky || r & 1 == 1)) {
            r += 1;
        }
        Some(scale(r, e + shift))
    } else {
        Some(scale(m, e))
    }
}

fn scale(r: u64, e: i64) -> f32 {
    if r == 0 {
        return 0.0;
    }
    if e > 300 {
        return f32::INFINITY;
    }
    if e < -300 {
        return 0.0;
    }
    (r as f64 * 2f64.powi(e as i32)) as f32
}

/// `%g` of a `float` promoted to `double`: six significant digits, shortest of the two forms, no
/// trailing zeros, at least two exponent digits.
pub(crate) fn push_g(out: &mut Vec<u8>, x: f64) {
    if x.is_nan() {
        out.extend_from_slice(if x.is_sign_negative() { b"-nan" } else { b"nan" });
        return;
    }
    if x.is_infinite() {
        out.extend_from_slice(if x < 0.0 { b"-inf" } else { b"inf" });
        return;
    }
    if x == 0.0 {
        out.extend_from_slice(if x.is_sign_negative() { b"-0" } else { b"0" });
        return;
    }
    let sci = format!("{:.5e}", x);
    let (mant, ex) = sci.split_once('e').expect("{:e} prints an exponent");
    let ex: i32 = ex.parse().expect("{:e} prints a decimal exponent");
    if !(-4..6).contains(&ex) {
        out.extend_from_slice(strip_zeros(mant).as_bytes());
        out.push(b'e');
        out.push(if ex < 0 { b'-' } else { b'+' });
        let a = ex.unsigned_abs() as u64;
        if a < 10 {
            out.push(b'0');
        }
        push_u64(out, a);
    } else {
        let fixed = format!("{:.*}", (5 - ex) as usize, x);
        out.extend_from_slice(strip_zeros(&fixed).as_bytes());
    }
}

fn strip_zeros(s: &str) -> &str {
    if s.contains('.') { s.trim_end_matches('0').trim_end_matches('.') } else { s }
}

/// `atan` from FreeBSD's msun (s_atan.c, Sun's fdlibm), so that every platform agrees.  The oracle
/// calls glibc's; the difference is below an ulp of a double, and the only use is a value that is
/// narrowed to `float` and printed with six digits.  `fontfile_oracle.rs` checks the printed values
/// against the oracle for every `SlantFont` in pdftex.map.
pub(crate) fn atan(x: f64) -> f64 {
    const ATANHI: [f64; 4] = [
        4.63647609000806093515e-01,
        7.85398163397448278999e-01,
        9.82793723247329054082e-01,
        1.57079632679489655800e+00,
    ];
    const ATANLO: [f64; 4] = [
        2.26987774529616870924e-17,
        3.06161699786838301793e-17,
        1.39033110312309984516e-17,
        6.12323399573676603587e-17,
    ];
    const AT: [f64; 11] = [
        3.33333333333329318027e-01,
        -1.99999999998764832476e-01,
        1.42857142725034663711e-01,
        -1.11111104054623557880e-01,
        9.09088713343650656196e-02,
        -7.69187620504482999495e-02,
        6.66107313738753120669e-02,
        -5.83357013379057348645e-02,
        4.97687799461593236017e-02,
        -3.65315727442169155270e-02,
        1.62858201153657823623e-02,
    ];
    let bits = x.to_bits();
    let hx = (bits >> 32) as u32;
    let lx = bits as u32;
    let ix = hx & 0x7fff_ffff;
    let neg = (hx >> 31) != 0;
    if ix >= 0x4410_0000 {
        if ix > 0x7ff0_0000 || (ix == 0x7ff0_0000 && lx != 0) {
            return x + x;
        }
        let r = ATANHI[3] + ATANLO[3];
        return if neg { -r } else { r };
    }
    let mut z = x;
    let id: i32;
    if ix < 0x3fdc_0000 {
        if ix < 0x3e40_0000 {
            return x;
        }
        id = -1;
    } else {
        z = z.abs();
        if ix < 0x3ff3_0000 {
            if ix < 0x3fe6_0000 {
                id = 0;
                z = (2.0 * z - 1.0) / (2.0 + z);
            } else {
                id = 1;
                z = (z - 1.0) / (z + 1.0);
            }
        } else if ix < 0x4003_8000 {
            id = 2;
            z = (z - 1.5) / (1.0 + 1.5 * z);
        } else {
            id = 3;
            z = -1.0 / z;
        }
    }
    let w0 = z * z;
    let w = w0 * w0;
    let s1 = w0 * (AT[0] + w * (AT[2] + w * (AT[4] + w * (AT[6] + w * (AT[8] + w * AT[10])))));
    let s2 = w * (AT[1] + w * (AT[3] + w * (AT[5] + w * (AT[7] + w * AT[9]))));
    if id < 0 {
        return z - z * (s1 + s2);
    }
    let id = id as usize;
    let r = ATANHI[id] - (z * (s1 + s2) - ATANLO[id] - z);
    if neg { -r } else { r }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn g(x: f64) -> String {
        let mut v = Vec::new();
        push_g(&mut v, x);
        String::from_utf8(v).unwrap()
    }

    #[test]
    fn g_formats_like_printf() {
        assert_eq!(g(0.001), "0.001");
        assert_eq!(g(1.0), "1");
        assert_eq!(g(-0.167), "-0.167");
        assert_eq!(g(1234567.0), "1.23457e+06");
        assert_eq!(g(123456.0), "123456");
        assert_eq!(g(0.0001), "0.0001");
        assert_eq!(g(0.00001), "1e-05");
        assert_eq!(g(1e100), "1e+100");
        // Exact ties go to the even digit, as glibc does in the default rounding mode.
        assert_eq!(g(1234565.0), "1.23456e+06");
        assert_eq!(g(1234575.0), "1.23458e+06");
        assert_eq!(g(0.5), "0.5");
        assert_eq!(g(f64::INFINITY), "inf");
        assert_eq!(g(-0.0), "-0");
        // float promoted to double, the way writet1.c prints a FontMatrix
        assert_eq!(g(0.001f32 as f64), "0.001");
        assert_eq!(g(0.00083f32 as f64), "0.00083");
    }

    fn fl(s: &str) -> Option<(f32, usize)> {
        let b = s.as_bytes();
        let mut sc = Scan::new(b, 0);
        sc.float().map(|v| (v, sc.i))
    }

    #[test]
    fn float_follows_glibc() {
        assert_eq!(fl("0.5ExtendFont"), None, "an exponent with no digits fails, on the oracle");
        assert_eq!(fl("0.5 ExtendFont"), Some((0.5, 3)));
        assert_eq!(fl(".167 SlantFont"), Some((0.167, 4)));
        assert_eq!(fl("-.5"), Some((-0.5, 3)));
        assert_eq!(fl("1e3x"), Some((1000.0, 3)));
        assert_eq!(fl("1e+x"), None);
        assert_eq!(fl("0x1p"), None);
        assert_eq!(fl("  7 "), Some((7.0, 3)));
        assert_eq!(fl("."), None);
        assert_eq!(fl("e5"), None);
        assert_eq!(fl("TeXBase1Encoding"), None);
        assert_eq!(fl(""), None);
        assert_eq!(fl("0x1p-3 "), Some((0.125, 6)));
        assert_eq!(fl("0x"), None);
        assert_eq!(fl("0x10"), Some((16.0, 4)));
        assert_eq!(fl("0x1.8p1"), Some((3.0, 7)));
        assert!(fl("nan").unwrap().0.is_nan());
        assert_eq!(fl("infx"), Some((f32::INFINITY, 3)));
        assert_eq!(fl("-Infinity"), Some((f32::NEG_INFINITY, 9)));
    }

    #[test]
    fn hexadecimal_floats_round_like_strtof() {
        let v = |s: &str| fl(s).unwrap().0;
        let one = |e: i32| 1.0 + 2f32.powi(e);
        // Ties go to the even mantissa; anything past the tie, however far, goes up.
        assert_eq!(v("0x1.000001p0"), 1.0);
        assert_eq!(v("0x1.000003p0"), one(-22));
        assert_eq!(v("0x1.000001000000000000000001p0"), one(-23));
        assert_eq!(v("0x1000001000000000000p-72"), 1.0);
        // The subnormal range and the ends of the range.
        assert_eq!(v("0x1p-149"), 2f32.powi(-149));
        assert_eq!(v("0x1p-150"), 0.0);
        assert_eq!(v("0x1.8p-149"), 2.0 * 2f32.powi(-149));
        assert_eq!(v("0x1p-400"), 0.0);
        assert_eq!(v("0x1p-2000"), 0.0);
        assert_eq!(v("0x1p128"), f32::INFINITY);
        assert_eq!(v("0x1p2000"), f32::INFINITY);
        assert_eq!(v("0x0"), 0.0);
        assert_eq!(v("0x0.0p5"), 0.0);
        assert_eq!(v("0x."), 0.0);
        assert_eq!(v("-0x1.8p1"), -3.0);
        // More digits than the accumulator keeps, before and after the point.
        assert_eq!(v("0x10000000000000000000p-76"), 1.0);
        assert_eq!(v("0x1.00000000000000000000p0"), 1.0);
    }

    #[test]
    fn g_of_nan() {
        assert_eq!(g(f64::NAN), "nan");
        assert_eq!(g(-f64::NAN), "-nan");
    }

    #[test]
    fn atan_beyond_the_range_the_fonts_use() {
        for x in [1.2f64, 1.3, 2.0, 2.4, 2.5, 10.0, 1e10, 1e300, -3.0, 1e-30, 0.5, 0.75] {
            assert!((atan(x) - x.atan()).abs() <= 2.0 * f64::EPSILON, "{}", x);
        }
        assert!(atan(f64::NAN).is_nan());
        assert_eq!(atan(f64::INFINITY), core::f64::consts::FRAC_PI_2);
        assert_eq!(atan(f64::NEG_INFINITY), -core::f64::consts::FRAC_PI_2);
    }

    fn int(s: &str, base: u32) -> Option<(i64, usize)> {
        let b = s.as_bytes();
        let mut sc = Scan::new(b, 0);
        sc.int(base, usize::MAX).map(|v| (v, sc.i))
    }

    #[test]
    fn int_follows_glibc() {
        assert_eq!(int(" 12/x", 0), Some((12, 3)));
        assert_eq!(int("010", 0), Some((8, 3)));
        assert_eq!(int("0x1F ", 0), Some((31, 4)));
        assert_eq!(int("0b101", 0), Some((5, 5)));
        assert_eq!(int("-3", 0), Some((-3, 2)));
        assert_eq!(int("08", 0), Some((0, 1)));
        assert_eq!(int("/x", 0), None);
        assert_eq!(int("-", 0), None);
        assert_eq!(int("0x", 0), Some((0, 2)));
        assert_eq!(int("41", 16), Some((0x41, 2)));
    }

    #[test]
    fn suffix_ignores_one_newline() {
        let b = b"mark currentfile closefile\n\0";
        assert!(str_suffix(b, 0, 27, b"closefile"));
        assert!(!str_suffix(b, 0, 27, b"closefil"));
        assert!(!str_suffix(b, 0, 1, b"xx"));
    }

    #[test]
    fn atan_agrees_with_the_system_to_float_precision() {
        // Every SlantFont pdfTeX accepts, in thousandths.  The result is narrowed to float.
        for k in -1000..=1000 {
            let x = k as f64 * 1E-3;
            let a = atan(x);
            let b = x.atan();
            assert!((a - b).abs() <= 2.0 * f64::EPSILON, "{} {} {}", x, a, b);
            assert_eq!(a as f32, b as f32, "{}", x);
        }
    }

    #[test]
    fn hex_output() {
        let mut v = Vec::new();
        push_hex(&mut v, 0x41, 4);
        push_hex(&mut v, 0, 2);
        push_hex(&mut v, 0x10FFFF, 4);
        assert_eq!(v, b"004100" .iter().copied().chain(b"10FFFF".iter().copied()).collect::<Vec<u8>>());
    }
}
