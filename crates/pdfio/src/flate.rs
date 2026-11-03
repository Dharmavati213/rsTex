//! Deflate and inflate through `zlib-rs`, with the zlib wrapper that PDF's `/FlateDecode` means.
use zlib_rs::{Deflate, DeflateFlush, Inflate, InflateFlush, Status};

/// Content streams and object streams (DESIGN.md section 9).
pub const LEVEL_CONTENT: i32 = 6;
/// `FontFile` streams: eexec data is incompressible, so effort there is wasted.
pub const LEVEL_FONTFILE: i32 = 1;

/// What a stream holds, which decides the effort spent on it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Stream {
    /// Page contents, form XObjects, object streams, and everything else.
    Content,
    /// An embedded font program.
    FontFile,
}

/// The zlib level for a stream under `\pdfcompresslevel`.  `None` means do not compress: at level 0
/// WEB does not go through `write_zip` at all.  Any other value gives the level of DESIGN.md section 9:
/// the document's choice between 1 and 9 is a size-for-time trade that rstex makes itself, and the
/// compressed bytes are not comparable with the oracle's in any case.
pub fn level_for(pdf_compress_level: i32, stream: Stream) -> Option<i32> {
    if pdf_compress_level <= 0 {
        return None;
    }
    Some(match stream {
        Stream::Content => LEVEL_CONTENT,
        Stream::FontFile => LEVEL_FONTFILE,
    })
}

/// Window size, as `deflateInit` and `inflateInit` use by default.
const WINDOW_BITS: u8 = 15;

/// Output space to add at a time.  `writezip.c`'s `ZIP_BUF_SIZE`.
const OUT_STEP: usize = 32768;

/// Deflates `input` as one complete zlib stream at `level`, appending to `out`.
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

/// A zlib stream written in pieces: `writezip.c`.  Call [`Deflater::write`] for each piece and
/// [`Deflater::finish`] for the last; it is then ready for the next stream at the same level.
pub struct Deflater {
    inner: Deflate,
    level: i32,
}

impl Deflater {
    /// `deflateInit`.
    pub fn new(level: i32) -> Deflater {
        Deflater { inner: Deflate::new(level, true, WINDOW_BITS), level }
    }

    /// The level this stream is compressed at.
    pub fn level(&self) -> i32 {
        self.level
    }

    /// Starts a new stream, at another level if asked: `deflateReset`, or `deflateEnd` and
    /// `deflateInit` when `\pdfcompresslevel` changed in mid document.
    pub fn reset(&mut self, level: i32) {
        if level == self.level {
            self.inner.reset();
        } else {
            *self = Deflater::new(level);
        }
    }

    /// `deflate (Z_NO_FLUSH)`: compresses a piece, appending what is ready to `out`.
    pub fn write(&mut self, input: &[u8], out: &mut Vec<u8>) {
        self.run(input, out, DeflateFlush::NoFlush);
    }

    /// A whole stream in one piece, with the deflater kept in `slot` for the next one, as `writezip.c` keeps its `z_stream`.
    /// A deflater that has been used gives the same bytes as a new one.
    pub fn whole(slot: &mut Option<Deflater>, input: &[u8], level: i32, out: &mut Vec<u8>) {
        let d = slot.get_or_insert_with(|| Deflater::new(level));
        if d.level() != level {
            d.reset(level);
        }
        d.finish(input, out);
    }

    /// `deflate (Z_FINISH)`: the last piece, which may be empty, and the end of the stream.
    pub fn finish(&mut self, input: &[u8], out: &mut Vec<u8>) {
        self.run(input, out, DeflateFlush::Finish);
        self.inner.reset();
    }

    /// The bytes written so far of this stream: `pdf_stream_length` follows `c_stream.total_out`.
    pub fn total_out(&self) -> u64 {
        self.inner.total_out()
    }

    fn run(&mut self, input: &[u8], out: &mut Vec<u8>, flush: DeflateFlush) {
        let mut consumed = 0;
        loop {
            let at = out.len();
            out.resize(at + OUT_STEP, 0);
            let (in0, out0) = (self.inner.total_in(), self.inner.total_out());
            let status = self.inner.compress(&input[consumed..], &mut out[at..], flush);
            let produced = (self.inner.total_out() - out0) as usize;
            consumed += (self.inner.total_in() - in0) as usize;
            out.truncate(at + produced);
            let status = match status {
                Ok(s) => s,
                // Only a broken stream state gets here: writezip.c calls pdftex_fail.
                Err(e) => panic!("zlib-rs: deflate failed: {}", e.as_str()),
            };
            match flush {
                DeflateFlush::Finish if status == Status::StreamEnd => return,
                DeflateFlush::Finish => {}
                // All taken, and there was room left, so nothing is waiting to come out.
                _ if consumed == input.len() && produced < OUT_STEP => return,
                _ => {}
            }
        }
    }
}

