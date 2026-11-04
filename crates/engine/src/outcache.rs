//! What the last run's PDF can give this one: the output cache of DESIGN section 9.
//!
//! Between two runs of a document most streams of the PDF do not change, and the last PDF is on the disk with all of them deflated.  So no stream is stored twice: a small index in the
//! cache directory says, for each stream of that file, the hash of its input and level, where its bytes are, and a sum of those bytes.  A run reads those parts of the old file before
//! `b_open_out` truncates it, and no others: a picture copied as it stands is not in the index, and may be most of the file.  A stream whose hash is in the index takes the old bytes instead
//! of being deflated.  The bytes that come out of a stream depend on nothing but its input and level (DESIGN section 9), and a sum over the bytes taken says that they are what the index
//! says.  The build id and a fingerprint of the CPU are in the index, so another build or another kind of machine does not use it.
//!
//! Some streams can be known without being made.  A Type 1 font: the key is all that `write_t1` reads, with the font file by its stamp, and the entry is all it leaves, so a hit does not read
//! the font; one that gave a warning or read what an earlier font left in `T1Carry` is not kept: `CarryUse`.  A font's `/ToUnicode` CMap: the key is all that `write_tounicode` reads.
//! A PNG: the key is the file's bytes and the parameters, and the entry the list of operations that `write_png` gave, with its streams left out.
//!
//! The first run of a PDF does none of this and leaves a word in the table `seen`: most of what a file compiled once would pay for is the hashing, and it would get nothing for it.  The next
//! run writes the first index, and the one after that gets its hits.  `RSTEX_CHECKPOINT_POLICY=always` writes one at first sight, as it does a checkpoint.
#![forbid(unsafe_code)]

use crate::platform;
use rstex_fontfile as ff;
use rstex_fontfile::{CarryUse, FontDims, GlyphNames, GlyphSet};
use rstex_image::out::{ObjNo, Op};
use std::ops::Range;
use std::path::{Path, PathBuf};

const MAGIC: u64 = u64::from_le_bytes(*b"RSTXOC02");
/// As for the map tables and the kpse index: a stat this young could still change without showing.
const RACY_NS: u64 = 2_000_000_000;
/// Files kept in `outcache/`, those of the last 30 days aside.  Each run replaces its own, so its time is that of the last run that changed anything.
const KEEP_FILES: usize = 1024;
const KEEP_AGE_S: u64 = 30 * 24 * 3600;
/// A temporary this old belongs to a run that died.
const STALE_TMP_S: u64 = 24 * 3600;
/// Words in the table of the PDFs seen once.  A slot taken by another document only costs the first index a run later.
const SEEN_FILE: &str = "seen";
const SEEN_SLOTS: u64 = 1 << 12;
/// Parts of the old file this close are read as one: about what a system call costs in bytes copied, and the dictionary between two streams is a tenth of that.
const READ_GAP: u64 = 1 << 14;

/// The bytes in the index of an entry of each table, at the least.
const STREAM_ENTRY: usize = 40;
const DERIVED_ENTRY: usize = 40;
const IMAGE_ENTRY: usize = 40;

/// Seeds of the hashes that are keys: each kind of thing has its own.
const SEED_FONT: u64 = 0x7431;
const SEED_CMAP: u64 = 0x7475;
const SEED_TABLE: u64 = 0x7476;
const SEED_PNG: u64 = 0x706e67;
const SEED_PARAMS: u64 = 0x706e68;
const SEED_INDEX: u64 = 7;
const SEED_NAME: u64 = 0x6f63;

const P1: u64 = 0x9E3779B185EBCA87;
const P2: u64 = 0xC2B2AE3D27D4EB4F;
const P3: u64 = 0x165667B19E3779F9;
const P4: u64 = 0x85EBCA77C2B2AE63;
const P5: u64 = 0x27D4EB2F165667C5;

/// Four lanes of XXH64's round, and both words of the result out of all four.  Each step is one-to-one in the lane, so one changed stripe stays a difference to the end, and two changed
/// stripes can only cancel by chance, in all four lanes.  Against accident, not against an attacker, who can write in the cache directory anyway.
pub fn hash128(data: &[u8], seed: u64) -> u128 {
    #[inline(always)]
    fn round(acc: u64, w: u64) -> u64 {
        acc.wrapping_add(w.wrapping_mul(P2)).rotate_left(31).wrapping_mul(P1)
    }
    #[inline(always)]
    fn word(b: &[u8]) -> u64 {
        u64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]])
    }
    #[inline(always)]
    fn avalanche(mut h: u64) -> u64 {
        h ^= h >> 33;
        h = h.wrapping_mul(P2);
        h ^= h >> 29;
        h = h.wrapping_mul(P3);
        h ^ (h >> 32)
    }
    let (mut a, mut b, mut c, mut d) = (seed.wrapping_add(P1).wrapping_add(P2), seed.wrapping_add(P2), seed, seed.wrapping_sub(P1));
    let mut stripes = data.chunks_exact(32);
    for s in &mut stripes {
        a = round(a, word(&s[0..8]));
        b = round(b, word(&s[8..16]));
        c = round(c, word(&s[16..24]));
        d = round(d, word(&s[24..32]));
    }
    let rest = stripes.remainder();
    if !rest.is_empty() {
        let mut last = [0u8; 32];
        last[..rest.len()].copy_from_slice(rest);
        a = round(a, word(&last[0..8]));
        b = round(b, word(&last[8..16]));
        c = round(c, word(&last[16..24]));
        d = round(d, word(&last[24..32]));
    }
    let len = data.len() as u64;
    let lo = a.rotate_left(1).wrapping_add(b.rotate_left(7)).wrapping_add(c.rotate_left(12)).wrapping_add(d.rotate_left(18)) ^ len.wrapping_mul(P5);
    let hi = a.rotate_left(46).wrapping_mul(P4) ^ b.rotate_left(29).wrapping_mul(P3) ^ c.rotate_left(17).wrapping_mul(P1) ^ d.rotate_left(5).wrapping_mul(P2) ^ len;
    ((avalanche(hi) as u128) << 64) | avalanche(lo) as u128
}

/// What decides which of zlib-rs's code paths run: nothing shows they give the same bytes, so a cache directory shared by two kinds of machine does not mix them.
fn cpu_fingerprint() -> u64 {
    #[cfg(target_arch = "x86_64")]
    {
        (std::arch::is_x86_feature_detected!("avx2") as u64)
            | (std::arch::is_x86_feature_detected!("avx512f") as u64) << 1
            | (std::arch::is_x86_feature_detected!("pclmulqdq") as u64) << 2
            | (std::arch::is_x86_feature_detected!("sse4.2") as u64) << 3
            | (std::arch::is_x86_feature_detected!("avx512vl") as u64) << 4
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        0
    }
}

