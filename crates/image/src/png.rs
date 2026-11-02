//! writepng.c, and as much of libpng 1.6.59 as the oracle's `read_png_info` and `write_png` reach.
//!
//! The oracle links the system libpng, so its reader is part of the behaviour: which chunks count as present decides
//! between copying IDAT and re-encoding, its gamma tables decide the bytes when `\pdfimageapplygamma=1`, and its errors
//! are the failures.  This file follows pnggroup/libpng tag v1.6.59 with the `pnglibconf.h` Arch ships, which differs
//! from the prebuilt one in `PNG_ZLIB_VERNUM` only.  Function names are libpng's where the code is a translation.
//!
//! Every `png_error` ends in `pdftex_fail("libpng: internal error")`, through the `setjmp` in `read_png_info`.
//!
//! Differences from the C, none of which a reader of the PDF can see:
//! - Rows with fewer than 8 bits to a pixel and spare bits at the end get zeros there.  `png_combine_row` keeps those
//!   bits of the buffer it copies into, and `write_png`'s buffers come from `malloc`, so in the oracle they are whatever
//!   the heap held, and change from run to run.
//! - libpng stops storing `sPLT`, text and unknown chunks after 1000 of them, which could only change the copy decision
//!   for a file with a thousand chunks before an `sPLT`.  That count is not kept.
//! - Ancillary chunks that only store text or metadata are skipped without inflating them, since their errors are all
//!   warnings.

use crate::out::{ObjNo, Op, SlotAlloc, Writer};
use crate::{Fail, IMAGE_COLOR_B, IMAGE_COLOR_C, IMAGE_COLOR_I};
use zlib_rs::{Inflate, InflateFlush, Status};

const COLOR_GRAY: u8 = 0;
const COLOR_RGB: u8 = 2;
const COLOR_PALETTE: u8 = 3;
const COLOR_GRAY_ALPHA: u8 = 4;
const COLOR_RGB_ALPHA: u8 = 6;
const MASK_COLOR: u8 = 2;
const MASK_ALPHA: u8 = 4;

const FP_1: i32 = 100000;
const GAMMA_THRESHOLD_FIXED: i32 = 5000;
const GAMMA_SRGB_INVERSE: i32 = 45455;
const LIB_GAMMA_MIN: i32 = 1000;
const LIB_GAMMA_MAX: i32 = 10000000;
const MAX_GAMMA_8: u32 = 11;
const USER_WIDTH_MAX: u32 = 1000000;
const USER_HEIGHT_MAX: u32 = 1000000;
const USER_CHUNK_MALLOC_MAX: u32 = 8000000;
const IDAT_READ_SIZE: usize = 8192;
const LZ77_MIN: u32 = 2 + 5 + 4;

const HAVE_IHDR: u32 = 0x01;
const HAVE_PLTE: u32 = 0x02;
const HAVE_IDAT: u32 = 0x04;
const AFTER_IDAT: u32 = 0x08;
const H_COL: u32 = HAVE_PLTE | HAVE_IDAT;

const fn chunk(n: &[u8; 4]) -> u32 {
    u32::from_be_bytes(*n)
}
const C_IHDR: u32 = chunk(b"IHDR");
const C_PLTE: u32 = chunk(b"PLTE");
const C_IDAT: u32 = chunk(b"IDAT");
const C_IEND: u32 = chunk(b"IEND");
const C_TRNS: u32 = chunk(b"tRNS");
const C_CHRM: u32 = chunk(b"cHRM");
const C_GAMA: u32 = chunk(b"gAMA");
const C_ICCP: u32 = chunk(b"iCCP");
const C_SBIT: u32 = chunk(b"sBIT");
const C_SRGB: u32 = chunk(b"sRGB");
const C_CICP: u32 = chunk(b"cICP");
const C_MDCV: u32 = chunk(b"mDCV");
const C_EXIF: u32 = chunk(b"eXIf");
const C_CLLI: u32 = chunk(b"cLLI");
const C_TEXT: u32 = chunk(b"tEXt");
const C_ZTXT: u32 = chunk(b"zTXt");
const C_ITXT: u32 = chunk(b"iTXt");
const C_BKGD: u32 = chunk(b"bKGD");
const C_HIST: u32 = chunk(b"hIST");
const C_PHYS: u32 = chunk(b"pHYs");
const C_SPLT: u32 = chunk(b"sPLT");
const C_TIME: u32 = chunk(b"tIME");
const C_OFFS: u32 = chunk(b"oFFs");
const C_PCAL: u32 = chunk(b"pCAL");
const C_SCAL: u32 = chunk(b"sCAL");

// png_get_valid flags, in a word of our own.
const V_CHRM: u32 = 1 << 0;
const V_ICCP: u32 = 1 << 1;
const V_SBIT: u32 = 1 << 2;
const V_SRGB: u32 = 1 << 3;
const V_BKGD: u32 = 1 << 4;
const V_HIST: u32 = 1 << 5;
const V_TRNS: u32 = 1 << 6;
const V_SPLT: u32 = 1 << 7;
const V_GAMA: u32 = 1 << 8;
const V_PHYS: u32 = 1 << 9;

fn png_error() -> Fail {
    Fail::new("libpng: internal error")
}

fn rowbytes(pixel_depth: u32, width: u32) -> usize {
    if pixel_depth >= 8 {
        width as usize * (pixel_depth >> 3) as usize
    } else {
        (width as usize * pixel_depth as usize + 7) >> 3
    }
}

/// What `png_read_info` leaves in `png_struct` and `png_info`, plus the fields `read_png_info` sets in the image entry.
#[derive(Clone, Debug, Default)]
pub struct PngInfo {
    /// `img_width`.
    pub width: u32,
    /// `img_height`.
    pub height: u32,
    /// `img_xres`.
    pub x_res: i32,
    /// `img_yres`.
    pub y_res: i32,
    /// `img_color`.
    pub img_color: i32,
    /// `png_get_bit_depth` before any transform: what `imagecolordepth` returns.
    pub bit_depth: u8,
    /// `png_get_color_type` before any transform.
    pub color_type: u8,
    /// `png_get_interlace_type`.
    pub interlace_type: u8,
    pixel_depth: u32,
    channels: u32,
    valid: u32,
    gamma: i32,
    chunk_gamma: i32,
    palette: Vec<[u8; 3]>,
    trans_alpha: Vec<u8>,
    num_trans: u32,
    trans_gray: u16,
    trans_rgb: [u16; 3],
    sig_gray: u8,
    sig_rgb: [u8; 3],
    /// File offset of the first IDAT's data, and its length.
    idat_data: usize,
    idat_size: u32,
}

impl PngInfo {
    /// The test in `read_png_info` for adding a page group.  When true the engine does what the C does:
    /// `if (transparent_page_group == 0) transparent_page_group = pdfnewobjnum();
    /// if (pdfpagegroupval == 0) pdfpagegroupval = transparent_page_group;
    /// img_group_ref(img) = pdfpagegroupval;`
    pub fn wants_page_group(&self, pdf_major: i32, pdf_minor: i32) -> bool {
        (pdf_major > 1 || pdf_minor >= 4) && (self.color_type == COLOR_GRAY_ALPHA || self.color_type == COLOR_RGB_ALPHA)
    }
}

struct Input<'a> {
    d: &'a [u8],
    pos: usize,
    crc: u32,
    chunk_name: u32,
}

impl Input<'_> {
    /// `png_read_data`: a short read is `png_error("Read Error")`.
    fn take(&mut self, n: usize) -> Result<&[u8], Fail> {
        if n > self.d.len() - self.pos.min(self.d.len()) {
            return Err(png_error());
        }
        let s = &self.d[self.pos..self.pos + n];
        self.pos += n;
        Ok(s)
    }

    fn crc_read(&mut self, n: usize) -> Result<Vec<u8>, Fail> {
        let v = self.take(n)?.to_vec();
        self.crc = zlib_rs::crc32::crc32(self.crc, &v);
        Ok(v)
    }

    /// `png_read_chunk_header`.
    fn read_chunk_header(&mut self) -> Result<u32, Fail> {
        let b = self.take(8)?;
        let length = u32::from_be_bytes([b[0], b[1], b[2], b[3]]);
        let name = u32::from_be_bytes([b[4], b[5], b[6], b[7]]);
        if length > 0x7fffffff {
            return Err(png_error());
        }
        self.chunk_name = name;
        self.crc = zlib_rs::crc32::crc32(0, &name.to_be_bytes());
        if !check_chunk_name(name) {
            return Err(png_error());
        }
        Ok(length)
    }

    /// The CRC check of `png_crc_finish_critical`, after `skip` more bytes; true means a CRC error.
    fn crc_error_after(&mut self, skip: usize) -> Result<bool, Fail> {
        let s = self.take(skip)?.to_vec();
        self.crc = zlib_rs::crc32::crc32(self.crc, &s);
        let c = self.take(4)?;
        let stored = u32::from_be_bytes([c[0], c[1], c[2], c[3]]);
        Ok(stored != self.crc)
    }

    /// `png_crc_finish`: an error for a critical chunk, a warning for an ancillary one.  True means a CRC error.
    fn crc_finish(&mut self, skip: usize) -> Result<bool, Fail> {
        let bad = self.crc_error_after(skip)?;
        if bad && !ancillary(self.chunk_name) {
            return Err(png_error());
        }
        Ok(bad)
    }

    /// `png_crc_finish_critical(.., 1)`: handled as ancillary whatever the chunk is.
    fn crc_finish_ancillary(&mut self, skip: usize) -> Result<(), Fail> {
        self.crc_error_after(skip)?;
        Ok(())
    }
}

fn ancillary(name: u32) -> bool {
    name & 0x20000000 != 0
}

/// `check_chunk_name`.
fn check_chunk_name(name: u32) -> bool {
    let mut n = name & !u32::from_be_bytes([32, 32, 0, 32]);
    let mut t = (n & !0x1f1f1f1fu32) ^ 0x40404040;
    n = n.wrapping_sub(u32::from_be_bytes([65, 65, 65, 65]));
    t |= n;
    n = n.wrapping_sub(u32::from_be_bytes([25, 25, 25, 26]));
    t |= !n;
    t & 0xe0e0e0e0 == 0
}

