//! What the last run's PDF can give this one: a prototype of the output cache.
//!
//! Between two runs of a document most streams of the PDF do not change, and the last PDF is on the disk with all of them deflated.  So no stream is stored twice: a small
//! index in the cache directory says, for each stream of that file, the hash of its input and level, where its bytes are, and a sum of those bytes.  A run reads the old file before
//! `b_open_out` truncates it, and a stream whose hash is in the index takes the old bytes instead of being deflated.  The bytes that come out of a stream depend on nothing but its
//! input and level (DESIGN section 9), and a sum over the bytes taken says that they are what the index says.  The build id is in the index, so another build does not use it.
//!
//! A Type 1 font goes further: the key is all that `write_t1` reads, with the font file by its stamp, and the entry is all it leaves, so a hit does not read the font.
//! A font that gave a warning or read what an earlier font left in `T1Carry` is not kept: `CarryUse`.
#![forbid(unsafe_code)]

use crate::platform;
use rstex_fontfile as ff;
use rstex_fontfile::{CarryUse, FontDims, GlyphNames, GlyphSet};
use rstex_image::out::{ObjNo, Op};
use std::path::PathBuf;

const MAGIC: u64 = u64::from_le_bytes(*b"RSTXOC01");
/// As for the map tables and the kpse index: a stat this young could still change without showing.
const RACY_NS: u64 = 2_000_000_000;
/// Files kept in `outcache/`, those of the last 30 days aside.  Each run replaces its own, so its time is that of the last use.
const KEEP_FILES: usize = 1024;
const KEEP_AGE_S: u64 = 30 * 24 * 3600;

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

fn sum64(data: &[u8]) -> u64 {
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
    /// `loc.len` and `loc.sum` are not known yet: a worker has it.
    held: bool,
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
        for _ in 0..r.u64()? {
            cc.push(r.i64()?);
        }
        Some(FontHit { lengths, tag, fontname, dims, gl, builtin, carry: CarryUse { reusable: true, last_arg_in, last_arg_out, fontname_offset, stack_ptr, cc } })
    }
}

/// The last run's index and PDF.
struct Old {
    pdf: Vec<u8>,
    /// Sorted by key.
    streams: Vec<(u128, Loc)>,
    /// Sorted by key: (key, the stream, the blob).
    fonts: Vec<(u128, Loc, Vec<u8>)>,
    /// Sorted by key: (key, what `image_keep` was given, the streams with their keys).
    images: Vec<(u128, u32, bool, Vec<u8>, Vec<(u128, Loc)>)>,
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
}

/// Non-POD state, in `self.host`.  Nothing here survives a run but the index it writes.
#[derive(Default)]
pub struct HVars {
    asked: bool,
    on: bool,
    index_path: Option<PathBuf>,
    pdf_path: Vec<u8>,
    had_index: bool,
    old_index: Vec<u8>,
    old: Option<Old>,
    new: Vec<New>,
    new_fonts: Vec<(u128, usize, Vec<u8>)>,
    /// `new` up to here has offsets that count what the workers made.
    settled: usize,
    /// The entry of the stream now being finished.
    cur: Option<usize>,
    /// A font or an image was found: its stream takes these bytes, with this key, and a font's blob goes on to the next index.
    precompressed: Option<(u128, Loc, Option<Vec<u8>>)>,
    /// The streams of the image that was found.
    image_streams: Vec<(u128, Loc)>,
    /// (key, list of operations, its first stream, how many slots, whether it needs a page group, how many streams).
    new_images: Vec<(u128, Vec<u8>, usize, u32, bool, usize)>,
    /// A font was parsed: when its stream comes, this goes in `new_fonts`.
    font_to_keep: Option<(u128, Vec<u8>)>,
    pub kind: u8,
    pub stats: Stats,
    log: Vec<(u8, u64, u64, bool)>,
}

fn domain_key(input: &[u8], level: i32) -> u128 {
    hash128(input, level as u64)
}

impl HVars {
    fn enabled(&mut self) -> bool {
        if !self.asked {
            self.asked = true;
            self.on = platform::env_var(b"RSTEX_NO_OUTCACHE").is_none();
        }
        self.on
    }