/// The sum of a stream's deflated bytes.
pub fn sum64(data: &[u8]) -> u64 {
    crate::checkpoint::xxh64(data, 0x4f43)
}

/// Where a stream's deflated bytes lie.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Loc {
    pub off: u64,
    pub len: u64,
    pub sum: u64,
}

/// A stream of this run's PDF.
#[derive(Clone, Copy, Default)]
struct New {
    key: u128,
    loc: Loc,
}

/// What `write_t1` leaves, in the bytes of the index.
pub struct FontHit {
    pub lengths: (i32, i32, i32),
    pub tag: Option<[u8; 6]>,
    pub fontname: Vec<u8>,
    pub dims: FontDims,
    pub gl: GlyphSet,
    pub builtin: Option<GlyphNames>,
    pub carry: CarryUse,
}

struct Reader<'a> {
    b: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let s = self.b.get(self.at..self.at.checked_add(n)?)?;
        self.at += n;
        Some(s)
    }
    fn u64(&mut self) -> Option<u64> {
        Some(u64::from_le_bytes(self.take(8)?.try_into().ok()?))
    }
    fn u128(&mut self) -> Option<u128> {
        Some(u128::from_le_bytes(self.take(16)?.try_into().ok()?))
    }
    fn i64(&mut self) -> Option<i64> {
        Some(self.u64()? as i64)
    }
    fn vec(&mut self) -> Option<&'a [u8]> {
        let n = self.u64()? as usize;
        self.take(n)
    }
    /// Where in the bytes the next [`Reader::vec`] is.
    fn range(&mut self) -> Option<Range<usize>> {
        let n = self.u64()? as usize;
        let at = self.at;
        self.take(n)?;
        Some(at..at + n)
    }
    /// A count of entries of at least `size` bytes each: no more than the bytes left can hold.
    fn count(&mut self, size: usize) -> Option<usize> {
        let n = self.u64()? as usize;
        (n <= (self.b.len() - self.at) / size).then_some(n)
    }
    fn opt_i64(&mut self) -> Option<Option<i64>> {
        let has = self.u64()?;
        let v = self.i64()?;
        Some((has != 0).then_some(v))
    }
}

fn put_u64(o: &mut Vec<u8>, x: u64) {
    o.extend_from_slice(&x.to_le_bytes());
}

fn put_vec(o: &mut Vec<u8>, s: &[u8]) {
    put_u64(o, s.len() as u64);
    o.extend_from_slice(s);
}

fn put_opt_i64(o: &mut Vec<u8>, x: Option<i64>) {
    put_u64(o, x.is_some() as u64);
    put_u64(o, x.unwrap_or(0) as u64);
}

impl FontHit {
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut o = Vec::with_capacity(4096);
        put_u64(&mut o, self.lengths.0 as u32 as u64);
        put_u64(&mut o, self.lengths.1 as u32 as u64);
        put_u64(&mut o, self.lengths.2 as u32 as u64);
        match self.tag {
            Some(t) => {
                put_u64(&mut o, 1);
                o.extend_from_slice(&t);
            }
            None => {
                put_u64(&mut o, 0);
                o.extend_from_slice(&[0; 6]);
            }
        }
        put_vec(&mut o, &self.fontname);
        for p in &self.dims {
            put_u64(&mut o, p.val as u32 as u64 | (p.set as u64) << 32);
        }
        put_u64(&mut o, self.gl.len() as u64);
        for g in self.gl.iter() {
            put_vec(&mut o, g);
        }
        match &self.builtin {
            Some(n) => {
                put_u64(&mut o, 1);
                for c in 0..256 {
                    if n.is_notdef(c) {
                        put_u64(&mut o, u64::MAX);
                    } else {
                        put_vec(&mut o, n.name(c));
                    }
                }
            }
            None => put_u64(&mut o, 0),
        }
        put_opt_i64(&mut o, self.carry.last_arg_in);
        put_opt_i64(&mut o, self.carry.last_arg_out);
        put_opt_i64(&mut o, self.carry.fontname_offset);
        put_u64(&mut o, self.carry.stack_ptr as u64);
        put_u64(&mut o, self.carry.cc.len() as u64);
        for v in &self.carry.cc {
            put_u64(&mut o, *v as u64);
        }
        o
    }

    pub fn from_bytes(b: &[u8]) -> Option<FontHit> {
        let mut r = Reader { b, at: 0 };
        let lengths = (r.u64()? as u32 as i32, r.u64()? as u32 as i32, r.u64()? as u32 as i32);
        let has_tag = r.u64()? != 0;
        let t = r.take(6)?;
        let tag = has_tag.then(|| [t[0], t[1], t[2], t[3], t[4], t[5]]);
        let fontname = r.vec()?.to_vec();
        let mut dims = ff::new_font_dims();
        for p in &mut dims {
            let x = r.u64()?;
            p.val = x as u32 as i32;
            p.set = (x >> 32) != 0;
        }
        let mut gl = GlyphSet::new();
        for _ in 0..r.u64()? {
            gl.insert(r.vec()?);
        }
        let builtin = if r.u64()? != 0 {
            let mut n = GlyphNames::new();
            for c in 0..256 {
                let save = r.at;
                if r.u64()? != u64::MAX {
                    r.at = save;
                    n.set(c, r.vec()?);
                }
            }
            Some(n)
        } else {
            None
        };
        let last_arg_in = r.opt_i64()?;
        let last_arg_out = r.opt_i64()?;
        let fontname_offset = r.opt_i64()?;
        let stack_ptr = r.u64()? as usize;
        let mut cc = Vec::new();
        for _ in 0..r.count(8)? {
            cc.push(r.i64()?);
        }
        Some(FontHit { lengths, tag, fontname, dims, gl, builtin, carry: CarryUse { reusable: true, last_arg_in, last_arg_out, fontname_offset, stack_ptr, cc } })
    }
}

/// A stream found by what it was made from, not by its bytes: a font or a CMap.
struct OldDerived {
    key: u128,
    stream: u128,
    /// What else it left, in `Old::index`.
    blob: Range<usize>,
}

/// A PNG's output.  Its streams are in `Old::streams`, by [`image_stream_key`].
struct OldImage {
    key: u128,
    slots: u32,
    needs_page_group: bool,
    /// The list of operations, in `Old::index`.
    ops: Range<usize>,
}

/// The last run's index and what it names of the PDF.
struct Old {
    index: Vec<u8>,
    /// The parts of the file that were read, one after the other.
    read: Vec<u8>,
    /// Those parts, sorted: where each was in the file, how long, where it is in `read`.
    parts: Vec<(u64, u64, usize)>,
    /// Sorted by key.
    streams: Vec<(u128, Loc)>,
    /// Sorted by key.
    derived: Vec<OldDerived>,
    /// Sorted by key.
    images: Vec<OldImage>,
    /// Of the file.
    size: u64,
}

