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
//!
//! One that a run can see: the C holds a row of the image at a time, and here the image is one buffer until it is
//! deflated whole (docs/PORTING.md section 18).  A header that says more than memory holds still fails as it does there,
//! as only the rows the file has are held (`read_image`); but an image whose rows are all there and more than memory
//! holds ends with `xmalloc`'s message and status 1 where the C goes on.

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

/// The image data: the IDAT chunks and the zstream that inflates them.
struct Idat<'a> {
    inp: Input<'a>,
    idat_size: u32,
    z: Inflate,
    zin: Vec<u8>,
    zin_pos: usize,
    zstream_ended: bool,
}

struct Rows<'a> {
    idat: Idat<'a>,
    row_buf: Vec<u8>,
    prev_row: Vec<u8>,
    /// From `PngParams::kernels`.
    kernels: Kernels,
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

impl Idat<'_> {
    /// `png_read_IDAT_data`, filling `output`.  `output` of `None` is the check after the last row.
    fn read_idat_data(&mut self, mut output: Option<&mut [u8]>) -> Result<(), Fail> {
        let check_only = output.is_none();
        let mut filled = 0usize;
        let mut tmp = [0u8; 1024];
        let mut avail_out = output.as_ref().map_or(0, |o| o.len());
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
            let out = match output.as_deref_mut() {
                Some(o) => &mut o[filled..],
                None => &mut tmp[..],
            };
            let ret = self.z.decompress(&self.zin[self.zin_pos..], out, InflateFlush::NoFlush);
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
            self.read_idat_data(None)?;
            self.zstream_ended = true;
        }
        // Spurious bytes in the buffer are part of idat_size's accounting already: only what was not read is skipped.
        self.inp.crc_finish(self.idat_size as usize)?;
        Ok(())
    }
}

