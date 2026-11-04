//! Glyph outlines of a Type 1 font, for the SVG of the `commute` command: PFB or PFA in, charstrings interpreted, path segments out, in the font's own units.
//! `writet1.c` never looks inside a charstring beyond what subsetting asks, so nothing here comes from `t1.rs` but the names of StandardEncoding.
//!
//! Only `+ - * /` on numbers read from the file, so the same bytes on every platform.  Hints are skipped, as a renderer at this size has no use for them: what is drawn is the
//! outline, the same one a PDF viewer gets from the subset.

use crate::t1::STANDARD_GLYPH_NAMES;

/// One piece of a glyph's path, in the font's units, y up.
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum OutlineSeg {
    Move(f64, f64),
    Line(f64, f64),
    Curve(f64, f64, f64, f64, f64, f64),
    Close,
}

/// `eexec` and charstring encryption's constants.
const C1: u16 = 52845;
const C2: u16 = 22719;
const EEXEC_KEY: u16 = 55665;
const CS_KEY: u16 = 4330;
/// How deep a charstring may call: Type 1's own limit is 10, and a damaged font must end.
const MAX_CALLS: u32 = 16;
/// The stack of the Type 1 spec has 24 places.
const STACK: usize = 48;

fn decrypt(data: &[u8], key: u16, skip: usize, out: &mut Vec<u8>) {
    let mut r = key;
    let at = out.len();
    out.resize(at + data.len(), 0);
    for (d, &c) in out[at..].iter_mut().zip(data) {
        *d = c ^ (r >> 8) as u8;
        r = (u16::from(c).wrapping_add(r)).wrapping_mul(C1).wrapping_add(C2);
    }
    out.drain(at..at + skip.min(data.len()));
}

fn is_white(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\r' | b'\n' | 0 | 12)
}

fn find(hay: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    let tail = hay.get(from..)?;
    tail.windows(needle.len()).position(|w| w == needle).map(|i| i + from)
}

/// The next PostScript token at `*i`: a name keeps its slash, and brackets and braces stand alone.
fn token<'a>(b: &'a [u8], i: &mut usize) -> &'a [u8] {
    while *i < b.len() && is_white(b[*i]) {
        *i += 1;
    }
    let s = *i;
    if s >= b.len() {
        return &[];
    }
    if matches!(b[s], b'[' | b']' | b'{' | b'}') {
        *i += 1;
        return &b[s..s + 1];
    }
    *i += 1;
    while *i < b.len() && !is_white(b[*i]) && !matches!(b[*i], b'/' | b'[' | b']' | b'{' | b'}') {
        *i += 1;
    }
    &b[s..*i]
}

fn int_of(t: &[u8]) -> Option<i64> {
    std::str::from_utf8(t).ok()?.parse().ok()
}

fn real_of(t: &[u8]) -> Option<f64> {
    std::str::from_utf8(t).ok()?.parse().ok()
}

/// The clear part and the encrypted part of a font file.
fn split(file: &[u8]) -> Option<(Vec<u8>, Vec<u8>)> {
    if file.first() == Some(&0x80) {
        let (mut clear, mut enc) = (Vec::new(), Vec::new());
        let mut i = 0;
        while let Some(&0x80) = file.get(i) {
            let kind = *file.get(i + 1)?;
            if kind == 3 {
                break;
            }
            let n = u32::from_le_bytes(file.get(i + 2..i + 6)?.try_into().ok()?) as usize;
            let seg = file.get(i + 6..i + 6 + n)?;
            if kind == 1 && enc.is_empty() {
                clear.extend_from_slice(seg);
            } else if kind == 2 {
                enc.extend_from_slice(seg);
            }
            i += 6 + n;
        }
        return Some((clear, enc));
    }
    let at = find(file, b"eexec", 0)? + 5;
    let clear = file[..at].to_vec();
    let mut i = at;
    while i < file.len() && is_white(file[i]) {
        i += 1;
    }
    let rest = &file[i..];
    let hex = rest.len() >= 4 && rest[..4].iter().all(u8::is_ascii_hexdigit);
    if !hex {
        return Some((clear, rest.to_vec()));
    }
    let mut enc = Vec::with_capacity(rest.len() / 2);
    let mut high = None;
    for &c in rest {
        let v = match c {
            b'0'..=b'9' => c - b'0',
            b'a'..=b'f' => c - b'a' + 10,
            b'A'..=b'F' => c - b'A' + 10,
            _ if is_white(c) => continue,
            _ => break,
        };
        match high.take() {
            None => high = Some(v),
            Some(h) => enc.push(h << 4 | v),
        }
    }
    Some((clear, enc))
}