impl Old {
    fn stream(&self, key: u128) -> Option<Loc> {
        self.streams.binary_search_by(|e| e.0.cmp(&key)).ok().map(|i| self.streams[i].1)
    }

    /// The bytes of a stream, with no question of whether they are right.
    fn bytes_at(&self, loc: Loc) -> Option<&[u8]> {
        let i = self.parts.partition_point(|p| p.0 <= loc.off).checked_sub(1)?;
        let (off, _, at) = self.parts[i];
        let from = at + (loc.off - off) as usize;
        self.read.get(from..from.checked_add(loc.len as usize)?)
    }

    /// The bytes of a stream, if they are what the index says.  Only here does anything depend on the old file.
    fn bytes_of(&self, loc: Loc) -> Option<&[u8]> {
        self.bytes_at(loc).filter(|s| sum64(s) == loc.sum)
    }
}

/// What a font, a CMap or an image found: the stream that goes out next.
struct Taken {
    key: u128,
    loc: Loc,
    /// A font's or a CMap's entry in the next index: its key and what else it left.
    derived: Option<(u128, Vec<u8>)>,
}

/// What a run has done, for the trace.
#[derive(Default, Clone, Copy)]
pub struct Stats {
    pub streams: u64,
    pub hits: u64,
    pub bytes_in: u64,
    pub hit_in: u64,
    pub bytes_out: u64,
    pub fonts: u64,
    pub font_hits: u64,
    pub sum_failures: u64,
    pub cmaps: u64,
    pub cmap_hits: u64,
}

/// Non-POD state, in `self.host`.  Nothing here survives a run but the index it writes.
#[derive(Default)]
pub struct HVars {
    /// `--no-snapshot` or `-ini`: the driver says so before anything else runs.
    pub literal: bool,
    asked: bool,
    allowed: bool,
    /// This run reads the old file if it can, and leaves an index.
    active: bool,
    index_path: Option<PathBuf>,
    pdf_path: Vec<u8>,
    /// `pdf_path` from the root: what the index is of.
    abs_path: Vec<u8>,
    had_index: bool,
    old: Option<Old>,
    new: Vec<New>,
    /// (key, the stream in `new`, what else it left).
    new_derived: Vec<(u128, usize, Vec<u8>)>,
    /// `new` up to here has offsets that count what the workers made.
    settled: usize,
    /// The entry of the stream now being finished.
    cur: Option<usize>,
    /// A font, a CMap or an image was found: the next stream to end is this.
    taken: Option<Taken>,
    /// The streams of the image that was found.
    image_streams: Vec<(u128, Loc)>,
    /// The image whose streams are going out, and how many have so far.
    image_mode: Option<(u128, u32)>,
    /// (key, list of operations, its first stream, how many slots, whether it needs a page group, how many streams).
    new_images: Vec<(u128, Vec<u8>, usize, u32, bool, usize)>,
    /// A font or a CMap was made: when its stream comes, this goes in `new_derived`.
    derived_to_keep: Option<(u128, Vec<u8>)>,
    /// `\pdfglyphtounicode`'s table since the format by its hash, for as far as its log had got.
    table_identity: Option<(usize, u128)>,
    /// What kind of stream this is, for the log of `RSTEX_OUTCACHE_LOG`.
    pub kind: u8,
    pub stats: Stats,
    logging: bool,
    log: Vec<(u8, u64, u64, bool)>,
}

fn domain_key(input: &[u8], level: i32) -> u128 {
    hash128(input, level as u64)
}

/// The key in the index of an image's stream, counting from 1.  An image's streams are known by the file they came from: hashing what was unpacked from it would cost as much as hashing a big
/// file again, for nothing.
fn image_stream_key(image: u128, n: u32) -> u128 {
    image ^ (n as u128).wrapping_mul(0x9E37_79B9_7F4A_7C15_F39C_C060_5CED_C835)
}

/// What a format keeps of its table of `\pdfglyphtounicode`.
pub fn table_hash(sealed: &[u8]) -> u128 {
    hash128(sealed, SEED_TABLE + 1)
}

/// `path` from the root.
fn absolute(path: &[u8]) -> Option<Vec<u8>> {
    if platform::is_absolute(path) {
        return Some(path.to_vec());
    }
    let mut abs = platform::path_to_bytes(&std::env::current_dir().ok()?);
    abs.push(b'/');
    abs.extend_from_slice(path);
    Some(abs)
}

/// True if a run has been here before.  If not, the next one will find that it has.  A file that cannot be written only means no index.
fn seen_before(dir: &Path, hash: u64) -> bool {
    if platform::env_var(b"RSTEX_CHECKPOINT_POLICY").as_deref() == Some(b"always") {
        return true;
    }
    let path = dir.join(SEEN_FILE);
    let (at, word) = ((hash % SEEN_SLOTS) * 8, hash | 1);
    let mut b = [0; 8];
    if rstex_kpse::open_private(&path).is_some_and(|f| platform::read_exact_at(&f, at, &mut b)) && u64::from_le_bytes(b) == word {
        return true;
    }
    let _ = std::fs::create_dir_all(dir);
    if let Some(mut f) = rstex_kpse::open_private_to_update(&path) {
        use std::io::{Seek, Write};
        let mut write = || -> std::io::Result<()> {
            // Whole from the start, and with holes where the system has them, so that any slot can be read.
            if f.metadata()?.len() != SEEN_SLOTS * 8 {
                f.set_len(SEEN_SLOTS * 8)?;
            }
            f.seek(std::io::SeekFrom::Start(at))?;
            f.write_all(&word.to_le_bytes())
        };
        let _ = write();
    }
    false
}

/// The parts of the file at `path` that hold these streams, as few and as small as they can be: (the bytes, where each part was).
fn read_parts(path: &[u8], streams: &[(u128, Loc)]) -> Option<(Vec<u8>, Vec<(u64, u64, usize)>)> {
    let mut locs: Vec<(u64, u64)> = streams.iter().map(|(_, l)| (l.off, l.len)).collect();
    locs.sort_unstable();
    let mut parts: Vec<(u64, u64, usize)> = Vec::new();
    for (off, len) in locs {
        match parts.last_mut() {
            Some((o, l, _)) if off <= *o + *l + READ_GAP => *l = (*l).max(off + len - *o),
            _ => parts.push((off, len, 0)),
        }
    }
    let mut total = 0;
    for p in &mut parts {
        p.2 = total;
        total += p.1 as usize;
    }
    let mut read = vec![0; total];
    if total > 0 {
        let f = std::fs::File::open(platform::path_from_bytes(path)).ok()?;
        for &(off, len, at) in &parts {
            if !platform::read_exact_at(&f, off, &mut read[at..at + len as usize]) {
                return None;
            }
        }
    }
    Some((read, parts))
}