impl Rows<'_> {
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
        let n = ri.rowbytes + 1;
        self.row_buf[0] = 255;
        self.idat.read_idat_data(Some(&mut self.row_buf[..n]))?;
        let filter = self.row_buf[0];
        if filter > 0 {
            if filter >= 5 {
                return Err(png_error());
            }
            (self.kernels.unfilter)(&mut self.row_buf[..n], &self.prev_row[..n], ((ri.pixel_depth as usize) + 7) >> 3);
        }
        self.prev_row[..n].copy_from_slice(&self.row_buf[..n]);
        self.do_read_transformations(&mut ri);
        Ok(ri)
    }

    /// Where `read_plain_rows` reads rows `width` pixels wide, if it may: if `do_read_transformations` leaves every
    /// row, of whole bytes, as the filters gave it, and the memory can be had.  Slots of `read_row`'s size, for the row
    /// above and a group of up to 32 rows, in up to 512 KiB: runs of the bands of `unfilter_rows_avx2`, of 4 or 8 rows,
    /// and near the cache.  And zeros, which are the row above the first.
    fn group_buffer(&self, width: u32, height: u32) -> Option<Vec<u8>> {
        let alpha = self.file_color_type == COLOR_RGB_ALPHA || self.file_color_type == COLOR_GRAY_ALPHA;
        if self.file_pixel_depth < 8
            || self.t.expand
            || (self.t.strip_alpha && alpha)
            || (self.t.gamma && self.file_color_type != COLOR_PALETTE)
            || self.t.chop_16
        {
            return None;
        }
        let stride = rowbytes(self.file_pixel_depth as u32, width) + 1;
        let group = ((512 << 10) / stride).clamp(1, 32).min(height as usize);
        let mut buf = Vec::new();
        buf.try_reserve_exact(stride * (group + 1)).ok()?;
        buf.resize(stride * (group + 1), 0);
        Some(buf)
    }

    /// `read_rows` for an image that `group_buffer` gives `buf` for: `read_row`'s bytes, by other means.  Each row is
    /// read into a slot of `buf` as `read_row` reads it into `row_buf`, a filter byte and the row, by the same request
    /// to the zstream, so a short or broken stream or a bad filter byte fails at the same row, and `sink` reserves for
    /// it there.  Then a group of rows is unfiltered at once, each under the one before it, which lets
    /// `unfilter_rows_avx2` take Paeth rows several at a step, and they go to `sink` in order.
    fn read_plain_rows(
        &mut self,
        width: u32,
        height: u32,
        mut buf: Vec<u8>,
        sink: &mut impl RowSink,
    ) -> Result<(), Fail> {
        let stride = rowbytes(self.file_pixel_depth as u32, width) + 1;
        let group = buf.len() / stride - 1;
        let bpp = self.file_pixel_depth as usize >> 3;
        let h = height as usize;
        let mut y = 0;
        while y < h {
            let n = (h - y).min(group);
            for k in 1..=n {
                let slot = &mut buf[k * stride..(k + 1) * stride];
                slot[0] = 255;
                self.idat.read_idat_data(Some(slot))?;
                if slot[0] >= 5 {
                    return Err(png_error());
                }
                sink.reserve(y + k - 1)?;
            }
            let (above, rows) = buf.split_at_mut(stride);
            (self.kernels.unfilter)(&mut rows[..n * stride], above, bpp);
            for row in rows[..n * stride].chunks_exact(stride) {
                sink.take(&row[1..]);
            }
            buf.copy_within(n * stride..(n + 1) * stride, 0);
            y += n;
        }
        Ok(())
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

/// The code the rows of a picture go through, as the CPU allows: [`Kernels::PLAIN`], or the same compiled for a CPU
/// feature, `unfilter_rows_avx2` and `split_row_avx2`.  Code compiled for a feature may only be called in `unsafe`,
/// once the feature is known to be there, and this crate has no `unsafe`: so the engine makes the choice, in
/// `platform::png_kernels`, and hands it over in `PngParams::kernels`.  Each gives the same bytes:
/// `tests::filters_are_the_c_ones`, `tests::split_alpha_takes_the_first_byte_of_alpha` and `tests/png_kernels.rs`.
#[derive(Clone, Copy, Debug)]
pub struct Kernels {
    /// Undoes the row filters of a group of rows, as [`unfilter_rows`] does.
    pub unfilter: fn(rows: &mut [u8], prev: &[u8], bpp: usize),
    /// Splits a row of pixels with alpha into their colour and their alpha, as [`split_row`] does.
    pub split: fn(row: &[u8], color: &mut [u8], alpha: &mut [u8], rgb: bool, wide: bool),
}

impl Kernels {
    /// What every CPU runs.
    pub const PLAIN: Kernels = Kernels { unfilter: unfilter_rows, split: split_row };
}

/// `png_read_filter_row` for each row of `rows`, which holds whole rows, each in the slot `png_read_row` reads it into:
/// its filter byte, below 5, then its bytes, as many as `prev` has after its first.  `prev` is the row above the first,
/// unfiltered, in a slot of the same kind, and `bpp` the bytes to a pixel, 1 for pixels of fewer bits.  Each row is
/// unfiltered in place, under the one before it.  A pixel's bytes depend only on those of the pixels before it, so what
/// C does byte by byte is done here in the lanes of a pixel, which the compiler turns into vector operations: a third
/// of the time on a Paeth row of four bytes to the pixel.
pub fn unfilter_rows(rows: &mut [u8], prev: &[u8], bpp: usize) {
    let mut above = prev;
    for row in rows.chunks_exact_mut(prev.len()) {
        match bpp {
            1 => filter_row::<1>(row, above),
            2 => filter_row::<2>(row, above),
            3 => filter_row::<3>(row, above),
            4 => filter_row::<4>(row, above),
            6 => filter_row::<6>(row, above),
            _ => filter_row::<8>(row, above),
        }
        above = row;
    }
}

/// [`unfilter_rows`] compiled for AVX2, which only a CPU that has it may call.  Three operands, so none of the register
/// copies that SSE2's two take, and `vpblendvb` for Paeth's choices.  Paeth is serial along a row, so a run of Paeth
/// rows of pixels of 1, 2 or 4 bytes goes through `band`, several rows at a step.
///
/// # Safety
/// The CPU has AVX2.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
pub fn unfilter_rows_avx2(rows: &mut [u8], prev: &[u8], bpp: usize) {
    let stride = prev.len();
    let height = band::height(bpp);
    let mut above = prev;
    let mut rest = rows;
    while rest.len() >= stride {
        if height > 0 && rest.len() >= height * stride && (0..height).all(|k| rest[k * stride] == 4) {
            let (rows, tail) = std::mem::take(&mut rest).split_at_mut(height * stride);
            match bpp {
                1 => band::paeth_1(rows, above),
                2 => band::paeth_2(rows, above),
                _ => band::paeth_4(rows, above),
            }
            let rows: &[u8] = rows;
            above = &rows[(height - 1) * stride..];
            rest = tail;
        } else {
            let (row, tail) = std::mem::take(&mut rest).split_at_mut(stride);
            match bpp {
                1 => filter_row_avx2::<1>(row, above),
                2 => filter_row_avx2::<2>(row, above),
                3 => filter_row_avx2::<3>(row, above),
                4 => filter_row_avx2::<4>(row, above),
                6 => filter_row_avx2::<6>(row, above),
                _ => filter_row_avx2::<8>(row, above),
            }
            above = row;
            rest = tail;
        }
    }
}

/// One row of [`unfilter_rows`].  A function of its own for each width of pixel: inlined into the loop over the rows, a
/// pixel's lanes are no longer put in vector registers.
#[inline(never)]
fn filter_row<const BPP: usize>(row: &mut [u8], prev: &[u8]) {
    filter_row_in::<BPP>(row, prev)
}

/// `filter_row` compiled for AVX2.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
#[inline(never)]
fn filter_row_avx2<const BPP: usize>(row: &mut [u8], prev: &[u8]) {
    filter_row_in::<BPP>(row, prev)
}

/// `row[0]` and `prev[0]` are the filter bytes.
#[inline(always)]
fn filter_row_in<const BPP: usize>(row: &mut [u8], prev: &[u8]) {
    let filter = row[0];
    let (row, prev) = (&mut row[1..], &prev[1..]);
    match filter {
        0 => {}
        2 => {
            for (r, p) in row.iter_mut().zip(prev) {
                *r = r.wrapping_add(*p);
            }
        }
        _ => filter_lanes::<BPP>(row, prev, filter),
    }
}

/// Filters 1, 3 and 4, for pixels of `BPP` bytes.  `a` is the pixel to the left and `c` the one above that, zero for the first pixel.
#[inline(always)]
fn filter_lanes<const BPP: usize>(row: &mut [u8], prev: &[u8], filter: u8) {
    let mut a = [0i16; BPP];
    let mut c = [0i16; BPP];
    match filter {
        1 => {
            for r in row.chunks_exact_mut(BPP) {
                for i in 0..BPP {
                    let v = (r[i] as i16 + a[i]) & 0xff;
                    r[i] = v as u8;
                    a[i] = v;
                }
            }
        }
        3 => {
            for (r, p) in row.chunks_exact_mut(BPP).zip(prev.chunks_exact(BPP)) {
                for i in 0..BPP {
                    let v = (r[i] as i16 + (p[i] as i16 + a[i]) / 2) & 0xff;
                    r[i] = v as u8;
                    a[i] = v;
                }
            }
        }
        _ => {
            // Paeth as stb_image has it, which picks what libpng's three distances pick for every a, b and c
            // (`tests::paeth_is_libpngs_for_every_input`): with lo and hi the smaller and larger of a and b, the
            // predictor is hi where 3c <= lo + hi + lo, lo where lo + hi + hi <= 3c, and c otherwise.  Half the
            // operations of the distances, and nothing in it but a min, a max, two compares and two selects.
            for (r, p) in row.chunks_exact_mut(BPP).zip(prev.chunks_exact(BPP)) {
                for i in 0..BPP {
                    let b = p[i] as i16;
                    let v = (r[i] as i16 + paeth(a[i], b, c[i])) & 0xff;
                    r[i] = v as u8;
                    a[i] = v;
                    c[i] = b;
                }
            }
        }
    }
}

/// The Paeth predictor in stb_image's form: see `filter_lanes`.
#[inline(always)]
fn paeth(a: i16, b: i16, c: i16) -> i16 {
    let thresh = c * 3 - (a + b);
    let lo = a.min(b);
    let hi = a.max(b);
    let t0 = if hi <= thresh { lo } else { c };
    if thresh <= lo { hi } else { t0 }
}

/// Runs of Paeth rows for `unfilter_rows_avx2`, several rows at a step.  Row `k` of a band runs `k` pixels behind the
/// first, in 16-bit lanes of a register, one for each byte of a pixel, after those of row `k - 1`.  So at each step a
/// lane's `a` is its own last byte, its `b` the last byte of row `k - 1`'s lane for the same byte, which has just done
/// the pixel above, or for the first row the pixel of the row above the band, and its `c` its `b` of the step before:
/// all of it in registers, and each step does one pixel of each row for the work of one.  A lane whose row has not
/// started yet takes zeros and gives zeros, which are what `a` and `c` are at the start of a row.  A lane whose row has
/// ended gives what no row stores.
///
/// The arithmetic is `paeth`'s in each lane.  Safe code: the intrinsics that read no memory are safe in a function
/// compiled for their feature, and the rows are slices.
#[cfg(target_arch = "x86_64")]
mod band {
    use core::arch::x86_64::*;

    /// How many rows a band has, for pixels of `bpp` bytes, or 0 where there is no band.
    pub(super) fn height(bpp: usize) -> usize {
        match bpp {
            1 => 8,
            2 | 4 => 4,
            _ => 0,
        }
    }

    /// The rows of a band, each as its pixels, and the row above it.
    #[allow(clippy::type_complexity)]
    fn rows<'a, const BPP: usize, const K: usize>(
        band: &'a mut [u8],
        above: &'a [u8],
    ) -> ([&'a mut [[u8; BPP]]; K], &'a [[u8; BPP]]) {
        let mut it = band.chunks_exact_mut(above.len());
        (std::array::from_fn(|_| it.next().unwrap()[1..].as_chunks_mut().0), above[1..].as_chunks().0)
    }

    /// `paeth` in each 16-bit lane, and the byte it makes of `x`.
    #[target_feature(enable = "avx2")]
    #[inline]
    fn step(a: __m128i, b: __m128i, c: __m128i, x: __m128i) -> __m128i {
        let thresh = _mm_sub_epi16(_mm_add_epi16(c, _mm_add_epi16(c, c)), _mm_add_epi16(a, b));
        let lo = _mm_min_epi16(a, b);
        let hi = _mm_max_epi16(a, b);
        let t0 = _mm_blendv_epi8(lo, c, _mm_cmpgt_epi16(hi, thresh));
        let pred = _mm_blendv_epi8(hi, t0, _mm_cmpgt_epi16(thresh, lo));
        _mm_and_si128(_mm_add_epi16(x, pred), _mm_set1_epi16(0xff))
    }

    /// `step` in the 16 lanes of a 256-bit register.
    #[target_feature(enable = "avx2")]
    #[inline]
    fn step_wide(a: __m256i, b: __m256i, c: __m256i, x: __m256i) -> __m256i {
        let thresh = _mm256_sub_epi16(_mm256_add_epi16(c, _mm256_add_epi16(c, c)), _mm256_add_epi16(a, b));
        let lo = _mm256_min_epi16(a, b);
        let hi = _mm256_max_epi16(a, b);
        let t0 = _mm256_blendv_epi8(lo, c, _mm256_cmpgt_epi16(hi, thresh));
        let pred = _mm256_blendv_epi8(hi, t0, _mm256_cmpgt_epi16(thresh, lo));
        _mm256_and_si256(_mm256_add_epi16(x, pred), _mm256_set1_epi16(0xff))
    }

    /// The `b` of each lane of 8, for pixels of `BPP` bytes: each row's from the row before, the first's from `top`.
    #[target_feature(enable = "avx2")]
    #[inline]
    fn above<const BPP: usize>(out: __m128i, top: [u8; BPP]) -> __m128i {
        let mut t = 0i32;
        for (i, &v) in top.iter().enumerate() {
            t |= (v as i32) << (16 * i);
        }
        let below = if BPP == 1 { _mm_slli_si128::<2>(out) } else { _mm_slli_si128::<4>(out) };
        _mm_or_si128(below, _mm_cvtsi32_si128(t))
    }

    /// `above` for 16 lanes and pixels of 4 bytes: a row's lanes are a 64-bit quarter, and one permutation moves them
    /// up.
    #[target_feature(enable = "avx2")]
    #[inline]
    fn above_wide(out: __m256i, top: [u8; 4]) -> __m256i {
        let up = _mm256_permute4x64_epi64::<0b10_01_00_00>(out);
        _mm256_blend_epi32::<0b0000_0011>(up, _mm256_cvtepu8_epi16(_mm_cvtsi32_si128(i32::from_le_bytes(top))))
    }

    /// Step `t` of a band of 8 lanes where some row has no pixel, at the start of the band or at its end.
    #[target_feature(enable = "avx2")]
    #[inline]
    fn edge<const BPP: usize, const K: usize>(
        rows: &mut [&mut [[u8; BPP]]; K],
        top: &[[u8; BPP]],
        t: usize,
        out: &mut __m128i,
        c: &mut __m128i,
    ) {
        let w = top.len();
        let mut x = [0u8; 8];
        for (k, row) in rows.iter().enumerate() {
            if t >= k && t - k < w {
                x[k * BPP..][..BPP].copy_from_slice(&row[t - k]);
            }
        }
        let b = above::<BPP>(*out, if t < w { top[t] } else { [0; BPP] });
        *out = step(*out, b, *c, _mm_cvtepu8_epi16(_mm_cvtsi64_si128(i64::from_le_bytes(x))));
        *c = b;
        let got = _mm_cvtsi128_si64(_mm_packus_epi16(*out, *out)).to_le_bytes();
        for (k, row) in rows.iter_mut().enumerate() {
            if t >= k && t - k < w {
                row[t - k].copy_from_slice(&got[k * BPP..][..BPP]);
            }
        }
    }

    /// `edge` for 16 lanes and pixels of 4 bytes.
    #[target_feature(enable = "avx2")]
    #[inline]
    fn edge_wide(rows: &mut [&mut [[u8; 4]]; 4], top: &[[u8; 4]], t: usize, out: &mut __m256i, c: &mut __m256i) {
        let w = top.len();
        let mut x = [0u8; 16];
        for (k, row) in rows.iter().enumerate() {
            if t >= k && t - k < w {
                x[k * 4..][..4].copy_from_slice(&row[t - k]);
            }
        }
        let b = above_wide(*out, if t < w { top[t] } else { [0; 4] });
        let lo = i64::from_le_bytes(x[..8].try_into().unwrap());
        let hi = i64::from_le_bytes(x[8..].try_into().unwrap());
        *out = step_wide(*out, b, *c, _mm256_cvtepu8_epi16(_mm_set_epi64x(hi, lo)));
        *c = b;
        let got = packed_wide(*out);
        let got = [_mm_cvtsi128_si64(got).to_le_bytes(), _mm_extract_epi64::<1>(got).to_le_bytes()].concat();
        for (k, row) in rows.iter_mut().enumerate() {
            if t >= k && t - k < w {
                row[t - k].copy_from_slice(&got[k * 4..][..4]);
            }
        }
    }

    /// The bytes of 16 lanes, in order.
    #[target_feature(enable = "avx2")]
    #[inline]
    fn packed_wide(out: __m256i) -> __m128i {
        // packus packs within each 128-bit half: rows 0 and 1 in the first quarter, 2 and 3 in the third.
        _mm256_castsi256_si128(_mm256_permute4x64_epi64::<0b00_00_10_00>(_mm256_packus_epi16(out, out)))
    }

    /// A band of 8 rows of pixels of 1 byte.
    #[target_feature(enable = "avx2")]
    pub(super) fn paeth_1(band: &mut [u8], above_band: &[u8]) {
        const K: usize = 8;
        let (mut rows, top) = rows::<1, K>(band, above_band);
        let w = top.len();
        let (mut out, mut c) = (_mm_setzero_si128(), _mm_setzero_si128());
        for t in 0..K - 1 {
            edge(&mut rows, top, t, &mut out, &mut c);
        }
        if w > K - 1 {
            // Every row has a pixel at these steps: row k's from K - 1 - k on.
            let m = w - (K - 1);
            let mut k = 0;
            let [r0, r1, r2, r3, r4, r5, r6, r7] = rows.each_mut().map(|r| {
                k += 1;
                &mut r[K - k..][..m]
            });
            let top = &top[K - 1..][..m];
            for j in 0..m {
                let x = _mm_cvtsi32_si128(r0[j][0] as i32);
                let x = _mm_insert_epi8::<1>(x, r1[j][0] as i32);
                let x = _mm_insert_epi8::<2>(x, r2[j][0] as i32);
                let x = _mm_insert_epi8::<3>(x, r3[j][0] as i32);
                let x = _mm_insert_epi8::<4>(x, r4[j][0] as i32);
                let x = _mm_insert_epi8::<5>(x, r5[j][0] as i32);
                let x = _mm_insert_epi8::<6>(x, r6[j][0] as i32);
                let x = _mm_insert_epi8::<7>(x, r7[j][0] as i32);
                let b = above::<1>(out, top[j]);
                out = step(out, b, c, _mm_cvtepu8_epi16(x));
                c = b;
                let p = _mm_packus_epi16(out, out);
                r0[j][0] = _mm_extract_epi8::<0>(p) as u8;
                r1[j][0] = _mm_extract_epi8::<1>(p) as u8;
                r2[j][0] = _mm_extract_epi8::<2>(p) as u8;
                r3[j][0] = _mm_extract_epi8::<3>(p) as u8;
                r4[j][0] = _mm_extract_epi8::<4>(p) as u8;
                r5[j][0] = _mm_extract_epi8::<5>(p) as u8;
                r6[j][0] = _mm_extract_epi8::<6>(p) as u8;
                r7[j][0] = _mm_extract_epi8::<7>(p) as u8;
            }
        }
        for t in w.max(K - 1)..w + K - 1 {
            edge(&mut rows, top, t, &mut out, &mut c);
        }
    }

    /// A band of 4 rows of pixels of 2 bytes.
    #[target_feature(enable = "avx2")]
    pub(super) fn paeth_2(band: &mut [u8], above_band: &[u8]) {
        const K: usize = 4;
        let (mut rows, top) = rows::<2, K>(band, above_band);
        let w = top.len();
        let (mut out, mut c) = (_mm_setzero_si128(), _mm_setzero_si128());
        for t in 0..K - 1 {
            edge(&mut rows, top, t, &mut out, &mut c);
        }
        if w > K - 1 {
            let m = w - (K - 1);
            let mut k = 0;
            let [r0, r1, r2, r3] = rows.each_mut().map(|r| {
                k += 1;
                &mut r[K - k..][..m]
            });
            let top = &top[K - 1..][..m];
            for j in 0..m {
                let x = _mm_cvtsi32_si128(u16::from_le_bytes(r0[j]) as i32);
                let x = _mm_insert_epi16::<1>(x, u16::from_le_bytes(r1[j]) as i32);
                let x = _mm_insert_epi16::<2>(x, u16::from_le_bytes(r2[j]) as i32);
                let x = _mm_insert_epi16::<3>(x, u16::from_le_bytes(r3[j]) as i32);
                let b = above::<2>(out, top[j]);
                out = step(out, b, c, _mm_cvtepu8_epi16(x));
                c = b;
                let p = _mm_packus_epi16(out, out);
                r0[j] = (_mm_extract_epi16::<0>(p) as u16).to_le_bytes();
                r1[j] = (_mm_extract_epi16::<1>(p) as u16).to_le_bytes();
                r2[j] = (_mm_extract_epi16::<2>(p) as u16).to_le_bytes();
                r3[j] = (_mm_extract_epi16::<3>(p) as u16).to_le_bytes();
            }
        }
        for t in w.max(K - 1)..w + K - 1 {
            edge(&mut rows, top, t, &mut out, &mut c);
        }
    }

    /// A band of 4 rows of pixels of 4 bytes, in the four quarters of a 256-bit register.
    #[target_feature(enable = "avx2")]
    pub(super) fn paeth_4(band: &mut [u8], above_band: &[u8]) {
        const K: usize = 4;
        let (mut rows, top) = rows::<4, K>(band, above_band);
        let w = top.len();
        let (mut out, mut c) = (_mm256_setzero_si256(), _mm256_setzero_si256());
        for t in 0..K - 1 {
            edge_wide(&mut rows, top, t, &mut out, &mut c);
        }
        if w > K - 1 {
            let m = w - (K - 1);
            let mut k = 0;
            let [r0, r1, r2, r3] = rows.each_mut().map(|r| {
                k += 1;
                &mut r[K - k..][..m]
            });
            let top = &top[K - 1..][..m];
            for j in 0..m {
                let x = _mm_cvtsi32_si128(i32::from_le_bytes(r0[j]));
                let x = _mm_insert_epi32::<1>(x, i32::from_le_bytes(r1[j]));
                let x = _mm_insert_epi32::<2>(x, i32::from_le_bytes(r2[j]));
                let x = _mm_insert_epi32::<3>(x, i32::from_le_bytes(r3[j]));
                let b = above_wide(out, top[j]);
                out = step_wide(out, b, c, _mm256_cvtepu8_epi16(x));
                c = b;
                let p = packed_wide(out);
                r0[j] = _mm_cvtsi128_si32(p).to_le_bytes();
                r1[j] = _mm_extract_epi32::<1>(p).to_le_bytes();
                r2[j] = _mm_extract_epi32::<2>(p).to_le_bytes();
                r3[j] = _mm_extract_epi32::<3>(p).to_le_bytes();
            }
        }
        for t in w.max(K - 1)..w + K - 1 {
            edge_wide(&mut rows, top, t, &mut out, &mut c);
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
    /// [`Kernels::PLAIN`], or the same compiled for a feature that the CPU has.
    pub kernels: Kernels,
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
            idat: Idat {
                inp: Input { d: data, pos: info.idat_data, crc: 0, chunk_name: C_IDAT },
                idat_size: info.idat_size,
                z: Inflate::new(true, 15),
                zin: Vec::new(),
                zin_pos: 0,
                zstream_ended: false,
            },
            row_buf: vec![0u8; max_row],
            prev_row: vec![0u8; max_row],
            kernels: p.kernels,
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
        r.idat.inp.crc = zlib_rs::crc32::crc32(0, &C_IDAT.to_be_bytes());
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
                    // write_png_gray_alpha and write_png_rgb_alpha allocate the SMask before they read a row: a half or a
                    // quarter of each row's bytes, for every row.  Where malloc refuses that, xmalloc ends the run.
                    let smask_size = rowbytes(out_depth, info.width) / if device_rgb { 4 } else { 2 } * info.height as usize;
                    let mut smask = Vec::new();
                    if smask.try_reserve_exact(smask_size).is_err() {
                        return Err(Fail::xmalloc(smask_size));
                    }
                    w.op(Op::BeginStream);
                    let px = if info.interlace_type == 0 {
                        read_image_split(&mut r, info, out_depth, device_rgb, bit_depth == 16, &mut smask)?
                    } else {
                        let image = read_image(&mut r, info, out_depth)?;
                        split_alpha(image, device_rgb, bit_depth == 16, &mut smask)
                    };
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
///
/// The C never holds the image whole: one row at a time, or each row in an `xmalloc` of its own when interlaced.  So a
/// header that says more than memory holds is no failure there, and a file short of rows stops at the first row it
/// does not have, "Not enough image data".  Here the buffer is had whole if it can be, untouched but for the rows that
/// are written, and otherwise grows with the rows the file has: a short file fails as there.  Only an image whose rows
/// are all there and more than memory holds ends here where the C goes on, by `xmalloc`'s message.
fn read_image(r: &mut Rows<'_>, info: &PngInfo, out_depth: u32) -> Result<Vec<u8>, Fail> {
    let out_rowbytes = rowbytes(out_depth, info.width);
    let h = info.height as usize;
    let total = out_rowbytes * h;
    let mut image = Vec::new();
    if info.interlace_type == 0 {
        let _ = image.try_reserve_exact(total);
        let used_bits = (info.width as usize * out_depth as usize) % 8;
        let keep_mask: u8 = if used_bits == 0 { 0xff } else { !(0xffu8 >> used_bits) };
        let mut sink = Whole { image, out_rowbytes, keep_mask };
        read_rows(r, info, &mut sink)?;
        return Ok(sink.image);
    }
    // The passes write each row in pieces, so it is zeroed first: by the allocator, if the whole can be had (asked
    // for and given back first, as `vec!` cannot fail softly), and otherwise row by row as the passes reach them.
    if Vec::<u8>::new().try_reserve_exact(total).is_ok() {
        image = vec![0u8; total];
    }
    let reach = |image: &mut Vec<u8>, end: usize| -> Result<(), Fail> {
        if image.len() < end {
            if image.try_reserve(end - image.len()).is_err() {
                return Err(Fail::xmalloc(end));
            }
            image.resize(end, 0);
        }
        Ok(())
    };
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
            reach(&mut image, (y + 1) * out_rowbytes)?;
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
    reach(&mut image, total)?;
    r.idat.finish_idat()?;
    Ok(image)
}

/// `read_image` and `split_alpha` in one pass, for an image that is not interlaced: each row goes straight to the
/// colour and to `smask` as it comes, as the C's `write_png_gray_alpha` and `write_png_rgb_alpha` do, and is never held
/// with its alpha.  The bytes are those of the two steps: a row of such an image is whole pixels, and no bits are left
/// over at its end for `read_image`'s mask.  It holds less than they would, the colour and the alpha and no more, so an
/// image whose rows are all there and that memory does not hold fails later than there if at all; it fails with the
/// size `read_image` would have asked for.
fn read_image_split(
    r: &mut Rows<'_>,
    info: &PngInfo,
    out_depth: u32,
    rgb: bool,
    wide: bool,
    smask: &mut Vec<u8>,
) -> Result<Vec<u8>, Fail> {
    let (color_bytes, total) = match (rgb, wide) {
        (false, false) => (1, 2),
        (false, true) => (2, 4),
        (true, false) => (3, 4),
        (true, true) => (6, 8),
    };
    let out_rowbytes = rowbytes(out_depth, info.width);
    let color_row = out_rowbytes / total * color_bytes;
    let mut color = Vec::new();
    // A byte more than the colour, for `split_row`'s last store.
    let _ = color.try_reserve_exact(color_row * info.height as usize + 1);
    let split = r.kernels.split;
    let mut sink = Split { color, smask, color_row, out_rowbytes, color_bytes, total, rgb, wide, split };
    read_rows(r, info, &mut sink)?;
    let mut color = sink.color;
    color.shrink_to_fit();
    Ok(color)
}

/// Where `read_rows` puts the rows of an image.
trait RowSink {
    /// Room for row `y`, which has just been read, or `xmalloc`'s end.
    fn reserve(&mut self, y: usize) -> Result<(), Fail>;
    /// The next row.
    fn take(&mut self, row: &[u8]);
}

/// The rows of a picture that is not interlaced, from `read_plain_rows` where it may and from `read_row` elsewhere, to
/// `sink`.
fn read_rows(r: &mut Rows<'_>, info: &PngInfo, sink: &mut impl RowSink) -> Result<(), Fail> {
    if let Some(buf) = r.group_buffer(info.width, info.height) {
        r.read_plain_rows(info.width, info.height, buf, sink)?;
    } else {
        let file_row = rowbytes(info.pixel_depth, info.width);
        r.prev_row[..file_row + 1].fill(0);
        for y in 0..info.height as usize {
            let ri = r.read_row(info.width)?;
            sink.reserve(y)?;
            sink.take(&r.row_buf[1..=ri.rowbytes]);
        }
    }
    r.idat.finish_idat()
}

/// `read_image`'s rows, one after the other.
struct Whole {
    image: Vec<u8>,
    out_rowbytes: usize,
    keep_mask: u8,
}

impl RowSink for Whole {
    fn reserve(&mut self, y: usize) -> Result<(), Fail> {
        let end = (y + 1) * self.out_rowbytes;
        if self.image.try_reserve(end - self.image.len()).is_err() {
            return Err(Fail::xmalloc(end));
        }
        Ok(())
    }

    fn take(&mut self, row: &[u8]) {
        self.image.extend_from_slice(row);
        // png_combine_row keeps the bits past the last pixel from what the row buffer held before: malloc's leftovers
        // in the C.  Zero here.
        let last = self.image.len() - 1;
        self.image[last] &= self.keep_mask;
    }
}

/// `read_image_split`'s rows: each pixel's first `color_bytes` of `total` to `color`, and the byte after them to
/// `smask`.
struct Split<'a> {
    color: Vec<u8>,
    smask: &'a mut Vec<u8>,
    color_row: usize,
    out_rowbytes: usize,
    color_bytes: usize,
    total: usize,
    rgb: bool,
    wide: bool,
    split: fn(&[u8], &mut [u8], &mut [u8], bool, bool),
}

impl RowSink for Split<'_> {
    fn reserve(&mut self, y: usize) -> Result<(), Fail> {
        if self.color.try_reserve((y + 1) * self.color_row + 1 - self.color.len()).is_err() {
            return Err(Fail::xmalloc((y + 1) * self.out_rowbytes));
        }
        Ok(())
    }

    fn take(&mut self, row: &[u8]) {
        let n = row.len() / self.total;
        let (c0, a0) = (self.color.len(), self.smask.len());
        self.color.resize(c0 + n * self.color_bytes + 1, 0);
        self.smask.resize(a0 + n, 0);
        (self.split)(row, &mut self.color[c0..], &mut self.smask[a0..], self.rgb, self.wide);
        self.color.truncate(c0 + n * self.color_bytes);
    }
}

/// One row of `split_alpha`: the colour of each pixel to `color`, and the first byte of its alpha to `alpha`.  `rgb`
/// and `wide` say what a pixel is, as there.  `color` has a byte more than the colour, and `alpha` a byte for each
/// pixel.
pub fn split_row(row: &[u8], color: &mut [u8], alpha: &mut [u8], rgb: bool, wide: bool) {
    match (rgb, wide) {
        (false, false) => split_blocks::<1, 2>(row, color, alpha),
        (false, true) => split_blocks::<2, 4>(row, color, alpha),
        (true, false) => split_blocks::<3, 4>(row, color, alpha),
        (true, true) => split_blocks::<6, 8>(row, color, alpha),
    }
}

/// [`split_row`] compiled for AVX2, which only a CPU that has it may call: some three times as fast, gray and alpha of
/// a byte each by the compiler's `vpshufb`, and RGBA by `split_rgba_avx2`.
///
/// # Safety
/// The CPU has AVX2.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
pub fn split_row_avx2(row: &[u8], color: &mut [u8], alpha: &mut [u8], rgb: bool, wide: bool) {
    match (rgb, wide) {
        (false, false) => split_gray_avx2(row, color, alpha),
        (false, true) => split_blocks::<2, 4>(row, color, alpha),
        (true, false) => split_rgba_avx2(row, color, alpha),
        (true, true) => split_blocks::<6, 8>(row, color, alpha),
    }
}

/// Gray and alpha of a byte each.  A function of its own, since in a larger one the compiler may not see the loop as
/// one to put in vector registers.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
#[inline(never)]
fn split_gray_avx2(row: &[u8], color: &mut [u8], alpha: &mut [u8]) {
    for ((p, c), a) in row.as_chunks::<2>().0.iter().zip(color.iter_mut()).zip(alpha.iter_mut()) {
        *c = p[0];
        *a = p[1];
    }
}

/// RGBA of a byte each, eight pixels at a step: `vpshufb` puts the colour of each 128-bit half before its alpha, and
/// `vpermd` the two colours together, 24 bytes, before the two alphas.  The loads and stores are of whole words, which
/// the compiler makes one load and three stores.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
#[inline(never)]
fn split_rgba_avx2(row: &[u8], color: &mut [u8], alpha: &mut [u8]) {
    use core::arch::x86_64::*;
    let (px, _) = row.as_chunks::<32>();
    let order = _mm256_setr_epi8(
        0, 1, 2, 4, 5, 6, 8, 9, 10, 12, 13, 14, 3, 7, 11, 15, 0, 1, 2, 4, 5, 6, 8, 9, 10, 12, 13, 14, 3, 7, 11, 15,
    );
    let gather = _mm256_setr_epi32(0, 1, 2, 4, 5, 6, 3, 7);
    // `color` has 24 bytes for each 8 pixels and `alpha` 8, so the pieces of all three come out alike.
    for ((s, c), a) in px.iter().zip(color.as_chunks_mut::<24>().0).zip(alpha.as_chunks_mut::<8>().0) {
        let q = |k: usize| i64::from_le_bytes(s[k * 8..][..8].try_into().unwrap());
        let v = _mm256_setr_epi64x(q(0), q(1), q(2), q(3));
        let v = _mm256_permutevar8x32_epi32(_mm256_shuffle_epi8(v, order), gather);
        let (lo, hi) = (_mm256_castsi256_si128(v), _mm256_extracti128_si256::<1>(v));
        c[..8].copy_from_slice(&_mm_cvtsi128_si64(lo).to_le_bytes());
        c[8..16].copy_from_slice(&_mm_extract_epi64::<1>(lo).to_le_bytes());
        c[16..].copy_from_slice(&_mm_cvtsi128_si64(hi).to_le_bytes());
        *a = _mm_extract_epi64::<1>(hi).to_le_bytes();
    }
    let done = px.len() * 8;
    split_blocks::<3, 4>(&row[done * 4..], &mut color[done * 3..], &mut alpha[done..]);
}

/// [`split_row`] for pixels of `TOTAL` bytes, `COLOR` of them colour, in blocks of 16 pixels with nothing to check
/// inside.  The colour of a pixel of four bytes goes as all four, each store over the alpha the one before it left,
/// which is why `dc` has a byte more than the colour.
#[inline(always)]
fn split_blocks<const COLOR: usize, const TOTAL: usize>(row: &[u8], dc: &mut [u8], da: &mut [u8]) {
    const B: usize = 16;
    let n = row.len() / TOTAL;
    let (row, dc, da) = (&row[..n * TOTAL], &mut dc[..n * COLOR + 1], &mut da[..n]);
    let blocks = n / B;
    for k in 0..blocks {
        let s = &row[k * B * TOTAL..(k + 1) * B * TOTAL];
        let a = &mut da[k * B..(k + 1) * B];
        for i in 0..B {
            a[i] = s[i * TOTAL + COLOR];
        }
        let c = &mut dc[k * B * COLOR..(k + 1) * B * COLOR + 1];
        for i in 0..B {
            if COLOR == 3 && TOTAL == 4 {
                c[i * 3..i * 3 + 4].copy_from_slice(&s[i * 4..i * 4 + 4]);
            } else {
                for j in 0..COLOR {
                    c[i * COLOR + j] = s[i * TOTAL + j];
                }
            }
        }
    }
    for i in blocks * B..n {
        da[i] = row[i * TOTAL + COLOR];
        for j in 0..COLOR {
            dc[i * COLOR + j] = row[i * TOTAL + j];
        }
    }
}

/// `write_gray_pixel_8`, `write_gray_pixel_16`, `write_rgb_pixel_8`, `write_rgb_pixel_16`, and the loop writing the
/// SMask, which keeps the high byte only of a 16-bit alpha.  The colour stays in `image`, and the alpha goes to `smask`,
/// which comes with room for it: what the C allocated before it read a row.
fn split_alpha(image: Vec<u8>, rgb: bool, wide: bool, smask: &mut Vec<u8>) -> Vec<u8> {
    match (rgb, wide) {
        (false, false) => split_pixels::<1, 2>(image, smask),
        (false, true) => split_pixels::<2, 4>(image, smask),
        (true, false) => split_pixels::<3, 4>(image, smask),
        (true, true) => split_pixels::<6, 8>(image, smask),
    }
}

/// Each pixel of `TOTAL` bytes keeps its first `COLOR` bytes, moved down in place, and gives the byte after them to
/// `smask`.
fn split_pixels<const COLOR: usize, const TOTAL: usize>(mut image: Vec<u8>, smask: &mut Vec<u8>) -> Vec<u8> {
    let n = image.len() / TOTAL;
    smask.extend(image.chunks_exact(TOTAL).map(|p| p[COLOR]));
    for i in 0..n {
        for k in 0..COLOR {
            image[i * COLOR + k] = image[i * TOTAL + k];
        }
    }
    image.truncate(n * COLOR);
    // The colour stream is held to the end of the run, so give back the alpha's part of the buffer: a realloc that
    // glibc does with mremap at this size, without a copy.
    image.shrink_to_fit();
    image
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `png_read_filter_row_*`, byte by byte as C has them.
    fn c_filter(row: &mut [u8], prev: &[u8], bpp: usize, filter: u8) {
        let n = row.len();
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

    #[test]
    fn filters_are_the_c_ones() {
        let mut seed = 77u32;
        let mut rnd = move || {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            (seed >> 16) as u8
        };
        for (depth, bpp) in [(1u32, 1usize), (8, 1), (16, 2), (24, 3), (32, 4), (48, 6), (64, 8)] {
            for pixels in [1usize, 2, 3, 5, 16, 17, 64, 251] {
                let n = rowbytes(depth, pixels as u32);
                for round in 0..40 {
                    // Noise, flat runs and extremes, since each side of Paeth's comparisons must come up.
                    let mut byte = |k: usize| match round % 4 {
                        0 => rnd(),
                        1 => rnd() & 3,
                        2 => [0u8, 255][(rnd() & 1) as usize],
                        _ => (k * 37) as u8,
                    };
                    let prev: Vec<u8> = (0..=n).map(&mut byte).collect();
                    let row: Vec<u8> = (0..=n).map(&mut byte).collect();
                    for filter in 1..=4 {
                        let mut got = row.clone();
                        got[0] = filter;
                        unfilter_rows(&mut got, &prev, bpp);
                        let mut want = row[1..].to_vec();
                        c_filter(&mut want, &prev[1..], bpp, filter);
                        assert_eq!(&got[1..], &want[..], "depth {depth} pixels {pixels} filter {filter}");
                    }
                }
            }
        }
    }

    /// The predictor of `filter_lanes`, against libpng's `png_read_filter_row_paeth_1byte_pixel`, over all 2^24 inputs.
    #[test]
    fn paeth_is_libpngs_for_every_input() {
        for a in 0..256i32 {
            for b in 0..256i32 {
                for c in 0..256i32 {
                    let p = b - c;
                    let mut pc = a - c;
                    let mut pa = p.abs();
                    let pb = pc.abs();
                    pc = (p + pc).abs();
                    let mut want = a;
                    if pb < pa {
                        pa = pb;
                        want = b;
                    }
                    if pc < pa {
                        want = c;
                    }
                    // The first pixel's predictor is the byte above it, c here, so that pixel comes out as a.
                    let mut row = [(a - c).rem_euclid(256) as u8, 0];
                    filter_lanes::<1>(&mut row, &[c as u8, b as u8], 4);
                    assert_eq!(row[0] as i32, a);
                    assert_eq!(row[1] as i32, want, "a {a} b {b} c {c}");
                }
            }
        }
    }

    #[test]
    fn split_alpha_takes_the_first_byte_of_alpha() {
        let split = |image: &[u8], rgb, wide| {
            let mut smask = Vec::new();
            let px = split_alpha(image.to_vec(), rgb, wide, &mut smask);
            (px, smask)
        };
        assert_eq!(split(&[1, 2, 3, 4], false, false), (vec![1, 3], vec![2, 4]));
        assert_eq!(split(&[1, 2, 3, 4, 5, 6, 7, 8], false, true), (vec![1, 2, 5, 6], vec![3, 7]));
        assert_eq!(split(&[1, 2, 3, 4, 5, 6, 7, 8], true, false), (vec![1, 2, 3, 5, 6, 7], vec![4, 8]));
        assert_eq!(split(&[1, 2, 3, 4, 5, 6, 7, 8], true, true), (vec![1, 2, 3, 4, 5, 6], vec![7]));
        // A row at a time, as `read_image_split` does, across the blocks of `split_blocks` and what they leave over.
        let mut seed = 5u32;
        for (rgb, wide, total) in [(false, false, 2), (false, true, 4), (true, false, 4), (true, true, 8)] {
            let color = if rgb { total * 3 / 4 } else { total / 2 };
            for pixels in [0usize, 1, 7, 15, 16, 17, 31, 32, 33, 100] {
                let row: Vec<u8> = (0..pixels * total)
                    .map(|_| {
                        seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                        (seed >> 24) as u8
                    })
                    .collect();
                let (want, want_alpha) = split(&row, rgb, wide);
                let mut got = vec![0u8; pixels * color + 1];
                let mut alpha = vec![0u8; pixels];
                split_row(&row, &mut got, &mut alpha, rgb, wide);
                got.pop();
                assert_eq!((got, alpha), (want, want_alpha), "rgb {rgb} wide {wide} pixels {pixels}");
            }
        }
    }

    /// Rows that come several at a time, as `read_plain_rows` hands them over: each under the one before it, with runs
    /// of Paeth around the bands' heights, among the other filters, at widths around a band's ends.
    #[test]
    fn a_group_of_rows_unfilters_as_its_rows_do() {
        let mut seed = 99u32;
        let mut rnd = move || {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            (seed >> 16) as u8
        };
        for bpp in [1usize, 2, 3, 4, 6, 8] {
            for width in [1usize, 2, 3, 4, 7, 8, 9, 33, 100] {
                let stride = width * bpp + 1;
                for round in 0..12 {
                    let h = 1 + (rnd() as usize % 40);
                    let mut rows: Vec<u8> = (0..h * stride)
                        .map(|k| if round % 4 == 3 { [0u8, 255][(rnd() & 1) as usize] } else { rnd() ^ k as u8 })
                        .collect();
                    for y in 0..h {
                        rows[y * stride] = match round % 3 {
                            0 => 4,
                            1 => [4, 4, 4, 4, 4, 4, 0, 1, 2, 3][rnd() as usize % 10],
                            _ => rnd() % 5,
                        };
                    }
                    let prev: Vec<u8> = (0..stride).map(|_| rnd()).collect();
                    let mut want = rows.clone();
                    for y in 0..h {
                        let (above, rest) = want.split_at_mut(y * stride);
                        let above = if y == 0 { &prev[1..] } else { &above[(y - 1) * stride + 1..] };
                        let filter = rest[0];
                        if filter != 0 {
                            c_filter(&mut rest[1..stride], above, bpp, filter);
                        }
                    }
                    let mut got = rows.clone();
                    unfilter_rows(&mut got, &prev, bpp);
                    assert_eq!(got, want, "bpp {bpp} width {width} round {round}");
                }
            }
        }
    }

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

    /// A PNG with these header fields whose image data is `raw`, in one stored deflate block.
    fn png_file(w: u32, h: u32, depth: u8, color: u8, interlace: u8, raw: &[u8]) -> Vec<u8> {
        fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
            out.extend_from_slice(&(data.len() as u32).to_be_bytes());
            let start = out.len();
            out.extend_from_slice(kind);
            out.extend_from_slice(data);
            let crc = zlib_rs::crc32::crc32(0, &out[start..]);
            out.extend_from_slice(&crc.to_be_bytes());
        }
        let mut ihdr = [w.to_be_bytes(), h.to_be_bytes()].concat();
        ihdr.extend_from_slice(&[depth, color, 0, 0, interlace]);
        let mut z = vec![0x78, 0x01, 0x01];
        z.extend_from_slice(&(raw.len() as u16).to_le_bytes());
        z.extend_from_slice(&(!(raw.len() as u16)).to_le_bytes());
        z.extend_from_slice(raw);
        let (mut a, mut b) = (1u32, 0u32);
        for &x in raw {
            a = (a + x as u32) % 65521;
            b = (b + a) % 65521;
        }
        z.extend_from_slice(&(b << 16 | a).to_be_bytes());
        let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
        chunk(&mut out, b"IHDR", &ihdr);
        chunk(&mut out, b"IDAT", &z);
        chunk(&mut out, b"IEND", &[]);
        out
    }

    /// `write_png` at PDF 1.`minor` with pdfTeX's defaults.  At 1.4 no image is copied, and 16 bits are made 8.
    fn write(data: &[u8], minor: i32) -> Result<PngOutput, Fail> {
        let info = read_png_info(data)?;
        let p = PngParams {
            pdf_major: 1,
            pdf_minor: minor,
            image_hicolor: 1,
            image_apply_gamma: 0,
            gamma: 1000,
            image_gamma: 2200,
            colorspace_ref: 0,
            kernels: Kernels::PLAIN,
        };
        write_png(&info, data, &p, &mut SlotAlloc::default())
    }

    /// The bytes of the image's stream: what follows the first `BeginStream`.
    fn stream(out: &PngOutput) -> &[u8] {
        let i = out.ops.iter().position(|o| *o == Op::BeginStream).unwrap();
        match &out.ops[i + 1] {
            Op::Puts(b) => b,
            o => panic!("{o:?}"),
        }
    }

    /// A header of 10^6 by 10^6, libpng's limit, over eight bytes of data.  pdfTeX reads a row at a time, or allocates each
    /// row of an interlaced image apart, and the first row is not there: "Not enough image data".  But first it allocates
    /// the SMask of an image with alpha, whole, and if malloc refuses that, xmalloc ends the run.  Nothing here is had
    /// whole that the C does not have whole.
    #[test]
    fn a_header_bigger_than_memory_fails_where_the_c_does() {
        let short =
            |depth, color, interlace, minor| write(&png_file(1000000, 1000000, depth, color, interlace, &[0; 8]), minor).err();
        assert_eq!(short(16, COLOR_GRAY, 0, 4), Some(png_error()));
        assert_eq!(short(8, COLOR_RGB, 1, 5), Some(png_error()));
        // Whether malloc gives a terabyte is the machine's to say.
        let smask = |n: usize| Some(if Vec::<u8>::new().try_reserve_exact(n).is_ok() { png_error() } else { Fail::xmalloc(n) });
        assert_eq!(short(8, COLOR_GRAY_ALPHA, 0, 5), smask(1000000000000));
        assert_eq!(short(16, COLOR_RGB_ALPHA, 1, 5), smask(2000000000000));
        assert_eq!(short(16, COLOR_RGB_ALPHA, 0, 4), smask(1000000000000));
        let m = b"fatal: memory exhausted (xmalloc of 1000000000000 bytes).\n";
        assert_eq!(Fail::xmalloc(1000000000000), Fail(m.to_vec(), crate::End::Exit));
    }

    /// An interlaced image of 3 by 3, its seven passes as libpng reads them: each row has its pixels, and the rest zeros
    /// until a later pass.
    #[test]
    fn interlaced_rows_come_together() {
        let raw = [0, 1, 0, 3, 0, 7, 9, 0, 2, 0, 8, 0, 4, 5, 6];
        let out = write(&png_file(3, 3, 8, COLOR_GRAY, 1, &raw), 5).unwrap();
        assert_eq!(stream(&out), &[1, 2, 3, 4, 5, 6, 7, 8, 9]);
        let raw = [0, 1, 0, 3];
        assert_eq!(write(&png_file(3, 3, 8, COLOR_GRAY, 1, &raw), 5).err(), Some(png_error()));
    }

    #[test]
    fn chunk_names() {
        assert!(check_chunk_name(C_IHDR));
        assert!(check_chunk_name(C_GAMA));
        assert!(!check_chunk_name(u32::from_be_bytes(*b"I2DR")));
        assert!(!check_chunk_name(u32::from_be_bytes([0x49, 0x48, 0x20, 0x52])));
    }
}