/// Inflates at most `want` bytes of `input` the way `png_inflate_read` reads a truncated stream.
fn inflate_up_to(input: &[u8], want: usize) -> Vec<u8> {
    let mut z = Inflate::new(true, 15);
    let mut out = vec![0u8; want];
    let mut got = 0;
    let mut used = 0;
    while got < want {
        let (i0, o0) = (z.total_in(), z.total_out());
        let r = z.decompress(&input[used..], &mut out[got..], InflateFlush::SyncFlush);
        used += (z.total_in() - i0) as usize;
        got += (z.total_out() - o0) as usize;
        match r {
            Ok(Status::Ok) if z.total_in() != i0 || z.total_out() != o0 => {}
            _ => break,
        }
    }
    out.truncate(got);
    out
}

/// `png_icc_check_length` and `png_icc_check_header`.
fn icc_check_header(profile_length: u32, p: &[u8], color_type: u8) -> bool {
    let u32at = |i: usize| u32::from_be_bytes([p[i], p[i + 1], p[i + 2], p[i + 3]]);
    if profile_length < 132 || profile_length > USER_CHUNK_MALLOC_MAX {
        return false;
    }
    if u32at(0) != profile_length {
        return false;
    }
    if p[8] > 3 && profile_length & 3 != 0 {
        return false;
    }
    let tag_count = u32at(128);
    if tag_count > 357913930 || profile_length < 132 + 12 * tag_count {
        return false;
    }
    if u32at(64) >= 0xffff {
        return false;
    }
    if u32at(36) != 0x61637370 {
        return false;
    }
    // A PCS illuminant other than D50, at offset 68, is only a warning.
    match u32at(16) {
        0x52474220 => {
            if color_type & MASK_COLOR == 0 {
                return false;
            }
        }
        0x47524159 => {
            if color_type & MASK_COLOR != 0 {
                return false;
            }
        }
        _ => return false,
    }
    match u32at(12) {
        0x61627374 | 0x6c696e6b => return false,
        _ => {}
    }
    matches!(u32at(20), 0x58595a20 | 0x4c616220)
}

/// `png_handle_iCCP`.  Returns true when the profile is accepted.  A CRC error does not matter, as in libpng.
fn handle_iccp(inp: &mut Input<'_>, length: u32, color_type: u8) -> Result<bool, Fail> {
    let read_length = length.min(81) as usize;
    let keyword = inp.crc_read(read_length)?;
    let rest = length - read_length as u32;
    if rest < LZ77_MIN {
        inp.crc_finish(rest as usize)?;
        return Ok(false);
    }
    let mut keyword_length = 0;
    while keyword_length < 80 && keyword_length < read_length && keyword[keyword_length] != 0 {
        keyword_length += 1;
    }
    let mut ok = false;
    if (1..=79).contains(&keyword_length) && keyword_length + 1 < read_length && keyword[keyword_length + 1] == 0 {
        let start = inp.pos;
        let avail = rest as usize;
        if avail <= inp.d.len() - start.min(inp.d.len()) {
            let mut compressed = keyword[keyword_length + 2..].to_vec();
            compressed.extend_from_slice(&inp.d[start..start + avail]);
            let header = inflate_up_to(&compressed, 132);
            if header.len() == 132 {
                let profile_length = u32::from_be_bytes([header[0], header[1], header[2], header[3]]);
                if icc_check_header(profile_length, &header, color_type) {
                    let tag_count = u32::from_be_bytes([header[128], header[129], header[130], header[131]]) as usize;
                    let table = inflate_up_to(&compressed, 132 + 12 * tag_count);
                    if table.len() == 132 + 12 * tag_count {
                        let mut good = true;
                        for t in 0..tag_count {
                            let at = 132 + 12 * t;
                            let start = u32::from_be_bytes([table[at + 4], table[at + 5], table[at + 6], table[at + 7]]);
                            let len = u32::from_be_bytes([table[at + 8], table[at + 9], table[at + 10], table[at + 11]]);
                            if start > profile_length || len > profile_length - start {
                                good = false;
                                break;
                            }
                        }
                        if good && inflate_up_to(&compressed, profile_length as usize).len() == profile_length as usize {
                            ok = true;
                        }
                    }
                }
            }
        }
    }
    inp.crc_finish_ancillary(rest as usize)?;
    Ok(ok)
}

/// `read_png_info`: `png_read_info` up to the first IDAT, then the image entry.
pub fn read_png_info(data: &[u8]) -> Result<PngInfo, Fail> {
    const SIG: [u8; 8] = [137, 80, 78, 71, 13, 10, 26, 10];
    let mut inp = Input { d: data, pos: 0, crc: 0, chunk_name: 0 };
    if inp.take(8)? != SIG {
        return Err(png_error());
    }
    let mut info = PngInfo::default();
    let mut mode = 0u32;
    let mut seen_cicp = false;
    let mut seen: u32 = 0; // chunks_seen, for the ones that may not repeat
    loop {
        let length = inp.read_chunk_header()?;
        let name = inp.chunk_name;
        if name == C_IDAT {
            if mode & HAVE_IHDR == 0 {
                return Err(png_error());
            }
            if info.color_type == COLOR_PALETTE && mode & HAVE_PLTE == 0 {
                return Err(png_error());
            }
            info.idat_data = inp.pos;
            info.idat_size = length;
            break;
        }
        if name != C_IHDR && mode & HAVE_IHDR == 0 && known_chunk(name) {
            return Err(png_error());
        }
        // The table in png_handle_chunk: before, after, multiple, min and max length.
        let rule: Option<(u32, u32, bool, u32, u32)> = match name {
            C_IHDR => Some((HAVE_IHDR, 0, false, 13, 13)),
            C_PLTE => Some((0, HAVE_IHDR, true, 0, u32::MAX)),
            C_IEND => Some((0, AFTER_IDAT, false, 0, u32::MAX)),
            C_TRNS => Some((HAVE_IDAT, HAVE_IHDR, false, 0, 256)),
            C_CHRM => Some((H_COL, HAVE_IHDR, false, 32, 32)),
            C_GAMA => Some((H_COL, HAVE_IHDR, false, 4, 4)),
            C_ICCP => Some((H_COL, HAVE_IHDR, false, 3 + LZ77_MIN, u32::MAX)),
            C_SBIT => Some((H_COL, HAVE_IHDR, false, 1, 4)),
            C_SRGB => Some((H_COL, HAVE_IHDR, false, 1, 1)),
            C_CICP => Some((H_COL, HAVE_IHDR, false, 4, 4)),
            C_MDCV => Some((H_COL, HAVE_IHDR, false, 24, 24)),
            C_EXIF => Some((0, HAVE_IHDR, false, 4, USER_CHUNK_MALLOC_MAX)),
            C_CLLI => Some((H_COL, HAVE_IHDR, false, 8, 8)),
            C_TEXT => Some((0, HAVE_IHDR, true, 2, u32::MAX)),
            C_ZTXT => Some((0, HAVE_IHDR, true, 3 + LZ77_MIN, USER_CHUNK_MALLOC_MAX)),
            C_ITXT => Some((0, HAVE_IHDR, true, 6, u32::MAX)),
            C_BKGD => Some((HAVE_IDAT, HAVE_IHDR, false, 1, 6)),
            C_HIST => Some((HAVE_IDAT, HAVE_PLTE, false, 0, 1024)),
            C_PHYS => Some((HAVE_IDAT, HAVE_IHDR, false, 9, 9)),
            C_SPLT => Some((HAVE_IDAT, HAVE_IHDR, true, 3, u32::MAX)),
            C_TIME => Some((0, HAVE_IHDR, false, 7, 7)),
            C_OFFS => Some((HAVE_IDAT, HAVE_IHDR, false, 9, 9)),
            C_PCAL => Some((HAVE_IDAT, HAVE_IHDR, false, 14, u32::MAX)),
            C_SCAL => Some((HAVE_IDAT, HAVE_IHDR, false, 4, USER_CHUNK_MALLOC_MAX)),
            _ => None,
        };
        let Some((before, after, multiple, min_len, max_len)) = rule else {
            // png_handle_unknown with the default handling: skip it, and a critical one is an error.
            inp.crc_finish_ancillary(length as usize)?;
            if !ancillary(name) {
                return Err(png_error());
            }
            continue;
        };
        let bit = seen_bit(name);
        let wrong = mode & before != 0
            || mode & after != after
            || (!multiple && seen & bit != 0)
            || length < min_len
            || length > max_len;
        if wrong {
            if !ancillary(name) {
                return Err(png_error());
            }
            inp.crc_finish(length as usize)?;
            continue;
        }
        let handled = match name {
            C_IHDR => {
                mode |= HAVE_IHDR;
                handle_ihdr(&mut inp, &mut info)?;
                true
            }
            C_PLTE => handle_plte(&mut inp, &mut info, length, &mut mode, seen)?,
            C_IEND => {
                mode |= AFTER_IDAT;
                inp.crc_finish_ancillary(length as usize)?;
                true
            }
            C_TRNS => handle_trns(&mut inp, &mut info, length, mode)?,
            C_BKGD => handle_bkgd(&mut inp, &mut info, length, mode)?,
            C_HIST => handle_hist(&mut inp, &mut info, length)?,
            C_SBIT => handle_sbit(&mut inp, &mut info, length)?,
            C_GAMA => {
                let b = inp.crc_read(4)?;
                if inp.crc_finish(0)? {
                    false
                } else {
                    let g = u32::from_be_bytes([b[0], b[1], b[2], b[3]]);
                    if g > 0x7fffffff {
                        false
                    } else {
                        info.gamma = g as i32;
                        info.valid |= V_GAMA;
                        if info.chunk_gamma == 0 {
                            info.chunk_gamma = g as i32;
                        }
                        true
                    }
                }
            }
            C_CHRM => {
                let b = inp.crc_read(32)?;
                if inp.crc_finish(0)? {
                    false
                } else {
                    let bad = b.chunks(4).any(|c| u32::from_be_bytes([c[0], c[1], c[2], c[3]]) == 0x80000000);
                    if !bad {
                        info.valid |= V_CHRM;
                    }
                    !bad
                }
            }
            C_SRGB => {
                let b = inp.crc_read(1)?;
                if inp.crc_finish(0)? || b[0] > 3 {
                    false
                } else {
                    info.valid |= V_SRGB;
                    if !seen_cicp || info.chunk_gamma == 0 {
                        info.chunk_gamma = GAMMA_SRGB_INVERSE;
                    }
                    true
                }
            }
            C_ICCP => {
                let ok = handle_iccp(&mut inp, length, info.color_type)?;
                if ok {
                    info.valid |= V_ICCP;
                }
                ok
            }
            C_SPLT => handle_splt(&mut inp, &mut info, length)?,
            C_PHYS => {
                let b = inp.crc_read(9)?;
                if inp.crc_finish(0)? {
                    false
                } else {
                    info.valid |= V_PHYS;
                    // png_get_x_pixels_per_meter is 0 unless the unit is the metre.
                    let (mut x, mut y) = (0u32, 0u32);
                    if b[8] == 1 {
                        x = u32::from_be_bytes([b[0], b[1], b[2], b[3]]);
                        y = u32::from_be_bytes([b[4], b[5], b[6], b[7]]);
                    }
                    info.x_res = crate::zround(0.0254 * x as f64);
                    info.y_res = crate::zround(0.0254 * y as f64);
                    true
                }
            }
            C_CICP => {
                let bad = inp.crc_finish(length as usize)?;
                if !bad {
                    seen_cicp = true;
                }
                !bad
            }
            _ => {
                // Handlers that store text or metadata pdfTeX never looks at; their errors are all benign.
                let bad = inp.crc_finish(length as usize)?;
                !bad
            }
        };
        if handled {
            seen |= bit;
        }
    }
    info.img_color = match info.color_type {
        COLOR_PALETTE => IMAGE_COLOR_C | IMAGE_COLOR_I,
        COLOR_GRAY | COLOR_GRAY_ALPHA => IMAGE_COLOR_B,
        _ => IMAGE_COLOR_C,
    };
    Ok(info)
}

