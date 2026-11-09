//! `png::Kernels`: the code compiled for AVX2 against the plain code, which `png.rs`'s own tests hold against libpng's.
//! The crate cannot call the AVX2 code, which takes `unsafe`, so it is called from here, by `common::kernels`, as the
//! engine's `platform::png_kernels` does, and only where the CPU has AVX2.
#![cfg(target_arch = "x86_64")]

mod common;

use rstex_image::png::Kernels;

fn avx2() -> Option<Kernels> {
    let found = common::kernels().into_iter().find(|(name, _)| *name == "avx2").map(|(_, k)| k);
    if found.is_none() {
        eprintln!("no AVX2 here: nothing to test");
    }
    found
}

struct Rnd(u32);

impl Rnd {
    fn byte(&mut self) -> u8 {
        self.0 = self.0.wrapping_mul(1664525).wrapping_add(1013904223);
        (self.0 >> 16) as u8
    }
}

/// libpng's `png_read_filter_row_paeth_1byte_pixel` predictor.
fn paeth(a: u8, b: u8, c: u8) -> u8 {
    let (a, b, c) = (a as i32, b as i32, c as i32);
    let p = b - c;
    let mut pc = a - c;
    let mut pa = p.abs();
    let pb = pc.abs();
    pc = (p + pc).abs();
    let mut pred = a;
    if pb < pa {
        pa = pb;
        pred = b;
    }
    if pc < pa {
        pred = c;
    }
    pred as u8
}

/// Groups of rows of every width of pixel, with runs of Paeth longer and shorter than a band among the other filters,
/// at widths around the ends of a band, the same under both.
#[test]
fn avx2_unfilters_as_the_plain_code() {
    let Some(avx2) = avx2() else { return };
    let mut rnd = Rnd(7);
    for bpp in [1usize, 2, 3, 4, 6, 8] {
        for width in [1usize, 2, 3, 4, 5, 6, 7, 8, 9, 10, 15, 16, 17, 31, 32, 33, 100, 257] {
            let stride = width * bpp + 1;
            for round in 0..16 {
                let h = 1 + rnd.byte() as usize % 40;
                let mut rows: Vec<u8> = (0..h * stride)
                    .map(|_| match round % 4 {
                        0 | 1 => rnd.byte(),
                        2 => [0, 255][(rnd.byte() & 1) as usize],
                        _ => rnd.byte() & 3,
                    })
                    .collect();
                for y in 0..h {
                    rows[y * stride] = match round % 3 {
                        0 => 4,
                        1 => [4, 4, 4, 4, 4, 4, 4, 0, 1, 2, 3][rnd.byte() as usize % 11],
                        _ => rnd.byte() % 5,
                    };
                }
                let prev: Vec<u8> = (0..stride).map(|_| rnd.byte()).collect();
                let mut want = rows.clone();
                (Kernels::PLAIN.unfilter)(&mut want, &prev, bpp);
                let mut got = rows.clone();
                (avx2.unfilter)(&mut got, &prev, bpp);
                assert_eq!(got, want, "bpp {bpp} width {width} round {round}");
            }
        }
    }
}

/// Each band, of pixels of 1, 2 and 4 bytes, over every `a`, `b` and `c` of Paeth's: the first row of a band of Paeth
/// rows has them at its odd pixels, `a` from the even pixel before, made by its filtered bytes, and `b` and `c` from
/// the row above.  It must come out as libpng's predictor has it; the other rows of the band are noise, which
/// `avx2_unfilters_as_the_plain_code` covers.  Each lane does the same operations, so the first row's lanes stand for
/// all.
#[test]
fn avx2_paeth_bands_take_every_input() {
    let Some(avx2) = avx2() else { return };
    let mut rnd = Rnd(11);
    for (bpp, height) in [(1usize, 8usize), (2, 4), (4, 4)] {
        // The 2^24 triples in pieces of 2^16 with one `c`, each piece the first row of a band.
        let pixels = 2 * 65536 / bpp;
        let stride = pixels * bpp + 1;
        let noise: Vec<u8> = (0..height * stride).map(|_| rnd.byte()).collect();
        let mut prev = vec![0u8; stride];
        let mut want = vec![0u8; stride];
        for c in 0..256usize {
            for t in 0..65536usize {
                let (a, b) = ((t >> 8) as u8, t as u8);
                let even = 1 + 2 * (t / bpp) * bpp + t % bpp;
                prev[even] = c as u8;
                prev[even + bpp] = b;
                want[even] = a;
                want[even + bpp] = a ^ b ^ 0x5a;
            }
            let mut rows = noise.clone();
            for x in 1..stride {
                let (left, upleft) = if x > bpp { (want[x - bpp], prev[x - bpp]) } else { (0, 0) };
                rows[x] = want[x].wrapping_sub(paeth(left, prev[x], upleft));
            }
            for y in 0..height {
                rows[y * stride] = 4;
            }
            (avx2.unfilter)(&mut rows, &prev, bpp);
            assert!(rows[1..stride] == want[1..], "bpp {bpp} c {c}");
        }
    }
}

/// Rows of each kind of pixel with alpha, across the blocks of each way and what they leave over.
#[test]
fn avx2_splits_as_the_plain_code() {
    let Some(avx2) = avx2() else { return };
    let mut rnd = Rnd(3);
    for (rgb, wide, total) in [(false, false, 2), (false, true, 4), (true, false, 4), (true, true, 8)] {
        let color = if rgb { total * 3 / 4 } else { total / 2 };
        for pixels in (0..70).chain([100, 255, 256, 257, 1000, 4096]) {
            let row: Vec<u8> = (0..pixels * total).map(|_| rnd.byte()).collect();
            let split = |k: &Kernels| {
                let mut c = vec![0u8; pixels * color + 1];
                let mut a = vec![0u8; pixels];
                (k.split)(&row, &mut c, &mut a, rgb, wide);
                c.pop();
                (c, a)
            };
            assert_eq!(split(&avx2), split(&Kernels::PLAIN), "rgb {rgb} wide {wide} pixels {pixels}");
        }
    }
}
