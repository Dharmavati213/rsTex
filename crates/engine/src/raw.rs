//! Unchecked storage: the array wrappers, the memory word, and the image's allocation.
//!
//! With `platform/`, the only engine code allowed `unsafe`; all others are `#![forbid(unsafe_code)]`.  The SSE2 loop that
//! decrypts Type 1 fonts is here too, for `rstex_fontfile`, which forbids it: [`eexec_decrypt`].
//! The formal unsoundness of safe accessors over unchecked indexing is an accepted, documented
//! trade (DESIGN.md section 4): `--features checked` turns every index into a bounds check, and
//! CI runs the corpus that way.  OS calls live in `platform/`, not here.
//!
//! # Word layout, and the overlap audit
//!
//! A [`Word`] is 8 bytes:
//!
//! ```text
//! offset:     0        2        4        6
//! rh/int/sc:  [-------i32-------]
//! lh:                           [-------i32-------]
//! b1, b0:                       [--b1---][--b0---]
//! qqqq:       [--b0--][--b1--]  [--b2--][--b3--]    (u16 each)
//! gr:         [------------------f64------------------]
//! ```
//!
//! The oracle's layout (`texmfmem.h`, little-endian "big TeX"), 16 bytes, where `halfword` is as wide as `integer`, has `lh` and `gr` at 0, `b1` and
//! `b0` at 0 and 2, and `rh`, `int`, `sc` and `qqqq` at 8.  So `rh` and `sc` are one place there as here, which `do_dest` shows when it reads
//! the width of a destination that never had one (`pdfship/dest_stale_words`).  rstex has 8 bytes.  It keeps `b0` and `b1` on `lh` as the oracle has
//! them, `int` and `sc` away from them, and `int` and `sc` on `gr`'s low half, where they change it least.  `qqqq` is wider here because
//! quarterwords are `u16`.
//!
//! Audit (every `.hh`, `.qqqq`, `.int`, `.sc`, `.gr` and every field macro in the merged WEB):
//!
//! - `.qqqq` is used only on `font_info` (char info, lig/kern, exten words), the delimiter fields
//!   §859 (`small_fam`..`large_char`) with `mem[..].qqqq:=null_delimiter` §1359, the four-byte
//!   pseudo-file lines §1754-1756, and `tmp_w` §710.  Each of those words is always read through
//!   the view it was written through; no site reads a `qqqq` word through `hh`, `int` or `gr`.
//! - `.gr` is used only by `glue_set` §153 and the debug print §132.  `glue_stretch` (`.sc`, §177)
//!   shares the same word of an unset node, but `fin_align` assigns `glue_set` before any read.
//! - `int` over `rh`: `new_hlist` §901 is `mem[nucleus(q)].int` while `math_type(nucleus(q))` is
//!   `.hh.rh` of the same word.  The mlist code never relies on one showing through the other, and
//!   the overlap is the same as the oracle's anyway.
//! - `b0`, `b1` over `lh`: noads use `info(x)` (sub box pointer) or `fam`, `character` in the same
//!   word, selected by `math_type`; it is the same overlap as the oracle's.
//! - `nest[p].aux_field` is read as `.sc` (`prev_depth`), `.hh.lh` (`space_factor`), `.hh.rh`
//!   (`clang`) or `.int` (`incompleat_noad`), selected consistently by `mode`.
//! - `.hh` whole-field copies (§862, §918, §1341, §1343, §1359, §1364) copy the entire 8-byte word
//!   here; the source word always holds an `hh`, so this is the same.
//! - No site stores a negative value into a quarterword field; a `grep` over every field macro with
//!   a `:=-` right-hand side finds none, so reading `b0`, `b1` as unsigned is safe.  (The oracle's
//!   `b0`, `b1` are C `short`.)
//!
//! - One site does read what another view left: §834 and §840 return from `hpack(cal_expand_ratio)` without setting the
//!   glue fields, and if `font_expand_ratio` then rounds to 0 the box stays as it is, in the paragraph, and §1713 shows it in
//!   any case.  So `glue_sign(r)`, `glue_order(r)` and `glue_set(r)` are whatever the word held, as they are on the oracle.
//!   `lh` left there by a node that went before reads as `b0`=0, normal, and so does the oracle's, and that is why `b0` is
//!   the high half of `lh`.  What can differ: an old `sc` shows in `glue_set` here and not there, an old `null` is 0
//!   here and negative there, and an old pointer has another value.  PORTING section 15, "Known divergences".
//!
//! - `do_dest` §1637 reads what is left too, under a matrix: `set_rect_dimens` takes `pdf_width`, `pdf_height` and `pdf_depth` of a
//!   destination, and only `fitr` has set them.  PORTING section 15, "Known divergences".
//!
//! Conclusion: apart from those two, no WEB or change-file code depends on an overlap this layout changes.

use core::ops::{Index, IndexMut};

/// Marker for plain data allowed in the POD image.
///
/// Implementors must be `Copy`, contain no pointers, and be valid when all their bytes are zero,
/// since the image starts as a zeroed mapping.  Use [`crate::pod_struct!`]; it checks each field.
pub trait Pod: Copy + 'static {}
impl Pod for i32 {}
impl Pod for i64 {}
impl Pod for u8 {}
impl Pod for u16 {}
impl Pod for u32 {}
impl Pod for u64 {}
impl Pod for f64 {}
impl Pod for bool {}
impl<T: Pod, const N: usize> Pod for [T; N] {}
impl<T: Pod, const N: usize> Pod for A<T, N> {}
impl<T: Pod, const LO: i32, const N: usize> Pod for AO<T, LO, N> {}
impl Pod for Word {}