/// Why inflating stopped.  Whatever came out before that stays in the output.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum InflateError {
    /// The stream needs a preset dictionary.
    NeedDict,
    /// The data is not a valid zlib stream, or its checksum is wrong.
    Data,
    /// The data ended before the stream did.
    Truncated,
}

impl std::fmt::Display for InflateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            InflateError::NeedDict => "the stream needs a preset dictionary",
            InflateError::Data => "invalid compressed data",
            InflateError::Truncated => "compressed data ended early",
        })
    }
}

impl std::error::Error for InflateError {}

/// Inflates one complete zlib stream, appending to `out`.  Returns the number of input bytes the stream
/// took, since a PDF's or PNG's data may be followed by something else.
pub fn inflate(input: &[u8], out: &mut Vec<u8>) -> Result<usize, InflateError> {
    let mut z = Inflater::new();
    let consumed = z.push(input, out)?;
    if z.finished() { Ok(consumed) } else { Err(InflateError::Truncated) }
}

/// A zlib stream read in pieces, as a PNG's `IDAT` chunks are.
pub struct Inflater {
    inner: Inflate,
    finished: bool,
}

impl Default for Inflater {
    fn default() -> Inflater {
        Inflater::new()
    }
}

impl Inflater {
    /// A stream with the zlib wrapper.
    pub fn new() -> Inflater {
        Inflater { inner: Inflate::new(true, WINDOW_BITS), finished: false }
    }

    /// True once the end of the stream was seen, and its checksum was right.
    pub fn finished(&self) -> bool {
        self.finished
    }

