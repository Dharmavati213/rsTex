//! `pkin.c`: reading a PK font, and the two `sscanf`s of a `.pgc` file in `writet3.c`.
//!
//! `pkin.c` reads `t3_file` through statics.  Here the file is a slice and the place in it is the caller's, with [`CharDesc`], and what
//! `unpack` keeps in statics lives for one call.  The types are the C's: `halfword` is an `i32` here as there, and `shalfword` an `i16`,
//! so a width beyond it wraps as it does there.
use crate::FontError;
use crate::cstr::{Scan, push_g};

/// `chardesc`.
#[derive(Clone, Debug, Default)]
pub struct CharDesc {
    pub charcode: i32,
    pub cwidth: i32,
    pub cheight: i32,
    pub xoff: i32,
    pub yoff: i32,
    pub xescape: i32,
    /// Sixteen bits in each, as `halfword *raster`.
    pub raster: Vec<i32>,
}

/// What comes next from the packed numbers: `realfunc`.
#[derive(Clone, Copy, PartialEq)]
enum RealFunc {
    PkPackedNum,
    Rest,
}

const GPOWER: [i32; 17] = [0, 1, 3, 7, 15, 31, 63, 127, 255, 511, 1023, 2047, 4095, 8191, 16383, 32767, 65535];

/// `gpower[k]`.  A character 32768 pixels wide or more has a negative `shalfword hbit`, and with a black run at the end of
/// a row `unpack` reads `gpower[wordweight - hbit]`, past the array: the C's undefined behaviour, a `SIGSEGV` on the
/// oracle.  Here it is the failure the same character gets with a white run there, which the raster's bound gives.
fn gpower(k: i32) -> Result<i32, FontError> {
    usize::try_from(k)
        .ok()
        .and_then(|k| GPOWER.get(k).copied())
        .ok_or_else(|| FontError::new(b"error while unpacking; more bits than required"))
}

/// The file, and the statics of `pkin.c`.
struct Pk<'a, 'b> {
    data: &'a [u8],
    pos: &'b mut usize,
    inputbyte: i32,
    flagbyte: i32,
    bitweight: i32,
    dynf: i32,
    repeatcount: i32,
    realfunc: RealFunc,
    pk_remainder: i64,
}

impl Pk<'_, '_> {
    fn pkbyte(&mut self) -> Result<i32, FontError> {
        match self.data.get(*self.pos) {
            Some(&b) => {
                *self.pos += 1;
                Ok(b as i32)
            }
            None => Err(FontError::new(b"unexpected eof in pk file")),
        }
    }

    fn pkduo(&mut self) -> Result<i32, FontError> {
        let mut i = self.pkbyte()?;
        if i > 127 {
            i -= 256;
        }
        Ok(i * 256 + self.pkbyte()?)
    }

    fn pktrio(&mut self) -> Result<i32, FontError> {
        let mut i = self.pkduo()?;
        i = i * 256 + self.pkbyte()?;
        Ok(i)
    }

    fn pkquad(&mut self) -> Result<i32, FontError> {
        let mut i = self.pktrio()?;
        i = i.wrapping_mul(256).wrapping_add(self.pkbyte()?);
        Ok(i)
    }

    fn getnyb(&mut self) -> Result<i32, FontError> {
        if self.bitweight == 0 {
            self.bitweight = 16;
            self.inputbyte = self.pkbyte()?;
            Ok(self.inputbyte >> 4)
        } else {
            self.bitweight = 0;
            Ok(self.inputbyte & 15)
        }
    }

    fn getbit(&mut self) -> Result<bool, FontError> {
        self.bitweight >>= 1;
        if self.bitweight == 0 {
            self.inputbyte = self.pkbyte()?;
            self.bitweight = 128;
        }
        Ok(self.inputbyte & self.bitweight != 0)
    }

    fn real(&mut self) -> Result<i32, FontError> {
        match self.realfunc {
            RealFunc::PkPackedNum => self.pkpackednum(),
            RealFunc::Rest => self.rest(),
        }
    }

