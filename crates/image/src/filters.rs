//! xpdf's stream decoders (Stream.cc), run to the end at once instead of a character at a time.
//!
//! None of them fails: damaged data ends the output where xpdf's `getChar` would return EOF, so these return what was
//! decoded.  Each keeps the oddities of the original, because a page's content array is decoded through them and the
//! bytes are copied to the output.

use crate::enc_tables::{FLATE_FIXED_DIST, FLATE_FIXED_LIT};

const DECOMPRESSION_BOMB_SIZE: u64 = 50000000;
const DECOMPRESSION_BOMB_RATIO: u64 = 200;
const GFX_COLOR_MAX_COMPS: i32 = 32;

fn is_c_space(c: i32) -> bool {
    matches!(c, 0x20 | 0x09..=0x0d)
}

/// `Lexer::isSpace`.
pub(crate) fn lexer_is_space(c: i32) -> bool {
    matches!(c, 0 | 9 | 10 | 12 | 13 | 32)
}

/// `StreamPredictor` over a whole decoded stream.  Returns `None` when the predictor is not ok, in which case xpdf
/// drops it and reads the stream as it is.
pub(crate) fn predict(input: &[u8], predictor: i32, width: i32, n_comps: i32, n_bits: i32) -> Option<Vec<u8>> {
    let n_vals = width.wrapping_mul(n_comps);
    let pix_bytes = (n_comps.wrapping_mul(n_bits).wrapping_add(7)) >> 3;
    let row_bytes = ((n_vals.wrapping_mul(n_bits).wrapping_add(7)) >> 3).wrapping_add(pix_bytes);
    if width <= 0
        || n_comps <= 0
        || n_bits <= 0
        || n_comps > GFX_COLOR_MAX_COMPS
        || n_bits > 16
        || width >= i32::MAX / n_comps
        || n_vals >= (i32::MAX - 7) / n_bits
    {
        return None;
    }
    let (pix_bytes, row_bytes) = (pix_bytes as usize, row_bytes as usize);
    let mut pred_line = vec![0u8; row_bytes];
    let mut out = Vec::with_capacity(input.len());
    let mut pos = 0usize;
    let mut up_left = [0u8; GFX_COLOR_MAX_COMPS as usize * 2 + 1];
    loop {
        // getNextLine
        let cur_pred: i32;
        if predictor >= 10 {
            if pos >= input.len() {
                return Some(out);
            }
            cur_pred = input[pos] as i32 + 10;
            pos += 1;
        } else {
            cur_pred = predictor;
        }
        up_left[..=pix_bytes].fill(0);
        let mut i = pix_bytes;
        while i < row_bytes {
            for j in (1..=pix_bytes).rev() {
                up_left[j] = up_left[j - 1];
            }
            up_left[0] = pred_line[i];
            if pos >= input.len() {
                if i > pix_bytes {
                    break;
                }
                return Some(out);
            }
            let c = input[pos] as i32;
            pos += 1;
            let v = match cur_pred {
                11 => pred_line[i - pix_bytes] as i32 + c,
                12 => pred_line[i] as i32 + c,
                13 => ((pred_line[i - pix_bytes] as i32 + pred_line[i] as i32) >> 1) + c,
                14 => {
                    let left = pred_line[i - pix_bytes] as i32;
                    let up = pred_line[i] as i32;
                    let upleft = up_left[pix_bytes] as i32;
                    let p = left + up - upleft;
                    let (pa, pb, pc) = ((p - left).abs(), (p - up).abs(), (p - upleft).abs());
                    if pa <= pb && pa <= pc {
                        left + c
                    } else if pb <= pc {
                        up + c
                    } else {
                        upleft + c
                    }
                }
                _ => c,
            };
            pred_line[i] = v as u8;
            i += 1;
        }
        if predictor == 2 {
            if n_bits == 8 {
                let nc = n_comps as usize;
                for i in pix_bytes..row_bytes {
                    pred_line[i] = pred_line[i].wrapping_add(pred_line[i - nc]);
                }
            } else if n_bits == 16 {
                let nc = n_comps as usize;
                let mut i = pix_bytes;
                while i < row_bytes {
                    let c = ((pred_line[i] as i32 + pred_line[i - 2 * nc] as i32) << 8)
                        + pred_line[i + 1] as i32
                        + pred_line[i + 1 - 2 * nc] as i32;
                    pred_line[i] = (c >> 8) as u8;
                    pred_line[i + 1] = c as u8;
                    i += 2;
                }
            } else {
                up_left[..n_comps as usize].fill(0);
                let bit_mask: u64 = (1u64 << n_bits) - 1;
                let (mut in_buf, mut out_buf) = (0u64, 0u64);
                let (mut in_bits, mut out_bits) = (0i32, 0i32);
                let (mut j, mut k) = (pix_bytes, pix_bytes);
                for _ in 0..width {
                    for kk in 0..n_comps as usize {
                        if in_bits < n_bits {
                            in_buf = (in_buf << 8) | (pred_line[j] as u64 & 0xff);
                            j += 1;
                            in_bits += 8;
                        }
                        up_left[kk] = (((up_left[kk] as u64).wrapping_add(in_buf >> (in_bits - n_bits))) & bit_mask) as u8;
                        in_bits -= n_bits;
                        out_buf = (out_buf << n_bits) | up_left[kk] as u64;
                        out_bits += n_bits;
                        if out_bits >= 8 {
                            pred_line[k] = (out_buf >> (out_bits - 8)) as u8;
                            k += 1;
                            out_bits -= 8;
                        }
                    }
                }
                if out_bits > 0 {
                    pred_line[k] = ((out_buf << (8 - out_bits)) + (in_buf & ((1u64 << (8 - out_bits)) - 1))) as u8;
                }
            }
        }
        out.extend_from_slice(&pred_line[pix_bytes..row_bytes]);
    }
}