fn known_chunk(name: u32) -> bool {
    matches!(
        name,
        C_PLTE
            | C_IEND
            | C_TRNS
            | C_CHRM
            | C_GAMA
            | C_ICCP
            | C_SBIT
            | C_SRGB
            | C_CICP
            | C_MDCV
            | C_EXIF
            | C_CLLI
            | C_TEXT
            | C_ZTXT
            | C_ITXT
            | C_BKGD
            | C_HIST
            | C_PHYS
            | C_SPLT
            | C_TIME
            | C_OFFS
            | C_PCAL
            | C_SCAL
    )
}

fn seen_bit(name: u32) -> u32 {
    match name {
        C_TRNS => 1 << 0,
        C_CHRM => 1 << 1,
        C_GAMA => 1 << 2,
        C_ICCP => 1 << 3,
        C_SBIT => 1 << 4,
        C_SRGB => 1 << 5,
        C_CICP => 1 << 6,
        C_MDCV => 1 << 7,
        C_EXIF => 1 << 8,
        C_CLLI => 1 << 9,
        C_BKGD => 1 << 10,
        C_HIST => 1 << 11,
        C_PHYS => 1 << 12,
        C_TIME => 1 << 13,
        C_OFFS => 1 << 14,
        C_PCAL => 1 << 15,
        C_SCAL => 1 << 16,
        C_IEND => 1 << 17,
        _ => 0,
    }
}

const SEEN_TRNS: u32 = 1 << 0;
const SEEN_BKGD: u32 = 1 << 10;

/// `png_handle_IHDR`, with `png_check_IHDR`.
fn handle_ihdr(inp: &mut Input<'_>, info: &mut PngInfo) -> Result<(), Fail> {
    let b = inp.crc_read(13)?;
    inp.crc_finish(0)?;
    let width = u32::from_be_bytes([b[0], b[1], b[2], b[3]]);
    let height = u32::from_be_bytes([b[4], b[5], b[6], b[7]]);
    let (bit_depth, color_type, compression, filter, interlace) = (b[8], b[9], b[10], b[11], b[12]);
    if width > 0x7fffffff || height > 0x7fffffff {
        return Err(png_error());
    }
    let mut error = width == 0 || width > USER_WIDTH_MAX || height == 0 || height > USER_HEIGHT_MAX;
    error |= !matches!(bit_depth, 1 | 2 | 4 | 8 | 16);
    error |= !matches!(color_type, 0 | 2 | 3 | 4 | 6);
    error |= color_type == COLOR_PALETTE && bit_depth > 8;
    error |= matches!(color_type, COLOR_RGB | COLOR_GRAY_ALPHA | COLOR_RGB_ALPHA) && bit_depth < 8;
    error |= interlace >= 2;
    error |= compression != 0;
    error |= filter != 0;
    if error {
        return Err(png_error());
    }
    info.width = width;
    info.height = height;
    info.bit_depth = bit_depth;
    info.color_type = color_type;
    info.interlace_type = interlace;
    info.channels = match color_type {
        COLOR_RGB => 3,
        COLOR_GRAY_ALPHA => 2,
        COLOR_RGB_ALPHA => 4,
        _ => 1,
    };
    info.pixel_depth = bit_depth as u32 * info.channels;
    Ok(())
}

/// `png_handle_PLTE`.
fn handle_plte(inp: &mut Input<'_>, info: &mut PngInfo, length: u32, mode: &mut u32, seen: u32) -> Result<bool, Fail> {
    let palette_image = info.color_type == COLOR_PALETTE;
    let bad = *mode & HAVE_PLTE != 0
        || *mode & HAVE_IDAT != 0
        || info.color_type & MASK_COLOR == 0
        || length > 3 * 256
        || length % 3 != 0
        || (!palette_image && (seen & SEEN_TRNS != 0 || seen & SEEN_BKGD != 0));
    if bad {
        if palette_image {
            inp.crc_finish(length as usize)?;
            return Err(png_error());
        }
        inp.crc_finish_ancillary(length as usize)?;
        return Ok(false);
    }
    let max = if palette_image { 1u32 << info.bit_depth } else { 256 };
    let num = if length > 3 * max { max } else { length / 3 };
    let b = inp.crc_read(num as usize * 3)?;
    let skip = (length - 3 * num) as usize;
    if palette_image {
        inp.crc_finish(skip)?;
    } else {
        inp.crc_finish_ancillary(skip)?;
    }
    *mode |= HAVE_PLTE;
    if num == 0 {
        return Err(png_error()); // png_set_PLTE: "Invalid palette"
    }
    info.palette = b.chunks(3).map(|c| [c[0], c[1], c[2]]).collect();
    Ok(true)
}

/// `png_handle_tRNS`.
fn handle_trns(inp: &mut Input<'_>, info: &mut PngInfo, length: u32, mode: u32) -> Result<bool, Fail> {
    let mut readbuf = Vec::new();
    let mut gray = info.trans_gray;
    let mut rgb = info.trans_rgb;
    let num_trans;
    match info.color_type {
        COLOR_GRAY => {
            if length != 2 {
                inp.crc_finish(length as usize)?;
                return Ok(false);
            }
            let b = inp.crc_read(2)?;
            num_trans = 1;
            gray = u16::from_be_bytes([b[0], b[1]]);
        }
        COLOR_RGB => {
            if length != 6 {
                inp.crc_finish(length as usize)?;
                return Ok(false);
            }
            let b = inp.crc_read(6)?;
            num_trans = 1;
            rgb = [
                u16::from_be_bytes([b[0], b[1]]),
                u16::from_be_bytes([b[2], b[3]]),
                u16::from_be_bytes([b[4], b[5]]),
            ];
        }
        COLOR_PALETTE => {
            if mode & HAVE_PLTE == 0 {
                inp.crc_finish(length as usize)?;
                return Ok(false);
            }
            if length > info.palette.len() as u32 || length > 256 || length == 0 {
                inp.crc_finish(length as usize)?;
                return Ok(false);
            }
            readbuf = inp.crc_read(length as usize)?;
            num_trans = length;
        }
        _ => {
            inp.crc_finish(length as usize)?;
            return Ok(false);
        }
    }
    if inp.crc_finish(0)? {
        info.num_trans = 0;
        return Ok(false);
    }
    info.num_trans = num_trans;
    info.trans_gray = gray;
    info.trans_rgb = rgb;
    info.trans_alpha = readbuf;
    info.valid |= V_TRNS;
    Ok(true)
}

/// `png_handle_bKGD`.
fn handle_bkgd(inp: &mut Input<'_>, info: &mut PngInfo, length: u32, mode: u32) -> Result<bool, Fail> {
    let truelen = if info.color_type == COLOR_PALETTE {
        if mode & HAVE_PLTE == 0 {
            inp.crc_finish(length as usize)?;
            return Ok(false);
        }
        1
    } else if info.color_type & MASK_COLOR != 0 {
        6
    } else {
        2
    };
    if length != truelen {
        inp.crc_finish(length as usize)?;
        return Ok(false);
    }
    let b = inp.crc_read(truelen as usize)?;
    if inp.crc_finish(0)? {
        return Ok(false);
    }
    if info.color_type == COLOR_PALETTE {
        if !info.palette.is_empty() && b[0] as usize >= info.palette.len() {
            return Ok(false);
        }
    } else if info.color_type & MASK_COLOR == 0 {
        if info.bit_depth <= 8 && (b[0] != 0 || (b[1] as u32) >= (1u32 << info.bit_depth)) {
            return Ok(false);
        }
    } else if info.bit_depth <= 8 && (b[0] != 0 || b[2] != 0 || b[4] != 0) {
        return Ok(false);
    }
    info.valid |= V_BKGD;
    Ok(true)
}

/// `png_handle_hIST`.
fn handle_hist(inp: &mut Input<'_>, info: &mut PngInfo, length: u32) -> Result<bool, Fail> {
    let num = length / 2;
    if length != num * 2 || num != info.palette.len() as u32 || num > 256 {
        inp.crc_finish(length as usize)?;
        return Ok(false);
    }
    if inp.crc_finish(length as usize)? {
        return Ok(false);
    }
    if info.palette.is_empty() {
        return Ok(true); // png_set_hIST: "Invalid palette size, hIST allocation skipped"
    }
    info.valid |= V_HIST;
    Ok(true)
}