/// The index in `b`, if it is one for this build and this machine, and describes the file now at `path`.
fn load(b: Vec<u8>, abs: &[u8], path: &[u8]) -> Option<Old> {
    if b.len() < 16 || crate::checkpoint::xxh64(&b[..b.len() - 8], SEED_INDEX) != u64::from_le_bytes(b[b.len() - 8..].try_into().ok()?) {
        return None;
    }
    let mut r = Reader { b: &b[..b.len() - 8], at: 0 };
    if r.u64()? != MAGIC || r.vec()? != crate::image::FMT_BUILD_ID.as_bytes() || r.u64()? != cpu_fingerprint() || r.vec()? != abs {
        return None;
    }
    // Only the size: what is taken from the file is checked by its sum, so a stat could only save reading a file that has changed, and a run that changes nothing would have
    // to write the index again for the new stat.
    let size = r.u64()?;
    let md = std::fs::metadata(platform::path_from_bytes(path)).ok()?;
    if !md.is_file() || md.len() != size {
        return None;
    }
    let n = r.count(STREAM_ENTRY)?;
    let mut streams = Vec::with_capacity(n);
    for _ in 0..n {
        let key = r.u128()?;
        let loc = Loc { off: r.u64()?, len: r.u64()?, sum: r.u64()? };
        if loc.off.checked_add(loc.len)? > size {
            return None;
        }
        streams.push((key, loc));
    }
    let m = r.count(DERIVED_ENTRY)?;
    let mut derived = Vec::with_capacity(m);
    for _ in 0..m {
        derived.push(OldDerived { key: r.u128()?, stream: r.u128()?, blob: r.range()? });
    }
    let k = r.count(IMAGE_ENTRY)?;
    let mut images = Vec::with_capacity(k);
    for _ in 0..k {
        images.push(OldImage { key: r.u128()?, slots: r.u64()? as u32, needs_page_group: r.u64()? != 0, ops: r.range()? });
    }
    // Last: an index that does not fit the file costs no read of it.
    let (read, parts) = read_parts(path, &streams)?;
    Some(Old { index: b, read, parts, streams, derived, images, size })
}

impl HVars {
    fn allowed(&mut self) -> bool {
        if !self.asked {
            self.asked = true;
            // What judges the literal engine says RSTEX_NO_SNAPSHOT.
            self.allowed = !self.literal && platform::env_var(b"RSTEX_NO_OUTCACHE").is_none() && platform::env_var(b"RSTEX_NO_SNAPSHOT").is_none();
            self.logging = platform::env_var(b"RSTEX_OUTCACHE_LOG").is_some();
        }
        self.allowed
    }

    /// The PDF at `path` is about to be truncated.  If the index of that path is good and describes that file, what it names of the file is read now.
    pub fn capture(&mut self, path: &[u8]) {
        // A file that could not be made may be followed by another name.
        self.active = false;
        self.old = None;
        if !self.allowed() {
            return;
        }
        let _p = crate::prof_scope!("outcache capture");
        let Some(dir) = rstex_kpse::cache_dir() else { return };
        let Some(abs) = absolute(path) else { return };
        let hash = crate::checkpoint::xxh64(&abs, SEED_NAME);
        let dir = dir.join("outcache");
        let index_path = dir.join(format!("{hash:016x}.oc"));
        let mut index = Vec::new();
        self.had_index = rstex_kpse::open_private(&index_path).is_some_and(|mut f| std::io::Read::read_to_end(&mut f, &mut index).is_ok());
        if self.had_index {
            self.old = load(index, &abs, path);
        } else if !seen_before(&dir, hash) {
            return;
        }
        self.active = true;
        self.index_path = Some(index_path);
        self.pdf_path = path.to_vec();
        self.abs_path = abs;
    }

    /// The stream in `input` is complete.  The bytes from the old PDF, if it holds them.  Either way the stream gets an entry in the next index.
    pub fn stream(&mut self, input: &[u8], level: i32, out: &mut Vec<u8>) -> Option<u64> {
        if !self.active {
            return None;
        }
        let _p = crate::prof_scope!("outcache stream");
        self.stats.streams += 1;
        self.stats.bytes_in += input.len() as u64;
        if let Some(t) = self.taken.take() {
            // What found it has checked the sum.
            let s = self.old.as_ref().and_then(|o| o.bytes_at(t.loc)).expect("what found it has seen these");
            out.extend_from_slice(s);
            self.stats.hits += 1;
            self.cur = Some(self.new.len());
            if let Some((key, blob)) = t.derived {
                self.new_derived.push((key, self.new.len(), blob));
            }
            self.new.push(New { key: t.key, loc: Loc { off: 0, len: t.loc.len, sum: t.loc.sum } });
            self.note_log(0, t.loc.len, true);
            return Some(t.loc.len);
        }
        let key = match &mut self.image_mode {
            Some((base, n)) => {
                *n += 1;
                image_stream_key(*base, *n)
            }
            None => domain_key(input, level),
        };
        self.cur = Some(self.new.len());
        if let Some((k, blob)) = self.derived_to_keep.take() {
            self.new_derived.push((k, self.new.len(), blob));
        }
        if let (Some(old), None) = (&self.old, &self.image_mode) {
            if let Some(loc) = old.stream(key) {
                if let Some(s) = old.bytes_of(loc) {
                    out.extend_from_slice(s);
                    self.stats.hits += 1;
                    self.stats.hit_in += input.len() as u64;
                    self.new.push(New { key, loc: Loc { off: 0, len: loc.len, sum: loc.sum } });
                    self.note_log(input.len() as u64, loc.len, true);
                    return Some(loc.len);
                }
                self.stats.sum_failures += 1;
            }
        }
        self.new.push(New { key, loc: Loc::default() });
        None
    }

    fn note_log(&mut self, input: u64, out: u64, hit: bool) {
        self.stats.bytes_out += out;
        if self.logging {
            self.log.push((self.kind, input, out, hit));
        }
    }

    /// A stream that was deflated here: its bytes end `bytes`.
    pub fn deflated(&mut self, bytes: &[u8], input: u64) {
        if let Some(i) = self.cur {
            self.new[i].loc.len = bytes.len() as u64;
            self.new[i].loc.sum = sum64(bytes);
            self.note_log(input, bytes.len() as u64, false);
        }
    }

    /// A stream that went to a worker.
    pub fn to_worker(&mut self, input: u64) -> Option<usize> {
        let i = self.cur.take()?;
        if self.logging {
            self.log.push((self.kind, input, 0, false));
        }
        Some(i)
    }