/// What a font's charstrings are, parsed once.
pub struct T1Outlines {
    clear: Vec<u8>,
    private: Vec<u8>,
    len_iv: usize,
    encrypted: bool,
    subrs: Vec<(u32, u32)>,
    /// Name's range, and the charstring's, in `private`.  A diagram asks for few: a sort would cost more than looking along it.
    glyphs: Vec<((u32, u32), (u32, u32))>,
    /// For each code, the name's range in `clear`, or none.
    encoding: [(u32, u32); 256],
    standard_encoding: bool,
    /// `/FontMatrix`.
    pub font_matrix: [f64; 6],
}

impl T1Outlines {
    /// `None` if this is no font this reader understands.
    pub fn parse(file: &[u8]) -> Option<T1Outlines> {
        let (clear, enc) = split(file)?;
        let mut private = Vec::with_capacity(enc.len());
        decrypt(&enc, EEXEC_KEY, 4, &mut private);
        let font_matrix = Self::read_matrix(&clear).unwrap_or([0.001, 0.0, 0.0, 0.001, 0.0, 0.0]);
        let mut f = T1Outlines { clear, private, len_iv: 4, encrypted: true, subrs: Vec::new(), glyphs: Vec::with_capacity(256), encoding: [(0, 0); 256], standard_encoding: false, font_matrix };
        f.read_encoding();
        f.read_private()?;
        Some(f)
    }

    fn read_matrix(clear: &[u8]) -> Option<[f64; 6]> {
        let mut i = find(clear, b"/FontMatrix", 0)? + 11;
        if token(clear, &mut i) != b"[" {
            return None;
        }
        let mut m = [0.0; 6];
        for v in m.iter_mut() {
            *v = real_of(token(clear, &mut i))?;
        }
        Some(m)
    }

    fn read_encoding(&mut self) {
        let clear = &self.clear;
        let Some(at) = find(clear, b"/Encoding", 0) else { return };
        let mut i = at + 9;
        if token(clear, &mut i) == b"StandardEncoding" {
            self.standard_encoding = true;
            return;
        }
        loop {
            let t = token(clear, &mut i);
            if t.is_empty() || t == b"def" || t == b"readonly" {
                return;
            }
            if t != b"dup" {
                continue;
            }
            let Some(code) = int_of(token(clear, &mut i)) else { continue };
            let name = token(clear, &mut i);
            if (0..256).contains(&code) && name.first() == Some(&b'/') {
                self.encoding[code as usize] = ((i - name.len() + 1) as u32, i as u32);
            }
        }
    }

