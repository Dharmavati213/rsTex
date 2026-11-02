//! Numbers as pdfTeX writes them.  Each appends to a buffer and allocates nothing but its growth, except
//! [`write_g`], which only C's `sprintf` callers use: a font's `FontMatrix` and `ItalicAngle`.

/// `pdf_print_int`, §686, and C's `%i`: plain decimal.
pub fn write_int(out: &mut Vec<u8>, n: i64) {
    let mut buf = [0u8; 20];
    let mut k = buf.len();
    let mut m = n.unsigned_abs();
    loop {
        k -= 1;
        buf[k] = b'0' + (m % 10) as u8;
        m /= 10;
        if m == 0 {
            break;
        }
    }
    if n < 0 {
        out.push(b'-');
    }
    out.extend_from_slice(&buf[k..]);
}

/// `pdf_print_fw_int`, §702: the low `w` digits of `n`, zero padded, as in a cross-reference table.  A
/// larger number loses its high digits, as in WEB, and `w` above 23 would overrun `dig` in WEB.
pub fn write_fw_int(out: &mut Vec<u8>, n: i64, w: usize) {
    let start = out.len();
    out.resize(start + w, b'0');
    let mut m = n;
    for k in (0..w).rev() {
        // Pascal's mod, which web2c makes C's %: a negative number gives bytes below '0', as in WEB.
        out[start + k] = (b'0' as i64 + m % 10) as u8;
        m /= 10;
    }
}

/// `pdf_out_bytes`, §702: the low `w` bytes of `n`, big-endian, as in a cross-reference stream.
pub fn write_out_bytes(out: &mut Vec<u8>, n: i64, w: usize) {
    let start = out.len();
    out.resize(start + w, 0);
    let mut m = n;
    for k in (0..w).rev() {
        out[start + k] = (m % 256) as u8;
        m /= 256;
    }
}

/// `ten_pow`, §690.
const TEN_POW: [i64; 10] = [1, 10, 100, 1000, 10000, 100000, 1000000, 10000000, 100000000, 1000000000];

/// `pdf_print_real`, §690: `m / 10^d`, with no trailing zeros and no point if nothing follows it.
pub fn write_real(out: &mut Vec<u8>, m: i32, d: usize) {
    let mut m = m as i64;
    let mut d = d;
    if m < 0 {
        out.push(b'-');
        m = -m;
    }
    write_int(out, m / TEN_POW[d]);
    m %= TEN_POW[d];
    if m > 0 {
        out.push(b'.');
        d -= 1;
        while m < TEN_POW[d] {
            out.push(b'0');
            d -= 1;
        }
        while m % 10 == 0 {
            m /= 10;
        }
        write_int(out, m);
    }
}

/// C's `%g` of a `double`.  A `float` goes through this as C passes it: widened, `x as f64`.
///
/// Six significant digits, rounded exactly as glibc does from the binary value; `%e` style if the
/// exponent is below -4 or at least 6, otherwise `%f` style; trailing zeros and a bare point removed.
pub fn write_g(out: &mut Vec<u8>, x: f64) {
    const P: i32 = 6;
    if x.is_nan() {
        // glibc prints the sign of a NaN.
        out.extend_from_slice(if x.is_sign_negative() { b"-nan" } else { b"nan" });
        return;
    }
    if x.is_infinite() {
        out.extend_from_slice(if x < 0.0 { b"-inf" } else { b"inf" });
        return;
    }
    if x.is_sign_negative() {
        out.push(b'-');
    }
    let a = x.abs();
    if a == 0.0 {
        out.push(b'0');
        return;
    }
    // Rust's {:e} with a precision rounds the exact binary value half to even, as glibc does.
    let e = format!("{:.*e}", (P - 1) as usize, a);
    let (mantissa, exp) = e.split_once('e').expect("format of {:e}");
    let exp: i32 = exp.parse().expect("format of {:e}");
    if exp >= -4 && exp < P {
        let f = format!("{:.*}", (P - 1 - exp) as usize, a);
        out.extend_from_slice(strip_zeros(&f).as_bytes());
    } else {
        out.extend_from_slice(strip_zeros(mantissa).as_bytes());
        out.push(b'e');
        out.push(if exp < 0 { b'-' } else { b'+' });
        let ea = exp.unsigned_abs();
        // At least two digits.
        if ea < 10 {
            out.push(b'0');
        }
        write_int(out, ea as i64);
    }
}

/// Removes the zeros at the end of a fraction, and the point if nothing is left after it.
fn strip_zeros(s: &str) -> &str {
    if s.contains('.') { s.trim_end_matches('0').trim_end_matches('.') } else { s }
}

const HEX_UPPER: &[u8; 16] = b"0123456789ABCDEF";

/// `convertStringToHexString`: each byte as `%02X`, for `/ID` and `\pdfmdfivesum`.
pub fn write_hex(out: &mut Vec<u8>, bytes: &[u8]) {
    out.reserve(2 * bytes.len());
    for &b in bytes {
        out.push(HEX_UPPER[(b >> 4) as usize]);
        out.push(HEX_UPPER[(b & 15) as usize]);
    }
}