    fn pkpackednum(&mut self) -> Result<i32, FontError> {
        let mut i = self.getnyb()?;
        if i == 0 {
            let mut j;
            loop {
                j = self.getnyb()?;
                i += 1;
                if j != 0 {
                    break;
                }
            }
            if i > 3 {
                // Damn, we got a huge count!  We *fake* it by giving an artificially large repeat count.
                self.handlehuge(i, j)
            } else {
                while i > 0 {
                    j = j * 16 + self.getnyb()?;
                    i -= 1;
                }
                Ok(j - 15 + (13 - self.dynf) * 16 + self.dynf)
            }
        } else if i <= self.dynf {
            Ok(i)
        } else if i < 14 {
            Ok((i - self.dynf - 1) * 16 + self.getnyb()? + self.dynf + 1)
        } else {
            if i == 14 {
                self.repeatcount = self.pkpackednum()?;
            } else {
                self.repeatcount = 1;
            }
            self.real()
        }
    }

    fn rest(&mut self) -> Result<i32, FontError> {
        if self.pk_remainder < 0 {
            self.pk_remainder = -self.pk_remainder;
            Ok(0)
        } else if self.pk_remainder > 0 {
            if self.pk_remainder > 4000 {
                self.pk_remainder = 4000 - self.pk_remainder;
                Ok(4000)
            } else {
                let i = self.pk_remainder as i32;
                self.pk_remainder = 0;
                self.realfunc = RealFunc::PkPackedNum;
                Ok(i)
            }
        } else {
            Err(FontError::new(b"shouldn't happen"))
        }
    }

    fn handlehuge(&mut self, mut i: i32, k: i32) -> Result<i32, FontError> {
        let mut j = k as i64;
        while i != 0 {
            j = (j << 4) + self.getnyb()? as i64;
            i -= 1;
        }
        self.pk_remainder = j - 15 + ((13 - self.dynf) * 16 + self.dynf) as i64;
        self.realfunc = RealFunc::Rest;
        self.rest()
    }

    fn unpack(&mut self, cd: &mut CharDesc) -> Result<(), FontError> {
        let wordwidth = ((cd.cwidth as i64 + 15) / 16) as i16;
        let i = (2 * cd.cheight as i64 * wordwidth as i64).max(2);
        // `xtalloc(i, halfword)`: `i` halfwords of 8 bytes on the oracle, where `xmalloc` prints its message and exits if
        // `malloc` refuses them.  They are reserved here as they are there, so that the two fail alike, and only what is
        // written is touched.  That is twice what a bitmap of this size can write, and past it is the C's overflow.
        let total = cd.cheight as i64 * wordwidth as i64;
        cd.raster.clear();
        if usize::try_from(2 * i).map_or(true, |n| cd.raster.try_reserve_exact(n).is_err()) {
            return Err(FontError::xmalloc(i as u64 * 8));
        }
        let put = |word: i32, r: &mut Vec<i32>| -> Result<(), FontError> {
            if r.len() as i64 > total {
                return Err(FontError::new(b"error while unpacking; more bits than required"));
            }
            r.push(word);
            Ok(())
        };
        self.realfunc = RealFunc::PkPackedNum;
        self.dynf = self.flagbyte / 16;
        let mut turnon = self.flagbyte & 8 != 0;
        if self.dynf == 14 {
            self.bitweight = 0;
            for _ in 1..=cd.cheight {
                let mut word = 0;
                let mut wordweight = 32768;
                for _ in 1..=cd.cwidth {
                    if self.getbit()? {
                        word += wordweight;
                    }
                    wordweight >>= 1;
                    if wordweight == 0 {
                        put(word, &mut cd.raster)?;
                        word = 0;
                        wordweight = 32768;
                    }
                }
                if wordweight != 32768 {
                    put(word, &mut cd.raster)?;
                }
            }
        } else {
            let mut rowsleft = cd.cheight as i16;
            let mut hbit = cd.cwidth as i16;
            self.repeatcount = 0;
            let mut wordweight = 16;
            let mut word = 0;
            self.bitweight = 0;
            while rowsleft > 0 {
                let mut count = self.real()?;
                while count != 0 {
                    if count < wordweight && count < hbit as i32 {
                        if turnon {
                            word += gpower(wordweight)? - gpower(wordweight - count)?;
                        }
                        hbit = (hbit as i32 - count) as i16;
                        wordweight -= count;
                        count = 0;
                    } else if count >= hbit as i32 && hbit as i32 <= wordweight {
                        if turnon {
                            word += gpower(wordweight)? - gpower(wordweight - hbit as i32)?;
                        }
                        put(word, &mut cd.raster)?;
                        for _ in 1..=self.repeatcount {
                            for _ in 1..=wordwidth {
                                let n = cd.raster.len();
                                let w = if n >= wordwidth as usize { cd.raster[n - wordwidth as usize] } else { 0 };
                                put(w, &mut cd.raster)?;
                            }
                        }
                        rowsleft = (rowsleft as i32 - (self.repeatcount + 1)) as i16;
                        self.repeatcount = 0;
                        word = 0;
                        wordweight = 16;
                        count -= hbit as i32;
                        hbit = cd.cwidth as i16;
                    } else {
                        if turnon {
                            word += gpower(wordweight)?;
                        }
                        put(word, &mut cd.raster)?;
                        word = 0;
                        count -= wordweight;
                        hbit = (hbit as i32 - wordweight) as i16;
                        wordweight = 16;
                    }
                }
                turnon = !turnon;
            }
            if rowsleft != 0 || hbit as i32 != cd.cwidth {
                return Err(FontError::new(b"error while unpacking; more bits than required"));
            }
        }
        Ok(())
    }
}