    /// The offset of the bytes of the stream that was just written, as the file stands without what the workers have.
    pub fn written_at(&mut self, file_offset_after: i64) {
        if let Some(i) = self.cur.take() {
            self.new[i].loc.off = (file_offset_after - self.new[i].loc.len as i64) as u64;
        }
    }

    pub fn worker_at(&mut self, entry: Option<usize>, at: i64) {
        if let Some(i) = entry {
            self.new[i].loc.off = at as u64;
        }
    }

    /// The workers' results are in the file.  `gained` is what `pdfout_settle` computes, and `made` the bytes by entry.
    pub fn settled(&mut self, made: &[(usize, u64, u64)], gained: &[(i64, i64)]) {
        if !self.active {
            return;
        }
        for &(i, len, sum) in made {
            self.new[i].loc.len = len;
            self.new[i].loc.sum = sum;
        }
        let mut held = made.iter().map(|m| m.0).peekable();
        for i in self.settled..self.new.len() {
            let at = self.new[i].loc.off as i64;
            // A worker's stream begins where its own `at` says, after what those before it gained.
            let was_held = held.next_if_eq(&i).is_some();
            let k = if was_held { gained.partition_point(|g| g.0 < at) } else { gained.partition_point(|g| g.0 <= at) };
            if k > 0 {
                self.new[i].loc.off = (at + gained[k - 1].1) as u64;
            }
        }
        self.settled = self.new.len();
    }

    /// The stream of the entry with this key, if the bytes are right: what a font or a CMap is about to stand for.
    fn take_derived(&mut self, key: u128) -> Option<&[u8]> {
        let old = self.old.as_ref()?;
        let i = old.derived.binary_search_by(|e| e.key.cmp(&key)).ok()?;
        let d = &old.derived[i];
        let loc = old.stream(d.stream)?;
        if old.bytes_of(loc).is_none() {
            self.stats.sum_failures += 1;
            return None;
        }
        let blob = old.index.get(d.blob.clone())?;
        self.taken = Some(Taken { key: d.stream, loc, derived: Some((key, blob.to_vec())) });
        Some(blob)
    }

    pub fn font_key(&mut self, parts: &[&[u8]]) -> u128 {
        let mut b = Vec::with_capacity(4096);
        for p in parts {
            b.extend_from_slice(&(p.len() as u64).to_le_bytes());
            b.extend_from_slice(p);
        }
        hash128(&b, SEED_FONT)
    }

    pub fn font_lookup(&mut self, key: u128) -> Option<FontHit> {
        let _p = crate::prof_scope!("outcache font lookup");
        let hit = FontHit::from_bytes(self.take_derived(key)?);
        if hit.is_none() {
            self.taken = None;
        }
        hit
    }

    /// The font is going out as the old PDF has it after all.
    pub fn font_refused(&mut self) {
        self.taken = None;
    }

    pub fn font_keep(&mut self, key: u128, hit: &FontHit) {
        self.derived_to_keep = Some((key, hit.to_bytes()));
    }

    /// The key of a CMap, from the rest of what `write_tounicode` reads and `\pdfglyphtounicode`'s table: the entries since the format, a log that only grows, and the hash of the format's.
    pub fn cmap_key(&mut self, rest: &[u8], log: &[u8], sealed: u128) -> u128 {
        let identity = match self.table_identity {
            Some((len, h)) if len == log.len() => h,
            _ => {
                let h = hash128(log, SEED_TABLE);
                self.table_identity = Some((log.len(), h));
                h
            }
        };
        let mut b = Vec::with_capacity(rest.len() + 32);
        b.extend_from_slice(&identity.to_le_bytes());
        b.extend_from_slice(&sealed.to_le_bytes());
        b.extend_from_slice(rest);
        hash128(&b, SEED_CMAP)
    }

    /// True if the old PDF has this CMap.
    pub fn cmap_lookup(&mut self, key: u128) -> bool {
        self.stats.cmaps += 1;
        let found = self.take_derived(key).is_some();
        self.stats.cmap_hits += found as u64;
        found
    }

    pub fn cmap_keep(&mut self, key: u128) {
        self.derived_to_keep = Some((key, Vec::new()));
    }

    /// The stream of a font or a CMap has gone out: what was waiting for it has been used, or has no stream to go with.
    pub fn derived_written(&mut self) {
        self.derived_to_keep = None;
        self.taken = None;
    }

    pub fn new_len(&self) -> usize {
        self.new.len()
    }

    /// The streams from here on are those of the image with this key.
    pub fn image_begin(&mut self, key: u128) {
        self.image_mode = Some((key, 0));
    }

    /// The key of a PNG file's whole output, by its bytes.
    pub fn image_key(&mut self, data: &[u8], params: &[i32]) -> Option<u128> {
        if !self.active {
            return None;
        }
        let _p = crate::prof_scope!("outcache image key");
        let mut p = Vec::with_capacity(params.len() * 4);
        for x in params {
            p.extend_from_slice(&x.to_le_bytes());
        }
        Some(hash128(data, SEED_PNG) ^ hash128(&p, SEED_PARAMS).rotate_left(1))
    }

    pub fn image_lookup(&mut self, key: u128) -> Option<ImageHit> {
        let old = self.old.as_ref()?;
        let i = old.images.binary_search_by(|e| e.key.cmp(&key)).ok()?;
        let e = &old.images[i];
        let ser = old.index.get(e.ops.clone())?;
        let ops = ops_from_bytes(ser)?;
        let count = ops.iter().filter(|o| matches!(o, Op::BeginStream)).count() as u32;
        let mut streams = Vec::with_capacity(count as usize);
        for n in 1..=count {
            let k = image_stream_key(key, n);
            let loc = old.stream(k)?;
            if old.bytes_of(loc).is_none() {
                self.stats.sum_failures += 1;
                return None;
            }
            streams.push((k, loc));
        }
        let hit = ImageHit { ops, ser: ser.to_vec(), slots: e.slots, needs_page_group: e.needs_page_group };
        self.image_streams = streams;
        Some(hit)
    }

    /// `Op::Precompressed`: the next stream to end is this one of the image.
    pub fn take_stream(&mut self, i: u32) {
        let (key, loc) = self.image_streams[i as usize];
        self.taken = Some(Taken { key, loc, derived: None });
    }

    /// An image's streams have just gone out, from `first` on in `new`.
    pub fn image_keep(&mut self, key: u128, ser: Vec<u8>, first: usize, slots: u32, needs_page_group: bool) {
        self.image_mode = None;
        let count = self.new.len() - first;
        self.new_images.push((key, ser, first, slots, needs_page_group, count));
    }