/// `ASCIIHexStream`.
pub(crate) fn ascii_hex(input: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(input.len() / 2);
    let mut pos = 0usize;
    let getc = |p: &mut usize| -> i32 {
        if *p < input.len() {
            let c = input[*p] as i32;
            *p += 1;
            c
        } else {
            -1
        }
    };
    let mut eof = false;
    loop {
        if eof {
            return out;
        }
        let mut c1;
        loop {
            c1 = getc(&mut pos);
            if !is_c_space(c1) {
                break;
            }
        }
        if c1 == b'>' as i32 {
            return out;
        }
        let mut c2;
        loop {
            c2 = getc(&mut pos);
            if !is_c_space(c2) {
                break;
            }
        }
        if c2 == b'>' as i32 {
            eof = true;
            c2 = b'0' as i32;
        }
        let mut x: i32;
        if (b'0' as i32..=b'9' as i32).contains(&c1) {
            x = (c1 - b'0' as i32) << 4;
        } else if (b'A' as i32..=b'F' as i32).contains(&c1) {
            x = (c1 - b'A' as i32 + 10) << 4;
        } else if (b'a' as i32..=b'f' as i32).contains(&c1) {
            x = (c1 - b'a' as i32 + 10) << 4;
        } else if c1 == -1 {
            eof = true;
            x = 0;
        } else {
            x = 0;
        }
        if (b'0' as i32..=b'9' as i32).contains(&c2) {
            x += c2 - b'0' as i32;
        } else if (b'A' as i32..=b'F' as i32).contains(&c2) {
            x += c2 - b'A' as i32 + 10;
        } else if (b'a' as i32..=b'f' as i32).contains(&c2) {
            x += c2 - b'a' as i32 + 10;
        } else if c2 == -1 {
            eof = true;
            x = 0;
        }
        out.push((x & 0xff) as u8);
    }
}

