//! Unchecked storage: the array wrappers, the memory word, and the image's allocation.
//!
//! With `platform/`, the only engine code allowed `unsafe`; all others are `#![forbid(unsafe_code)]`.
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

    /// Appends when there is room; callers use `Tex::arena_push`.
    #[inline]
    pub fn push_within_cap<const N: usize>(&mut self, arena: &mut A<u64, N>, x: T) {
        assert!(self.len < self.cap);
        let i = self.len;
        // SAFETY: `slot` checked the bounds.
        unsafe { self.slot(arena, i).write(x) }
        self.len += 1;
    }

    /// Copies the elements to `to`, which must have room; returns `to` with this length.
    pub fn moved_to<const N: usize>(self, mut to: PodVec<T>, arena: &mut A<u64, N>) -> PodVec<T> {
        assert!(self.len <= to.cap);
        for i in 0..self.len {
            let x = self.get(arena, i);
            to.len = i + 1;
            to.set(arena, i, x);
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
        panic!("rstex: cannot reserve {} bytes of address space", core::mem::size_of::<T>());
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

/// Pascal `odd`.
#[inline(always)]
pub const fn odd(x: i32) -> bool {
    x & 1 != 0
}

/// `abs`.  The port never makes `i32::MIN`: [`round`] and [`sat32`] stop at -(2^31-1).  The wrap only
/// keeps this total; the oracle's `integer` is 64 bits and has no such value.
#[inline(always)]
pub const fn abs(x: i32) -> i32 {
    x.wrapping_abs()
}

/// Stores a wide intermediate in an `i32`, clamped to +-(2^31-1) like [`round`].
///
/// The oracle's `integer` and `scaled` are 64 bits, so where WEB's arithmetic leaves the `i32` range
/// the oracle goes on with the exact value.  Such a site computes in `i64` and clamps on store, which
/// keeps the sign and a comparison with anything below 2^31, in particular "Dimension too large".  What is
/// stored is another number all the same, so printing it differs, and so does what is computed from it
/// later: docs/PORTING.md section 11, and "Known divergences" in section 15.
#[inline(always)]
pub const fn sat32(x: i64) -> i32 {
    if x > i32::MAX as i64 {
        i32::MAX
    } else if x < -(i32::MAX as i64) {
        -i32::MAX
    } else {
        x as i32
    }
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

#[cfg(test)]
mod tests {
    use super::*;

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
    fn pascal_division_is_rust_division() {
        assert_eq!(-7 / 2, -3, "div truncates toward zero");
        assert_eq!(-7 % 2, -1, "mod takes the sign of the dividend");
        assert_eq!(7 % -2, 1);
        assert!(odd(-3));
        assert!(!odd(-4));
        assert_eq!(abs(-5), 5);
        assert_eq!(abs(i32::MIN), i32::MIN, "total, though the port never makes this value");
    }

    #[test]
    fn sat32_is_symmetric_and_keeps_the_sign() {
        assert_eq!(sat32(0), 0);
        assert_eq!(sat32(-5), -5);
        assert_eq!(sat32(2147483647), 2147483647);
        assert_eq!(sat32(2147483648), 2147483647, "\\count1=2147483647 \\advance\\count1 by 1 stays positive");
        assert_eq!(sat32(-2147483647), -2147483647);
        assert_eq!(sat32(-2147483648), -2147483647, "never i32::MIN, which negate and print_scaled cannot take");
        assert_eq!(sat32(6442450941), 2147483647);
        assert_eq!(sat32(4 * (16383 * 65536)), 2147483647, "\\hbox of 4 x 16383pt: still >=2^30, so Dimension too large");
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