    /// Takes a piece, appending what comes out of it to `out`, and returns how much of the piece it
    /// took.  That is all of it unless the stream ended within it.
    pub fn push(&mut self, input: &[u8], out: &mut Vec<u8>) -> Result<usize, InflateError> {
        let mut consumed = 0;
        while !self.finished {
            let at = out.len();
            out.resize(at + OUT_STEP, 0);
            let (in0, out0) = (self.inner.total_in(), self.inner.total_out());
            let status = self.inner.decompress(&input[consumed..], &mut out[at..], InflateFlush::NoFlush);
            let produced = (self.inner.total_out() - out0) as usize;
            let took = (self.inner.total_in() - in0) as usize;
            consumed += took;
            out.truncate(at + produced);
            match status {
                Ok(Status::StreamEnd) => self.finished = true,
                Ok(_) => {
                    // No more input and room left over: this piece is used up.
                    if consumed == input.len() && produced < OUT_STEP {
                        break;
                    }
                }
                Err(zlib_rs::InflateError::NeedDict { .. }) => return Err(InflateError::NeedDict),
                Err(_) => return Err(InflateError::Data),
            }
        }
        Ok(consumed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A reproducible mix of runs and noise.
    pub(crate) fn sample(n: usize) -> Vec<u8> {
        let mut x = 0x1234_5678u32;
        (0..n)
            .map(|i| {
                x = x.wrapping_mul(2654435761).wrapping_add(i as u32);
                if i % 64 < 40 { b"BT /F1 9.9626 Tf 72 720 Td (Hello) Tj ET\n"[i % 41] } else { (x >> 24) as u8 }
            })
            .collect()
    }

    #[test]
    fn deflate_shrinks_and_has_a_zlib_header() {
        let input = vec![b'a'; 10000];
        let mut out = vec![1, 2];
        deflate(&input, 6, &mut out);
        assert_eq!(&out[..2], &[1, 2], "appends");
        assert_eq!(out[2], 0x78, "zlib header");
        assert!(out.len() < 200);
    }

    #[test]
    fn round_trip_at_every_level_and_size() {
        for n in [0, 1, 2, 100, 32767, 32768, 32769, 70000, 1 << 20] {
            let input = sample(n);
            for level in 0..=9 {
                let mut z = Vec::new();
                deflate(&input, level, &mut z);
                let mut back = vec![9];
                let took = inflate(&z, &mut back).unwrap();
                assert_eq!(took, z.len());
                assert_eq!(&back[1..], &input[..], "size {n}, level {level}");
            }
        }
    }

    /// `input` through a `Deflater`, in pieces of `piece` bytes.
    fn in_pieces(input: &[u8], level: i32, piece: usize) -> Vec<u8> {
        let mut d = Deflater::new(level);
        let mut out = Vec::new();
        let mut chunks = input.chunks(piece).peekable();
        while let Some(c) = chunks.next() {
            if chunks.peek().is_some() {
                d.write(c, &mut out);
            } else {
                d.finish(c, &mut out);
            }
        }
        out
    }

    #[test]
    fn pieces_are_reproducible_and_inflate_to_the_data() {
        let input = sample(200_000);
        for level in [1, 6, 9] {
            let mut whole = Vec::new();
            deflate(&input, level, &mut whole);
            // Everything in one finish is what deflate does.
            assert_eq!(in_pieces(&input, level, input.len()), whole, "level {level}");
            for piece in [1usize, 7, 4096, 16384, 65536] {
                if piece == 1 && level != 1 {
                    continue;
                }
                let z = in_pieces(&input, level, piece);
                // The bytes may differ from `whole`, but the same pieces always give the same bytes.
                assert_eq!(z, in_pieces(&input, level, piece), "level {level}, pieces of {piece}");
                let mut back = Vec::new();
                assert_eq!(inflate(&z, &mut back), Ok(z.len()));
                assert_eq!(back, input, "level {level}, pieces of {piece}");
            }
        }
    }

    #[test]
    fn a_finished_deflater_is_ready_for_the_next_stream() {
        let mut d = Deflater::new(6);
        let mut a = Vec::new();
        d.finish(b"first stream", &mut a);
        assert_eq!(d.total_out(), 0, "finish resets, so the next stream counts from zero");
        let mut b = Vec::new();
        d.write(b"second ", &mut b);
        d.finish(b"stream", &mut b);
        let mut out = Vec::new();
        inflate(&a, &mut out).unwrap();
        assert_eq!(out, b"first stream");
        out.clear();
        inflate(&b, &mut out).unwrap();
        assert_eq!(out, b"second stream");
        // Another level, as a change of \pdfcompresslevel in mid document.
        d.reset(1);
        assert_eq!(d.level(), 1);
        let mut c = Vec::new();
        d.finish(b"third", &mut c);
        let mut expected = Vec::new();
        deflate(b"third", 1, &mut expected);
        assert_eq!(c, expected);
        // An empty stream is still a valid one.
        let mut e = Vec::new();
        d.finish(b"", &mut e);
        out.clear();
        assert_eq!(inflate(&e, &mut out), Ok(e.len()));
        assert!(out.is_empty());
    }

    #[test]
    fn total_out_counts_while_writing() {
        let input = sample(500_000);
        let mut d = Deflater::new(1);
        let mut out = vec![0u8; 3];
        d.write(&input, &mut out);
        assert_eq!(d.total_out(), out.len() as u64 - 3);
    }

    #[test]
    fn inflate_reports_what_it_took_and_why_it_stopped() {
        let mut z = Vec::new();
        deflate(b"some data", 6, &mut z);
        let n = z.len();
        // Trailing bytes are not taken.
        z.extend_from_slice(b"endstream");
        let mut out = Vec::new();
        assert_eq!(inflate(&z, &mut out), Ok(n));
        assert_eq!(out, b"some data");
        // Truncated.
        out.clear();
        assert_eq!(inflate(&z[..n - 1], &mut out), Err(InflateError::Truncated));
        out.clear();
        assert_eq!(inflate(&[], &mut out), Err(InflateError::Truncated));
        // A wrong checksum.
        let mut bad = z[..n].to_vec();
        bad[n - 1] ^= 1;
        out.clear();
        assert_eq!(inflate(&bad, &mut out), Err(InflateError::Data));
        // Not zlib at all.
        out.clear();
        assert_eq!(inflate(b"not a zlib stream", &mut out), Err(InflateError::Data));
    }

    #[test]
    fn inflate_in_pieces() {
        let input = sample(100_000);
        let mut z = Vec::new();
        deflate(&input, 6, &mut z);
        for piece in [1, 13, 4096, z.len()] {
            let mut i = Inflater::new();
            let mut out = Vec::new();
            for c in z.chunks(piece) {
                assert!(!i.finished());
                assert_eq!(i.push(c, &mut out), Ok(c.len()));
            }
            assert!(i.finished());
            assert_eq!(out, input, "pieces of {piece}");
        }
    }

    #[test]
    fn levels_as_design_section_9() {
        assert_eq!(level_for(0, Stream::Content), None);
        assert_eq!(level_for(0, Stream::FontFile), None);
        for l in 1..=9 {
            assert_eq!(level_for(l, Stream::Content), Some(6));
            assert_eq!(level_for(l, Stream::FontFile), Some(1));
        }
    }
}