    /// The PDF at `path` is about to be truncated.  If the index of that path is good and describes that file, the file is read now.
    pub fn capture(&mut self, path: &[u8]) {
        if !self.enabled() {
            return;
        }
        let _p = crate::prof_scope!("outcache capture");
        let Some(dir) = rstex_kpse::cache_dir() else { return };
        let mut abs = Vec::new();
        if !platform::is_absolute(path) {
            let Ok(cwd) = std::env::current_dir() else { return };
            abs.extend_from_slice(cwd.as_os_str().as_encoded_bytes());
            abs.push(b'/');
        }
        abs.extend_from_slice(path);
        let name = format!("{:016x}.oc", crate::checkpoint::xxh64(&abs, 0x6f63));
        let index_path = dir.join("outcache").join(name);
        self.pdf_path = path.to_vec();
        if let Ok(b) = std::fs::read(&index_path) {
            self.had_index = true;
            self.old = self.load(&b, &abs, path);
            if self.old.is_some() {
                self.old_index = b;
            }
        }
        self.index_path = Some(index_path);
    }

    fn load(&self, b: &[u8], abs: &[u8], path: &[u8]) -> Option<Old> {
        if b.len() < 16 || crate::checkpoint::xxh64(&b[..b.len() - 8], 7) != u64::from_le_bytes(b[b.len() - 8..].try_into().ok()?) {
            return None;
        }
        let mut r = Reader { b: &b[..b.len() - 8], at: 0 };
        if r.u64()? != MAGIC || r.vec()? != crate::image::FMT_BUILD_ID.as_bytes() || r.u64()? != cpu_fingerprint() || r.vec()? != abs {
            return None;
        }
        // Only the size: what is taken from the file is checked by its sum, so a stat could only save reading a file that has changed, and a run that changes nothing would have
        // to write the index again for the new stat.
        let size = r.u64()?;
        if std::fs::metadata(platform::path_from_bytes(path)).ok()?.len() != size {
            return None;
        }
        let (n, m) = (r.u64()? as usize, r.u64()? as usize);
        let mut streams = Vec::with_capacity(n);
        for _ in 0..n {
            let key = r.u128()?;
            let loc = Loc { off: r.u64()?, len: r.u64()?, sum: r.u64()? };
            streams.push((key, loc));
        }
        let mut fonts = Vec::with_capacity(m);
        for _ in 0..m {
            let key = r.u128()?;
            let loc = Loc { off: r.u64()?, len: r.u64()?, sum: r.u64()? };
            fonts.push((key, loc, r.vec()?.to_vec()));
        }
        let k = r.u64()? as usize;
        let mut images = Vec::with_capacity(k);
        for _ in 0..k {
            let key = r.u128()?;
            let slots = r.u64()? as u32;
            let needs = r.u64()? != 0;
            let ser = r.vec()?.to_vec();
            let c = r.u64()? as usize;
            let mut st = Vec::with_capacity(c);
            for _ in 0..c {
                let sk = r.u128()?;
                st.push((sk, Loc { off: r.u64()?, len: r.u64()?, sum: r.u64()? }));
            }
            images.push((key, slots, needs, ser, st));
        }
        // Read last: an index that does not fit the file costs no read of it.
        let pdf = std::fs::read(platform::path_from_bytes(path)).ok()?;
        Some(Old { pdf, streams, fonts, images })
    }

    fn bytes_of(&self, loc: Loc) -> Option<&[u8]> {
        let old = self.old.as_ref()?;
        let s = old.pdf.get(loc.off as usize..(loc.off + loc.len) as usize)?;
        (sum64(s) == loc.sum).then_some(s)
    }

    /// The stream in `input` is complete.  The bytes from the old PDF, if it holds them.  Either way the stream gets an entry in the next index.
    pub fn stream(&mut self, input: &[u8], level: i32, out: &mut Vec<u8>) -> Option<u64> {
        if !self.on || self.index_path.is_none() {
            return None;
        }
        let _p = crate::prof_scope!("outcache stream");
        self.stats.streams += 1;
        self.stats.bytes_in += input.len() as u64;
        if let Some((key, loc, blob)) = self.precompressed.take() {
            let at = out.len();
            let got = self.bytes_of(loc).map(|s| out.extend_from_slice(s));
            debug_assert!(got.is_some(), "font_lookup checked this");
            self.stats.hits += 1;
            self.cur = Some(self.new.len());
            if let Some(blob) = blob {
                self.new_fonts.push((key, self.new.len(), blob));
            }
            self.new.push(New { key, loc: Loc { off: 0, len: loc.len, sum: loc.sum }, held: false });
            self.note_log(0, loc.len, true);
            return Some((out.len() - at) as u64);
        }
        let key = domain_key(input, level);
        self.cur = Some(self.new.len());
        if let Some((fk, blob)) = self.font_to_keep.take() {
            self.new_fonts.push((fk, self.new.len(), blob));
        }
        let mut hit = None;
        if let Some(old) = &self.old {
            if let Ok(i) = old.streams.binary_search_by(|e| e.0.cmp(&key)) {
                hit = Some(old.streams[i].1);
            }
        }
        if let Some(loc) = hit {
            let at = out.len();
            let good = self.bytes_of(loc).map(|s| out.extend_from_slice(s)).is_some();
            if good {
                self.stats.hits += 1;
                self.stats.hit_in += input.len() as u64;
                self.new.push(New { key, loc: Loc { off: 0, len: loc.len, sum: loc.sum }, held: false });
                self.note_log(input.len() as u64, loc.len, true);
                return Some((out.len() - at) as u64);
            }
            self.stats.sum_failures += 1;
        }
        self.new.push(New { key, loc: Loc::default(), held: false });
        None
    }