/// `png_handle_sBIT`.
fn handle_sbit(inp: &mut Input<'_>, info: &mut PngInfo, length: u32) -> Result<bool, Fail> {
    let (truelen, sample_depth) =
        if info.color_type == COLOR_PALETTE { (3, 8) } else { (info.channels, info.bit_depth) };
    if length != truelen {
        inp.crc_finish(length as usize)?;
        return Ok(false);
    }
    let mut buf = [sample_depth; 4];
    let b = inp.crc_read(truelen as usize)?;
    buf[..truelen as usize].copy_from_slice(&b);
    if inp.crc_finish(0)? {
        return Ok(false);
    }
    for &v in &buf[..truelen as usize] {
        if v == 0 || v > sample_depth {
            return Ok(false);
        }
    }
    if info.color_type & MASK_COLOR != 0 {
        info.sig_rgb = [buf[0], buf[1], buf[2]];
    } else {
        info.sig_gray = buf[0];
        info.sig_rgb = [buf[0], buf[0], buf[0]];
    }
    info.valid |= V_SBIT;
    Ok(true)
}

/// `png_handle_sPLT`.
fn handle_splt(inp: &mut Input<'_>, info: &mut PngInfo, length: u32) -> Result<bool, Fail> {
    let buffer = inp.crc_read(length as usize)?;
    if inp.crc_finish(0)? {
        return Ok(false);
    }
    let name_end = buffer.iter().position(|&c| c == 0).unwrap_or(buffer.len());
    let entry_start = name_end + 1;
    if length < 2 || entry_start > length as usize - 2 {
        return Ok(false);
    }
    let depth = buffer[entry_start];
    let entry_size = if depth == 8 { 6 } else { 10 };
    let data_length = length as usize - (entry_start + 1);
    if data_length % entry_size != 0 || data_length == 0 {
        return Ok(false);
    }
    info.valid |= V_SPLT;
    Ok(true)
}

//------------------------------------------------------------------------
// pow.  libm's differs by platform; output must not.
//------------------------------------------------------------------------

#[derive(Clone, Copy)]
struct Dd(f64, f64);

fn two_sum(a: f64, b: f64) -> Dd {
    let s = a + b;
    let v = s - a;
    Dd(s, (a - (s - v)) + (b - v))
}

fn split(a: f64) -> (f64, f64) {
    let t = 134217729.0 * a;
    let hi = t - (t - a);
    (hi, a - hi)
}

fn two_prod(a: f64, b: f64) -> Dd {
    let p = a * b;
    let (ah, al) = split(a);
    let (bh, bl) = split(b);
    Dd(p, ((ah * bh - p) + ah * bl + al * bh) + al * bl)
}

fn dd_add(a: Dd, b: Dd) -> Dd {
    let s = two_sum(a.0, b.0);
    let t = two_sum(a.1, b.1);
    let v = two_sum(s.0, s.1 + t.0);
    two_sum(v.0, v.1 + t.1)
}

fn dd_neg(a: Dd) -> Dd {
    Dd(-a.0, -a.1)
}

fn dd_mul(a: Dd, b: Dd) -> Dd {
    let p = two_prod(a.0, b.0);
    two_sum(p.0, p.1 + (a.0 * b.1 + a.1 * b.0))
}

fn dd_div(a: Dd, b: Dd) -> Dd {
    let q1 = a.0 / b.0;
    let r = dd_add(a, dd_neg(dd_mul(b, Dd(q1, 0.0))));
    let q2 = r.0 / b.0;
    let r2 = dd_add(r, dd_neg(dd_mul(b, Dd(q2, 0.0))));
    let q3 = r2.0 / b.0;
    dd_add(two_sum(q1, q2), Dd(q3, 0.0))
}

const LN2: Dd = Dd(std::f64::consts::LN_2, 2.3190468138462996e-17);

/// e^x to about 30 digits, as a pair and a binary exponent.
fn dd_exp(x: Dd) -> (Dd, i32) {
    let k = (x.0 / LN2.0 + if x.0 >= 0.0 { 0.5 } else { -0.5 }) as i32;
    let r = dd_add(x, dd_neg(dd_mul(LN2, Dd(k as f64, 0.0))));
    // r / 512, then 2^9 squarings of 1 + t.
    let m = Dd(r.0 / 512.0, r.1 / 512.0);
    let mut term = m;
    let mut sum = m;
    for n in 2..14 {
        term = dd_mul(term, m);
        term = dd_div(term, Dd(n as f64, 0.0));
        sum = dd_add(sum, term);
    }
    // (1 + sum)^2 - 1 = sum * (2 + sum)
    for _ in 0..9 {
        sum = dd_mul(sum, dd_add(Dd(2.0, 0.0), sum));
    }
    (dd_add(Dd(1.0, 0.0), sum), k)
}

fn scale2(v: f64, k: i32) -> f64 {
    // Multiplying in steps keeps the exact power of two representable.
    let mut r = v;
    let mut e = k;
    while e > 1000 {
        r *= f64::from_bits(((1000 + 1023) as u64) << 52);
        e -= 1000;
    }
    while e < -1000 {
        r *= f64::from_bits(((-1000 + 1023) as u64) << 52);
        e += 1000;
    }
    r * f64::from_bits(((e + 1023) as u64) << 52)
}

/// The natural logarithm of a positive finite `x`, by Newton's iteration on `dd_exp`.
fn dd_ln(x: f64) -> Dd {
    // A first guess from the exponent and a short series; two iterations then pass double-double accuracy.
    let bits = x.to_bits();
    let mut e = ((bits >> 52) & 0x7ff) as i32 - 1023;
    let mut f = f64::from_bits((bits & !(0x7ffu64 << 52)) | (1023u64 << 52));
    if e == -1023 {
        // subnormal
        let n = x * 18014398509481984.0; // 2^54
        let b2 = n.to_bits();
        e = ((b2 >> 52) & 0x7ff) as i32 - 1023 - 54;
        f = f64::from_bits((b2 & !(0x7ffu64 << 52)) | (1023u64 << 52));
    }
    let z = (f - 1.0) / (f + 1.0);
    let z2 = z * z;
    let mut guess = e as f64 * LN2.0 + 2.0 * z * (1.0 + z2 / 3.0 + z2 * z2 / 5.0 + z2 * z2 * z2 / 7.0);
    // Newton on plain doubles first: exp is not needed to full accuracy here.
    let mut y = Dd(guess, 0.0);
    for _ in 0..4 {
        let (ex, k) = dd_exp(y);
        let exv = Dd(scale2(ex.0, k), scale2(ex.1, k));
        // y += 2 (x - e^y) / (x + e^y), cubically convergent
        let num = dd_add(Dd(x, 0.0), dd_neg(exv));
        let den = dd_add(Dd(x, 0.0), exv);
        let q = dd_div(num, den);
        y = dd_add(y, Dd(2.0 * q.0, 2.0 * q.1));
    }
    guess = y.0;
    let _ = guess;
    y
}

/// `pow(x, y)` for `x` in [0, 1] and any finite `y`, correctly rounded to within the double-double's error.  Same
/// result on every platform, unlike libm's.
pub(crate) fn pow(x: f64, y: f64) -> f64 {
    if x == 1.0 || y == 0.0 {
        return 1.0;
    }
    if x == 0.0 {
        return if y > 0.0 { 0.0 } else { f64::INFINITY };
    }
    let l = dd_ln(x);
    let p = dd_mul(l, Dd(y, 0.0));
    if p.0 < -800.0 {
        return 0.0;
    }
    let (ex, k) = dd_exp(p);
    let r = two_sum(ex.0, ex.1);
    scale2(r.0, k)
}

//------------------------------------------------------------------------
// gamma
//------------------------------------------------------------------------

/// `png_gamma_significant`.
fn gamma_significant(g: i32) -> bool {
    g < FP_1 - GAMMA_THRESHOLD_FIXED || g > FP_1 + GAMMA_THRESHOLD_FIXED
}

/// `png_reciprocal`.
fn reciprocal(a: i32) -> i32 {
    let r = (1e10 / a as f64 + 0.5).floor();
    if (-2147483648.0..=2147483647.0).contains(&r) { r as i32 } else { 0 }
}

/// `png_reciprocal2`.
fn reciprocal2(a: i32, b: i32) -> i32 {
    if a != 0 && b != 0 {
        let r = ((1e15 / a as f64) / b as f64 + 0.5).floor();
        if (-2147483648.0..=2147483647.0).contains(&r) {
            return r as i32;
        }
    }
    0
}

/// `png_gamma_threshold`, with `png_muldiv`.
fn gamma_threshold(screen: i32, file: i32) -> bool {
    if screen == 0 || file == 0 {
        return gamma_significant(0);
    }
    let r = ((screen as f64 * file as f64) / FP_1 as f64 + 0.5).floor();
    if (-2147483648.0..=2147483647.0).contains(&r) { gamma_significant(r as i32) } else { true }
}

/// `floor(scale * pow(x, y) + .5)`, the form of every table entry.  libm's `pow` gets the answer unless the value is
/// close to a rounding boundary, where a last-place difference between platforms could show; there `pow` decides.
fn rounded_pow(scale: f64, x: f64, y: f64) -> f64 {
    let est = scale * x.powf(y) + 0.5;
    let f = est.floor();
    if est - f > 1e-6 && f + 1.0 - est > 1e-6 {
        return f;
    }
    (scale * pow(x, y) + 0.5).floor()
}

/// `png_gamma_8bit_correct`.
fn gamma_8bit_correct(value: u32, g: i32) -> u8 {
    if value > 0 && value < 255 {
        return rounded_pow(255.0, value as f64 / 255.0, g as f64 * 0.00001) as u8;
    }
    (value & 0xff) as u8
}

/// `png_gamma_16bit_correct`.
fn gamma_16bit_correct(value: u32, g: i32) -> u16 {
    if value > 0 && value < 65535 {
        return rounded_pow(65535.0, value as f64 / 65535.0, g as f64 * 0.00001) as u16;
    }
    value as u16
}

/// `png_build_8bit_table`.
fn build_8bit_table(g: i32) -> Vec<u8> {
    if gamma_significant(g) {
        (0..256).map(|i| gamma_8bit_correct(i, g)).collect()
    } else {
        (0..256).map(|i| i as u8).collect()
    }
}

/// `png_build_16bit_table`: `num` subtables of 256 entries, flat.
fn build_16bit_table(shift: u32, g: i32) -> Vec<u16> {
    let num = 1u32 << (8 - shift);
    let fmax = 1.0 / ((1i32 << (16 - shift)) - 1) as f64;
    let max = (1u32 << (16 - shift)) - 1;
    let max_by_2 = 1u32 << (15 - shift);
    let mut t = vec![0u16; num as usize * 256];
    let sig = gamma_significant(g);
    for i in 0..num {
        for j in 0..256u32 {
            let mut ig = (j << (8 - shift)) + i;
            let v = if sig {
                rounded_pow(65535.0, ig as f64 * fmax, g as f64 * 0.00001) as u16
            } else {
                if shift != 0 {
                    ig = (ig * 65535 + max_by_2) / max;
                }
                ig as u16
            };
            t[i as usize * 256 + j as usize] = v;
        }
    }
    t
}