/// Declares a `#[repr(C)]` struct whose fields are all [`Pod`], and implements `Pod` for it.
///
/// A module's own state is declared this way in the module's file (see docs/PORTING.md, "Adding a
/// global"):
///
/// ```text
/// crate::pod_struct! {
///     /// State owned by this module.
///     pub struct Vars {
///         pub my_counter: i32,
///         pub my_table: A<i32, 256>,
///     }
/// }
/// ```
#[macro_export]
macro_rules! pod_struct {
    ($(#[$m:meta])* pub struct $name:ident { $($(#[$fm:meta])* pub $f:ident : $t:ty),* $(,)? }) => {
        $(#[$m])*
        #[repr(C)]
        #[derive(Clone, Copy)]
        pub struct $name { $($(#[$fm])* pub $f: $t),* }
        impl $crate::raw::Pod for $name {}
        const _: () = {
            fn _check<T: $crate::raw::Pod>() {}
            fn _all() { $(_check::<$t>();)* }
        };
    };
}

/// Integer storage cells for `at` and `put`: compact storage is an accessor concern only.
pub trait Cell32: Copy {
    fn to_i32(self) -> i32;
    fn from_i32(v: i32) -> Self;
}
impl Cell32 for u8 {
    #[inline(always)]
    fn to_i32(self) -> i32 {
        self as i32
    }
    #[inline(always)]
    fn from_i32(v: i32) -> Self {
        v as u8
    }
}
impl Cell32 for u16 {
    #[inline(always)]
    fn to_i32(self) -> i32 {
        self as i32
    }
    #[inline(always)]
    fn from_i32(v: i32) -> Self {
        v as u16
    }
}
impl Cell32 for i32 {
    #[inline(always)]
    fn to_i32(self) -> i32 {
        self
    }
    #[inline(always)]
    fn from_i32(v: i32) -> Self {
        v
    }
}

/// Fixed-capacity array indexed by `i32`, zero-based.
///
/// `self.eqtb[p]` needs no cast.  An index is converted by `i as u32 as usize`, which is a zero
/// extension (no `movslq`).  Unchecked in release, bounds-checked under `--features checked`.
/// A Pascal `array[1..n]` becomes `A<T, {n+1}>` and index 0 is unused, so WEB's subscripts stay
/// literal.
#[repr(transparent)]
#[derive(Clone, Copy)]
pub struct A<T, const N: usize>(pub [T; N]);

impl<T, const N: usize> Index<i32> for A<T, N> {
    type Output = T;
    #[inline(always)]
    fn index(&self, i: i32) -> &T {
        #[cfg(feature = "checked")]
        {
            &self.0[i as u32 as usize]
        }
        #[cfg(not(feature = "checked"))]
        // SAFETY: documented trade, DESIGN.md section 4; CI runs the corpus with `checked`.
        unsafe {
            self.0.get_unchecked(i as u32 as usize)
        }
    }
}

impl<T, const N: usize> IndexMut<i32> for A<T, N> {
    #[inline(always)]
    fn index_mut(&mut self, i: i32) -> &mut T {
        #[cfg(feature = "checked")]
        {
            &mut self.0[i as u32 as usize]
        }
        #[cfg(not(feature = "checked"))]
        // SAFETY: as for `index`.
        unsafe {
            self.0.get_unchecked_mut(i as u32 as usize)
        }
    }
}

impl<T: Cell32, const N: usize> A<T, N> {
    /// Reads a compact cell as `i32`: `self.buffer.at(k)`.
    #[inline(always)]
    pub fn at(&self, i: i32) -> i32 {
        self[i].to_i32()
    }
    /// Writes a compact cell from `i32`: `self.buffer.put(k, c)`.
    #[inline(always)]
    pub fn put(&mut self, i: i32, v: i32) {
        self[i] = T::from_i32(v);
    }
}

impl<T, const N: usize> A<T, N> {
    pub const CAP: usize = N;
    #[inline(always)]
    pub fn as_slice(&self) -> &[T] {
        &self.0
    }
    #[inline(always)]
    pub fn as_mut_slice(&mut self) -> &mut [T] {
        &mut self.0
    }
    /// The elements `a..b`.
    #[inline]
    pub fn range(&self, a: i32, b: i32) -> &[T] {
        &self.0[a as u32 as usize..b as u32 as usize]
    }
    /// The elements `a..b`, mutably.
    #[inline]
    pub fn range_mut(&mut self, a: i32, b: i32) -> &mut [T] {
        &mut self.0[a as u32 as usize..b as u32 as usize]
    }
}

impl<T: Copy, const N: usize> A<T, N> {
    /// `self[dst..dst+n] := self[src..src+n]`; the ranges may overlap.
    #[inline]
    pub fn copy_within(&mut self, src: i32, dst: i32, n: i32) {
        let s = src as u32 as usize;
        self.0.copy_within(s..s + n as u32 as usize, dst as u32 as usize);
    }
}

/// `dst[..n] := src[..n]` for token lists, which are mostly a few cells long: a call to `memmove` costs more than the copy.
/// The two ranges do not overlap unless `n` is above 16.
///
/// # Safety
/// Both ranges are inside their arrays.
#[cfg(not(feature = "checked"))]
#[inline(always)]
unsafe fn copy_cells(src: *const i32, dst: *mut i32, n: usize) {
    use core::ptr::{read_unaligned, write_unaligned};
    type C2 = [i32; 2];
    type C4 = [i32; 4];
    // SAFETY: every offset below is under `n`, as far as the width of what is moved.
    unsafe {
        if n <= 4 {
            if n >= 2 {
                let (a, b) = (read_unaligned(src as *const C2), read_unaligned(src.add(n - 2) as *const C2));
                write_unaligned(dst as *mut C2, a);
                write_unaligned(dst.add(n - 2) as *mut C2, b);
            } else if n == 1 {
                *dst = *src;
            }
        } else if n <= 8 {
            let (a, b) = (read_unaligned(src as *const C4), read_unaligned(src.add(n - 4) as *const C4));
            write_unaligned(dst as *mut C4, a);
            write_unaligned(dst.add(n - 4) as *mut C4, b);
        } else if n <= 16 {
            let (a, b) = (read_unaligned(src as *const C4), read_unaligned(src.add(4) as *const C4));
            let (c, d) = (read_unaligned(src.add(n - 8) as *const C4), read_unaligned(src.add(n - 4) as *const C4));
            write_unaligned(dst as *mut C4, a);
            write_unaligned(dst.add(4) as *mut C4, b);
            write_unaligned(dst.add(n - 8) as *mut C4, c);
            write_unaligned(dst.add(n - 4) as *mut C4, d);
        } else {
            core::ptr::copy(src, dst, n);
        }
    }
}

impl<const N: usize> A<i32, N> {
    /// `self[dst..dst+n] := from[src..src+n]`, for the token heap and the builder.
    #[inline(always)]
    pub fn copy_cells_from<const M: usize>(&mut self, dst: i32, from: &A<i32, M>, src: i32, n: i32) {
        #[cfg(feature = "checked")]
        {
            let (d, s, k) = (dst as u32 as usize, src as u32 as usize, n as u32 as usize);
            self.0[d..d + k].copy_from_slice(&from.0[s..s + k]);
        }
        #[cfg(not(feature = "checked"))]
        // SAFETY: as for `A`'s indexing: the documented trade of DESIGN.md section 4.
        unsafe {
            copy_cells(from.0.as_ptr().add(src as u32 as usize), self.0.as_mut_ptr().add(dst as u32 as usize), n as u32 as usize);
        }
    }

    /// `self[dst..dst+n] := self[src..src+n]`, for a list and a new block: the ranges do not overlap.
    #[inline(always)]
    pub fn copy_cells_within(&mut self, src: i32, dst: i32, n: i32) {
        #[cfg(feature = "checked")]
        {
            let s = src as u32 as usize;
            assert!(src + n <= dst || dst + n <= src, "overlapping ranges");
            self.0.copy_within(s..s + n as u32 as usize, dst as u32 as usize);
        }
        #[cfg(not(feature = "checked"))]
        // SAFETY: as for `copy_cells_from`.
        unsafe {
            let p = self.0.as_mut_ptr();
            copy_cells(p.add(src as u32 as usize) as *const i32, p.add(dst as u32 as usize), n as u32 as usize);
        }
    }
}

/// Fixed-capacity array with a non-zero, possibly negative, lower bound `LO`.
///
/// `AO<i32, {-35111}, 70223>` is `array[-35111..35111] of integer`.  Used where the wasted low
/// part would be large, or where the bound is negative.
#[repr(transparent)]
#[derive(Clone, Copy)]
pub struct AO<T, const LO: i32, const N: usize>(pub [T; N]);

impl<T, const LO: i32, const N: usize> Index<i32> for AO<T, LO, N> {
    type Output = T;
    #[inline(always)]
    fn index(&self, i: i32) -> &T {
        let k = i.wrapping_sub(LO) as u32 as usize;
        #[cfg(feature = "checked")]
        {
            &self.0[k]
        }
        #[cfg(not(feature = "checked"))]
        // SAFETY: as for `A`.
        unsafe {
            self.0.get_unchecked(k)
        }
    }
}

impl<T, const LO: i32, const N: usize> IndexMut<i32> for AO<T, LO, N> {
    #[inline(always)]
    fn index_mut(&mut self, i: i32) -> &mut T {
        let k = i.wrapping_sub(LO) as u32 as usize;
        #[cfg(feature = "checked")]
        {
            &mut self.0[k]
        }
        #[cfg(not(feature = "checked"))]
        // SAFETY: as for `A`.
        unsafe {
            self.0.get_unchecked_mut(k)
        }
    }
}

impl<T: Cell32, const LO: i32, const N: usize> AO<T, LO, N> {
    #[inline(always)]
    pub fn at(&self, i: i32) -> i32 {
        self[i].to_i32()
    }
    #[inline(always)]
    pub fn put(&mut self, i: i32, v: i32) {
        self[i] = T::from_i32(v);
    }
}

/// `four_quarters` as a value in a variable (`cur_i`, `main_j`, `font_check[f]`): plain `i32`
/// fields, so ported code reads `i.b0` with no casts.  Inside a [`Word`] it is stored as 4 x u16.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FourQuarters {
    pub b0: i32,
    pub b1: i32,
    pub b2: i32,
    pub b3: i32,
}
impl Pod for FourQuarters {}

#[repr(C)]
#[derive(Clone, Copy)]
struct Hh {
    rh: i32,
    lh: i32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct Bb {
    _rh: i32,
    b1: u16,
    b0: u16,
}

/// `memory_word`, 8 bytes: see the module documentation for the layout and the overlap audit.
///
/// ```text
/// self.mem[p].rh()        // mem[p].hh.rh
/// self.mem[p].set_lh(x)   // mem[p].hh.lh:=x
/// self.mem[p].b0()        // mem[p].hh.b0
/// self.mem[p].int()       // mem[p].int (also .sc())
/// self.mem[p].gr()        // mem[p].gr, f64
/// self.mem[p].qqqq()      // mem[p].qqqq, a FourQuarters
/// self.mem[p].q0()        // mem[p].qqqq.b0
/// ```
#[repr(C)]
#[derive(Clone, Copy)]
pub union Word {
    hh: Hh,
    bb: Bb,
    q: [u16; 4],
    gr: f64,
    raw: u64,
}

impl Word {
    pub const ZERO: Word = Word { raw: 0 };

    /// A word from its two halves: `.hh.rh = rh`, `.hh.lh = lh`.  A whole-word store.
    #[inline(always)]
    pub const fn hh(rh: i32, lh: i32) -> Word {
        Word { hh: Hh { rh, lh } }
    }
    /// A word with `.hh.rh = rh`, `.hh.b0 = b0`, `.hh.b1 = b1`.  A whole-word store.
    #[inline(always)]
    pub const fn hb(rh: i32, b0: i32, b1: i32) -> Word {
        Word { bb: Bb { _rh: rh, b0: b0 as u16, b1: b1 as u16 } }
    }

    #[inline(always)]
    pub fn raw(&self) -> u64 {
        // SAFETY: all views of a Word are plain data and every bit pattern is valid.
        unsafe { self.raw }
    }
    #[inline(always)]
    pub fn from_raw(raw: u64) -> Word {
        Word { raw }
    }

    #[inline(always)]
    pub fn rh(&self) -> i32 {
        // SAFETY: as for `raw`.
        unsafe { self.hh.rh }
    }
    #[inline(always)]
    pub fn set_rh(&mut self, v: i32) {
        self.hh.rh = v;
    }
    #[inline(always)]
    pub fn lh(&self) -> i32 {
        // SAFETY: as for `raw`.
        unsafe { self.hh.lh }
    }
    #[inline(always)]
    pub fn set_lh(&mut self, v: i32) {
        self.hh.lh = v;
    }
    #[inline(always)]
    pub fn b0(&self) -> i32 {
        // SAFETY: as for `raw`.
        unsafe { self.bb.b0 as i32 }
    }
    #[inline(always)]
    pub fn set_b0(&mut self, v: i32) {
        self.bb.b0 = v as u16;
    }
    #[inline(always)]
    pub fn b1(&self) -> i32 {
        // SAFETY: as for `raw`.
        unsafe { self.bb.b1 as i32 }
    }
    #[inline(always)]
    pub fn set_b1(&mut self, v: i32) {
        self.bb.b1 = v as u16;
    }
    #[inline(always)]
    pub fn int(&self) -> i32 {
        self.rh()
    }
    #[inline(always)]
    pub fn set_int(&mut self, v: i32) {
        self.hh.rh = v;
    }
    #[inline(always)]
    pub fn sc(&self) -> i32 {
        self.rh()
    }
    #[inline(always)]
    pub fn set_sc(&mut self, v: i32) {
        self.hh.rh = v;
    }
    #[inline(always)]
    pub fn gr(&self) -> f64 {
        // SAFETY: as for `raw`; every bit pattern is a valid f64.
        unsafe { self.gr }
    }
    #[inline(always)]
    pub fn set_gr(&mut self, v: f64) {
        self.gr = v;
    }
    #[inline(always)]
    pub fn q0(&self) -> i32 {
        // SAFETY: as for `raw`.
        unsafe { self.q[0] as i32 }
    }
    #[inline(always)]
    pub fn set_q0(&mut self, v: i32) {
        // SAFETY: as for `set_rh`.
        unsafe { self.q[0] = v as u16 }
    }
    #[inline(always)]
    pub fn q1(&self) -> i32 {
        // SAFETY: as for `raw`.
        unsafe { self.q[1] as i32 }
    }
    #[inline(always)]
    pub fn set_q1(&mut self, v: i32) {
        // SAFETY: as for `set_rh`.
        unsafe { self.q[1] = v as u16 }
    }
    #[inline(always)]
    pub fn q2(&self) -> i32 {
        // SAFETY: as for `raw`.
        unsafe { self.q[2] as i32 }
    }
    #[inline(always)]
    pub fn set_q2(&mut self, v: i32) {
        // SAFETY: as for `set_rh`.
        unsafe { self.q[2] = v as u16 }
    }
    #[inline(always)]
    pub fn q3(&self) -> i32 {
        // SAFETY: as for `raw`.
        unsafe { self.q[3] as i32 }
    }
    #[inline(always)]
    pub fn set_q3(&mut self, v: i32) {
        // SAFETY: as for `set_rh`.
        unsafe { self.q[3] = v as u16 }
    }
    #[inline(always)]
    pub fn qqqq(&self) -> FourQuarters {
        // SAFETY: as for `raw`.
        let q = unsafe { self.q };
        FourQuarters { b0: q[0] as i32, b1: q[1] as i32, b2: q[2] as i32, b3: q[3] as i32 }
    }
    #[inline(always)]
    pub fn set_qqqq(&mut self, w: FourQuarters) {
        self.q = [w.b0 as u16, w.b1 as u16, w.b2 as u16, w.b3 as u16];
    }
}

impl core::fmt::Debug for Word {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "Word(rh={}, lh={})", self.rh(), self.lh())
    }
}

const _: () = assert!(core::mem::size_of::<Word>() == 8);
const _: () = assert!(core::mem::align_of::<Word>() == 8);

// ---------------------------------------------------------------------------
// The reservation.
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// PodVec: a vector in the image's arena.
// ---------------------------------------------------------------------------

/// A growable array of `T` in the image's arena, `self.arena`: what were C statics and `^` arrays
/// (font map tree, ToUnicode table, colour stacks, image table).  It stores indices, not pointers,
/// so it survives a snapshot.  Never use `HashMap` or `Vec` for state that must survive one.
#[repr(C)]
pub struct PodVec<T> {
    pub start: i32,
    pub len: i32,
    pub cap: i32,
    _t: core::marker::PhantomData<T>,
}

impl<T> Clone for PodVec<T> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<T> Copy for PodVec<T> {}
impl<T: 'static> Pod for PodVec<T> {}

impl<T: Pod> PodVec<T> {
    /// The empty vector; `arena_push` allocates when it first needs room.  A zero `PodVec` is this.
    pub const EMPTY: PodVec<T> = PodVec { start: 0, len: 0, cap: 0, _t: core::marker::PhantomData };

    pub(crate) fn from_parts(start: i32, cap: i32) -> PodVec<T> {
        PodVec { start, len: 0, cap, _t: core::marker::PhantomData }
    }

    fn slot<const N: usize>(self, arena: &mut A<u64, N>, i: i32) -> *mut T {
        assert!(core::mem::align_of::<T>() <= 8);
        assert!(i >= 0 && i < self.cap, "PodVec index {} out of bounds {}", i, self.cap);
        let base = arena.as_mut_slice().as_mut_ptr() as *mut u8;
        // SAFETY: start..start+cap*size_of::<T>()/8 words were allocated by `arena_vec`, inside
        // the arena, and the assert above checks the index.  Word alignment is at least T's.
        unsafe { base.add(self.start as usize * 8 + i as usize * core::mem::size_of::<T>()) as *mut T }
    }

    /// Element `i`.
    #[inline]
    pub fn get<const N: usize>(self, arena: &mut A<u64, N>, i: i32) -> T {
        assert!(i < self.len, "PodVec read past its length");
        // SAFETY: `slot` checked the bounds; T is Pod, so any stored bytes are a valid T.
        unsafe { self.slot(arena, i).read() }
    }

    /// Element `i`:=x.
    #[inline]
    pub fn set<const N: usize>(self, arena: &mut A<u64, N>, i: i32, x: T) {
        assert!(i < self.len, "PodVec write past its length");
        // SAFETY: `slot` checked the bounds.
        unsafe { self.slot(arena, i).write(x) }
    }

    /// All the elements.
    #[inline]
    pub fn as_slice<const N: usize>(self, arena: &A<u64, N>) -> &[T] {
        assert!(core::mem::align_of::<T>() <= 8);
        assert!((self.start as usize + (self.len.max(0) as usize * core::mem::size_of::<T>()).div_ceil(8)) <= N, "PodVec outside its arena");
        // SAFETY: start..start+len elements are inside the arena, by the assert, and word alignment is at least T's; T is Pod, so any stored bytes are a valid T.
        unsafe { core::slice::from_raw_parts((arena.0.as_ptr() as *const u8).add(self.start as usize * 8) as *const T, self.len.max(0) as usize) }
    }

    /// Appends when there is room; callers use `Tex::arena_push`.
    #[inline]
    pub fn push_within_cap<const N: usize>(&mut self, arena: &mut A<u64, N>, x: T) {
        assert!(self.len < self.cap);
        let i = self.len;
        // SAFETY: `slot` checked the bounds.
        unsafe { self.slot(arena, i).write(x) }
        self.len += 1;
    }

    /// Copies the elements to `to`, which must have room; returns `to` with this length.  One copy of their bytes, where
    /// `get` and `set` took two checked accesses an element.  It copies padding too, which of the element types only
    /// `ColStack` has, at its end.
    pub fn moved_to<const N: usize>(self, mut to: PodVec<T>, arena: &mut A<u64, N>) -> PodVec<T> {
        assert!(self.len <= to.cap);
        if self.len > 0 {
            assert!(self.len <= self.cap, "PodVec index {} out of bounds {}", self.len - 1, self.cap);
            let n = self.len as usize * core::mem::size_of::<T>();
            let from = self.start as usize * 8;
            // `copy_within` checks both ranges against the arena.
            pod_bytes_mut(arena).copy_within(from..from + n, to.start as usize * 8);
        }
        to.len = self.len;
        to
    }
}

/// Allocates a zeroed `T` in its own 2 MiB-aligned, lazily committed reservation
/// (`platform::reserve_zeroed`), writes `host` at byte offset `host_offset`, and returns the
/// never-moved value.
///
/// # Contract, checked at the call site in `state.rs`
///
/// `T` is valid when its bytes are zero except for one field of type `H` at `host_offset`, which
/// is the case for `Tex`: every other field is `Pod`.  The pages commit lazily, so capacities cost
/// nothing until used.
pub fn alloc_image<T, H>(host_offset: usize, host: H) -> &'static mut T {
    assert!(host_offset + core::mem::size_of::<H>() <= core::mem::size_of::<T>());
    let p = crate::platform::reserve_zeroed(core::mem::size_of::<T>());
    if p.is_null() {
        // Not a bug, so no panic: a limit such as `ulimit -v`, or `vm.overcommit_memory=2`.
        crate::platform::fatal(&format!("rstex: fatal: cannot reserve {} bytes of address space for the engine's memory: only what it touches gets used, but a limit on the address space stops it.\n", core::mem::size_of::<T>()));
    }
    // SAFETY: the reservation is zeroed and at least `size_of::<T>()` bytes; its alignment of 2 MiB
    // exceeds any field's.  The host slot is inside `T` by the assert above and is zero bytes, so
    // `write` does not drop anything.  By the contract of this function the zero bytes form a valid
    // `T` once the host field is written.  The reservation lives until `free_image`.
    unsafe {
        p.add(host_offset).cast::<H>().write(host);
        &mut *(p as *mut T)
    }
}

/// Gives back a value from [`alloc_image`].  The host field must have been dropped by the caller.
pub fn free_image<T>(p: &'static mut T) {
    // SAFETY: `p` came from `alloc_image`, and we consumed the only reference to it.
    unsafe {
        crate::platform::release(p as *mut T as *mut u8, core::mem::size_of::<T>());
    }
}

/// Ends the life of a value in place; used on the host field before [`free_image`].
pub fn drop_in_place_of<H>(h: &mut H) {
    // SAFETY: the caller frees the enclosing image right after and never touches `h` again.
    unsafe { core::ptr::drop_in_place(h) }
}

// ---------------------------------------------------------------------------
// Pascal arithmetic that needs more than an operator; see docs/PORTING.md.
// ---------------------------------------------------------------------------

/// web2c's `zround` (lib/zround.c): round half away from zero, clamped to +-2147483647.
#[inline]
pub fn round(r: f64) -> i32 {
    if r > 2147483647.0 {
        2147483647
    } else if r < -2147483647.0 {
        -2147483647
    } else if r >= 0.0 {
        (r + 0.5) as i32
    } else {
        (r - 0.5) as i32
    }
}

/// C's `(int)` of a double, as x86-64's `cvttsd2si` gives it on the official pdfTeX: truncated, and -2^31 for what is out of range, where
/// Rust's `as` stops at either end.  Arch's casts to its 64-bit `integer`, which an `i32` cannot hold.
#[inline]
pub fn c_int(r: f64) -> i32 {
    if r > -2147483649.0 && r < 2147483648.0 { r as i32 } else { i32::MIN }
}

/// Pascal `odd`.
#[inline(always)]
pub const fn odd(x: i32) -> bool {
    x & 1 != 0
}

/// `abs`.  `abs(-2^31)` is -2^31, as in the official pdfTeX, whose `integer` is an `int`, and which then does
/// not take `\\dimen1` for too large: docs/PORTING.md section 11.
#[inline(always)]
pub const fn abs(x: i32) -> i32 {
    x.wrapping_abs()
}

/// What an `i32` word keeps of a sum held in an `i64`: its low 32 bits.
///
/// Past 2^31 the two pdfTeXes part.  Arch's has a 64-bit `integer` and holds the true value; the official one has
/// an `int`, and the same store wraps.  An `i32` cannot hold the first, so a word takes the second: that agrees with a
/// real pdfTeX, the one most people have, and a sum that comes back through it comes back right, as on both.
/// docs/PORTING.md section 11 has the rule.
#[inline(always)]
pub const fn wrap32(x: i64) -> i32 {
    x as i32
}

// ---------------------------------------------------------------------------
// Bytes of the image, for format files and checkpoints.  Modules other than this one forbid unsafe.
// ---------------------------------------------------------------------------

/// The bytes of a POD value: a field or a record of the format file.
#[inline]
pub fn pod_bytes<T: Pod>(x: &T) -> &[u8] {
    // SAFETY: a Pod type has no pointers, and the slice covers exactly `x`.
    unsafe { core::slice::from_raw_parts(x as *const T as *const u8, core::mem::size_of::<T>()) }
}

/// The bytes of a POD value, to read it from a file written by this same build.  Files are keyed on
/// the build id (DESIGN.md section 7), so a `bool` in them holds 0 or 1.
#[inline]
pub fn pod_bytes_mut<T: Pod>(x: &mut T) -> &mut [u8] {
    // SAFETY: as pod_bytes, for a file of this build.
    unsafe { core::slice::from_raw_parts_mut(x as *mut T as *mut u8, core::mem::size_of::<T>()) }
}

/// `len` bytes at `offset` in the image: a [`crate::state::PodSection`]'s live range.
#[inline]
pub fn image_bytes<T>(t: &T, offset: usize, len: usize) -> &[u8] {
    assert!(offset.checked_add(len).is_some_and(|e| e <= core::mem::size_of::<T>()), "outside the image");
    // SAFETY: the range is inside `*t`, by the assert.
    unsafe { core::slice::from_raw_parts((t as *const T as *const u8).add(offset), len) }
}

/// `len` bytes at `offset` in the image, to read a section into.  Call `platform::commit_range` on
/// them first: on Windows a read into uncommitted pages fails rather than faulting.
#[inline]
pub fn image_bytes_mut<T>(t: &mut T, offset: usize, len: usize) -> &mut [u8] {
    assert!(offset.checked_add(len).is_some_and(|e| e <= core::mem::size_of::<T>()), "outside the image");
    // SAFETY: the range is inside `*t`, by the assert.
    unsafe { core::slice::from_raw_parts_mut((t as *mut T as *mut u8).add(offset), len) }
}

// ---------------------------------------------------------------------------
// eexec decryption eight bytes at a time, for `rstex_fontfile`, which forbids unsafe.
// ---------------------------------------------------------------------------

/// [`rstex_fontfile::decrypt_run`], the same bytes and state after, as [`rstex_fontfile::T1Input::decrypt`]: on x86_64 the
/// whole groups of eight bytes go through SSE2, and what is left through the loop.  Elsewhere it is the loop.
///
/// The state after a cipher byte `c` is `r' = A*r + (A*c + B)` modulo 2^16, with A = 52845 and B = 22719: affine in `r` once
/// `c` is known.  So the states before eight bytes are a prefix sum of eight such maps, which three steps of a scan give in
/// eight lanes of 16 bits, each product and sum modulo 2^16 as the loop's are: some 34 instructions for eight bytes, where
/// the loop takes 9 or 10 a byte.  Most of the bytes of a Type 1 font are charstrings, which this decrypts.  Calling a
/// `#[target_feature]` function takes `unsafe` even where the build has the feature, and `rstex_fontfile` forbids `unsafe`,
/// so the font code gets this function from the engine.
pub fn eexec_decrypt(src: &[u8], dst: &mut [u8], r: u16) -> u16 {
    let n = src.len().min(dst.len());
    #[cfg(all(target_arch = "x86_64", target_feature = "sse2"))]
    // SAFETY: the build has SSE2 (every x86_64 target rstex ships on does), so every processor that runs it has SSE2.
    let (done, r) = unsafe { eexec_decrypt_sse2(&src[..n], &mut dst[..n], r) };
    #[cfg(not(all(target_arch = "x86_64", target_feature = "sse2")))]
    let done = 0;
    rstex_fontfile::decrypt_run(&src[done..n], &mut dst[done..n], r)
}

/// The whole groups of eight bytes of [`eexec_decrypt`]: how many bytes it did, and the state after them.
#[cfg(all(target_arch = "x86_64", target_feature = "sse2"))]
#[target_feature(enable = "sse2")]
fn eexec_decrypt_sse2(src: &[u8], dst: &mut [u8], mut r: u16) -> (usize, u16) {
    use core::arch::x86_64::*;
    const A: u16 = 52845;
    const B: u16 = 22719;
    /// A^k modulo 2^16, as a lane.
    const fn a_pow(k: u32) -> i16 {
        let mut x: u16 = 1;
        let mut i = 0;
        while i < k {
            x = x.wrapping_mul(A);
            i += 1;
        }
        x as i16
    }
    let a1 = _mm_set1_epi16(A as i16);
    let b = _mm_set1_epi16(B as i16);
    let a2 = _mm_set1_epi16(a_pow(2));
    let a4 = _mm_set1_epi16(a_pow(4));
    let a_k = _mm_setr_epi16(a_pow(0), a_pow(1), a_pow(2), a_pow(3), a_pow(4), a_pow(5), a_pow(6), a_pow(7));
    let a8 = a_pow(8) as u16;
    let zero = _mm_setzero_si128();
    let n = src.len().min(dst.len()) / 8 * 8;
    for (s, d) in src[..n].chunks_exact(8).zip(dst[..n].chunks_exact_mut(8)) {
        let s: [u8; 8] = s.try_into().expect("eight bytes");
        // c[k], the cipher bytes, one to a lane
        let c = _mm_unpacklo_epi8(_mm_cvtsi64_si128(i64::from_le_bytes(s)), zero);
        // y[k] = A*c[k] + B, then x[k] = the sum over j <= k of A^(k-j) * y[j], in steps of 1, 2 and 4 lanes
        let mut x = _mm_add_epi16(_mm_mullo_epi16(c, a1), b);
        x = _mm_add_epi16(x, _mm_mullo_epi16(_mm_slli_si128::<2>(x), a1));
        x = _mm_add_epi16(x, _mm_mullo_epi16(_mm_slli_si128::<4>(x), a2));
        x = _mm_add_epi16(x, _mm_mullo_epi16(_mm_slli_si128::<8>(x), a4));
        // the state before byte k is A^k * r + x[k-1], and the plain byte c[k] ^ (state >> 8), 0..255 for `packus`
        let st = _mm_add_epi16(_mm_slli_si128::<2>(x), _mm_mullo_epi16(a_k, _mm_set1_epi16(r as i16)));
        let p = _mm_xor_si128(c, _mm_srli_epi16::<8>(st));
        d.copy_from_slice(&_mm_cvtsi128_si64(_mm_packus_epi16(p, p)).to_le_bytes());
        // the state after the eight: A^8 * r + x[7]
        r = (_mm_extract_epi16::<7>(x) as u16).wrapping_add(a8.wrapping_mul(r));
    }
    (n, r)
}

/// [`image_bytes_mut`] for several ranges at once, `(offset, len)` each, in the order of the image and none overlapping another, which is asserted: one read can fill them all.
/// `out` gets one slice for each range, from the start.
pub fn image_ranges_mut<'a, T>(t: &'a mut T, ranges: &[(usize, usize)], out: &mut [&'a mut [u8]]) {
    assert!(ranges.len() <= out.len());
    let mut end = 0;
    for &(at, n) in ranges {
        assert!(at >= end && at.checked_add(n).is_some_and(|e| e <= core::mem::size_of::<T>()), "outside the image, or out of order");
        end = at + n;
    }
    let base = t as *mut T as *mut u8;
    for (o, &(at, n)) in out.iter_mut().zip(ranges) {
        // SAFETY: each range is inside `*t` and after the one before it, by the asserts, so the slices are disjoint parts of the one borrow of `*t`.
        *o = unsafe { core::slice::from_raw_parts_mut(base.add(at), n) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The slices are the ranges asked for, of the one value; ranges out of order, overlapping or past its end are refused.
    #[test]
    fn image_ranges_are_the_ones_asked_for() {
        let mut t = [0u8; 64];
        let mut out: [&mut [u8]; 4] = Default::default();
        image_ranges_mut(&mut t, &[(0, 3), (3, 0), (10, 4), (60, 4)], &mut out);
        assert_eq!(out.iter().map(|s| s.len()).collect::<Vec<_>>(), [3, 0, 4, 4]);
        out[0].fill(1);
        out[2].fill(2);
        out[3].fill(3);
        assert_eq!(&t[..4], [1, 1, 1, 0]);
        assert_eq!(&t[9..15], [0, 2, 2, 2, 2, 0]);
        assert_eq!(&t[59..], [0, 3, 3, 3, 3]);
        for bad in [&[(0, 4), (3, 4)][..], &[(8, 4), (0, 4)], &[(60, 5)], &[(1, usize::MAX)]] {
            let r = std::panic::catch_unwind(|| {
                let mut t = [0u8; 64];
                let mut out: [&mut [u8]; 2] = Default::default();
                image_ranges_mut(&mut t, bad, &mut out);
            });
            assert!(r.is_err(), "{bad:?}");
        }
    }

    #[test]
    fn word_views_overlap_as_documented() {
        let mut w = Word::ZERO;
        w.set_rh(-7);
        assert_eq!(w.int(), -7, "int overlaps rh");
        assert_eq!(w.sc(), -7, "sc is int");
        w.set_lh(0x0003_0002);
        assert_eq!(w.b0(), 3, "b0 is the high half of lh, as in texmfmem.h");
        assert_eq!(w.b1(), 2, "b1 is the low half of lh");
        assert_eq!(w.rh(), -7, "lh does not disturb rh");
        w.set_b0(9);
        assert_eq!(w.lh(), 0x0009_0002);
        w.set_gr(1.5);
        assert_eq!(w.gr(), 1.5);
        let x = Word::hb(5, 6, 7);
        assert_eq!((x.rh(), x.b0(), x.b1()), (5, 6, 7));
        let y = Word::hh(5, 6);
        assert_eq!((y.rh(), y.lh()), (5, 6));
    }

    #[test]
    fn word_qqqq() {
        let mut w = Word::ZERO;
        w.set_qqqq(FourQuarters { b0: 1, b1: 2, b2: 300, b3: 65535 });
        let q = w.qqqq();
        assert_eq!((q.b0, q.b1, q.b2, q.b3), (1, 2, 300, 65535));
        assert_eq!((w.q0(), w.q1(), w.q2(), w.q3()), (1, 2, 300, 65535));
        w.set_q2(4);
        assert_eq!(w.qqqq().b2, 4);
    }

    #[test]
    fn quarterwords_are_unsigned_16_bits() {
        let mut w = Word::ZERO;
        w.set_b1(65535);
        assert_eq!(w.b1(), 65535);
        w.set_b0(9000);
        assert_eq!(w.b0(), 9000, "font numbers fit");
    }

    #[test]
    fn arrays_index_by_i32() {
        let mut a: A<i32, 4> = A([0; 4]);
        a[3] = 7;
        assert_eq!(a[3], 7);
        let mut b: A<u8, 4> = A([0; 4]);
        b.put(1, 200);
        assert_eq!(b.at(1), 200, "compact cells read back as i32");
        b.put(2, 300);
        assert_eq!(b.at(2), 44, "a store truncates");
        let mut c: A<u16, 2> = A([0; 2]);
        c.put(1, 65535);
        assert_eq!(c.at(1), 65535);
        let mut o: AO<i32, -2, 5> = AO([0; 5]);
        o[-2] = 1;
        o[2] = 2;
        assert_eq!((o[-2], o[2]), (1, 2));
        assert_eq!(o.0, [1, 0, 0, 0, 2]);
        let mut p: AO<i32, 10, 2> = AO([0; 2]);
        p[11] = 3;
        assert_eq!(p.0[1], 3);
    }

    #[cfg(feature = "checked")]
    #[test]
    #[should_panic]
    fn checked_catches_a_negative_index() {
        let a: A<i32, 4> = A([0; 4]);
        let i = -1;
        let _ = a[i];
    }

    #[cfg(feature = "checked")]
    #[test]
    #[should_panic]
    fn checked_catches_an_index_past_the_end() {
        let a: A<i32, 4> = A([0; 4]);
        let i = 4;
        let _ = a[i];
    }

    #[test]
    fn round_is_zround() {
        assert_eq!(round(0.5), 1);
        assert_eq!(round(1.5), 2);
        assert_eq!(round(2.5), 3, "half away from zero, not to even");
        assert_eq!(round(-0.5), -1);
        assert_eq!(round(-2.5), -3);
        assert_eq!(round(0.49999), 0);
        assert_eq!(round(3e9), 2147483647, "clamped");
        assert_eq!(round(-3e9), -2147483647, "clamped, to -(2^31-1)");
        assert_eq!(round(2147483647.0), 2147483647);
    }

    #[test]
    fn c_int_is_what_x86_64_gives() {
        assert_eq!(c_int(1.9), 1);
        assert_eq!(c_int(-1.9), -1);
        assert_eq!(c_int(2147483647.5), 2147483647);
        assert_eq!(c_int(2147483648.0), i32::MIN);
        assert_eq!(c_int(-2147483648.5), i32::MIN);
        assert_eq!(c_int(-2147483649.0), i32::MIN);
        assert_eq!(c_int(1e300), i32::MIN);
        assert_eq!(c_int(f64::NAN), i32::MIN);
    }

    #[test]
    fn pascal_division_is_rust_division() {
        assert_eq!(-7 / 2, -3, "div truncates toward zero");
        assert_eq!(-7 % 2, -1, "mod takes the sign of the dividend");
        assert_eq!(7 % -2, 1);
        assert!(odd(-3));
        assert!(!odd(-4));
        assert_eq!(abs(-5), 5);
        assert_eq!(abs(i32::MIN), i32::MIN, "as on the official pdfTeX, whose negate of -2^31 is itself");
    }

    #[test]
    fn wrap32_is_what_the_official_pdftex_stores() {
        assert_eq!(wrap32(0), 0);
        assert_eq!(wrap32(-5), -5);
        assert_eq!(wrap32(2147483647), 2147483647);
        assert_eq!(wrap32(2147483648), -2147483648, "\\count1=2147483647 \\advance\\count1 by 1");
        assert_eq!(wrap32(-2147483648), -2147483648);
        assert_eq!(wrap32(-2147483649), 2147483647);
        assert_eq!(wrap32(6442450941), 2147483645, "three \\advance of 2147483647");
        assert_eq!(wrap32(3 * (16383 * 65536)), -1073938432, "\\hbox of 3 x 16383pt");
    }

    #[test]
    fn bytes_of_the_image() {
        let mut x: i32 = 0x01020304;
        assert_eq!(pod_bytes(&x), &[4, 3, 2, 1]);
        pod_bytes_mut(&mut x)[0] = 5;
        assert_eq!(x, 0x01020305);
        let mut a: [u16; 3] = [1, 2, 3];
        assert_eq!(image_bytes(&a, 2, 2), &[2, 0]);
        image_bytes_mut(&mut a, 4, 2)[1] = 1;
        assert_eq!(a[2], 0x0103);
    }

    #[test]
    fn pod_vec_in_an_arena() {
        crate::pod_struct! {
            pub struct Pair { pub a: i32, pub b: i32 }
        }
        let mut arena: Box<A<u64, 64>> = Box::new(A([0; 64]));
        let mut v: PodVec<Pair> = PodVec::from_parts(4, 3);
        assert_eq!(v.len, 0);
        v.push_within_cap(&mut arena, Pair { a: 1, b: 2 });
        v.push_within_cap(&mut arena, Pair { a: 3, b: 4 });
        assert_eq!(v.get(&mut arena, 1).b, 4);
        v.set(&mut arena, 0, Pair { a: 9, b: 9 });
        let w = v.moved_to(PodVec::from_parts(20, 8), &mut arena);
        assert_eq!(w.len, 2);
        assert_eq!(w.get(&mut arena, 0).a, 9);
        assert_eq!(w.get(&mut arena, 1).a, 3);
    }

    #[test]
    fn moved_to_copies_the_elements_and_nothing_else() {
        crate::pod_struct! {
            pub struct Three { pub a: u8, pub b: u8, pub c: u8 }
        }
        let mut arena: Box<A<u64, 64>> = Box::new(A([0x5a5a_5a5a_5a5a_5a5a; 64]));
        let mut v: PodVec<Three> = PodVec::from_parts(2, 5);
        for i in 0..5 {
            v.push_within_cap(&mut arena, Three { a: i, b: 10 + i, c: 20 + i });
        }
        let before = arena.0;
        let w = v.moved_to(PodVec::from_parts(30, 9), &mut arena);
        assert_eq!(w.len, 5);
        for i in 0..5 {
            let t = w.get(&mut arena, i);
            assert_eq!((t.a, t.b, t.c), (i as u8, 10 + i as u8, 20 + i as u8));
        }
        // 15 bytes at word 30, and every other byte as it was
        let bytes = |a: &[u64; 64]| a.iter().flat_map(|w| w.to_le_bytes()).collect::<Vec<u8>>();
        let (after, was) = (bytes(&arena.0), bytes(&before));
        for k in 0..after.len() {
            if !(240..255).contains(&k) {
                assert_eq!(after[k], was[k], "byte {k}");
            }
        }
        let e: PodVec<Three> = PodVec::EMPTY;
        assert_eq!(e.moved_to(PodVec::from_parts(40, 1), &mut arena).len, 0);
    }

    #[test]
    fn eexec_decrypt_is_the_loop() {
        let mut x: u64 = 0x9E37_79B9_7F4A_7C15;
        let mut next = move || {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x
        };
        let check = |src: &[u8], dst_len: usize, r: u16| {
            let (mut a, mut b) = (vec![0u8; dst_len], vec![0u8; dst_len]);
            assert_eq!(eexec_decrypt(src, &mut a, r), rstex_fontfile::decrypt_run(src, &mut b, r), "{src:?} from {r}");
            assert_eq!(a, b, "{src:?} from {r}");
        };
        // every state to start from, with lengths on both sides of a group of eight
        for r in 0..=u16::MAX {
            let src: Vec<u8> = (0..next() % 41).map(|_| next() as u8).collect();
            check(&src, src.len(), r);
        }
        // long runs, a destination longer or shorter than the source, and runs of one byte
        for i in 0..2000u64 {
            let len = (next() % 5000) as usize;
            let src: Vec<u8> = if i % 7 == 0 { vec![next() as u8; len] } else { (0..len).map(|_| next() as u8).collect() };
            let dst_len = match i % 3 {
                0 => len,
                1 => len + (next() % 9) as usize,
                _ => len.saturating_sub((next() % 9) as usize),
            };
            check(&src, dst_len, next() as u16);
        }
    }

    #[test]
    fn image_is_zeroed_aligned_and_lazily_committed() {
        #[repr(C)]
        struct Big {
            a: [u64; 1 << 24],
            host: Vec<u8>,
        }
        let b: &'static mut Big = alloc_image::<Big, Vec<u8>>(core::mem::offset_of!(Big, host), vec![1, 2, 3]);
        assert_eq!(b as *mut Big as usize % (2 << 20), 0, "2 MiB aligned");
        assert_eq!(b.a[12345], 0, "zeroed");
        b.a[(1 << 24) - 1] = 5;
        assert_eq!(b.host, vec![1, 2, 3], "the host field was constructed in place");
        drop_in_place_of(&mut b.host);
        free_image(b);
    }
}
