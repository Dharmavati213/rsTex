//! The PDF as a list of chunks, with deferred fields and no seek-back (DESIGN.md section 6).
//!
//! Bytes are appended.  A field whose value is not yet known, such as `/Length`, is reserved and filled
//! later, in memory.  Nothing here touches a file: the owner of the output writes the chunks out when
//! the document is done, with [`ChunkWriter::write_to`] or [`ChunkWriter::chunks`].  A stream being compressed
//! elsewhere has its place kept by [`ChunkWriter::reserve_chunk`] until [`ChunkWriter::fill_reserved`]; until then its
//! bytes count for nothing in the offsets, and a [`ChunkWriter::patch`] says offsets of that kind.

/// Size of a chunk, unless a single write is larger.
const CHUNK: usize = 1 << 20;

/// A reserved field, to be filled once with [`ChunkWriter::fill`].  A plain value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Deferred {
    offset: u64,
    len: u32,
}

impl Deferred {
    /// Where the field starts in the file.
    pub fn offset(&self) -> u64 {
        self.offset
    }

    /// The bytes reserved.
    pub fn len(&self) -> usize {
        self.len as usize
    }

    /// True for a field of no bytes.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}

/// See the module documentation.  `pdf_offset` of WEB is [`ChunkWriter::offset`].
#[derive(Default)]
pub struct ChunkWriter {
    chunks: Vec<Vec<u8>>,
    /// The offset of each chunk's first byte.
    starts: Vec<u64>,
    /// All bytes so far.
    total: u64,
}

impl ChunkWriter {
    /// An empty output.
    pub fn new() -> ChunkWriter {
        ChunkWriter::default()
    }

    /// The offset the next byte will have.
    #[inline]
    pub fn offset(&self) -> i64 {
        self.total as i64
    }

    /// The last chunk, if `need` more bytes fit without reallocating, or a new one.
    #[inline]
    fn room(&mut self, need: usize) -> &mut Vec<u8> {
        let fits = self.chunks.last().is_some_and(|c| c.capacity() - c.len() >= need);
        if !fits {
            self.starts.push(self.total);
            self.chunks.push(Vec::with_capacity(CHUNK.max(need)));
        }
        self.chunks.last_mut().expect("just made sure")
    }

    /// Appends bytes.
    #[inline]
    pub fn write(&mut self, s: &[u8]) {
        self.room(s.len()).extend_from_slice(s);
        self.total += s.len() as u64;
    }

    /// Appends one chunk that is ready, without copying it.  An empty one is ignored.
    pub fn push_chunk(&mut self, chunk: Vec<u8>) {
        if chunk.is_empty() {
            return;
        }
        self.starts.push(self.total);
        self.total += chunk.len() as u64;
        self.chunks.push(chunk);
        // The next write must not go into it, since its capacity is not ours to plan with: starting a
        // new chunk then is what `room` does when it does not fit, and an extend within capacity is fine.
    }

    /// Keeps a place for a chunk that is not ready, which holds no bytes until [`ChunkWriter::fill_reserved`].
    /// Returns what that wants to know about the place.
    pub fn reserve_chunk(&mut self) -> usize {
        self.starts.push(self.total);
        self.chunks.push(Vec::new());
        self.chunks.len() - 1
    }

    /// Puts ready chunks in their places.  What comes after a place moves up by what the chunk holds.
    pub fn fill_reserved(&mut self, filled: impl IntoIterator<Item = (usize, Vec<u8>)>) {
        for (place, chunk) in filled {
            debug_assert!(self.chunks[place].is_empty());
            self.chunks[place] = chunk;
        }
        let mut at = 0;
        for (start, chunk) in self.starts.iter_mut().zip(&self.chunks) {
            *start = at;
            at += chunk.len() as u64;
        }
        self.total = at;
    }