    /// Along the words of the private dictionary to `/CharStrings`.  Binary data is only ever stepped over, by the length before it.
    fn read_private(&mut self) -> Option<()> {
        let b = core::mem::take(&mut self.private);
        let mut i = 0;
        loop {
            let t = token(&b, &mut i);
            if t.is_empty() {
                return None;
            }
            if t == b"/CharStrings" {
                break;
            }
            if t == b"/lenIV" {
                match int_of(token(&b, &mut i)) {
                    Some(n) if n < 0 => self.encrypted = false,
                    Some(n) => self.len_iv = n as usize,
                    None => {}
                }
            } else if t == b"/Subrs" {
                let n = int_of(token(&b, &mut i))?;
                self.subrs.resize(n.clamp(0, 65535) as usize, (0, 0));
                loop {
                    let at = i;
                    let t = token(&b, &mut i);
                    if t.is_empty() || t == b"ND" || t == b"|-" || t == b"def" {
                        break;
                    }
                    if t.first() == Some(&b'/') {
                        i = at;
                        break;
                    }
                    if t != b"dup" {
                        continue;
                    }
                    let k = int_of(token(&b, &mut i))? as usize;
                    let len = int_of(token(&b, &mut i))? as usize;
                    token(&b, &mut i); // RD or -|
                    let s = i + 1;
                    i = s + len;
                    if i > b.len() {
                        return None;
                    }
                    if let Some(r) = self.subrs.get_mut(k) {
                        *r = (s as u32, i as u32);
                    }
                }
            }
        }
        loop {
            let t = token(&b, &mut i);
            if t.is_empty() || t == b"begin" {
                break;
            }
        }
        loop {
            let at_name = {
                while i < b.len() && is_white(b[i]) {
                    i += 1;
                }
                i
            };
            let t = token(&b, &mut i);
            if t.first() != Some(&b'/') {
                break;
            }
            let name = (at_name as u32 + 1, i as u32);
            let len = int_of(token(&b, &mut i))? as usize;
            token(&b, &mut i); // RD or -|
            let s = i + 1;
            i = s + len;
            if i > b.len() {
                return None;
            }
            self.glyphs.push((name, (s as u32, i as u32)));
            token(&b, &mut i); // ND or |-, or the first word of `noaccess def`
            let save = i;
            if token(&b, &mut i) != b"def" {
                i = save;
            }
        }
        self.private = b;
        Some(())
    }

    /// The font's own name for a code, empty for none.
    pub fn encoding_name(&self, code: usize) -> &[u8] {
        if self.standard_encoding {
            return STANDARD_GLYPH_NAMES.get(code).copied().unwrap_or(&[]);
        }
        let Some(&(a, b)) = self.encoding.get(code) else { return &[] };
        &self.clear[a as usize..b as usize]
    }

    fn charstring(&self, name: &[u8]) -> Option<(usize, usize)> {
        let g = self.glyphs.iter().find(|g| &self.private[g.0.0 as usize..g.0.1 as usize] == name)?;
        Some((g.1.0 as usize, g.1.1 as usize))
    }

    /// The glyph's path, if the font has it.
    pub fn glyph(&self, name: &[u8]) -> Option<Vec<OutlineSeg>> {
        let cs = self.charstring(name)?;
        let mut it = Interp { f: self, out: Vec::with_capacity(32), st: [0.0; STACK], n: 0, ps: Vec::new(), x: 0.0, y: 0.0, sbx: 0.0, ox: 0.0, oy: 0.0, flex: None, done: false };
        it.run(cs, 0)?;
        Some(it.out)
    }
}

struct Interp<'a> {
    f: &'a T1Outlines,
    out: Vec<OutlineSeg>,
    st: [f64; STACK],
    n: usize,
    /// What `callothersubr` left for `pop`.
    ps: Vec<f64>,
    x: f64,
    y: f64,
    sbx: f64,
    /// Where `seac` puts the accent.
    ox: f64,
    oy: f64,
    /// Between the `1` and the `0` of the flex procedures: the points `rmoveto` went through.
    flex: Option<Vec<(f64, f64)>>,
    done: bool,
}

