//! Deflate through zlib-rs, chunk writer helpers, MD5 (DESIGN.md section 9, M2).
//!
//! Pure Rust: zlib-rs at level 6 for content and object streams, level 1 for FontFile.
#![forbid(unsafe_code)]

/// Deflates `input` as a zlib stream at `level`, appending to `out`.
pub fn deflate(input: &[u8], level: i32, out: &mut Vec<u8>) {
    let start = out.len();
    let bound = zlib_rs::compress_bound(input.len());
    out.resize(start + bound, 0);
    let config = zlib_rs::DeflateConfig::new(level);
    let (compressed, rc) = zlib_rs::compress_slice(&mut out[start..], input, config);
    assert!(rc == zlib_rs::ReturnCode::Ok, "zlib-rs: compress failed");
    let n = compressed.len();
    out.truncate(start + n);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deflate_shrinks_and_has_a_zlib_header() {
        let input = vec![b'a'; 10000];
        let mut out = vec![1, 2];
        deflate(&input, 6, &mut out);
        assert_eq!(&out[..2], &[1, 2], "appends");
        assert_eq!(out[2], 0x78, "zlib header");
        assert!(out.len() < 200);
    }
}