    pub fn on(&self) -> bool {
        self.active
    }

    /// The PDF has been published.  The next run's index, for the file now there.
    pub fn publish(&mut self) {
        if !self.active {
            return;
        }
        let Some(index_path) = self.index_path.clone() else { return };
        let _p = crate::prof_scope!("outcache publish");
        let Ok(md) = std::fs::metadata(platform::path_from_bytes(&self.pdf_path)) else { return };
        if !md.is_file() {
            return;
        }
        let mut streams: Vec<(u128, Loc)> = self.new.iter().map(|n| (n.key, n.loc)).collect();
        streams.sort_unstable_by_key(|e| e.0);
        streams.dedup_by_key(|e| e.0);
        let mut derived: Vec<(u128, u128, &[u8])> = self.new_derived.iter().map(|(k, i, b)| (*k, self.new[*i].key, b.as_slice())).collect();
        derived.sort_unstable_by_key(|e| e.0);
        derived.dedup_by_key(|e| e.0);
        let mut images: Vec<_> = self.new_images.iter().filter(|m| m.5 == ops_stream_count(&m.1)).collect();
        images.sort_unstable_by_key(|m| m.0);
        images.dedup_by_key(|m| m.0);
        let mut o = Vec::with_capacity(streams.len() * STREAM_ENTRY + derived.iter().map(|f| f.2.len() + DERIVED_ENTRY).sum::<usize>() + images.iter().map(|m| m.1.len() + IMAGE_ENTRY).sum::<usize>() + 256);
        put_u64(&mut o, MAGIC);
        put_vec(&mut o, crate::image::FMT_BUILD_ID.as_bytes());
        put_u64(&mut o, cpu_fingerprint());
        put_vec(&mut o, &self.abs_path);
        put_u64(&mut o, md.len());
        put_u64(&mut o, streams.len() as u64);
        for (key, loc) in &streams {
            o.extend_from_slice(&key.to_le_bytes());
            put_u64(&mut o, loc.off);
            put_u64(&mut o, loc.len);
            put_u64(&mut o, loc.sum);
        }
        put_u64(&mut o, derived.len() as u64);
        for (key, stream, blob) in &derived {
            o.extend_from_slice(&key.to_le_bytes());
            o.extend_from_slice(&stream.to_le_bytes());
            put_vec(&mut o, blob);
        }
        put_u64(&mut o, images.len() as u64);
        for (key, ser, _, slots, needs, _) in images {
            o.extend_from_slice(&key.to_le_bytes());
            put_u64(&mut o, *slots as u64);
            put_u64(&mut o, *needs as u64);
            put_vec(&mut o, ser);
        }
        let sum = crate::checkpoint::xxh64(&o, SEED_INDEX);
        put_u64(&mut o, sum);
        // A run that changed nothing has nothing to write.
        if self.old.as_ref().is_none_or(|old| old.index != o) {
            rstex_kpse::publish_path(&index_path, &o);
        }
        // Only a run that adds a file can make the directory larger, and listing it is 1 in 64 of those.
        if !self.had_index && sum & 63 == 0 {
            prune(index_path.parent().expect("a file in outcache/"));
        }
        self.trace(o.len());
    }

    fn trace(&self, index_bytes: usize) {
        let s = self.stats;
        if platform::env_var(b"RSTEX_OUTCACHE_TRACE").is_some() {
            eprintln!(
                "rstex: outcache: streams {} hits {} bytes_in {} hit_in {} bytes_out {} fonts {} font_hits {} sum_failures {} cmaps {} cmap_hits {} index {} old_read {} old_size {}",
                s.streams,
                s.hits,
                s.bytes_in,
                s.hit_in,
                s.bytes_out,
                s.fonts,
                s.font_hits,
                s.sum_failures,
                s.cmaps,
                s.cmap_hits,
                index_bytes,
                self.old.as_ref().map_or(0, |o| o.read.len()),
                self.old.as_ref().map_or(0, |o| o.size),
            );
        }
        if let Some(p) = platform::env_var(b"RSTEX_OUTCACHE_LOG") {
            let mut t = String::new();
            for (k, i, o, h) in &self.log {
                t.push_str(&format!("{}\t{}\t{}\t{}\n", *k as char, i, o, *h as u8));
            }
            let _ = std::fs::write(platform::path_from_bytes(&p), t);
        }
    }
}

/// A PNG's output, found in the index.
pub struct ImageHit {
    pub ops: Vec<Op>,
    pub ser: Vec<u8>,
    pub slots: u32,
    pub needs_page_group: bool,
}

/// Slots in a list that came from the index are relative to the first.
pub fn rebase_ops(ops: &mut [Op], base: u32) {
    for op in ops {
        match op {
            Op::NewObj(s) | Op::Num(s) => *s += base,
            Op::BeginDict { obj: ObjNo::Slot(s), .. } | Op::BeginObj { obj: ObjNo::Slot(s), .. } => *s += base,
            _ => {}
        }
    }
}

/// A list of operations as the index keeps it, with what is in a stream left out and its slots relative to `base`.  `None` for what a run may not simply repeat: a warning, a part of the
/// input copied outside a stream that is not deflated, an object number passed in, or a PDF's fonts.
pub fn ops_to_bytes(ops: &[Op], base: u32) -> Option<Vec<u8>> {
    let mut o = Vec::with_capacity(256);
    let mut in_stream = false;
    let mut streams = 0u64;
    let mut any = false;
    for op in ops {
        match op {
            Op::BeginStream => {
                in_stream = true;
                any = true;
                put_u64(&mut o, 1);
                put_u64(&mut o, 7);
                put_u64(&mut o, streams);
                streams += 1;
            }
            Op::EndStream => {
                if !in_stream {
                    return None;
                }
                in_stream = false;
                put_u64(&mut o, 2);
            }
            Op::Puts(_) if in_stream => {}
            Op::Puts(b) => {
                put_u64(&mut o, 3);
                put_vec(&mut o, b);
            }
            Op::NewObj(s) => {
                put_u64(&mut o, 4);
                put_u64(&mut o, (*s - base) as u64);
            }
            Op::Num(s) => {
                put_u64(&mut o, 5);
                put_u64(&mut o, (*s - base) as u64);
            }
            Op::BeginDict { obj: ObjNo::Slot(s), level } => {
                put_u64(&mut o, 6);
                put_u64(&mut o, (*s - base) as u64);
                put_u64(&mut o, *level as u32 as u64);
            }
            Op::Flush => put_u64(&mut o, 8),
            Op::TexPrint(b) => {
                put_u64(&mut o, 9);
                put_vec(&mut o, b);
            }
            _ => return None,
        }
    }
    (any && !in_stream).then_some(o)
}

