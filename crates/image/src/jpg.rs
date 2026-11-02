//! writejpg.c: `read_jpg_info` and `write_jpg`.
//!
//! The C reads a `FILE`.  Here it is a position over the byte slice, with stdio's rules: `getc` at the end gives -1 and
//! sets the end-of-file flag, a seek clears the flag and may go past the end.

use crate::out::{Op, SlotAlloc, Writer};
use crate::{Fail, IMAGE_COLOR_B, IMAGE_COLOR_C};

const JPG_GRAY: i32 = 1;
const JPG_RGB: i32 = 3;
const JPG_CMYK: i32 = 4;

const M_SOF0: i32 = 0xc0;
const M_SOF1: i32 = 0xc1;
const M_SOF2: i32 = 0xc2;
const M_SOF3: i32 = 0xc3;
const M_SOF5: i32 = 0xc5;
const M_SOF6: i32 = 0xc6;
const M_SOF7: i32 = 0xc7;
const M_SOF9: i32 = 0xc9;
const M_SOF10: i32 = 0xca;
const M_SOF11: i32 = 0xcb;
const M_SOF13: i32 = 0xcd;
const M_SOF14: i32 = 0xce;
const M_SOF15: i32 = 0xcf;
const M_RST0: i32 = 0xd0;
const M_RST7: i32 = 0xd7;
const M_SOI: i32 = 0xd8;
const M_EOI: i32 = 0xd9;
const M_TEM: i32 = 0x01;

/// What `read_jpg_info` leaves in the image entry and in `JPG_IMAGE_INFO`.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct JpgInfo {
    /// `img_width`.
    pub width: i32,
    /// `img_height`.
    pub height: i32,
    /// `img_xres`.
    pub x_res: i32,
    /// `img_yres`.
    pub y_res: i32,
    /// `img_color`.
    pub img_color: i32,
    /// `jpg_ptr->bits_per_component`.
    pub bits_per_component: i32,
    /// `jpg_ptr->color_space`.
    pub color_space: i32,
    /// `jpg_ptr->length`, the size of the file.
    pub length: usize,
}

struct File<'a> {
    d: &'a [u8],
    pos: u64,
    eof: bool,
}

impl File<'_> {
    fn getc(&mut self) -> i32 {
        if self.pos < self.d.len() as u64 {
            let c = self.d[self.pos as usize];
            self.pos += 1;
            c as i32
        } else {
            self.eof = true;
            -1
        }
    }

    fn seek_set(&mut self, p: u64) {
        self.pos = p;
        self.eof = false;
    }

    fn seek_cur(&mut self, delta: u64) {
        self.pos = self.pos.saturating_add(delta);
        self.eof = false;
    }

    /// `read2bytes`: `(c << 8) + xgetc(f)`, as an unsigned int, so -1 at the end of the file comes along.
    fn read2bytes(&mut self) -> u32 {
        let c = self.getc();
        (c.wrapping_shl(8)).wrapping_add(self.getc()) as u32
    }

    /// `get_unsigned_pair`: bytes through `unsigned char`, so -1 is 255.
    fn get_unsigned_pair(&mut self) -> u16 {
        let a = self.getc() as u8 as u16;
        (a << 8) | self.getc() as u8 as u16
    }
}

/// `read_exif_bytes`.  The C runs off a damaged segment's end; here those bytes read as zero.
fn read_exif_bytes(b: &[u8], p: &mut i64, n: i32, bigendian: bool) -> u32 {
    let at = |i: i64| -> u32 {
        if i >= 0 && (i as usize) < b.len() { b[i as usize] as u32 } else { 0 }
    };
    let mut rval: u32 = 0;
    let mut pp = *p;
    if bigendian {
        if n == 4 {
            rval = rval.wrapping_add(at(pp));
            pp += 1;
            rval <<= 8;
            rval = rval.wrapping_add(at(pp));
            pp += 1;
            rval <<= 8;
        }
        rval = rval.wrapping_add(at(pp));
        pp += 1;
        rval <<= 8;
        rval = rval.wrapping_add(at(pp));
    } else {
        pp += n as i64;
        if n == 4 {
            pp -= 1;
            rval = rval.wrapping_add(at(pp));
            rval <<= 8;
            pp -= 1;
            rval = rval.wrapping_add(at(pp));
            rval <<= 8;
        }
        pp -= 1;
        rval = rval.wrapping_add(at(pp));
        rval <<= 8;
        pp -= 1;
        rval = rval.wrapping_add(at(pp));
    }
    *p += n as i64;
    rval
}