    fn note_log(&mut self, input: u64, out: u64, hit: bool) {
        self.stats.bytes_out += out;
        self.log.push((self.kind_now(), input, out, hit));
    }

    fn kind_now(&self) -> u8 {
        self.kind
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
        self.new[i].held = true;
        self.log.push((self.kind, input, 0, false));
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
    pub fn settled(&mut self, made: &[(Option<usize>, u64, u64)], gained: &[(i64, i64)]) {
        for &(e, len, sum) in made {
            if let Some(i) = e {
                self.new[i].loc.len = len;
                self.new[i].loc.sum = sum;
            }
        }
        let held: std::collections::HashSet<usize> = made.iter().filter_map(|m| m.0).collect();
        for i in self.settled..self.new.len() {
            let at = self.new[i].loc.off as i64;
            // A worker's stream begins where its own `at` says, after what those before it gained.
            let k = if held.contains(&i) { gained.partition_point(|g| g.0 < at) } else { gained.partition_point(|g| g.0 <= at) };
            if k > 0 {
                self.new[i].loc.off = (at + gained[k - 1].1) as u64;
            }
            self.new[i].held = false;
        }
        self.settled = self.new.len();
    }

    pub fn font_key(&mut self, parts: &[&[u8]]) -> u128 {
        let mut b = Vec::with_capacity(4096);
        for p in parts {
            b.extend_from_slice(&(p.len() as u64).to_le_bytes());
            b.extend_from_slice(p);
        }
        hash128(&b, 0x7431)
    }

    pub fn font_lookup(&mut self, key: u128) -> Option<FontHit> {
        let _p = crate::prof_scope!("outcache font lookup");
        let old = self.old.as_ref()?;
        let i = old.fonts.binary_search_by(|e| e.0.cmp(&key)).ok()?;
        let (loc, blob) = (old.fonts[i].1, old.fonts[i].2.clone());
        let hit = FontHit::from_bytes(&blob)?;
        self.bytes_of(loc)?;
        self.precompressed = Some((key, loc, Some(blob)));
        Some(hit)
    }

    pub fn new_len(&self) -> usize {
        self.new.len()
    }

    /// The key of a PNG file's whole output, by its bytes.
    pub fn image_key(&mut self, data: &[u8], params: &[i32]) -> Option<u128> {
        if !self.on() {
            return None;
        }
        let _p = crate::prof_scope!("outcache image key");
        let mut p = Vec::with_capacity(params.len() * 4);
        for x in params {
            p.extend_from_slice(&x.to_le_bytes());
        }
        Some(hash128(data, 0x706e67) ^ hash128(&p, 0x706e68).rotate_left(1))
    }

    pub fn image_lookup(&mut self, key: u128) -> Option<ImageHit> {
        let old = self.old.as_ref()?;
        let i = old.images.binary_search_by(|e| e.0.cmp(&key)).ok()?;
        let (slots, needs, ser, streams) = (old.images[i].1, old.images[i].2, old.images[i].3.clone(), old.images[i].4.clone());
        for (_, loc) in &streams {
            self.bytes_of(*loc)?;
        }
        let ops = ops_from_bytes(&ser)?;
        self.image_streams = streams;
        Some(ImageHit { ops, ser, slots, needs_page_group: needs })
    }

    /// `Op::Precompressed`: the next stream to end is this one of the image.
    pub fn take_stream(&mut self, i: u32) {
        let (k, loc) = self.image_streams[i as usize];
        self.precompressed = Some((k, loc, None));
    }

    /// An image's streams have just gone out, from `first` on in `new`.
    pub fn image_keep(&mut self, key: u128, ser: Vec<u8>, first: usize, slots: u32, needs_page_group: bool) {
        let count = self.new.len() - first;
        self.new_images.push((key, ser, first, slots, needs_page_group, count));
    }

    /// The font is going out as the old PDF has it after all.
    pub fn font_refused(&mut self) {
        self.precompressed = None;
    }

    pub fn font_keep(&mut self, key: u128, hit: &FontHit) {
        self.font_to_keep = Some((key, hit.to_bytes()));
    }

    pub fn on(&mut self) -> bool {
        self.enabled() && self.index_path.is_some()
    }

    /// The PDF has been published.  The next run's index, for the file now there.
    pub fn publish(&mut self) {
        let Some(index_path) = self.index_path.clone() else { return };
        let _p = crate::prof_scope!("outcache publish");
        let path = self.pdf_path.clone();
        let Ok(md) = std::fs::metadata(platform::path_from_bytes(&path)) else { return };
        let mut abs = Vec::new();
        if !platform::is_absolute(&path) {
            let Ok(cwd) = std::env::current_dir() else { return };
            abs.extend_from_slice(cwd.as_os_str().as_encoded_bytes());
            abs.push(b'/');
        }
        abs.extend_from_slice(&path);
        let mut streams: Vec<(u128, Loc)> = self.new.iter().map(|n| (n.key, n.loc)).collect();
        let fonts: Vec<(u128, Loc, &[u8])> = {
            let mut f: Vec<_> = self.new_fonts.iter().map(|(k, i, b)| (*k, self.new[*i].loc, b.as_slice())).collect();
            f.sort_by_key(|e| e.0);
            f
        };
        streams.sort_by_key(|e| e.0);
        streams.dedup_by_key(|e| e.0);
        let mut images: Vec<_> = self.new_images.iter().filter(|m| m.5 == ops_stream_count(&m.1)).collect();
        images.sort_by_key(|m| m.0);
        let mut o = Vec::with_capacity(streams.len() * 40 + fonts.iter().map(|f| f.2.len() + 48).sum::<usize>() + 256);
        put_u64(&mut o, MAGIC);
        put_vec(&mut o, crate::image::FMT_BUILD_ID.as_bytes());
        put_u64(&mut o, cpu_fingerprint());
        put_vec(&mut o, &abs);
        put_u64(&mut o, md.len());
        put_u64(&mut o, streams.len() as u64);
        put_u64(&mut o, fonts.len() as u64);
        for (key, loc) in &streams {
            o.extend_from_slice(&key.to_le_bytes());
            put_u64(&mut o, loc.off);
            put_u64(&mut o, loc.len);
            put_u64(&mut o, loc.sum);
        }
        for (key, loc, blob) in &fonts {
            o.extend_from_slice(&key.to_le_bytes());
            put_u64(&mut o, loc.off);
            put_u64(&mut o, loc.len);
            put_u64(&mut o, loc.sum);
            put_vec(&mut o, blob);
        }
        put_u64(&mut o, images.len() as u64);
        for (key, ser, first, slots, needs, count) in images {
            o.extend_from_slice(&key.to_le_bytes());
            put_u64(&mut o, *slots as u64);
            put_u64(&mut o, *needs as u64);
            put_vec(&mut o, ser);
            put_u64(&mut o, *count as u64);
            for n in &self.new[*first..*first + *count] {
                o.extend_from_slice(&n.key.to_le_bytes());
                put_u64(&mut o, n.loc.off);
                put_u64(&mut o, n.loc.len);
                put_u64(&mut o, n.loc.sum);
            }
        }
        let sum = crate::checkpoint::xxh64(&o, 7);
        put_u64(&mut o, sum);
        // A run that changed nothing has nothing to write.
        if self.old_index != o {
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
                "rstex: outcache: streams {} hits {} bytes_in {} hit_in {} bytes_out {} fonts {} font_hits {} sum_failures {} index {} old_pdf {}",
                s.streams,
                s.hits,
                s.bytes_in,
                s.hit_in,
                s.bytes_out,
                s.fonts,
                s.font_hits,
                s.sum_failures,
                index_bytes,
                self.old.as_ref().map_or(0, |o| o.pdf.len())
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
fn prune(dir: &std::path::Path) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let mut files = Vec::new();
    for e in rd.flatten() {
        if let Ok(md) = e.metadata() {
            let age = md.modified().ok().and_then(|t| t.elapsed().ok()).map_or(0, |d| d.as_secs());
            files.push((age, e.path()));
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