    /// Reserves `len` bytes, which stay spaces until [`ChunkWriter::fill`].
    pub fn defer(&mut self, len: usize) -> Deferred {
        let offset = self.total;
        let c = self.room(len);
        let at = c.len();
        c.resize(at + len, b' ');
        self.total += len as u64;
        Deferred { offset, len: u32::try_from(len).expect("a deferred field is short") }
    }

    /// Fills a deferred field.  What `s` does not cover stays a space, as with `/Length` in pdfTeX.
    /// A value that does not fit is a logic error in the caller, and panics.
    pub fn fill(&mut self, d: Deferred, s: &[u8]) {
        assert!(s.len() <= d.len(), "deferred field too small: {} bytes for {}", s.len(), d.len());
        self.patch(d.offset, s);
    }

    /// Overwrites bytes already written, by offset: what `writestreamlength` does with a seek.  Writing
    /// outside what exists is a logic error in the caller, and panics.
    pub fn patch(&mut self, offset: u64, s: &[u8]) {
        assert!(offset + s.len() as u64 <= self.total, "patch at {offset} of {} bytes past the end {}", s.len(), self.total);
        if s.is_empty() {
            return;
        }
        // Patches are usually in the last chunk or nearly.
        let mut i = self.starts.partition_point(|&st| st <= offset) - 1;
        let mut at = (offset - self.starts[i]) as usize;
        let mut rest = s;
        while !rest.is_empty() {
            let c = &mut self.chunks[i];
            let take = rest.len().min(c.len() - at);
            c[at..at + take].copy_from_slice(&rest[..take]);
            rest = &rest[take..];
            i += 1;
            at = 0;
        }
    }

    /// The chunks in order.
    pub fn chunks(&self) -> impl Iterator<Item = &[u8]> {
        self.chunks.iter().map(|c| &c[..])
    }

    /// Writes everything out.
    pub fn write_to<W: std::io::Write>(&self, w: &mut W) -> std::io::Result<()> {
        for c in &self.chunks {
            w.write_all(c)?;
        }
        Ok(())
    }