/// `ASCII85Stream`.
pub(crate) fn ascii85(input: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(input.len() * 4 / 5);
    let mut pos = 0usize;
    let getc = |p: &mut usize| -> i32 {
        if *p < input.len() {
            let c = input[*p] as i32;
            *p += 1;
            c
        } else {
            -1
        }
    };
    let (mut index, mut n) = (0usize, 0usize);
    let mut eof = false;
    let mut c = [0i32; 5];
    let mut b = [0u8; 4];
    loop {
        if index >= n {
            if eof {
                return out;
            }
            index = 0;
            loop {
                c[0] = getc(&mut pos);
                if !lexer_is_space(c[0]) {
                    break;
                }
            }
            if c[0] == b'~' as i32 || c[0] == -1 {
                return out;
            } else if c[0] == b'z' as i32 {
                b = [0; 4];
                n = 4;
            } else {
                let mut k = 1;
                while k < 5 {
                    loop {
                        c[k] = getc(&mut pos);
                        if !lexer_is_space(c[k]) {
                            break;
                        }
                    }
                    if c[k] == b'~' as i32 || c[k] == -1 {
                        break;
                    }
                    k += 1;
                }
                n = k - 1;
                if k < 5 && (c[k] == b'~' as i32 || c[k] == -1) {
                    for e in c.iter_mut().skip(k + 1) {
                        *e = 0x21 + 84;
                    }
                    eof = true;
                    // c[k] itself goes into the sum, as in the C.
                }
                let mut t: u64 = 0;
                for &e in &c {
                    t = t.wrapping_mul(85).wrapping_add((e - 0x21) as i64 as u64);
                }
                for k in (0..4).rev() {
                    b[k] = (t & 0xff) as u8;
                    t >>= 8;
                }
            }
        }
        // getChar returns b[index] even when n came out 0.
        out.push(b[index.min(3)]);
        index += 1;
    }
}

/// `LZWStream`, without its predictor.
pub(crate) fn lzw(input: &[u8], early: i32) -> Vec<u8> {
    #[derive(Clone, Copy, Default)]
    struct Entry {
        length: i32,
        head: i32,
        tail: u8,
    }
    let mut table = vec![Entry::default(); 4097];
    let mut seq_buf = vec![0u8; 4097];
    let mut out = Vec::with_capacity(input.len() * 2);
    let mut pos = 0usize;
    let (mut input_buf, mut input_bits): (u32, i32) = (0, 0);
    let mut next_code: i32 = 258;
    let mut next_bits: i32 = 9;
    let mut seq_length: i32 = 0;
    let mut first = true;
    let mut new_char: i32 = 0;
    let mut prev_code: i32 = 0;
    let mut total_in: u64 = 0;
    let mut total_out: u64 = 0;
    loop {
        // getCode
        let code: i32;
        loop {
            if input_bits >= next_bits {
                break;
            }
            if pos >= input.len() {
                return out;
            }
            input_buf = (input_buf << 8) | input[pos] as u32;
            pos += 1;
            input_bits += 8;
            total_in += 1;
        }
        code = ((input_buf >> (input_bits - next_bits)) & ((1u32 << next_bits) - 1)) as i32;
        input_bits -= next_bits;
        if code == 257 {
            return out;
        }
        if code == 256 {
            next_code = 258;
            next_bits = 9;
            seq_length = 0;
            first = true;
            continue;
        }
        if next_code >= 4097 {
            next_code = 258;
            next_bits = 9;
            seq_length = 0;
            first = true;
        }
        let next_length = seq_length + 1;
        if code < 256 {
            seq_buf[0] = code as u8;
            seq_length = 1;
        } else if code < next_code {
            seq_length = table[code as usize].length;
            let mut i = seq_length - 1;
            let mut j = code;
            while i > 0 {
                seq_buf[i as usize] = table[j as usize].tail;
                j = table[j as usize].head;
                i -= 1;
            }
            seq_buf[0] = j as u8;
        } else if code == next_code {
            seq_buf[seq_length as usize] = new_char as u8;
            seq_length += 1;
        } else {
            return out;
        }
        new_char = seq_buf[0] as i32;
        if first {
            first = false;
        } else {
            let e = &mut table[next_code as usize];
            e.length = next_length;
            e.head = prev_code;
            e.tail = new_char as u8;
            next_code += 1;
            if next_code + early == 512 {
                next_bits = 10;
            } else if next_code + early == 1024 {
                next_bits = 11;
            } else if next_code + early == 2048 {
                next_bits = 12;
            }
        }
        prev_code = code;
        total_out += seq_length as u64;
        if total_out > DECOMPRESSION_BOMB_SIZE && total_in < total_out / DECOMPRESSION_BOMB_RATIO {
            return out;
        }
        out.extend_from_slice(&seq_buf[..seq_length as usize]);
    }
}