/// `readchar`: checks the preamble if asked to, and reads the next character definition into `cd`.  False when there is none.  `pos` is
/// where the C's file position is.
pub fn pk_read_char(data: &[u8], pos: &mut usize, check_preamble: bool, cd: &mut CharDesc) -> Result<bool, FontError> {
    let mut p = Pk {
        data,
        pos,
        inputbyte: 0,
        flagbyte: 0,
        bitweight: 0,
        dynf: 0,
        repeatcount: 0,
        realfunc: RealFunc::PkPackedNum,
        pk_remainder: 0,
    };
    // Check the preamble of the pkfile.
    if check_preamble {
        if p.pkbyte()? != 247 {
            return Err(FontError::new(b"bad pk file, expected pre"));
        }
        if p.pkbyte()? != 89 {
            return Err(FontError::new(b"bad version of pk file"));
        }
        let mut i = p.pkbyte()?; // creator of pkfile
        while i > 0 {
            p.pkbyte()?;
            i -= 1;
        }
        p.pkquad()?; // design size
        p.pkquad()?; // checksum
        p.pkquad()?; // hppp
        p.pkquad()?; // vppp
    }
    // Now we skip to the desired character definition.
    loop {
        p.flagbyte = p.pkbyte()?;
        if p.flagbyte == 245 {
            return Ok(false); // character not found
        }
        if p.flagbyte < 240 {
            let length: i64;
            match p.flagbyte & 7 {
                0..=3 => {
                    length = ((p.flagbyte & 7) * 256 + p.pkbyte()? - 3) as i64;
                    cd.charcode = p.pkbyte()?;
                    p.pktrio()?; // TFMwidth
                    cd.xescape = p.pkbyte()?; // pixel width
                    cd.cwidth = p.pkbyte()?;
                    cd.cheight = p.pkbyte()?;
                    cd.xoff = p.pkbyte()?;
                    cd.yoff = p.pkbyte()?;
                    if cd.xoff > 127 {
                        cd.xoff -= 256;
                    }
                    if cd.yoff > 127 {
                        cd.yoff -= 256;
                    }
                }
                4..=6 => {
                    let l = (p.flagbyte & 3) as i64 * 65536 + p.pkbyte()? as i64 * 256;
                    length = l + p.pkbyte()? as i64 - 4;
                    cd.charcode = p.pkbyte()?;
                    p.pktrio()?; // TFMwidth
                    cd.xescape = p.pkduo()?; // pixelwidth
                    cd.cwidth = p.pkduo()?;
                    cd.cheight = p.pkduo()?;
                    cd.xoff = p.pkduo()?;
                    cd.yoff = p.pkduo()?;
                }
                _ => {
                    length = p.pkquad()? as i64 - 9;
                    cd.charcode = p.pkquad()?;
                    p.pkquad()?; // TFMwidth
                    cd.xescape = p.pkquad()?; // pixelwidth
                    p.pkquad()?;
                    cd.cwidth = p.pkquad()?;
                    cd.cheight = p.pkquad()?;
                    cd.xoff = p.pkquad()?;
                    cd.yoff = p.pkquad()?;
                }
            }
            if length <= 0 {
                let msg = format!("packet length ({length}) too small");
                return Err(FontError::new(msg.as_bytes()));
            }
            p.unpack(cd)?;
            return Ok(true);
        }
        let mut k: i64 = 0;
        match p.flagbyte {
            240..=243 => {
                if p.flagbyte == 243 {
                    k = p.pkbyte()? as i64;
                    if k > 127 {
                        k -= 256;
                    }
                }
                if p.flagbyte >= 242 {
                    k = k * 256 + p.pkbyte()? as i64;
                }
                if p.flagbyte >= 241 {
                    k = k * 256 + p.pkbyte()? as i64;
                }
                k = k * 256 + p.pkbyte()? as i64;
                while k > 0 {
                    p.pkbyte()?;
                    k -= 1;
                }
            }
            244 => {
                p.pkquad()?;
            }
            246 => {}
            _ => {
                let msg = format!("unexpected command ({})", p.flagbyte);
                return Err(FontError::new(msg.as_bytes()));
            }
        }
    }
}