    /// The whole output in one piece.
    pub fn contents(&self) -> Vec<u8> {
        self.chunks.concat()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defers_without_seeking() {
        let mut p = ChunkWriter::new();
        p.write(b"abc ");
        assert_eq!(p.offset(), 4);
        let d = p.defer(6);
        assert_eq!((d.offset(), d.len(), d.is_empty()), (4, 6, false));
        p.write(b" def");
        assert_eq!(p.offset(), 14);
        assert_eq!(p.contents(), b"abc        def");
        p.fill(d, b"123");
        assert_eq!(p.contents(), b"abc 123    def");
        let mut v = Vec::new();
        p.write_to(&mut v).unwrap();
        assert_eq!(v, b"abc 123    def");
    }

    #[test]
    #[should_panic(expected = "deferred field too small")]
    fn a_value_too_long_is_refused() {
        let mut p = ChunkWriter::new();
        let d = p.defer(2);
        p.fill(d, b"123");
    }

    #[test]
    #[should_panic(expected = "past the end")]
    fn a_patch_past_the_end_is_refused() {
        let mut p = ChunkWriter::new();
        p.write(b"abc");
        p.patch(2, b"xy");
    }

    #[test]
    fn chunks_begin_when_the_last_is_full() {
        let mut p = ChunkWriter::new();
        p.write(&vec![1u8; CHUNK - 1]);
        assert_eq!(p.chunks().count(), 1);
        p.write(&[2]);
        assert_eq!(p.chunks().count(), 1);
        p.write(&[3]);
        assert_eq!(p.chunks().count(), 2);
        // A write larger than a chunk gets one to itself.
        p.write(&vec![4u8; 3 * CHUNK]);
        assert_eq!(p.chunks().count(), 3);
        assert_eq!(p.offset() as usize, 4 * CHUNK + 1);
        assert_eq!(p.chunks().map(<[u8]>::len).sum::<usize>(), 4 * CHUNK + 1);
    }

    #[test]
    fn a_patch_may_cross_chunks() {
        let mut p = ChunkWriter::new();
        p.write(&vec![b'a'; CHUNK]);
        p.write(&vec![b'b'; 10]);
        assert_eq!(p.chunks().count(), 2);
        p.patch(CHUNK as u64 - 2, b"WXYZ");
        let c = p.contents();
        assert_eq!(&c[CHUNK - 3..CHUNK + 3], b"aWXYZb");
    }

    #[test]
    fn a_handed_over_chunk_keeps_its_place() {
        let mut p = ChunkWriter::new();
        p.write(b"head ");
        let d = p.defer(3);
        p.push_chunk(Vec::new());
        p.push_chunk(b"middle".to_vec());
        p.write(b" tail");
        p.fill(d, b"xyz");
        assert_eq!(p.offset(), 19);
        assert_eq!(p.contents(), b"head xyzmiddle tail");
        p.patch(8, b"M");
        assert_eq!(p.contents(), b"head xyzMiddle tail");
    }

    #[test]
    fn a_reserved_place_holds_its_position() {
        let mut p = ChunkWriter::new();
        p.write(b"head ");
        let d = p.defer(3);
        let a = p.reserve_chunk();
        p.write(b" middle ");
        let e = p.defer(2);
        let b = p.reserve_chunk();
        let c = p.reserve_chunk(); // nothing between, and nothing after it yet
        assert_eq!(p.offset(), 18);
        // Offsets do not count what is not there yet.
        p.fill(d, b"xyz");
        p.fill(e, b"pq");
        p.patch(9, b"M");
        p.fill_reserved([(a, b"AAAA".to_vec()), (b, Vec::new()), (c, b"CC".to_vec())]);
        p.write(b" tail");
        assert_eq!(p.contents(), b"head xyzAAAA Middle pqCC tail");
        assert_eq!(p.offset(), 29);
        // And the next places work as the first did.
        let f = p.reserve_chunk();
        p.write(b"!");
        p.patch(0, b"H");
        p.fill_reserved([(f, b"FF".to_vec())]);
        assert_eq!(p.contents(), b"Head xyzAAAA Middle pqCC tailFF!");
        assert_eq!(p.offset(), 32);
    }

    /// Anything done to a `ChunkWriter` gives what the same done to one flat buffer gives.
    #[test]
    fn behaves_as_one_flat_buffer() {
        let mut x = 88172645463325252u64;
        let mut next = move |m: u64| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x % m
        };
        let mut p = ChunkWriter::new();
        let mut flat: Vec<u8> = Vec::new();
        let mut deferred: Vec<Deferred> = Vec::new();
        for step in 0..4000u64 {
            let byte = (step % 251) as u8;
            match next(5) {
                0 | 1 => {
                    // Big enough now and then to cross chunks.
                    let n = if next(50) == 0 { next(1 << 19) } else { next(300) } as usize;
                    let data = vec![byte; n];
                    p.write(&data);
                    flat.extend_from_slice(&data);
                }
                2 => {
                    let n = next(12) as usize;
                    let d = p.defer(n);
                    assert_eq!(d.offset() as usize, flat.len());
                    flat.resize(flat.len() + n, b' ');
                    deferred.push(d);
                }
                3 => {
                    if let Some(d) = deferred.pop() {
                        let v = vec![byte; next(d.len() as u64 + 1) as usize];
                        p.fill(d, &v);
                        flat[d.offset() as usize..d.offset() as usize + v.len()].copy_from_slice(&v);
                    }
                }
                _ => {
                    if !flat.is_empty() {
                        let at = next(flat.len() as u64) as usize;
                        let n = (next(4000) as usize).min(flat.len() - at);
                        let v = vec![byte; n];
                        p.patch(at as u64, &v);
                        flat[at..at + n].copy_from_slice(&v);
                    }
                }
            }
            assert_eq!(p.offset() as usize, flat.len());
        }
        assert_eq!(p.contents(), flat);
    }
}
