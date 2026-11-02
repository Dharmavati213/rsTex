//! Test inputs, made at run time and the same every time: PNG, JPEG, JBIG2 and PDF files, and the cases that use them.
//!
//! `tests/oracle/gen_image_tables.py` writes these files under `target/`, runs them through the oracle and commits the
//! results as a table; `tests/image_oracle.rs` makes them again and checks the port against the table.  So nothing
//! binary is committed.  JPEG and JBIG2 data here is not real image data: pdfTeX reads the headers and copies the rest.

use std::collections::BTreeMap;

/// A case: `\immediate\pdfximage` of each image in turn, under these engine parameters.
#[derive(Clone, Debug)]
pub struct Case {
    pub name: String,
    pub params: String,
    pub images: Vec<String>,
}

/// All of it.
#[derive(Default)]
pub struct Set {
    pub files: BTreeMap<String, Vec<u8>>,
    pub cases: Vec<Case>,
    /// PostScript name to font file, for the fake fonts of the PDF cases: entries of the real pdftex.map.
    pub fontmap: Vec<(String, String)>,
}

impl Set {
    fn file(&mut self, name: &str, data: Vec<u8>) {
        self.files.insert(name.to_string(), data);
    }

    fn case(&mut self, name: &str, params: &str, images: &[&str]) {
        self.cases.push(Case {
            name: name.to_string(),
            params: params.to_string(),
            images: images.iter().map(|s| s.to_string()).collect(),
        });
    }

    /// A file used by one case with the given parameters.
    fn one(&mut self, name: &str, params: &str, data: Vec<u8>) {
        self.file(name, data);
        self.case(name, params, &[name]);
    }
}

//------------------------------------------------------------------------
// PNG
//------------------------------------------------------------------------

fn crc(data: &[u8]) -> u32 {
    zlib_rs::crc32::crc32(0, data)
}

pub fn png_chunk(out: &mut Vec<u8>, name: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    let mut c = name.to_vec();
    c.extend_from_slice(data);
    out.extend_from_slice(&c);
    out.extend_from_slice(&crc(&c).to_be_bytes());
}

pub fn deflate(input: &[u8], level: i32) -> Vec<u8> {
    let mut out = vec![0u8; zlib_rs::compress_bound(input.len()) + 64];
    let config = zlib_rs::DeflateConfig::new(level);
    let (c, rc) = zlib_rs::compress_slice(&mut out, input, config);
    assert!(rc == zlib_rs::ReturnCode::Ok);
    c.to_vec()
}

fn channels(ct: u8) -> usize {
    match ct {
        2 => 3,
        4 => 2,
        6 => 4,
        _ => 1,
    }
}

/// A sample at (x, y, channel), fitting the bit depth.
fn sample(x: usize, y: usize, c: usize, bd: u8) -> u32 {
    let v = (x * 7 + y * 13 + c * 29 + x * y * 3 + 5) as u32;
    match bd {
        16 => (v.wrapping_mul(2654435761) >> 16) & 0xffff,
        8 => v.wrapping_mul(97) & 0xff,
        n => v & ((1u32 << n) - 1),
    }
}

fn filter_row(ft: u8, row: &[u8], prev: &[u8], bpp: usize, out: &mut Vec<u8>) {
    out.push(ft);
    for i in 0..row.len() {
        let a = if i >= bpp { row[i - bpp] as i32 } else { 0 };
        let b = prev[i] as i32;
        let c = if i >= bpp { prev[i - bpp] as i32 } else { 0 };
        let pred = match ft {
            0 => 0,
            1 => a,
            2 => b,
            3 => (a + b) / 2,
            _ => {
                let p = a + b - c;
                let (pa, pb, pc) = ((p - a).abs(), (p - b).abs(), (p - c).abs());
                if pa <= pb && pa <= pc {
                    a
                } else if pb <= pc {
                    b
                } else {
                    c
                }
            }
        };
        out.push((row[i] as i32 - pred) as u8);
    }
}

/// The filtered image data: one sub-image, or the seven of Adam7.
fn png_scanlines(w: usize, h: usize, ct: u8, bd: u8, interlace: bool, filters: bool, palette_len: usize) -> Vec<u8> {
    let ch = channels(ct);
    let depth = ch * bd as usize;
    let bpp = (depth + 7) >> 3;
    let mut out = Vec::new();
    let mut emit = |xs: usize, xi: usize, ys: usize, yi: usize| {
        let iw = (w + xi - 1 - xs) / xi;
        let ih = (h + yi - 1 - ys) / yi;
        if iw == 0 || ih == 0 {
            return;
        }
        let rb = (iw * depth + 7) >> 3;
        let mut prev = vec![0u8; rb];
        for iy in 0..ih {
            let y = ys + iy * yi;
            let mut row = vec![0u8; rb];
            for ix in 0..iw {
                let x = xs + ix * xi;
                for c in 0..ch {
                    let mut v = sample(x, y, c, bd);
                    if ct == 3 && palette_len > 0 {
                        v %= palette_len as u32;
                    }
                    let bit = (ix * ch + c) * bd as usize;
                    match bd {
                        16 => {
                            row[bit / 8] = (v >> 8) as u8;
                            row[bit / 8 + 1] = v as u8;
                        }
                        8 => row[bit / 8] = v as u8,
                        n => row[bit / 8] |= (v as u8) << (8 - n as usize - (bit & 7)),
                    }
                }
            }
            let ft = if filters { (iy % 5) as u8 } else { 0 };
            filter_row(ft, &row, &prev, bpp, &mut out);
            prev = row;
        }
    };
    if interlace {
        for p in 0..7 {
            let (xs, xi, ys, yi) = [(0, 8, 0, 8), (4, 8, 0, 8), (0, 4, 4, 8), (2, 4, 0, 4), (0, 2, 2, 4), (1, 2, 0, 2), (0, 1, 1, 2)][p];
            emit(xs, xi, ys, yi);
        }
    } else {
        emit(0, 1, 0, 1);
    }
    out
}

