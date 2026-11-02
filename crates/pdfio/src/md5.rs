//! MD5, RFC 1321, in safe Rust.  pdfTeX uses it for `\pdfmdfivesum`, the `/ID` of a file, and the
//! subset tag of a font.  None of these is a security matter.

/// A digest being computed: `md5_state_t`.
#[derive(Clone)]
pub struct Md5 {
    state: [u32; 4],
    /// Message length so far, in bytes.
    len: u64,
    block: [u8; 64],
    used: usize,
}

impl Default for Md5 {
    fn default() -> Md5 {
        Md5::new()
    }
}

/// RFC 1321 section 3.4, the table of `floor(2^32 * abs(sin(i)))`.
const K: [u32; 64] = [
    0xd76aa478, 0xe8c7b756, 0x242070db, 0xc1bdceee, 0xf57c0faf, 0x4787c62a, 0xa8304613, 0xfd469501, 0x698098d8, 0x8b44f7af,
    0xffff5bb1, 0x895cd7be, 0x6b901122, 0xfd987193, 0xa679438e, 0x49b40821, 0xf61e2562, 0xc040b340, 0x265e5a51, 0xe9b6c7aa,
    0xd62f105d, 0x02441453, 0xd8a1e681, 0xe7d3fbc8, 0x21e1cde6, 0xc33707d6, 0xf4d50d87, 0x455a14ed, 0xa9e3e905, 0xfcefa3f8,
    0x676f02d9, 0x8d2a4c8a, 0xfffa3942, 0x8771f681, 0x6d9d6122, 0xfde5380c, 0xa4beea44, 0x4bdecfa9, 0xf6bb4b60, 0xbebfbc70,
    0x289b7ec6, 0xeaa127fa, 0xd4ef3085, 0x04881d05, 0xd9d4d039, 0xe6db99e5, 0x1fa27cf8, 0xc4ac5665, 0xf4292244, 0x432aff97,
    0xab9423a7, 0xfc93a039, 0x655b59c3, 0x8f0ccc92, 0xffeff47d, 0x85845dd1, 0x6fa87e4f, 0xfe2ce6e0, 0xa3014314, 0x4e0811a1,
    0xf7537e82, 0xbd3af235, 0x2ad7d2bb, 0xeb86d391,
];

/// The shift of each step.
const S: [u32; 64] = [
    7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 5, 9, 14, 20, 5, 9, 14, 20, 5, 9, 14, 20, 5, 9, 14, 20, 4, 11, 16, 23, 4,
    11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21,
];

fn compress(state: &mut [u32; 4], block: &[u8]) {
    let mut x = [0u32; 16];
    for (w, c) in x.iter_mut().zip(block.chunks_exact(4)) {
        *w = u32::from_le_bytes([c[0], c[1], c[2], c[3]]);
    }
    let [mut a, mut b, mut c, mut d] = *state;
    for i in 0..64 {
        let (f, g) = match i / 16 {
            0 => ((b & c) | (!b & d), i),
            1 => ((d & b) | (!d & c), (5 * i + 1) % 16),
            2 => (b ^ c ^ d, (3 * i + 5) % 16),
            _ => (c ^ (b | !d), (7 * i) % 16),
        };
        let t = a.wrapping_add(f).wrapping_add(K[i]).wrapping_add(x[g]);
        a = d;
        d = c;
        c = b;
        b = b.wrapping_add(t.rotate_left(S[i]));
    }
    state[0] = state[0].wrapping_add(a);
    state[1] = state[1].wrapping_add(b);
    state[2] = state[2].wrapping_add(c);
    state[3] = state[3].wrapping_add(d);
}

impl Md5 {
    /// `md5_init`.
    pub const fn new() -> Md5 {
        Md5 { state: [0x67452301, 0xefcdab89, 0x98badcfe, 0x10325476], len: 0, block: [0; 64], used: 0 }
    }

    /// `md5_append`.
    pub fn update(&mut self, mut data: &[u8]) {
        self.len = self.len.wrapping_add(data.len() as u64);
        if self.used > 0 {
            let take = data.len().min(64 - self.used);
            self.block[self.used..self.used + take].copy_from_slice(&data[..take]);
            self.used += take;
            data = &data[take..];
            if self.used < 64 {
                return;
            }
            let block = self.block;
            compress(&mut self.state, &block);
            self.used = 0;
        }
        let mut blocks = data.chunks_exact(64);
        for b in &mut blocks {
            compress(&mut self.state, b);
        }
        let rest = blocks.remainder();
        self.block[..rest.len()].copy_from_slice(rest);
        self.used = rest.len();
    }

    /// `md5_finish`.
    pub fn finish(mut self) -> [u8; 16] {
        let bits = self.len.wrapping_mul(8);
        self.update(&[0x80]);
        while self.used != 56 {
            self.update(&[0]);
        }
        self.update(&bits.to_le_bytes());
        let mut out = [0u8; 16];
        for (o, w) in out.chunks_exact_mut(4).zip(self.state) {
            o.copy_from_slice(&w.to_le_bytes());
        }
        out
    }
}

/// The digest of `data`.
pub fn md5(data: &[u8]) -> [u8; 16] {
    let mut m = Md5::new();
    m.update(data);
    m.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::numfmt::write_hex;

    fn hex(d: [u8; 16]) -> String {
        let mut v = Vec::new();
        write_hex(&mut v, &d);
        String::from_utf8(v).unwrap().to_ascii_lowercase()
    }

    /// RFC 1321, appendix A.5: every vector of its test suite.
    #[test]
    fn rfc_1321_vectors() {
        for (input, digest) in [
            (&b""[..], "d41d8cd98f00b204e9800998ecf8427e"),
            (b"a", "0cc175b9c0f1b6a831c399e269772661"),
            (b"abc", "900150983cd24fb0d6963f7d28e17f72"),
            (b"message digest", "f96b697d7cb7938d525a2f31aaf161d0"),
            (b"abcdefghijklmnopqrstuvwxyz", "c3fcd3d76192e4007dfb496cca67e13b"),
        ] {
            assert_eq!(hex(md5(input)), digest, "{}", String::from_utf8_lossy(input));
        }
        let alnum: Vec<u8> = (b'A'..=b'Z').chain(b'a'..=b'z').chain(b'0'..=b'9').collect();
        assert_eq!(hex(md5(&alnum)), "d174ab98d277d9f5a5611c2c9f419d9f");
        assert_eq!(hex(md5(&b"1234567890".repeat(8))), "57edf4a22be3c955ac49da2e2107b67a");
    }

    #[test]
    fn pieces_give_the_same_digest() {
        let data: Vec<u8> = (0..1000u32).map(|i| (i.wrapping_mul(31) >> 2) as u8).collect();
        for n in [0, 1, 54, 55, 56, 57, 63, 64, 65, 119, 120, 128, 1000] {
            let whole = md5(&data[..n]);
            for piece in [1, 3, 63, 64, 65, 200] {
                let mut m = Md5::new();
                for c in data[..n].chunks(piece) {
                    m.update(c);
                }
                assert_eq!(m.finish(), whole, "length {n}, pieces of {piece}");
            }
        }
    }

    #[test]
    fn a_clone_goes_on_independently() {
        let mut a = Md5::new();
        a.update(b"mess");
        let mut b = a.clone();
        a.update(b"age digest");
        b.update(b"age digest");
        assert_eq!(a.finish(), md5(b"message digest"));
        assert_eq!(b.finish(), md5(b"message digest"));
    }
}