/// `RunLengthStream`.
pub(crate) fn run_length(input: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(input.len());
    let mut pos = 0usize;
    // (char)str->getChar(): -1 is 0xff.
    let getc = |p: &mut usize| -> u8 {
        if *p < input.len() {
            let c = input[*p];
            *p += 1;
            c
        } else {
            0xff
        }
    };
    loop {
        if pos >= input.len() {
            return out;
        }
        let c = input[pos] as i32;
        pos += 1;
        if c == 0x80 {
            return out;
        }
        if c < 0x80 {
            for _ in 0..c + 1 {
                out.push(getc(&mut pos));
            }
        } else {
            let n = 0x101 - c;
            let v = getc(&mut pos);
            for _ in 0..n {
                out.push(v);
            }
        }
    }
}

const FLATE_WINDOW: usize = 32768;
const FLATE_MASK: usize = FLATE_WINDOW - 1;
const FLATE_MAX_LIT_CODES: i32 = 288;
const FLATE_MAX_DIST_CODES: i32 = 30;
const FLATE_MAX_CODE_LEN_CODES: usize = 19;

const CODE_LEN_CODE_MAP: [usize; FLATE_MAX_CODE_LEN_CODES] = [16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15];

const LENGTH_DECODE: [(i32, i32); 31] = [
    (0, 3), (0, 4), (0, 5), (0, 6), (0, 7), (0, 8), (0, 9), (0, 10), (1, 11), (1, 13), (1, 15), (1, 17), (2, 19), (2, 23),
    (2, 27), (2, 31), (3, 35), (3, 43), (3, 51), (3, 59), (4, 67), (4, 83), (4, 99), (4, 115), (5, 131), (5, 163), (5, 195),
    (5, 227), (0, 258), (0, 258), (0, 258),
];

const DIST_DECODE: [(i32, i32); 30] = [
    (0, 1), (0, 2), (0, 3), (0, 4), (1, 5), (1, 7), (2, 9), (2, 13), (3, 17), (3, 25), (4, 33), (4, 49), (5, 65), (5, 97),
    (6, 129), (6, 193), (7, 257), (7, 385), (8, 513), (8, 769), (9, 1025), (9, 1537), (10, 2049), (10, 3073), (11, 4097),
    (11, 6145), (12, 8193), (12, 12289), (13, 16385), (13, 24577),
];

#[derive(Default)]
struct HuffTab {
    codes: Vec<(u16, u16)>,
    max_len: i32,
}

struct Flate<'a> {
    input: &'a [u8],
    pos: usize,
    code_buf: u64,
    code_size: i32,
    total_in: u64,
}