impl Interp<'_> {
    fn run(&mut self, range: (usize, usize), depth: u32) -> Option<()> {
        if depth > MAX_CALLS {
            return None;
        }
        let raw = self.f.private.get(range.0..range.1)?;
        let mut plain = Vec::with_capacity(raw.len());
        let code: &[u8] = if self.f.encrypted {
            decrypt(raw, CS_KEY, self.f.len_iv, &mut plain);
            &plain
        } else {
            raw
        };
        let mut i = 0;
        while i < code.len() && !self.done {
            let c = code[i];
            i += 1;
            match c {
                32..=246 => self.push(i32::from(c) as f64 - 139.0),
                247..=250 => {
                    let b = i32::from(*code.get(i)?);
                    i += 1;
                    self.push(f64::from((i32::from(c) - 247) * 256 + b + 108));
                }
                251..=254 => {
                    let b = i32::from(*code.get(i)?);
                    i += 1;
                    self.push(f64::from(-(i32::from(c) - 251) * 256 - b - 108));
                }
                255 => {
                    let v = i32::from_be_bytes(code.get(i..i + 4)?.try_into().ok()?);
                    i += 4;
                    self.push(f64::from(v));
                }
                1 | 3 => self.n = 0, // hstem, vstem
                4 => {
                    let d = self.arg(0);
                    self.moveto(0.0, d);
                }
                5 => {
                    let (a, b) = (self.arg(0), self.arg(1));
                    self.lineto(a, b);
                }
                6 => {
                    let a = self.arg(0);
                    self.lineto(a, 0.0);
                }
                7 => {
                    let a = self.arg(0);
                    self.lineto(0.0, a);
                }
                8 => {
                    let a = [self.arg(0), self.arg(1), self.arg(2), self.arg(3), self.arg(4), self.arg(5)];
                    self.curveto(a);
                }
                9 => {
                    self.out.push(OutlineSeg::Close);
                    self.n = 0;
                }
                10 => {
                    if self.n == 0 {
                        return None;
                    }
                    self.n -= 1;
                    let k = self.st[self.n] as usize;
                    let r = *self.f.subrs.get(k)?;
                    self.run((r.0 as usize, r.1 as usize), depth + 1)?;
                }
                11 => return Some(()), // return
                13 => {
                    self.sbx = self.arg(0);
                    self.x = self.sbx;
                    self.y = 0.0;
                    self.n = 0;
                }
                14 => {
                    self.done = true;
                    return Some(());
                }
                21 => {
                    let (a, b) = (self.arg(0), self.arg(1));
                    self.moveto(a, b);
                }
                22 => {
                    let a = self.arg(0);
                    self.moveto(a, 0.0);
                }
                30 => {
                    let a = [0.0, self.arg(0), self.arg(1), self.arg(2), self.arg(3), 0.0];
                    self.curveto(a);
                }
                31 => {
                    let a = [self.arg(0), 0.0, self.arg(1), self.arg(2), 0.0, self.arg(3)];
                    self.curveto(a);
                }
                12 => {
                    let e = *code.get(i)?;
                    i += 1;
                    match e {
                        0 | 1 | 2 => self.n = 0, // dotsection, vstem3, hstem3
                        6 => {
                            self.seac(depth)?;
                            return Some(());
                        }
                        7 => {
                            self.sbx = self.arg(0);
                            self.x = self.sbx;
                            self.y = self.arg(1);
                            self.n = 0;
                        }
                        12 => {
                            if self.n < 2 {
                                return None;
                            }
                            let v = self.st[self.n - 2] / self.st[self.n - 1];
                            self.n -= 1;
                            self.st[self.n - 1] = v;
                        }
                        16 => self.callothersubr()?,
                        17 => {
                            let v = self.ps.pop().unwrap_or(0.0);
                            self.push(v);
                        }
                        33 => {
                            self.x = self.arg(0);
                            self.y = self.arg(1);
                            self.n = 0;
                        }
                        _ => self.n = 0,
                    }
                }
                _ => self.n = 0,
            }
        }
        Some(())
    }

    #[inline]
    fn push(&mut self, v: f64) {
        if self.n < STACK {
            self.st[self.n] = v;
            self.n += 1;
        }
    }

    #[inline]
    fn arg(&self, k: usize) -> f64 {
        self.st[k]
    }

    fn moveto(&mut self, dx: f64, dy: f64) {
        self.x += dx;
        self.y += dy;
        self.n = 0;
        if let Some(pts) = &mut self.flex {
            pts.push((self.x, self.y));
        } else {
            self.out.push(OutlineSeg::Move(self.x + self.ox, self.y + self.oy));
        }
    }

    fn lineto(&mut self, dx: f64, dy: f64) {
        self.x += dx;
        self.y += dy;
        self.n = 0;
        self.out.push(OutlineSeg::Line(self.x + self.ox, self.y + self.oy));
    }

    fn curveto(&mut self, d: [f64; 6]) {
        let (x1, y1) = (self.x + d[0], self.y + d[1]);
        let (x2, y2) = (x1 + d[2], y1 + d[3]);
        self.x = x2 + d[4];
        self.y = y2 + d[5];
        self.n = 0;
        self.out.push(OutlineSeg::Curve(x1 + self.ox, y1 + self.oy, x2 + self.ox, y2 + self.oy, self.x + self.ox, self.y + self.oy));
    }

    fn callothersubr(&mut self) -> Option<()> {
        if self.n < 2 {
            return None;
        }
        let which = self.st[self.n - 1] as i32;
        let args = self.st[self.n - 2] as usize;
        self.n -= 2;
        if args > self.n {
            return None;
        }
        match which {
            1 => self.flex = Some(Vec::with_capacity(7)),
            2 => {}
            0 => {
                let pts = self.flex.take()?;
                if pts.len() < 7 {
                    return None;
                }
                let (ox, oy) = (self.ox, self.oy);
                for a in [&pts[1..4], &pts[4..7]] {
                    self.out.push(OutlineSeg::Curve(a[0].0 + ox, a[0].1 + oy, a[1].0 + ox, a[1].1 + oy, a[2].0 + ox, a[2].1 + oy));
                }
                // The height, then the point, which `pop pop setcurrentpoint` gets back.
                self.ps.clear();
                self.ps.push(self.st[self.n - 1]);
                self.ps.push(self.st[self.n - 2]);
            }
            _ => {
                // Hint replacement gives its argument back for `callsubr`, and so does an unknown one.
                self.ps.clear();
                for k in 0..args {
                    self.ps.push(self.st[self.n - 1 - k]);
                }
            }
        }
        self.n -= args;
        Some(())
    }

    /// The accented character: two glyphs of StandardEncoding.
    fn seac(&mut self, depth: u32) -> Option<()> {
        let (asb, adx, ady) = (self.arg(0), self.arg(1), self.arg(2));
        let (bchar, achar) = (self.arg(3) as usize, self.arg(4) as usize);
        self.n = 0;
        let sbx = self.sbx;
        let base = self.f.charstring(STANDARD_GLYPH_NAMES.get(bchar)?)?;
        self.run(base, depth + 1)?;
        self.done = false;
        self.ox = sbx + adx - asb;
        self.oy = ady;
        let acc = self.f.charstring(STANDARD_GLYPH_NAMES.get(achar)?)?;
        self.run(acc, depth + 1)?;
        self.done = true;
        Some(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn encrypt(data: &[u8], key: u16, lead: usize) -> Vec<u8> {
        let mut r = key;
        let mut out = Vec::new();
        for &p in std::iter::repeat_n(&0u8, lead).chain(data.iter()) {
            let c = p ^ (r >> 8) as u8;
            r = (u16::from(c).wrapping_add(r)).wrapping_mul(C1).wrapping_add(C2);
            out.push(c);
        }
        out
    }

    fn num(v: i32) -> Vec<u8> {
        if (-107..=107).contains(&v) {
            vec![(v + 139) as u8]
        } else {
            let mut o = vec![255];
            o.extend_from_slice(&v.to_be_bytes());
            o
        }
    }

    fn cs(ops: &[&[i32]]) -> Vec<u8> {
        // A list of numbers then an operator, the last entry of each; 1200 + e is an escape.
        let mut o = Vec::new();
        for g in ops {
            let (op, nums) = g.split_last().unwrap();
            for &n in nums {
                o.extend(num(n));
            }
            if *op >= 1200 {
                o.extend_from_slice(&[12, (*op - 1200) as u8]);
            } else {
                o.push(*op as u8);
            }
        }
        o
    }

    /// A font made here: its charstrings are the test.
    fn font(glyphs: &[(&str, Vec<u8>)], subrs: &[Vec<u8>], pfb: bool) -> Vec<u8> {
        let mut private = b"dup /Private 8 dict dup begin\n/RD {string currentfile exch readstring pop} executeonly def\n/lenIV 4 def\n".to_vec();
        private.extend_from_slice(format!("/Subrs {} array\n", subrs.len()).as_bytes());
        for (k, s) in subrs.iter().enumerate() {
            let e = encrypt(s, CS_KEY, 4);
            private.extend_from_slice(format!("dup {} {} RD ", k, e.len()).as_bytes());
            private.extend_from_slice(&e);
            private.extend_from_slice(b" NP\n");
        }
        private.extend_from_slice(b"ND\n");
        private.extend_from_slice(format!("2 index /CharStrings {} dict dup begin\n", glyphs.len()).as_bytes());
        for (n, s) in glyphs {
            let e = encrypt(s, CS_KEY, 4);
            private.extend_from_slice(format!("/{} {} RD ", n, e.len()).as_bytes());
            private.extend_from_slice(&e);
            private.extend_from_slice(b" ND\n");
        }
        private.extend_from_slice(b"end\nend\nreadonly put\nnoaccess put\ndup /FontName get exch definefont pop\nmark currentfile closefile\n");
        let clear = b"%!PS-AdobeFont-1.0: X\n/FontMatrix [0.001 0 0 0.001 0 0] readonly def\n/Encoding 256 array\n0 1 255 {1 index exch /.notdef put} for\ndup 65 /A put\ndup 66 /B put\nreadonly def\ncurrentfile eexec\n".to_vec();
        let enc = encrypt(&private, EEXEC_KEY, 4);
        let mut out = Vec::new();
        if pfb {
            out.extend_from_slice(&[0x80, 1]);
            out.extend_from_slice(&(clear.len() as u32).to_le_bytes());
            out.extend_from_slice(&clear);
            out.extend_from_slice(&[0x80, 2]);
            out.extend_from_slice(&(enc.len() as u32).to_le_bytes());
            out.extend_from_slice(&enc);
            out.extend_from_slice(&[0x80, 3]);
        } else {
            out.extend_from_slice(&clear);
            for b in &enc {
                out.extend_from_slice(format!("{b:02x}").as_bytes());
            }
            out.push(b'\n');
        }
        out
    }

    const HSBW: i32 = 13;
    const RMOVETO: i32 = 21;
    const RLINETO: i32 = 5;
    const RRCURVETO: i32 = 8;
    const CLOSEPATH: i32 = 9;
    const ENDCHAR: i32 = 14;
    const CALLSUBR: i32 = 10;
    const RETURN: i32 = 11;

    #[test]
    fn a_square_and_its_side_bearing() {
        let a = cs(&[&[50, 500, HSBW], &[10, 20, RMOVETO], &[100, 0, RLINETO], &[0, 100, RLINETO], &[CLOSEPATH], &[ENDCHAR]]);
        for pfb in [true, false] {
            let f = T1Outlines::parse(&font(&[("A", a.clone()), (".notdef", cs(&[&[0, 0, HSBW], &[ENDCHAR]]))], &[], pfb)).unwrap();
            assert_eq!(f.encoding_name(65), b"A");
            assert_eq!(f.encoding_name(67), b"");
            assert_eq!(f.font_matrix[0], 0.001);
            let g = f.glyph(b"A").unwrap();
            assert_eq!(g, vec![OutlineSeg::Move(60.0, 20.0), OutlineSeg::Line(160.0, 20.0), OutlineSeg::Line(160.0, 120.0), OutlineSeg::Close]);
            assert!(f.glyph(b"nosuch").is_none());
        }
    }

    #[test]
    fn subroutines_and_div() {
        // 400 2 div is 200, through a subroutine.
        let subr = cs(&[&[100, 100, RLINETO], &[RETURN]]);
        let mut body = cs(&[&[0, 500, HSBW], &[0, 0, RMOVETO]]);
        body.extend(num(0));
        body.push(CALLSUBR as u8);
        body.extend(num(400));
        body.extend(num(2));
        body.extend_from_slice(&[12, 12]);
        body.extend(num(0));
        body.push(RLINETO as u8);
        body.push(ENDCHAR as u8);
        let f = T1Outlines::parse(&font(&[("A", body)], &[subr], true)).unwrap();
        assert_eq!(f.glyph(b"A").unwrap(), vec![OutlineSeg::Move(0.0, 0.0), OutlineSeg::Line(100.0, 100.0), OutlineSeg::Line(300.0, 100.0)]);
    }

    #[test]
    fn a_curve_in_each_short_form() {
        let a = cs(&[&[0, 500, HSBW], &[0, 0, RMOVETO], &[1, 2, 3, 4, 5, 6, RRCURVETO], &[10, 20, 30, 40, 30], &[10, 20, 30, 40, 31], &[ENDCHAR]]);
        let f = T1Outlines::parse(&font(&[("A", a)], &[], true)).unwrap();
        assert_eq!(
            f.glyph(b"A").unwrap(),
            vec![
                OutlineSeg::Move(0.0, 0.0),
                OutlineSeg::Curve(1.0, 2.0, 4.0, 6.0, 9.0, 12.0),
                OutlineSeg::Curve(9.0, 22.0, 29.0, 52.0, 69.0, 52.0),
                OutlineSeg::Curve(79.0, 52.0, 99.0, 82.0, 99.0, 122.0),
            ]
        );
    }

    #[test]
    fn flex_gives_two_curves_and_the_right_point() {
        // The usual sequence: 1 callothersubr, then seven rmoveto each with 2 callothersubr, then 0 callothersubr with its three arguments.
        const OTHER: i32 = 1216;
        const POP: i32 = 1217;
        const SETCURRENT: i32 = 1233;
        let mut g: Vec<Vec<i32>> = vec![vec![0, 500, HSBW], vec![0, 0, RMOVETO], vec![0, 1, OTHER]];
        for (dx, dy) in [(50, 0), (-40, 5), (10, 5), (30, 0), (10, 0), (10, -5), (30, -5)] {
            g.push(vec![dx, dy, RMOVETO]);
            g.push(vec![0, 2, OTHER]);
        }
        g.push(vec![50, 100, 0, 3, 0, OTHER]);
        g.push(vec![POP]);
        g.push(vec![POP]);
        g.push(vec![SETCURRENT]);
        g.push(vec![5, 5, RLINETO]);
        g.push(vec![ENDCHAR]);
        let refs: Vec<&[i32]> = g.iter().map(Vec::as_slice).collect();
        let f = T1Outlines::parse(&font(&[("A", cs(&refs))], &[], true)).unwrap();
        assert_eq!(
            f.glyph(b"A").unwrap(),
            vec![
                OutlineSeg::Move(0.0, 0.0),
                OutlineSeg::Curve(10.0, 5.0, 20.0, 10.0, 50.0, 10.0),
                OutlineSeg::Curve(60.0, 10.0, 70.0, 5.0, 100.0, 0.0),
                OutlineSeg::Line(105.0, 5.0),
            ]
        );
    }

    #[test]
    fn an_accent_goes_where_seac_says() {
        const SEAC: i32 = 1206;
        // Code 65 is A and 96 is quoteleft in StandardEncoding.
        let base = cs(&[&[10, 500, HSBW], &[0, 0, RMOVETO], &[1, 1, RLINETO], &[ENDCHAR]]);
        let acc = cs(&[&[20, 300, HSBW], &[0, 0, RMOVETO], &[2, 2, RLINETO], &[ENDCHAR]]);
        let both = cs(&[&[30, 500, HSBW], &[20, 100, 200, 65, 96, SEAC]]);
        let f = T1Outlines::parse(&font(&[("A", base), ("quoteleft", acc), ("Aacute", both)], &[], true)).unwrap();
        assert_eq!(
            f.glyph(b"Aacute").unwrap(),
            vec![OutlineSeg::Move(10.0, 0.0), OutlineSeg::Line(11.0, 1.0), OutlineSeg::Move(130.0, 200.0), OutlineSeg::Line(132.0, 202.0)]
        );
    }

    #[test]
    fn nonsense_is_no_font() {
        assert!(T1Outlines::parse(b"").is_none());
        assert!(T1Outlines::parse(b"%!PS not a font").is_none());
        assert!(T1Outlines::parse(&[0x80, 1, 255, 255, 255, 255]).is_none());
    }
}