/// `read_APP1_Exif`.
fn read_app1_exif(f: &mut File<'_>, length: u16) -> (i32, i32) {
    let mut buffer = vec![0u8; length as usize];
    for slot in buffer.iter_mut() {
        let c = f.getc();
        if c < 0 {
            break;
        }
        *slot = c as u8;
    }
    let len = length as usize;
    let mut q = 0usize;
    while q < len && buffer[q] == 0 {
        q += 1;
    }
    let tiff_header = q as i64;
    let at = |i: i64| -> u8 {
        if i >= 0 && (i as usize) < buffer.len() { buffer[i as usize] } else { 0 }
    };
    let bigendian = if at(tiff_header) == b'M' && at(tiff_header + 1) == b'M' {
        true
    } else if at(tiff_header) == b'I' && at(tiff_header + 1) == b'I' {
        false
    } else {
        return (0, 0);
    };
    let mut xres = 72.0f64;
    let mut yres = 72.0f64;
    let mut res_unit = 1.0f64;
    let (mut value, mut num, mut den) = (0i32, 0i32, 0i32);
    let mut p = tiff_header + 2;
    let i = read_exif_bytes(&buffer, &mut p, 2, bigendian) as i32;
    if i != 42 {
        return (0, 0);
    }
    let i = read_exif_bytes(&buffer, &mut p, 4, bigendian) as i32;
    p = tiff_header + i as i64;
    let mut num_fields = read_exif_bytes(&buffer, &mut p, 2, bigendian) as i32;
    while num_fields > 0 {
        num_fields -= 1;
        let tag = read_exif_bytes(&buffer, &mut p, 2, bigendian) as i32;
        let ty = read_exif_bytes(&buffer, &mut p, 2, bigendian) as i32;
        read_exif_bytes(&buffer, &mut p, 4, bigendian);
        match ty {
            1 | 7 => {
                value = at(p) as i32;
                p += 4;
            }
            3 => {
                value = read_exif_bytes(&buffer, &mut p, 2, bigendian) as i32;
                p += 2;
            }
            4 | 9 => value = read_exif_bytes(&buffer, &mut p, 4, bigendian) as i32,
            5 | 10 => {
                value = read_exif_bytes(&buffer, &mut p, 4, bigendian) as i32;
                let mut rp = tiff_header + value as i64;
                num = read_exif_bytes(&buffer, &mut rp, 4, bigendian) as i32;
                den = read_exif_bytes(&buffer, &mut rp, 4, bigendian) as i32;
            }
            _ => p += 4,
        }
        match tag {
            282 => {
                if den != 0 {
                    xres = num.wrapping_div(den) as f64;
                }
            }
            283 => {
                if den != 0 {
                    yres = num.wrapping_div(den) as f64;
                }
            }
            296 => match value {
                2 => res_unit = 1.0,
                3 => res_unit = 2.54,
                _ => {}
            },
            _ => {}
        }
    }
    ((xres * res_unit) as i32, (yres * res_unit) as i32)
}