/// C's `%0wX`, as in a ToUnicode CMap: `%02X`, `%04X`.  More digits than `w` are all written.
pub fn write_hex_width(out: &mut Vec<u8>, n: u64, w: usize) {
    let digits = ((64 - n.leading_zeros() as usize).div_ceil(4)).max(1);
    for _ in digits..w {
        out.push(b'0');
    }
    for i in (0..digits).rev() {
        out.push(HEX_UPPER[((n >> (4 * i)) & 15) as usize]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(f: impl FnOnce(&mut Vec<u8>)) -> String {
        let mut v = Vec::new();
        f(&mut v);
        String::from_utf8_lossy(&v).into_owned()
    }

    #[test]
    fn ints() {
        for (n, t) in [(0i64, "0"), (7, "7"), (-7, "-7"), (10, "10"), (1234567890123, "1234567890123"), (i64::MIN, "-9223372036854775808"), (i64::MAX, "9223372036854775807"), (-100000000, "-100000000"), (-100000001, "-100000001")] {
            assert_eq!(s(|o| write_int(o, n)), t);
        }
    }

    #[test]
    fn fixed_width_as_a_cross_reference_table() {
        assert_eq!(s(|o| write_fw_int(o, 0, 10)), "0000000000");
        assert_eq!(s(|o| write_fw_int(o, 12345, 10)), "0000012345");
        assert_eq!(s(|o| write_fw_int(o, 65535, 5)), "65535");
        assert_eq!(s(|o| write_fw_int(o, 123456, 3)), "456", "the high digits go, as in WEB");
        assert_eq!(s(|o| write_fw_int(o, 5, 0)), "");
    }

    #[test]
    fn out_bytes_as_a_cross_reference_stream() {
        let bytes = |n, w| {
            let mut v = vec![];
            write_out_bytes(&mut v, n, w);
            v
        };
        assert_eq!(bytes(0x010203, 3), [1, 2, 3]);
        assert_eq!(bytes(0x010203, 2), [2, 3]);
        assert_eq!(bytes(0x01, 4), [0, 0, 0, 1]);
        assert_eq!(bytes(0, 1), [0]);
    }

    #[test]
    fn reals_as_section_690() {
        for (m, d, t) in [
            (0, 3, "0"),
            (1000, 3, "1"),
            (1500, 3, "1.5"),
            (1005, 3, "1.005"),
            (1050, 3, "1.05"),
            (5, 3, "0.005"),
            (50, 3, "0.05"),
            (500, 3, "0.5"),
            (-1500, 3, "-1.5"),
            (-5, 4, "-0.0005"),
            (99626, 4, "9.9626"),
            (100000, 4, "10"),
            (123, 0, "123"),
            (-123, 0, "-123"),
            (i32::MAX, 9, "2.147483647"),
            (i32::MIN, 9, "-2.147483648"),
        ] {
            assert_eq!(s(|o| write_real(o, m, d)), t, "{m} {d}");
        }
    }

    #[test]
    fn hex() {
        assert_eq!(s(|o| write_hex(o, &[0, 0x0f, 0xab, 0xff])), "000FABFF");
        assert_eq!(s(|o| write_hex(o, &[])), "");
        assert_eq!(s(|o| write_hex_width(o, 0xab, 2)), "AB");
        assert_eq!(s(|o| write_hex_width(o, 0xa, 2)), "0A");
        assert_eq!(s(|o| write_hex_width(o, 0xa, 4)), "000A");
        assert_eq!(s(|o| write_hex_width(o, 0, 2)), "00");
        assert_eq!(s(|o| write_hex_width(o, 0, 0)), "0");
        assert_eq!(s(|o| write_hex_width(o, 0x1f600, 4)), "1F600");
        assert_eq!(s(|o| write_hex_width(o, u64::MAX, 2)), "FFFFFFFFFFFFFFFF");
    }

    /// `crates/pdfio/testdata/numfmt_g.tsv`, made by `tests/oracle/gen_pdfio_tables.py`.
    #[test]
    fn g_as_c_prints_it() {
        let table = include_str!("../testdata/numfmt_g.tsv");
        let mut n = 0;
        for line in table.lines() {
            let (bits, text) = line.split_once('\t').unwrap();
            let x = f64::from_bits(u64::from_str_radix(bits, 16).unwrap());
            assert_eq!(s(|o| write_g(o, x)), text, "{x:e}");
            n += 1;
        }
        assert!(n > 1000, "{n} cases");
    }

    #[test]
    fn g_of_the_values_a_font_has() {
        for (x, t) in [(0.001f64, "0.001"), (1.0, "1"), (0.0, "0"), (-0.0, "-0"), (100000.0, "100000"), (1000000.0, "1e+06"), (0.0001, "0.0001"), (0.00001, "1e-05")] {
            assert_eq!(s(|o| write_g(o, x)), t);
        }
        // Widened from a float, as C passes it: not the same as the shortest decimal of the float.
        assert_eq!(s(|o| write_g(o, 0.001f32 as f64)), "0.001");
        assert_eq!(s(|o| write_g(o, -16.5f32 as f64)), "-16.5");
    }
}