fn ops_stream_count(b: &[u8]) -> usize {
    ops_from_bytes(b).map_or(usize::MAX, |ops| ops.iter().filter(|o| matches!(o, Op::BeginStream)).count())
}

fn ops_from_bytes(b: &[u8]) -> Option<Vec<Op>> {
    let mut r = Reader { b, at: 0 };
    let mut ops = Vec::new();
    while r.at < b.len() {
        match r.u64()? {
            1 => ops.push(Op::BeginStream),
            7 => ops.push(Op::Precompressed(r.u64()? as u32)),
            2 => ops.push(Op::EndStream),
            3 => ops.push(Op::Puts(r.vec()?.to_vec())),
            4 => ops.push(Op::NewObj(r.u64()? as u32)),
            5 => ops.push(Op::Num(r.u64()? as u32)),
            6 => {
                let s = r.u64()? as u32;
                let level = r.u64()? as u32 as i32;
                ops.push(Op::BeginDict { obj: ObjNo::Slot(s), level });
            }
            8 => ops.push(Op::Flush),
            9 => ops.push(Op::TexPrint(r.vec()?.to_vec())),
            _ => return None,
        }
    }
    Some(ops)
}

/// At most `KEEP_FILES`, unless younger than `KEEP_AGE_S`.  Only a run that adds a file comes here.
fn prune(dir: &Path) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let mut files = Vec::new();
    for e in rd.flatten() {
        let Ok(md) = e.metadata() else { continue };
        let age = md.modified().ok().and_then(|t| t.elapsed().ok()).map_or(0, |d| d.as_secs());
        let path = e.path();
        match path.extension() {
            Some(x) if x == "oc" => files.push((age, path)),
            Some(x) if x.as_encoded_bytes().starts_with(b"tmp") && age > STALE_TMP_S => {
                let _ = std::fs::remove_file(path);
            }
            _ => {}
        }
    }
    files.sort();
    for (age, path) in files.into_iter().skip(KEEP_FILES) {
        if age > KEEP_AGE_S {
            let _ = std::fs::remove_file(path);
        }
    }
}