/// `read_jpg_info`.  `pdf_major` and `pdf_minor` are `fixedpdfmajorversion` and `fixedpdfminorversion`.
pub fn read_jpg_info(data: &[u8], pdf_major: i32, pdf_minor: i32) -> Result<JpgInfo, Fail> {
    let mut info = JpgInfo { length: data.len(), ..JpgInfo::default() };
    let mut f = File { d: data, pos: 0, eof: false };
    let mut x_res: i64 = 0;
    let mut y_res: i64 = 0;
    if f.read2bytes() != 0xFFD8 {
        return Err(Fail::new("reading JPEG image failed (no JPEG header found)"));
    }
    // currently JFIF and Exif files allow extracting img_xres and img_yres
    let appmk = f.read2bytes() as u16;
    if appmk == 0xFFE0 {
        let _ = f.read2bytes();
        let jpg_id = b"JFIF\0";
        let mut i = 0;
        while i < 5 {
            if f.getc() != jpg_id[i] as i32 {
                break;
            }
            i += 1;
        }
        if i == 5 {
            let _ = f.read2bytes();
            let units = f.getc();
            x_res = f.read2bytes() as i64;
            y_res = f.read2bytes() as i64;
            match units {
                1 => {}
                2 => {
                    x_res = (x_res as f64 * 2.54) as i64;
                    y_res = (y_res as f64 * 2.54) as i64;
                }
                _ => {
                    x_res = 0;
                    y_res = 0;
                }
            }
        }
        if x_res == 0 && y_res != 0 {
            x_res = y_res;
        }
        if y_res == 0 && x_res != 0 {
            y_res = x_res;
        }
    } else if appmk == 0xFFE1 {
        let (mut xxres, mut yyres) = (0, 0);
        let mut length = f.get_unsigned_pair().wrapping_sub(2);
        if length > 5 {
            let mut app_sig = [0u8; 5];
            for slot in app_sig.iter_mut() {
                let c = f.getc();
                if c < 0 {
                    // if (fread(app_sig, sizeof(char), 5, fp) != 5) return;
                    info.x_res = x_res.min(i32::MAX as i64) as i32;
                    info.y_res = y_res.min(i32::MAX as i64) as i32;
                    return Ok(info);
                }
                *slot = c as u8;
            }
            length -= 5;
            if &app_sig == b"Exif\0" {
                (xxres, yyres) = read_app1_exif(&mut f, length);
            }
        }
        x_res = xxres as i64;
        y_res = yyres as i64;
    }
    // `integer` is 64 bits in the C, so -1 from a file ending here reads as a large resolution, and is refused as one.
    info.x_res = x_res.min(i32::MAX as i64) as i32;
    info.y_res = y_res.min(i32::MAX as i64) as i32;
    f.seek_set(0);
    loop {
        if f.eof {
            return Err(Fail::new("reading JPEG image failed (premature file end)"));
        }
        if f.getc() != 0xFF {
            return Err(Fail::new("reading JPEG image failed (no marker found)"));
        }
        let m = f.getc();
        match m {
            M_SOF5 | M_SOF6 | M_SOF7 | M_SOF9 | M_SOF10 | M_SOF11 | M_SOF13 | M_SOF14 | M_SOF15 => {
                return Err(Fail::new("unsupported type of compression"));
            }
            M_SOF0 | M_SOF1 | M_SOF2 | M_SOF3 => {
                if m == M_SOF2 && pdf_major == 1 && pdf_minor <= 2 {
                    return Err(Fail::new("cannot use progressive DCT with PDF-1.2"));
                }
                let _ = f.read2bytes();
                info.bits_per_component = (f.getc() as u8) as i32;
                info.height = f.read2bytes() as i32;
                info.width = f.read2bytes() as i32;
                info.color_space = f.getc();
                info.img_color = match info.color_space {
                    JPG_GRAY => IMAGE_COLOR_B,
                    JPG_RGB | JPG_CMYK => IMAGE_COLOR_C,
                    n => return Err(Fail(format!("Unsupported color space {n}").into_bytes())),
                };
                return Ok(info);
            }
            M_SOI | M_EOI | M_TEM | M_RST0..=M_RST7 => {}
            _ => {
                // `read2bytes(...) - 2` is unsigned, so a short length seeks far ahead rather than back.
                let n = f.read2bytes().wrapping_sub(2);
                f.seek_cur(n as u64);
            }
        }
    }
}

/// `write_jpg`.  `colorspace_ref` is `img_colorspace_ref`.  The engine has called `pdf_begin_dict` and printed the
/// attributes, as `pdf_write_image` does.
pub fn write_jpg(info: &JpgInfo, colorspace_ref: i32, slots: &mut SlotAlloc) -> Result<Vec<Op>, Fail> {
    let mut w = Writer::new(slots);
    w.puts(b"/Type /XObject\n/Subtype /Image\n");
    w.printf(format_args!(
        "/Width {}\n/Height {}\n/BitsPerComponent {}\n/Length {}\n",
        info.width, info.height, info.bits_per_component, info.length as i32
    ));
    w.puts(b"/ColorSpace ");
    if colorspace_ref != 0 {
        w.printf(format_args!("{colorspace_ref} 0 R\n"));
    } else {
        match info.color_space {
            JPG_GRAY => w.puts(b"/DeviceGray\n"),
            JPG_RGB => w.puts(b"/DeviceRGB\n"),
            JPG_CMYK => w.puts(b"/DeviceCMYK\n/Decode [1 0 1 0 1 0 1 0]\n"),
            n => return Err(Fail(format!("Unsupported color space {n}").into_bytes())),
        }
    }
    w.puts(b"/Filter /DCTDecode\n>>\nstream\n");
    w.copy_input(0, info.length);
    w.op(Op::EndStream);
    Ok(w.ops)
}