/// `sscanf(line, "%i %i %i %i %i %i %i %i =", ...) != 8` of `t3_write_glyph`, with `line` after the name: all eight, or none.
pub fn pgc_glyph_preamble(line: &[u8]) -> Option<[i32; 8]> {
    let mut s = Scan::new(line, 0);
    let mut out = [0; 8];
    for x in &mut out {
        *x = s.int(0, usize::MAX)? as i32;
    }
    Some(out)
}

/// `sscanf(line, "%g", &t3_font_scale)` of `writet3`.
pub fn pgc_font_scale(line: &[u8]) -> Option<f32> {
    Scan::new(line, 0).float()
}

/// `%g` of a `float` promoted to `double`, for the `/FontMatrix` of a `.pgc` font.
pub fn pgc_print_g(x: f32) -> Vec<u8> {
    let mut b = Vec::new();
    push_g(&mut b, x as f64);
    b
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pre() -> Vec<u8> {
        let mut f = vec![247, 89, 2, b'x', b'y'];
        f.extend_from_slice(&[0; 16]);
        f
    }

    #[test]
    fn a_bad_preamble_fails_as_the_c_does() {
        let mut cd = CharDesc::default();
        let msg = |d: &[u8]| {
            let mut pos = 0;
            String::from_utf8(pk_read_char(d, &mut pos, true, &mut CharDesc::default()).unwrap_err().message).unwrap()
        };
        assert_eq!(msg(&[1]), "bad pk file, expected pre");
        assert_eq!(msg(&[247, 1]), "bad version of pk file");
        assert_eq!(msg(&[247]), "unexpected eof in pk file");
        let mut pos = 0;
        assert!(!pk_read_char(&[pre(), vec![245]].concat(), &mut pos, true, &mut cd).unwrap());
    }

    #[test]
    fn a_char_in_bits_and_in_runs() {
        // dyn_f 14: a raster of 3 rows of 10 bits, whose rows are 1111100000, 0000011111 and 1010101010.
        let bits = [0b11111000u8, 0b00000001, 0b11111010, 0b10101010];
        let mut f = pre();
        f.extend_from_slice(&[14 << 4, (8 + bits.len()) as u8, 65, 0, 0, 0, 12, 10, 3, 0, 2]);
        f.extend_from_slice(&bits);
        // dyn_f 2: the same size, all black: 30 pixels on, as a two-nybble run.
        f.extend_from_slice(&[(2 << 4) | 8, 10, 66, 0, 0, 0, 12, 10, 3, 1, 255]);
        // 30 = (i - dyn_f - 1) * 16 + nybble + dyn_f + 1: i=3 and nybble 13 give 16*0+13+3=16; i=4 and 11 gives 16+11+3=30
        f.extend_from_slice(&[0x4b]);
        f.push(245);
        let mut pos = 0;
        let mut cd = CharDesc::default();
        assert!(pk_read_char(&f, &mut pos, true, &mut cd).unwrap());
        assert_eq!((cd.charcode, cd.cwidth, cd.cheight, cd.xoff, cd.yoff, cd.xescape), (65, 10, 3, 0, 2, 12));
        assert_eq!(&cd.raster[..3], &[0b1111100000u16 as i32 * 64, 0b0000011111 * 64, 0b1010101010 * 64]);
        assert!(pk_read_char(&f, &mut pos, false, &mut cd).unwrap());
        assert_eq!((cd.charcode, cd.xoff, cd.yoff), (66, 1, -1));
        assert_eq!(&cd.raster[..3], &[0b1111111111u16 as i32 * 64; 3]);
        assert!(!pk_read_char(&f, &mut pos, false, &mut cd).unwrap());
    }

    /// A long-form character whose raster no `malloc` gives, 524272 by 2^31-1: kpathsea's message with the C's size, `i`
    /// halfwords of 8 bytes, and nothing allocated.
    #[test]
    fn a_raster_malloc_refuses_ends_as_xmalloc_does() {
        let mut f = pre();
        // dyn_f 13, white first, the long form: packet length, code, TFM width, dx, dy, width, height, offsets.
        f.push(0xd7);
        for v in [41, 65, 0, 0, 0, 524272, i32::MAX, 0, 0] {
            f.extend_from_slice(&v.to_be_bytes());
        }
        f.extend_from_slice(&[0x11; 8]);
        f.push(245);
        let e = pk_read_char(&f, &mut 0, true, &mut CharDesc::default()).unwrap_err();
        assert!(e.exit);
        assert_eq!(e.message, b"fatal: memory exhausted (xmalloc of 1125865546579984 bytes).\n");
    }

    /// The extended short form of a character of this width and height, black first or not, with eight bytes of runs.
    fn short_form(width: u16, height: u16, black: bool) -> Vec<u8> {
        let mut f = pre();
        f.push(13 << 4 | if black { 8 } else { 0 } | 4);
        let body = [&[65, 0, 0, 0][..], &10u16.to_be_bytes(), &width.to_be_bytes(), &height.to_be_bytes(), &[0; 4], &[0x11; 8]].concat();
        f.extend_from_slice(&(body.len() as u16).to_be_bytes());
        f.extend_from_slice(&body);
        f.push(245);
        f
    }

    /// A width of 32768 or more is negative in `hbit`, and a black run that ends the row reads `gpower` past its end in
    /// the C, which dies of it.  Here it fails as the white run does, which the raster's bound stops.
    #[test]
    fn a_row_wider_than_a_shalfword_fails() {
        let read = |f: &[u8]| pk_read_char(f, &mut 0, true, &mut CharDesc::default()).map_err(|e| e.message);
        let more = Err(b"error while unpacking; more bits than required".to_vec());
        for width in [32768, 40000, 65535] {
            assert_eq!(read(&short_form(width, 1, true)), more, "{width}");
            assert_eq!(read(&short_form(width, 1, false)), more, "{width}");
        }
        assert_eq!(read(&short_form(16, 1, true)), Ok(true));
        assert_eq!(read(&short_form(32767, 1, true)), Err(b"unexpected eof in pk file".to_vec()));
        // No row, no run read: the null glyph, as in the C.
        assert_eq!(read(&short_form(40000, 0, true)), Ok(true));
    }

    #[test]
    fn pgc_lines() {
        assert_eq!(pgc_glyph_preamble(b"65 0x10 010 -1 1 2 3 4 =\n"), Some([65, 16, 8, -1, 1, 2, 3, 4]));
        assert_eq!(pgc_glyph_preamble(b"65 1 2 3 =\n"), None);
        assert_eq!(pgc_font_scale(b"0.5\n"), Some(0.5));
        assert_eq!(pgc_font_scale(b"x\n"), None);
        assert_eq!(pgc_print_g(0.001), b"0.001");
    }
}