/// `png_build_16to8_table`.
fn build_16to8_table(shift: u32, g: i32) -> Vec<u16> {
    let num = 1u32 << (8 - shift);
    let max = (1u32 << (16 - shift)) - 1;
    let mut t = vec![0u16; num as usize * 256];
    let mut last = 0u32;
    let put = |t: &mut Vec<u16>, last: u32, out: u16| {
        let sub = (last & (0xff >> shift)) as usize;
        let idx = (last >> (8 - shift)) as usize;
        t[sub * 256 + idx] = out;
    };
    for i in 0..255u32 {
        let out = (i * 257) as u16;
        let bound = gamma_16bit_correct(out as u32 + 128, g) as u32;
        let bound = (bound * max + 32768) / 65535 + 1;
        while last < bound {
            put(&mut t, last, out);
            last += 1;
        }
    }
    while last < (num << 8) {
        put(&mut t, last, 65535);
        last += 1;
    }
    t
}

/// `png_set_gamma`, down to the two fixed-point values.  `None` means `png_fixed_error`, which is a `png_error`.
fn convert_gamma_value(mut g: f64) -> Result<i32, Fail> {
    if g > 0.0 && g < 128.0 {
        g *= FP_1 as f64;
    }
    g = (g + 0.5).floor();
    if !(-2147483648.0..=2147483647.0).contains(&g) {
        return Err(png_error());
    }
    Ok(g as i32)
}

fn translate_gamma_flags(g: i32, is_screen: bool) -> i32 {
    if g == -1 || g == FP_1 / -1 {
        return if is_screen { 220000 } else { GAMMA_SRGB_INVERSE };
    }
    if g == -2 || g == FP_1 / -2 {
        return if is_screen { 151724 } else { 65909 };
    }
    g
}

//------------------------------------------------------------------------
// the row machine
//------------------------------------------------------------------------

#[derive(Default)]
struct Transforms {
    expand: bool,
    expand_trns: bool,
    strip_alpha: bool,
    chop_16: bool,
    gamma: bool,
    gamma_table: Vec<u8>,
    gamma_16_table: Vec<u16>,
    gamma_shift: u32,
}

#[derive(Clone, Copy)]
struct RowInfo {
    width: u32,
    color_type: u8,
    bit_depth: u8,
    channels: u8,
    pixel_depth: u8,
    rowbytes: usize,
}

struct Rows<'a> {
    inp: Input<'a>,
    idat_size: u32,
    z: Inflate,
    zin: Vec<u8>,
    zin_pos: usize,
    zstream_ended: bool,
    row_buf: Vec<u8>,
    prev_row: Vec<u8>,
    t: Transforms,
    palette: Vec<[u8; 3]>,
    trans_alpha: Vec<u8>,
    num_trans: u32,
    trans_gray: u16,
    trans_rgb: [u16; 3],
    file_color_type: u8,
    file_bit_depth: u8,
    file_channels: u8,
    file_pixel_depth: u8,
}

impl Rows<'_> {
    /// `png_read_IDAT_data`.  `output` of `None` is the check after the last row.
    fn read_idat_data(&mut self, n: usize, check_only: bool) -> Result<(), Fail> {
        let mut filled = 0usize;
        let mut tmp = [0u8; 1024];
        let mut avail_out = if check_only { 0 } else { n };
        loop {
            if self.zin_pos == self.zin.len() {
                while self.idat_size == 0 {
                    self.inp.crc_finish(0)?;
                    self.idat_size = self.inp.read_chunk_header()?;
                    if self.inp.chunk_name != C_IDAT {
                        return Err(png_error());
                    }
                }
                let avail_in = (self.idat_size as usize).min(IDAT_READ_SIZE);
                self.zin = self.inp.crc_read(avail_in)?;
                self.idat_size -= avail_in as u32;
                self.zin_pos = 0;
            }
            let (i0, o0) = (self.z.total_in(), self.z.total_out());
            let ret = if check_only {
                self.z.decompress(&self.zin[self.zin_pos..], &mut tmp, InflateFlush::NoFlush)
            } else {
                let out = &mut self.row_buf[filled..n];
                self.z.decompress(&self.zin[self.zin_pos..], out, InflateFlush::NoFlush)
            };
            self.zin_pos += (self.z.total_in() - i0) as usize;
            let produced = (self.z.total_out() - o0) as usize;
            if check_only {
                avail_out += produced;
            } else {
                filled += produced;
                avail_out -= produced;
            }
            match ret {
                Ok(Status::StreamEnd) => {
                    self.zstream_ended = true;
                    break;
                }
                Ok(Status::Ok) => {}
                _ => {
                    if check_only {
                        return Ok(());
                    }
                    return Err(png_error());
                }
            }
            // In the check, avail_out counts what came out: the loop runs on while there is any.
            if avail_out == 0 {
                break;
            }
        }
        if !check_only && avail_out > 0 {
            return Err(png_error()); // "Not enough image data"
        }
        Ok(())
    }

    /// `png_read_finish_IDAT`.
    fn finish_idat(&mut self) -> Result<(), Fail> {
        if !self.zstream_ended {
            self.read_idat_data(0, true)?;
            self.zstream_ended = true;
        }
        // Spurious bytes in the buffer are part of idat_size's accounting already: only what was not read is skipped.
        self.inp.crc_finish(self.idat_size as usize)?;
        Ok(())
    }

    /// The part of `png_read_row` that reads a filtered row of `iwidth` pixels and transforms it in place, leaving the
    /// pixels at `row_buf[1..]`.
    fn read_row(&mut self, iwidth: u32) -> Result<RowInfo, Fail> {
        let mut ri = RowInfo {
            width: iwidth,
            color_type: self.file_color_type,
            bit_depth: self.file_bit_depth,
            channels: self.file_channels,
            pixel_depth: self.file_pixel_depth,
            rowbytes: rowbytes(self.file_pixel_depth as u32, iwidth),
        };
        self.row_buf[0] = 255;
        self.read_idat_data(ri.rowbytes + 1, false)?;
        let filter = self.row_buf[0];
        if filter > 0 {
            if filter >= 5 {
                return Err(png_error());
            }
            read_filter_row(&ri, &mut self.row_buf, &self.prev_row, filter);
        }
        let n = ri.rowbytes + 1;
        self.prev_row[..n].copy_from_slice(&self.row_buf[..n]);
        self.do_read_transformations(&mut ri);
        Ok(ri)
    }

    /// `png_do_read_transformations`, for the transforms `write_png` asks for.
    fn do_read_transformations(&mut self, ri: &mut RowInfo) {
        let row = &mut self.row_buf[1..];
        if self.t.expand {
            if ri.color_type == COLOR_PALETTE {
                do_expand_palette(ri, row, &self.palette, &self.trans_alpha, self.num_trans);
            } else if self.num_trans != 0 && self.t.expand_trns {
                do_expand(ri, row, Some((self.trans_gray, self.trans_rgb)));
            } else {
                do_expand(ri, row, None);
            }
        }
        if self.t.strip_alpha && (ri.color_type == COLOR_RGB_ALPHA || ri.color_type == COLOR_GRAY_ALPHA) {
            do_strip_channel(ri, row);
        }
        if self.t.gamma && self.file_color_type != COLOR_PALETTE {
            do_gamma(ri, row, &self.t);
        }
        if self.t.chop_16 {
            do_chop(ri, row);
        }
    }
}

/// `png_read_filter_row`, the plain C versions.  `row[0]` and `prev[0]` are the filter bytes.
fn read_filter_row(ri: &RowInfo, row_buf: &mut [u8], prev_row: &[u8], filter: u8) {
    let bpp = ((ri.pixel_depth as usize) + 7) >> 3;
    let n = ri.rowbytes;
    let row = &mut row_buf[1..=n];
    let prev = &prev_row[1..=n];
    match filter {
        1 => {
            for i in bpp..n {
                row[i] = row[i].wrapping_add(row[i - bpp]);
            }
        }
        2 => {
            for i in 0..n {
                row[i] = row[i].wrapping_add(prev[i]);
            }
        }
        3 => {
            for i in 0..bpp.min(n) {
                row[i] = row[i].wrapping_add(prev[i] / 2);
            }
            for i in bpp..n {
                row[i] = row[i].wrapping_add(((prev[i] as u32 + row[i - bpp] as u32) / 2) as u8);
            }
        }
        _ => {
            for i in 0..bpp.min(n) {
                row[i] = row[i].wrapping_add(prev[i]);
            }
            for i in bpp..n {
                let c = prev[i - bpp] as i32;
                let mut a = row[i - bpp] as i32;
                let b = prev[i] as i32;
                let p = b - c;
                let mut pc = a - c;
                let mut pa = p.abs();
                let pb = pc.abs();
                pc = (p + pc).abs();
                if pb < pa {
                    pa = pb;
                    a = b;
                }
                if pc < pa {
                    a = c;
                }
                row[i] = row[i].wrapping_add(a as u8);
            }
        }
    }
}

/// `png_do_expand_palette`.  Rows run backward in place, so the wider pixels do not overwrite unread ones.
fn do_expand_palette(ri: &mut RowInfo, row: &mut [u8], palette: &[[u8; 3]], trans: &[u8], num_trans: u32) {
    let w = ri.width as usize;
    let color = |i: usize, c: usize| -> u8 { palette.get(i).map_or(0, |p| p[c]) };
    if ri.bit_depth < 8 {
        let depth = ri.bit_depth as usize;
        let mask = (1u8 << depth) - 1;
        for i in (0..w).rev() {
            let bit = i * depth;
            let v = (row[bit >> 3] >> (8 - depth - (bit & 7))) & mask;
            row[i] = v;
        }
        ri.bit_depth = 8;
        ri.pixel_depth = 8;
        ri.rowbytes = w;
    }
    if num_trans > 0 {
        for i in (0..w).rev() {
            let s = row[i] as usize;
            row[4 * i + 3] = if s >= num_trans as usize { 0xff } else { trans.get(s).copied().unwrap_or(0xff) };
            row[4 * i + 2] = color(s, 2);
            row[4 * i + 1] = color(s, 1);
            row[4 * i] = color(s, 0);
        }
        ri.pixel_depth = 32;
        ri.rowbytes = w * 4;
        ri.color_type = COLOR_RGB_ALPHA;
        ri.channels = 4;
    } else {
        for i in (0..w).rev() {
            let s = row[i] as usize;
            row[3 * i + 2] = color(s, 2);
            row[3 * i + 1] = color(s, 1);
            row[3 * i] = color(s, 0);
        }
        ri.pixel_depth = 24;
        ri.rowbytes = w * 3;
        ri.color_type = COLOR_RGB;
        ri.channels = 3;
    }
}