impl Flate<'_> {
    fn getc(&mut self) -> i32 {
        if self.pos < self.input.len() {
            let c = self.input[self.pos] as i32;
            self.pos += 1;
            c
        } else {
            -1
        }
    }

    /// `getHuffmanCodeWord`.
    fn huffman(&mut self, tab: &HuffTab) -> i32 {
        while self.code_size < tab.max_len {
            let c = self.getc();
            if c < 0 {
                break;
            }
            self.code_buf |= (c as u64) << self.code_size;
            self.code_size += 8;
            self.total_in += 1;
        }
        let (len, val) = tab.codes[(self.code_buf & ((1u64 << tab.max_len) - 1)) as usize];
        if self.code_size == 0 || self.code_size < len as i32 || len == 0 {
            return -1;
        }
        self.code_buf >>= len;
        self.code_size -= len as i32;
        val as i32
    }

    /// `getCodeWord`.
    fn bits(&mut self, bits: i32) -> i32 {
        while self.code_size < bits {
            let c = self.getc();
            if c < 0 {
                return -1;
            }
            self.code_buf |= (c as u64) << self.code_size;
            self.code_size += 8;
            self.total_in += 1;
        }
        let c = (self.code_buf & ((1u64 << bits) - 1)) as i32;
        self.code_buf >>= bits;
        self.code_size -= bits;
        c
    }

    /// `readDynamicCodes`.
    fn read_dynamic_codes(&mut self, lit: &mut HuffTab, dist: &mut HuffTab) -> bool {
        let mut num_lit = self.bits(5);
        if num_lit < 0 {
            return false;
        }
        num_lit += 257;
        let mut num_dist = self.bits(5);
        if num_dist < 0 {
            return false;
        }
        num_dist += 1;
        let mut num_clen = self.bits(4);
        if num_clen < 0 {
            return false;
        }
        num_clen += 4;
        if num_lit > FLATE_MAX_LIT_CODES || num_dist > FLATE_MAX_DIST_CODES || num_clen as usize > FLATE_MAX_CODE_LEN_CODES {
            return false;
        }
        let mut clen = [0i32; FLATE_MAX_CODE_LEN_CODES];
        for i in 0..num_clen as usize {
            let v = self.bits(3);
            if v == -1 {
                return false;
            }
            clen[CODE_LEN_CODE_MAP[i]] = v;
        }
        let clen_tab = comp_huffman_codes(&clen);
        let total = (num_lit + num_dist) as usize;
        let mut lengths = vec![0i32; total];
        let mut len = 0;
        let mut i = 0usize;
        while i < total {
            let code = self.huffman(&clen_tab);
            if code == -1 {
                return false;
            }
            match code {
                16 => {
                    let r = self.bits(2);
                    if r == -1 {
                        return false;
                    }
                    let r = r as usize + 3;
                    if i + r > total {
                        return false;
                    }
                    lengths[i..i + r].fill(len);
                    i += r;
                }
                17 => {
                    let r = self.bits(3);
                    if r == -1 {
                        return false;
                    }
                    let r = r as usize + 3;
                    if i + r > total {
                        return false;
                    }
                    len = 0;
                    i += r;
                }
                18 => {
                    let r = self.bits(7);
                    if r == -1 {
                        return false;
                    }
                    let r = r as usize + 11;
                    if i + r > total {
                        return false;
                    }
                    len = 0;
                    i += r;
                }
                _ => {
                    len = code;
                    lengths[i] = code;
                    i += 1;
                }
            }
        }
        *lit = comp_huffman_codes(&lengths[..num_lit as usize]);
        *dist = comp_huffman_codes(&lengths[num_lit as usize..]);
        true
    }
}

/// `compHuffmanCodes`.
fn comp_huffman_codes(lengths: &[i32]) -> HuffTab {
    let max_len = lengths.iter().copied().max().unwrap_or(0).max(0);
    let tab_size = 1usize << max_len;
    let mut codes = vec![(0u16, 0u16); tab_size];
    let mut code = 0u32;
    let mut skip = 2usize;
    for len in 1..=max_len {
        for (val, &l) in lengths.iter().enumerate() {
            if l == len {
                let mut code2 = 0u32;
                let mut t = code;
                for _ in 0..len {
                    code2 = (code2 << 1) | (t & 1);
                    t >>= 1;
                }
                let mut i = code2 as usize;
                while i < tab_size {
                    codes[i] = (len as u16, val as u16);
                    i += skip;
                }
                code += 1;
            }
        }
        code <<= 1;
        skip <<= 1;
    }
    HuffTab { codes, max_len }
}