/// How to build a PNG.
#[derive(Clone)]
pub struct PngOpt {
    w: usize,
    h: usize,
    ct: u8,
    bd: u8,
    interlace: bool,
    filters: bool,
    idat_split: usize,
    palette: usize,
    before_plte: Vec<(&'static [u8; 4], Vec<u8>)>,
    after_plte: Vec<(&'static [u8; 4], Vec<u8>)>,
    between_idat: Vec<(&'static [u8; 4], Vec<u8>)>,
    after_idat: Vec<(&'static [u8; 4], Vec<u8>)>,
}

impl PngOpt {
    fn new(w: usize, h: usize, ct: u8, bd: u8) -> PngOpt {
        PngOpt {
            w,
            h,
            ct,
            bd,
            interlace: false,
            filters: true,
            idat_split: 0,
            palette: 1usize << bd.min(8),
            before_plte: Vec::new(),
            after_plte: Vec::new(),
            between_idat: Vec::new(),
            after_idat: Vec::new(),
        }
    }

    fn laced(mut self, on: bool) -> PngOpt {
        self.interlace = on;
        self
    }

    fn before(mut self, name: &'static [u8; 4], d: Vec<u8>) -> PngOpt {
        self.before_plte.push((name, d));
        self
    }

    fn after(mut self, name: &'static [u8; 4], d: Vec<u8>) -> PngOpt {
        self.after_plte.push((name, d));
        self
    }

    fn build(&self) -> Vec<u8> {
        let mut out = vec![137, 80, 78, 71, 13, 10, 26, 10];
        let mut ihdr = Vec::new();
        ihdr.extend_from_slice(&(self.w as u32).to_be_bytes());
        ihdr.extend_from_slice(&(self.h as u32).to_be_bytes());
        ihdr.extend_from_slice(&[self.bd, self.ct, 0, 0, self.interlace as u8]);
        png_chunk(&mut out, b"IHDR", &ihdr);
        for (n, d) in &self.before_plte {
            png_chunk(&mut out, n, d);
        }
        if self.ct == 3 {
            let mut plte = Vec::new();
            for i in 0..self.palette {
                plte.extend_from_slice(&[(i * 37 + 3) as u8, (i * 91 + 7) as u8, (i * 13 + 200) as u8]);
            }
            png_chunk(&mut out, b"PLTE", &plte);
        }
        for (n, d) in &self.after_plte {
            png_chunk(&mut out, n, d);
        }
        let raw = png_scanlines(self.w, self.h, self.ct, self.bd, self.interlace, self.filters, self.palette);
        let z = deflate(&raw, 6);
        if self.idat_split == 0 {
            png_chunk(&mut out, b"IDAT", &z);
        } else {
            let mut parts = z.chunks(self.idat_split);
            if let Some(first) = parts.next() {
                png_chunk(&mut out, b"IDAT", first);
            }
            for (n, d) in &self.between_idat {
                png_chunk(&mut out, n, d);
            }
            for p in parts {
                png_chunk(&mut out, b"IDAT", p);
            }
        }
        for (n, d) in &self.after_idat {
            png_chunk(&mut out, n, d);
        }
        png_chunk(&mut out, b"IEND", b"");
        out
    }
}

fn be32(v: u32) -> Vec<u8> {
    v.to_be_bytes().to_vec()
}

/// A minimal ICC profile, accepted by `png_icc_check_header`.
fn icc_profile(space: &[u8; 4]) -> Vec<u8> {
    let mut p = vec![0u8; 132 + 12 + 20];
    let n = p.len() as u32;
    p[0..4].copy_from_slice(&n.to_be_bytes());
    p[8] = 2;
    p[12..16].copy_from_slice(b"mntr");
    p[16..20].copy_from_slice(space);
    p[20..24].copy_from_slice(b"XYZ ");
    p[36..40].copy_from_slice(b"acsp");
    p[68..80].copy_from_slice(&[0x00, 0x00, 0xf6, 0xd6, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0xd3, 0x2d]);
    p[128..132].copy_from_slice(&1u32.to_be_bytes());
    p[132..136].copy_from_slice(b"wtpt");
    p[136..140].copy_from_slice(&144u32.to_be_bytes());
    p[140..144].copy_from_slice(&20u32.to_be_bytes());
    p
}

fn iccp_chunk(space: &[u8; 4]) -> Vec<u8> {
    let mut d = b"icc\0\0".to_vec();
    // Stored, so the chunk is long enough to pass libpng's test on the first 81 bytes and what follows them.
    let prof = icc_profile(space);
    d.extend_from_slice(&deflate_stored(&prof));
    d
}

/// A zlib stream of stored blocks.
fn deflate_stored(input: &[u8]) -> Vec<u8> {
    let mut out = vec![0x78, 0x01];
    let n = input.len();
    out.push(1);
    out.extend_from_slice(&(n as u16).to_le_bytes());
    out.extend_from_slice(&(!(n as u16)).to_le_bytes());
    out.extend_from_slice(input);
    let (mut a, mut b) = (1u32, 0u32);
    for &c in input {
        a = (a + c as u32) % 65521;
        b = (b + a) % 65521;
    }
    out.extend_from_slice(&((b << 16) | a).to_be_bytes());
    out
}

const COMBOS: [(u8, u8); 15] = [
    (0, 1),
    (0, 2),
    (0, 4),
    (0, 8),
    (0, 16),
    (2, 8),
    (2, 16),
    (3, 1),
    (3, 2),
    (3, 4),
    (3, 8),
    (4, 8),
    (4, 16),
    (6, 8),
    (6, 16),
];

fn trns_for(ct: u8, bd: u8) -> Option<Vec<u8>> {
    match ct {
        0 => Some(((1u32 << bd.min(15)) as u16 / 2).to_be_bytes().to_vec()),
        2 => {
            let mut v = sample(1, 1, 0, bd).to_be_bytes()[2..].to_vec();
            v.extend_from_slice(&sample(1, 1, 1, bd).to_be_bytes()[2..]);
            v.extend_from_slice(&sample(1, 1, 2, bd).to_be_bytes()[2..]);
            Some(v)
        }
        3 => Some(vec![0, 128, 255]),
        _ => None,
    }
}

fn gen_png(s: &mut Set) {
    // Colour type, depth, interlace and size.
    for &(ct, bd) in &COMBOS {
        for lace in [false, true] {
            for &(w, h) in &[(1, 1), (2, 2), (4, 3), (8, 8), (9, 7), (16, 1), (1, 16), (33, 17)] {
                let name = format!("p{ct}_{bd}_{}_{w}x{h}.png", if lace { "i" } else { "n" });
                s.one(&name, "", PngOpt::new(w, h, ct, bd).laced(lace).build());
            }
        }
    }
    // Ancillary chunks, for the copy decision and for gamma.
    for &(ct, bd) in &COMBOS {
        for lace in [false, true] {
            let l = if lace { "i" } else { "n" };
            let base = || PngOpt::new(9, 7, ct, bd).laced(lace);
            let sbit_len = if ct == 3 { 3 } else { channels(ct) };
            let sdepth = if ct == 3 { 8 } else { bd };
            let bkgd = match ct {
                3 => vec![0],
                0 | 4 => vec![0, 1],
                _ => vec![0, 1, 0, 2, 0, 3],
            };
            let hist = vec![0u8; 2 * (1usize << bd.min(8))];
            let variants: Vec<(&str, PngOpt)> = vec![
                ("gama1", base().before(b"gAMA", be32(100000))),
                ("gama45", base().before(b"gAMA", be32(45455))),
                ("gama0", base().before(b"gAMA", be32(0))),
                ("srgb", base().before(b"sRGB", vec![0])),
                ("srgbgama", base().before(b"sRGB", vec![0]).before(b"gAMA", be32(45455))),
                ("chrm", base().before(b"cHRM", {
                    let mut v = Vec::new();
                    for n in [31270u32, 32900, 64000, 33000, 30000, 60000, 15000, 6000] {
                        v.extend_from_slice(&be32(n));
                    }
                    v
                })),
                ("sbit", base().before(b"sBIT", vec![sdepth.min(5); sbit_len])),
                ("bkgd", base().after(b"bKGD", bkgd)),
                ("phys", base().before(b"pHYs", [be32(3780), be32(3780), vec![1]].concat())),
                ("physu", base().before(b"pHYs", [be32(2), be32(3), vec![0]].concat())),
                ("text", base().before(b"tEXt", b"Title\0x".to_vec())),
                ("splt", base().after(b"sPLT", {
                    let mut v = b"p\0".to_vec();
                    v.push(8);
                    v.extend_from_slice(&[1, 2, 3, 4, 0, 1]);
                    v
                })),
                ("iccp", base().before(b"iCCP", iccp_chunk(if ct & 2 != 0 { b"RGB " } else { b"GRAY" }))),
                ("iccpbad", base().before(b"iCCP", iccp_chunk(if ct & 2 != 0 { b"GRAY" } else { b"RGB " }))),
                ("unknown", base().before(b"quXx", vec![1, 2, 3])),
            ];
            for (tag, o) in variants {
                s.one(&format!("a{ct}_{bd}_{l}_{tag}.png"), "", o.build());
            }
            if ct == 3 {
                s.one(&format!("a{ct}_{bd}_{l}_hist.png"), "", base().after(b"hIST", hist).build());
            }
            if let Some(t) = trns_for(ct, bd) {
                s.one(&format!("a{ct}_{bd}_{l}_trns.png"), "", base().after(b"tRNS", t).build());
            }
        }
    }
    // The engine's parameters: 16-bit, PDF versions, gamma.
    let params: [(&str, &str); 10] = [
        ("hi0", "hicolor=0"),
        ("min4", "minor=4"),
        ("min3", "minor=3"),
        ("min2", "minor=2"),
        ("min1", "minor=1"),
        ("maj2", "major=2 minor=0"),
        ("ag1", "applygamma=1 gamma=1000 imagegamma=2200"),
        ("ag2", "applygamma=1 gamma=2200 imagegamma=2200"),
        ("ag3", "applygamma=1 gamma=1800 imagegamma=1000"),
        ("ag4", "applygamma=1 gamma=2200 imagegamma=2200 hicolor=0"),
    ];
    for &(ct, bd) in &COMBOS {
        for lace in [false, true] {
            let l = if lace { "i" } else { "n" };
            let plain = PngOpt::new(9, 7, ct, bd).laced(lace).build();
            let gama = PngOpt::new(9, 7, ct, bd).laced(lace).before(b"gAMA", be32(45455)).build();
            let sbit = PngOpt::new(9, 7, ct, bd).laced(lace).before(b"sBIT", vec![if ct == 3 { 5 } else { bd.min(5) }; if ct == 3 { 3 } else { channels(ct) }]).build();
            let plain_name = format!("v{ct}_{bd}_{l}.png");
            let gama_name = format!("v{ct}_{bd}_{l}_g.png");
            let sbit_name = format!("v{ct}_{bd}_{l}_s.png");
            s.file(&plain_name, plain);
            s.file(&gama_name, gama);
            s.file(&sbit_name, sbit);
            let trns = trns_for(ct, bd).map(|t| {
                let n = format!("v{ct}_{bd}_{l}_t.png");
                s.file(&n, PngOpt::new(9, 7, ct, bd).laced(lace).after(b"tRNS", t).build());
                n
            });
            for (tag, p) in params {
                s.case(&format!("v{ct}_{bd}_{l}_{tag}"), p, &[&plain_name]);
                s.case(&format!("v{ct}_{bd}_{l}_{tag}_g"), p, &[&gama_name]);
                if tag.starts_with("ag") {
                    s.case(&format!("v{ct}_{bd}_{l}_{tag}_s"), p, &[&sbit_name]);
                }
                if let Some(n) = &trns {
                    s.case(&format!("v{ct}_{bd}_{l}_{tag}_t"), p, &[n.as_str()]);
                }
            }
        }
    }
    // Structure and failures.
    let base = || PngOpt::new(9, 7, 2, 8);
    let mut split = base();
    split.idat_split = 11;
    s.one("s_split.png", "", split.build());
    let mut split = base().before(b"gAMA", be32(45455));
    split.idat_split = 11;
    s.one("s_split_g.png", "", split.build());
    let mut broken = base();
    broken.idat_split = 11;
    broken.between_idat.push((b"tEXt", b"a\0b".to_vec()));
    s.one("s_idat_broken.png", "", broken.build());
    let mut after = base();
    after.after_idat.push((b"tEXt", b"a\0b".to_vec()));
    s.one("s_after_idat.png", "", after.build());
    let mut zero = base();
    zero.idat_split = 1000000;
    s.one("s_one_idat.png", "", zero.build());
    let mut nofilter = base();
    nofilter.filters = false;
    s.one("s_nofilter.png", "", nofilter.build());
    let good = base().before(b"gAMA", be32(45455)).build();
    // Damage, each through the path that re-encodes.
    let idat = find_chunk(&good, b"IDAT").unwrap();
    let mut d = good.clone();
    d[idat + 8] ^= 0x55;
    s.one("e_idat_flip.png", "", d);
    let mut d = good.clone();
    let len = u32::from_be_bytes([good[idat], good[idat + 1], good[idat + 2], good[idat + 3]]) as usize;
    d[idat + 8 + len] ^= 1;
    s.one("e_idat_crc.png", "", d);
    let mut d = good.clone();
    d[idat + 8 + len - 1] ^= 1;
    s.one("e_adler.png", "", d);
    let mut d = good.clone();
    d.truncate(idat + 20);
    s.one("e_truncated.png", "", d);
    let mut d = good.clone();
    d[8 + 8 + 9] = 1; // colour type 1
    fix_chunk_crc(&mut d, 8);
    s.one("e_colortype.png", "", d);
    let mut d = good.clone();
    d[8 + 4] = b'X';
    s.one("e_no_ihdr.png", "", d);
    let mut d = good.clone();
    d[3] = b'x';
    s.file("e_sig.png", d);
    s.case("e_sig.png", "", &["e_sig.png"]);
    let mut crit = vec![137, 80, 78, 71, 13, 10, 26, 10];
    let mut ihdr = be32(2);
    ihdr.extend_from_slice(&be32(2));
    ihdr.extend_from_slice(&[8, 0, 0, 0, 0]);
    png_chunk(&mut crit, b"IHDR", &ihdr);
    png_chunk(&mut crit, b"CRIT", b"x");
    png_chunk(&mut crit, b"IDAT", &deflate(&[0, 1, 2, 0, 3, 4], 6));
    png_chunk(&mut crit, b"IEND", b"");
    s.one("e_critical.png", "", crit);
    let mut short = vec![137, 80, 78, 71, 13, 10, 26, 10];
    let mut ihdr = be32(4);
    ihdr.extend_from_slice(&be32(4));
    ihdr.extend_from_slice(&[8, 0, 0, 0, 0]);
    png_chunk(&mut short, b"IHDR", &ihdr);
    png_chunk(&mut short, b"gAMA", &be32(45455));
    png_chunk(&mut short, b"IDAT", &deflate(&[0, 1, 2], 6));
    png_chunk(&mut short, b"IEND", b"");
    s.one("e_short_data.png", "", short);
    let mut long = vec![137, 80, 78, 71, 13, 10, 26, 10];
    let mut ihdr = be32(2);
    ihdr.extend_from_slice(&be32(1));
    ihdr.extend_from_slice(&[8, 0, 0, 0, 0]);
    png_chunk(&mut long, b"IHDR", &ihdr);
    png_chunk(&mut long, b"gAMA", &be32(45455));
    png_chunk(&mut long, b"IDAT", &deflate(&[0, 1, 2, 9, 9, 9, 9, 9, 9, 9], 6));
    png_chunk(&mut long, b"IEND", b"");
    s.one("e_extra_data.png", "", long);
    let mut no_plte = vec![137, 80, 78, 71, 13, 10, 26, 10];
    let mut ihdr = be32(2);
    ihdr.extend_from_slice(&be32(2));
    ihdr.extend_from_slice(&[8, 3, 0, 0, 0]);
    png_chunk(&mut no_plte, b"IHDR", &ihdr);
    png_chunk(&mut no_plte, b"IDAT", &deflate(&[0, 1, 2, 0, 3, 4], 6));
    png_chunk(&mut no_plte, b"IEND", b"");
    s.one("e_no_plte.png", "", no_plte);
    // A bad CRC on an ancillary chunk only loses the chunk.
    let mut d = base().before(b"gAMA", be32(45455)).build();
    let g = find_chunk(&d, b"gAMA").unwrap();
    d[g + 8 + 4] ^= 1;
    s.one("s_gama_crc.png", "", d);
    // Colorspace reference, palette too.
    s.file("cs_rgb.png", base().build());
    s.case("cs_rgb_copy", "", &["cs_rgb.png,colorspace=77"]);
    s.file("cs_pal.png", PngOpt::new(9, 7, 3, 4).build());
    s.case("cs_pal", "", &["cs_pal.png,colorspace=77"]);
    s.file("cs_rgba.png", PngOpt::new(9, 7, 6, 8).build());
    s.case("cs_rgba", "", &["cs_rgba.png,colorspace=77"]);
    s.file("cs_ga.png", PngOpt::new(9, 7, 4, 8).build());
    s.case("cs_ga", "", &["cs_ga.png,colorspace=77"]);
    s.file("cs_g.png", PngOpt::new(9, 7, 0, 8).before(b"gAMA", be32(45455)).build());
    s.case("cs_g", "", &["cs_g.png,colorspace=77"]);
    // Two images with alpha: one page group object.
    s.file("twoa.png", PngOpt::new(5, 5, 6, 8).build());
    s.file("twob.png", PngOpt::new(6, 6, 4, 8).build());
    s.case("two_alpha", "", &["twoa.png", "twob.png"]);
    // Sizes around the 8192-byte input buffer of png_read_IDAT_data, and a row longer than the PDF buffer.
    s.one("big_noise.png", "", PngOpt::new(200, 200, 2, 16).before(b"gAMA", be32(45455)).build());
    s.one("big_wide.png", "", PngOpt::new(9000, 2, 6, 8).build());
    s.one("big_wide16.png", "", PngOpt::new(5000, 2, 6, 16).build());
    s.one("big_wide_i.png", "", PngOpt::new(5000, 3, 4, 8).laced(true).build());
    let mut many = PngOpt::new(300, 300, 0, 16);
    many.idat_split = 8192;
    s.one("big_idat8192.png", "applygamma=1 gamma=2200", many.build());
}

fn find_chunk(d: &[u8], name: &[u8; 4]) -> Option<usize> {
    let mut p = 8;
    while p + 8 <= d.len() {
        let n = u32::from_be_bytes([d[p], d[p + 1], d[p + 2], d[p + 3]]) as usize;
        if &d[p + 4..p + 8] == name {
            return Some(p);
        }
        p += 12 + n;
    }
    None
}

fn fix_chunk_crc(d: &mut [u8], at: usize) {
    let n = u32::from_be_bytes([d[at], d[at + 1], d[at + 2], d[at + 3]]) as usize;
    let c = crc(&d[at + 4..at + 8 + n]);
    d[at + 8 + n..at + 12 + n].copy_from_slice(&c.to_be_bytes());
}

//------------------------------------------------------------------------
// JPEG: headers only
//------------------------------------------------------------------------

fn jseg(out: &mut Vec<u8>, marker: u8, body: &[u8]) {
    out.extend_from_slice(&[0xff, marker]);
    out.extend_from_slice(&((body.len() + 2) as u16).to_be_bytes());
    out.extend_from_slice(body);
}

fn jfif(units: u8, x: u16, y: u16) -> Vec<u8> {
    let mut b = b"JFIF\0\x01\x02".to_vec();
    b.push(units);
    b.extend_from_slice(&x.to_be_bytes());
    b.extend_from_slice(&y.to_be_bytes());
    b.extend_from_slice(&[0, 0]);
    b
}

fn jpeg(head: &[(u8, Vec<u8>)], sof: u8, bits: u8, h: u16, w: u16, comps: u8) -> Vec<u8> {
    let mut out = vec![0xff, 0xd8];
    for (m, b) in head {
        jseg(&mut out, *m, b);
    }
    let mut f = vec![bits];
    f.extend_from_slice(&h.to_be_bytes());
    f.extend_from_slice(&w.to_be_bytes());
    f.push(comps);
    for i in 0..comps {
        f.extend_from_slice(&[i + 1, 0x11, 0]);
    }
    jseg(&mut out, sof, &f);
    jseg(&mut out, 0xda, &[1, 1, 0, 0, 63, 0]);
    out.extend_from_slice(&[0x12, 0x34, 0x56, 0x78, 0xff, 0x00, 0x9a]);
    out.extend_from_slice(&[0xff, 0xd9]);
    out
}

/// A TIFF block with XResolution, YResolution and ResolutionUnit.
fn exif(big: bool, xres: (u32, u32), yres: (u32, u32), unit: u16) -> Vec<u8> {
    let w32 = |v: u32| if big { v.to_be_bytes() } else { v.to_le_bytes() };
    let w16 = |v: u16| if big { v.to_be_bytes() } else { v.to_le_bytes() };
    let mut b = b"Exif\0\0".to_vec();
    let mut t: Vec<u8> = Vec::new();
    t.extend_from_slice(if big { b"MM" } else { b"II" });
    t.extend_from_slice(&w16(42));
    t.extend_from_slice(&w32(8));
    t.extend_from_slice(&w16(3));
    let data = 8 + 2 + 3 * 12 + 4;
    for (tag, off) in [(282u16, data), (283, data + 8)] {
        t.extend_from_slice(&w16(tag));
        t.extend_from_slice(&w16(5));
        t.extend_from_slice(&w32(1));
        t.extend_from_slice(&w32(off));
    }
    t.extend_from_slice(&w16(296));
    t.extend_from_slice(&w16(3));
    t.extend_from_slice(&w32(1));
    t.extend_from_slice(&w16(unit));
    t.extend_from_slice(&[0, 0]);
    t.extend_from_slice(&[0, 0, 0, 0]);
    t.extend_from_slice(&w32(xres.0));
    t.extend_from_slice(&w32(xres.1));
    t.extend_from_slice(&w32(yres.0));
    t.extend_from_slice(&w32(yres.1));
    b.extend_from_slice(&t);
    b
}

fn gen_jpeg(s: &mut Set) {
    let adobe = |t: u8| -> Vec<u8> {
        let mut b = b"Adobe\0\x64\0\0\0\0".to_vec();
        b.push(t);
        b
    };
    let plain = |sof, comps| jpeg(&[(0xe0, jfif(1, 72, 72))], sof, 8, 7, 9, comps);
    s.one("j_gray.jpg", "", plain(0xc0, 1));
    s.one("j_rgb.jpg", "", plain(0xc0, 3));
    s.one("j_cmyk.jpg", "", plain(0xc0, 4));
    s.one("j_sof1.jpg", "", plain(0xc1, 3));
    s.one("j_prog.jpg", "", plain(0xc2, 3));
    s.one("j_sof3.jpg", "", plain(0xc3, 1));
    s.one("j_bits12.jpg", "", jpeg(&[], 0xc1, 12, 7, 9, 3));
    s.one("j_nojfif.jpg", "", jpeg(&[], 0xc0, 8, 7, 9, 3));
    s.one("j_adobe.jpg", "", jpeg(&[(0xee, adobe(1))], 0xc0, 8, 7, 9, 3));
    s.one("j_adobe_cmyk.jpg", "", jpeg(&[(0xee, adobe(2))], 0xc0, 8, 7, 9, 4));
    s.one("j_jfif_adobe.jpg", "", jpeg(&[(0xe0, jfif(0, 1, 1)), (0xee, adobe(0))], 0xc0, 8, 7, 9, 3));
    s.one("j_comment.jpg", "", jpeg(&[(0xfe, b"hello".to_vec()), (0xdb, vec![0; 65]), (0xc4, vec![0; 29])], 0xc0, 8, 7, 9, 3));
    for (units, x, y) in [(0u8, 300u16, 300u16), (1, 300, 300), (1, 300, 150), (2, 118, 118), (2, 118, 59), (1, 0, 300), (1, 300, 0), (0, 0, 0), (1, 1, 1), (3, 100, 100), (1, 65535, 65535)] {
        s.one(&format!("j_dens_{units}_{x}_{y}.jpg"), "", jpeg(&[(0xe0, jfif(units, x, y))], 0xc0, 8, 700, 900, 3));
    }
    for (big, x, y, unit) in [
        (true, (300, 1), (300, 1), 2u16),
        (false, (300, 1), (300, 1), 2),
        (true, (118, 1), (118, 1), 3),
        (false, (118, 1), (59, 1), 3),
        (true, (72, 1), (72, 1), 1),
        (false, (301, 2), (301, 2), 2),
        (true, (300, 0), (300, 0), 2),
    ] {
        let n = format!("j_exif_{}_{}_{}_{unit}.jpg", if big { "m" } else { "i" }, x.0, y.0);
        s.one(&n, "", jpeg(&[(0xe1, exif(big, x, y, unit))], 0xc0, 8, 700, 900, 3));
    }
    s.one("j_app1_other.jpg", "", jpeg(&[(0xe1, b"http://ns.adobe.com/xap/1.0/\0<x/>".to_vec())], 0xc0, 8, 7, 9, 3));
    s.one("j_app1_short.jpg", "", jpeg(&[(0xe1, b"abc".to_vec())], 0xc0, 8, 7, 9, 3));
    s.one("j_exif_junk.jpg", "", jpeg(&[(0xe1, b"Exif\0\0nonsense".to_vec())], 0xc0, 8, 7, 9, 3));
    s.file("j_cs.jpg", plain(0xc0, 3));
    s.case("j_cs", "", &["j_cs.jpg,colorspace=55"]);
    s.file("j_cmyk_cs.jpg", plain(0xc0, 4));
    s.case("j_cmyk_cs", "", &["j_cmyk_cs.jpg,colorspace=55"]);
    s.file("j_prog12.jpg", plain(0xc2, 3));
    s.case("j_prog12", "minor=2", &["j_prog12.jpg"]);
    s.case("j_prog13", "minor=3", &["j_prog12.jpg"]);
    s.file("j_v14.jpg", plain(0xc0, 3));
    s.case("j_v14", "minor=4", &["j_v14.jpg"]);
    // Failures.
    s.one("je_sof9.jpg", "", plain(0xc9, 3));
    s.one("je_sof5.jpg", "", plain(0xc5, 3));
    s.one("je_comps2.jpg", "", plain(0xc0, 2));
    let mut t = plain(0xc0, 3);
    t.truncate(30);
    s.one("je_trunc.jpg", "", t);
    let mut t = vec![0xff, 0xd8, 0xff, 0xd9, 0xff, 0xd8];
    t.extend_from_slice(&plain(0xc0, 1)[2..]);
    s.one("j_soi_eoi.jpg", "", t);
    s.one("je_nomarker.jpg", "", vec![0xff, 0xd8, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]);
    s.one("je_short.jpg", "", vec![0xff, 0xd8, 0xff, 0xe0, 0x00, 0x10, 0x00, 0x00]);
    // The extension decides when the header does not.
    s.one("je_header.jpg", "", vec![b'x'; 16]);
}

//------------------------------------------------------------------------
// JBIG2
//------------------------------------------------------------------------

#[derive(Clone)]
struct Seg {
    num: u32,
    ty: u8,
    refs: Vec<u32>,
    page: u32,
    long_page: bool,
    data: Vec<u8>,
}

fn seg(num: u32, ty: u8, refs: &[u32], page: u32, data: Vec<u8>) -> Seg {
    Seg { num, ty, refs: refs.to_vec(), page, long_page: false, data }
}

fn page_info(w: u32, h: u32, xres: u32, yres: u32, striping: u16) -> Vec<u8> {
    let mut d = Vec::new();
    d.extend_from_slice(&w.to_be_bytes());
    d.extend_from_slice(&h.to_be_bytes());
    d.extend_from_slice(&xres.to_be_bytes());
    d.extend_from_slice(&yres.to_be_bytes());
    d.push(0);
    d.extend_from_slice(&striping.to_be_bytes());
    d
}

fn seg_header(s: &Seg, out: &mut Vec<u8>) {
    out.extend_from_slice(&s.num.to_be_bytes());
    out.push(s.ty | if s.long_page { 0x40 } else { 0 });
    let n = s.refs.len();
    if n < 5 {
        out.push((n as u8) << 5);
    } else {
        out.push(7 << 5);
        let bytes = 5 + n / 8;
        out.extend_from_slice(&(n as u32).to_be_bytes()[1..]);
        out.extend_from_slice(&vec![0u8; bytes - 4]);
    }
    for &r in &s.refs {
        if s.num <= 256 {
            out.push(r as u8);
        } else if s.num <= 65536 {
            out.extend_from_slice(&(r as u16).to_be_bytes());
        } else {
            out.extend_from_slice(&r.to_be_bytes());
        }
    }
    if s.long_page {
        out.extend_from_slice(&s.page.to_be_bytes());
    } else {
        out.push(s.page as u8);
    }
    out.extend_from_slice(&(s.data.len() as u32).to_be_bytes());
}

fn jbig2_file(segs: &[Seg], sequential: bool, pages: Option<u32>) -> Vec<u8> {
    let mut out = vec![0x97, b'J', b'B', b'2', 0x0d, 0x0a, 0x1a, 0x0a];
    let mut flags = if sequential { 1 } else { 0 };
    if pages.is_none() {
        flags |= 2;
    }
    out.push(flags);
    if let Some(p) = pages {
        out.extend_from_slice(&p.to_be_bytes());
    }
    if sequential {
        for s in segs {
            seg_header(s, &mut out);
            out.extend_from_slice(&s.data);
        }
    } else {
        for s in segs {
            seg_header(s, &mut out);
        }
        for s in segs {
            out.extend_from_slice(&s.data);
        }
    }
    out
}

fn gen_jbig2(s: &mut Set) {
    let region = |n: u8| vec![n; 18 + n as usize];
    let basic = vec![
        seg(0, 48, &[], 1, page_info(20, 10, 11811, 11811, 0)),
        seg(1, 38, &[], 1, region(3)),
        seg(2, 49, &[], 1, vec![]),
        seg(3, 51, &[], 0, vec![]),
    ];
    s.one("b_seq.jb2", "", jbig2_file(&basic, true, Some(1)));
    s.one("b_seq_nopages.jb2", "", jbig2_file(&basic, true, None));
    s.one("b_rand.jb2", "", jbig2_file(&basic, false, Some(1)));
    s.one("b_noeof.jb2", "", jbig2_file(&basic[..3], true, Some(1)));
    // Page 0 segments: a global symbol dictionary used, one not used.
    let globals = vec![
        seg(0, 0, &[], 0, region(5)),
        seg(1, 0, &[], 0, region(6)),
        seg(2, 48, &[], 1, page_info(20, 10, 0, 0, 0)),
        seg(3, 6, &[0], 1, region(7)),
        seg(4, 49, &[], 1, vec![]),
        seg(5, 48, &[], 2, page_info(30, 15, 3937, 7874, 0)),
        seg(6, 6, &[1], 2, region(8)),
        seg(7, 49, &[], 2, vec![]),
        seg(8, 51, &[], 0, vec![]),
    ];
    s.file("b_globals.jb2", jbig2_file(&globals, true, Some(2)));
    s.case("b_globals_p1", "", &["b_globals.jb2"]);
    s.case("b_globals_p2", "", &["b_globals.jb2,page=2"]);
    s.case("b_globals_both", "", &["b_globals.jb2", "b_globals.jb2,page=2"]);
    s.case("b_globals_rev", "", &["b_globals.jb2,page=2", "b_globals.jb2"]);
    s.case("be_page3", "", &["b_globals.jb2,page=3"]);
    s.case("be_page0", "", &["b_globals.jb2,page=0"]);
    s.case("be_minor3", "minor=3", &["b_globals.jb2"]);
    s.file("b_globals_rand.jb2", jbig2_file(&globals, false, Some(2)));
    s.case("b_globals_rand", "", &["b_globals_rand.jb2", "b_globals_rand.jb2,page=2"]);
    // A global that refers to another global.
    let chain = vec![
        seg(0, 0, &[], 0, region(5)),
        seg(1, 0, &[0], 0, region(6)),
        seg(2, 48, &[], 1, page_info(20, 10, 0, 0, 0)),
        seg(3, 6, &[1], 1, region(7)),
        seg(4, 49, &[], 1, vec![]),
        seg(5, 51, &[], 0, vec![]),
    ];
    s.one("b_chain.jb2", "", jbig2_file(&chain, true, Some(1)));
    // Stripes: the height comes from the end-of-stripe segment.
    let stripes = vec![
        seg(0, 48, &[], 1, page_info(20, 0xffffffff, 0, 0, 0x8000 | 8)),
        seg(1, 38, &[], 1, region(3)),
        seg(2, 50, &[], 1, 7u32.to_be_bytes().to_vec()),
        seg(3, 38, &[], 1, region(4)),
        seg(4, 50, &[], 1, 15u32.to_be_bytes().to_vec()),
        seg(5, 49, &[], 1, vec![]),
        seg(6, 51, &[], 0, vec![]),
    ];
    s.one("b_stripes.jb2", "", jbig2_file(&stripes, true, Some(1)));
    s.one("b_stripes_rand.jb2", "", jbig2_file(&stripes, false, Some(1)));
    // Other sizes of the fields: page association of four bytes, a page number above 255 in it, wide segment numbers,
    // many references.
    let mut wide = vec![
        seg(300, 48, &[], 2, page_info(20, 10, 0, 0, 0)),
        seg(301, 6, &[300], 2, region(3)),
        seg(70000, 6, &[300], 2, region(4)),
        seg(70001, 49, &[], 2, vec![]),
    ];
    for w in wide.iter_mut() {
        w.long_page = true;
    }
    s.file("b_wide.jb2", jbig2_file(&wide, true, Some(1)));
    s.case("b_wide", "", &["b_wide.jb2,page=2"]);
    let many = vec![
        seg(0, 0, &[], 0, region(1)),
        seg(1, 0, &[], 0, region(2)),
        seg(2, 48, &[], 1, page_info(20, 10, 0, 0, 0)),
        seg(3, 6, &[0, 1, 0, 1, 0, 1], 1, region(3)),
        seg(4, 49, &[], 1, vec![]),
    ];
    s.one("b_many_refs.jb2", "", jbig2_file(&many, true, Some(1)));
    // A file read but not found on the page asked for still has its globals flushed.
    s.one("be_badid.jb2", "", b"\x97JB2\r\n\x1a\x0bxxxxxxxx".to_vec());
    let mut t = jbig2_file(&basic, true, Some(1));
    t.truncate(30);
    s.one("be_trunc.jb2", "", t);
    s.one("be_badseg.jb2", "", jbig2_file(&[seg(0, 63, &[], 1, vec![])], true, Some(1)));
    s.file("b_two_a.jb2", jbig2_file(&globals, true, Some(2)));
    s.file("b_two_b.jb2", jbig2_file(&globals, true, Some(2)));
    s.case("b_two_files", "", &["b_two_b.jb2", "b_two_a.jb2"]);
}

//------------------------------------------------------------------------
// PDF
//------------------------------------------------------------------------

/// A file being put together: objects in file order.
#[derive(Clone, Default)]
struct Pdf {
    version: String,
    objs: Vec<(i32, i32, Vec<u8>)>,
    trailer: String,
}

fn stream(dict: &str, data: &[u8]) -> Vec<u8> {
    let mut v = format!("<< {dict} /Length {} >>\nstream\n", data.len()).into_bytes();
    v.extend_from_slice(data);
    v.extend_from_slice(b"\nendstream");
    v
}

impl Pdf {
    fn new() -> Pdf {
        Pdf { version: "1.4".into(), objs: Vec::new(), trailer: "/Root 1 0 R".into() }
    }

    fn add(mut self, num: i32, body: &str) -> Pdf {
        self.objs.push((num, 0, body.as_bytes().to_vec()));
        self
    }

    fn raw(mut self, num: i32, body: Vec<u8>) -> Pdf {
        self.objs.push((num, 0, body));
        self
    }

    fn trailer(mut self, t: &str) -> Pdf {
        self.trailer = t.to_string();
        self
    }

    fn version(mut self, v: &str) -> Pdf {
        self.version = v.to_string();
        self
    }

    fn head(&self) -> Vec<u8> {
        format!("%PDF-{}\n%\u{e2}\u{e3}\n", self.version).into_bytes()
    }

    fn body(&self, out: &mut Vec<u8>) -> Vec<(i32, i32, usize)> {
        let mut off = Vec::new();
        for (n, g, b) in &self.objs {
            off.push((*n, *g, out.len()));
            out.extend_from_slice(format!("{n} {g} obj\n").as_bytes());
            out.extend_from_slice(b);
            out.extend_from_slice(b"\nendobj\n");
        }
        off
    }

    /// Classic xref table.
    fn classic(&self) -> Vec<u8> {
        let mut out = self.head();
        let off = self.body(&mut out);
        let start = out.len();
        write_xref_table(&mut out, &off);
        let size = off.iter().map(|o| o.0).max().unwrap_or(0) + 1;
        out.extend_from_slice(format!("trailer\n<< /Size {size} {} >>\nstartxref\n{start}\n%%EOF\n", self.trailer).as_bytes());
        out
    }

    /// An xref stream, with the objects named in `compressed` in one object stream.
    fn xref_stream(&self, compressed: &[i32], w: [usize; 3], predictor: bool) -> Vec<u8> {
        let mut out = self.head();
        let plain = Pdf {
            objs: self.objs.iter().filter(|o| !compressed.contains(&o.0)).cloned().collect(),
            ..self.clone()
        };
        let max = self.objs.iter().map(|o| o.0).max().unwrap_or(0);
        let objstm = max + 1;
        let xrefn = max + 2;
        let mut entries: Vec<(i32, u8, u64, u32)> = vec![(0, 0, 0, 65535)];
        let off = plain.body(&mut out);
        for (n, g, o) in &off {
            entries.push((*n, 1, *o as u64, *g as u32));
        }
        if !compressed.is_empty() {
            let mut header = String::new();
            let mut bodies: Vec<u8> = Vec::new();
            for (i, n) in compressed.iter().enumerate() {
                let (_, _, b) = self.objs.iter().find(|o| o.0 == *n).unwrap();
                header.push_str(&format!("{n} {} ", bodies.len()));
                bodies.extend_from_slice(b);
                bodies.push(b'\n');
                entries.push((*n, 2, objstm as u64, i as u32));
            }
            let first = header.len();
            let mut data = header.into_bytes();
            data.extend_from_slice(&bodies);
            let z = deflate(&data, 6);
            let at = out.len();
            entries.push((objstm, 1, at as u64, 0));
            out.extend_from_slice(format!("{objstm} 0 obj\n").as_bytes());
            out.extend_from_slice(&stream(&format!("/Type /ObjStm /N {} /First {first} /Filter /FlateDecode", compressed.len()), &z));
            out.extend_from_slice(b"\nendobj\n");
        }
        let at = out.len();
        entries.push((xrefn, 1, at as u64, 0));
        entries.sort();
        let rowlen = w[0] + w[1] + w[2];
        let mut raw = Vec::new();
        let mut index = String::new();
        let mut i = 0;
        while i < entries.len() {
            let start = i;
            while i + 1 < entries.len() && entries[i + 1].0 == entries[i].0 + 1 {
                i += 1;
            }
            index.push_str(&format!("{} {} ", entries[start].0, i - start + 1));
            i += 1;
        }
        for (_, t, o, g) in &entries {
            raw.extend_from_slice(&(*t as u64).to_be_bytes()[8 - w[0]..]);
            raw.extend_from_slice(&o.to_be_bytes()[8 - w[1]..]);
            raw.extend_from_slice(&(*g as u64).to_be_bytes()[8 - w[2]..]);
        }
        let (data, parms) = if predictor {
            let mut f = Vec::new();
            let mut prev = vec![0u8; rowlen];
            for row in raw.chunks(rowlen) {
                filter_row(2, row, &prev, 1, &mut f);
                prev = row.to_vec();
            }
            (f, format!("/DecodeParms << /Predictor 12 /Columns {rowlen} >>"))
        } else {
            (raw, String::new())
        };
        let z = deflate(&data, 6);
        out.extend_from_slice(format!("{xrefn} 0 obj\n").as_bytes());
        out.extend_from_slice(&stream(
            &format!(
                "/Type /XRef /Size {} /W [{} {} {}] /Index [{index}] /Filter /FlateDecode {parms} {}",
                xrefn + 1,
                w[0],
                w[1],
                w[2],
                self.trailer
            ),
            &z,
        ));
        out.extend_from_slice(format!("\nendobj\nstartxref\n{at}\n%%EOF\n").as_bytes());
        out
    }
}

fn write_xref_table(out: &mut Vec<u8>, off: &[(i32, i32, usize)]) {
    let mut sorted = off.to_vec();
    sorted.sort();
    out.extend_from_slice(b"xref\n0 1\n0000000000 65535 f \n");
    let mut i = 0;
    while i < sorted.len() {
        let start = i;
        while i + 1 < sorted.len() && sorted[i + 1].0 == sorted[i].0 + 1 {
            i += 1;
        }
        out.extend_from_slice(format!("{} {}\n", sorted[start].0, i - start + 1).as_bytes());
        for e in &sorted[start..=i] {
            out.extend_from_slice(format!("{:010} {:05} n \n", e.2, e.1).as_bytes());
        }
        i += 1;
    }
}

const CONTENT: &[u8] = b"q 1 0 0 1 72 72 cm 0 0 100 100 re f Q\n";

/// One page, with what goes in the page dictionary.
fn doc(page_extra: &str) -> Pdf {
    Pdf::new()
        .add(1, "<< /Type /Catalog /Pages 2 0 R >>")
        .add(2, "<< /Type /Pages /Kids [3 0 R] /Count 1 >>")
        .add(3, &format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 100] /Contents 4 0 R {page_extra} >>"))
        .raw(4, stream("", CONTENT))
}

/// A Type 1 font file, as far as xpdf reads it.
fn fake_type1(name: &str, enc: &str) -> Vec<u8> {
    format!(
        "%!PS-AdobeFont-1.0: {name} 1.0\n11 dict begin\n/FontName /{name} def\n{enc}\nreadonly def\ncurrentfile eexec\nxxxx\n"
    )
    .into_bytes()
}

/// A CFF font with glyphs .notdef, A, B and a format 0 encoding.
fn fake_cff(enc_codes: &[u8]) -> Vec<u8> {
    let mut f = vec![1, 0, 4, 1]; // header
    let index = |items: &[&[u8]]| -> Vec<u8> {
        let mut o = (items.len() as u16).to_be_bytes().to_vec();
        if items.is_empty() {
            return o;
        }
        o.push(1);
        let mut at = 1u8;
        o.push(at);
        for i in items {
            at += i.len() as u8;
            o.push(at);
        }
        for i in items {
            o.extend_from_slice(i);
        }
        o
    };
    f.extend_from_slice(&index(&[b"FAKE"]));
    // Top DICT with five-byte integers, so its size does not depend on the offsets.
    let top_len = 4 * 6 + 1 + 5 + 1;
    let name_idx_end = f.len();
    let top_index_len = 2 + 1 + 2 + top_len;
    let strings = index(&[]);
    let gsubrs = index(&[]);
    let base = name_idx_end + top_index_len + strings.len() + gsubrs.len();
    let charset = vec![0u8, 0, 34, 0, 35]; // format 0: SID 34 is A, 35 is B
    let mut encoding = vec![0u8, enc_codes.len() as u8];
    encoding.extend_from_slice(enc_codes);
    let charstrings = index(&[&[14], &[14], &[14]]);
    let private = vec![139u8, 20]; // defaultWidthX 0
    let charset_off = base;
    let enc_off = charset_off + charset.len();
    let cs_off = enc_off + encoding.len();
    let priv_off = cs_off + charstrings.len();
    let mut top = Vec::new();
    let int5 = |v: usize, o: &mut Vec<u8>| {
        o.push(29);
        o.extend_from_slice(&(v as u32).to_be_bytes());
    };
    int5(charset_off, &mut top);
    top.push(15);
    int5(enc_off, &mut top);
    top.push(16);
    int5(cs_off, &mut top);
    top.push(17);
    int5(private.len(), &mut top);
    int5(priv_off, &mut top);
    top.push(18);
    assert_eq!(top.len(), 5 * 5 + 4);
    let top_len2 = top.len();
    assert!(top_len2 <= top_len);
    while top.len() < top_len {
        top.insert(0, 139 + 0);
        top.insert(1, 14); // a padding operator with an operand that nobody reads
        if top.len() > top_len {
            top.truncate(top_len);
        }
    }
    f.extend_from_slice(&index(&[&top]));
    f.extend_from_slice(&strings);
    f.extend_from_slice(&gsubrs);
    f.extend_from_slice(&charset);
    f.extend_from_slice(&encoding);
    f.extend_from_slice(&charstrings);
    f.extend_from_slice(&private);
    f
}

const ENC_AB: &str = "/Encoding 256 array\n0 1 255 {1 index exch /.notdef put} for\ndup 65 /A put\ndup 66 /B put";

/// A page using font `fname`, whose objects are 5 (font), 6 (descriptor), 7 (font file).
fn font_doc(fname: &str, font_keys: &str, desc_keys: &str, file: Vec<u8>) -> Pdf {
    Pdf::new()
        .add(1, "<< /Type /Catalog /Pages 2 0 R >>")
        .add(2, "<< /Type /Pages /Kids [3 0 R] /Count 1 >>")
        .add(3, "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 100] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> /ProcSet [/PDF /Text] >> >>")
        .raw(4, stream("", b"BT /F1 12 Tf (A) Tj ET\n"))
        .add(5, &format!("<< /Type /Font /Subtype /Type1 /BaseFont /{fname} /FirstChar 65 /LastChar 66 /Widths [1 2] /FontDescriptor 6 0 R {font_keys} >>"))
        .add(6, &format!("<< /Type /FontDescriptor /FontName /{fname} /Flags 4 /StemV 80.4 /FontFile 7 0 R {desc_keys} >>"))
        .raw(7, stream("/Length1 10 /Length2 10 /Length3 0", &file))
}

fn gen_pdf(s: &mut Set) {
    s.fontmap.push(("CMR10".into(), "cmr10.pfb".into()));
    s.fontmap.push(("CMBX12".into(), "cmbx12.pfb".into()));
    let info = "<< /Producer (x) /Title (T\\(1\\)) >>";
    // --- structure of the file
    let simple = doc("/Resources << /ProcSet [/PDF] >>");
    s.one("d_classic.pdf", "", simple.classic());
    s.one("d_v15.pdf", "", simple.clone().version("1.5").classic());
    s.one("d_v17.pdf", "", simple.clone().version("1.7").classic());
    s.file("d_v17e.pdf", simple.clone().version("1.7").classic());
    s.case("d_v17_err1", "errorlevel=1", &["d_v17e.pdf"]);
    s.case("d_v17_errm1", "errorlevel=-1", &["d_v17e.pdf"]);
    s.case("d_v14_min4", "minor=4", &["d_v15.pdf"]);
    s.one("d_v2.pdf", "", simple.clone().version("2.0").classic());
    s.one("d_xstream.pdf", "", simple.xref_stream(&[], [1, 3, 1], false));
    s.one("d_xstream_pred.pdf", "", simple.xref_stream(&[], [1, 2, 0], true));
    s.one("d_objstm.pdf", "", simple.xref_stream(&[1, 2, 3], [1, 4, 2], false));
    s.one("d_objstm_pred.pdf", "", simple.xref_stream(&[1, 2, 3], [1, 2, 1], true));
    // Garbage before the header moves every offset.
    let mut g = b"garbage line\n".to_vec();
    g.extend_from_slice(&simple.classic());
    s.file("d_garbage.pdf", g);
    s.case("d_garbage.pdf", "", &["d_garbage.pdf"]);
    // An incremental update: object 3 replaced, a Prev chain.
    {
        let base = doc("/Resources << /ProcSet [/PDF] >>");
        let mut out = base.classic();
        let prev = last_startxref(&out);
        let at = out.len();
        out.extend_from_slice(b"3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 150] /Contents 4 0 R /Rotate 90 >>\nendobj\n");
        let xr = out.len();
        out.extend_from_slice(format!("xref\n3 1\n{at:010} 00000 n \ntrailer\n<< /Size 5 /Root 1 0 R /Prev {prev} >>\nstartxref\n{xr}\n%%EOF\n").as_bytes());
        s.one("d_update.pdf", "", out);
    }
    // A hybrid file: objects only in an xref stream, reached through XRefStm.
    {
        let hidden = 5;
        let mut out = format!("%PDF-1.5\n").into_bytes();
        let base = doc("/Resources 5 0 R");
        let off = base.body(&mut out);
        let body = b"<< /ProcSet [/PDF /Text] >>\n";
        let hdr = b"5 0 ";
        let mut data = hdr.to_vec();
        data.extend_from_slice(body);
        let z = deflate(&data, 6);
        let os_at = out.len();
        out.extend_from_slice(b"6 0 obj\n");
        out.extend_from_slice(&stream(&format!("/Type /ObjStm /N 1 /First {}", hdr.len()), &data));
        out.extend_from_slice(b"\nendobj\n");
        let _ = z;
        let xs_at = out.len();
        let raw = [2u8, 0, 6, 0, 1, (os_at >> 8) as u8, os_at as u8, 0];
        out.extend_from_slice(b"7 0 obj\n");
        out.extend_from_slice(&stream("/Type /XRef /W [1 2 1] /Index [5 2] /Size 8", &raw));
        out.extend_from_slice(b"\nendobj\n");
        let xr = out.len();
        write_xref_table(&mut out, &off);
        out.extend_from_slice(format!("trailer\n<< /Size 8 /Root 1 0 R /XRefStm {xs_at} >>\nstartxref\n{xr}\n%%EOF\n").as_bytes());
        let _ = hidden;
        s.one("d_hybrid.pdf", "", out);
    }
    // Damage: repaired by scanning.
    {
        let good = doc("/Resources << /ProcSet [/PDF] >>").classic();
        let at = good.windows(9).rposition(|w| w == b"startxref").unwrap();
        let mut d = good[..at].to_vec();
        d.extend_from_slice(b"startxref\n99999\n%%EOF\n");
        s.one("r_bad_startxref.pdf", "", d);
        let mut d = good[..at].to_vec();
        d.extend_from_slice(b"%%EOF\n");
        s.one("r_no_startxref.pdf", "", d);
        let xr = good.windows(4).rposition(|w| w == b"xref").unwrap();
        let tail_at = good.windows(7).rposition(|w| w == b"trailer").unwrap();
        let mut d = good[..xr].to_vec();
        d.extend_from_slice(&good[tail_at..]);
        s.one("r_no_table.pdf", "", d);
        // Offsets all wrong.
        let text = String::from_utf8_lossy(&good).replace("0000000", "0000001");
        let _ = text;
        let mut d = good.clone();
        for i in xr..tail_at {
            if d[i..].starts_with(b"00000") && i + 10 < tail_at {
                d[i + 4] = b'1';
                break;
            }
        }
        s.one("r_bad_offset.pdf", "", d);
        // Damage plus an object stream and an xref stream: the scan finds both.
        let os = doc("/Resources << /ProcSet [/PDF] >>").xref_stream(&[1, 2, 3], [1, 3, 1], false);
        let at = os.windows(9).rposition(|w| w == b"startxref").unwrap();
        let mut d = os[..at].to_vec();
        d.extend_from_slice(b"%%EOF\n");
        s.one("r_objstm.pdf", "", d);
        s.one("re_empty.pdf", "", b"%PDF-1.4\nnothing here at all\n".to_vec());
        s.one("re_noroot.pdf", "", b"%PDF-1.4\n1 0 obj\n<< /A 1 >>\nendobj\ntrailer\n<< /Size 2 >>\n".to_vec());
    }
    // --- the page tree
    let inherit = Pdf::new()
        .add(1, "<< /Type /Catalog /Pages 2 0 R >>")
        .add(2, "<< /Type /Pages /Kids [3 0 R 8 0 R] /Count 3 /MediaBox [0 0 400 300] /CropBox [10 10 390 290] /Rotate 180 /Resources << /ProcSet [/PDF] /ExtGState << /G1 9 0 R >> >> >>")
        .add(3, "<< /Type /Pages /Parent 2 0 R /Kids [4 0 R 5 0 R] /Count 2 /MediaBox [0 0 200 100] /Resources << /XObject << /X1 10 0 R >> >> >>")
        .add(4, "<< /Type /Page /Parent 3 0 R /Contents 6 0 R >>")
        .add(5, "<< /Type /Page /Parent 3 0 R /Contents 6 0 R /Rotate 270 /Resources << /ExtGState << /G2 9 0 R >> /ProcSet [/Text] >> >>")
        .raw(6, stream("", CONTENT))
        .add(8, "<< /Type /Page /Parent 2 0 R /Contents 6 0 R /CropBox [20 20 100 100] /BleedBox [21 21 99 99] /TrimBox [22 22 98 98] /ArtBox [23 23 97 97] >>")
        .add(9, "<< /Type /ExtGState /CA 0.5 >>")
        .raw(10, stream("/Type /XObject /Subtype /Form /BBox [0 0 1 1]", b"0 0 1 1 re f\n"));
    s.file("t_inherit.pdf", inherit.classic());
    for p in 1..=3 {
        for b in 1..=5 {
            s.case(&format!("t_inherit_p{p}_b{b}"), "", &[&format!("t_inherit.pdf,page={p},box={b}")]);
        }
    }
    s.case("te_page4", "", &["t_inherit.pdf,page=4"]);
    s.case("te_page0", "", &["t_inherit.pdf,page=0"]);
    s.case("t_two_pages", "", &["t_inherit.pdf,page=1", "t_inherit.pdf,page=3"]);
    s.case("t_same_page_twice", "", &["t_inherit.pdf,page=1", "t_inherit.pdf,page=1"]);
    // Count 0, and one absurdly large: the tree is counted.  And a /Page as /Pages.
    let count0 = Pdf::new()
        .add(1, "<< /Pages 2 0 R >>")
        .add(2, "<< /Kids [3 0 R 3 0 R] /Count 0 >>")
        .add(3, "<< /Type /Page /MediaBox [0 0 50 60] >>");
    s.one("t_count0.pdf", "", count0.classic());
    let bare = Pdf::new().add(1, "<< /Pages 2 0 R >>").add(2, "<< /Type /Page /MediaBox [5 6 50 60] >>");
    s.one("t_page_as_pages.pdf", "", bare.classic());
    let badtree = Pdf::new().add(1, "<< /Pages 2 0 R >>").add(2, "<< /Kids [2 0 R] /Count 1 >>");
    s.one("t_loop.pdf", "", badtree.classic());
    let nopages = Pdf::new().add(1, "<< /Pages 7 >>");
    s.one("te_pages_int.pdf", "", nopages.classic());
    // No MediaBox; boxes outside the media box; reversed; odd lengths.
    s.one("t_nomedia.pdf", "", Pdf::new().add(1, "<< /Pages 2 0 R >>").add(2, "<< /Type /Pages /Kids [3 0 R] /Count 1 >>").add(3, "<< /Type /Page >>").classic());
    s.one("t_box_clip.pdf", "", doc("/CropBox [-50 -50 500 500]").classic());
    s.one("t_box_reversed.pdf", "", doc("/CropBox [100 90 20 10]").classic());
    s.one("t_box_short.pdf", "", doc("/CropBox [1 2 3]").classic());
    s.one("t_box_reals.pdf", "", doc("/CropBox [0.5 1.25 199.9375 99.123456789]").classic());
    s.one("t_box_ref.pdf", "", doc("/CropBox [5 0 R 6 7 8]").add(5, "1.5").classic());
    s.one("t_box_huge.pdf", "", doc("/MediaBox [0 0 2000000000 3000000000]").classic());
    // Rotation.
    for r in [90, 180, 270, 360, -90, 450, 45, 91] {
        s.one(&format!("t_rot{r}.pdf"), "", doc(&format!("/Rotate {r} /CropBox [10 20 110 70]")).classic());
    }
    s.one("t_rot_ref.pdf", "", doc("/Rotate 5 0 R").add(5, "90").classic());
    s.one("t_rot_real.pdf", "", doc("/Rotate 90.0").classic());
    // Annotations and contents of the wrong type make the page an empty default one.
    s.one("t_bad_annots.pdf", "", doc("/Annots 3").classic());
    // --- named destinations
    let dests = doc("/Resources << >>")
        .add(5, "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 77 88] >>")
        .add(6, "<< /p1 [3 0 R /Fit] /p2 << /D [5 0 R /XYZ 0 0 null] >> /num [0 /Fit] /badkind [3 0 R /Nope] /notpage [4 0 R /Fit] /fitr [3 0 R /FitR 1 2 3 4] >>");
    let mut dests = dests;
    dests.objs[0].2 = b"<< /Type /Catalog /Pages 2 0 R /Dests 6 0 R >>".to_vec();
    dests.objs[1].2 = b"<< /Type /Pages /Kids [3 0 R 5 0 R] /Count 2 >>".to_vec();
    s.file("n_dests.pdf", dests.classic());
    s.case("n_p1", "", &["n_dests.pdf,named=p1"]);
    s.case("n_p2", "", &["n_dests.pdf,named=p2"]);
    s.case("ne_missing", "", &["n_dests.pdf,named=zz"]);
    s.case("ne_num", "", &["n_dests.pdf,named=num"]);
    s.case("ne_badkind", "", &["n_dests.pdf,named=badkind"]);
    s.case("ne_notpage", "", &["n_dests.pdf,named=notpage"]);
    s.case("n_fitr", "", &["n_dests.pdf,named=fitr"]);
    let tree = doc("/Resources << >>")
        .add(5, "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 77 88] >>")
        .add(6, "<< /Kids [7 0 R 8 0 R] >>")
        .add(7, "<< /Limits [(a) (c)] /Names [(a) [3 0 R /Fit] (c) [5 0 R /Fit]] >>")
        .add(8, "<< /Limits [(m) (z)] /Names [(m) [5 0 R /Fit] (z) [3 0 R /Fit]] >>");
    let mut tree = tree;
    tree.objs[0].2 = b"<< /Type /Catalog /Pages 2 0 R /Names << /Dests 6 0 R >> >>".to_vec();
    tree.objs[1].2 = b"<< /Type /Pages /Kids [3 0 R 5 0 R] /Count 2 >>".to_vec();
    s.file("n_tree.pdf", tree.classic());
    s.case("n_tree_c", "", &["n_tree.pdf,named=c"]);
    s.case("n_tree_m", "", &["n_tree.pdf,named=m"]);
    s.case("ne_tree_b", "", &["n_tree.pdf,named=b"]);
    // --- what is copied, and how it is printed
    let values = doc(
        "/Resources << /ExtGState << /A 5 0 R /B << /Direct true /F false /N null /I -12 /R 3.14159 /R2 -0.5 /R3 .25 /R4 100.0 /R5 0.0000001 /R6 123456.789012 /R7 1e3 >> >> /Pattern 6 0 R /ColorSpace << /C1 [/ICCBased 7 0 R] /C2 [/Indexed /DeviceRGB 1 <000102030405>] /C3 [/Name1/Name2 3 /Name3] >> /Shading << >> >>",
    )
    .add(5, "<< /Names [/A#20B /C#23 /D.e-f_g+h /\u{7f} /(x /#e4#ff] /Str [(plain) (par\\(en\\)s) (back\\\\slash) (nl\\n) (oct\\101) (high\\344) (\\000nul) () <48656C6C6F> <00FF> <4>] >>")
    .add(6, "[8 0 R 9 0 R 8 0 R]")
    .raw(7, stream("/N 3 /Alternate /DeviceRGB", b"profile bytes"))
    .add(8, "<< /Self 8 0 R /Other 9 0 R /Missing 99 0 R /Gen 9 1 R >>")
    .add(9, "(indirect string)");
    s.one("v_values.pdf", "", values.classic());
    let order = doc("/Resources << /Z << /A 5 0 R >> /Y << /B 6 0 R >> /X << /C 7 0 R >> /Subtype /Foo >> /LastModified (D:2000) /Metadata 8 0 R /PieceInfo << /App 9 0 R >> /SeparationInfo 10 0 R")
        .add(5, "<< /Next 6 0 R >>")
        .add(6, "<< /Next 7 0 R >>")
        .add(7, "<< /Back 5 0 R >>")
        .raw(8, stream("/Type /Metadata /Subtype /XML", b"<x/>"))
        .add(9, "<< /Private 5 0 R >>")
        .add(10, "<< /Pages [3 0 R] >>");
    s.one("v_order.pdf", "", order.classic());
    let mut withinfo = doc("/Resources << >>").trailer("/Root 1 0 R /Info 5 0 R").add(5, info);
    s.one("v_info.pdf", "", withinfo.classic());
    s.file("v_info2.pdf", withinfo.classic());
    s.case("v_suppress1", "suppressinfo=1", &["v_info2.pdf"]);
    s.case("v_suppress2", "suppressinfo=2", &["v_info2.pdf"]);
    s.case("v_suppress4", "suppressinfo=4", &["v_info2.pdf"]);
    s.case("v_suppress8", "suppressinfo=8", &["v_info2.pdf"]);
    s.case("v_suppress15", "suppressinfo=15", &["v_info2.pdf"]);
    s.case("v_underscore", "underscore=1", &["v_info2.pdf"]);
    withinfo.trailer = format!("/Root 1 0 R /Info {info}");
    s.one("v_info_direct.pdf", "", withinfo.classic());
    s.one("v_meta_direct.pdf", "", doc("/Metadata << /A 1 >>").classic());
    s.one("v_no_resources.pdf", "", doc("").classic());
    s.one("v_res_ref.pdf", "", doc("/Resources 5 0 R").add(5, "<< /ProcSet 6 0 R /Font 7 0 R /ExtGState 8 0 R /Odd 9 0 R /Subtype 10 0 R >>").add(6, "[/PDF]").add(7, "<< >>").add(8, "<< /A 1 >>").add(9, "12").add(10, "/Named").classic());
    s.one("v_res_subtype_bad.pdf", "", doc("/Resources << /Subtype 3 >>").classic());
    s.one("ve_procset.pdf", "", doc("/Resources << /ProcSet /PDF >>").classic());
    s.one("ve_procset2.pdf", "", doc("/Resources << /ProcSet [/PDF 3] >>").classic());
    s.one("ve_font.pdf", "", doc("/Resources << /Font 3 >>").classic());
    s.one("ve_font2.pdf", "", doc("/Resources << /Font << /F 3 >> >>").classic());
    s.one("v_font_direct.pdf", "", doc("/Resources << /Font << /F << /Type /Font /Subtype /Type3 >> >> >>").classic());
    s.one("ve_ref0.pdf", "", doc("/Resources << /A << /B 0 0 R >> >>").classic());
    s.one("v_dup_keys.pdf", "", doc("/Resources << /A << /X 1 /Y 2 /X 3 >> /A << /Z 4 >> >>").classic());
    s.one("v_comments.pdf", "", doc("/Resources << % comment\n /A << /X % more\n 1 >> >>").classic());
    s.one("v_cmd_in_array.pdf", "", doc("/Resources << /A << /X [1 foo 2] >> >>").classic());
    // A long number, and a real long enough to lose digits.
    s.one("v_bignum.pdf", "", doc("/Resources << /A << /B 2147483648 /C 4294967297 /D 99999999999999999999.5 /E 0.123456789 /F -0.0000004 >> >>").classic());
    // --- the page group
    s.one("g_group.pdf", "", doc("/Group << /S /Transparency /CS /DeviceRGB >> /Resources << >>").classic());
    s.file("g_group2.pdf", doc("/Group << /S /Transparency /CS /DeviceRGB >> /Resources << >>").classic());
    s.case("g_suppress", "suppressgroupwarning=1", &["g_group2.pdf"]);
    s.one("g_group_ref.pdf", "", doc("/Group 5 0 R /Resources << >>").add(5, "<< /S /Transparency >>").classic());
    s.one("g_group_notdict.pdf", "", doc("/Group 7 /Resources << >>").classic());
    // Written at shipout, when the page takes the group: as a separate object, once.
    s.case("gs_deferred", "deferred=1 next=6 group=5", &["g_group2.pdf"]);
    s.file("g_group3.pdf", doc("/Group 5 0 R /Resources << /A 6 0 R >>").add(5, "<< /S /Transparency /Ref 6 0 R >>").add(6, "<< /Z 1 >>").classic());
    s.case("gs_deferred_ref", "deferred=1 next=7 group=5", &["g_group3.pdf"]);
    s.file("g_nogroup.pdf", doc("/Resources << /A 5 0 R >>").add(5, "<< /Z 1 >>").classic());
    s.case("gs_deferred_none", "deferred=1 next=6", &["g_nogroup.pdf"]);
    s.case("gs_deferred_png", "deferred=1 next=7", &["twoa.png"]);
    s.case("gs_deferred_jpg", "deferred=1 next=6", &["j_rgb.jpg"]);
    // --- contents
    let two = |a: Vec<u8>, b: Vec<u8>| doc("/Resources << >>").raw(5, a).raw(6, b);
    let mut c = two(stream("", b"q\n"), stream("", b"Q"));
    c.objs[2].2 = b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 9 9] /Contents [5 0 R 6 0 R 5 0 R] >>".to_vec();
    s.one("c_array.pdf", "", c.classic());
    let flate = stream("/Filter /FlateDecode", &deflate(b"0 0 1 1 re f\n", 6));
    let mut c = two(flate.clone(), stream("/Filter [/ASCIIHexDecode /FlateDecode]", &hex_of(&deflate(b"1 1 2 2 re f", 6))));
    c.objs[2].2 = b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 9 9] /Contents [5 0 R 6 0 R] >>".to_vec();
    s.one("c_array_filters.pdf", "", c.classic());
    let mut c = two(stream("/Filter /ASCII85Decode", b"0Q(4Q+D>/+~>"), stream("/Filter /RunLengthDecode", &[2, b'a', b'b', b'c', 254, b'x', 128]));
    c.objs[2].2 = b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 9 9] /Contents [5 0 R 6 0 R] >>".to_vec();
    s.one("c_array_ascii.pdf", "", c.classic());
    let mut c = two(stream("/Filter /LZWDecode", &[0x80, 0x0B, 0x60, 0x50, 0x22, 0x0C, 0x0C, 0x85, 0x01]), stream("/Filter /FlateDecode /DecodeParms << /Predictor 12 /Columns 4 >>", &deflate(&[2, b'a', b'b', b'c', b'd', 2, 1, 1, 1, 1], 6)));
    c.objs[2].2 = b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 9 9] /Contents [5 0 R 6 0 R] >>".to_vec();
    s.one("c_array_lzw_pred.pdf", "", c.classic());
    let mut c = two(stream("", b""), stream("", b"x\n"));
    c.objs[2].2 = b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 9 9] /Contents [5 0 R 6 0 R 5 0 R] >>".to_vec();
    s.one("c_array_empty.pdf", "", c.classic());
    let mut c = doc("");
    c.objs[2].2 = b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 9 9] /Contents [] >>".to_vec();
    s.one("c_array_none.pdf", "", c.classic());
    s.one("c_flate_raw.pdf", "", {
        let mut c = doc("/Resources << >>");
        c.objs[3].2 = flate.clone();
        c.classic()
    });
    s.one("c_filter_parms.pdf", "", {
        let mut c = doc("/Resources << >>");
        c.objs[3].2 = stream("/Filter [/FlateDecode] /DecodeParms [<< /Predictor 1 >>]", &deflate(b"x", 6));
        c.classic()
    });
    s.one("c_length_ref.pdf", "", {
        let mut c = doc("/Resources << >>").add(5, &CONTENT.len().to_string());
        let mut body = b"<< /Length 5 0 R >>\nstream\n".to_vec();
        body.extend_from_slice(CONTENT);
        body.extend_from_slice(b"\nendstream");
        c.objs[3].2 = body;
        c.classic()
    });
    s.one("c_length_wrong.pdf", "", {
        let mut c = doc("/Resources << >>");
        let mut body = b"<< /Length 5 >>\nstream\n".to_vec();
        body.extend_from_slice(CONTENT);
        body.extend_from_slice(b"\nendstream");
        c.objs[3].2 = body;
        c.classic()
    });
    s.one("c_length_missing.pdf", "", {
        let mut c = doc("/Resources << >>").add(5, "<< /A 6 0 R >>");
        let mut body = b"<< /Foo 1 >>\nstream\nabc\nendstream".to_vec();
        c.objs[3].2 = std::mem::take(&mut body);
        c.classic()
    });
    s.one("ce_external.pdf", "", {
        let mut c = doc("/Resources << >>");
        c.objs[3].2 = stream("/F (other.dat)", b"");
        c.classic()
    });
    s.one("ce_contents_direct.pdf", "", {
        let mut c = doc("/Resources << >>");
        c.objs[2].2 = b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 9 9] /Contents (x) >>".to_vec();
        c.classic()
    });
    // A stream among the copied objects: raw, whatever its filter, and with its /Length indirect.
    s.one("o_streams.pdf", "", doc("/Resources << /XObject << /I 5 0 R /J 6 0 R >> >>")
        .raw(5, stream("/Type /XObject /Subtype /Image /Width 1 /Height 1 /Filter /DCTDecode /Extra 7 0 R", b"\xff\xd8not a jpeg"))
        .add(7, "<< /Q 8 0 R >>")
        .raw(8, stream("/Filter /FlateDecode", &deflate(b"zzz", 6)))
        .raw(6, {
            let mut b = b"<< /Length 9 0 R /Filter /Unknown >>\nstream\nraw raw".to_vec();
            b.extend_from_slice(b"\nendstream");
            b
        })
        .add(9, "7")
        .classic());
    // --- fonts
    let t1 = |n: &str| fake_type1(n, ENC_AB);
    s.one("f_t1.pdf", "", font_doc("CMR10", "", "/CharSet (/A/B)", t1("CMR10")).classic());
    s.one("f_t1_tag.pdf", "", font_doc("ABCDEF+CMR10", "", "/CharSet (/A/B)", t1("CMR10")).classic());
    s.one("f_nocharset.pdf", "", font_doc("CMR10", "", "", t1("CMR10")).classic());
    s.one("f_unknown_font.pdf", "", font_doc("NotInTheMap", "", "", t1("NotInTheMap")).classic());
    s.file("f_copy.pdf", font_doc("CMR10", "", "", t1("CMR10")).classic());
    s.case("f_copyfonts", "copyfonts=1", &["f_copy.pdf"]);
    s.one("f_encoding_name.pdf", "", font_doc("CMR10", "/Encoding /WinAnsiEncoding", "", t1("CMR10")).classic());
    s.one("f_encoding_mac.pdf", "", font_doc("CMR10", "/Encoding /MacRomanEncoding", "", t1("CMR10")).classic());
    s.one("f_encoding_expert.pdf", "", font_doc("CMR10", "/Encoding /MacExpertEncoding", "", t1("CMR10")).classic());
    s.one("f_encoding_diff.pdf", "", font_doc("CMR10", "/Encoding << /Differences [65 /B /A 70 /X /.notdef 300 /Y] >>", "", t1("CMR10")).classic());
    s.one("f_encoding_base.pdf", "", font_doc("CMR10", "/Encoding << /BaseEncoding /WinAnsiEncoding /Differences [1 /foo] >>", "", t1("CMR10")).classic());
    s.one("f_enc_standard.pdf", "", font_doc("CMR10", "", "", fake_type1("CMR10", "/Encoding StandardEncoding def")).classic());
    s.one("f_enc_none.pdf", "", font_doc("CMR10", "", "", fake_type1("CMR10", "")).classic());
    s.one("f_enc_many.pdf", "", font_doc("CMR10", "", "", fake_type1("CMR10", "/Encoding 256 array\ndup 8#101 /A put dup 66 /B put\ndup 67 /C\nput\n  dup 68 /D put\nreadonly def")).classic());
    let mut pfb = vec![0x80, 1];
    let f = t1("CMR10");
    pfb.extend_from_slice(&(f.len() as u32).to_le_bytes());
    pfb.extend_from_slice(&f);
    pfb.extend_from_slice(&[0x80, 3]);
    s.one("f_pfb.pdf", "", font_doc("CMR10", "", "", pfb).classic());
    s.one("f_garbage_file.pdf", "", font_doc("CMR10", "", "", b"not a font".to_vec()).classic());
    s.one("f_type1c.pdf", "", {
        let mut d = font_doc("CMR10", "", "", Vec::new());
        d.objs[5].2 = b"<< /Type /FontDescriptor /FontName /CMR10 /Flags 4 /StemV 69 /FontFile3 7 0 R >>".to_vec();
        d.objs[6].2 = stream("/Subtype /Type1C", &fake_cff(&[65, 66]));
        d.classic()
    });
    s.one("f_type1c_diff.pdf", "", {
        let mut d = font_doc("CMR10", "/Encoding << /Differences [90 /Z] >>", "", Vec::new());
        d.objs[5].2 = b"<< /Type /FontDescriptor /FontName /CMR10 /Flags 4 /StemV 69 /FontFile3 7 0 R >>".to_vec();
        d.objs[6].2 = stream("/Subtype /Type1C", &fake_cff(&[97, 98]));
        d.classic()
    });
    s.one("f_opentype_ff3.pdf", "", {
        let mut d = font_doc("CMR10", "", "", Vec::new());
        d.objs[5].2 = b"<< /Type /FontDescriptor /FontName /CMR10 /Flags 4 /StemV 69 /FontFile3 7 0 R >>".to_vec();
        d.objs[6].2 = stream("/Subtype /OpenType", b"OTTO");
        d.classic()
    });
    s.one("f_flate_file.pdf", "", {
        let mut d = font_doc("CMR10", "", "", Vec::new());
        d.objs[6].2 = stream("/Filter /FlateDecode", &deflate(&t1("CMR10"), 6));
        d.classic()
    });
    s.one("f_stemv_missing.pdf", "", {
        let mut d = font_doc("CMR10", "", "", t1("CMR10"));
        d.objs[5].2 = b"<< /Type /FontDescriptor /FontName /CMR10 /Flags 4 /StemV 100 /FontFile 7 0 R >>".to_vec();
        d.classic()
    });
    // Two fonts with one file: one descriptor object between them.
    s.one("f_two.pdf", "", {
        let mut d = font_doc("CMR10", "", "", t1("CMR10"));
        d.objs[2].2 = b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 9 9] /Contents 4 0 R /Resources << /Font << /F1 5 0 R /F2 5 0 R /F3 8 0 R /F4 11 0 R >> /ProcSet [/PDF /Text] >> >>".to_vec();
        d.add(8, "<< /Type /Font /Subtype /Type1 /BaseFont /CMBX12 /FontDescriptor 9 0 R /Encoding << /Differences [1 /one] >> >>")
            .add(9, "<< /Type /FontDescriptor /FontName /CMBX12 /StemV 99 /FontFile 10 0 R >>")
            .raw(10, stream("", &fake_type1("CMBX12", ENC_AB)))
            .add(11, "<< /Type /Font /Subtype /TrueType /BaseFont /Arial >>")
            .classic()
    });
    s.file("f_shared.pdf", font_doc("CMR10", "", "", t1("CMR10")).classic());
    s.case("f_shared_twice_open", "", &["f_shared.pdf", "f_shared.pdf"]);
    // The font's own descriptor reachable from another copied object.
    s.one("f_desc_referenced.pdf", "", {
        let mut d = font_doc("CMR10", "", "", t1("CMR10"));
        d.objs[2].2 = b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 9 9] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> /ExtGState << /G << /D 6 0 R /FF 5 0 R >> >> >> >>".to_vec();
        d.classic()
    });
    s.one("f_desc_before_font.pdf", "", {
        let mut d = font_doc("CMR10", "", "", t1("CMR10"));
        d.objs[2].2 = b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 9 9] /Contents 4 0 R /Metadata 6 0 R /Resources << /Font << /F1 5 0 R >> >> >>".to_vec();
        d.classic()
    });
}

fn hex_of(d: &[u8]) -> Vec<u8> {
    let mut o = Vec::new();
    for b in d {
        o.extend_from_slice(format!("{b:02x}").as_bytes());
    }
    o.push(b'>');
    o
}

fn last_startxref(d: &[u8]) -> usize {
    let at = d.windows(9).rposition(|w| w == b"startxref").unwrap() + 9;
    let s = String::from_utf8_lossy(&d[at..]);
    s.split_whitespace().next().unwrap().parse().unwrap()
}

/// Everything.
pub fn all() -> Set {
    let mut s = Set::default();
    gen_png(&mut s);
    gen_jpeg(&mut s);
    gen_jbig2(&mut s);
    gen_pdf(&mut s);
    s
}