/// `png_do_expand`.
fn do_expand(ri: &mut RowInfo, row: &mut [u8], trans: Option<(u16, [u16; 3])>) {
    let w = ri.width as usize;
    if ri.color_type == COLOR_GRAY {
        let mut gray = trans.map_or(0, |t| t.0) as u32;
        if ri.bit_depth < 8 {
            let depth = ri.bit_depth as usize;
            let (mask, factor): (u8, u32) = match depth {
                1 => (1, 0xff),
                2 => (3, 0x55),
                _ => (15, 0x11),
            };
            gray = (gray & mask as u32) * factor;
            for i in (0..w).rev() {
                let bit = i * depth;
                let v = (row[bit >> 3] >> (8 - depth - (bit & 7))) & mask;
                row[i] = (v as u32 * factor) as u8;
            }
            ri.bit_depth = 8;
            ri.pixel_depth = 8;
            ri.rowbytes = w;
        }
        if trans.is_some() {
            if ri.bit_depth == 8 {
                let g = gray & 0xff;
                for i in (0..w).rev() {
                    let s = row[i];
                    row[2 * i + 1] = if s as u32 == g { 0 } else { 0xff };
                    row[2 * i] = s;
                }
            } else if ri.bit_depth == 16 {
                let (gh, gl) = (((gray >> 8) & 0xff) as u8, (gray & 0xff) as u8);
                for i in (0..w).rev() {
                    let (h, l) = (row[2 * i], row[2 * i + 1]);
                    let a = if h == gh && l == gl { 0 } else { 0xff };
                    row[4 * i + 3] = a;
                    row[4 * i + 2] = a;
                    row[4 * i + 1] = l;
                    row[4 * i] = h;
                }
            }
            ri.color_type = COLOR_GRAY_ALPHA;
            ri.channels = 2;
            ri.pixel_depth = ri.bit_depth << 1;
            ri.rowbytes = rowbytes(ri.pixel_depth as u32, ri.width);
        }
    } else if ri.color_type == COLOR_RGB {
        if let Some((_, rgb)) = trans {
            if ri.bit_depth == 8 {
                let (r, g, b) = (rgb[0] as u8, rgb[1] as u8, rgb[2] as u8);
                for i in (0..w).rev() {
                    let (s0, s1, s2) = (row[3 * i], row[3 * i + 1], row[3 * i + 2]);
                    row[4 * i + 3] = if s0 == r && s1 == g && s2 == b { 0 } else { 0xff };
                    row[4 * i + 2] = s2;
                    row[4 * i + 1] = s1;
                    row[4 * i] = s0;
                }
            } else if ri.bit_depth == 16 {
                for i in (0..w).rev() {
                    let mut px = [0u8; 6];
                    px.copy_from_slice(&row[6 * i..6 * i + 6]);
                    let same = px[0] == (rgb[0] >> 8) as u8
                        && px[1] == rgb[0] as u8
                        && px[2] == (rgb[1] >> 8) as u8
                        && px[3] == rgb[1] as u8
                        && px[4] == (rgb[2] >> 8) as u8
                        && px[5] == rgb[2] as u8;
                    let a = if same { 0 } else { 0xff };
                    row[8 * i + 7] = a;
                    row[8 * i + 6] = a;
                    row[8 * i..8 * i + 6].copy_from_slice(&px);
                }
            }
            ri.color_type = COLOR_RGB_ALPHA;
            ri.channels = 4;
            ri.pixel_depth = ri.bit_depth << 2;
            ri.rowbytes = rowbytes(ri.pixel_depth as u32, ri.width);
        }
    }
}

/// `png_do_strip_channel(.., at_start = 0)`.
fn do_strip_channel(ri: &mut RowInfo, row: &mut [u8]) {
    let w = ri.width as usize;
    let bytes = (ri.bit_depth / 8) as usize;
    let keep = (ri.channels as usize - 1) * bytes;
    let total = ri.channels as usize * bytes;
    for i in 1..w {
        row.copy_within(i * total..i * total + keep, i * keep);
    }
    ri.pixel_depth = (keep * 8) as u8;
    ri.channels -= 1;
    ri.color_type &= !MASK_ALPHA;
    ri.rowbytes = w * keep;
}

/// `png_do_gamma`.
fn do_gamma(ri: &RowInfo, row: &mut [u8], t: &Transforms) {
    let w = ri.width as usize;
    let gt = &t.gamma_table;
    let shift = t.gamma_shift;
    let g16 = |hi: u8, lo: u8| -> u16 { t.gamma_16_table[((lo as usize) >> shift) * 256 + hi as usize] };
    if ri.bit_depth == 16 && t.gamma_16_table.is_empty() {
        return;
    }
    if ri.bit_depth <= 8 && gt.is_empty() {
        return;
    }
    match ri.color_type {
        COLOR_RGB => {
            if ri.bit_depth == 8 {
                for b in row[..w * 3].iter_mut() {
                    *b = gt[*b as usize];
                }
            } else {
                for s in row[..w * 6].chunks_exact_mut(2) {
                    let v = g16(s[0], s[1]);
                    s[0] = (v >> 8) as u8;
                    s[1] = v as u8;
                }
            }
        }
        COLOR_RGB_ALPHA => {
            if ri.bit_depth == 8 {
                for p in row[..w * 4].chunks_exact_mut(4) {
                    p[0] = gt[p[0] as usize];
                    p[1] = gt[p[1] as usize];
                    p[2] = gt[p[2] as usize];
                }
            } else {
                for p in row[..w * 8].chunks_exact_mut(8) {
                    for s in p[..6].chunks_exact_mut(2) {
                        let v = g16(s[0], s[1]);
                        s[0] = (v >> 8) as u8;
                        s[1] = v as u8;
                    }
                }
            }
        }
        COLOR_GRAY_ALPHA => {
            if ri.bit_depth == 8 {
                for p in row[..w * 2].chunks_exact_mut(2) {
                    p[0] = gt[p[0] as usize];
                }
            } else {
                for p in row[..w * 4].chunks_exact_mut(4) {
                    let v = g16(p[0], p[1]);
                    p[0] = (v >> 8) as u8;
                    p[1] = v as u8;
                }
            }
        }
        COLOR_GRAY => {
            let at = |v: i32| gt[v as usize] as i32;
            if ri.bit_depth == 2 {
                let mut i = 0;
                let mut sp = 0;
                while i < w {
                    let s = row[sp] as i32;
                    let (a, b, c, d) = (s & 0xc0, s & 0x30, s & 0x0c, s & 0x03);
                    row[sp] = ((at(a | (a >> 2) | (a >> 4) | (a >> 6)) & 0xc0)
                        | ((at((b << 2) | b | (b >> 2) | (b >> 4)) >> 2) & 0x30)
                        | ((at((c << 4) | (c << 2) | c | (c >> 2)) >> 4) & 0x0c)
                        | (at((d << 6) | (d << 4) | (d << 2) | d) >> 6)) as u8;
                    sp += 1;
                    i += 4;
                }
            }
            if ri.bit_depth == 4 {
                let mut i = 0;
                let mut sp = 0;
                while i < w {
                    let s = row[sp] as i32;
                    let (msb, lsb) = (s & 0xf0, s & 0x0f);
                    row[sp] = ((at(msb | (msb >> 4)) & 0xf0) | (at((lsb << 4) | lsb) >> 4)) as u8;
                    sp += 1;
                    i += 2;
                }
            } else if ri.bit_depth == 8 {
                for b in row[..w].iter_mut() {
                    *b = gt[*b as usize];
                }
            } else if ri.bit_depth == 16 {
                for s in row[..w * 2].chunks_exact_mut(2) {
                    let v = g16(s[0], s[1]);
                    s[0] = (v >> 8) as u8;
                    s[1] = v as u8;
                }
            }
        }
        _ => {}
    }
}

/// `png_do_chop`.
fn do_chop(ri: &mut RowInfo, row: &mut [u8]) {
    if ri.bit_depth == 16 {
        let n = ri.rowbytes / 2;
        for i in 0..n {
            row[i] = row[2 * i];
        }
        ri.bit_depth = 8;
        ri.pixel_depth = 8 * ri.channels;
        ri.rowbytes = ri.width as usize * ri.channels as usize;
    }
}

//------------------------------------------------------------------------
// write_png
//------------------------------------------------------------------------

/// The engine's values `write_png` reads.
#[derive(Clone, Copy, Debug)]
pub struct PngParams {
    /// `fixedpdfmajorversion`.
    pub pdf_major: i32,
    /// `fixedpdfminorversion`.
    pub pdf_minor: i32,
    /// `fixedimagehicolor`.
    pub image_hicolor: i32,
    /// `fixedimageapplygamma`.
    pub image_apply_gamma: i32,
    /// `fixedgamma`.
    pub gamma: i32,
    /// `fixedimagegamma`.
    pub image_gamma: i32,
    /// `img_colorspace_ref`.
    pub colorspace_ref: i32,
}

/// The result of `write_png`.
#[derive(Clone, Debug)]
pub struct PngOutput {
    /// Everything up to the `pdfflush()` before `write_additional_png_objects`.
    pub ops: Vec<Op>,
    /// `last_png_needs_page_group`: if set, and the engine's `transparent_page_group` is above 0 and not yet written,
    /// it writes `png_page_group_object` and remembers that.
    pub needs_page_group: bool,
}

