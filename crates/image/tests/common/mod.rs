//! Shared by `tests/image_oracle.rs` and `examples/imgsim.rs`.
#![allow(dead_code)]

pub mod gen_inputs;
pub mod sim;

use rstex_image::png;

/// The `png::Kernels` this CPU can run, each by name: the plain ones, and those compiled for AVX2 where the CPU has it,
/// which the engine's `platform::png_kernels` takes there.
pub fn kernels() -> Vec<(&'static str, png::Kernels)> {
    #[allow(unused_mut)]
    let mut all = vec![("plain", png::Kernels::PLAIN)];
    #[cfg(target_arch = "x86_64")]
    if std::arch::is_x86_feature_detected!("avx2") {
        // SAFETY, for both: the CPU has AVX2, all that the functions were compiled to need.
        all.push((
            "avx2",
            png::Kernels {
                unfilter: |rows, prev, bpp| unsafe { png::unfilter_rows_avx2(rows, prev, bpp) },
                split: |row, color, alpha, rgb, wide| unsafe { png::split_row_avx2(row, color, alpha, rgb, wide) },
            },
        ));
    }
    all
}

/// SHA-256, for the table of results: the workspace has no crate for it and the tests want no new dependency.
pub fn sha256(data: &[u8]) -> [u8; 32] {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5, 0xd807aa98, 0x12835b01,
        0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc,
        0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147,
        0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
        0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116, 0x1e376c08,
        0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3, 0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208,
        0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
    ];
    let mut h: [u32; 8] = [0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19];
    let mut msg = data.to_vec();
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&((data.len() as u64) * 8).to_be_bytes());
    let mut w = [0u32; 64];
    for block in msg.chunks(64) {
        for (i, c) in block.chunks(4).enumerate() {
            w[i] = u32::from_be_bytes([c[0], c[1], c[2], c[3]]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
        }
        let mut v = h;
        for i in 0..64 {
            let s1 = v[4].rotate_right(6) ^ v[4].rotate_right(11) ^ v[4].rotate_right(25);
            let ch = (v[4] & v[5]) ^ (!v[4] & v[6]);
            let t1 = v[7].wrapping_add(s1).wrapping_add(ch).wrapping_add(K[i]).wrapping_add(w[i]);
            let s0 = v[0].rotate_right(2) ^ v[0].rotate_right(13) ^ v[0].rotate_right(22);
            let maj = (v[0] & v[1]) ^ (v[0] & v[2]) ^ (v[1] & v[2]);
            let t2 = s0.wrapping_add(maj);
            v[7] = v[6];
            v[6] = v[5];
            v[5] = v[4];
            v[4] = v[3].wrapping_add(t1);
            v[3] = v[2];
            v[2] = v[1];
            v[1] = v[0];
            v[0] = t1.wrapping_add(t2);
        }
        for i in 0..8 {
            h[i] = h[i].wrapping_add(v[i]);
        }
    }
    let mut out = [0u8; 32];
    for (i, x) in h.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&x.to_be_bytes());
    }
    out
}

/// The first 16 hex digits of the SHA-256.
pub fn short_hash(data: &[u8]) -> String {
    let h = sha256(data);
    h[..8].iter().map(|b| format!("{b:02x}")).collect()
}

/// A case's objects, as one string of bytes: number, length, bytes.
pub fn objects_blob(objects: &[(i32, Vec<u8>)]) -> Vec<u8> {
    let mut b = Vec::new();
    for (n, o) in objects {
        b.extend_from_slice(format!("{n} {}\n", o.len()).as_bytes());
        b.extend_from_slice(o);
    }
    b
}

/// The line a case comes to in the table.  Same format from the oracle (`tests/oracle/gen_image_tables.py`).
pub fn result_line(out: &sim::RunOutput) -> String {
    let seen: Vec<String> = out.seen.iter().map(|s| format!("{}/{}/{}/{}", s.pages, s.depth, s.wd, s.ht)).collect();
    let printed = String::from_utf8_lossy(&out.printed);
    let mut warnings: Vec<String> = out.warnings.iter().map(|w| String::from_utf8_lossy(w).into_owned()).collect();
    warnings.sort();
    match &out.failed {
        Some(m) => format!("seen={} fail={}", seen.join(","), String::from_utf8_lossy(m)),
        None => format!(
            "n={} h={} seen={} printed={} warn={}",
            out.objects.len(),
            short_hash(&objects_blob(&out.objects)),
            seen.join(","),
            printed,
            warnings.join("|")
        ),
    }
}

/// The table's rows group cases, so the table stays small: a family is the name up to its third `_` for the PNG sets of
/// hundreds, and otherwise the part before the first `_`.  `tests/oracle/gen_image_tables.py` has the same function.
pub fn family(name: &str) -> String {
    let b = name.as_bytes();
    let f: Vec<&str> = name.split('_').collect();
    if (b[0] == b'p' || b[0] == b'a' || b[0] == b'v') && b[1].is_ascii_digit() && f.len() >= 3 {
        f[..3].join("_")
    } else {
        f[0].to_string()
    }
}