/// A font file with a stat that shows any later change, or none to rely on.
pub fn stamp_settled(stamp: &rstex_kpse::Stamp) -> bool {
    platform::STAT_SEES_CONTENT && stamp[0] != 0 && stamp[2].max(stamp[3]) + RACY_NS <= rstex_kpse::now_ns()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash128_sees_any_one_change() {
        let mut x: u64 = 88172645463325252;
        let data: Vec<u8> = (0..5000)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                x as u8
            })
            .collect();
        let mut seen = std::collections::HashSet::new();
        for n in [0, 1, 7, 8, 31, 32, 33, 63, 64, 65, 4999] {
            assert!(seen.insert(hash128(&data[..n], 6)), "prefix {n}");
        }
        let whole = hash128(&data, 6);
        assert_ne!(whole, hash128(&data, 1), "another level");
        for at in (0..data.len()).step_by(61) {
            let mut d = data.clone();
            d[at] ^= 1;
            assert_ne!(hash128(&d, 6), whole, "{at}");
        }
        // Trailing zeros are not padding.
        let mut d = data[..40].to_vec();
        let a = hash128(&d, 6);
        d.push(0);
        assert_ne!(a, hash128(&d, 6));
    }

    #[test]
    fn a_font_comes_back_as_it_went() {
        let mut gl = GlyphSet::new();
        gl.insert(b"a");
        gl.insert(b".notdef");
        let mut names = GlyphNames::new();
        names.set(65, b"A");
        let mut dims = ff::new_font_dims();
        dims[2].val = -7;
        dims[2].set = true;
        let f = FontHit {
            lengths: (1, 2, 3),
            tag: Some(*b"ABCDEF"),
            fontname: b"CMR10".to_vec(),
            dims,
            gl,
            builtin: Some(names),
            carry: CarryUse { reusable: true, last_arg_in: Some(3), last_arg_out: None, fontname_offset: Some(99), stack_ptr: 0, cc: vec![1, -2] },
        };
        let g = FontHit::from_bytes(&f.to_bytes()).unwrap();
        assert_eq!(g.lengths, f.lengths);
        assert_eq!(g.tag, f.tag);
        assert_eq!(g.fontname, f.fontname);
        assert_eq!(g.dims, f.dims);
        assert_eq!(g.gl.iter().collect::<Vec<_>>(), f.gl.iter().collect::<Vec<_>>());
        let n = g.builtin.as_ref().unwrap();
        assert_eq!(n.name(65), b"A");
        assert!(n.is_notdef(66));
        assert_eq!(g.carry, f.carry);
        assert!(FontHit::from_bytes(&f.to_bytes()[..50]).is_none());
    }

    #[test]
    fn a_list_of_operations_keeps_all_but_the_bodies() {
        let ops = vec![
            Op::Puts(b"/Type".to_vec()),
            Op::NewObj(12),
            Op::Num(12),
            Op::BeginStream,
            Op::Puts(vec![1, 2, 3]),
            Op::EndStream,
            Op::Flush,
            Op::BeginDict { obj: ObjNo::Slot(12), level: 0 },
            Op::BeginStream,
            Op::Puts(vec![4]),
            Op::EndStream,
            Op::TexPrint(b"x".to_vec()),
        ];
        let b = ops_to_bytes(&ops, 10).unwrap();
        assert_eq!(ops_stream_count(&b), 2);
        let mut back = ops_from_bytes(&b).unwrap();
        rebase_ops(&mut back, 100);
        assert_eq!(
            back,
            vec![
                Op::Puts(b"/Type".to_vec()),
                Op::NewObj(102),
                Op::Num(102),
                Op::BeginStream,
                Op::Precompressed(0),
                Op::EndStream,
                Op::Flush,
                Op::BeginDict { obj: ObjNo::Slot(102), level: 0 },
                Op::BeginStream,
                Op::Precompressed(1),
                Op::EndStream,
                Op::TexPrint(b"x".to_vec()),
            ]
        );
        assert!(ops_to_bytes(&[Op::Warn(b"w".to_vec())], 0).is_none(), "a warning is not kept");
        assert!(ops_to_bytes(&[Op::BeginDict { obj: ObjNo::Known(5), level: 0 }], 0).is_none());
        assert!(ops_to_bytes(&[Op::Puts(b"no stream".to_vec())], 0).is_none(), "nothing to save");
    }

    fn loc(off: u64, len: u64, file: &[u8]) -> Loc {
        Loc { off, len, sum: sum64(&file[off as usize..(off + len) as usize]) }
    }

    #[test]
    fn only_the_parts_the_index_names_are_read() {
        let dir = std::env::temp_dir().join(format!("rstex-oc-parts-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("x.pdf");
        let file: Vec<u8> = (0..3 * READ_GAP as usize + 5000).map(|i| (i * 7 + i / 251) as u8).collect();
        std::fs::write(&f, &file).unwrap();
        let path = platform::path_to_bytes(&f);
        // Two close together, a gap too wide to read across, and one inside another's part.
        let far = 2 * READ_GAP + 4000;
        let streams = [(1, loc(10, 100, &file)), (2, loc(150, 50, &file)), (3, loc(far, 200, &file)), (4, loc(far + 20, 30, &file))];
        let (read, parts) = read_parts(&path, &streams).unwrap();
        assert_eq!(parts, vec![(10, 190, 0), (far, 200, 190)]);
        assert_eq!(read.len(), 390);
        let old = Old { index: Vec::new(), read, parts, streams: streams.to_vec(), derived: Vec::new(), images: Vec::new(), size: file.len() as u64 };
        for (_, l) in &streams {
            assert_eq!(old.bytes_of(*l).unwrap(), &file[l.off as usize..(l.off + l.len) as usize]);
        }
        assert!(old.bytes_of(Loc { sum: streams[0].1.sum ^ 1, ..streams[0].1 }).is_none(), "a sum that does not fit");
        assert!(old.bytes_at(Loc { off: 5, len: 1, sum: 0 }).is_none(), "before the first part");
        assert!(old.bytes_at(Loc { off: far + 150, len: 100_000, sum: 0 }).is_none(), "past the last");
        // The file has gone short since the size was looked at.
        std::fs::write(&f, &file[..300]).unwrap();
        assert!(read_parts(&path, &streams).is_none());
        // Nothing named, nothing opened.
        std::fs::remove_file(&f).unwrap();
        assert_eq!(read_parts(&path, &[]).unwrap().0.len(), 0);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn an_index_comes_back_for_its_file_and_no_other() {
        let dir = std::env::temp_dir().join(format!("rstex-oc-index-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("x.pdf");
        let file: Vec<u8> = (0..4000).map(|i| (i * 3) as u8).collect();
        std::fs::write(&f, &file).unwrap();
        let path = platform::path_to_bytes(&f);
        let index_path = dir.join("i.oc");
        let mut v = HVars { active: true, index_path: Some(index_path.clone()), pdf_path: path.clone(), abs_path: path.clone(), ..HVars::default() };
        for (key, off, len) in [(5u128, 100, 300), (9, 1000, 20)] {
            v.new.push(New { key, loc: loc(off, len, &file) });
        }
        v.new_derived.push((77, 1, b"blob".to_vec()));
        let ser = ops_to_bytes(&[Op::Puts(b"/Type".to_vec()), Op::BeginStream, Op::EndStream], 0).unwrap();
        v.new_images.push((88, ser.clone(), 0, 3, true, 1));
        v.publish();
        let bytes = std::fs::read(&index_path).unwrap();
        let old = load(bytes.clone(), &path, &path).unwrap();
        assert_eq!(old.streams.iter().map(|s| s.0).collect::<Vec<_>>(), vec![5, 9]);
        assert_eq!(old.size, 4000);
        assert_eq!(old.read.len(), 920, "from the first stream's start to the second's end, the gap being small");
        assert_eq!((old.derived[0].key, old.derived[0].stream), (77, 9));
        assert_eq!(&old.index[old.derived[0].blob.clone()], b"blob");
        assert_eq!((old.images[0].key, old.images[0].slots, old.images[0].needs_page_group), (88, 3, true));
        assert_eq!(&old.index[old.images[0].ops.clone()], &ser[..]);
        assert!(load(bytes.clone(), b"/elsewhere.pdf", &path).is_none(), "the index of another path");
        let mut flipped = bytes.clone();
        flipped[40] ^= 1;
        assert!(load(flipped, &path, &path).is_none(), "its sum");
        assert!(load(bytes[..bytes.len() - 1].to_vec(), &path, &path).is_none(), "cut short");
        // A count that the bytes cannot hold is no reason to reserve memory for it.
        let mut r = Reader { b: &[255; 16], at: 0 };
        assert!(r.count(STREAM_ENTRY).is_none());
        std::fs::write(&f, &file[..3999]).unwrap();
        assert!(load(bytes.clone(), &path, &path).is_none(), "the size of the file");
        std::fs::remove_file(&f).unwrap();
        std::fs::create_dir(&f).unwrap();
        assert!(load(bytes, &path, &path).is_none(), "what is not a file is not read");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_first_sight_of_a_pdf_leaves_a_word() {
        let dir = std::env::temp_dir().join(format!("rstex-oc-seen-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // RSTEX_CHECKPOINT_POLICY is not set by what runs the tests.
        assert!(!seen_before(&dir, 0x1234_5000));
        assert_eq!(std::fs::metadata(dir.join(SEEN_FILE)).unwrap().len(), SEEN_SLOTS * 8);
        assert!(seen_before(&dir, 0x1234_5000));
        assert!(!seen_before(&dir, 0x1234_5000 + SEEN_SLOTS), "the same slot, and another PDF");
        assert!(!seen_before(&dir, 0x1234_5000), "it took the slot");
        assert!(!seen_before(&dir, 7));
        assert!(seen_before(&dir, 7));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn pruning_keeps_the_newest_and_the_young() {
        let dir = std::env::temp_dir().join(format!("rstex-oc-prune-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let old = std::time::SystemTime::now() - std::time::Duration::from_secs(KEEP_AGE_S + 3600);
        for i in 0..KEEP_FILES + 40 {
            let p = dir.join(format!("{i}.oc"));
            let f = std::fs::File::create(&p).unwrap();
            // The first 40 are the oldest of all.
            let t = old - std::time::Duration::from_secs((KEEP_FILES + 40 - i) as u64);
            f.set_modified(t).unwrap();
        }
        let young = dir.join("young.oc");
        std::fs::write(&young, b"x").unwrap();
        // What is not an index stays, unless it is a temporary of a run that died.
        let table = dir.join(SEEN_FILE);
        std::fs::File::create(&table).unwrap().set_modified(old).unwrap();
        let dead = dir.join("1.tmp99");
        std::fs::File::create(&dead).unwrap().set_modified(old).unwrap();
        let alive = dir.join("2.tmp98");
        std::fs::write(&alive, b"x").unwrap();
        prune(&dir);
        let left = std::fs::read_dir(&dir).unwrap().count();
        assert_eq!(left, KEEP_FILES + 2, "{left}");
        assert!(young.exists());
        assert!(table.exists() && alive.exists() && !dead.exists());
        assert!(!dir.join("0.oc").exists());
        assert!(dir.join(format!("{}.oc", KEEP_FILES + 39)).exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