/// The object `write_additional_png_objects` writes.  `compress_level` is `getpdfcompresslevel()`.
pub fn png_page_group_object(transparent_page_group: i32, compress_level: i32) -> Vec<Op> {
    let mut v = vec![Op::BeginObj { obj: ObjNo::Known(transparent_page_group), level: 2 }];
    let mut s = Vec::new();
    if compress_level == 0 {
        s.extend_from_slice(b"%PTEX Group needed for transparent pngs\n");
    }
    s.extend_from_slice(b"<</Type/Group /S/Transparency /CS/DeviceRGB /I true>>\n");
    v.push(Op::Puts(s));
    v.push(Op::EndObj);
    v
}

/// `spng_getint`.
fn spng_getint(d: &[u8], pos: &mut i64) -> Result<i32, Fail> {
    if *pos < 0 || *pos as u64 + 4 > d.len() as u64 {
        return Err(Fail::new("writepng: reading chunk type failed"));
    }
    let p = *pos as usize;
    *pos += 4;
    Ok(i32::from_be_bytes([d[p], d[p + 1], d[p + 2], d[p + 3]]))
}

fn seek_rel(pos: &mut i64, delta: i64) -> Result<(), Fail> {
    if *pos + delta < 0 {
        return Err(Fail::new("writepng: fseek in PNG file failed"));
    }
    *pos += delta;
    Ok(())
}

/// `copy_png`.
fn copy_png(w: &mut Writer<'_>, d: &[u8], width: u32, color_type: u8, bit_depth: u8) -> Result<(), Fail> {
    const IDAT: i32 = 0x49444154;
    const IEND: i32 = 0x49454E44;
    let mut streamlength: i32 = 0;
    let mut pos: i64 = 8;
    // Chunk lengths past the first IDAT were never checked, and a negative one seeks back: the C can loop for ever.
    let mut guard = 0u64;
    loop {
        guard += 1;
        if guard > d.len() as u64 + 16 {
            return Err(Fail::new("writepng: reading chunk type failed"));
        }
        let len = spng_getint(d, &mut pos)?;
        let ty = spng_getint(d, &mut pos)?;
        match ty {
            IEND => break,
            IDAT => {
                streamlength = streamlength.wrapping_add(len);
                seek_rel(&mut pos, len as i64 + 4)?;
            }
            _ => seek_rel(&mut pos, len as i64 + 4)?,
        }
    }
    w.printf(format_args!(
        "/Length {}\n/Filter/FlateDecode\n/DecodeParms<</Colors {}/Columns {}/BitsPerComponent {}/Predictor 10>>\n>>\nstream\n",
        streamlength,
        if color_type == COLOR_RGB { 3 } else { 1 },
        width as i32,
        bit_depth as i32
    ));
    pos = 8;
    let mut idat = 0;
    loop {
        let len = spng_getint(d, &mut pos)?;
        let ty = spng_getint(d, &mut pos)?;
        match ty {
            IDAT => {
                if idat == 2 {
                    return Err(Fail::new("writepng: IDAT chunk sequence broken"));
                }
                idat = 1;
                if len > 0 {
                    w.copy_input(pos as usize, len as usize);
                    pos += len as i64;
                }
                seek_rel(&mut pos, 4)?;
            }
            IEND => {
                w.op(Op::EndStream);
                break;
            }
            _ => {
                if idat == 1 {
                    idat = 2;
                }
                seek_rel(&mut pos, len as i64 + 4)?;
            }
        }
    }
    Ok(())
}

/// `write_png`.  The engine has called `pdf_begin_dict` and printed the attributes.
pub fn write_png(info: &PngInfo, data: &[u8], p: &PngParams, slots: &mut SlotAlloc) -> Result<PngOutput, Fail> {
    let mut w = Writer::new(slots);
    let mut png_copy = true;
    let (pdf_major, pdf_minor) = (p.pdf_major, p.pdf_minor);
    let mut hicolor = p.image_hicolor;
    if pdf_major == 1 && pdf_minor < 5 {
        hicolor = 0;
    }
    w.puts(b"/Type /XObject\n/Subtype /Image\n");

    let mut t = Transforms::default();
    let num_trans_valid = info.valid & V_TRNS != 0 && info.num_trans != 0;
    if num_trans_valid {
        t.expand = true;
        t.expand_trns = true;
        png_copy = false;
    }
    if pdf_major == 1 && pdf_minor < 4 && info.color_type & MASK_ALPHA != 0 {
        t.strip_alpha = true;
        png_copy = false;
    }
    if info.bit_depth == 16 && hicolor == 0 {
        t.chop_16 = true;
        png_copy = false;
    }
    // gamma support
    let int_file_gamma = if info.valid & V_GAMA != 0 { info.gamma } else { 0 };
    let (mut set_file_gamma, mut set_screen_gamma) = (0, 0);
    if p.image_apply_gamma != 0 {
        // png_get_gAMA: the fixed value through png_float.  Without gAMA: 1000.0 / fixedimagegamma.
        let file: f64 = if info.valid & V_GAMA != 0 {
            info.gamma as f64 * 0.00001
        } else {
            1000.0 / p.image_gamma as f64
        };
        let scrn = convert_gamma_value(p.gamma as f64 / 1000.0)?;
        let file = convert_gamma_value(file)?;
        let scrn = translate_gamma_flags(scrn, true);
        let file = translate_gamma_flags(file, false);
        // png_app_error is a png_error here: only warnings are downgraded on read.
        if file <= 0 || scrn <= 0 {
            return Err(png_error());
        }
        // unsupported_gamma: a warning, and the call does nothing.
        if !((file < LIB_GAMMA_MIN || file > LIB_GAMMA_MAX) || (scrn < LIB_GAMMA_MIN || scrn > LIB_GAMMA_MAX)) {
            set_file_gamma = file;
            set_screen_gamma = scrn;
        }
        png_copy = false;
    }

    // png_read_update_info: png_read_start_row with png_init_read_transformations, then png_read_transform_info.
    let mut palette = info.palette.clone();
    {
        // png_init_gamma_values
        let mut file_gamma = set_file_gamma;
        if file_gamma == 0 {
            file_gamma = info.chunk_gamma;
        }
        if file_gamma == 0 && set_screen_gamma != 0 {
            file_gamma = reciprocal(set_screen_gamma);
        }
        let mut screen_gamma = set_screen_gamma;
        let mut gamma_correction = false;
        if file_gamma > 0 {
            if screen_gamma > 0 {
                gamma_correction = gamma_threshold(file_gamma, screen_gamma);
            } else {
                screen_gamma = reciprocal(file_gamma);
            }
        } else {
            file_gamma = FP_1;
            screen_gamma = FP_1;
        }
        t.gamma = gamma_correction;
        if t.strip_alpha {
            t.expand_trns = false;
        }
        if t.gamma {
            // png_build_gamma_table, bit_depth being the file's.
            let correction = if screen_gamma > 0 { reciprocal2(screen_gamma, file_gamma) } else { FP_1 };
            if info.bit_depth <= 8 {
                t.gamma_table = build_8bit_table(correction);
            } else {
                let sig_bit = if info.color_type & MASK_COLOR != 0 {
                    info.sig_rgb[0].max(info.sig_rgb[1]).max(info.sig_rgb[2])
                } else {
                    info.sig_gray
                } as u32;
                let mut shift = if sig_bit > 0 && sig_bit < 16 { 16 - sig_bit } else { 0 };
                if t.chop_16 && shift < 16 - MAX_GAMMA_8 {
                    shift = 16 - MAX_GAMMA_8;
                }
                if shift > 8 {
                    shift = 8;
                }
                t.gamma_shift = shift;
                t.gamma_16_table = if t.chop_16 {
                    build_16to8_table(shift, reciprocal(correction))
                } else {
                    build_16bit_table(shift, correction)
                };
            }
            if info.color_type == COLOR_PALETTE {
                for c in palette.iter_mut() {
                    c[0] = t.gamma_table[c[0] as usize];
                    c[1] = t.gamma_table[c[1] as usize];
                    c[2] = t.gamma_table[c[2] as usize];
                }
                t.gamma = false;
            }
        }
    }

    // png_read_transform_info
    let mut color_type = info.color_type;
    let mut bit_depth = info.bit_depth;
    if t.expand {
        if color_type == COLOR_PALETTE {
            color_type = if info.num_trans > 0 { COLOR_RGB_ALPHA } else { COLOR_RGB };
            bit_depth = 8;
        } else {
            if info.num_trans != 0 && t.expand_trns {
                color_type |= MASK_ALPHA;
            }
            if bit_depth < 8 {
                bit_depth = 8;
            }
        }
    }
    if bit_depth == 16 && t.chop_16 {
        bit_depth = 8;
    }
    if t.strip_alpha {
        color_type &= !MASK_ALPHA;
    }
    let mut channels: u32 = if color_type == COLOR_PALETTE {
        1
    } else if color_type & MASK_COLOR != 0 {
        3
    } else {
        1
    };
    if color_type & MASK_ALPHA != 0 {
        channels += 1;
    }
    let out_depth = channels * bit_depth as u32;

    w.printf(format_args!(
        "/Width {}\n/Height {}\n/BitsPerComponent {}\n",
        info.width as i32, info.height as i32, bit_depth as i32
    ));
    w.puts(b"/ColorSpace ");

    let copy_ok = png_copy
        && (pdf_major > 1 || pdf_minor > 1)
        && info.interlace_type == 0
        && (color_type == COLOR_GRAY || color_type == COLOR_RGB)
        && p.image_apply_gamma == 0
        && (info.valid & V_GAMA == 0 || int_file_gamma == FP_1)
        && info.valid & (V_CHRM | V_ICCP | V_SBIT | V_SRGB | V_BKGD | V_HIST | V_TRNS | V_SPLT) == 0;
    let mut needs_page_group = false;
    if copy_ok {
        if p.colorspace_ref != 0 {
            w.printf(format_args!("{} 0 R\n", p.colorspace_ref));
        } else if color_type == COLOR_GRAY {
            w.puts(b"/DeviceGray\n");
        } else {
            w.puts(b"/DeviceRGB\n");
        }
        w.op(Op::TexPrint(b" (PNG copy)".to_vec()));
        copy_png(&mut w, data, info.width, color_type, bit_depth)?;
    } else {
        // png_read_start_row: the buffers, and the zstream.
        let max_row = rowbytes(64, (info.width + 7) & !7) + 1 + 8;
        let mut r = Rows {
            inp: Input { d: data, pos: info.idat_data, crc: 0, chunk_name: C_IDAT },
            idat_size: info.idat_size,
            z: Inflate::new(true, 15),
            zin: Vec::new(),
            zin_pos: 0,
            zstream_ended: false,
            row_buf: vec![0u8; max_row],
            prev_row: vec![0u8; max_row],
            t,
            palette: palette.clone(),
            trans_alpha: info.trans_alpha.clone(),
            num_trans: info.num_trans,
            trans_gray: info.trans_gray,
            trans_rgb: info.trans_rgb,
            file_color_type: info.color_type,
            file_bit_depth: info.bit_depth,
            file_channels: info.channels as u8,
            file_pixel_depth: info.pixel_depth as u8,
        };
        // The CRC runs over the chunk name and what follows.
        r.inp.crc = zlib_rs::crc32::crc32(0, &C_IDAT.to_be_bytes());
        match color_type {
            COLOR_PALETTE => {
                let palette_obj = w.new_obj();
                if p.colorspace_ref != 0 {
                    w.printf(format_args!("{} 0 R\n", p.colorspace_ref));
                } else {
                    w.puts(b"[/Indexed /DeviceRGB ");
                    w.printf(format_args!("{} ", palette.len() as i32 - 1));
                    w.num(palette_obj);
                    w.puts(b" 0 R]\n");
                }
                w.op(Op::BeginStream);
                let image = read_image(&mut r, info, out_depth)?;
                w.raw(image);
                w.op(Op::EndStream);
                w.op(Op::BeginDict { obj: ObjNo::Slot(palette_obj), level: 0 });
                w.op(Op::BeginStream);
                let mut pal = Vec::with_capacity(palette.len() * 3);
                for c in &palette {
                    pal.extend_from_slice(c);
                }
                w.raw(pal);
                w.op(Op::EndStream);
            }
            COLOR_GRAY | COLOR_RGB => {
                if p.colorspace_ref != 0 {
                    w.printf(format_args!("{} 0 R\n", p.colorspace_ref));
                } else if color_type == COLOR_GRAY {
                    w.puts(b"/DeviceGray\n");
                } else {
                    w.puts(b"/DeviceRGB\n");
                }
                w.op(Op::BeginStream);
                let image = read_image(&mut r, info, out_depth)?;
                w.raw(image);
                w.op(Op::EndStream);
            }
            _ => {
                let device_rgb = color_type == COLOR_RGB_ALPHA;
                let with_alpha = pdf_minor >= 4 || pdf_major > 1;
                if p.colorspace_ref != 0 {
                    w.printf(format_args!("{} 0 R\n", p.colorspace_ref));
                } else if device_rgb {
                    w.puts(b"/DeviceRGB\n");
                } else {
                    w.puts(b"/DeviceGray\n");
                }
                if with_alpha {
                    let smask_obj = w.new_obj();
                    w.puts(b"/SMask ");
                    w.num(smask_obj);
                    w.puts(b" 0 R\n");
                    w.op(Op::BeginStream);
                    let image = read_image(&mut r, info, out_depth)?;
                    let (px, smask) = split_alpha(&image, device_rgb, bit_depth == 16);
                    w.raw(px);
                    w.op(Op::EndStream);
                    w.op(Op::Flush);
                    w.op(Op::BeginDict { obj: ObjNo::Slot(smask_obj), level: 0 });
                    w.puts(b"/Type /XObject\n/Subtype /Image\n");
                    w.printf(format_args!(
                        "/Width {}\n/Height {}\n/BitsPerComponent {}\n",
                        info.width as i32,
                        info.height as i32,
                        if bit_depth == 16 { 8 } else { bit_depth as i32 }
                    ));
                    w.puts(b"/ColorSpace /DeviceGray\n");
                    w.op(Op::BeginStream);
                    w.raw(smask);
                    w.op(Op::EndStream);
                    needs_page_group = true;
                } else {
                    // write_png_gray or write_png_rgb on a row that still has the alpha bytes: a palette or
                    // simple-transparency image below PDF 1.4, where png_set_strip_alpha was not called.
                    w.op(Op::BeginStream);
                    let image = read_image(&mut r, info, out_depth)?;
                    w.raw(image);
                    w.op(Op::EndStream);
                }
            }
        }
    }
    w.op(Op::Flush);
    Ok(PngOutput { ops: w.ops, needs_page_group })
}