/// `FlateStream`, without its predictor.
pub(crate) fn flate(input: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(input.len() * 3);
    // reset: read the header
    if input.len() < 2 {
        return out;
    }
    let (cmf, flg) = (input[0] as i32, input[1] as i32);
    if cmf & 0x0f != 0x08 || ((cmf << 8) + flg) % 31 != 0 || flg & 0x20 != 0 {
        return out;
    }
    let mut f = Flate { input, pos: 2, code_buf: 0, code_size: 0, total_in: 2 };
    let mut buf = vec![0u8; FLATE_WINDOW];
    let mut index = 0usize;
    let mut total_out = 0u64;
    let mut end_of_block = true;
    let mut eof = false;
    let mut compressed = false;
    let mut block_len = 0i32;
    let mut lit = HuffTab::default();
    let mut dist = HuffTab::default();
    let fixed_lit = HuffTab { codes: FLATE_FIXED_LIT.to_vec(), max_len: 9 };
    let fixed_dist = HuffTab { codes: FLATE_FIXED_DIST.to_vec(), max_len: 5 };
    loop {
        // getChar: while (remain == 0) { if (endOfBlock && eof) return EOF; readSome(); }
        if end_of_block && eof {
            return out;
        }
        // readSome
        if end_of_block {
            // startBlock.  A failed getCodeWord gives -1 here: last block, and type 3 after the shift.
            let mut hdr = f.bits(3);
            if hdr & 1 != 0 {
                eof = true;
            }
            hdr >>= 1;
            let mut ok = true;
            if hdr == 0 {
                compressed = false;
                let mut b = [0i32; 4];
                for slot in b.iter_mut() {
                    *slot = f.getc();
                    if *slot < 0 {
                        ok = false;
                        break;
                    }
                }
                if ok {
                    block_len = b[0] | (b[1] << 8);
                    let check = b[2] | (b[3] << 8);
                    if check != (!block_len & 0xffff) {
                        ok = false;
                    }
                }
                if ok {
                    f.code_buf = 0;
                    f.code_size = 0;
                    f.total_in += 4;
                }
            } else if hdr == 1 {
                compressed = true;
                lit = HuffTab { codes: Vec::new(), max_len: -1 };
                dist = HuffTab { codes: Vec::new(), max_len: -1 };
            } else if hdr == 2 {
                compressed = true;
                ok = f.read_dynamic_codes(&mut lit, &mut dist);
            } else {
                ok = false;
            }
            if !ok {
                // endOfBlock = eof = gTrue; return gFalse: readSome returns, and the outer loop ends.
                return out;
            }
            end_of_block = false;
        }
        let remain: usize;
        if compressed {
            let lt = if lit.max_len < 0 { &fixed_lit } else { &lit };
            let dt = if dist.max_len < 0 { &fixed_dist } else { &dist };
            let code1 = f.huffman(lt);
            if code1 < 0 {
                return out;
            }
            if code1 < 256 {
                buf[index] = code1 as u8;
                remain = 1;
            } else if code1 == 256 {
                end_of_block = true;
                remain = 0;
            } else {
                let idx = (code1 - 257) as usize;
                let (lbits, lfirst) = LENGTH_DECODE[idx];
                let mut code2 = lbits;
                if code2 > 0 {
                    code2 = f.bits(code2);
                    if code2 < 0 {
                        return out;
                    }
                }
                let len = (lfirst + code2) as usize;
                let dcode = f.huffman(dt);
                if dcode < 0 {
                    return out;
                }
                // Codes 30 and 31 cannot occur: the fixed table has no entry for them and a dynamic one is limited to 30.
                let (dbits, dfirst) = DIST_DECODE[dcode as usize];
                let mut code3 = dbits;
                if code3 > 0 {
                    code3 = f.bits(code3);
                    if code3 < 0 {
                        return out;
                    }
                }
                let d = (dfirst + code3) as usize;
                let src = index.wrapping_sub(d) & FLATE_MASK;
                for k in 0..len {
                    buf[(index + k) & FLATE_MASK] = buf[(src + k) & FLATE_MASK];
                }
                remain = len;
            }
        } else {
            let len = block_len.min(FLATE_WINDOW as i32) as usize;
            let mut i = 0usize;
            let mut j = index;
            while i < len {
                let c = f.getc();
                if c < 0 {
                    end_of_block = true;
                    eof = true;
                    break;
                }
                buf[j] = c as u8;
                i += 1;
                j = (j + 1) & FLATE_MASK;
            }
            remain = i;
            block_len -= len as i32;
            if block_len == 0 {
                end_of_block = true;
            }
            f.total_in += remain as u64;
        }
        total_out += remain as u64;
        if total_out > DECOMPRESSION_BOMB_SIZE && f.total_in < total_out / DECOMPRESSION_BOMB_RATIO {
            return out;
        }
        for _ in 0..remain {
            out.push(buf[index]);
            index = (index + 1) & FLATE_MASK;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn deflated(input: &[u8]) -> Vec<u8> {
        let mut out = vec![0u8; zlib_rs::compress_bound(input.len())];
        let config = zlib_rs::DeflateConfig::new(6);
        let (c, rc) = zlib_rs::compress_slice(&mut out, input, config);
        assert!(rc == zlib_rs::ReturnCode::Ok);
        c.to_vec()
    }

    #[test]
    fn flate_round_trip() {
        let mut input = Vec::new();
        for i in 0..200000u32 {
            input.push((i * i >> 7) as u8);
        }
        assert_eq!(flate(&deflated(&input)), input);
        assert_eq!(flate(&deflated(b"")), b"");
        assert_eq!(flate(&deflated(b"a")), b"a");
    }

    #[test]
    fn flate_truncated_gives_what_it_has() {
        let input = vec![b'a'; 100000];
        let z = deflated(&input);
        let got = flate(&z[..z.len() / 2]);
        assert!(got.len() < input.len());
        assert!(got.iter().all(|&c| c == b'a'));
        assert!(flate(b"x").is_empty());
        assert!(flate(b"not a zlib stream").is_empty());
    }

    #[test]
    fn hex_and_85() {
        assert_eq!(ascii_hex(b"48 65 6c6C 6f>"), b"Hello");
        assert_eq!(ascii_hex(b"4>"), b"\x40");
        assert_eq!(ascii_hex(b"48"), b"H\0");
        assert_eq!(ascii85(b"87cURDZ~>"), b"Hello");
        assert_eq!(ascii85(b"z~>"), b"\0\0\0\0");
    }

    #[test]
    fn run_length_and_lzw() {
        assert_eq!(run_length(&[2, b'a', b'b', b'c', 254, b'x', 128, 9]), b"abcxxx");
        // The PDF reference's example: 45 45 45 45 45 65 45 45 45 66.
        let code = [0x80u8, 0x0B, 0x60, 0x50, 0x22, 0x0C, 0x0C, 0x85, 0x01];
        assert_eq!(lzw(&code, 1), [45, 45, 45, 45, 45, 65, 45, 45, 45, 66]);
    }

    #[test]
    fn png_predictors() {
        let input = [2, 1, 2, 3, 1, 1, 1, 1];
        assert_eq!(predict(&input, 12, 3, 1, 8).unwrap(), [1, 2, 3, 1, 2, 3]);
        assert!(predict(&input, 12, 0, 1, 8).is_none());
        assert!(predict(&input, 12, 1, 1, 32).is_none());
    }
}