/// The image part of `write_noninterlaced` and `write_interlaced`, all rows in one buffer.
fn read_image(r: &mut Rows<'_>, info: &PngInfo, out_depth: u32) -> Result<Vec<u8>, Fail> {
    let out_rowbytes = rowbytes(out_depth, info.width);
    let h = info.height as usize;
    let mut image = vec![0u8; out_rowbytes * h];
    if info.interlace_type == 0 {
        let file_row = rowbytes(info.pixel_depth, info.width);
        r.prev_row[..file_row + 1].fill(0);
        let used_bits = (info.width as usize * out_depth as usize) % 8;
        let keep_mask: u8 = if used_bits == 0 { 0xff } else { !(0xffu8 >> used_bits) };
        for y in 0..h {
            let ri = r.read_row(info.width)?;
            let dst = &mut image[y * out_rowbytes..(y + 1) * out_rowbytes];
            dst.copy_from_slice(&r.row_buf[1..=ri.rowbytes]);
            // png_combine_row keeps the bits past the last pixel from what the row buffer held before: malloc's
            // leftovers in the C.  Zero here.
            dst[out_rowbytes - 1] &= keep_mask;
        }
    } else {
        const PASS_START: [u32; 7] = [0, 4, 0, 2, 0, 1, 0];
        const PASS_INC: [u32; 7] = [8, 8, 4, 4, 2, 2, 1];
        const PASS_YSTART: [u32; 7] = [0, 0, 4, 0, 2, 0, 1];
        const PASS_YINC: [u32; 7] = [8, 8, 8, 4, 4, 2, 2];
        let file_row = rowbytes(info.pixel_depth, info.width);
        for pass in 0..7 {
            r.prev_row[..file_row + 1].fill(0);
            let (xs, xi, ys, yi) = (PASS_START[pass], PASS_INC[pass], PASS_YSTART[pass], PASS_YINC[pass]);
            let min_width = match pass {
                1 => 5,
                3 => 3,
                5 => 2,
                _ => 1,
            };
            if info.width < min_width {
                continue;
            }
            let iwidth = (info.width + xi - 1 - xs) / xi;
            let mut y = ys as usize;
            while y < h {
                let ri = r.read_row(iwidth)?;
                let dst = &mut image[y * out_rowbytes..(y + 1) * out_rowbytes];
                let pd = ri.pixel_depth as usize;
                if pd >= 8 {
                    let n = pd / 8;
                    for i in 0..iwidth as usize {
                        let x = xs as usize + i * xi as usize;
                        dst[x * n..(x + 1) * n].copy_from_slice(&r.row_buf[1 + i * n..1 + (i + 1) * n]);
                    }
                } else {
                    let mask = ((1u16 << pd) - 1) as u8;
                    for i in 0..iwidth as usize {
                        let sb = i * pd;
                        let v = (r.row_buf[1 + (sb >> 3)] >> (8 - pd - (sb & 7))) & mask;
                        let db = (xs as usize + i * xi as usize) * pd;
                        let sh = 8 - pd - (db & 7);
                        dst[db >> 3] = (dst[db >> 3] & !(mask << sh)) | (v << sh);
                    }
                }
                y += yi as usize;
            }
        }
    }
    r.finish_idat()?;
    Ok(image)
}

/// `write_gray_pixel_8`, `write_gray_pixel_16`, `write_rgb_pixel_8`, `write_rgb_pixel_16`, and the loop writing the
/// SMask, which keeps the high byte only of a 16-bit alpha.
fn split_alpha(image: &[u8], rgb: bool, wide: bool) -> (Vec<u8>, Vec<u8>) {
    let (color, alpha) = match (rgb, wide) {
        (false, false) => (1, 1),
        (false, true) => (2, 2),
        (true, false) => (3, 1),
        (true, true) => (6, 2),
    };
    let total = color + alpha;
    let n = image.len() / total;
    let mut px = Vec::with_capacity(n * color);
    let mut smask = Vec::with_capacity(n);
    for p in image.chunks_exact(total) {
        px.extend_from_slice(&p[..color]);
        smask.push(p[color]);
    }
    (px, smask)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pow_agrees_with_libm_over_the_inputs_the_tables_use() {
        // The tables take floor(255 * pow(x, g) + .5): one ulp from libm does not move them.  Compare those results.
        for g in [0.45455f64, 0.5, 1.0 / 2.2, 2.2, 1.8, 0.7, 3.3, 10.0, 0.01] {
            for v in 1..255 {
                let x = v as f64 / 255.0;
                let a = (255.0 * pow(x, g) + 0.5).floor();
                let b = (255.0 * x.powf(g) + 0.5).floor();
                assert_eq!(a, b, "x={x} g={g}");
            }
            for v in (1..65535).step_by(97) {
                let x = v as f64 / 65535.0;
                let a = (65535.0 * pow(x, g) + 0.5).floor();
                let b = (65535.0 * x.powf(g) + 0.5).floor();
                assert_eq!(a, b, "x={x} g={g}");
            }
        }
    }

    #[test]
    fn rounded_pow_is_the_exact_rounding() {
        for g in [0.45455f64, 2.2, 0.7] {
            for v in (1..65535).step_by(61) {
                let x = v as f64 / 65535.0;
                assert_eq!(rounded_pow(65535.0, x, g), (65535.0 * pow(x, g) + 0.5).floor(), "x={x} g={g}");
            }
        }
        assert_eq!(rounded_pow(255.0, 0.0, 2.0), 0.0);
        assert_eq!(rounded_pow(255.0, 1.0, 2.0), 255.0);
    }

    #[test]
    fn pow_edges() {
        assert_eq!(pow(0.0, 2.0), 0.0);
        assert_eq!(pow(1.0, 2.0), 1.0);
        assert_eq!(pow(0.5, 0.0), 1.0);
        assert!(pow(0.5, 21474.0) == 0.0);
        assert!((pow(0.25, 0.5) - 0.5).abs() < 1e-15);
    }

    #[test]
    fn chunk_names() {
        assert!(check_chunk_name(C_IHDR));
        assert!(check_chunk_name(C_GAMA));
        assert!(!check_chunk_name(u32::from_be_bytes(*b"I2DR")));
        assert!(!check_chunk_name(u32::from_be_bytes([0x49, 0x48, 0x20, 0x52])));
    }
}
